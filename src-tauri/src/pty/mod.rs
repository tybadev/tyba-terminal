//! PTY pool: spawn, I/O e batching de output.
//!
//! Princípio #3 do CLAUDE.md: NUNCA um evento IPC por chunk de PTY.
//! O reader thread acumula bytes e flusha para o webview a cada
//! `FLUSH_INTERVAL` (~1 frame), via evento `pty://output/{session_id}`.
//! Payload em base64 porque chunks de PTY podem quebrar UTF-8 no meio
//! de uma sequência — o frontend decodifica para Uint8Array e passa
//! direto ao xterm.js.

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

pub type PtyId = Uuid;

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
    /// Bytes do PTY em base64 (decodificar no frontend antes do xterm.write).
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
}

/// Pool de PTYs vivos, indexados por id de sessão.
#[derive(Default)]
pub struct PtyPool {
    ptys: Mutex<HashMap<PtyId, PtyHandle>>,
}

impl PtyPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawna `cmd` num PTY novo e inicia o reader thread com batching.
    ///
    /// `env` é a allowlist já filtrada pelo chamador (princípio #6:
    /// sessões de agente NUNCA herdam o env completo do usuário —
    /// o filtro acontece antes de chegar aqui).
    pub fn spawn(
        &self,
        app: AppHandle,
        session_id: PtyId,
        mut cmd: CommandBuilder,
        env: Option<&HashMap<String, String>>,
        cols: u16,
        rows: u16,
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

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Open(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Open(e.to_string()))?;

        self.ptys.lock().insert(
            session_id,
            PtyHandle {
                master: pair.master,
                writer,
                child,
            },
        );

        // Reader thread: acumula e flusha a cada FLUSH_INTERVAL.
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
                        Ok(0) => break, // EOF: processo saiu
                        Ok(n) => {
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
        Ok(())
    }

    /// Mata a sessão inteira. Princípio #9: process group, não só o pai.
    /// `portable_pty::Child::kill` no Unix envia sinal ao processo líder
    /// do PTY; como spawmamos o shell como líder de sessão do PTY, o
    /// SIGHUP na destruição do master derruba o grupo. Para agentes que
    /// ignoram SIGHUP, o killpg explícito entra na trait Sandbox (fase 5).
    pub fn kill(&self, id: PtyId) -> Result<(), PtyError> {
        let mut ptys = self.ptys.lock();
        let mut handle = ptys.remove(&id).ok_or(PtyError::NotFound(id))?;
        let _ = handle.child.kill();
        Ok(())
    }

    pub fn is_alive(&self, id: PtyId) -> bool {
        self.ptys.lock().contains_key(&id)
    }
}

pub type SharedPtyPool = Arc<PtyPool>;
