use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::Handler;

pub const MAX_INFLIGHT: usize = 32;

struct InflightGuard(Arc<AtomicUsize>);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

// ============================ Unix (AF_UNIX socket) ============================

#[cfg(unix)]
mod imp {
    use super::{Handler, InflightGuard, MAX_INFLIGHT};
    use crate::hook_ipc::framing;
    use std::fs;
    use std::io::BufReader;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread::{self, JoinHandle};

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
                        Ok(stream) => dispatch(stream, &handler, &inflight),
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

    fn dispatch(stream: UnixStream, handler: &Handler, inflight: &Arc<AtomicUsize>) {
        let handler = handler.clone();
        let inflight = inflight.clone();
        if inflight.fetch_add(1, Ordering::SeqCst) >= MAX_INFLIGHT {
            let guard = InflightGuard(inflight);
            thread::spawn(move || {
                let _guard = guard;
                framing::reject_overloaded(stream);
            });
            return;
        }
        thread::spawn(move || {
            let _guard = InflightGuard(inflight);
            serve(stream, handler);
        });
    }

    fn serve(stream: UnixStream, handler: Handler) {
        let Ok(read_half) = stream.try_clone() else {
            return;
        };
        framing::serve_connection(BufReader::new(read_half), stream, &handler);
    }
}

// ============================ Windows (named pipe) ============================

#[cfg(windows)]
mod imp {
    use super::{Handler, InflightGuard, MAX_INFLIGHT};
    use crate::hook_ipc::framing;
    use crate::hook_ipc::pipe;
    use std::io::BufReader;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread::{self, JoinHandle};

    pub struct HookServer {
        name: Vec<u16>,
        shutdown: Arc<AtomicBool>,
        accept_handle: Option<JoinHandle<()>>,
    }

    impl HookServer {
        pub fn bind(socket_path: &Path, handler: Handler) -> std::io::Result<HookServer> {
            let name = pipe::pipe_name(socket_path);
            let shutdown = Arc::new(AtomicBool::new(false));

            let accept_name = name.clone();
            let accept_shutdown = shutdown.clone();
            let inflight = Arc::new(AtomicUsize::new(0));
            let accept_handle = thread::spawn(move || loop {
                if accept_shutdown.load(Ordering::SeqCst) {
                    break;
                }
                let instance = pipe::create_instance(&accept_name);
                if instance == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
                    break;
                }
                let connected = pipe::wait_connect(instance);
                if accept_shutdown.load(Ordering::SeqCst) {
                    pipe::close(instance);
                    break;
                }
                if !connected {
                    pipe::close(instance);
                    continue;
                }
                dispatch(pipe::into_file(instance), &handler, &inflight);
            });

            Ok(HookServer {
                name,
                shutdown,
                accept_handle: Some(accept_handle),
            })
        }

        pub fn shutdown(mut self) {
            self.stop();
        }

        fn stop(&mut self) {
            self.shutdown.store(true, Ordering::SeqCst);
            // desbloqueia o ConnectNamedPipe pendente conectando um cliente efêmero
            let _ = pipe::connect(&self.name);
            if let Some(handle) = self.accept_handle.take() {
                let _ = handle.join();
            }
        }
    }

    impl Drop for HookServer {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn dispatch(stream: pipe::Pipe, handler: &Handler, inflight: &Arc<AtomicUsize>) {
        let handler = handler.clone();
        let inflight = inflight.clone();
        if inflight.fetch_add(1, Ordering::SeqCst) >= MAX_INFLIGHT {
            let guard = InflightGuard(inflight);
            thread::spawn(move || {
                let _guard = guard;
                framing::reject_overloaded(stream);
            });
            return;
        }
        thread::spawn(move || {
            let _guard = InflightGuard(inflight);
            serve(stream, handler);
        });
    }

    fn serve(stream: pipe::Pipe, handler: Handler) {
        let Ok(read_half) = stream.try_clone() else {
            return;
        };
        framing::serve_connection(BufReader::new(read_half), stream, &handler);
    }
}

pub use imp::HookServer;
