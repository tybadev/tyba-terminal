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

const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
const READ_BUF_SIZE: usize = 8 * 1024;
const SCROLLBACK_LINES: usize = 1000;

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

        let screen: SharedScreen =
            Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK_LINES)));
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
        std::thread::Builder::new()
            .name(format!("pty-reader-{session_id}"))
            .spawn(move || {
                let mut buf = [0u8; READ_BUF_SIZE];
                let mut pending: Vec<u8> = Vec::with_capacity(READ_BUF_SIZE);
                let mut last_flush = Instant::now();

                let flush = |pending: &mut Vec<u8>, app: &AppHandle| {
                    if pending.is_empty() {
                        return;
                    }
                    let data = base64::engine::general_purpose::STANDARD.encode(&pending);
                    let _ = app.emit(&output_event, PtyOutputPayload { data });
                    pending.clear();
                };

                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            reader_screen.lock().process(&buf[..n]);
                            pending.extend_from_slice(&buf[..n]);
                            if last_flush.elapsed() >= FLUSH_INTERVAL {
                                flush(&mut pending, &app);
                                last_flush = Instant::now();
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                flush(&mut pending, &app);
                let _ = app.emit(&exit_event, PtyExitPayload { code: None });
                on_exit();
            })
            .expect("failed to spawn pty reader thread");

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

    pub fn scrollback(&self, id: PtyId) -> Result<Vec<u8>, PtyError> {
        let ptys = self.ptys.lock();
        let handle = ptys.get(&id).ok_or(PtyError::NotFound(id))?;
        let bytes = handle.screen.lock().screen().contents_formatted();
        Ok(bytes)
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
