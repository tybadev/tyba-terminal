//! Os PTYs que o daemon segura: abre o processo, lê os bytes, guarda um anel
//! dos mais recentes e repassa a quem está ligado. Nada aqui grava em disco
//! (princípio 10): o anel vive e morre na memória do daemon.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::Arc;

use parking_lot::Mutex;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use uuid::Uuid;

use super::protocol::{Frame, PtyInfo, Reply};

/// Quanto da saída recente fica para quem religar. A tela e o histórico de
/// 1000 linhas do app cabem com folga.
pub const RING_BYTES: usize = 1024 * 1024;
const READ_BUF: usize = 8 * 1024;
const REAP_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// Quem recebe a saída: a fila de saída de uma conexão.
pub type Outbox = SyncSender<Frame<Reply>>;

struct Output {
    ring: VecDeque<u8>,
    /// O anel já descartou bytes do começo: o primeiro pedaço pode ser meia
    /// sequência de escape.
    trimmed: bool,
    subscribers: HashMap<u64, Outbox>,
    exited: Option<Option<i32>>,
}

/// O filho e se ele já foi colhido, sob o MESMO lock: depois da colheita o
/// pid pode ser de outro processo, e matar por ele mataria um estranho.
struct ChildSlot {
    child: Box<dyn Child + Send + Sync>,
    reaped: bool,
}

struct Pty {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<ChildSlot>,
    pid: Option<u32>,
    size: Mutex<(u16, u16)>,
    output: Mutex<Output>,
}

#[derive(Default)]
pub struct Host {
    ptys: Mutex<HashMap<Uuid, Arc<Pty>>>,
}

pub struct SpawnSpec {
    pub id: Uuid,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

fn not_found(id: Uuid) -> String {
    format!("pty {id} não existe")
}

impl Host {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn(&self, spec: SpawnSpec) -> Result<Option<u32>, String> {
        let Some((program, args)) = spec.argv.split_first() else {
            return Err("spawn sem comando".into());
        };
        if self.ptys.lock().contains_key(&spec.id) {
            return Err(format!("pty {} já existe", spec.id));
        }
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        // O env é o que o app mandou, inteiro. O do daemon não vaza.
        cmd.env_clear();
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }
        let size = PtySize {
            rows: spec.rows,
            cols: spec.cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = portable_pty::native_pty_system()
            .openpty(size)
            .map_err(|e| e.to_string())?;
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let pid = child.process_id();
        let pty = Arc::new(Pty {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(ChildSlot {
                child,
                reaped: false,
            }),
            pid,
            size: Mutex::new((spec.cols, spec.rows)),
            output: Mutex::new(Output {
                ring: VecDeque::new(),
                trimmed: false,
                subscribers: HashMap::new(),
                exited: None,
            }),
        });
        {
            let mut ptys = self.ptys.lock();
            if ptys.contains_key(&spec.id) {
                drop(ptys);
                kill_tree(&pty);
                return Err(format!("pty {} já existe", spec.id));
            }
            ptys.insert(spec.id, Arc::clone(&pty));
        }
        let id = spec.id;
        let spawned = std::thread::Builder::new()
            .name(format!("daemon-pty-{id}"))
            .spawn(move || pump(id, &pty, reader));
        if let Err(e) = spawned {
            if let Some(pty) = self.ptys.lock().remove(&id) {
                kill_tree(&pty);
            }
            return Err(format!("thread de leitura: {e}"));
        }
        Ok(pid)
    }

    fn get(&self, id: Uuid) -> Result<Arc<Pty>, String> {
        self.ptys
            .lock()
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found(id))
    }

    pub fn write(&self, id: Uuid, data: &[u8]) -> Result<(), String> {
        let pty = self.get(id)?;
        let mut writer = pty.writer.lock();
        writer.write_all(data).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())
    }

    pub fn resize(&self, id: Uuid, cols: u16, rows: u16) -> Result<(), String> {
        let pty = self.get(id)?;
        pty.master
            .lock()
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
        *pty.size.lock() = (cols, rows);
        Ok(())
    }

    /// Mata o grupo inteiro (princípio 9) e esquece o PTY. Quem estava ligado
    /// ainda recebe o `Exit` quando a leitura fechar.
    pub fn kill(&self, id: Uuid) -> Result<(), String> {
        let pty = self.ptys.lock().remove(&id).ok_or_else(|| not_found(id))?;
        kill_tree(&pty);
        Ok(())
    }

    /// Todos, para o "Encerrar todas as sessões".
    pub fn kill_all(&self) {
        let all: Vec<_> = self.ptys.lock().drain().map(|(_, pty)| pty).collect();
        for pty in all {
            kill_tree(&pty);
        }
    }

    pub fn list(&self) -> Vec<PtyInfo> {
        let mut out: Vec<PtyInfo> = self
            .ptys
            .lock()
            .iter()
            .map(|(id, pty)| {
                let (cols, rows) = *pty.size.lock();
                PtyInfo {
                    id: *id,
                    pid: pty.pid,
                    cols,
                    rows,
                    exited: pty.output.lock().exited,
                }
            })
            .collect();
        out.sort_by_key(|info| info.id);
        out
    }

    /// Liga `outbox` à saída de `id`. A resposta `Attached` (com o anel) entra
    /// na fila ANTES de qualquer `Output` novo: as duas coisas acontecem sob o
    /// mesmo lock que a leitura usa para repassar.
    pub fn attach(&self, id: Uuid, seq: u64, conn: u64, outbox: &Outbox) -> Result<(), String> {
        let pty = self.get(id)?;
        let mut output = pty.output.lock();
        let ring = ring_from_line_start(&output.ring, output.trimmed);
        let reply = Frame::with_body(
            Reply::Attached {
                seq,
                id,
                exited: output.exited,
            },
            ring,
        );
        // `try_send`: esperar aqui seguraria o lock da saída, e com ele a
        // leitura deste PTY para todo mundo, por causa de um cliente lento.
        outbox
            .try_send(reply)
            .map_err(|_| "conexão lenta ou fechada".to_string())?;
        if output.exited.is_none() {
            output.subscribers.insert(conn, outbox.clone());
        }
        Ok(())
    }

    pub fn detach(&self, id: Uuid, conn: u64) {
        if let Ok(pty) = self.get(id) {
            pty.output.lock().subscribers.remove(&conn);
        }
    }

    /// A conexão caiu: sai de todo PTY em que estava ligada.
    pub fn drop_connection(&self, conn: u64) {
        let all: Vec<_> = self.ptys.lock().values().cloned().collect();
        for pty in all {
            pty.output.lock().subscribers.remove(&conn);
        }
    }

    /// Grupo em primeiro plano do terminal e se o terminal ecoa a linha.
    pub fn query(&self, id: Uuid) -> Result<(Option<u32>, Option<bool>), String> {
        let pty = self.get(id)?;
        let master = pty.master.lock();
        let foreground = master
            .process_group_leader()
            .and_then(|g| u32::try_from(g).ok());
        let echo = master.as_raw_fd().and_then(|fd| {
            let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
            // SAFETY: o fd é o master deste pty, vivo enquanto o lock acima
            // segura o `MasterPty`.
            if unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) } != 0 {
                return None;
            }
            let termios = unsafe { termios.assume_init() };
            Some(termios.c_lflag & libc::ECHO != 0)
        });
        Ok((foreground, echo))
    }

    pub fn is_empty(&self) -> bool {
        self.ptys.lock().is_empty()
    }
}

