use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::agent::notify;

use crate::agent::claude_settings::claude_settings_are_broad;
use crate::agent::hooks_settings::{claude_session_settings, hook_command};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::agent::hooks_settings::{observer_hook_command, observer_session_settings};
use crate::agent::subagents::SharedSubagents;
use crate::agent::usage::{AgentAlert, AgentUsage, EVENT_AGENT_ALERT, STATUSLINE_HOOK_EVENT};
use crate::agent::{AgentRunner, ClaudeCodeRunner, CodexRunner, HookSetup};
use crate::approvals::hybrid::{plan_pre_tool_use, HybridDecision, PreToolUsePlan};
use crate::approvals::tool_action::{normalize_tool_use, ToolAction};
use crate::approvals::{now_ms, ApprovalAsk, Decision, RiskLevel, SharedApprovals};
use crate::hook_ipc::{HookAction, HookEvent, HookServer};
use crate::pty::SharedPtyPool;
use crate::sandbox::SandboxSpec;
use crate::session::store::{ApprovalHistoryEntry, Store};
use crate::session::{
    AgentRunnerKind, AwaitingReason, CreateSessionOpts, Session, SessionId, SessionKind,
    SessionStatus, SharedSessionManager,
};
use crate::status::agent_events::{
    native_prompt_status, note_for_event, signal_for, status_for, status_on_idle_prompt,
    status_on_interrupt, status_on_session_end, status_on_stop_failure, AgentSignal,
    INTERRUPTED_SUMMARY,
};

pub const EVENT_AGENT_READY: &str = "agent://ready";
const HOOK_SETTINGS_FILE: &str = "hooks.json";
const HOOK_SOCKET_FILE: &str = "hook.sock";
const MAX_SOCKET_PATH: usize = 100;

#[derive(Clone, serde::Serialize)]
struct AgentReadyPayload {
    session_id: SessionId,
}

#[derive(Default)]
pub struct HookServerRegistry {
    servers: Mutex<HashMap<SessionId, HookServer>>,
}

impl HookServerRegistry {
    /// Review round 1 (correção/contrato), achado BLOQUEANTE: `if let Some(old)
    /// = self.servers.lock()...insert(...) { old.shutdown() }` estende o
    /// tempo de vida do `MutexGuard` temporário até o FIM do `if let` — o
    /// `shutdown()` (que faz `join()` bloqueante na accept-thread antiga)
    /// corre com o Mutex de `servers` AINDA TRANCADO. Se esse `join()`
    /// travar (ver `stop()` em `hook_ipc/server.rs` — `UnixStream::connect`
    /// só acorda a accept-loop antiga se o path ainda for DELA), todo o
    /// resto do registry — `insert`/`shutdown` de QUALQUER outra sessão —
    /// trava atrás do mesmo Mutex, para sempre. Solta o guard ANTES de
    /// chamar `shutdown()`: mesmo que o `join()` demore ou trave, o Mutex
    /// segue destravado para o resto do app.
    pub fn insert(&self, id: SessionId, server: HookServer) {
        let old = self
            .servers
            .lock()
            .expect("hook servers lock")
            .insert(id, server);
        if let Some(old) = old {
            old.shutdown();
        }
    }

    pub fn shutdown(&self, id: SessionId) {
        let removed = self.servers.lock().expect("hook servers lock").remove(&id);
        if let Some(server) = removed {
            server.shutdown();
        }
    }
}

pub struct AgentSessionCtx {
    pub app: AppHandle,
    pub sessions: SharedSessionManager,
    pub pty_pool: SharedPtyPool,
    pub approvals: SharedApprovals,
    pub store: Arc<Store>,
    pub servers: Arc<HookServerRegistry>,
    pub subagents: SharedSubagents,
}

pub(crate) fn decision_label(decision: Decision) -> &'static str {
    match decision {
        Decision::Approved => "approved",
        Decision::Denied => "denied",
        Decision::ApprovedAlways => "approved_always",
    }
}

pub(crate) fn risk_label(risk: RiskLevel) -> &'static str {
    match risk {
        RiskLevel::Green => "green",
        RiskLevel::Yellow => "yellow",
        RiskLevel::Red => "red",
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_history(
    store: &Store,
    session_id: SessionId,
    command: String,
    cwd: Option<String>,
    risk: RiskLevel,
    decision: &str,
    requested_at_ms: u64,
) {
    let entry = ApprovalHistoryEntry {
        session_id: session_id.to_string(),
        command,
        cwd,
        risk: risk_label(risk).to_string(),
        decision: decision.to_string(),
        requested_at_ms,
        resolved_at_ms: now_ms(),
    };
    let _ = store.insert_approval_history(&entry);
}

struct HandlerCtx {
    app: AppHandle,
    sessions: SharedSessionManager,
    approvals: SharedApprovals,
    store: Arc<Store>,
    subagents: SharedSubagents,
    session_id: SessionId,
    runner_kind: AgentRunnerKind,
    worktree_root: PathBuf,
    /// Raiz do repositório PRINCIPAL (não o worktree): é onde o Claude lê e
    /// grava o `.claude/settings.local.json` do projeto (medido no B0 do fluxo
    /// híbrido). Entra na leitura de regra ampla.
    repo_root: PathBuf,
    /// `HOME` do agente — de onde o Claude dele lê `~/.claude`. Nunca o
    /// `CLAUDE_CONFIG_DIR` do app (denylist de env).
    agent_home: Option<PathBuf>,
    turn_settle: Arc<std::sync::atomic::AtomicU64>,
    /// Último `transcript_path` que já virou id de conversa. Existe para o
    /// handler não reabrir o arquivo a cada `PreToolUse`: o id mora no cabeçalho
    /// e não muda enquanto o transcript for o mesmo.
    seen_transcript: Arc<Mutex<Option<String>>>,
    /// Agente livre (claude digitado num pane): o hook só observa. O
    /// `PreToolUse` nunca vira card nem espera ninguém — ver
    /// [`session_route`].
    observe_only: bool,
}

fn main_window_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false)
}

/// A sessão que o dono está olhando agora: o painel em foco do workspace ativo.
fn viewed_session(app: &AppHandle) -> Option<SessionId> {
    app.try_state::<crate::AppState>()
        .and_then(|state| state.layout.active_session())
}

/// O aviso só se cala quando o dono já está vendo a sessão. Com o TYBA em
/// foco noutra sessão (o claude roda em muriki-api, ele olha outro shell), o
/// "terminou" e o "precisa de você" saem igual — é o caso que mais importa.
fn notify_suppressed(window_focused: bool, viewed: Option<SessionId>, id: SessionId) -> bool {
    window_focused && viewed == Some(id)
}

/// O aviso do sistema, com a política já resolvida.
///
/// `pub(crate)` porque o palpite de tela também sai por aqui: são fontes
/// diferentes com a mesma saída, e duplicar isto significaria duplicar o
/// silêncio-com-janela-em-foco, a redação e a leitura de preferência — três
/// lugares para uma delas envelhecer sozinha.
pub(crate) fn notify_native(
    app: &AppHandle,
    sessions: &SharedSessionManager,
    store: &Store,
    kind: notify::NotifyKind,
    id: SessionId,
    body: &str,
) {
    let focused = main_window_focused(app);
    let viewed = viewed_session(app);
    if notify_suppressed(focused, viewed, id) {
        eprintln!("[tyba] aviso {kind:?} de {id} calado: a sessão está na tela");
        return;
    }
    // Preferência ilegível não silencia o aviso: `ok().flatten()` cai em `None`,
    // que é "nunca escolhi", que é o default ligado. Perder o aviso de um agente
    // bloqueado por causa de um erro de leitura seria o dano maior.
    let enabled_raw = store.get_setting(kind.enabled_key()).ok().flatten();
    let sound_raw = store.get_setting(kind.sound_key()).ok().flatten();
    let policy = notify::resolve(kind, enabled_raw.as_deref(), sound_raw.as_deref());
    if !policy.enabled {
        eprintln!("[tyba] aviso {kind:?} de {id} desligado nas preferências");
        return;
    }
    let title = sessions
        .get(id)
        .map(|s| s.title)
        .unwrap_or_else(|| "sessão de agente".into());
    let body = crate::session::redact::redact(body);
    let mut builder = app
        .notification()
        .builder()
        .title(format!("Tyba — {title}"))
        .body(body.as_ref());
    if let Some(sound) = policy.sound {
        builder = builder.sound(sound);
    }
    match builder.show() {
        Ok(()) => eprintln!(
            "[tyba] aviso {kind:?} de {id} enviado (janela em foco: {focused}, na tela: {viewed:?})"
        ),
        Err(e) => eprintln!("[tyba] aviso {kind:?} de {id} falhou: {e}"),
    }
}

fn notify_awaiting_input(ctx: &HandlerCtx, body: &str) {
    notify_native(
        &ctx.app,
        &ctx.sessions,
        &ctx.store,
        notify::NotifyKind::Request,
        ctx.session_id,
        body,
    );
}

const TURN_END_SETTLE_MS: u64 = 2000;
const TURN_SUMMARY_MAX_CHARS: usize = 140;

/// A fala final do turno, e se ela é autoritativa.
///
/// `last_assistant_message` vem do payload do hook (Codex) e é a fala definitiva
/// do turno — não precisa de settle. O tail do transcript é melhor-esforço: o
/// agente pode não ter flushado a última mensagem quando o `Stop` dispara.
struct TurnSummary {
    text: Option<String>,
    settled: bool,
}

fn turn_summary(event: &HookEvent) -> TurnSummary {
    let inline = event
        .raw
        .get("last_assistant_message")
        .and_then(|m| m.as_str())
        .and_then(|m| crate::status::transcript::clean_summary(m, TURN_SUMMARY_MAX_CHARS));
    if inline.is_some() {
        return TurnSummary {
            text: inline,
            settled: true,
        };
    }
    let text = event
        .raw
        .get("transcript_path")
        .and_then(|p| p.as_str())
        .and_then(|path| {
            crate::status::transcript::last_assistant_text(
                std::path::Path::new(path),
                TURN_SUMMARY_MAX_CHARS,
            )
        });
    TurnSummary {
        text,
        settled: false,
    }
}

fn notify_turn_ended(ctx: &HandlerCtx, transcript_path: Option<String>, needs_settle: bool) {
    use std::sync::atomic::Ordering;

    let transcript_path = needs_settle.then_some(transcript_path).flatten();
    let generation = ctx.turn_settle.fetch_add(1, Ordering::SeqCst) + 1;
    let app = ctx.app.clone();
    let sessions = ctx.sessions.clone();
    let turn_settle = ctx.turn_settle.clone();
    let store = ctx.store.clone();
    let id = ctx.session_id;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(TURN_END_SETTLE_MS));
        if turn_settle.load(Ordering::SeqCst) != generation {
            eprintln!("[tyba] fim de turno de {id} sem aviso: turno novo começou");
            return;
        }
        let Some(session) = sessions.get(id) else {
            return;
        };
        let SessionStatus::Idle { summary } = &session.status else {
            eprintln!(
                "[tyba] fim de turno de {id} sem aviso: status {:?}",
                session.status
            );
            return;
        };
        let settled = transcript_path.as_deref().and_then(|path| {
            crate::status::transcript::last_assistant_text(
                std::path::Path::new(path),
                TURN_SUMMARY_MAX_CHARS,
            )
        });
        let summary = match settled {
            Some(fresh) if summary.as_deref() != Some(fresh.as_str()) => {
                sessions.set_status(
                    &app,
                    id,
                    SessionStatus::Idle {
                        summary: Some(fresh.clone()),
                    },
                );
                Some(fresh)
            }
            Some(fresh) => Some(fresh),
            None => summary.clone(),
        };
        let body = summary.unwrap_or_else(|| "Terminou o que tinha pra rodar".into());
        notify_native(&app, &sessions, &store, notify::NotifyKind::Done, id, &body);
    });
}

/// Regra ampla nas configurações do Claude que esta sessão carrega. Sem
/// `HOME` conhecido não dá para ler o escopo do usuário: responde "amplo",
/// que só custa uma pergunta a mais — nunca um amarelo calado.
fn claude_settings_broad(ctx: &HandlerCtx) -> bool {
    match &ctx.agent_home {
        Some(home) => claude_settings_are_broad(home, &ctx.worktree_root, &ctx.repo_root),
        None => true,
    }
}

fn on_pre_tool_use(ctx: &HandlerCtx, event: &HookEvent) -> HookAction {
    let tool = event.tool_name.as_deref().unwrap_or("");
    let input = event.tool_input.as_ref();
    let PreToolUsePlan {
        normalized,
        risk,
        reason,
        refused_by_core,
        decision,
    } = plan_pre_tool_use(&ctx.runner_kind, tool, input, &ctx.worktree_root, || {
        claude_settings_broad(ctx)
    });
    let subagent_spawn = if let ToolAction::Subagent {
        agent_type,
        description,
    } = &normalized.action
    {
        ctx.subagents.on_spawn_requested(
            &ctx.app,
            ctx.session_id,
            agent_type.clone(),
            description.clone(),
        );
        Some((agent_type.clone(), description.clone()))
    } else {
        None
    };
    // Três textos da mesma chamada (ver `NormalizedToolUse`): o exibido vai ao
    // pedido e ao histórico; a chave, à allowlist da sessão; o resumo, à dica
    // de status e à notificação do SO. A recusa já leu o inteiro no plano.
    let command = normalized.display_text();
    let allowlist_key = normalized.allowlist_key();
    let summary = normalized.summary();
    let cwd = event.cwd.clone();
    // O que o subagente está fazendo agora, para a linha dele no painel.
    // Antes da decisão: um pedido de aprovação também é "o que ele faz".
    if let Some(agent_id) = event.agent_id.as_deref() {
        ctx.subagents.on_tool_use(
            &ctx.app,
            ctx.session_id,
            agent_id,
            crate::agent::subagents::tool_activity(tool, input, &normalized.description),
        );
    }

    match decision {
        HybridDecision::Allow => {
            record_history(
                &ctx.store,
                ctx.session_id,
                command,
                cwd,
                risk,
                "auto_allowed",
                now_ms(),
            );
            HookAction::Allow { reason: None }
        }
        // Amarelo do Claude: quem pergunta é o Claude. Sem pedido, sem toast,
        // sem linha de histórico aqui (R1, R2 do fluxo híbrido).
        HybridDecision::Pass => HookAction::Pass,
        HybridDecision::Ask(reason) => HookAction::Ask { reason },
        HybridDecision::Gate => {
            if risk != RiskLevel::Red
                && ctx
                    .approvals
                    .is_session_allowed(ctx.session_id, &allowlist_key)
            {
                record_history(
                    &ctx.store,
                    ctx.session_id,
                    command,
                    cwd,
                    risk,
                    "session_allowed",
                    now_ms(),
                );
                return HookAction::Allow { reason: None };
            }
            ctx.sessions.set_status(
                &ctx.app,
                ctx.session_id,
                SessionStatus::AwaitingInput {
                    hint: Some(summary.clone()),
                    reason: AwaitingReason::Approval,
                },
            );
            notify_awaiting_input(ctx, &format!("Aprovação pendente: {summary}"));
            let outcome = ctx.approvals.request_blocking(
                &ctx.app,
                ctx.session_id,
                ApprovalAsk {
                    command: command.clone(),
                    allowlist_key,
                    cwd: cwd.clone(),
                    context: None,
                    risk,
                    risk_reason: reason,
                    refused_by_core,
                },
                event
                    .agent_id
                    .as_deref()
                    .map(crate::agent::subagents::strip_agent_prefix),
            );
            ctx.sessions
                .set_status(&ctx.app, ctx.session_id, SessionStatus::Running);
            // O `AwaitingInput` da aprovação apagou o "subagentes trabalhando";
            // se quem pediu foi um subagente com o turno do orquestrador
            // encerrado, a sessão continua segura só por eles.
            if label_held_after_running(
                event.agent_id.is_some(),
                ctx.subagents.orchestrator_idle(ctx.session_id),
            ) {
                mark_held_by_subagents(ctx);
            }
            match outcome {
                Ok((_request, resolution)) => {
                    if resolution.decision.is_approval() {
                        HookAction::Allow { reason: None }
                    } else {
                        deny_subagent_spawn(ctx, &subagent_spawn);
                        HookAction::Deny {
                            reason: resolution
                                .feedback
                                .unwrap_or_else(|| "negado no Tyba".into()),
                        }
                    }
                }
                Err(reason) => {
                    record_history(
                        &ctx.store,
                        ctx.session_id,
                        command,
                        cwd,
                        risk,
                        "refused",
                        now_ms(),
                    );
                    deny_subagent_spawn(ctx, &subagent_spawn);
                    HookAction::Deny { reason }
                }
            }
        }
    }
}

