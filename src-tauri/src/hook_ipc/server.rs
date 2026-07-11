use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use super::protocol::{hook_event_from_value, RequestEnvelope, ResponseEnvelope, PROTOCOL_VERSION};
use super::{HookAction, HookEvent};

pub type Handler = Arc<dyn Fn(HookEvent) -> HookAction + Send + Sync>;

pub const MAX_INFLIGHT: usize = 32;

const OVERLOADED_REASON: &str =
    "limite de aprovações simultâneas do Tyba atingido — resolva as pendentes e tente de novo";

struct InflightGuard(Arc<AtomicUsize>);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct HookServer {
    socket_path: PathBuf,
    shutdown: Arc<AtomicBool>,
    accept_handle: Option<JoinHandle<()>>,
}

impl HookServer {
    pub fn bind(socket_path: &Path, handler: Handler) -> std::io::Result<HookServer> {
        if socket_path.exists() {
            let _ = fs::remove_file(socket_path);
        }
        let listener = UnixListener::bind(socket_path)?;
        let shutdown = Arc::new(AtomicBool::new(false));

        let accept_shutdown = shutdown.clone();
        let inflight = Arc::new(AtomicUsize::new(0));
        let accept_handle = thread::spawn(move || {
            for stream in listener.incoming() {
                if accept_shutdown.load(Ordering::SeqCst) {
                    break;
                }
                match stream {
                    Ok(stream) => {
                        let handler = handler.clone();
                        let inflight = inflight.clone();
                        if inflight.fetch_add(1, Ordering::SeqCst) >= MAX_INFLIGHT {
                            let guard = InflightGuard(inflight);
                            thread::spawn(move || {
                                let _guard = guard;
                                reject_overloaded(stream);
                            });
                            continue;
                        }
                        thread::spawn(move || {
                            let _guard = InflightGuard(inflight);
                            handle_connection(stream, handler);
                        });
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(HookServer {
            socket_path: socket_path.to_path_buf(),
            shutdown,
            accept_handle: Some(accept_handle),
        })
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(handle) = self.accept_handle.take() {
            let _ = handle.join();
        }
        let _ = fs::remove_file(&self.socket_path);
    }
}

impl Drop for HookServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn write_response(stream: UnixStream, response: &ResponseEnvelope) {
    let Ok(mut payload) = serde_json::to_vec(response) else {
        return;
    };
    payload.push(b'\n');
    let mut writer = stream;
    let _ = writer.write_all(&payload);
    let _ = writer.flush();
}

fn reject_overloaded(stream: UnixStream) {
    write_response(
        stream,
        &ResponseEnvelope {
            v: PROTOCOL_VERSION,
            action: "deny".into(),
            reason: Some(OVERLOADED_REASON.into()),
        },
    );
}

fn handle_connection(stream: UnixStream, handler: Handler) {
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => return,
        Ok(_) => {}
    }

    let Ok(request) = serde_json::from_str::<RequestEnvelope>(line.trim_end()) else {
        return;
    };
    let event = hook_event_from_value(request.event);
    let action = handler(event);

    let response = match action {
        HookAction::Allow { reason } => ResponseEnvelope {
            v: PROTOCOL_VERSION,
            action: "allow".into(),
            reason,
        },
        HookAction::Deny { reason } => ResponseEnvelope {
            v: PROTOCOL_VERSION,
            action: "deny".into(),
            reason: Some(reason),
        },
        HookAction::Ack => ResponseEnvelope {
            v: PROTOCOL_VERSION,
            action: "ack".into(),
            reason: None,
        },
    };

    write_response(stream, &response);
}
