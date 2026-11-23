//! O socket de controle da CLI (`tyba ls`, `tyba capture`), irmão do canal
//! shim↔core (`hook_ipc::channel`): mesma pasta privada por usuário, mesma
//! disciplina — pasta 0700, socket 0600, peer cred do kernel conferido em
//! toda conexão, leitura com teto de bytes e de tempo. Um pedido por conexão,
//! uma linha JSON de ida e uma de volta.
//!
//! Só leitura nesta onda: `ls` e `capture` (já redigido). `send`/`run` vão
//! precisar do guarda-corpo e ficam para depois.

use std::io::{BufRead, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use super::{ControlRequest, ControlResponse};

/// O segundo argumento diz se o cliente ainda está do outro lado: o
/// `run --wait` pergunta enquanto espera, e desiste quando ninguém mais lê
/// (sem isso, 16 clientes mortos prenderiam todas as vagas).
pub type ControlHandler =
    Arc<dyn Fn(ControlRequest, &dyn Fn() -> bool) -> ControlResponse + Send + Sync>;

const MAX_INFLIGHT: usize = 16;
/// Um pedido é uma linha minúscula (`{"v":1,"op":"capture",…}`).
const MAX_REQUEST_BYTES: u64 = 16 * 1024;
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(2);

struct InflightGuard(Arc<AtomicUsize>);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct ControlServer {
    socket_path: PathBuf,
    shutdown: Arc<AtomicBool>,
    accept_handle: Option<JoinHandle<()>>,
}

impl ControlServer {
    /// Liga sob uma pasta PRIVADA (0700, do usuário — criada ou conferida
    /// aqui) e põe 0600 no próprio socket.
    pub fn bind(socket_path: &Path, handler: ControlHandler) -> std::io::Result<ControlServer> {
        use std::os::unix::fs::PermissionsExt;

        let parent = socket_path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "socket de controle sem diretório pai",
            )
        })?;
        crate::session::create_private_dir(parent)?;
        crate::session::verify_private_dir(parent)?;
        if socket_path.exists() {
            let _ = std::fs::remove_file(socket_path);
        }
        let listener = UnixListener::bind(socket_path)?;
        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;

        let shutdown = Arc::new(AtomicBool::new(false));
        let accept_shutdown = shutdown.clone();
        let inflight = Arc::new(AtomicUsize::new(0));
        let accept_handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if accept_shutdown.load(Ordering::SeqCst) {
                    break;
                }
                match stream {
                    Ok(stream) => dispatch(stream, &handler, &inflight),
                    Err(_) => break,
                }
            }
        });
        Ok(ControlServer {
            socket_path: socket_path.to_path_buf(),
            shutdown,
            accept_handle: Some(accept_handle),
        })
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(handle) = self.accept_handle.take() {
            let _ = handle.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn dispatch(stream: UnixStream, handler: &ControlHandler, inflight: &Arc<AtomicUsize>) {
    let guard = InflightGuard(inflight.clone());
    if inflight.fetch_add(1, Ordering::SeqCst) >= MAX_INFLIGHT {
        return;
    }
    let handler = handler.clone();
    std::thread::spawn(move || {
        let _guard = guard;
        serve(stream, &handler);
    });
}

/// O cliente fechou? `recv` com `MSG_PEEK | MSG_DONTWAIT`: 0 é fim de
/// conexão; nada para ler (`EAGAIN`) é cliente vivo esperando.
fn client_connected(stream: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let mut byte = 0u8;
    let n = unsafe {
        libc::recv(
            stream.as_raw_fd(),
            (&mut byte as *mut u8).cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    match n {
        0 => false,
        n if n > 0 => true,
        _ => matches!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        ),
    }
}

fn serve(stream: UnixStream, handler: &ControlHandler) {
    // Quem conectou tem de ser do MESMO usuário, pelo kernel — nunca pelo
    // que o pedido diz. Sem leitura de peer cred, recusa (fail closed).
    let trusted = crate::hook_ipc::peercred::peer_cred(&stream).is_some_and(|cred| {
        crate::hook_ipc::peercred::is_trusted_peer(&cred, unsafe { libc::getuid() })
    });
    if !trusted {
        return;
    }
    let _ = stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT));
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = std::io::BufReader::new(read_half.take(MAX_REQUEST_BYTES));
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => return,
        Ok(_) => {}
    }
    let response = match serde_json::from_str::<ControlRequest>(line.trim_end()) {
        Ok(request) if request.v == super::PROTOCOL_VERSION => {
            handler(request, &|| client_connected(&stream))
        }
        Ok(_) => ControlResponse::error(super::EXIT_FAILURE, "versão de protocolo diferente"),
        Err(_) => ControlResponse::error(super::EXIT_USAGE, "pedido inválido"),
    };
    let Ok(mut payload) = serde_json::to_vec(&response) else {
        return;
    };
    payload.push(b'\n');
    let mut writer = stream;
    let _ = writer.write_all(&payload);
    let _ = writer.flush();
}
