use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;
use std::time::Duration;

use serde::Serialize;

use super::protocol::{RequestEnvelope, ResponseEnvelope, PROTOCOL_VERSION};

const PRE_TOOL_USE: &str = "PreToolUse";
const FAIL_CLOSED_REASON: &str = "TYBA: transporte de hook indisponível — negado (fail-closed).";

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
}

#[derive(Serialize)]
struct PreToolUseOutput {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecificOutput,
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
    let is_pre_tool_use = event
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .map(|name| name == PRE_TOOL_USE)
        .unwrap_or(false);

    let Some(socket_path) = socket_path else {
        if is_pre_tool_use {
            emit_pre_tool_use(&mut stdout, "deny", FAIL_CLOSED_REASON);
        }
        return 0;
    };

    let Some(stream) = connect_with_retry(socket_path, retry) else {
        if is_pre_tool_use {
            emit_pre_tool_use(&mut stdout, "deny", FAIL_CLOSED_REASON);
        }
        return 0;
    };

    match exchange(stream, &event) {
        Some(response) => emit_response(&mut stdout, &response, is_pre_tool_use),
        None => {
            if is_pre_tool_use {
                emit_pre_tool_use(&mut stdout, "deny", FAIL_CLOSED_REASON);
            }
        }
    }
    0
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

fn emit_response<W: Write>(stdout: &mut W, response: &ResponseEnvelope, is_pre_tool_use: bool) {
    match response.action.as_str() {
        "allow" if is_pre_tool_use => {
            emit_pre_tool_use(stdout, "allow", response.reason.as_deref().unwrap_or(""))
        }
        "deny" if is_pre_tool_use => {
            emit_pre_tool_use(stdout, "deny", response.reason.as_deref().unwrap_or(""))
        }
        "ack" => {}
        "allow" | "deny" => {}
        _ => {
            if is_pre_tool_use {
                emit_pre_tool_use(stdout, "deny", FAIL_CLOSED_REASON);
            }
        }
    }
}

fn emit_pre_tool_use<W: Write>(stdout: &mut W, decision: &'static str, reason: &str) {
    let output = PreToolUseOutput {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: PRE_TOOL_USE,
            permission_decision: decision,
            permission_decision_reason: reason.to_string(),
        },
    };
    if let Ok(mut bytes) = serde_json::to_vec(&output) {
        bytes.push(b'\n');
        let _ = stdout.write_all(&bytes);
        let _ = stdout.flush();
    }
}
