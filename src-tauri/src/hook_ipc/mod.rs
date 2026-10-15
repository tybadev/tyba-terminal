pub mod channel;
mod client;
mod framing;
// O canal shim↔core (shim v2) existe no Linux (SO_PEERCRED) e, desde o M3 do
// claude-monitor, no macOS (getpeereid + LOCAL_PEERPID). Windows não tem shim.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod peercred;
#[cfg(windows)]
mod pipe;
mod protocol;
mod server;

use std::path::PathBuf;
use std::sync::Arc;

pub use client::{request, run_client, run_guard_only_client, run_observer_client};
#[cfg(test)]
pub(crate) use protocol::hook_event_from_value;
pub use protocol::ResponseEnvelope;
pub use server::HookServer;

pub type Handler = Arc<dyn Fn(HookEvent) -> HookAction + Send + Sync>;

pub struct HookEvent {
    pub hook_event_name: String,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub notification_type: Option<String>,
    pub cwd: Option<String>,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    pub raw: serde_json::Value,
}

pub enum HookAction {
    Allow {
        reason: Option<String>,
    },
    Deny {
        reason: String,
    },
    Ack,
    /// Sem decisão: o cliente sai calado e valem as regras do próprio agente
    /// (amarelo do Claude no fluxo híbrido).
    Pass,
    /// Força a pergunta nativa do agente, com o motivo (amarelo do Claude
    /// quando a config dele dispensaria a pergunta).
    Ask {
        reason: String,
    },
}

/// `tyba _hook --observe`: o hook do claude digitado num pane (agente livre).
/// Ver [`client::run_observer_client`].
pub const OBSERVE_FLAG: &str = "--observe";

/// `tyba _hook --guard-only`: o hook do claude aberto de dentro do claude
/// hospedado. Só o guarda-corpo, nunca o socket. Ver
/// [`client::run_guard_only_client`].
pub const GUARD_ONLY_FLAG: &str = "--guard-only";

pub fn maybe_run_hook_mode() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("_hook") {
        return None;
    }
    let mode = args.next();
    let socket = || std::env::var_os("TYBA_HOOK_SOCKET").map(PathBuf::from);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let guard = &crate::agent::claude_plugin::guard_rail_output;
    // Windows não tem shim nem claude digitado com hook do TYBA: sem o
    // guarda-corpo (o módulo é só unix), o modo `_hook` não recusa nada.
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let guard = &|_: &serde_json::Value| -> Option<String> { None };
    let code = match mode.as_deref() {
        Some(GUARD_ONLY_FLAG) => run_guard_only_client(stdin.lock(), stdout.lock(), guard),
        Some(OBSERVE_FLAG) => {
            run_observer_client(stdin.lock(), stdout.lock(), socket().as_deref(), guard)
        }
        _ => run_client(stdin.lock(), stdout.lock(), socket().as_deref()),
    };
    Some(code)
}

#[cfg(all(test, unix))]
mod tests;
