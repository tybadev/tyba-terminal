use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::agent::notify;

use crate::agent::hooks_settings::{hook_command, hooks_settings_json};
use crate::agent::subagents::SharedSubagents;
use crate::agent::{AgentRunner, ClaudeCodeRunner, CodexRunner, HookSetup};
use crate::approvals::tool_action::{normalize_tool_use, ToolAction};
use crate::approvals::{now_ms, Decision, RiskLevel, SharedApprovals};
use crate::hook_ipc::{HookAction, HookEvent, HookServer};
use crate::pty::SharedPtyPool;
use crate::sandbox::SandboxSpec;
use crate::session::store::{ApprovalHistoryEntry, Store};
use crate::session::{
    AgentRunnerKind, AwaitingReason, CreateSessionOpts, Session, SessionId, SessionKind,
    SessionStatus, SharedSessionManager,
};
use crate::status::agent_events::{signal_for, status_for, AgentSignal};

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
    pub fn insert(&self, id: SessionId, server: HookServer) {
        if let Some(old) = self
            .servers
            .lock()
            .expect("hook servers lock")
            .insert(id, server)
        {
            old.shutdown();
        }
    }

    pub fn shutdown(&self, id: SessionId) {
        if let Some(server) = self.servers.lock().expect("hook servers lock").remove(&id) {
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
    turn_settle: Arc<std::sync::atomic::AtomicU64>,
    /// Último `transcript_path` que já virou id de conversa. Existe para o
    /// handler não reabrir o arquivo a cada `PreToolUse`: o id mora no cabeçalho
    /// e não muda enquanto o transcript for o mesmo.
    seen_transcript: Arc<Mutex<Option<String>>>,
}

fn main_window_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false)
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
    if main_window_focused(app) {
        return;
    }
    // Preferência ilegível não silencia o aviso: `ok().flatten()` cai em `None`,
    // que é "nunca escolhi", que é o default ligado. Perder o aviso de um agente
    // bloqueado por causa de um erro de leitura seria o dano maior.
    let enabled_raw = store.get_setting(kind.enabled_key()).ok().flatten();
    let sound_raw = store.get_setting(kind.sound_key()).ok().flatten();
    let policy = notify::resolve(kind, enabled_raw.as_deref(), sound_raw.as_deref());
    if !policy.enabled {
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
    let _ = builder.show();
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
            return;
        }
        let Some(session) = sessions.get(id) else {
            return;
        };
        let SessionStatus::Idle { summary } = &session.status else {
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

fn on_pre_tool_use(ctx: &HandlerCtx, event: &HookEvent) -> HookAction {
    let tool = event.tool_name.as_deref().unwrap_or("");
    let input = event.tool_input.as_ref();
    let normalized = normalize_tool_use(&ctx.runner_kind, tool, input);
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
    let command = normalized.description;
    let cwd = event.cwd.clone();

    match normalized.action.classify(&ctx.worktree_root) {
        RiskLevel::Green => {
            record_history(
                &ctx.store,
                ctx.session_id,
                command,
                cwd,
                RiskLevel::Green,
                "auto_allowed",
                now_ms(),
            );
            HookAction::Allow { reason: None }
        }
        risk => {
            if risk != RiskLevel::Red && ctx.approvals.is_session_allowed(ctx.session_id, &command)
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
                    hint: Some(command.clone()),
                    reason: AwaitingReason::Approval,
                },
            );
            notify_awaiting_input(ctx, &format!("Aprovação pendente: {command}"));
            let outcome = ctx.approvals.request_blocking(
                &ctx.app,
                ctx.session_id,
                command.clone(),
                cwd.clone(),
                None,
                risk,
            );
            ctx.sessions
                .set_status(&ctx.app, ctx.session_id, SessionStatus::Running);
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
        ctx.sessions
            .set_agent_conversation_id(&ctx.app, ctx.session_id, &id);
    }
}