fn deny_subagent_spawn(ctx: &HandlerCtx, spawn: &Option<(Option<String>, Option<String>)>) {
    if let Some((agent_type, description)) = spawn {
        ctx.subagents.on_spawn_denied(
            &ctx.app,
            ctx.session_id,
            agent_type.clone(),
            description.clone(),
        );
    }
}

/// Guarda o id da conversa nativa que o agente acabou de reportar.
///
/// É a única janela em que o TYBA aprende esse id: quem o escreve é a CLI, no
/// transcript/rollout dela, e o caminho desse arquivo chega aqui pelo payload do
/// hook. Sem isto, uma sessão de agente que morre com o app não tem como ser
/// retomada — o histórico existe no disco, mas ninguém sabe qual conversa é.
/// Teto do `cwd` do payload gravado com a conversa: o `PATH_MAX` comum. Acima
/// disso não é pasta de verdade, e o payload não escolhe o tamanho do que vai
/// para o SQLite.
const MAX_CONVERSATION_CWD: usize = 4096;

fn capture_conversation_id(ctx: &HandlerCtx, event: &HookEvent) {
    let path = event
        .raw
        .get("transcript_path")
        .and_then(|p| p.as_str())
        .unwrap_or_default();
    if ctx
        .seen_transcript
        .lock()
        .expect("seen transcript lock")
        .as_deref()
        == Some(path)
    {
        return;
    }
    let found = crate::agent::conversation::from_hook_payload(&event.raw);
    // Payload sem transcript e sem id não marca nada como lido: não houve o que
    // ler, e o próximo evento ainda pode trazer a fonte.
    if found.is_some() || !path.is_empty() {
        *ctx.seen_transcript.lock().expect("seen transcript lock") = Some(path.to_string());
    }
    if let Some(id) = found {
        // A pasta da conversa, para o restart poder retomá-la de onde ela
        // rodava. Dado do payload: só vira `cd` depois de validado
        // (`agent::claude_resume_dir`).
        let cwd = event
            .raw
            .get("cwd")
            .and_then(|c| c.as_str())
            .filter(|c| c.len() <= MAX_CONVERSATION_CWD)
            .map(Path::new)
            .filter(|c| c.is_absolute());
        ctx.sessions
            .set_agent_conversation_id(&ctx.app, ctx.session_id, &id, cwd);
    }
}

/// O modelo que o agente declara no payload (`model`, extensão do Codex
/// presente em todo evento menos `SessionEnd`; vale para qualquer runner que o
/// mande). Só do fio principal: evento com `agent_id` é de subagente, que pode
/// rodar outro modelo.
fn capture_model(ctx: &HandlerCtx, event: &HookEvent) {
    if let Some(model) = model_of(event) {
        ctx.sessions
            .set_agent_model(&ctx.app, ctx.session_id, model);
    }
}

fn model_of(event: &HookEvent) -> Option<&str> {
    if event.agent_id.is_some() {
        return None;
    }
    event.raw.get("model").and_then(serde_json::Value::as_str)
}

/// Se o `SessionEnd` encerra os subagentes da sessão. `reason: "clear"` não
/// encerra: o `/clear` troca de conversa com o MESMO processo vivo, e um
/// subagente em background continua rodando — marcá-lo concluído mentiria no
/// painel (M2 do claude-monitor). Ele termina pelo próprio `SubagentStop` ou
/// pelo watcher do transcript. Qualquer outro motivo (saída, logout) encerra.
fn session_end_ends_subagents(raw: &serde_json::Value) -> bool {
    raw.get("reason").and_then(serde_json::Value::as_str) != Some("clear")
}

/// `SubagentStart` com a sessão ociosa: um subagente em background acordado
/// pela própria tarefa volta a trabalhar, e a sessão volta a `Running`. Só
/// sai de `Idle` — prompt aberto, saída e falha continuam mandando.
/// A sessão segue `Running` só por subagente (turno do orquestrador
/// encerrado): a UI troca "em andamento" por "subagentes trabalhando". Prompt
/// aberto (`AwaitingInput`) não ganha a marca — esperar alguém vence.
///
/// Só sessão de agente: num shell (claude hospedado) o que a UI mostra é o
/// palpite de tela, e a marca ali vem do `ObservedHold`, que olha a mesma
/// tela — duas fontes escrevendo o campo brigariam a cada poll.
fn mark_held_by_subagents(ctx: &HandlerCtx) {
    let held = ctx.sessions.get(ctx.session_id).is_some_and(|s| {
        held_by_subagents(
            matches!(s.kind, SessionKind::Agent { .. }),
            &s.status,
            ctx.subagents.has_active(ctx.session_id),
        )
    });
    if held {
        ctx.sessions
            .set_subagents_working(&ctx.app, ctx.session_id, true);
    }
}

/// A marca exige subagente ativo de fato: numa corrida com o watcher o último
/// pode ter fechado entre o sinal e aqui, e a marca ficaria sem ninguém.
fn held_by_subagents(is_agent: bool, status: &SessionStatus, has_active: bool) -> bool {
    is_agent && matches!(status, SessionStatus::Running) && has_active
}

/// Depois de voltar a `Running` por evento de subagente (ferramenta, decisão
/// de aprovação), a marca volta se o orquestrador segue com o turno
/// encerrado. Evento do próprio orquestrador nunca a repõe.
fn label_held_after_running(from_subagent: bool, orchestrator_idle: bool) -> bool {
    from_subagent && orchestrator_idle
}

fn status_on_subagent_start(current: &SessionStatus, active: bool) -> Option<SessionStatus> {
    (active && matches!(current, SessionStatus::Idle { .. })).then_some(SessionStatus::Running)
}

/// `Stop` com subagente ativo segura a sessão em `Running` — menos quando há
/// um prompt aberto (nativo, aprovação, resposta): esse continua mandando.
fn status_on_held_stop(current: &SessionStatus) -> Option<SessionStatus> {
    (!matches!(current, SessionStatus::AwaitingInput { .. })).then_some(SessionStatus::Running)
}

/// Para onde um evento de hook vai depois do sinal de status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookRoute {
    /// `on_pre_tool_use`: classificação, decisão híbrida e gate.
    ToolUse,
    /// Agente livre: `on_observed_tool_use` — só o monitor, sem decisão.
    Observe,
    /// O prompt nativo do Claude abriu: marca a sessão, avisa, `ack`.
    NativePrompt,
    OpenUrl,
    /// O `tyba _statusline` mandando o uso da sessão.
    Usage,
    Ack,
}

/// O roteamento, sem efeito colateral.
///
/// `PermissionRequest` do Claude **nunca** vai a `on_pre_tool_use`: lá ele
/// poderia sair `allow`, e responder `allow` a esse evento aprovaria o prompt
/// nativo em nome do usuário (invariante 4 do fluxo híbrido). O do Codex
/// continua no gate — é o protocolo de aprovação dele.
fn hook_route(runner: &AgentRunnerKind, hook_event_name: &str) -> HookRoute {
    match (hook_event_name, runner) {
        ("TybaOpenUrl", _) => HookRoute::OpenUrl,
        (STATUSLINE_HOOK_EVENT, _) => HookRoute::Usage,
        ("PreToolUse", _) => HookRoute::ToolUse,
        ("PermissionRequest", AgentRunnerKind::ClaudeCode) => HookRoute::NativePrompt,
        ("PermissionRequest", _) => HookRoute::ToolUse,
        _ => HookRoute::Ack,
    }
}

/// A rota de um evento numa sessão. No agente livre (`observe_only`), o que
/// iria ao gate vai ao observador: nenhum card, nenhuma espera, nenhuma
/// decisão. O `PermissionRequest` do Claude já é só aviso (`NativePrompt`).
fn session_route(runner: &AgentRunnerKind, hook_event_name: &str, observe_only: bool) -> HookRoute {
    match hook_route(runner, hook_event_name) {
        HookRoute::ToolUse if observe_only => HookRoute::Observe,
        route => route,
    }
}

/// `PreToolUse` do agente livre: alimenta o painel de subagentes (o pedido
/// de subagente e o que cada um está fazendo) e sai sem decisão. O
/// guarda-corpo já foi aplicado no próprio hook (`tyba _hook --observe`), e o
/// cliente ignora esta resposta de qualquer forma.
fn on_observed_tool_use(ctx: &HandlerCtx, event: &HookEvent) -> HookAction {
    let tool = event.tool_name.as_deref().unwrap_or("");
    let input = event.tool_input.as_ref();
    let normalized = normalize_tool_use(&ctx.runner_kind, tool, input);
    if let ToolAction::Subagent {
        agent_type,
        description,
    } = &normalized.action
    {
        ctx.subagents.on_spawn_requested(
            &ctx.app,
            ctx.session_id,
            agent_type.clone(),
            description.clone(),
        );
    }
    if let Some(agent_id) = event.agent_id.as_deref() {
        ctx.subagents.on_tool_use(
            &ctx.app,
            ctx.session_id,
            agent_id,
            crate::agent::subagents::tool_activity(tool, input, &normalized.description),
        );
    }
    HookAction::Pass
}

/// O prompt nativo do Claude está aberto no terminal da sessão.
///
/// O `PermissionRequest` chega ~0,1 s depois do `PreToolUse` da mesma chamada,
/// mas só depois de o Claude receber a resposta daquele — e o handler do
/// `PreToolUse` põe `Running` **antes** de responder. Pela causalidade, este
/// `NativePrompt` é sempre o último a escrever; e o próprio `PermissionRequest`
/// não gera sinal de status (`signal_for` devolve `None`), então nada o
/// sobrescreve com `Running` no mesmo evento. Quem tira daqui: `PostToolUse`,
/// o próximo `PreToolUse`, `UserPromptSubmit` (via sinal) e `SessionEnd`.
fn on_native_prompt(ctx: &HandlerCtx, event: &HookEvent) -> HookAction {
    let summary = normalize_tool_use(
        &ctx.runner_kind,
        event.tool_name.as_deref().unwrap_or(""),
        event.tool_input.as_ref(),
    )
    .summary();
    let hint = (!summary.trim().is_empty()).then_some(summary);
    ctx.sessions
        .set_status(&ctx.app, ctx.session_id, native_prompt_status(hint.clone()));
    let body = match hint {
        Some(summary) => format!("Pede permissão no terminal: {summary}"),
        None => "Pede permissão no terminal".to_string(),
    };
    notify_native(
        &ctx.app,
        &ctx.sessions,
        &ctx.store,
        notify::NotifyKind::NativePrompt,
        ctx.session_id,
        &body,
    );
    HookAction::Ack
}

/// Chave que casa o `PermissionRequest` de uma chamada com o `PostToolUse`
/// dela.
///
/// O `PermissionRequest` não traz `tool_use_id` (medido no B0); os quatro
/// campos que batem entre `PreToolUse`, `PermissionRequest` e `PostToolUse` da
/// mesma chamada são `session_id`, `prompt_id`, `tool_name` e `tool_input`. O
/// `tool_input` entra canônico (chaves de objeto ordenadas, em qualquer
/// profundidade): a ordem em que o Claude serializa não é contrato, e a mesma
/// chamada com as chaves em outra ordem tem de dar a mesma chave. Campo ausente
/// entra como `null` — a chave fica mais fraca, nunca some.
///
/// SHA-256 sobre o array JSON canônico dos quatro: o array já separa os campos
/// sem ambiguidade, e o hash é estável entre versões do Rust (o `DefaultHasher`
/// não é, e a chave vai para o disco).
pub(crate) fn call_key(raw: &serde_json::Value) -> String {
    use sha2::Digest;

    let field = |name: &str| raw.get(name).cloned().unwrap_or(serde_json::Value::Null);
    let tuple = serde_json::Value::Array(vec![
        field("session_id"),
        field("prompt_id"),
        field("tool_name"),
        field("tool_input"),
    ]);
    let mut canonical = String::new();
    write_canonical_json(&tuple, &mut canonical);
    format!("{:x}", sha2::Sha256::digest(canonical.as_bytes()))
}

