mod client;
mod framing;
#[cfg(windows)]
mod pipe;
mod protocol;
mod server;

use std::path::PathBuf;
use std::sync::Arc;

pub use client::{request, run_client};
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
    Allow { reason: Option<String> },
    Deny { reason: String },
    Ack,
}

pub fn maybe_run_hook_mode() -> Option<i32> {
    if std::env::args().nth(1).as_deref() != Some("_hook") {
        return None;
    }
    let socket = std::env::var_os("TYBA_HOOK_SOCKET").map(PathBuf::from);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let code = run_client(stdin.lock(), stdout.lock(), socket.as_deref());
    Some(code)
}

#[cfg(all(test, unix))]
mod tests;