fn kill_tree(pty: &Pty) {
    // O lock segura a colheita: enquanto ele está preso aqui e `reaped` é
    // falso, o processo é no mínimo um zumbi e o pid ainda é dele.
    let mut slot = pty.child.lock();
    if slot.reaped {
        return;
    }
    if let Some(pid) = pty.pid {
        let _ = crate::pty::kill_process_group(pid);
    }
    let _ = slot.child.kill();
}

/// O anel a partir do começo de uma linha quando ele já perdeu o início:
/// religar no meio de uma sequência de escape pintaria lixo no topo.
fn ring_from_line_start(ring: &VecDeque<u8>, trimmed: bool) -> Vec<u8> {
    let (a, b) = ring.as_slices();
    let mut bytes = Vec::with_capacity(ring.len());
    bytes.extend_from_slice(a);
    bytes.extend_from_slice(b);
    if trimmed {
        if let Some(nl) = bytes.iter().position(|b| *b == b'\n') {
            bytes.drain(..=nl);
        }
    }
    bytes
}

fn push_ring(output: &mut Output, chunk: &[u8]) {
    output.ring.extend(chunk);
    let excess = output.ring.len().saturating_sub(RING_BYTES);
    if excess > 0 {
        output.ring.drain(..excess);
        output.trimmed = true;
    }
}

/// Lê o PTY até fechar. Quem não dá conta da saída é desligado — perder byte
/// no meio deixaria a tela dele errada; religar traz o anel inteiro.
fn pump(id: Uuid, pty: &Pty, mut reader: Box<dyn Read + Send>) {
    let mut buf = [0u8; READ_BUF];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let mut output = pty.output.lock();
        push_ring(&mut output, &buf[..n]);
        output.subscribers.retain(|_, outbox| {
            match outbox.try_send(Frame::with_body(Reply::Output { id }, buf[..n].to_vec())) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
            }
        });
    }
    let code = reap(pty);
    let mut output = pty.output.lock();
    output.exited = Some(code);
    for (_, outbox) in output.subscribers.drain() {
        let _ = outbox.try_send(Frame::new(Reply::Exit { id, code }));
    }
}

/// Colhe o filho sem segurar o lock enquanto espera: `kill` continua podendo
/// entrar (um processo que fechou o terminal e seguiu vivo). A colheita e o
/// `reaped` acontecem juntos, sob o lock.
fn reap(pty: &Pty) -> Option<i32> {
    loop {
        {
            let mut slot = pty.child.lock();
            match slot.child.try_wait() {
                Ok(Some(status)) => {
                    slot.reaped = true;
                    return Some(status.exit_code() as i32);
                }
                Ok(None) => {}
                Err(_) => {
                    slot.reaped = true;
                    return None;
                }
            }
        }
        std::thread::sleep(REAP_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_only_the_most_recent_bytes() {
        let mut output = Output {
            ring: VecDeque::new(),
            trimmed: false,
            subscribers: HashMap::new(),
            exited: None,
        };
        push_ring(&mut output, &vec![b'a'; RING_BYTES]);
        assert!(!output.trimmed);
        push_ring(&mut output, b"xyz");
        assert_eq!(output.ring.len(), RING_BYTES);
        assert!(output.trimmed);
        assert_eq!(
            output
                .ring
                .iter()
                .rev()
                .take(3)
                .copied()
                .collect::<Vec<_>>(),
            b"zyx"
        );
    }

    #[test]
    fn a_trimmed_ring_restarts_at_a_line_start() {
        let ring: VecDeque<u8> = b"[31mmeio\nlinha inteira\n".iter().copied().collect();
        assert_eq!(ring_from_line_start(&ring, true), b"linha inteira\n");
        assert_eq!(
            ring_from_line_start(&ring, false),
            b"[31mmeio\nlinha inteira\n"
        );
    }
}
