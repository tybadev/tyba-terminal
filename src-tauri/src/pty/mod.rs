use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use parking_lot::Mutex;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
const READ_BUF_SIZE: usize = 8 * 1024;
const SCROLLBACK_LINES: usize = 1000;
const CHANNEL_CAPACITY: usize = 128;

pub type PtyId = Uuid;

type SharedScreen = Arc<Mutex<vt100::Parser>>;

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("failed to open pty: {0}")]
    Open(String),
    #[error("failed to spawn command: {0}")]
    Spawn(String),
    #[error("pty not found: {0}")]
    NotFound(PtyId),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Serialize)]
pub struct PtyOutputPayload {
    pub data: String,
}

#[derive(Clone, Serialize)]
pub struct PtyExitPayload {
    pub code: Option<u32>,
}

#[derive(Clone, Serialize)]
pub struct SessionCommandPayload {
    /// Linha de comando em execução (shell integration), ou `None` quando ocioso.
    pub command: Option<String>,
    pub running: bool,
}

/// Diretório de trabalho reportado via `OSC 7`.
///
/// Atacante-controlável: qualquer processo que escreva no tty pode forjar.
/// Uso exclusivo de exibição — nunca embasa decisão de segurança.
#[derive(Clone, serde::Serialize)]
pub struct SessionCwdPayload {
    pub cwd: String,
}

#[derive(Debug, PartialEq, Eq)]
struct GateDecision {
    reset: bool,
    attached: bool,
}

#[derive(Default)]
struct EmitterGate {
    generation: u64,
}

impl EmitterGate {
    fn observe(&mut self, generation: u64) -> GateDecision {
        let reset = generation != self.generation;
        self.generation = generation;
        GateDecision {
            reset,
            attached: generation != 0,
        }
    }
}

struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    leader_pid: Option<u32>,
    screen: SharedScreen,
    generation: Arc<AtomicU64>,
}

#[derive(Default)]
pub struct PtyPool {
    ptys: Mutex<HashMap<PtyId, PtyHandle>>,
    next_generation: AtomicU64,
}

impl PtyPool {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &self,
        app: AppHandle,
        session_id: PtyId,
        mut cmd: CommandBuilder,
        env: Option<&HashMap<String, String>>,
        cols: u16,
        rows: u16,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<(), PtyError> {
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Open(e.to_string()))?;

        if let Some(env) = env {
            cmd.env_clear();
            for (k, v) in env {
                cmd.env(k, v);
            }
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Spawn(e.to_string()))?;
        drop(pair.slave);

        let leader_pid = child.process_id();

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Open(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Open(e.to_string()))?;

