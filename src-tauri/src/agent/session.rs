use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::agent::hooks_settings::{hook_command, hooks_settings_json};
use crate::agent::{AgentRunner, ClaudeCodeRunner};
use crate::approvals::tool_risk::{classify_tool_use, describe_tool_use};
use crate::approvals::{now_ms, Decision, RiskLevel, SharedApprovals};
use crate::hook_ipc::{HookAction, HookEvent, HookServer};
use crate::pty::SharedPtyPool;
use crate::sandbox::{PassthroughSandbox, Sandbox, SandboxSpec};
use crate::session::store::{ApprovalHistoryEntry, Store};
use crate::session::{
    AgentRunnerKind, CreateSessionOpts, Session, SessionId, SessionKind, SessionStatus,
    SharedSessionManager,
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
    worktree_root: PathBuf,
}

fn main_window_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false)
}

fn notify_awaiting_input(ctx: &HandlerCtx, body: &str) {
    if main_window_focused(&ctx.app) {
        return;
    }
    let title = ctx
        .sessions
        .get(ctx.session_id)
        .map(|s| s.title)
        .unwrap_or_else(|| "sessão de agente".into());
    let body = crate::session::redact::redact(body);
    let _ = ctx
        .app
        .notification()
        .builder()
        .title(format!("Tyba — {title}"))
        .body(body.as_ref())
        .show();
}

fn on_pre_tool_use(ctx: &HandlerCtx, event: &HookEvent) -> HookAction {
    let tool = event.tool_name.as_deref().unwrap_or("");
    let input = event.tool_input.as_ref();
    let command = describe_tool_use(tool, input);
    let cwd = event.cwd.clone();

    match classify_tool_use(tool, input, &ctx.worktree_root) {
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
            if let Some(status) = status_for(&signal) {
                let awaiting = matches!(status, SessionStatus::AwaitingInput { .. });
                ctx.sessions.set_status(&ctx.app, ctx.session_id, status);
                if awaiting {
                    notify_awaiting_input(ctx, "Agente aguardando sua resposta");
                }
            }
        }
        None => {}
    }

    if event.hook_event_name == "PreToolUse" {
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
    let runner: Box<dyn AgentRunner> = match &runner_kind {
        AgentRunnerKind::ClaudeCode => Box::new(ClaudeCodeRunner),
        AgentRunnerKind::Codex | AgentRunnerKind::Custom(_) => {
            return Err("runner disponível na Fase 5".into());
        }
    };
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
    let settings = hooks_settings_json(&hook_command(&exe));
    let settings_body =
        serde_json::to_string_pretty(&settings).map_err(|e| format!("settings de hooks: {e}"))?;
    crate::session::write_private(&runtime, HOOK_SETTINGS_FILE, &settings_body)
        .map_err(|e| format!("escrita dos settings de hooks: {e}"))?;
    let settings_path = runtime.join(HOOK_SETTINGS_FILE);

    let user_env: HashMap<String, String> = std::env::vars().collect();
    let config = consented_config(&ctx.store, &root);
    let env = crate::repo_config::agent_env(config.as_ref(), &user_env);

    let mut cmd = runner.build_command(&worktree.path, &env, &settings_path);
    cmd.env("TYBA_HOOK_SOCKET", &socket_path);
    let cmd = PassthroughSandbox.wrap(
        cmd,
        &SandboxSpec {
            writable_root: worktree.path.clone(),
            readable_root: root.clone(),
            allow_network: runner.needs_network(),
        },
    );

    let handler_ctx = HandlerCtx {
        app: ctx.app.clone(),
        sessions: ctx.sessions.clone(),
        approvals: ctx.approvals.clone(),
        store: ctx.store.clone(),
        session_id: id,
        worktree_root: worktree.path.clone(),
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
            opts.cols,
            opts.rows,
            on_exit,
        )
        .map_err(|e| {
            ctx.servers.shutdown(id);
            e.to_string()
        })
}
