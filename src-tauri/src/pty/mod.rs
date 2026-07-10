use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use parking_lot::Mutex;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

pub const EVENT_CWD_CHANGED: &str = "session://cwd-changed";

const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
const READ_BUF_SIZE: usize = 8 * 1024;
const SCROLLBACK_LINES: usize = 1000;
const CHANNEL_CAPACITY: usize = 128;

pub type PtyId = Uuid;

struct ScreenState {
    parser: vt100::Parser,
    pending: Vec<u8>,
    attachers: usize,
}

impl ScreenState {
    fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, SCROLLBACK_LINES),
            pending: Vec::with_capacity(READ_BUF_SIZE),
            attachers: 0,
        }
    }

    fn take_pending(&mut self) -> Option<Vec<u8>> {
        if self.attachers == 0 || self.pending.is_empty() {
            self.pending.clear();
            return None;
        }
        Some(std::mem::replace(
            &mut self.pending,
            Vec::with_capacity(READ_BUF_SIZE),
        ))
    }
}

type SharedScreen = Arc<Mutex<ScreenState>>;

fn emit_pending(state: &mut ScreenState, app: &AppHandle, event: &str) {
    if let Some(bytes) = state.take_pending() {
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let _ = app.emit(event, PtyOutputPayload { data });
    }
}

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
    pub agent_match: bool,
}

/// Diretório de trabalho reportado via `OSC 7`.
///
/// Atacante-controlável: qualquer processo que escreva no tty pode forjar.
/// Uso exclusivo de exibição — nunca embasa decisão de segurança.
#[derive(Clone, serde::Serialize)]
pub struct SessionCwdPayload {
    pub cwd: String,
}

#[derive(Clone, Serialize)]
pub struct SessionBracketedPayload {
    pub bracketed_paste: bool,
}

struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    leader_pid: Option<u32>,
    screen: SharedScreen,
}

#[derive(Default)]
pub struct PtyPool {
    ptys: Mutex<HashMap<PtyId, PtyHandle>>,
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

        let screen: SharedScreen = Arc::new(Mutex::new(ScreenState::new(rows, cols)));
        let reader_screen = Arc::clone(&screen);

        self.ptys.lock().insert(
            session_id,
            PtyHandle {
                master: pair.master,
                writer,
                child,
                leader_pid,
                screen,
            },
        );

        let output_event = format!("pty://output/{session_id}");
        let exit_event = format!("pty://exit/{session_id}");
        let command_event = format!("session://command/{session_id}");
        let cwd_event = format!("session://cwd/{session_id}");
        let bracketed_event = format!("session://bracketed/{session_id}");
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
                let mut queued = false;
                let mut last_flush = Instant::now();
                let mut osc = crate::status::OscParser::new();
                let mut last_cmd: Option<String> = None;
                let mut last_match = false;
                let mut last_cwd: Option<std::path::PathBuf> = None;
                let mut last_bracketed = false;

                loop {
                    let chunk = if !queued {
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
                            let due = last_flush.elapsed() >= FLUSH_INTERVAL;
                            let bracketed = {
                                let mut screen = reader_screen.lock();
                                screen.parser.process(&chunk);
                                if screen.attachers > 0 {
                                    screen.pending.extend_from_slice(&chunk);
                                }
                                if due {
                                    emit_pending(&mut screen, &app, &output_event);
                                }
                                queued = !screen.pending.is_empty();
                                screen.parser.screen().bracketed_paste()
                            };
                            if due {
                                last_flush = Instant::now();
                            }
                            if bracketed != last_bracketed {
                                last_bracketed = bracketed;
                                let _ = app.emit(
                                    &bracketed_event,
                                    SessionBracketedPayload {
                                        bracketed_paste: bracketed,
                                    },
                                );
                            }
                            for ev in osc.feed(&chunk) {
                                use crate::status::ShellEvent;
                                match ev {
                                    ShellEvent::CommandLine(cmd) => {
                                        last_match =
                                            crate::rich_input::agent_matcher().matches(&cmd);
                                        last_cmd = Some(cmd);
                                    }
                                    ShellEvent::CommandStart => {
                                        let _ = app.emit(
                                            &command_event,
                                            SessionCommandPayload {
                                                command: last_cmd.clone(),
                                                running: true,
                                                agent_match: last_match,
                                            },
                                        );
                                    }
                                    ShellEvent::CommandEnd(_) | ShellEvent::PromptStart => {
                                        last_cmd = None;
                                        last_match = false;
                                        let _ = app.emit(
                                            &command_event,
                                            SessionCommandPayload {
                                                command: None,
                                                running: false,
                                                agent_match: false,
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
                                            let _ = app.emit(EVENT_CWD_CHANGED, session_id);
                                        }
                                    }
                                }
                            }
                        }
                        None => {
                            {
                                let mut screen = reader_screen.lock();
                                emit_pending(&mut screen, &app, &output_event);
                                queued = false;
                            }
                            last_flush = Instant::now();
                        }
                    }
                }
                {
                    let mut screen = reader_screen.lock();
                    emit_pending(&mut screen, &app, &output_event);
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
        handle.screen.lock().parser.set_size(rows, cols);
        Ok(())
    }

