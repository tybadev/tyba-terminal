//! `tyba ls` e companhia: a CLI fina sobre o app ABERTO.
//!
//! O mesmo binário do app, num modo (como o `tyba _hook`): `tyba ls` fala com
//! o socket de controle do app que está rodando e imprime o que ele responde.
//! Não há daemon — fechou o app, os dados somem e a CLI sai com 4.
//!
//! Contrato (plano `tyba-cli`): alvo por ID → rótulo → posição → sufixo
//! ([`target`]); `--json` estável; códigos de saída abaixo; `TYBA_SESSION` e
//! `TYBA_PANE` no env de todo pane.

pub mod target;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod server;

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::layout::LayoutState;
use crate::session::{redact, AgentRunnerKind, ObservedState, Session, SessionKind, SessionStatus};

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILURE: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_TARGET: i32 = 3;
pub const EXIT_NOT_RUNNING: i32 = 4;
/// Ctrl-C: o shell já reporta 128+SIGINT quando o sinal mata o processo.
pub const EXIT_INTERRUPTED: i32 = 130;

pub const PROTOCOL_VERSION: u32 = 1;
const CONTROL_SOCKET_FILE: &str = "control.sock";
const SUBCOMMANDS: &[&str] = &["ls"];
const CLI_MODE: &str = "_cli";

const USAGE: &str = "\
uso:
  tyba ls [--json]       sessões e panes do TYBA aberto, com o estado do agente

A CLI fala com o app aberto: fechou o TYBA, os dados somem junto.
Saída: 0 ok, 1 falha, 2 uso inválido, 3 alvo não achado/ambíguo,
       4 TYBA não está aberto, 130 interrompido.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlRequest {
    pub v: u32,
    pub op: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlResponse {
    pub v: u32,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
}

impl ControlResponse {
    pub fn ok(result: serde_json::Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ok: true,
            code: None,
            error: None,
            result: Some(result),
        }
    }

    pub fn error(code: i32, message: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            ok: false,
            code: Some(code),
            error: Some(message.into()),
            result: None,
        }
    }
}

// ================================ `tyba ls` ================================

/// A saída do `tyba ls --json`. Campos estáveis: só se acrescenta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub workspaces: Vec<WorkspaceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub panes: Vec<PaneEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneEntry {
    /// O mesmo valor de `TYBA_PANE` no env do pane.
    pub id: String,
    /// 1-based, contando todos os panes na ordem desta lista.
    pub position: usize,
    /// O título do pane, redigido.
    pub label: String,
    /// `shell`, `agent`, `ssh` ou `container`.
    pub kind: String,
    pub agent: Option<AgentEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEntry {
    /// `claude`, `codex`, ou o id do manifesto que reconheceu a tela.
    pub name: String,
    /// `running`, `awaiting_input`, `idle`, `exited`, `failed`; `null` quando
    /// o sinal não diz.
    pub state: Option<String>,
    /// `true` = palpite de tela (agente que o TYBA não abriu), sem hook.
    pub observed: bool,
}

/// O layout na ordem da tela: workspaces, abas, panes da esquerda para a
/// direita. Pane de sessão que o core não conhece fica de fora.
pub fn build_listing(layout: &LayoutState, sessions: &[Session]) -> Listing {
    let mut position = 0;
    let workspaces = layout
        .workspaces
        .iter()
        .map(|ws| {
            let mut ids = Vec::new();
            for tab in &ws.tabs {
                if let Some(root) = &tab.root {
                    root.leaf_sessions(&mut ids);
                }
            }
            let panes = ids
                .iter()
                .filter_map(|id| sessions.iter().find(|s| s.id == *id))
                .map(|s| {
                    position += 1;
                    PaneEntry {
                        id: s.id.to_string(),
                        position,
                        label: redact::redact(&s.title).into_owned(),
                        kind: kind_name(&s.kind).into(),
                        agent: agent_of(s),
                    }
                })
                .collect();
            WorkspaceEntry {
                id: ws.id.to_string(),
                name: redact::redact(&ws.name).into_owned(),
                active: layout.active_workspace == Some(ws.id),
                panes,
            }
        })
        .collect();
    Listing { workspaces }
}

fn kind_name(kind: &SessionKind) -> &'static str {
    match kind {
        SessionKind::Shell => "shell",
        SessionKind::Agent { .. } => "agent",
        SessionKind::Ssh { .. } => "ssh",
        SessionKind::Container { .. } => "container",
    }
}