/// JSON com as chaves de todo objeto em ordem, sem espaço. Não depende da
/// feature `preserve_order` do `serde_json` estar desligada — se um dia alguém
/// a ligar, a chave continua a mesma.
fn write_canonical_json(value: &serde_json::Value, out: &mut String) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical_json(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical_json(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

/// O que um evento faz com as linhas deduzidas do prompt nativo (R8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InferredStep {
    /// O prompt nativo abriu: linha pendente.
    Open,
    /// A chamada rodou: a pendente dela vira aprovada.
    Approve,
    /// O turno seguiu sem o `PostToolUse`: toda pendente da sessão vira negada.
    DenyPending,
    None,
}

/// Sem efeito colateral, para ser testável.
///
/// `Open` segue exatamente o roteamento de [`hook_route`]: só o
/// `PermissionRequest` que vai ao prompt nativo abre linha — o do Codex vai ao
/// gate, que grava a própria linha `tyba`.
///
/// `Stop` **não** fecha: negar no prompt nativo não dispara `Stop` (medido no
/// B0), então ele nunca é o sinal de uma negação; e com subagente ou
/// ferramenta em paralelo um `Stop` poderia chegar com o prompt ainda aberto,
/// fechando como negado algo que o usuário ainda vai aprovar. Quem chega depois
/// de toda negação é o `UserPromptSubmit` da próxima mensagem.
fn inferred_step(runner: &AgentRunnerKind, hook_event_name: &str) -> InferredStep {
    if hook_route(runner, hook_event_name) == HookRoute::NativePrompt {
        return InferredStep::Open;
    }
    match hook_event_name {
        // Rodou — com ou sem erro, o usuário aprovou no prompt nativo.
        "PostToolUse" | "PostToolUseFailure" => InferredStep::Approve,
        "PreToolUse" | "UserPromptSubmit" | "SessionEnd" => InferredStep::DenyPending,
        _ => InferredStep::None,
    }
}

/// O que [`record_inferred`] gravou, para o histórico de notificações
/// acompanhar a mesma linha.
#[derive(Debug, Clone, PartialEq, Eq)]
enum InferredOutcome {
    /// Linha pendente nova: `id` em `approval_history`.
    Opened {
        id: i64,
        risk: RiskLevel,
        command: String,
    },
    /// A linha `id` fechou como aprovada.
    Approved {
        id: i64,
    },
    /// Pendentes da sessão fecharam como negadas.
    Denied,
    None,
}

/// Registra no histórico o que o prompt nativo do Claude decidiu, como
/// dedução. Falha de banco não derruba o hook: o histórico é registro, não
/// gate. Sem `HandlerCtx` (que exige `AppHandle`) para ser testável.
fn record_inferred(
    store: &Store,
    runner: &AgentRunnerKind,
    worktree_root: &Path,
    session_id: SessionId,
    event: &HookEvent,
    now: u64,
) -> InferredOutcome {
    let session_id = session_id.to_string();
    match inferred_step(runner, &event.hook_event_name) {
        InferredStep::Open => {
            // Só o texto e o risco; `|| false` porque a decisão híbrida não
            // importa aqui e não vale ler as configurações do Claude à toa.
            let plan = plan_pre_tool_use(
                runner,
                event.tool_name.as_deref().unwrap_or(""),
                event.tool_input.as_ref(),
                worktree_root,
                || false,
            );
            let command = plan.normalized.display_text();
            match store.insert_pending_inferred(
                &session_id,
                &command,
                event.cwd.as_deref(),
                risk_label(plan.risk),
                &call_key(&event.raw),
                now,
            ) {
                Ok(id) => InferredOutcome::Opened {
                    id,
                    risk: plan.risk,
                    command,
                },
                Err(_) => InferredOutcome::None,
            }
        }
        InferredStep::Approve => {
            match store.resolve_inferred_by_call_key(
                &session_id,
                &call_key(&event.raw),
                "approved",
                now,
            ) {
                Ok(Some(id)) => InferredOutcome::Approved { id },
                _ => InferredOutcome::None,
            }
        }
        InferredStep::DenyPending => {
            match store.deny_pending_inferred_for_session(&session_id, now) {
                Ok(n) if n > 0 => InferredOutcome::Denied,
                _ => InferredOutcome::None,
            }
        }
        InferredStep::None => InferredOutcome::None,
    }
}

/// Leva o desfecho da dedução ao histórico de notificações. O amarelo do
/// prompt nativo entra no histórico, mas continua decidido no terminal: aqui
/// só se grava, nunca se responde ao hook.
fn record_inferred_notification(
    n: &crate::notifications::Notifications,
    session_id: SessionId,
    outcome: &InferredOutcome,
) -> bool {
    match outcome {
        InferredOutcome::Opened { id, risk, command } => {
            n.native_prompt_opened(session_id, *id, *risk, command)
        }
        InferredOutcome::Approved { id } => n.native_prompt_approved(*id),
        InferredOutcome::Denied => n.native_prompts_denied(session_id),
        InferredOutcome::None => false,
    }
}

fn handle_event(ctx: &HandlerCtx, event: HookEvent) -> HookAction {
    capture_conversation_id(ctx, &event);
    // Antes do roteamento: o `PreToolUse` pode ficar bloqueado no gate de um
    // vermelho, e a pendente anterior tem de fechar já, não depois da decisão.
    let outcome = record_inferred(
        &ctx.store,
        &ctx.runner_kind,
        &ctx.worktree_root,
        ctx.session_id,
        &event,
        now_ms(),
    );
    crate::notifications::record(&ctx.app, |n| {
        record_inferred_notification(n, ctx.session_id, &outcome)
    });
    let signal = signal_for(&event.hook_event_name, event.notification_type.as_deref());
    capture_model(ctx, &event);
    if let Some(note) = note_for_event(signal.as_ref(), event.agent_id.is_some()) {
        ctx.sessions.set_status_note(&ctx.app, ctx.session_id, note);
    }
    match signal {
        Some(AgentSignal::Ready) => {
            let _ = ctx.app.emit(
                EVENT_AGENT_READY,
                AgentReadyPayload {
                    session_id: ctx.session_id,
                },
            );
        }
        Some(AgentSignal::SubagentStarted) => {
            if let Some(agent_id) = event.agent_id.clone() {
                let agent_type = event.agent_type.clone().unwrap_or_default();
                let parent = event
                    .raw
                    .get("transcript_path")
                    .and_then(|p| p.as_str())
                    .map(PathBuf::from);
                let coordination = ctx.subagents.on_subagent_started(
                    &ctx.app,
                    ctx.session_id,
                    agent_id,
                    agent_type,
                    None,
                    parent.as_deref(),
                );
                if let Some(coordination) = coordination {
                    crate::coordinate_subagent_viewer(&ctx.app, ctx.session_id, coordination);
                }
                let active = ctx.subagents.has_active(ctx.session_id);
                if let Some(next) = ctx
                    .sessions
                    .get(ctx.session_id)
                    .and_then(|s| status_on_subagent_start(&s.status, active))
                {
                    ctx.sessions.set_status(&ctx.app, ctx.session_id, next);
                    // Saiu de `Idle` por subagente, não pelo orquestrador.
                    mark_held_by_subagents(ctx);
                }
            }
        }
        Some(AgentSignal::SubagentEnded) => {
            if let Some(agent_id) = event.agent_id.clone() {
                let last_assistant_message = event
                    .raw
                    .get("last_assistant_message")
                    .and_then(|m| m.as_str())
                    .map(str::to_string);
                let idle = ctx.subagents.on_subagent_stopped(
                    &ctx.app,
                    ctx.session_id,
                    agent_id,
                    event.agent_type.clone(),
                    last_assistant_message,
                );
                // Só desce quem estava segurado em `Running`: um prompt
                // nativo aberto nesse meio-tempo continua mandando.
                let held = ctx
                    .sessions
                    .get(ctx.session_id)
                    .is_some_and(|s| matches!(s.status, SessionStatus::Running));
                if let (Some(summary), true) = (idle, held) {
                    ctx.sessions.set_status(
                        &ctx.app,
                        ctx.session_id,
                        SessionStatus::Idle { summary },
                    );
                }
            }
        }
        Some(AgentSignal::TurnFailed) => on_stop_failure(ctx, &event),
        Some(AgentSignal::Ended) => {
            if session_end_ends_subagents(&event.raw) {
                ctx.subagents.on_session_ended(&ctx.app, ctx.session_id);
            }
            // Claude hospedado num shell: o pane continua depois que ele sai,
            // e o painel não pode seguir mostrando o custo de uma conversa
            // que acabou. O `/clear` também zera — o próximo `statusLine`
            // repõe. Sessão de agente guarda o último número até morrer.
            if ctx
                .sessions
                .get(ctx.session_id)
                .is_some_and(|s| matches!(s.kind, SessionKind::Shell))
            {
                ctx.sessions.set_agent_usage(&ctx.app, ctx.session_id, None);
            }
            if let Some(status) = ctx
                .sessions
                .get(ctx.session_id)
                .and_then(|s| status_on_session_end(&s.status))
            {
                ctx.sessions.set_status(&ctx.app, ctx.session_id, status);
            }
        }
        Some(AgentSignal::Interrupted) => {
            use std::sync::atomic::Ordering;
            // Sem notificação: quem interrompeu foi o próprio usuário. Mesmo
            // cuidado do `Stop` com subagente vivo — a sessão segue `Running`
            // até o último terminar.
            ctx.turn_settle.fetch_add(1, Ordering::SeqCst);
            let summary = Some(INTERRUPTED_SUMMARY.to_string());
            if ctx
                .subagents
                .on_orchestrator_stop(&ctx.app, ctx.session_id, summary)
            {
                mark_held_by_subagents(ctx);
            } else {
                if let Some(status) = ctx
                    .sessions
                    .get(ctx.session_id)
                    .and_then(|s| status_on_interrupt(&s.status))
                {
                    // Antes do `set_status`: o `running → idle` da interrupção
                    // não é "terminou" no histórico.
                    crate::notifications::record(&ctx.app, |n| n.interrupted(ctx.session_id));
                    ctx.sessions.set_status(&ctx.app, ctx.session_id, status);
                }
            }
        }
        Some(signal) => {
            use std::sync::atomic::Ordering;
            match status_for(&signal) {
                Some(SessionStatus::Running) => {
                    // Ferramenta de subagente (`agent_id` no payload) não é o
                    // orquestrador voltando a trabalhar: o turno dele segue
                    // encerrado, e o motivo do `Running` continua sendo os
                    // subagentes.
                    let from_subagent = event.agent_id.is_some();
                    if !from_subagent {
                        ctx.subagents.note_orchestrator_working(ctx.session_id);
                        ctx.sessions
                            .set_subagents_working(&ctx.app, ctx.session_id, false);
                        ctx.turn_settle.fetch_add(1, Ordering::SeqCst);
                    }
                    ctx.sessions
                        .set_status(&ctx.app, ctx.session_id, SessionStatus::Running);
                    if label_held_after_running(
                        from_subagent,
                        ctx.subagents.orchestrator_idle(ctx.session_id),
                    ) {
                        mark_held_by_subagents(ctx);
                    }
                }
                Some(status @ SessionStatus::AwaitingInput { .. }) => {
                    // O `idle_prompt` não rebaixa o prompt nativo a `Reply`: a
                    // pergunta de permissão continua aberta no terminal.
                    let next = match ctx.sessions.get(ctx.session_id) {
                        Some(session) => status_on_idle_prompt(&session.status),
                        None => Some(status),
                    };
                    if let Some(next) = next {
                        ctx.sessions.set_status(&ctx.app, ctx.session_id, next);
                        notify_awaiting_input(ctx, "Agente aguardando sua resposta");
                    }
                }
                Some(SessionStatus::Idle { .. }) => {
                    let turn = turn_summary(&event);
                    let summary = turn.text;
                    let needs_settle = !turn.settled;
                    // Subagente ativo segura a sessão em Running mesmo com o
                    // turno do orquestrador encerrado — com subagente
                    // trabalhando a sessão não está ociosa (decisão 4 revista).
                    // Só desce a Idle quando o último termina (hook, sidecar
                    // ou notificação na fila do pai).
                    if ctx
                        .subagents
                        .on_orchestrator_stop(&ctx.app, ctx.session_id, summary.clone())
                    {
                        ctx.turn_settle.fetch_add(1, Ordering::SeqCst);
                        if let Some(next) = ctx
                            .sessions
                            .get(ctx.session_id)
                            .and_then(|s| status_on_held_stop(&s.status))
                        {
                            ctx.sessions.set_status(&ctx.app, ctx.session_id, next);
                        }
                        mark_held_by_subagents(ctx);
                    } else {
                        ctx.sessions.set_status(
                            &ctx.app,
                            ctx.session_id,
                            SessionStatus::Idle { summary },
                        );
                        let transcript_path = event
                            .raw
                            .get("transcript_path")
                            .and_then(|p| p.as_str())
                            .map(str::to_string);
                        notify_turn_ended(ctx, transcript_path, needs_settle);
                    }
                }
                _ => {}
            }
        }
        None => {}
    }

    match session_route(&ctx.runner_kind, &event.hook_event_name, ctx.observe_only) {
        HookRoute::OpenUrl => handle_open_url(ctx, &event),
        HookRoute::Usage => on_statusline(ctx, &event),
        HookRoute::ToolUse => on_pre_tool_use(ctx, &event),
        HookRoute::Observe => on_observed_tool_use(ctx, &event),
        HookRoute::NativePrompt => on_native_prompt(ctx, &event),
        HookRoute::Ack => HookAction::Ack,
    }
}

/// O turno acabou por erro da API. O status vira `Idle` com o motivo (ver
/// [`status_on_stop_failure`]), e o aviso sai pelos dois caminhos de
/// sempre: o toast no app (`agent://alert`) e o aviso do sistema, que só sai
/// fora de foco e respeita a preferência de "pedido" — o agente parou e
/// precisa de alguém.
fn on_stop_failure(ctx: &HandlerCtx, event: &HookEvent) {
    use std::sync::atomic::Ordering;

    let error = crate::agent::usage::stop_failure_error(&event.raw);
    let reason = crate::agent::usage::stop_failure_reason(error.as_deref());
    let status = status_on_stop_failure(reason);
    let SessionStatus::Idle { summary } = &status else {
        return;
    };
    // Descarta placeholder não confirmado; o retorno não importa — o status
    // da falha prevalece.
    ctx.subagents
        .on_orchestrator_stop(&ctx.app, ctx.session_id, summary.clone());
    // Cancela um aviso de "terminou" que um `Stop` anterior tenha agendado.
    ctx.turn_settle.fetch_add(1, Ordering::SeqCst);
    // Antes do `set_status`: o `running → idle` que ele causa não é "terminou".
    crate::notifications::record(&ctx.app, |n| n.stop_failure(ctx.session_id, reason));
    ctx.sessions.set_status(&ctx.app, ctx.session_id, status);
    let _ = ctx.app.emit(
        EVENT_AGENT_ALERT,
        AgentAlert::StopFailure {
            session_id: ctx.session_id,
            error: error.unwrap_or_else(|| "unknown".into()),
            reason: reason.to_string(),
        },
    );
    notify_native(
        &ctx.app,
        &ctx.sessions,
        &ctx.store,
        notify::NotifyKind::Request,
        ctx.session_id,
        &format!("Parou: {reason}"),
    );
}

/// O uso que o `_statusline` mandou. Dado de dentro da jaula: passa de novo
/// por [`AgentUsage::sanitized`]. Cruzou 85% numa janela de limite → aviso,
/// uma vez por cruzamento (ver [`crate::agent::usage::LimitTracker`]).
fn on_statusline(ctx: &HandlerCtx, event: &HookEvent) -> HookAction {
    let Some(usage) = event
        .raw
        .get("usage")
        .cloned()
        .and_then(|u| serde_json::from_value::<AgentUsage>(u).ok())
        .map(AgentUsage::sanitized)
    else {
        return HookAction::Ack;
    };
    let warnings = crate::agent::usage::observe_limits(&usage);
    crate::notifications::record(&ctx.app, |n| n.usage(ctx.session_id, &usage));
    ctx.sessions
        .set_agent_usage(&ctx.app, ctx.session_id, Some(usage));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    for w in warnings {
        let _ = ctx.app.emit(
            EVENT_AGENT_ALERT,
            AgentAlert::UsageLimit {
                session_id: ctx.session_id,
                window: w.window,
                pct: w.pct,
                resets_at: w.resets_at,
            },
        );
        notify_native(
            &ctx.app,
            &ctx.sessions,
            &ctx.store,
            notify::NotifyKind::Request,
            ctx.session_id,
            &crate::agent::usage::limit_warning_text(&w, now),
        );
    }
    HookAction::Ack
}

/// Entrega B (§1.2/§5): a validação da URL é do core, nunca do shim — o shim
/// roda dentro da jaula e o agente pode falar com o socket direto. Ack só
/// depois de validar; deny cai no fallback de tela do próprio Claude (V4),
/// nunca num estado pior que o de hoje.
fn handle_open_url(ctx: &HandlerCtx, event: &HookEvent) -> HookAction {
    let Some(raw) = event.raw.get("url").and_then(|u| u.as_str()) else {
        return HookAction::Deny {
            reason: "TybaOpenUrl sem url".into(),
        };
    };
    match crate::agent::browser_bridge::validate_open_url(raw) {
        Ok(v) => {
            let _ = ctx.app.emit(
                crate::agent::browser_bridge::EVENT_AGENT_OPEN_URL,
                crate::agent::browser_bridge::OpenUrlPayload {
                    session_id: ctx.session_id,
                    url: v.url,
                    host: v.host,
                    known_login: v.known_login,
                },
            );
            HookAction::Ack
        }
        Err(reason) => HookAction::Deny { reason },
    }
}

fn runtime_dir(id: SessionId) -> Result<PathBuf, String> {
    let short = id.simple().to_string();
    let dir = std::env::temp_dir().join(format!("tyba-{}", &short[..12]));
    crate::session::create_private_dir(&dir)
        .and_then(|_| crate::session::verify_private_dir(&dir))
        .map_err(|e| format!("dir privado da sessão de agente: {e}"))?;
    Ok(dir)
}

fn consented_config(
    store: &Store,
    root: &std::path::Path,
) -> Option<crate::repo_config::RepoConfig> {
    let (config, hash) = crate::repo_config::load(root).ok().flatten()?;
    let repo_key = crate::repo::canonicalize_or(root)
        .to_string_lossy()
        .into_owned();
    (store.config_consent(&repo_key, &hash).ok().flatten() == Some(true)).then_some(config)
}

pub fn create_agent_session(
    ctx: &AgentSessionCtx,
    opts: CreateSessionOpts,
    on_exit: impl FnOnce(SessionId) + Send + 'static,
) -> Result<Session, String> {
    let SessionKind::Agent {
        runner: runner_kind,
    } = opts.kind.clone()
    else {
        return Err("sessão de agente exige kind agent".into());
    };
    let runner = build_runner(&runner_kind)?;
    let task = opts
        .worktree_task
        .clone()
        .ok_or("sessão de agente exige worktree")?;

    let base = crate::session::resolve_cwd(opts.cwd.as_deref());
    let root = crate::repo::toplevel(&base).ok_or("a pasta da sessão não é um repositório git")?;
    let root = crate::repo::canonicalize_or(&root);
    let worktree = crate::worktree::create(&root, &task)?;

    let result = spawn_prepared(
        ctx,
        SessionId::new_v4(),
        opts,
        runner,
        root,
        worktree.clone(),
        task,
        None,
        on_exit,
    );
    if result.is_err() {
        let _ = crate::worktree::remove(&worktree.path, true, true);
    }
    result
}

/// Sobe um agente numa pasta que **já existe** (o worktree de uma sessão, o repo
/// conflitado) em vez de criar um worktree novo.
///
/// Existe para que os fluxos de review e de resolução de conflitos passem pelo
/// mesmo `spawn_prepared` das sessões normais. Antes o front criava uma sessão de
/// **shell** e digitava `claude` nela: o agente subia sem sandbox, com o env
/// inteiro do usuário e — pior — **sem os hooks**, logo sem `PreToolUse`, logo
/// sem gate de aprovação. Um agente fora do inbox faz o que quiser.
pub fn attach_agent_session(
    ctx: &AgentSessionCtx,
    opts: CreateSessionOpts,
    on_exit: impl FnOnce(SessionId) + Send + 'static,
) -> Result<Session, String> {
    let SessionKind::Agent {
        runner: runner_kind,
    } = opts.kind.clone()
    else {
        return Err("sessão de agente exige kind agent".into());
    };
    let runner = build_runner(&runner_kind)?;

    let path = opts
        .cwd
        .clone()
        .ok_or("sessão de agente in-place exige a pasta alvo")?;
    let worktree = crate::worktree::existing(&path)?;
    let root = crate::repo::toplevel(&worktree.path)
        .map(|r| crate::repo::canonicalize_or(&r))
        .ok_or("a pasta da sessão não é um repositório git")?;

    let task = opts
        .worktree_task
        .clone()
        .or_else(|| opts.title.clone())
        .unwrap_or_else(|| worktree.branch.clone());
    let resume = attach_resume(opts.resume_conversation.as_deref(), runner.as_ref())?;

    // Sem `worktree::remove` no erro: a pasta é do usuário, não nossa.
    spawn_prepared(
        ctx,
        SessionId::new_v4(),
        opts,
        runner,
        root,
        worktree,
        task,
        resume,
        on_exit,
    )
}

/// O id de conversa que o attach do gate pede para retomar, já validado. Ele
/// volta do webview, então é dado não confiável: só passa o que
/// [`crate::agent::conversation::is_plausible`] aceita — nada com espaço,
/// aspas ou `-` inicial chega ao argv. Implausível recusa o spawn em vez de
/// cair calado numa sessão vazia (o core devolveu um id válido; um inválido
/// aqui é adulteração ou bug). Runner que não retoma por id ignora.
fn attach_resume(
    requested: Option<&str>,
    runner: &dyn AgentRunner,
) -> Result<Option<String>, String> {
    let Some(id) = requested else {
        return Ok(None);
    };
    if !crate::agent::conversation::is_plausible(id) {
        return Err("id de conversa inválido para retomar".into());
    }
    Ok(runner.resumes_conversations().then(|| id.to_string()))
}

// A partir daqui, e até o fim de `prepare_hosted_agent`, todo o código é
// exclusivo do canal shim↔core (shim v2, passo 2) — Linux e, desde o M3 do
// claude-monitor, macOS (mesmo gate de `hook_ipc::channel`/
// `agent::channel_host`). Nunca o caminho gerenciado normal (`spawn_prepared`
// e o resto deste arquivo).
#[cfg(any(target_os = "linux", target_os = "macos"))]
const LAUNCH_PLAN_FILE: &str = "launch.json";

/// O que o TYBA tira do env do shell do pane antes de subir o claude
/// digitado — só o que desligaria os hooks dele (`CLAUDE_CODE_SIMPLE` é o
/// modo `--bare` e `CLAUDE_CODE_SAFE_MODE` é o `--safe-mode`: os dois pulam
/// os hooks do `--settings`, e com eles o guarda-corpo; por isso as flags já
/// são recusadas) ou mentiria sobre a rota (`TYBA_SHIM` marca claude cru,
/// "sem gate").
#[cfg(any(target_os = "linux", target_os = "macos"))]
const HOSTED_ENV_REMOVE: &[&str] = &[
    "CLAUDE_CODE_SIMPLE",
    "CLAUDE_CODE_SAFE_MODE",
    crate::hook_ipc::channel::SHIM_ENV,
];

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct HostedCommand {
    cmd: portable_pty::CommandBuilder,
    /// Raiz que o histórico deduzido do prompt nativo usa para rotular a
    /// chamada (`HandlerCtx.worktree_root`): o toplevel git da cwd, ou a
    /// própria cwd fora de repositório.
    worktree_root: PathBuf,
}

/// A spec que acompanha o `wrap` do passthrough. Ele não a lê — o agente
/// livre não tem jaula —, mas o spawn continua passando pela trait
/// `Sandbox` (princípio 2), e a trait pede uma.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn passthrough_spec(cwd: &Path, runtime: &Path, socket_path: &Path, exe: &Path) -> SandboxSpec {
    SandboxSpec {
        writable_root: cwd.to_path_buf(),
        readable_root: cwd.to_path_buf(),
        allow_network: true,
        repo_git_dir: PathBuf::new(),
        worktree_git_dir: PathBuf::new(),
        runtime_dir: runtime.to_path_buf(),
        hook_socket: socket_path.to_path_buf(),
        tyba_exe: exe.to_path_buf(),
        tyba_data_dir: PathBuf::new(),
        home: PathBuf::new(),
        tmpdir: None,
        exec_path_dirs: Vec::new(),
        agent: Default::default(),
        read_allow_extra: Vec::new(),
        data_dir_reads: Vec::new(),
    }
}

