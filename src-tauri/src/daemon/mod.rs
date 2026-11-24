//! `tyba _daemon`: o processo que segura os PTYs para eles sobreviverem ao
//! app fechado. Só o dono dos terminais — sessões, status, hooks, blocos,
//! guarda-corpo e SQLite continuam no app. Plano em
//! `swell-docs/tyba/features/daemon/daemon.md`.
//!
//! Nesta fatia (D1) o daemon existe e é testado sozinho; o app ainda não fala
//! com ele.

pub mod endpoint;
pub mod host;
pub mod protocol;
pub mod server;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

const DAEMON_MODE: &str = "_daemon";
const SOCKET_FILE: &str = "daemon.sock";

/// Na mesma pasta privada do canal e da CLI (`tyba` ou `tyba-dev`, por
/// usuário): o app instalado e o build de dev têm daemons separados.
pub fn resolve_daemon_socket_path() -> PathBuf {
    crate::hook_ipc::channel::resolve_channel_socket_path().with_file_name(SOCKET_FILE)
}

/// `tyba _daemon` no `main`, antes do app.
pub fn maybe_run_daemon_mode() -> Option<i32> {
    if std::env::args().nth(1).as_deref() != Some(DAEMON_MODE) {
        return None;
    }
    Some(run(resolve_daemon_socket_path()))
}

fn run(socket: PathBuf) -> i32 {
    let published = match endpoint::publish(&socket) {
        Ok(published) => published,
        // Outro daemon já serve: quem pediu este usa aquele.
        Err(endpoint::PublishError::AlreadyServing) => return 0,
        Err(e) => {
            eprintln!("tyba _daemon: {e}");
            return 1;
        }
    };
    let server = server::Server::new(Arc::new(host::Host::new()));
    // `published` (listener e trava) vive enquanto o daemon serve.
    server.serve(&published.listener);
    0
}