/// O agente que o quadro de agentes mostraria: o gerenciado pelo `status`;
/// o deduzido da tela pelo palpite (`observed`), nunca pelo `status` do
/// shell, que está sempre "rodando".
fn agent_of(session: &Session) -> Option<AgentEntry> {
    match &session.kind {
        SessionKind::Agent { runner } => Some(AgentEntry {
            name: match runner {
                AgentRunnerKind::ClaudeCode => "claude".into(),
                AgentRunnerKind::Codex => "codex".into(),
                AgentRunnerKind::Custom(name) => redact::redact(name).into_owned(),
            },
            state: Some(
                match session.status {
                    SessionStatus::Running => "running",
                    SessionStatus::AwaitingInput { .. } => "awaiting_input",
                    SessionStatus::Idle { .. } => "idle",
                    SessionStatus::Exited { .. } => "exited",
                    SessionStatus::Failed { .. } => "failed",
                }
                .into(),
            ),
            observed: false,
        }),
        _ => session.observed.as_ref().map(|seen| AgentEntry {
            name: seen.agent.clone(),
            state: seen.state.map(|s| {
                match s {
                    ObservedState::Running => "running",
                    ObservedState::AwaitingInput => "awaiting_input",
                    ObservedState::Idle => "idle",
                }
                .into()
            }),
            observed: true,
        }),
    }
}

/// Lado do app: responde um pedido já autenticado (peer do mesmo usuário).
pub fn handle(
    request: &ControlRequest,
    layout: &LayoutState,
    sessions: &[Session],
) -> ControlResponse {
    match request.op.as_str() {
        "ls" => match serde_json::to_value(build_listing(layout, sessions)) {
            Ok(value) => ControlResponse::ok(value),
            Err(e) => ControlResponse::error(EXIT_FAILURE, e.to_string()),
        },
        other => ControlResponse::error(EXIT_USAGE, format!("operação desconhecida: {other}")),
    }
}

// ================================ cliente ================================

/// Onde o app liga o socket de controle: a mesma pasta privada do canal
/// shim↔core (`tyba` ou `tyba-dev`, por usuário).
pub fn resolve_control_socket_path() -> PathBuf {
    crate::hook_ipc::channel::resolve_channel_socket_path().with_file_name(CONTROL_SOCKET_FILE)
}