        let screen: SharedScreen =
            Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK_LINES)));
        let reader_screen = Arc::clone(&screen);
        let generation = Arc::new(AtomicU64::new(0));
        let emitter_generation = Arc::clone(&generation);

        self.ptys.lock().insert(
            session_id,
            PtyHandle {
                master: pair.master,
                writer,
                child,
                leader_pid,
                screen,
                generation,
            },
        );

        let output_event = format!("pty://output/{session_id}");
        let exit_event = format!("pty://exit/{session_id}");
        let command_event = format!("session://command/{session_id}");
        let cwd_event = format!("session://cwd/{session_id}");
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(CHANNEL_CAPACITY);

        std::thread::Builder::new()
            .name(format!("pty-reader-{session_id}"))
            .spawn(move || {
                let mut buf = [0u8; READ_BUF_SIZE];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| {
                let _ = self.kill(session_id);
                PtyError::Spawn(format!("pty reader thread: {e}"))
            })?;

        std::thread::Builder::new()
            .name(format!("pty-emitter-{session_id}"))
            .spawn(move || {
                let mut pending: Vec<u8> = Vec::with_capacity(READ_BUF_SIZE);
                let mut last_flush = Instant::now();
                let mut osc = crate::status::OscParser::new();
                let mut last_cmd: Option<String> = None;
                let mut last_cwd: Option<std::path::PathBuf> = None;
                let mut gate = EmitterGate::default();

                let flush = |pending: &mut Vec<u8>, app: &AppHandle| {
                    if pending.is_empty() {
                        return;
                    }
                    let data = base64::engine::general_purpose::STANDARD.encode(&pending);
                    let _ = app.emit(&output_event, PtyOutputPayload { data });
                    pending.clear();
                };

                loop {
                    let chunk = if pending.is_empty() {
                        match rx.recv() {
                            Ok(chunk) => Some(chunk),
                            Err(_) => break,
                        }
                    } else {
                        match rx.recv_timeout(FLUSH_INTERVAL) {
                            Ok(chunk) => Some(chunk),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    };
                    match chunk {
                        Some(chunk) => {
                            let decision = {
                                let mut screen = reader_screen.lock();
                                screen.process(&chunk);
                                gate.observe(emitter_generation.load(Ordering::Relaxed))
                            };
                            if decision.reset {
                                pending.clear();
                            }
                            for ev in osc.feed(&chunk) {
                                use crate::status::ShellEvent;
                                match ev {
                                    ShellEvent::CommandLine(cmd) => last_cmd = Some(cmd),
                                    ShellEvent::CommandStart => {
                                        let _ = app.emit(
                                            &command_event,
                                            SessionCommandPayload {
                                                command: last_cmd.clone(),
                                                running: true,
                                            },
                                        );
                                    }
                                    ShellEvent::CommandEnd(_) | ShellEvent::PromptStart => {
                                        last_cmd = None;
                                        let _ = app.emit(
                                            &command_event,
                                            SessionCommandPayload {
                                                command: None,
                                                running: false,
                                            },
                                        );
                                    }
                                    ShellEvent::InputStart => {}
                                    ShellEvent::Cwd(path) => {
                                        if last_cwd.as_deref() != Some(path.as_path()) {
                                            last_cwd = Some(path.clone());
                                            let _ = app.emit(
                                                &cwd_event,
                                                SessionCwdPayload {
                                                    cwd: path.to_string_lossy().into_owned(),
                                                },
                                            );
                                        }
                                    }
                                }
                            }
                            if decision.attached {
                                pending.extend_from_slice(&chunk);
                                if last_flush.elapsed() >= FLUSH_INTERVAL {
                                    flush(&mut pending, &app);
                                    last_flush = Instant::now();
                                }
                            }
                        }
                        None => {
                            let decision = gate.observe(emitter_generation.load(Ordering::Relaxed));
                            if decision.reset {
                                pending.clear();
                            }
                            if decision.attached {
                                flush(&mut pending, &app);
                            }
                            last_flush = Instant::now();
                        }
                    }
                }
                let decision = gate.observe(emitter_generation.load(Ordering::Relaxed));
                if decision.reset {
                    pending.clear();
                }
                if decision.attached {
                    flush(&mut pending, &app);
                }
                let _ = app.emit(&exit_event, PtyExitPayload { code: None });
                on_exit();
            })
            .map_err(|e| {
                let _ = self.kill(session_id);
                PtyError::Spawn(format!("pty emitter thread: {e}"))
            })?;

        Ok(())
    }

    pub fn write(&self, id: PtyId, data: &[u8]) -> Result<(), PtyError> {
        let mut ptys = self.ptys.lock();
        let handle = ptys.get_mut(&id).ok_or(PtyError::NotFound(id))?;
        handle.writer.write_all(data)?;
        handle.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, id: PtyId, cols: u16, rows: u16) -> Result<(), PtyError> {
        let ptys = self.ptys.lock();
        let handle = ptys.get(&id).ok_or(PtyError::NotFound(id))?;
        handle
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Open(e.to_string()))?;
        handle.screen.lock().set_size(rows, cols);
        Ok(())
    }

    pub fn attach(&self, app: &AppHandle, id: PtyId) -> Result<(), PtyError> {
        let ptys = self.ptys.lock();
        let handle = ptys.get(&id).ok_or(PtyError::NotFound(id))?;
        let screen = handle.screen.lock();
        let snapshot = screen.screen().contents_formatted();
        if !snapshot.is_empty() {
            let data = base64::engine::general_purpose::STANDARD.encode(&snapshot);
            let _ = app.emit(&format!("pty://output/{id}"), PtyOutputPayload { data });
        }
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed) + 1;
        handle.generation.store(generation, Ordering::Relaxed);
        Ok(())
    }

    pub fn detach(&self, id: PtyId) {
        if let Some(handle) = self.ptys.lock().get(&id) {
            handle.generation.store(0, Ordering::Relaxed);
        }
    }

    pub fn scrollback_text(&self, id: PtyId) -> Result<String, PtyError> {
        let ptys = self.ptys.lock();
        let handle = ptys.get(&id).ok_or(PtyError::NotFound(id))?;
        let text = handle.screen.lock().screen().contents();
        Ok(text)
    }

    pub fn kill(&self, id: PtyId) -> Result<(), PtyError> {
        let mut handle = {
            let mut ptys = self.ptys.lock();
            ptys.remove(&id).ok_or(PtyError::NotFound(id))?
        };
        if let Some(pid) = handle.leader_pid {
            let _ = kill_process_group(pid);
        }
        let _ = handle.child.kill();
        Ok(())
    }

    pub fn is_alive(&self, id: PtyId) -> bool {
        self.ptys.lock().contains_key(&id)
    }
}

