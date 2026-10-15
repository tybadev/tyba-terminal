use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::Serialize;

use super::framing;
use super::protocol::{RequestEnvelope, ResponseEnvelope, PROTOCOL_VERSION};

#[cfg(unix)]
type Stream = std::os::unix::net::UnixStream;
#[cfg(windows)]
type Stream = super::pipe::Pipe;

const PRE_TOOL_USE: &str = "PreToolUse";
const PERMISSION_REQUEST: &str = "PermissionRequest";
const FAIL_CLOSED_REASON: &str = "TYBA: transporte de hook indisponível — negado (fail-closed).";
const DENY_FALLBACK_REASON: &str = "negado no TYBA";
const ASK_FALLBACK_REASON: &str = "o TYBA pede confirmação";

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

/// Conexão-com-retry + framing, extraído de `run_client_inner` pra servir o
/// modo `_open-url` do shim de navegador (entrega B, §5.2) sem reusar
/// `run_client`: aquele SEMPRE devolve exit 0 (fail-closed pro gate de
/// PreToolUse/PermissionRequest — nunca pode travar uma tool), e o modo
/// `_open-url` precisa distinguir ack de deny/timeout no próprio exit code
/// (§1.2 — exit≠0 é o que faz o Claude cair no fallback de tela, V4). Retry
/// fixo em `RetryPlan::default()`: nenhum chamador de fora do módulo precisa
/// customizar o backoff. `run_client_inner` continua chamando o mesmo par
/// connect+exchange por baixo — o comportamento dele não muda (ver
/// hook_ipc::tests, que pinam a saída byte a byte e continuam verdes).
pub fn request(socket_path: &Path, event: &serde_json::Value) -> Option<ResponseEnvelope> {
    connect_and_exchange(socket_path, event, RetryPlan::default())
}

fn connect_and_exchange(
    socket_path: &Path,
    event: &serde_json::Value,
    retry: RetryPlan,
) -> Option<ResponseEnvelope> {
    let stream = connect_with_retry(socket_path, retry)?;
    exchange(stream, event)
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

    match connect_and_exchange(socket_path, &event, retry) {
        Some(response) => emit_response(&mut stdout, &response, kind, event.get("tool_input")),
        None => emit_transport_failure(&mut stdout, kind),
    }
    0
}

/// Retentativa curta do modo observador: o hook roda inline em toda
/// ferramenta, e um core ausente não pode custar segundos por chamada.
const OBSERVER_RETRY: RetryPlan = RetryPlan {
    attempts: 2,
    base: Duration::from_millis(20),
};

/// `tyba _hook --observe`, o hook do claude digitado num pane (agente livre).
/// Só observa: todo evento vai ao core, e a resposta do core é ignorada — o
/// TYBA nunca decide por ele. A única decisão é o guarda-corpo fixo
/// (`guard`, que devolve o JSON de `deny` ou `None`), tomada aqui, sem o
/// core. Core ausente, socket sumido ou resposta estranha: stdout vazio, que
/// para o Claude é "sem decisão". Nunca `allow`, nunca `deny` por falha.
/// O stdout sai primeiro; o envio ao core vem depois, sem ler resposta e com
/// teto (`send_and_forget`) — o hook nunca espera o core.
pub fn run_observer_client<R: Read, W: Write>(
    stdin: R,
    stdout: W,
    socket_path: Option<&Path>,
    guard: &dyn Fn(&serde_json::Value) -> Option<String>,
) -> i32 {
    run_observer_client_inner(stdin, stdout, socket_path, guard, OBSERVER_RETRY)
}

/// `tyba _hook --guard-only`, o hook do claude aberto de dentro do claude
/// hospedado: o guarda-corpo e nada mais. Não recebe socket — nunca fala com
/// o core, para o aninhado não sequestrar o monitor e a conversa do pane.
pub fn run_guard_only_client<R: Read, W: Write>(
    stdin: R,
    stdout: W,
    guard: &dyn Fn(&serde_json::Value) -> Option<String>,
) -> i32 {
    run_observer_client_inner(stdin, stdout, None, guard, OBSERVER_RETRY)
}

