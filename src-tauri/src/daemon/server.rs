//! O laço do daemon: aceita conexões do MESMO usuário (peer cred do kernel,
//! nunca o que o pedido diz), exige o `hello` com a versão do protocolo e
//! atende os pedidos. Cada conexão tem uma fila de saída e uma thread que a
//! escreve no socket: respostas e eventos saem na ordem em que entraram.

use std::io::BufWriter;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::host::{Host, Outbox, SpawnSpec};
use super::protocol::{read_frame, write_frame, Frame, Reply, Request, PROTOCOL_VERSION};

/// Quadros na fila de uma conexão antes de ela ser considerada lenta.
const OUTBOX_FRAMES: usize = 1024;
const MAX_CONNECTIONS: usize = 16;
/// O `hello` tem de chegar logo; depois, a conexão pode ficar quieta.
const HELLO_TIMEOUT: Duration = Duration::from_secs(2);
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

pub struct Server {
    host: Arc<Host>,
    next_conn: AtomicU64,
    connections: Arc<AtomicUsize>,
    shutdown: AtomicBool,
}

struct ConnGuard(Arc<AtomicUsize>);

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Server {
    pub fn new(host: Arc<Host>) -> Arc<Self> {
        Arc::new(Server {
            host,
            next_conn: AtomicU64::new(1),
            connections: Arc::new(AtomicUsize::new(0)),
            shutdown: AtomicBool::new(false),
        })
    }

    /// Atende até `stop`. Não remove o nome do socket ao sair (ver `endpoint`).
    pub fn serve(self: &Arc<Self>, listener: &UnixListener) {
        for stream in listener.incoming() {
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else {
                // Sem fd livre o `accept` falha na hora: sem a pausa, o laço
                // giraria a CPU inteira até sobrar um.
                std::thread::sleep(ACCEPT_BACKOFF);
                continue;
            };
            if !trusted(&stream) {
                continue;
            }
            let guard = ConnGuard(Arc::clone(&self.connections));
            if self.connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                continue;
            }
            let conn = self.next_conn.fetch_add(1, Ordering::SeqCst);
            let server = Arc::clone(self);
            let _ = std::thread::Builder::new()
                .name(format!("daemon-conn-{conn}"))
                .spawn(move || {
                    let _guard = guard;
                    server.handle(conn, stream);
                    server.host.drop_connection(conn);
                });
        }
    }

    /// Para o laço (para testes e para o encerramento). Acorda o `accept` com
    /// uma conexão própria, que é recusada logo depois.
    pub fn stop(&self, socket: &std::path::Path) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(socket);
    }

    fn handle(&self, conn: u64, stream: UnixStream) {
        let Ok(write_half) = stream.try_clone() else {
            return;
        };
        let (outbox, queue) = std::sync::mpsc::sync_channel::<Frame<Reply>>(OUTBOX_FRAMES);
        let writer = std::thread::Builder::new()
            .name(format!("daemon-out-{conn}"))
            .spawn(move || {
                let mut out = BufWriter::new(write_half);
                for frame in queue {
                    if write_frame(&mut out, &frame).is_err() {
                        break;
                    }
                }
                let _ = out
                    .into_inner()
                    .map(|s| s.shutdown(std::net::Shutdown::Both));
            });
        if writer.is_err() {
            return;
        }
        let mut input = std::io::BufReader::new(stream);
        let _ = input.get_ref().set_read_timeout(Some(HELLO_TIMEOUT));
        match read_frame::<Request>(&mut input) {
            Ok(Some(Frame {
                header: Request::Hello { v },
                ..
            })) if v == PROTOCOL_VERSION => {
                let _ = outbox.send(Frame::new(Reply::HelloOk {
                    v: PROTOCOL_VERSION,
                    pid: std::process::id(),
                }));
            }
            Ok(Some(_)) => {
                let _ = outbox.send(Frame::new(Reply::Error {
                    seq: 0,
                    message: format!("protocolo do daemon é a versão {PROTOCOL_VERSION}"),
                }));
                return;
            }
            _ => return,
        }
        let _ = input.get_ref().set_read_timeout(None);
        // Quadro inválido ou fim do fluxo: a conexão acaba. Os PTYs seguem.
        while let Ok(Some(frame)) = read_frame::<Request>(&mut input) {
            if let Some(reply) = self.answer(conn, &outbox, frame) {
                if outbox.send(reply).is_err() {
                    break;
                }
            }
        }
    }

    fn answer(&self, conn: u64, outbox: &Outbox, frame: Frame<Request>) -> Option<Frame<Reply>> {
        let done = |seq: u64, result: Result<(), String>| {
            Frame::new(match result {
                Ok(()) => Reply::Ok { seq },
                Err(message) => Reply::Error { seq, message },
            })
        };
        Some(match frame.header {
            Request::Hello { .. } => Frame::new(Reply::Error {
                seq: 0,
                message: "hello repetido".into(),
            }),
            Request::Spawn {
                seq,
                id,
                argv,
                env,
                cwd,
                cols,
                rows,
            } => Frame::new(
                match self.host.spawn(SpawnSpec {
                    id,
                    argv,
                    env,
                    cwd,
                    cols,
                    rows,
                }) {
                    Ok(pid) => Reply::Spawned { seq, pid },
                    Err(message) => Reply::Error { seq, message },
                },
            ),
            Request::Write { seq, id } => done(seq, self.host.write(id, &frame.body)),
            Request::Resize {
                seq,
                id,
                cols,
                rows,
            } => done(seq, self.host.resize(id, cols, rows)),
            Request::Kill { seq, id } => done(seq, self.host.kill(id)),
            Request::List { seq } => Frame::new(Reply::Ptys {
                seq,
                ptys: self.host.list(),
            }),
            // A resposta do `attach` entra na fila pelo próprio host, junto
            // com o registro: nada a mandar daqui quando deu certo.
            Request::Attach { seq, id } => match self.host.attach(id, seq, conn, outbox) {
                Ok(()) => return None,
                Err(message) => Frame::new(Reply::Error { seq, message }),
            },
            Request::Detach { seq, id } => {
                self.host.detach(id, conn);
                Frame::new(Reply::Ok { seq })
            }
            Request::Query { seq, id } => Frame::new(match self.host.query(id) {
                Ok((foreground_group, line_echo)) => Reply::State {
                    seq,
                    foreground_group,
                    line_echo,
                },
                Err(message) => Reply::Error { seq, message },
            }),
        })
    }
}

fn trusted(stream: &UnixStream) -> bool {
    crate::hook_ipc::peercred::peer_cred(stream).is_some_and(|cred| {
        crate::hook_ipc::peercred::is_trusted_peer(&cred, unsafe { libc::getuid() })
    })
}