/// `tyba ls …` no `main`, antes do app. Dois jeitos de entrar:
/// - `tyba _cli <sub> …` — o que o `tyba` do PATH do pane chama. Sempre CLI:
///   subcomando desconhecido sai com 2, nunca abre o app.
/// - `<binário> ls …` direto — só os subcomandos conhecidos; qualquer outro
///   primeiro argumento devolve `None` e o app sobe normalmente.
pub fn maybe_run_cli_mode() -> Option<i32> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let first = args.first()?.as_str();
    if first == CLI_MODE {
        args.remove(0);
    } else if !SUBCOMMANDS.contains(&first) {
        return None;
    }
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    Some(run(
        &args,
        &resolve_control_socket_path(),
        &mut stdout.lock(),
        &mut stderr.lock(),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Invocation {
    request: ControlRequest,
    json: bool,
}

fn parse_args(args: &[String]) -> Result<Invocation, String> {
    let (op, rest) = args.split_first().ok_or("falta o subcomando")?;
    let mut json = false;
    for arg in rest {
        match arg.as_str() {
            "--json" => json = true,
            other => return Err(format!("argumento desconhecido: {other}")),
        }
    }
    match op.as_str() {
        "ls" => Ok(Invocation {
            request: ControlRequest {
                v: PROTOCOL_VERSION,
                op: "ls".into(),
            },
            json,
        }),
        other => Err(format!("subcomando desconhecido: {other}")),
    }
}

/// O cliente inteiro, com o endereço e as saídas na mão (o teste usa um
/// socket num tempdir).
pub fn run(args: &[String], socket: &Path, out: &mut impl Write, err: &mut impl Write) -> i32 {
    if matches!(
        args.first().map(String::as_str),
        Some("help" | "--help" | "-h")
    ) {
        let _ = writeln!(out, "{USAGE}");
        return EXIT_OK;
    }
    let invocation = match parse_args(args) {
        Ok(i) => i,
        Err(message) => {
            let _ = writeln!(err, "tyba: {message}\n\n{USAGE}");
            return EXIT_USAGE;
        }
    };
    let response = match exchange(socket, &invocation.request) {
        Ok(r) => r,
        Err((code, message)) => {
            let _ = writeln!(err, "tyba: {message}");
            return code;
        }
    };
    if !response.ok {
        let _ = writeln!(
            err,
            "tyba: {}",
            response
                .error
                .as_deref()
                .unwrap_or("o TYBA recusou o pedido")
        );
        return response.code.unwrap_or(EXIT_FAILURE);
    }
    let result = response.result.unwrap_or(serde_json::Value::Null);
    let written = if invocation.json {
        writeln!(out, "{result}")
    } else {
        match serde_json::from_value::<Listing>(result) {
            Ok(listing) => write!(out, "{}", render_listing(&listing)),
            Err(e) => {
                let _ = writeln!(err, "tyba: resposta inesperada do TYBA: {e}");
                return EXIT_FAILURE;
            }
        }
    };
    match written {
        Ok(()) => EXIT_OK,
        Err(_) => EXIT_FAILURE,
    }
}

fn render_listing(listing: &Listing) -> String {
    let mut text = String::new();
    for ws in &listing.workspaces {
        let mark = if ws.active { " (ativa)" } else { "" };
        text.push_str(&format!("{}{mark}\n", ws.name));
        for pane in &ws.panes {
            let short: String = pane.id.chars().filter(|c| *c != '-').collect();
            let short = &short[short.len().saturating_sub(8)..];
            let agent = pane
                .agent
                .as_ref()
                .map(|a| format!("  {} · {}", a.name, state_label(a.state.as_deref())))
                .unwrap_or_default();
            text.push_str(&format!(
                "  {:>3}  {short}  {}  [{}]{agent}\n",
                pane.position, pane.label, pane.kind
            ));
        }
    }
    if listing.workspaces.is_empty() {
        text.push_str("nenhuma sessão aberta\n");
    }
    text
}

fn state_label(state: Option<&str>) -> &'static str {
    match state {
        Some("running") => "rodando",
        Some("awaiting_input") => "esperando você",
        Some("idle") => "parado",
        Some("exited") => "saiu",
        Some("failed") => "falhou",
        _ => "sem sinal",
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn exchange(socket: &Path, request: &ControlRequest) -> Result<ControlResponse, (i32, String)> {
    use std::io::{BufRead, BufReader, Read};
    use std::os::unix::net::UnixStream;

    const NOT_RUNNING: &str = "o TYBA não está aberto (a CLI fala com o app aberto)";
    // Pasta que não é do usuário, ou aberta para os outros: alguém pode ter
    // plantado o socket. Recusa antes de conectar.
    if socket.parent().is_some_and(|dir| dir.exists())
        && !crate::hook_ipc::channel::socket_dir_is_ours(socket)
    {
        return Err((
            EXIT_FAILURE,
            "a pasta do socket de controle não é privada deste usuário; recusado".into(),
        ));
    }
    let stream = UnixStream::connect(socket).map_err(|_| (EXIT_NOT_RUNNING, NOT_RUNNING.into()))?;
    // E quem respondeu é do mesmo usuário, pelo kernel.
    let ours = crate::hook_ipc::peercred::peer_cred(&stream).is_some_and(|cred| {
        crate::hook_ipc::peercred::is_trusted_peer(&cred, unsafe { libc::getuid() })
    });
    if !ours {
        return Err((
            EXIT_FAILURE,
            "o socket de controle não é de um TYBA deste usuário; recusado".into(),
        ));
    }
    let failure = |e: std::io::Error| (EXIT_FAILURE, format!("falha falando com o TYBA: {e}"));
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let mut payload = serde_json::to_vec(request).map_err(|e| (EXIT_FAILURE, e.to_string()))?;
    payload.push(b'\n');
    (&stream).write_all(&payload).map_err(failure)?;
    let mut line = String::new();
    // Teto da resposta: um scrollback inteiro cabe com folga.
    BufReader::new((&stream).take(64 * 1024 * 1024))
        .read_line(&mut line)
        .map_err(failure)?;
    if line.is_empty() {
        return Err((EXIT_FAILURE, "o TYBA fechou a conexão sem responder".into()));
    }
    serde_json::from_str(line.trim_end())
        .map_err(|e| (EXIT_FAILURE, format!("resposta inválida do TYBA: {e}")))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn exchange(_socket: &Path, _request: &ControlRequest) -> Result<ControlResponse, (i32, String)> {
    Err((
        EXIT_FAILURE,
        "a CLI do TYBA ainda não existe neste sistema (só Linux e macOS)".into(),
    ))
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests;