/// A parte de [`prepare_hosted_agent`] que NÃO precisa de `AgentSessionCtx`
/// (logo, nem de `AppHandle`): monta o `argv` e o env que o TYBA acrescenta.
/// Agente livre: sem jaula (o `Sandbox` é o passthrough) e sem env próprio —
/// o claude herda o env do shell do pane no próprio shim
/// ([`crate::hook_ipc::channel::hosted_child_env`]); o plano só carrega o
/// socket de hooks e os marcadores.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn build_hosted_command(
    runner: &dyn AgentRunner,
    cwd: &Path,
    hook_setup: &HookSetup,
    socket_path: &Path,
    runtime: &Path,
    exe: &Path,
    args: &[String],
) -> Result<HostedCommand, String> {
    use crate::sandbox::Sandbox;

    let worktree_root = crate::repo::toplevel(cwd)
        .map(|top| crate::repo::canonicalize_or(&top))
        .unwrap_or_else(|| cwd.to_path_buf());
    // Agente digitado no shell: o modelo é o que o usuário passou nos args, não
    // o de uma launch config. Env vazio aqui: o resto vem do pane.
    let mut cmd = runner.build_command(cwd, &HashMap::new(), hook_setup, None, None);
    // M6-A: os args que o usuário digitou (`-c`, `--resume <id>`, `-p ...`),
    // já reclassificados pelo `channel_host`, entram como argv depois do
    // `--settings` dos hooks — nunca por shell.
    cmd.args(args);
    cmd.env("TYBA_HOOK_SOCKET", socket_path);
    // Track C consome estes via `/proc/<agente>/environ`.
    cmd.env("TYBA_HOSTED", "1");
    cmd.env("TYBA_JAILED", "0");
    let spec = passthrough_spec(cwd, runtime, socket_path, exe);
    let cmd = crate::sandbox::passthrough::PassthroughSandbox.wrap(cmd, &spec)?;
    Ok(HostedCommand { cmd, worktree_root })
}

/// `argv`/`env`/`cwd` já resolvidos, na forma que `HostedPlan` serializa —
/// só o mapeamento, sem decisão nenhuma: `get_argv()` inclui o programa em
/// `[0]`, `iter_extra_env_as_str()` devolve só o que o TYBA setou (nunca o
/// env do processo core).
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn hosted_plan_from(
    cmd: &portable_pty::CommandBuilder,
    cwd: &Path,
) -> crate::hook_ipc::channel::HostedPlan {
    crate::hook_ipc::channel::HostedPlan {
        argv: cmd
            .get_argv()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect(),
        env: cmd
            .iter_extra_env_as_str()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        env_remove: HOSTED_ENV_REMOVE.iter().map(|k| k.to_string()).collect(),
        cwd: cwd.to_string_lossy().into_owned(),
    }
}

/// O `claude` digitado num pane (canal shim↔core, tech-spec §5), no modelo
/// do agente livre: sobe SEM jaula, com o env do shell do pane, e com os
/// hooks da sessão por `--settings` — que só observam (monitor, conversa,
/// subagentes, status) e aplicam o guarda-corpo fixo no `PreToolUse`
/// (`tyba _hook --observe`). Nenhum card de aprovação: quem pede permissão é
/// o prompt do próprio Claude. Aqui não sobe nada; quem executa é o PRÓPRIO
/// processo do shim, lendo o `launch.json` que esta função escreve e ele
/// apaga.
///
/// `id` é o `SessionId` da sessão de SHELL dona da conexão — nunca um id
/// novo — porque é o que faz o monitor (e todo o resto do hook) ficar
/// atribuído à sessão que o usuário já está olhando.
///
/// Devolve o caminho do plano escrito (0600 via [`crate::session::write_private_bytes`]).
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn prepare_hosted_agent(
    ctx: &AgentSessionCtx,
    id: SessionId,
    cwd: PathBuf,
    args: &[String],
) -> Result<PathBuf, String> {
    let runtime = runtime_dir(id)?;
    let socket_path = runtime.join(HOOK_SOCKET_FILE);
    if socket_path.as_os_str().len() > MAX_SOCKET_PATH {
        return Err("path do socket de hooks excede o limite do sistema".into());
    }

    let runner = build_runner(&AgentRunnerKind::ClaudeCode)?;
    let exe = std::env::current_exe().map_err(|e| format!("exe do Tyba: {e}"))?;
    let hook_cmd = observer_hook_command(&exe);
    let user_env: HashMap<String, String> = std::env::vars().collect();
    let settings = observer_session_settings(
        &hook_cmd,
        session_statusline(&exe, &user_env, &[&cwd, &main_repo_root(&cwd)]),
    );
    let settings_body =
        serde_json::to_string_pretty(&settings).map_err(|e| format!("settings de hooks: {e}"))?;
    crate::session::write_private(&runtime, HOOK_SETTINGS_FILE, &settings_body)
        .map_err(|e| format!("escrita dos settings de hooks: {e}"))?;
    let hook_setup = HookSetup {
        settings_path: runtime.join(HOOK_SETTINGS_FILE),
        hook_command: hook_cmd,
    };

    let HostedCommand { cmd, worktree_root } = build_hosted_command(
        runner.as_ref(),
        &cwd,
        &hook_setup,
        &socket_path,
        &runtime,
        &exe,
        args,
    )?;

    ctx.subagents.register_session(id);
    let handler_ctx = HandlerCtx {
        app: ctx.app.clone(),
        sessions: ctx.sessions.clone(),
        approvals: ctx.approvals.clone(),
        store: ctx.store.clone(),
        subagents: ctx.subagents.clone(),
        session_id: id,
        runner_kind: AgentRunnerKind::ClaudeCode,
        repo_root: main_repo_root(&worktree_root),
        agent_home: agent_home(&user_env),
        worktree_root,
        turn_settle: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        seen_transcript: Arc::new(Mutex::new(None)),
        observe_only: true,
    };
    // Review round 1, achado BLOQUEANTE: derruba o HookServer ANTIGO desta
    // sessão ANTES de religar em `socket_path` — nunca depois. `runtime_dir(id)`
    // é determinístico por `SessionId`: hospedar `claude` de novo no MESMO pane
    // (sem que o hospedado anterior tenha passado por `on_exit` nenhum — ele
    // não tem, ao contrário do caminho gerenciado, que já derruba o servidor em
    // `teardown_agent_session` antes de qualquer resume) reusa o MESMO
    // `socket_path`. Se o `HookServer::bind` novo acontecesse primeiro, o
    // `remove_file`+`UnixListener::bind` dele rouba o path ANTES de o antigo
    // ser avisado: o `UnixStream::connect` do `stop()` do antigo acordaria o
    // listener NOVO (mesmo path, inode diferente), nunca a accept-loop antiga
    // — que travaria para sempre em `accept()`, e o `join()` de `stop()` nunca
    // retornaria. Derrubar antes garante que o `connect` ainda acorda o dono
    // certo do path.
    ctx.servers.shutdown(id);
    let server = HookServer::bind(
        &socket_path,
        Arc::new(move |event| handle_event(&handler_ctx, event)),
    )
    .map_err(|e| format!("socket de hooks: {e}"))?;
    ctx.servers.insert(id, server);

    let plan = hosted_plan_from(&cmd, &cwd);
    let plan_body = serde_json::to_vec(&plan).map_err(|e| format!("serialização do plano: {e}"))?;
    if let Err(e) = crate::session::write_private_bytes(&runtime, LAUNCH_PLAN_FILE, &plan_body) {
        ctx.servers.shutdown(id);
        return Err(format!("escrita do plano de execução: {e}"));
    }
    Ok(runtime.join(LAUNCH_PLAN_FILE))
}