pub(crate) fn run_observer_client_inner<R: Read, W: Write>(
    mut stdin: R,
    mut stdout: W,
    socket_path: Option<&Path>,
    guard: &dyn Fn(&serde_json::Value) -> Option<String>,
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
    if event.get("hook_event_name").and_then(|v| v.as_str()) == Some(PRE_TOOL_USE) {
        if let Some(out) = guard(&event) {
            let _ = stdout.write_all(out.as_bytes());
            let _ = stdout.write_all(b"\n");
            let _ = stdout.flush();
        }
    }
    if let Some(socket_path) = socket_path {
        send_and_forget(socket_path, &event, retry);
    }
    0
}

/// Teto do envio ao core no modo observador (connect + escrita). Ninguém
/// espera a resposta, então um core travado (breakpoint, SIGSTOP, handler
/// preso no lock do Store) não pode segurar o hook: passado o teto, o hook
/// sai e o evento se perde — melhor que segurar a ferramenta do Claude.
pub(crate) const OBSERVER_SEND_BUDGET: Duration = Duration::from_millis(250);
#[cfg(unix)]
const OBSERVER_WRITE_TIMEOUT: Duration = Duration::from_millis(200);

/// Manda o evento ao core sem ler a resposta: conecta, escreve o frame e
/// fecha. O connect não tem timeout no std (um backlog cheio pode bloquear no
/// Linux), então tudo roda numa thread com teto; se ela não volta a tempo, o
/// processo do hook sai e a leva junto.
fn send_and_forget(socket_path: &Path, event: &serde_json::Value, retry: RetryPlan) {
    let Ok(mut payload) = serde_json::to_vec(&RequestEnvelope {
        v: PROTOCOL_VERSION,
        event: event.clone(),
    }) else {
        return;
    };
    payload.push(b'\n');
    let socket_path = socket_path.to_path_buf();
    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn(move || {
        if let Some(mut stream) = connect_with_retry(&socket_path, retry) {
            #[cfg(unix)]
            let _ = stream.set_write_timeout(Some(OBSERVER_WRITE_TIMEOUT));
            let _ = stream.write_all(&payload);
            let _ = stream.flush();
            #[cfg(unix)]
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        let _ = done_tx.send(());
    });
    let _ = done_rx.recv_timeout(OBSERVER_SEND_BUDGET);
}

fn emit_transport_failure<W: Write>(stdout: &mut W, kind: EventKind) {
    if kind == EventKind::PreToolUse {
        emit_pre_tool_use(stdout, "deny", FAIL_CLOSED_REASON);
    }
}

#[cfg(unix)]
fn connect_once(socket_path: &Path) -> Option<Stream> {
    std::os::unix::net::UnixStream::connect(socket_path).ok()
}

#[cfg(windows)]
fn connect_once(socket_path: &Path) -> Option<Stream> {
    super::pipe::connect(&super::pipe::pipe_name(socket_path))
}

fn connect_with_retry(socket_path: &Path, retry: RetryPlan) -> Option<Stream> {
    for attempt in 0..retry.attempts {
        if let Some(stream) = connect_once(socket_path) {
            return Some(stream);
        }
        // Depois da última tentativa não há mais o que esperar.
        if attempt + 1 < retry.attempts {
            thread::sleep(retry.base * 2u32.pow(attempt));
        }
    }
    None
}

fn exchange(stream: Stream, event: &serde_json::Value) -> Option<ResponseEnvelope> {
    let read_half = stream.try_clone().ok()?;
    framing::exchange(BufReader::new(read_half), stream, event)
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
        // Fluxo híbrido: `ask` só tem forma no PreToolUse, e sem
        // `updatedInput` — é pergunta, não aval. Em outro evento, silêncio.
        ("ask", EventKind::PreToolUse) => {
            emit_pre_tool_use(stdout, "ask", reason.unwrap_or(ASK_FALLBACK_REASON))
        }
        // `pass` é "sem decisão": nada no stdout, exit 0, e o agente segue as
        // regras dele. `ack` idem. Ação desconhecida continua fail-closed.
        ("pass", _)
        | ("ack", _)
        | ("ask", _)
        | ("allow", EventKind::Other)
        | ("deny", EventKind::Other) => {}
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