#[cfg(unix)]
fn kill_process_group(leader_pid: u32) -> std::io::Result<()> {
    let pgid = unsafe { libc::getpgid(leader_pid as libc::pid_t) };
    if pgid < 0 {
        let err = std::io::Error::last_os_error();
        return match err.raw_os_error() {
            Some(libc::ESRCH) => Ok(()),
            _ => Err(err),
        };
    }
    let rc = unsafe { libc::killpg(pgid, libc::SIGKILL) };
    if rc == -1 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            return Err(err);
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn kill_process_group(_leader_pid: u32) -> std::io::Result<()> {
    Ok(())
}

pub type SharedPtyPool = Arc<PtyPool>;

#[cfg(test)]
mod gate_tests {
    use super::EmitterGate;

    #[test]
    fn detached_pty_never_enqueues_output() {
        let mut gate = EmitterGate::default();
        let d = gate.observe(0);
        assert!(!d.attached);
        assert!(!d.reset);
    }

    #[test]
    fn attach_resets_pending_and_starts_emitting() {
        let mut gate = EmitterGate::default();
        let d = gate.observe(1);
        assert!(d.attached);
        assert!(d.reset);
    }

    #[test]
    fn steady_attachment_does_not_reset() {
        let mut gate = EmitterGate::default();
        gate.observe(1);
        let d = gate.observe(1);
        assert!(d.attached);
        assert!(!d.reset);
    }

    #[test]
    fn detach_discards_pending_and_stops_emitting() {
        let mut gate = EmitterGate::default();
        gate.observe(1);
        let d = gate.observe(0);
        assert!(!d.attached);
        assert!(d.reset);
    }

    #[test]
    fn reattach_with_new_generation_discards_bytes_covered_by_the_new_snapshot() {
        let mut gate = EmitterGate::default();
        gate.observe(1);
        gate.observe(0);
        let d = gate.observe(2);
        assert!(d.attached);
        assert!(d.reset);
    }

    #[test]
    fn reattach_without_an_intervening_detach_still_resets() {
        let mut gate = EmitterGate::default();
        gate.observe(1);
        let d = gate.observe(2);
        assert!(d.attached);
        assert!(d.reset);
    }
}

#[cfg(test)]
mod screen_tests {
    #[test]
    fn snapshot_preserves_visible_text() {
        let mut parser = vt100::Parser::new(24, 80, super::SCROLLBACK_LINES);
        parser.process(b"hello \x1b[31mred\x1b[0m world");
        let dump = parser.screen().contents_formatted();
        let text = String::from_utf8_lossy(&dump);
        assert!(text.contains("hello"));
        assert!(text.contains("red"));
        assert!(text.contains("world"));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::kill_process_group;
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn is_alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    fn wait_dead(pid: i32, within: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < within {
            if !is_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        !is_alive(pid)
    }

    #[test]
    fn kills_entire_process_group() {
        let mut leader = unsafe {
            Command::new("sh")
                .arg("-c")
                .arg("sleep 60 & echo $$ $!; wait")
                .stdout(Stdio::piped())
                .pre_exec(|| {
                    libc::setsid();
                    Ok(())
                })
                .spawn()
                .expect("spawn leader")
        };

        let stdout = leader.stdout.take().expect("piped stdout");
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("read pids");

        let mut ids = line.split_whitespace();
        let leader_pid: i32 = ids.next().unwrap().parse().unwrap();
        let child_pid: i32 = ids.next().unwrap().parse().unwrap();

        assert!(is_alive(leader_pid));
        assert!(is_alive(child_pid));

        kill_process_group(leader_pid as u32).expect("kill group");

        assert!(
            wait_dead(child_pid, Duration::from_secs(2)),
            "child survived group kill"
        );

        let status = leader.wait().expect("reap leader");
        assert_eq!(
            status.signal(),
            Some(libc::SIGKILL),
            "leader not killed by SIGKILL"
        );
    }
}