/// Sobe o agente de novo na conversa nativa que a sessão morta deixou no disco.
///
/// **Nunca automático.** Só chega aqui por clique explícito: retomar levanta um
/// processo com contexto que pode voltar a agir, e ação de agente não começa sem
/// intenção humana — por isso o `resume_startup` continua devolvendo a sessão de
/// agente morta, e o convite é do dono aceitar ou não.
///
/// Reaproveita o **mesmo `SessionId`**: é a mesma conversa, e é o que faz o pane
/// restaurado voltar a viver onde está, sem o layout ter de reapontar nada.
///
/// Falha fechado. Sem id de conversa, sem runner que saiba retomar, sem binário
/// ou sem a pasta, devolve `Err` e nada sobe — um agente meio retomado seria
/// pior que nenhum.
pub fn resume_agent_session(
    ctx: &AgentSessionCtx,
    session: &Session,
    cols: u16,
    rows: u16,
    on_exit: impl FnOnce(SessionId) + Send + 'static,
) -> Result<Session, String> {
    let target = resume_target(session)?;
    let runner = build_runner(&target.runner_kind)?;
    let worktree = crate::worktree::existing(&target.path)?;
    let root = crate::repo::toplevel(&worktree.path)
        .map(|r| crate::repo::canonicalize_or(&r))
        .ok_or("a pasta da sessão não é um repositório git")?;
    let task = worktree.branch.clone();
    let opts = CreateSessionOpts {
        kind: session.kind.clone(),
        title: Some(session.title.clone()),
        cwd: Some(target.path.clone()),
        cols,
        rows,
        worktree_task: None,
        attach_existing: true,
        shell: None,
        inherit_cwd_from: None,
        initial_prompt: None,
        // Nunca `true` aqui: a origem do gate não é desta chamada, é da
        // sessão anterior que ocupava o mesmo `SessionId` — `spawn_session`
        // é quem herda de `previous`. Ver `Session::opened_by_gate`.
        opened_by_gate: false,
        resume_conversation: None,
        agent_model: None,
        prepared_worktree: None,
    };
    spawn_prepared(
        ctx,
        session.id,
        opts,
        runner,
        root,
        worktree,
        task,
        Some(target.conversation_id),
        on_exit,
    )
}

struct ResumeTarget {
    runner_kind: AgentRunnerKind,
    conversation_id: String,
    path: PathBuf,
}

/// Tudo que retomar exige, ou o motivo de não dar. Chamado tanto pelo
/// [`resume_agent_session`] quanto pelo [`can_resume`], para que o convite na
/// tela e o que acontece no clique respondam à MESMA pergunta.
fn resume_target(session: &Session) -> Result<ResumeTarget, String> {
    let SessionKind::Agent {
        runner: runner_kind,
    } = session.kind.clone()
    else {
        return Err("retomar conversa só vale para sessão de agente".into());
    };
    let conversation_id = session
        .agent_conversation_id
        .clone()
        .filter(|id| crate::agent::conversation::is_plausible(id))
        .ok_or("a sessão não tem conversa nativa registrada")?;
    // `build_runner` recusa runner custom e binário fora do PATH — as duas
    // razões pelas quais o clique falharia depois de o convite ter aparecido.
    if !build_runner(&runner_kind)?.resumes_conversations() {
        return Err("este runner não retoma conversa por id".into());
    }
    let path = session
        .worktree
        .as_ref()
        .map(|w| w.path.clone())
        .or_else(|| session.cwd.clone())
        .ok_or("a sessão não guarda a pasta em que rodava")?;
    if !path.is_dir() {
        return Err("a pasta da sessão não existe mais".into());
    }
    Ok(ResumeTarget {
        runner_kind,
        conversation_id,
        path,
    })
}

/// Se o convite de retomar deve aparecer. Falso é o silêncio da decisão 3:
/// convite que leva a erro é pior que ausência de convite.
pub fn can_resume(session: &Session) -> bool {
    resume_target(session).is_ok()
}

fn build_runner(kind: &AgentRunnerKind) -> Result<Box<dyn AgentRunner>, String> {
    let runner: Box<dyn AgentRunner> = match kind {
        AgentRunnerKind::ClaudeCode => Box::new(ClaudeCodeRunner),
        AgentRunnerKind::Codex => Box::new(CodexRunner),
        AgentRunnerKind::Custom(_) => {
            return Err("runner custom disponível em fase futura".into());
        }
    };
    if !crate::agent::binary_available(kind) {
        let binary = crate::agent::runner_binary(kind).unwrap_or("?");
        return Err(format!("binário `{binary}` não encontrado no PATH"));
    }
    Ok(runner)
}

fn apply_git_overrides(env: &mut HashMap<String, String>) {
    let overrides = [("commit.gpgsign", "false"), ("tag.gpgsign", "false")];
    env.insert("GIT_CONFIG_COUNT".into(), overrides.len().to_string());
    for (i, (key, value)) in overrides.iter().enumerate() {
        env.insert(format!("GIT_CONFIG_KEY_{i}"), (*key).into());
        env.insert(format!("GIT_CONFIG_VALUE_{i}"), (*value).into());
    }
}

fn default_data_dir(env: &HashMap<String, String>, home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let _ = env;
        home.join("Library/Application Support")
            .join(crate::profile::IDENTIFIER)
    }
    #[cfg(not(target_os = "macos"))]
    {
        env.get("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"))
            .join(crate::profile::IDENTIFIER)
    }
}

/// `HOME` que o agente recebe (o mesmo que `sandbox_spec` exige): é de onde o
/// Claude da sessão lê `~/.claude`.
/// O `statusLine` da sessão: o `tyba _statusline`, carregando a statusline
/// efetiva do usuário (lida do `HOME` do agente e das pastas do projeto) para
/// que ela continue aparecendo — ver [`crate::agent::statusline`].
fn session_statusline(
    exe: &Path,
    env: &HashMap<String, String>,
    project_dirs: &[&Path],
) -> serde_json::Value {
    let home = agent_home(env);
    let user = crate::agent::statusline::read_user_statusline(home.as_deref(), project_dirs);
    crate::agent::statusline::statusline_settings(exe, user.as_ref())
}

fn agent_home(env: &HashMap<String, String>) -> Option<PathBuf> {
    env.get("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
}

/// Raiz do repositório principal de `path` (worktree ou checkout). Fora de um
/// repositório git, a própria pasta.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn main_repo_root(path: &Path) -> PathBuf {
    crate::worktree::main_repo_of(path)
        .map(|r| crate::repo::canonicalize_or(&r))
        .unwrap_or_else(|_| path.to_path_buf())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn sandbox_spec(
    runner: &dyn AgentRunner,
    env: &HashMap<String, String>,
    root: &Path,
    worktree: &Path,
    runtime: &Path,
    socket_path: &Path,
    exe: &Path,
    conversation: Option<&str>,
) -> Result<SandboxSpec, String> {
    let home = env
        .get("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
        .ok_or("HOME indisponível — sandbox exige a home do usuário")?;
    // M7: a jaula não depende de git. Os dirs do git só entram quando o
    // repositório é DESTA jaula (toplevel dentro de `root` ou do worktree da
    // sessão); pasta sem git, ou um repo maior que a jaula (o `~/.git` vazio
    // que faz a home inteira virar toplevel), usa um gitdir inerte no runtime
    // privado da sessão — mesmo padrão do LSP — e nunca concede escrita num
    // `.git` fora da jaula.
    let inside = |top: &Path| {
        let top = crate::repo::canonicalize_or(top);
        top.starts_with(crate::repo::canonicalize_or(root))
            || top.starts_with(crate::repo::canonicalize_or(worktree))
    };
    let git_dirs = match crate::repo::toplevel(worktree) {
        Some(top) if inside(&top) => crate::worktree::resolved_git_dirs(worktree)?,
        _ => {
            let inert = runtime.join(".no-git");
            crate::worktree::WorktreeGitDirs {
                git_dir: inert.clone(),
                common_dir: inert,
            }
        }
    };
    let mut exec_path_dirs: Vec<PathBuf> =
        std::env::split_paths(&crate::shell_path::agent_path()).collect();
    if let Some(parent) = crate::agent::resolved_binary(&runner.kind())
        .as_deref()
        .and_then(Path::parent)
    {
        exec_path_dirs.push(parent.to_path_buf());
    }
    let mut agent = runner.sandbox_access(&home, worktree);
    if let Some(id) = conversation.filter(|id| crate::agent::conversation::is_plausible(id)) {
        let own = runner.conversation_access(&home, id);
        agent.read.extend(own.read);
        agent.write.extend(own.write);
    }
    // Config quebrado ou recusado não impede a sessão: sem o `read_allow` a
    // jaula só fica mais fechada. O toast do mesmo arquivo já sai pelo
    // ConfigManager; aqui basta o log.
    let read_allow_extra = match crate::user_config::load(&home) {
        Ok(c) => c.sandbox_read_allow,
        Err(e) => {
            eprintln!("[tyba] sandbox: sessão sem o read_allow do usuário ({e})");
            Vec::new()
        }
    };
    let tyba_data_dir = std::env::var_os("TYBA_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_data_dir(env, &home));
    Ok(SandboxSpec {
        writable_root: worktree.to_path_buf(),
        readable_root: root.to_path_buf(),
        allow_network: runner.needs_network(),
        repo_git_dir: git_dirs.common_dir,
        worktree_git_dir: git_dirs.git_dir,
        runtime_dir: runtime.to_path_buf(),
        hook_socket: socket_path.to_path_buf(),
        tyba_exe: crate::repo::canonicalize_or(exe),
        tyba_data_dir,
        home,
        tmpdir: env.get("TMPDIR").map(PathBuf::from),
        exec_path_dirs,
        agent,
        read_allow_extra,
        data_dir_reads: vec![],
    })
}

#[allow(clippy::too_many_arguments)]
fn spawn_prepared(
    ctx: &AgentSessionCtx,
    id: SessionId,
    opts: CreateSessionOpts,
    runner: Box<dyn AgentRunner>,
    root: PathBuf,
    worktree: crate::worktree::Worktree,
    task: String,
    resume: Option<String>,
    on_exit: impl FnOnce(SessionId) + Send + 'static,
) -> Result<Session, String> {
    let runtime = runtime_dir(id)?;
    let socket_path = runtime.join(HOOK_SOCKET_FILE);
    if socket_path.as_os_str().len() > MAX_SOCKET_PATH {
        return Err("path do socket de hooks excede o limite do sistema".into());
    }

    let exe = std::env::current_exe().map_err(|e| format!("exe do Tyba: {e}"))?;
    let hook_cmd = hook_command(&exe);
    if matches!(runner.kind(), AgentRunnerKind::ClaudeCode) {
        let settings = claude_session_settings(
            &hook_cmd,
            session_statusline(&exe, &std::env::vars().collect(), &[&worktree.path, &root]),
        );
        let settings_body = serde_json::to_string_pretty(&settings)
            .map_err(|e| format!("settings de hooks: {e}"))?;
        crate::session::write_private(&runtime, HOOK_SETTINGS_FILE, &settings_body)
            .map_err(|e| format!("escrita dos settings de hooks: {e}"))?;
    }
    let hook_setup = HookSetup {
        settings_path: runtime.join(HOOK_SETTINGS_FILE),
        hook_command: hook_cmd,
    };

    let user_env: HashMap<String, String> = std::env::vars().collect();
    let config = consented_config(&ctx.store, &root);
    let mut env = crate::repo_config::agent_env(config.as_ref(), &user_env);
    apply_git_overrides(&mut env);

    let mut cmd = runner.build_command(
        &worktree.path,
        &env,
        &hook_setup,
        resume.as_deref(),
        opts.agent_model.as_deref(),
    );
    // M5 do claude-monitor: a jaula só libera o `file-history/<sessão>` da
    // conversa desta sessão, então o id precisa ser conhecido no spawn. Retomar
    // já o traz; conversa nova do Claude ganha um UUID nosso via
    // `--session-id` (gerado aqui, nunca vindo de fora).
    let pinned = (resume.is_none() && matches!(runner.kind(), AgentRunnerKind::ClaudeCode))
        .then(|| uuid::Uuid::new_v4().to_string());
    if let Some(id) = &pinned {
        cmd.arg("--session-id");
        cmd.arg(id);
    }
    let conversation = resume.as_deref().or(pinned.as_deref());
    cmd.env("TYBA_HOOK_SOCKET", &socket_path);
    // Entrega B (§5.2): setado pra TODA sessão de agente, sem branch por
    // runner — bônus por construção (gh/vercel login também passam a
    // funcionar); só o fluxo do Claude está sob a promessa. Falha silenciosa
    // aqui é aceitável (fs quebrado no runtime_dir já quebraria a sessão em
    // outro lugar) e não piora nada: sem BROWSER, o Claude só volta a cair no
    // fallback de tela de hoje.
    if let Ok(shim) = crate::agent::browser_bridge::write_browser_shim(&runtime, &exe) {
        crate::agent::browser_bridge::set_browser_env(&mut cmd, &shim);
    }
    // Camada A do Windows (Opção B): a jaula se aplica no spawn (token + ConPTY),
    // não por `wrap`. `jailed_spawner` devolve `Some` só no Windows; mac/linux
    // seguem reescrevendo argv via `wrap` e `jail` fica `None`.
    let (cmd, jail) = if runner.self_sandboxes() {
        (cmd, None)
    } else {
        let sandbox = crate::sandbox::platform_sandbox()?;
        let spec = sandbox_spec(
            runner.as_ref(),
            &env,
            &root,
            &worktree.path,
            &runtime,
            &socket_path,
            &exe,
            conversation,
        )?;
        // Entrega B: só no Linux, só pro Claude Code — é onde a credencial
        // depende da inversão da política (§2). Nunca bloqueia o spawn (§6).
        #[cfg(target_os = "linux")]
        if matches!(runner.kind(), AgentRunnerKind::ClaudeCode) {
            crate::agent::credentials::emit_warnings(&ctx.app, id, &env, &spec, &ctx.store);
        }
        // Entrega C, mecanismo 1 (preflight): mesmo ponto/condição do
        // `emit_warnings` acima, mas CROSS-PLATFORM (sem `cfg(linux)`) — a
        // ausência de login não é um problema só da jaula do Linux — e
        // ASSÍNCRONO: `spawn_preflight` devolve na hora, o trabalho de
        // verdade corre numa thread à parte e nunca atrasa a subida da
        // sessão (P3 do contrato de cobertura).
        //
        // Review de segurança round 2, REQUERIDO: SEM `worktree.path` aqui
        // -- o probe roda fora da jaula, e apontar o cwd pra área
        // não-confiável do repo seria exposição desnecessária de um
        // processo sem sandbox a config de projeto que o dono pode não ter
        // escrito. `spawn_preflight` resolve o próprio cwd neutro por
        // dentro (`neutral_preflight_cwd`); não recebe mais o worktree.
        if matches!(runner.kind(), AgentRunnerKind::ClaudeCode) {
            crate::agent::auth_preflight::spawn_preflight(
                ctx.app.clone(),
                id,
                crate::agent::resolved_binary(&AgentRunnerKind::ClaudeCode),
                env.clone(),
            );
        }
        match sandbox.jailed_spawner(&spec)? {
            Some(spawner) => (cmd, Some(spawner)),
            None => (sandbox.wrap(cmd, &spec)?, None),
        }
    };

    ctx.subagents.register_session(id);
    let handler_ctx = HandlerCtx {
        app: ctx.app.clone(),
        sessions: ctx.sessions.clone(),
        approvals: ctx.approvals.clone(),
        store: ctx.store.clone(),
        subagents: ctx.subagents.clone(),
        session_id: id,
        runner_kind: runner.kind(),
        worktree_root: worktree.path.clone(),
        repo_root: crate::worktree::main_repo_of(&worktree.path)
            .map(|r| crate::repo::canonicalize_or(&r))
            .unwrap_or_else(|_| root.clone()),
        agent_home: agent_home(&env),
        turn_settle: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        seen_transcript: Arc::new(Mutex::new(None)),
        observe_only: false,
    };
    let server = HookServer::bind(
        &socket_path,
        Arc::new(move |event| handle_event(&handler_ctx, event)),
    )
    .map_err(|e| format!("socket de hooks: {e}"))?;
    ctx.servers.insert(id, server);

    let title = opts.title.clone().unwrap_or_else(|| task.clone());
    // O id da conversa não é repassado aqui: `spawn_session` o herda da sessão
    // que ocupava este `SessionId` — que, no caso de retomar, é a mesma conversa.
    ctx.sessions
        .spawn_session(
            ctx.app.clone(),
            &ctx.pty_pool,
            id,
            cmd,
            SessionKind::Agent {
                runner: runner.kind(),
            },
            title,
            Some(root),
            Some(worktree),
            jail,
            opts.cols,
            opts.rows,
            opts.opened_by_gate,
            on_exit,
        )
        .map_err(|e| {
            ctx.servers.shutdown(id);
            e.to_string()
        })
}

#[cfg(test)]
#[cfg(unix)]
mod hook_server_registry_tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn no_op_handler() -> crate::hook_ipc::Handler {
        Arc::new(|_event| HookAction::Ack)
    }

    /// Review round 1 (correção/contrato), achado BLOQUEANTE: reproduz a
    /// sequência real de `prepare_hosted_agent` — hospedar `claude` puro,
    /// ele SAIR sem `on_exit` nenhum (o caminho hospedado não tem), e
    /// hospedar de novo no MESMO pane, que reusa o MESMO `socket_path`
    /// (`runtime_dir(id)` é determinístico por `SessionId`). Com o fix
    /// (`ctx.servers.shutdown(id)` ANTES do `HookServer::bind`), a 2ª
    /// hospedagem religa sem travar, e um 3º acesso ao registry prova que o
    /// Mutex não ficou preso em nenhum dos dois passos.
    #[test]
    fn rehosting_the_same_session_without_an_on_exit_does_not_deadlock_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("hook.sock");
        let registry = HookServerRegistry::default();
        let id = SessionId::new_v4();

        // 1ª hospedagem: liga e registra. Ninguém chama shutdown depois —
        // é exatamente o caso do agente hospedado que saiu sozinho.
        let server1 = HookServer::bind(&socket_path, no_op_handler()).expect("bind 1");
        registry.insert(id, server1);

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            // A sequência FIXA que `prepare_hosted_agent` agora segue:
            // derruba o antigo ANTES de religar no mesmo path.
            registry.shutdown(id);
            let server2 = HookServer::bind(&socket_path, no_op_handler()).expect("bind 2");
            registry.insert(id, server2);
            // Um terceiro acesso prova que o Mutex não ficou preso em
            // nenhum dos passos acima.
            registry.shutdown(id);
            let _ = tx.send(());
        });

        assert!(
            rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "religar a mesma sessão travou — exatamente o achado bloqueante \
             do review round 1 (join() de HookServer::stop preso, ou o Mutex \
             do registry preso atrás dele)"
        );
    }

    /// A metade que a reprodução acima não alcança: quando ALGUÉM (uma
    /// chamada errada, um caminho futuro que não segue a disciplina
    /// shutdown-antes-do-bind) trava um `shutdown()` de verdade — path
    /// roubado por um bind concorrente, o cenário exato do achado —, esse
    /// travamento tem que ficar CONTIDO na sessão travada, nunca vazar para
    /// o registry inteiro. Reproduz a ORDEM ERRADA de propósito (bind do
    /// novo servidor ANTES de avisar o registry) para travar o
    /// `old.shutdown()` por dentro do `insert` — aceito e esperado nesta
    /// prova — e confere que uma sessão DIFERENTE, no MESMO registry,
    /// continua funcionando. A thread travada nunca é joinada (não tem como
    /// — é o próprio bug sendo provado); o processo do teste a descarta ao
    /// sair.
    #[test]
    fn a_stuck_shutdown_never_blocks_the_registry_for_other_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("hook.sock");
        let registry = Arc::new(HookServerRegistry::default());
        let stuck_id = SessionId::new_v4();

        let server1 = HookServer::bind(&socket_path, no_op_handler()).expect("bind 1");
        registry.insert(stuck_id, server1);
        // Rouba o path ANTES de avisar o registry — a ordem que
        // `prepare_hosted_agent` tinha antes do fix. O `insert` abaixo vai
        // disparar `old.shutdown()` sobre um servidor cujo path já não é
        // mais dele: o `join()` nunca retorna.
        let server2 =
            HookServer::bind(&socket_path, no_op_handler()).expect("bind 2 (rouba o path)");

        let stuck_registry = Arc::clone(&registry);
        std::thread::spawn(move || {
            stuck_registry.insert(stuck_id, server2);
        });
        // Tempo para o `insert` acima começar e travar dentro do
        // `old.shutdown()`.
        std::thread::sleep(Duration::from_millis(200));

        let other_id = SessionId::new_v4();
        let other_dir = tempfile::tempdir().unwrap();
        let other_path = other_dir.path().join("hook.sock");
        let other_server = HookServer::bind(&other_path, no_op_handler()).expect("bind 3");
        let (tx, rx) = mpsc::channel();
        let other_registry = Arc::clone(&registry);
        std::thread::spawn(move || {
            other_registry.insert(other_id, other_server);
            other_registry.shutdown(other_id);
            let _ = tx.send(());
        });

        assert!(
            rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "uma sessão travada no shutdown travou o registry inteiro — o \
             Mutex ficou preso atrás de um join() bloqueante (achado \
             bloqueante do review round 1)"
        );
    }
}