    fn screen_of(&self, id: PtyId) -> Option<SharedScreen> {
        Some(Arc::clone(&self.ptys.lock().get(&id)?.screen))
    }

    pub fn attach(&self, app: &AppHandle, window: &str, id: PtyId) -> Result<(), PtyError> {
        let screen = self.screen_of(id).ok_or(PtyError::NotFound(id))?;
        let event = format!("pty://output/{id}");
        let mut state = screen.lock();

        emit_pending(&mut state, app, &event);

        let snapshot = state.parser.screen().contents_formatted();
        if !snapshot.is_empty() {
            let data = base64::engine::general_purpose::STANDARD.encode(&snapshot);
            let _ = app.emit_to(window, &event, PtyOutputPayload { data });
        }
        state.attachers += 1;
        Ok(())
    }

    pub fn detach(&self, id: PtyId) {
        let Some(screen) = self.screen_of(id) else {
            return;
        };
        let mut state = screen.lock();
        state.attachers = state.attachers.saturating_sub(1);
        if state.attachers == 0 {
            state.pending.clear();
        }
    }

    pub fn bracketed_paste(&self, id: PtyId) -> Option<bool> {
        let screen = self.screen_of(id)?;
        let enabled = screen.lock().parser.screen().bracketed_paste();
        Some(enabled)
    }

    pub fn scrollback_text(&self, id: PtyId) -> Result<String, PtyError> {
        let ptys = self.ptys.lock();
        let handle = ptys.get(&id).ok_or(PtyError::NotFound(id))?;
        let text = handle.screen.lock().parser.screen().contents();
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

    pub fn leader_pid(&self, id: PtyId) -> Option<u32> {
        self.ptys.lock().get(&id)?.leader_pid
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
mod screen_state_tests {
    use super::ScreenState;

    #[test]
    fn detached_screen_never_queues_bytes() {
        let mut state = ScreenState::new(24, 80);
        state.parser.process(b"hello");
        assert!(state.take_pending().is_none());
    }

    #[test]
    fn bytes_queued_while_detached_are_discarded() {
        let mut state = ScreenState::new(24, 80);
        state.pending.extend_from_slice(b"stale");
        assert!(state.take_pending().is_none());
        assert!(state.pending.is_empty());
    }

    #[test]
    fn attached_screen_hands_over_queued_bytes_once() {
        let mut state = ScreenState::new(24, 80);
        state.attachers = 1;
        state.pending.extend_from_slice(b"live");
        assert_eq!(state.take_pending().as_deref(), Some(&b"live"[..]));
        assert!(state.take_pending().is_none());
    }

    #[test]
    fn taking_pending_keeps_the_buffer_reusable() {
        let mut state = ScreenState::new(24, 80);
        state.attachers = 1;
        state.pending.extend_from_slice(b"a");
        state.take_pending();
        state.pending.extend_from_slice(b"b");
        assert_eq!(state.take_pending().as_deref(), Some(&b"b"[..]));
    }

    #[test]
    fn a_second_attacher_keeps_the_stream_alive_after_the_first_detaches() {
        let mut state = ScreenState::new(24, 80);
        state.attachers = 2;
        state.attachers = state.attachers.saturating_sub(1);
        state.pending.extend_from_slice(b"still live");
        assert_eq!(state.take_pending().as_deref(), Some(&b"still live"[..]));
    }

    #[test]
    fn detaching_below_zero_saturates() {
        let mut state = ScreenState::new(24, 80);
        state.attachers = state.attachers.saturating_sub(1);
        assert_eq!(state.attachers, 0);
    }
}

#[cfg(test)]
mod screen_tests {
    #[test]
    fn parser_rastreia_o_estado_de_bracketed_paste_do_programa() {
        let mut parser = vt100::Parser::new(24, 80, super::SCROLLBACK_LINES);
        assert!(!parser.screen().bracketed_paste());
        parser.process(b"\x1b[?2004h");
        assert!(parser.screen().bracketed_paste());
        parser.process(b"\x1b[?2004l");
        assert!(!parser.screen().bracketed_paste());
    }

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
