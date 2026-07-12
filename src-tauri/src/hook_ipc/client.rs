use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;
use std::time::Duration;

use serde::Serialize;

use super::protocol::{RequestEnvelope, ResponseEnvelope, PROTOCOL_VERSION};

const PRE_TOOL_USE: &str = "PreToolUse";
const PERMISSION_REQUEST: &str = "PermissionRequest";
const FAIL_CLOSED_REASON: &str = "TYBA: transporte de hook indisponível — negado (fail-closed).";
const DENY_FALLBACK_REASON: &str = "negado no TYBA";

#[derive(Clone, Copy, PartialEq, Eq)]
enum EventKind {
    PreToolUse,
    PermissionRequest,
    Other,
}

#[derive(Clone, Copy)]
pub struct RetryPlan {
    pub attempts: u32,
    pub base: Duration,
}

impl Default for RetryPlan {
    fn default() -> Self {
        RetryPlan {
            attempts: 5,
            base: Duration::from_millis(100),
        }
    }
}

#[derive(Serialize)]
struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "permissionDecision")]
    permission_decision: &'static str,
    #[serde(rename = "permissionDecisionReason")]
    permission_decision_reason: String,
    /// O Codex trata `permissionDecision: allow` sem `updatedInput` como
    /// unsupported: o hook vira Failed e a chamada cai no fluxo de permissão
    /// nativo (prompt do TUI). Ecoar o input original torna o allow válido e
    /// mantém a inbox do Tyba como única superfície de decisão.
    #[serde(rename = "updatedInput", skip_serializing_if = "Option::is_none")]
    updated_input: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct PreToolUseOutput {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecificOutput,
}

#[derive(Serialize)]
struct PermissionDecision {
    behavior: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Serialize)]
struct PermissionRequestSpecificOutput {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    decision: PermissionDecision,
}

#[derive(Serialize)]
struct PermissionRequestOutput {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: PermissionRequestSpecificOutput,
}

pub fn run_client<R: Read, W: Write>(stdin: R, stdout: W, socket_path: Option<&Path>) -> i32 {
    run_client_inner(stdin, stdout, socket_path, RetryPlan::default())
}

pub fn run_client_inner<R: Read, W: Write>(
    mut stdin: R,
    mut stdout: W,
    socket_path: Option<&Path>,
    retry: RetryPlan,
) -> i32 {
    let mut buf = Vec::new();
    if stdin.read_to_end(&mut buf).is_err() {
        return 0;
    }
    let event = match serde_json::from_slice::<serde_json::Value>(&buf) {
        Ok(value) if value.is_object() => value,
        _ => return 0,
    };
    let kind = match event.get("hook_event_name").and_then(|v| v.as_str()) {
        Some(PRE_TOOL_USE) => EventKind::PreToolUse,
        Some(PERMISSION_REQUEST) => EventKind::PermissionRequest,
        _ => EventKind::Other,
    };

    let Some(socket_path) = socket_path else {
        emit_transport_failure(&mut stdout, kind);
        return 0;
    };

    let Some(stream) = connect_with_retry(socket_path, retry) else {
        emit_transport_failure(&mut stdout, kind);
        return 0;
    };

    match exchange(stream, &event) {
        Some(response) => emit_response(&mut stdout, &response, kind, event.get("tool_input")),
        None => emit_transport_failure(&mut stdout, kind),
    }
    0
}

fn emit_transport_failure<W: Write>(stdout: &mut W, kind: EventKind) {
    if kind == EventKind::PreToolUse {
        emit_pre_tool_use(stdout, "deny", FAIL_CLOSED_REASON);
    }
}

fn connect_with_retry(socket_path: &Path, retry: RetryPlan) -> Option<UnixStream> {
    for attempt in 0..retry.attempts {
        if let Ok(stream) = UnixStream::connect(socket_path) {
            return Some(stream);
        }
        let backoff = retry.base * 2u32.pow(attempt);
        thread::sleep(backoff);
    }
    None
}

fn exchange(mut stream: UnixStream, event: &serde_json::Value) -> Option<ResponseEnvelope> {
    let request = RequestEnvelope {
        v: PROTOCOL_VERSION,
        event: event.clone(),
    };
    let mut payload = serde_json::to_vec(&request).ok()?;
    payload.push(b'\n');
    stream.write_all(&payload).ok()?;
    stream.flush().ok()?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => return None,
        Ok(_) => {}
    }
    serde_json::from_str::<ResponseEnvelope>(line.trim_end()).ok()
}

fn emit_response<W: Write>(
    stdout: &mut W,
    response: &ResponseEnvelope,
    kind: EventKind,
    tool_input: Option<&serde_json::Value>,
) {
    let reason = response.reason.as_deref().filter(|r| !r.trim().is_empty());
    match (response.action.as_str(), kind) {
        ("allow", EventKind::PreToolUse) => {
            emit_pre_tool_use_allow(stdout, reason.unwrap_or(""), tool_input.cloned())
        }
        ("deny", EventKind::PreToolUse) => {
            emit_pre_tool_use(stdout, "deny", reason.unwrap_or(DENY_FALLBACK_REASON))
        }
        ("allow", EventKind::PermissionRequest) => {
            emit_permission_request(stdout, "allow", reason.map(str::to_string))
        }
        ("deny", EventKind::PermissionRequest) => emit_permission_request(
            stdout,
            "deny",
            Some(reason.unwrap_or(DENY_FALLBACK_REASON).to_string()),
        ),
        ("ack", _) | ("allow", EventKind::Other) | ("deny", EventKind::Other) => {}
        (_, kind) => emit_transport_failure(stdout, kind),
    }
}

fn emit_permission_request<W: Write>(
    stdout: &mut W,
    behavior: &'static str,
    message: Option<String>,
) {
    let output = PermissionRequestOutput {
        hook_specific_output: PermissionRequestSpecificOutput {
            hook_event_name: PERMISSION_REQUEST,
            decision: PermissionDecision { behavior, message },
        },
    };
    if let Ok(mut bytes) = serde_json::to_vec(&output) {
        bytes.push(b'\n');
        let _ = stdout.write_all(&bytes);
        let _ = stdout.flush();
    }
}

fn emit_pre_tool_use<W: Write>(stdout: &mut W, decision: &'static str, reason: &str) {
    write_pre_tool_use(stdout, decision, reason, None);
}

fn emit_pre_tool_use_allow<W: Write>(
    stdout: &mut W,
    reason: &str,
    tool_input: Option<serde_json::Value>,
) {
    write_pre_tool_use(stdout, "allow", reason, tool_input);
}

fn write_pre_tool_use<W: Write>(
    stdout: &mut W,
    decision: &'static str,
    reason: &str,
    updated_input: Option<serde_json::Value>,
) {
    let output = PreToolUseOutput {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: PRE_TOOL_USE,
            permission_decision: decision,
            permission_decision_reason: reason.to_string(),
            updated_input,
        },
    };
    if let Ok(mut bytes) = serde_json::to_vec(&output) {
        bytes.push(b'\n');
        let _ = stdout.write_all(&bytes);
        let _ = stdout.flush();
    }
}