// Exercita `build_hosted_command`/`hosted_plan_from` — a cadeia exclusiva do
// canal shim↔core, no Linux e no macOS (mesmo gate acima).
#[cfg(test)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod build_hosted_command_tests {
    use super::*;

    /// `prepare_hosted_agent` inteiro precisa de `AgentSessionCtx` — e
    /// `AgentSessionCtx.app: AppHandle` não é genérico em `R`, então
    /// `tauri::test::mock_app()` não serve aqui. Por isso a decisão de
    /// env/argv foi extraída para `build_hosted_command`, testável sem app
    /// nenhum; o resto de `prepare_hosted_agent` é fiação (`HookServer::bind`,
    /// `ctx.servers.insert`).
    fn hook_setup(runtime: &Path) -> HookSetup {
        HookSetup {
            settings_path: runtime.join(HOOK_SETTINGS_FILE),
            hook_command: "tyba _hook --observe".into(),
        }
    }

    fn init_git_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .expect("git");
            assert!(status.success(), "git {args:?} falhou");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "test"]);
        std::fs::write(dir.join("README.md"), "x").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
    }

    fn build(cwd: &Path, runtime: &Path, args: &[String]) -> HostedCommand {
        build_hosted_command(
            &ClaudeCodeRunner,
            cwd,
            &hook_setup(runtime),
            &runtime.join(HOOK_SOCKET_FILE),
            runtime,
            &std::env::current_exe().unwrap(),
            args,
        )
        .expect("o claude digitado sempre monta")
    }

    #[test]
    fn broken_user_config_does_not_block_the_sandbox_spec() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".tyba")).unwrap();
        std::fs::write(home.path().join(".tyba/config.toml"), "[sandbox\n").unwrap();
        let work = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert("HOME".to_string(), home.path().display().to_string());
        let spec = sandbox_spec(
            &ClaudeCodeRunner,
            &env,
            work.path(),
            work.path(),
            runtime.path(),
            &runtime.path().join(HOOK_SOCKET_FILE),
            &std::env::current_exe().unwrap(),
            None,
        )
        .expect("config.toml quebrado não pode impedir a sessão");
        assert!(spec.read_allow_extra.is_empty());
    }

    /// Agente livre: o claude digitado passa pelo `Sandbox` passthrough —
    /// argv cru, sem `sandbox-exec`/`bwrap`, com os hooks e os marcadores.
    #[test]
    fn the_typed_claude_runs_through_the_passthrough_sandbox() {
        let repo = tempfile::tempdir().unwrap();
        init_git_repo(repo.path());
        let cwd = crate::repo::canonicalize_or(repo.path());
        let runtime = tempfile::tempdir().unwrap();
        let socket_path = runtime.path().join(HOOK_SOCKET_FILE);

        let hosted = build(&cwd, runtime.path(), &[]);
        let plan = hosted_plan_from(&hosted.cmd, &cwd);
        assert_eq!(plan.argv.first().map(String::as_str), Some("claude"));
        assert!(
            !plan.argv.iter().any(|a| a.contains("sandbox-exec")
                || a.contains("bwrap")
                || a.contains("(version 1)")),
            "nada de jaula no argv: {:?}",
            plan.argv
        );
        assert!(
            plan.argv.iter().any(|a| a == "--settings"),
            "{:?}",
            plan.argv
        );
        for (k, v) in [
            ("TYBA_HOSTED", "1".to_string()),
            ("TYBA_JAILED", "0".to_string()),
            (
                "TYBA_HOOK_SOCKET",
                socket_path.to_string_lossy().into_owned(),
            ),
        ] {
            assert!(
                plan.env.contains(&(k.to_string(), v)),
                "{k}: {:?}",
                plan.env
            );
        }
        assert!(
            !plan.env.iter().any(|(k, _)| k == "TYBA_SANDBOX"),
            "o passthrough não marca sandbox: {:?}",
            plan.env
        );
        assert_eq!(plan.cwd, cwd.to_string_lossy());
    }

    /// Agente livre: o plano carrega SÓ o que o TYBA acrescenta — nada do
    /// env do processo core —, e o env do shell do pane chega ao claude
    /// pelo próprio shim, menos o que desligaria os hooks.
    #[test]
    fn the_pane_env_reaches_the_child_and_tyba_only_adds_its_own_vars() {
        let cwd = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let hosted = build(cwd.path(), runtime.path(), &[]);
        let plan = hosted_plan_from(&hosted.cmd, cwd.path());

        let mut keys: Vec<&str> = plan.env.iter().map(|(k, _)| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["TYBA_HOOK_SOCKET", "TYBA_HOSTED", "TYBA_JAILED"]);

        let pane: Vec<(std::ffi::OsString, std::ffi::OsString)> = [
            ("TYBA_TEST_PANE_VAR", "1"),
            ("TERM", "xterm-kitty"),
            ("CLAUDE_CODE_SIMPLE", "1"),
            ("CLAUDE_CODE_SAFE_MODE", "1"),
            ("TYBA_SHIM", "fallback"),
        ]
        .iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let child = crate::hook_ipc::channel::hosted_child_env(pane, &plan.env_remove, &plan.env);
        let has = |k: &str| child.iter().any(|(key, _)| key == k);
        for k in [
            "TYBA_TEST_PANE_VAR",
            "TERM",
            "TYBA_HOOK_SOCKET",
            "TYBA_HOSTED",
        ] {
            assert!(has(k), "{k} faltou: {child:?}");
        }
        for k in ["CLAUDE_CODE_SIMPLE", "CLAUDE_CODE_SAFE_MODE", "TYBA_SHIM"] {
            assert!(
                !has(k),
                "{k} desligaria os hooks ou mentiria a rota: {child:?}"
            );
        }
    }

    #[test]
    fn worktree_root_is_the_repo_toplevel_or_the_cwd() {
        let repo = tempfile::tempdir().unwrap();
        init_git_repo(repo.path());
        let sub = repo.path().join("src");
        std::fs::create_dir(&sub).unwrap();
        let runtime = tempfile::tempdir().unwrap();
        assert_eq!(
            build(&sub, runtime.path(), &[]).worktree_root,
            crate::repo::canonicalize_or(repo.path())
        );
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(
            build(plain.path(), runtime.path(), &[]).worktree_root,
            plain.path()
        );
    }

    /// M6-A: os args do usuário entram no argv do plano um a um, depois dos
    /// hooks — nada vira string de shell.
    #[test]
    fn user_args_land_in_the_plan_argv_verbatim_after_the_hooks() {
        let cwd = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let args: Vec<String> = vec![
            "-c".into(),
            "--dangerously-skip-permissions".into(),
            "-p".into(),
            "a; rm -rf ~ $(x)".into(),
        ];
        let hosted = build(cwd.path(), runtime.path(), &args);
        let plan = hosted_plan_from(&hosted.cmd, cwd.path());
        let settings = plan.argv.iter().position(|a| a == "--settings").unwrap();
        assert_eq!(&plan.argv[plan.argv.len() - 4..], &args[..]);
        assert!(settings < plan.argv.len() - 4);
    }
}

#[cfg(test)]
mod subagent_status_tests {
    use super::*;

    fn awaiting() -> SessionStatus {
        SessionStatus::AwaitingInput {
            hint: None,
            reason: Default::default(),
        }
    }

    #[test]
    fn subagent_start_wakes_only_an_idle_session() {
        let idle = SessionStatus::Idle { summary: None };
        assert_eq!(
            status_on_subagent_start(&idle, true),
            Some(SessionStatus::Running)
        );
        assert_eq!(status_on_subagent_start(&idle, false), None);
        for current in [
            SessionStatus::Running,
            awaiting(),
            SessionStatus::Exited { code: 0 },
            SessionStatus::Failed { reason: "x".into() },
        ] {
            assert_eq!(
                status_on_subagent_start(&current, true),
                None,
                "{current:?}"
            );
        }
    }