fn handle_event(ctx: &HandlerCtx, event: HookEvent) -> HookAction {
    capture_conversation_id(ctx, &event);
    match signal_for(&event.hook_event_name, event.notification_type.as_deref()) {
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
                if let Some(summary) = idle {
                    ctx.sessions.set_status(
                        &ctx.app,
                        ctx.session_id,
                        SessionStatus::Idle { summary },
                    );
                }
            }
        }
        Some(AgentSignal::Ended) => {
            ctx.subagents.on_session_ended(&ctx.app, ctx.session_id);
        }
        Some(signal) => {
            use std::sync::atomic::Ordering;
            match status_for(&signal) {
                Some(SessionStatus::Running) => {
                    ctx.subagents.note_orchestrator_working(ctx.session_id);
                    ctx.turn_settle.fetch_add(1, Ordering::SeqCst);
                    ctx.sessions
                        .set_status(&ctx.app, ctx.session_id, SessionStatus::Running);
                }
                Some(status @ SessionStatus::AwaitingInput { .. }) => {
                    ctx.sessions.set_status(&ctx.app, ctx.session_id, status);
                    notify_awaiting_input(ctx, "Agente aguardando sua resposta");
                }
                Some(SessionStatus::Idle { .. }) => {
                    let turn = turn_summary(&event);
                    let summary = turn.text;
                    let needs_settle = !turn.settled;
                    ctx.subagents
                        .note_orchestrator_idle(ctx.session_id, summary.clone());
                    // Subagente async ativo segura a sessão em Running mesmo com o
                    // turno do orquestrador encerrado — só desce a Idle quando o
                    // último subagente termina (via hook ou fim por arquivo).
                    if ctx.subagents.has_active(ctx.session_id) {
                        ctx.turn_settle.fetch_add(1, Ordering::SeqCst);
                        ctx.sessions
                            .set_status(&ctx.app, ctx.session_id, SessionStatus::Running);
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

    if matches!(
        event.hook_event_name.as_str(),
        "PreToolUse" | "PermissionRequest"
    ) {
        return on_pre_tool_use(ctx, &event);
    }
    HookAction::Ack
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

    // Sem `worktree::remove` no erro: a pasta é do usuário, não nossa.
    spawn_prepared(
        ctx,
        SessionId::new_v4(),
        opts,
        runner,
        root,
        worktree,
        task,
        None,
        on_exit,
    )
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
        initial_prompt: None,
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
        home.join("Library/Application Support/dev.tyba.app")
    }
    #[cfg(not(target_os = "macos"))]
    {
        env.get("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"))
            .join("dev.tyba.app")
    }
}

pub(crate) fn sandbox_spec(
    runner: &dyn AgentRunner,
    env: &HashMap<String, String>,
    root: &Path,
    worktree: &Path,
    runtime: &Path,
    socket_path: &Path,
    exe: &Path,
) -> Result<SandboxSpec, String> {
    let home = env
        .get("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
        .ok_or("HOME indisponível — sandbox exige a home do usuário")?;
    let git_dirs = crate::worktree::resolved_git_dirs(worktree)?;
    let mut exec_path_dirs: Vec<PathBuf> =
        std::env::split_paths(&crate::shell_path::agent_path()).collect();
    if let Some(parent) = crate::agent::resolved_binary(&runner.kind())
        .as_deref()
        .and_then(Path::parent)
    {
        exec_path_dirs.push(parent.to_path_buf());
    }
    let agent = runner.sandbox_access(&home, worktree);
    let read_allow_extra = crate::user_config::load(&home)?.sandbox_read_allow;
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
        let settings = hooks_settings_json(&hook_cmd);
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

    let mut cmd = runner.build_command(&worktree.path, &env, &hook_setup, resume.as_deref());
    cmd.env("TYBA_HOOK_SOCKET", &socket_path);
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
        )?;
        // Entrega B: só no Linux, só pro Claude Code — é onde a credencial
        // depende da inversão da política (§2). Nunca bloqueia o spawn (§6).
        #[cfg(target_os = "linux")]
        if matches!(runner.kind(), AgentRunnerKind::ClaudeCode) {
            crate::agent::credentials::emit_warnings(&ctx.app, id, &env, &spec);
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
        turn_settle: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        seen_transcript: Arc::new(Mutex::new(None)),
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
            on_exit,
        )
        .map_err(|e| {
            ctx.servers.shutdown(id);
            e.to_string()
        })
}
