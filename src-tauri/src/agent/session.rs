use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::agent::hooks_settings::{hook_command, hooks_settings_json};
use crate::agent::{AgentRunner, ClaudeCodeRunner, CodexRunner, HookSetup};
use crate::approvals::tool_action::normalize_tool_use;
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
    session_id: SessionId,
    runner_kind: AgentRunnerKind,
    worktree_root: PathBuf,
    turn_settle: Arc<std::sync::atomic::AtomicU64>,
}

fn main_window_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false)
}

fn notify_native(app: &AppHandle, sessions: &SharedSessionManager, id: SessionId, body: &str) {
    if main_window_focused(app) {
        return;
    }
    let title = sessions
        .get(id)
        .map(|s| s.title)
        .unwrap_or_else(|| "sessão de agente".into());
    let body = crate::session::redact::redact(body);
    let _ = app
        .notification()
        .builder()
        .title(format!("Tyba — {title}"))
        .body(body.as_ref())
        .show();
}

fn notify_awaiting_input(ctx: &HandlerCtx, body: &str) {
    notify_native(&ctx.app, &ctx.sessions, ctx.session_id, body);
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
        notify_native(&app, &sessions, id, &body);
    });
}

fn on_pre_tool_use(ctx: &HandlerCtx, event: &HookEvent) -> HookAction {
    let tool = event.tool_name.as_deref().unwrap_or("");
    let input = event.tool_input.as_ref();
    let normalized = normalize_tool_use(&ctx.runner_kind, tool, input);
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
                    HookAction::Deny { reason }
                }
            }
        }
    }
}

fn handle_event(ctx: &HandlerCtx, event: HookEvent) -> HookAction {
    match signal_for(&event.hook_event_name, event.notification_type.as_deref()) {
        Some(AgentSignal::Ready) => {
            let _ = ctx.app.emit(
                EVENT_AGENT_READY,
                AgentReadyPayload {
                    session_id: ctx.session_id,
                },
            );
        }
        Some(signal) => {
            if let Some(mut status) = status_for(&signal) {
                let mut needs_settle = false;
                if let SessionStatus::Idle { summary } = &mut status {
                    let turn = turn_summary(&event);
                    *summary = turn.text;
                    needs_settle = !turn.settled;
                }
                let awaiting = matches!(status, SessionStatus::AwaitingInput { .. });
                let turn_ended = matches!(status, SessionStatus::Idle { .. });
                if matches!(status, SessionStatus::Running) {
                    ctx.turn_settle
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                ctx.sessions.set_status(&ctx.app, ctx.session_id, status);
                if awaiting {
                    notify_awaiting_input(ctx, "Agente aguardando sua resposta");
                }
                if turn_ended {
                    let transcript_path = event
                        .raw
                        .get("transcript_path")
                        .and_then(|p| p.as_str())
                        .map(str::to_string);
                    notify_turn_ended(ctx, transcript_path, needs_settle);
                }
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

    let result = spawn_prepared(ctx, opts, runner, root, worktree.clone(), task, on_exit);
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
    spawn_prepared(ctx, opts, runner, root, worktree, task, on_exit)
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

fn spawn_prepared(
    ctx: &AgentSessionCtx,
    opts: CreateSessionOpts,
    runner: Box<dyn AgentRunner>,
    root: PathBuf,
    worktree: crate::worktree::Worktree,
    task: String,
    on_exit: impl FnOnce(SessionId) + Send + 'static,
) -> Result<Session, String> {
    let id = SessionId::new_v4();
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

    let mut cmd = runner.build_command(&worktree.path, &env, &hook_setup);
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
        match sandbox.jailed_spawner(&spec)? {
            Some(spawner) => (cmd, Some(spawner)),
            None => (sandbox.wrap(cmd, &spec)?, None),
        }
    };

    let handler_ctx = HandlerCtx {
        app: ctx.app.clone(),
        sessions: ctx.sessions.clone(),
        approvals: ctx.approvals.clone(),
        store: ctx.store.clone(),
        session_id: id,
        runner_kind: runner.kind(),
        worktree_root: worktree.path.clone(),
        turn_settle: Arc::new(std::sync::atomic::AtomicU64::new(0)),
    };
    let server = HookServer::bind(
        &socket_path,
        Arc::new(move |event| handle_event(&handler_ctx, event)),
    )
    .map_err(|e| format!("socket de hooks: {e}"))?;
    ctx.servers.insert(id, server);

    let title = opts.title.clone().unwrap_or_else(|| task.clone());
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