    #[test]
    fn held_label_needs_an_active_subagent() {
        assert!(held_by_subagents(true, &SessionStatus::Running, true));
        assert!(!held_by_subagents(true, &SessionStatus::Running, false));
        assert!(!held_by_subagents(false, &SessionStatus::Running, true));
        assert!(!held_by_subagents(true, &awaiting(), true));
    }

    #[test]
    fn held_label_returns_only_for_subagent_events_after_the_turn() {
        assert!(label_held_after_running(true, true));
        assert!(!label_held_after_running(true, false));
        assert!(!label_held_after_running(false, true));
    }

    #[test]
    fn held_stop_keeps_an_open_prompt() {
        assert_eq!(status_on_held_stop(&awaiting()), None);
        assert_eq!(
            status_on_held_stop(&SessionStatus::Running),
            Some(SessionStatus::Running)
        );
        assert_eq!(
            status_on_held_stop(&SessionStatus::Idle { summary: None }),
            Some(SessionStatus::Running)
        );
    }
}

#[cfg(test)]
mod session_end_tests {
    use super::session_end_ends_subagents;
    use serde_json::json;

    #[test]
    fn clear_keeps_background_subagents_running() {
        // Payload sintético, escrito à mão no formato do hook SessionEnd.
        let raw = json!({"hook_event_name": "SessionEnd", "reason": "clear"});
        assert!(!session_end_ends_subagents(&raw));
    }

    #[test]
    fn exit_and_unknown_reasons_end_subagents() {
        for reason in ["prompt_input_exit", "logout", "other"] {
            let raw = json!({"hook_event_name": "SessionEnd", "reason": reason});
            assert!(session_end_ends_subagents(&raw), "{reason}");
        }
        assert!(session_end_ends_subagents(
            &json!({"hook_event_name": "SessionEnd"})
        ));
    }
}

#[cfg(test)]
mod attach_resume_tests {
    use super::*;

    #[test]
    fn no_requested_id_attaches_a_fresh_conversation() {
        assert_eq!(attach_resume(None, &ClaudeCodeRunner), Ok(None));
    }

    #[test]
    fn plausible_id_becomes_the_resume_target() {
        let id = "11111111-2222-3333-4444-555555555555";
        assert_eq!(
            attach_resume(Some(id), &ClaudeCodeRunner),
            Ok(Some(id.to_string()))
        );
    }

    #[test]
    fn implausible_id_refuses_instead_of_reaching_argv() {
        for bad in ["--dangerously-skip-permissions", "a b c d e f g h", "x"] {
            assert!(
                attach_resume(Some(bad), &ClaudeCodeRunner).is_err(),
                "{bad}"
            );
        }
    }
}

#[cfg(test)]
mod hook_route_tests {
    use super::*;

    const CLAUDE: AgentRunnerKind = AgentRunnerKind::ClaudeCode;
    const CODEX: AgentRunnerKind = AgentRunnerKind::Codex;

    /// Invariante 4: o `PermissionRequest` do Claude só sinaliza. Se fosse ao
    /// gate, um verde sairia `allow` — aprovando o prompt nativo pelo usuário.
    #[test]
    fn permission_request_do_claude_nunca_vai_ao_gate() {
        assert_eq!(
            hook_route(&CLAUDE, "PermissionRequest"),
            HookRoute::NativePrompt
        );
    }

    #[test]
    fn permission_request_do_codex_continua_no_gate() {
        assert_eq!(hook_route(&CODEX, "PermissionRequest"), HookRoute::ToolUse);
        assert_eq!(
            hook_route(&AgentRunnerKind::Custom("x".into()), "PermissionRequest"),
            HookRoute::ToolUse
        );
    }

    #[test]
    fn pre_tool_use_vai_ao_gate_em_qualquer_runner() {
        assert_eq!(hook_route(&CLAUDE, "PreToolUse"), HookRoute::ToolUse);
        assert_eq!(hook_route(&CODEX, "PreToolUse"), HookRoute::ToolUse);
    }

    /// Agente livre (claude digitado): o `PreToolUse` nunca chega ao gate —
    /// sem card, sem espera —, o `PermissionRequest` só avisa, e o resto do
    /// monitor segue igual. Sessão gerenciada continua no gate.
    #[test]
    fn notify_is_silent_only_when_the_session_is_on_screen() {
        let here = SessionId::new_v4();
        let other = SessionId::new_v4();
        assert!(notify_suppressed(true, Some(here), here));
        assert!(!notify_suppressed(true, Some(other), here));
        assert!(!notify_suppressed(true, None, here));
        assert!(!notify_suppressed(false, Some(here), here));
    }

    #[test]
    fn observe_only_never_routes_to_the_gate() {
        assert_eq!(
            session_route(&CLAUDE, "PreToolUse", true),
            HookRoute::Observe
        );
        assert_eq!(
            session_route(&CLAUDE, "PermissionRequest", true),
            HookRoute::NativePrompt
        );
        for name in ["PostToolUse", "Stop", "SubagentStart", "SessionEnd"] {
            assert_eq!(session_route(&CLAUDE, name, true), HookRoute::Ack, "{name}");
        }
        assert_eq!(
            session_route(&CLAUDE, "TybaStatusline", true),
            HookRoute::Usage
        );
        assert_eq!(
            session_route(&CLAUDE, "PreToolUse", false),
            HookRoute::ToolUse
        );
    }

    #[test]
    fn eventos_de_sinal_so_recebem_ack() {
        for name in [
            "PostToolUse",
            "UserPromptSubmit",
            "SessionStart",
            "SessionEnd",
            "Stop",
            "Notification",
            "SubagentStart",
            "SubagentStop",
        ] {
            assert_eq!(hook_route(&CLAUDE, name), HookRoute::Ack, "{name}");
        }
        assert_eq!(hook_route(&CLAUDE, "TybaOpenUrl"), HookRoute::OpenUrl);
        assert_eq!(hook_route(&CLAUDE, "TybaStatusline"), HookRoute::Usage);
        assert_eq!(hook_route(&CLAUDE, "StopFailure"), HookRoute::Ack);
    }

    /// O status que `handle_event` deixa depois de uma sequência de eventos,
    /// na mesma ordem que ele usa: primeiro o sinal (`signal_for` →
    /// `status_for`, e `SessionEnd` → `status_on_session_end`), depois o
    /// roteamento (`hook_route` → `NativePrompt`).
    fn status_after(runner: &AgentRunnerKind, events: &[&str]) -> SessionStatus {
        let mut status = SessionStatus::Idle { summary: None };
        for name in events {
            match signal_for(name, None) {
                Some(AgentSignal::Ended) => {
                    if let Some(next) = status_on_session_end(&status) {
                        status = next;
                    }
                }
                Some(signal) => {
                    if let Some(next) = status_for(&signal) {
                        status = next;
                    }
                }
                None => {}
            }
            if hook_route(runner, name) == HookRoute::NativePrompt {
                status = native_prompt_status(None);
            }
        }
        status
    }

    fn is_native_prompt(status: &SessionStatus) -> bool {
        matches!(
            status,
            SessionStatus::AwaitingInput {
                reason: AwaitingReason::NativePrompt,
                ..
            }
        )
    }

    /// O `PermissionRequest` chega logo depois do `PreToolUse` da mesma
    /// chamada; o `Running` do `PreToolUse` não pode ficar por cima.
    #[test]
    fn pre_tool_use_seguido_de_permission_request_termina_no_prompt_nativo() {
        let status = status_after(
            &CLAUDE,
            &["UserPromptSubmit", "PreToolUse", "PermissionRequest"],
        );
        assert!(is_native_prompt(&status), "{status:?}");
    }

    #[test]
    fn cada_saida_tira_do_prompt_nativo() {
        for exit in ["PostToolUse", "PreToolUse", "UserPromptSubmit"] {
            let status = status_after(&CLAUDE, &["PreToolUse", "PermissionRequest", exit]);
            assert_eq!(status, SessionStatus::Running, "{exit}");
        }
        let status = status_after(&CLAUDE, &["PreToolUse", "PermissionRequest", "SessionEnd"]);
        assert!(!is_native_prompt(&status), "SessionEnd: {status:?}");
    }

    /// O que `on_native_prompt` responde (`Ack`) chega ao Claude como stdout
    /// vazio, exit 0 — o prompt nativo segue com o usuário. Round-trip real
    /// pelo socket e pelo cliente `_hook`.
    #[cfg(unix)]
    #[test]
    fn ack_do_prompt_nativo_sai_vazio_para_o_claude() {
        use crate::hook_ipc::run_client;
        use crate::hook_ipc::HookServer;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hook.sock");
        let server = HookServer::bind(
            &path,
            Arc::new(
                |e: HookEvent| match hook_route(&CLAUDE, &e.hook_event_name) {
                    HookRoute::NativePrompt => HookAction::Ack,
                    // Qualquer outro roteamento é o gate, que pode sair `allow`:
                    // é isso que o teste tem de pegar.
                    _ => HookAction::Allow { reason: None },
                },
            ),
        )
        .unwrap();
        let event = serde_json::json!({
            "hook_event_name": "PermissionRequest",
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test"},
            "cwd": "/work"
        });
        let mut out = Vec::new();
        let code = run_client(
            std::io::Cursor::new(serde_json::to_vec(&event).unwrap()),
            &mut out,
            Some(&path),
        );
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(out).unwrap(), "");
        server.shutdown();
    }

    #[test]
    fn codex_nunca_entra_no_prompt_nativo() {
        let status = status_after(&CODEX, &["PreToolUse", "PermissionRequest"]);
        assert!(!is_native_prompt(&status), "{status:?}");
    }
}

/// Os hooks de observação do Codex, com payloads sintéticos no formato da doc
/// (learn.chatgpt.com/docs/hooks): campos comuns + os de cada evento.
#[cfg(test)]
mod codex_hook_events_tests {
    use super::*;
    use crate::hook_ipc::{hook_event_from_value, run_client, HookServer};
    use serde_json::{json, Value};

    const CODEX: AgentRunnerKind = AgentRunnerKind::Codex;
    const AGENT_ID: &str = "019a3f2e-7c1b-7d40-9f3e-5b2c8a1d4e6f";

    fn payload(name: &str, extra: Value) -> Value {
        let mut base = json!({
            "session_id": "thr_sintetico",
            "transcript_path": "/tmp/sintetico/rollout-2026-10-12T10-00-00-thr.jsonl",
            "cwd": "/work/wt",
            "hook_event_name": name,
            "model": "gpt-5.5-codex",
            "permission_mode": "default",
        });
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        base
    }

    fn fixtures() -> Vec<Value> {
        vec![
            payload(
                "UserPromptSubmit",
                json!({"turn_id": "t1", "prompt": "rode os testes"}),
            ),
            payload(
                "PostToolUse",
                json!({
                    "turn_id": "t1",
                    "tool_name": "Bash",
                    "tool_use_id": "call_1",
                    "tool_input": {"command": "cargo test"},
                    "tool_response": "ok"
                }),
            ),
            payload("PreCompact", json!({"turn_id": "t1", "trigger": "auto"})),
            payload("PostCompact", json!({"turn_id": "t1", "trigger": "auto"})),
            payload(
                "SubagentStart",
                json!({"turn_id": "t1", "agent_id": AGENT_ID, "agent_type": "worker"}),
            ),
            payload(
                "SubagentStop",
                json!({
                    "turn_id": "t1",
                    "agent_id": AGENT_ID,
                    "agent_type": "worker",
                    "agent_transcript_path": null,
                    "stop_hook_active": false,
                    "last_assistant_message": "Testes verdes."
                }),
            ),
            payload("Interrupt", json!({"turn_id": "t1"})),
            payload("SessionEnd", json!({"reason": "other"})),
        ]
    }

    #[test]
    fn cada_evento_novo_vira_o_sinal_do_caminho_do_claude() {
        let expected = [
            Some(AgentSignal::Working),
            Some(AgentSignal::Working),
            Some(AgentSignal::Compacting),
            None,
            Some(AgentSignal::SubagentStarted),
            Some(AgentSignal::SubagentEnded),
            Some(AgentSignal::Interrupted),
            Some(AgentSignal::Ended),
        ];
        for (raw, want) in fixtures().into_iter().zip(expected) {
            let event = hook_event_from_value(raw);
            assert_eq!(
                signal_for(&event.hook_event_name, event.notification_type.as_deref()),
                want,
                "{}",
                event.hook_event_name
            );
        }
    }

    /// Observação nunca passa pelo gate: só `PreToolUse`/`PermissionRequest`.
    #[test]
    fn eventos_novos_do_codex_so_recebem_ack() {
        for raw in fixtures() {
            let event = hook_event_from_value(raw);
            assert_eq!(
                hook_route(&CODEX, &event.hook_event_name),
                HookRoute::Ack,
                "{}",
                event.hook_event_name
            );
        }
    }

    #[test]
    fn subagente_do_codex_traz_id_tipo_e_fala_final() {
        let all = fixtures();
        let start = hook_event_from_value(all[4].clone());
        assert_eq!(start.agent_id.as_deref(), Some(AGENT_ID));
        assert_eq!(start.agent_type.as_deref(), Some("worker"));
        let stop = hook_event_from_value(all[5].clone());
        assert_eq!(stop.agent_id.as_deref(), Some(AGENT_ID));
        assert_eq!(stop.raw["last_assistant_message"], "Testes verdes.");
    }

    #[test]
    fn modelo_vem_do_fio_principal_e_nunca_do_subagente() {
        let all = fixtures();
        let prompt = hook_event_from_value(all[0].clone());
        assert_eq!(model_of(&prompt), Some("gpt-5.5-codex"));
        let sub = hook_event_from_value(all[4].clone());
        assert_eq!(model_of(&sub), None);
        let sem_modelo = hook_event_from_value(json!({"hook_event_name": "Stop"}));
        assert_eq!(model_of(&sem_modelo), None);
    }

    #[test]
    fn um_turno_do_codex_comeca_trabalhando_e_termina_parado() {
        let mut status = SessionStatus::Idle { summary: None };
        for name in [
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "PreCompact",
        ] {
            if let Some(next) = signal_for(name, None).as_ref().and_then(status_for) {
                status = next;
            }
        }
        assert_eq!(
            status,
            SessionStatus::Running,
            "PreCompact não para o turno"
        );
        assert_eq!(
            status_on_interrupt(&status),
            Some(SessionStatus::Idle {
                summary: Some(INTERRUPTED_SUMMARY.into())
            })
        );
    }

    /// `Stop`, `SubagentStop` e `Interrupt` do Codex só aceitam JSON ou nada
    /// no stdout; `ack` tem de sair vazio para todo evento de observação.
    #[test]
    fn ack_sai_vazio_para_todo_evento_de_observacao() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hook.sock");
        let server = HookServer::bind(&path, Arc::new(|_e: HookEvent| HookAction::Ack)).unwrap();
        let mut all = fixtures();
        all.push(payload(
            "Stop",
            json!({"turn_id": "t1", "stop_hook_active": false, "last_assistant_message": "pronto"}),
        ));
        for raw in all {
            let name = raw["hook_event_name"].as_str().unwrap().to_string();
            let mut out = Vec::new();
            let code = run_client(
                std::io::Cursor::new(serde_json::to_vec(&raw).unwrap()),
                &mut out,
                Some(&path),
            );
            assert_eq!(code, 0, "{name}");
            assert_eq!(String::from_utf8(out).unwrap(), "", "{name}");
        }
        server.shutdown();
    }

    /// Socket fora do ar: observação segue sem o hook (fail-open, stdout
    /// vazio); só o `PreToolUse` nega.
    #[test]
    fn sem_transporte_observacao_e_fail_open() {
        for raw in fixtures() {
            let name = raw["hook_event_name"].as_str().unwrap().to_string();
            let mut out = Vec::new();
            let code = run_client(
                std::io::Cursor::new(serde_json::to_vec(&raw).unwrap()),
                &mut out,
                None,
            );
            assert_eq!(code, 0, "{name}");
            assert_eq!(String::from_utf8(out).unwrap(), "", "{name}");
        }
    }
}

#[cfg(test)]
mod inferred_history_tests {
    use super::*;
    use serde_json::json;

    const CLAUDE: AgentRunnerKind = AgentRunnerKind::ClaudeCode;

    /// Payloads sintéticos com as chaves que o Claude 2.1.281 manda em cada
    /// evento (medicao-claude.md, pergunta 3). Nenhum sai de sessão real.
    fn permission_request(prompt_id: &str, command: &str) -> serde_json::Value {
        json!({
            "cwd": "/wt",
            "hook_event_name": "PermissionRequest",
            "permission_mode": "default",
            "permission_suggestions": [{
                "type": "addRules",
                "rules": [{"toolName": "Bash", "ruleContent": "python3 *"}],
                "behavior": "allow",
                "destination": "localSettings"
            }],
            "prompt_id": prompt_id,
            "scratchpad_dir": "/tmp/scratch",
            "session_id": "claude-sessao-1",
            "tool_input": {"command": command},
            "tool_name": "Bash",
            "transcript_path": "/tmp/transcript.jsonl"
        })
    }

    fn post_tool_use(prompt_id: &str, command: &str) -> serde_json::Value {
        json!({
            "cwd": "/wt",
            "duration_ms": 42,
            "hook_event_name": "PostToolUse",
            "permission_mode": "default",
            "prompt_id": prompt_id,
            "scratchpad_dir": "/tmp/scratch",
            "session_id": "claude-sessao-1",
            "tool_input": {"command": command},
            "tool_name": "Bash",
            "tool_response": {"stdout": "ok", "stderr": "", "interrupted": false},
            "tool_use_id": "toolu_sintetico_01",
            "transcript_path": "/tmp/transcript.jsonl"
        })
    }

    fn pre_tool_use(prompt_id: &str, command: &str) -> serde_json::Value {
        json!({
            "cwd": "/wt",
            "hook_event_name": "PreToolUse",
            "permission_mode": "default",
            "prompt_id": prompt_id,
            "scratchpad_dir": "/tmp/scratch",
            "session_id": "claude-sessao-1",
            "tool_input": {"command": command},
            "tool_name": "Bash",
            "tool_use_id": "toolu_sintetico_02",
            "transcript_path": "/tmp/transcript.jsonl"
        })
    }

    fn named(name: &str) -> serde_json::Value {
        json!({
            "hook_event_name": name,
            "prompt_id": "p2",
            "session_id": "claude-sessao-1",
            "cwd": "/wt"
        })
    }

    fn event(raw: serde_json::Value) -> HookEvent {
        let s = |k: &str| raw.get(k).and_then(|v| v.as_str()).map(str::to_string);
        HookEvent {
            hook_event_name: s("hook_event_name").unwrap_or_default(),
            tool_name: s("tool_name"),
            tool_input: raw.get("tool_input").cloned(),
            notification_type: s("notification_type"),
            cwd: s("cwd"),
            agent_id: None,
            agent_type: None,
            raw,
        }
    }

    fn feed(store: &Store, session: SessionId, raws: Vec<serde_json::Value>) {
        for (i, raw) in raws.into_iter().enumerate() {
            record_inferred(
                store,
                &CLAUDE,
                Path::new("/wt"),
                session,
                &event(raw),
                1_000 + i as u64,
            );
        }
    }

    /// A decisão de cada linha da sessão, na ordem de inserção.
    fn decisions(store: &Store, session: SessionId) -> Vec<String> {
        store
            .list_approval_history(Some(&session.to_string()), 100)
            .unwrap()
            .into_iter()
            .rev()
            .map(|e| e.decision)
            .collect()
    }

    /// A mesma chamada vista no `PermissionRequest` e no `PostToolUse` dá a
    /// mesma chave — embora os dois payloads tenham chaves diferentes
    /// (`permission_suggestions` num, `tool_use_id`/`tool_response` no outro).
    #[test]
    fn call_key_casa_permission_request_com_post_tool_use() {
        assert_eq!(
            call_key(&permission_request("p1", "cargo test")),
            call_key(&post_tool_use("p1", "cargo test"))
        );
        assert_eq!(
            call_key(&permission_request("p1", "cargo test")),
            call_key(&pre_tool_use("p1", "cargo test"))
        );
    }

    #[test]
    fn call_key_separa_chamadas_diferentes() {
        let base = call_key(&permission_request("p1", "cargo test"));
        assert_ne!(base, call_key(&permission_request("p2", "cargo test")));
        assert_ne!(base, call_key(&permission_request("p1", "cargo build")));
        let mut outra_sessao = permission_request("p1", "cargo test");
        outra_sessao["session_id"] = json!("claude-sessao-2");
        assert_ne!(base, call_key(&outra_sessao));
        let mut outra_ferramenta = permission_request("p1", "cargo test");
        outra_ferramenta["tool_name"] = json!("Write");
        assert_ne!(base, call_key(&outra_ferramenta));
    }

    /// A ordem das chaves do `tool_input` não é contrato do Claude: o mesmo
    /// input com as chaves em outra ordem (e aninhado) dá a mesma chave. O
    /// texto é parseado com `from_str` para a ordem de origem ser a do texto.
    #[test]
    fn call_key_nao_depende_da_ordem_das_chaves_do_tool_input() {
        let a: serde_json::Value = serde_json::from_str(
            r#"{"session_id":"s","prompt_id":"p","tool_name":"Edit",
                "tool_input":{"file_path":"/wt/a.rs","old_string":"x","new_string":"y",
                              "opts":{"b":1,"a":[{"z":1,"y":2}]}}}"#,
        )
        .unwrap();
        let b: serde_json::Value = serde_json::from_str(
            r#"{"tool_input":{"opts":{"a":[{"y":2,"z":1}],"b":1},"new_string":"y",
                              "old_string":"x","file_path":"/wt/a.rs"},
                "tool_name":"Edit","prompt_id":"p","session_id":"s"}"#,
        )
        .unwrap();
        assert_eq!(call_key(&a), call_key(&b));
    }

    #[test]
    fn json_canonico_ordena_chaves_em_toda_profundidade() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"b":{"d":1,"c":[{"f":null,"e":"é\"x"}]},"a":true}"#).unwrap();
        let mut out = String::new();
        write_canonical_json(&v, &mut out);
        assert_eq!(out, r#"{"a":true,"b":{"c":[{"e":"é\"x","f":null}],"d":1}}"#);
    }

    #[test]
    fn campos_ausentes_ainda_dao_chave_estavel() {
        let raw = json!({"hook_event_name": "PermissionRequest"});
        assert_eq!(call_key(&raw), call_key(&raw.clone()));
        assert_eq!(call_key(&raw).len(), 64);
    }

    #[test]
    fn passo_deduzido_de_cada_evento() {
        assert_eq!(
            inferred_step(&CLAUDE, "PermissionRequest"),
            InferredStep::Open
        );
        assert_eq!(inferred_step(&CLAUDE, "PostToolUse"), InferredStep::Approve);
        assert_eq!(
            inferred_step(&CLAUDE, "PostToolUseFailure"),
            InferredStep::Approve
        );
        for name in ["PreToolUse", "UserPromptSubmit", "SessionEnd"] {
            assert_eq!(
                inferred_step(&CLAUDE, name),
                InferredStep::DenyPending,
                "{name}"
            );
        }
        // Negar não dispara `Stop` (B0); e ele pode chegar com o prompt aberto.
        for name in ["Stop", "Notification", "SessionStart", "SubagentStop"] {
            assert_eq!(inferred_step(&CLAUDE, name), InferredStep::None, "{name}");
        }
        // O `PermissionRequest` do Codex vai ao gate, que grava a linha `tyba`.
        assert_ne!(
            inferred_step(&AgentRunnerKind::Codex, "PermissionRequest"),
            InferredStep::Open
        );
    }

    /// CA-15: pedido → a ferramenta rodou → aprovado (deduzido).
    #[test]
    fn ciclo_pendente_aprovado() {
        let store = Store::open_in_memory().unwrap();
        let session = SessionId::new_v4();
        feed(
            &store,
            session,
            vec![
                pre_tool_use("p1", "cargo test"),
                permission_request("p1", "cargo test"),
            ],
        );
        assert_eq!(decisions(&store, session), vec!["pending"]);

        feed(&store, session, vec![post_tool_use("p1", "cargo test")]);
        assert_eq!(decisions(&store, session), vec!["approved"]);
        let stats = store.agent_stats(0, None).unwrap();
        assert_eq!(stats.totals.requested, 1);
        assert_eq!(stats.totals.human_decided, 0);
    }

    /// CA-15: pedido → o turno segue sem o `PostToolUse` → negado (deduzido),
    /// por cada gatilho de fechamento.
    #[test]
    fn ciclo_pendente_negado_por_cada_gatilho() {
        for closer in [
            pre_tool_use("p1", "ls"),
            named("UserPromptSubmit"),
            named("SessionEnd"),
        ] {
            let store = Store::open_in_memory().unwrap();
            let session = SessionId::new_v4();
            let name = closer["hook_event_name"].clone();
            feed(
                &store,
                session,
                vec![
                    pre_tool_use("p1", "cargo test"),
                    permission_request("p1", "cargo test"),
                    closer,
                ],
            );
            assert_eq!(decisions(&store, session), vec!["denied"], "{name}");
        }
    }

    /// O fechamento do `teardown_agent_session` (lib.rs) é esta mesma chamada
    /// do store, com a sessão que acabou.
    #[test]
    fn teardown_fecha_a_pendente_como_negada() {
        let store = Store::open_in_memory().unwrap();
        let session = SessionId::new_v4();
        feed(
            &store,
            session,
            vec![permission_request("p1", "cargo test")],
        );
        store
            .deny_pending_inferred_for_session(&session.to_string(), 9_000)
            .unwrap();
        assert_eq!(decisions(&store, session), vec!["denied"]);
    }

    /// `Stop` não fecha: a pendente segue esperando o `PostToolUse`.
    #[test]
    fn stop_nao_fecha_a_pendente() {
        let store = Store::open_in_memory().unwrap();
        let session = SessionId::new_v4();
        feed(
            &store,
            session,
            vec![permission_request("p1", "cargo test"), named("Stop")],
        );
        assert_eq!(decisions(&store, session), vec!["pending"]);
        feed(&store, session, vec![post_tool_use("p1", "cargo test")]);
        assert_eq!(decisions(&store, session), vec!["approved"]);
    }

    fn outcome(store: &Store, session: SessionId, raw: serde_json::Value) -> InferredOutcome {
        record_inferred(
            store,
            &CLAUDE,
            Path::new("/wt"),
            session,
            &event(raw),
            1_000,
        )
    }

    /// O desfecho leva o id da linha deduzida: é por ele que a entrada do
    /// histórico de notificações acompanha a mesma linha.
    #[test]
    fn desfecho_da_deducao_leva_o_id_da_linha() {
        let store = Store::open_in_memory().unwrap();
        let session = SessionId::new_v4();
        let opened = outcome(&store, session, permission_request("p1", "cargo test"));
        let InferredOutcome::Opened { id, risk, command } = opened else {
            panic!("{opened:?}");
        };
        assert_eq!(risk, RiskLevel::Yellow);
        assert!(command.contains("cargo test"), "{command}");
        assert_eq!(
            outcome(&store, session, post_tool_use("p1", "cargo test")),
            InferredOutcome::Approved { id }
        );
        // Sem pendente, fechar não é desfecho.
        assert_eq!(
            outcome(&store, session, named("UserPromptSubmit")),
            InferredOutcome::None
        );
        outcome(&store, session, permission_request("p2", "npm i"));
        assert_eq!(
            outcome(&store, session, named("UserPromptSubmit")),
            InferredOutcome::Denied
        );
        assert_eq!(
            outcome(&store, session, named("Stop")),
            InferredOutcome::None
        );
    }

    /// Do hook ao histórico de notificações: o amarelo do prompt nativo entra
    /// pendente e sai com a decisão deduzida, na mesma entrada.
    #[test]
    fn amarelo_do_prompt_nativo_entra_no_historico_de_notificacoes() {
        use crate::notifications::{NotificationKind, Notifications};
        use crate::session::SessionManager;

        let store = Arc::new(Store::open_in_memory().unwrap());
        let sessions: crate::session::SharedSessionManager =
            Arc::new(SessionManager::new(Arc::clone(&store)));
        let n = Notifications::new(Arc::clone(&store), sessions);
        let session = SessionId::new_v4();
        let step = |raw: serde_json::Value| {
            let o = outcome(&store, session, raw);
            record_inferred_notification(&n, session, &o)
        };
        assert!(step(permission_request("p1", "cargo test")));
        assert!(step(post_tool_use("p1", "cargo test")));
        assert!(step(permission_request("p2", "npm i")));
        assert!(step(named("UserPromptSubmit")));
        assert!(!step(named("Stop")));

        let all = store
            .list_notifications(crate::approvals::now_ms() as i64)
            .unwrap();
        let mut decisions: Vec<(Option<String>, Option<Decision>, bool)> = all
            .iter()
            .filter(|e| e.kind == NotificationKind::NativeApproval)
            .map(|e| (e.command.clone(), e.decision, e.pending))
            .collect();
        decisions.reverse();
        assert_eq!(decisions.len(), 2);
        assert!(decisions[0].0.as_deref().unwrap().contains("cargo test"));
        assert_eq!(decisions[0].1, Some(Decision::Approved));
        assert!(decisions[1].0.as_deref().unwrap().contains("npm i"));
        assert_eq!(decisions[1].1, Some(Decision::Denied));
        assert!(decisions.iter().all(|d| !d.2));
    }

    /// Codex não abre linha deduzida: o `PermissionRequest` dele é o gate.
    #[test]
    fn codex_nao_grava_linha_deduzida() {
        let store = Store::open_in_memory().unwrap();
        let session = SessionId::new_v4();
        record_inferred(
            &store,
            &AgentRunnerKind::Codex,
            Path::new("/wt"),
            session,
            &event(permission_request("p1", "cargo test")),
            1_000,
        );
        assert!(decisions(&store, session).is_empty());
    }
}
