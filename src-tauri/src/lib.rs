pub mod agent;
pub mod approvals;
pub mod docker;
pub mod editor;
pub mod error;
pub mod forge;
pub mod hook_ipc;
pub mod layout;
pub mod pty;
pub mod repo;
pub mod repo_config;
pub mod rich_input;
pub mod sandbox;
pub mod session;
pub mod shell_path;
pub mod status;
pub mod theme;
pub mod user_config;
pub mod worktree;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use tauri::{AppHandle, Emitter, Listener, Manager, State};

use approvals::{ApprovalRequest, Decision, SharedApprovals};
use error::AppError;
use pty::SharedPtyPool;
use session::store::Store;
use session::{
    CreateSessionOpts, Session, SessionId, SessionKind, SessionStatus, SharedSessionManager,
};

struct AppState {
    store: Arc<Store>,
    pty_pool: SharedPtyPool,
    sessions: SharedSessionManager,
    approvals: SharedApprovals,
    themes: theme::SharedThemes,
    layout: layout::SharedLayout,
    docker: docker::SharedDocker,
    repos: repo::SharedRepoWatcher,
    repo_reconcile: std::sync::mpsc::Sender<()>,
    rich_input_submit: parking_lot::Mutex<()>,
    worktree_files: rich_input::FilesCache,
    hook_servers: Arc<agent::session::HookServerRegistry>,
}

fn watched_repo_roots(state: &AppState) -> std::collections::HashSet<std::path::PathBuf> {
    let mut roots: std::collections::HashSet<std::path::PathBuf> = state
        .layout
        .state()
        .workspaces
        .iter()
        .filter_map(|w| w.repo_root.as_deref())
        .map(|root| session::expand_home(std::path::Path::new(root)))
        .filter_map(|root| repo::toplevel(&root))
        .collect();

    for session in state.sessions.list() {
        if !matches!(session.status, SessionStatus::Running) {
            continue;
        }
        let Some(pid) = state.pty_pool.leader_pid(session.id) else {
            continue;
        };
        let Some(cwd) = repo::process_cwd(pid) else {
            continue;
        };
        if let Some(root) = repo::toplevel(&cwd) {
            roots.insert(root);
        }
    }
    roots
}

fn reconcile_repo_watchers(app: &AppHandle) {
    let state = app.state::<AppState>();
    let roots = watched_repo_roots(&state);
    state.repos.set_roots(app, roots);
    let _ = app.emit(repo::EVENT_RECONCILED, state.repos.snapshots());
}

fn emit_layout(app: &AppHandle, state: &State<'_, AppState>) {
    let _ = app.emit(layout::EVENT_CHANGED, state.layout.state());
    let _ = state.repo_reconcile.send(());
}

fn dispose_shells(state: &State<'_, AppState>, ids: &[SessionId]) {
    for id in ids {
        if let Some(s) = state.sessions.get(*id) {
            if matches!(s.kind, SessionKind::Shell) {
                state.sessions.dispose(&state.pty_pool, *id);
            }
        }
    }
}

fn dispose_all(app: &AppHandle, state: &State<'_, AppState>, ids: &[SessionId]) {
    for id in ids {
        teardown_agent_session(app, state, *id);
        state.sessions.dispose(&state.pty_pool, *id);
    }
}

fn session_exited(app: &AppHandle, id: SessionId) {
    let state = app.state::<AppState>();
    let Some(session) = state.sessions.get(id) else {
        return;
    };
    teardown_agent_session(app, &state, id);
    if matches!(session.kind, SessionKind::Shell) {
        state.sessions.dispose(&state.pty_pool, id);
        let _ = state.layout.session_disposed(id);
        emit_layout(app, &state);
    } else {
        state
            .sessions
            .set_status(app, id, SessionStatus::Exited { code: -1 });
        let _ = state.repo_reconcile.send(());
    }
}

#[derive(Clone, serde::Serialize)]
struct SetupResultPayload {
    session_id: SessionId,
    ok: bool,
    log: String,
}

/// Roda o `.tyba/setup.sh` do worktree em background quando (e só quando)
/// o usuário consentiu com o hash atual do script. Nunca bloqueia a criação.
fn run_setup_if_consented(app: &AppHandle, state: &AppState, session: &Session) {
    let Some(wt) = &session.worktree else { return };
    let Some(script) = worktree::setup_script(&wt.path) else {
        return;
    };
    let repo_key = repo::canonicalize_or(session.repo_root.as_deref().unwrap_or(&wt.path))
        .to_string_lossy()
        .into_owned();
    let allowed = state
        .store
        .setup_consent(&repo_key, &script.hash)
        .ok()
        .flatten();
    if allowed != Some(true) {
        return;
    }
    let app = app.clone();
    let path = wt.path.clone();
    let env = worktree::setup_env(&path);
    let session_id = session.id;
    std::thread::Builder::new()
        .name("worktree-setup".into())
        .spawn(move || {
            let (ok, log) = match worktree::run_setup(&path, &script, &env) {
                Ok(log) => (true, log),
                Err(e) => (false, e),
            };
            let _ = app.emit(
                &format!("worktree://setup/{session_id}"),
                SetupResultPayload {
                    session_id,
                    ok,
                    log,
                },
            );
        })
        .ok();
}

fn teardown_agent_session(app: &AppHandle, state: &AppState, id: SessionId) {
    state.hook_servers.shutdown(id);
    for request in state.approvals.expire_session(app, id) {
        agent::session::record_history(
            &state.store,
            request.session_id,
            request.command,
            request.cwd,
            request.risk,
            "expired",
            request.requested_at_ms,
        );
    }
}

#[tauri::command]
fn create_session(
    app: AppHandle,
    state: State<'_, AppState>,
    opts: CreateSessionOpts,
) -> Result<Session, String> {
    let handle = app.clone();
    let session = match &opts.kind {
        SessionKind::Agent { .. } => {
            let ctx = agent::session::AgentSessionCtx {
                app: app.clone(),
                sessions: Arc::clone(&state.sessions),
                pty_pool: Arc::clone(&state.pty_pool),
                approvals: Arc::clone(&state.approvals),
                store: Arc::clone(&state.store),
                servers: Arc::clone(&state.hook_servers),
            };
            agent::session::create_agent_session(&ctx, opts, move |id| session_exited(&handle, id))?
        }
        SessionKind::Shell => state
            .sessions
            .create_shell_session(app.clone(), &state.pty_pool, opts, move |id| {
                session_exited(&handle, id)
            })
            .map_err(|e| e.to_string())?,
    };
    run_setup_if_consented(&app, &state, &session);
    Ok(session)
}

#[derive(serde::Serialize)]
struct WorktreeStatus {
    path: std::path::PathBuf,
    branch: Option<String>,
    dirty: bool,
    ahead_of_head: u32,
    managed: bool,
    session_id: Option<SessionId>,
}

#[derive(serde::Serialize)]
struct WorktreeListing {
    root: std::path::PathBuf,
    worktrees: Vec<WorktreeStatus>,
}

#[tauri::command]
async fn worktree_list(
    state: State<'_, AppState>,
    repo_root: String,
) -> Result<WorktreeListing, String> {
    let param = repo::canonicalize_or(std::path::Path::new(&repo_root));
    let root = worktree::main_repo_of(&param)
        .map(|t| repo::canonicalize_or(&t))
        .map_err(|_| "fora de repositório git".to_string())?;
    let head = worktree::head_sha(&root)?;
    let by_path: std::collections::HashMap<std::path::PathBuf, SessionId> = state
        .sessions
        .list()
        .into_iter()
        .filter_map(|s| s.worktree.map(|wt| (repo::canonicalize_or(&wt.path), s.id)))
        .collect();

    let worktrees = worktree::list(&root)?
        .into_iter()
        .filter_map(|e| {
            let canonical = repo::canonicalize_or(&e.path);
            if canonical == root {
                return None;
            }
            Some(WorktreeStatus {
                dirty: worktree::is_dirty(&e.path).unwrap_or(false),
                ahead_of_head: worktree::ahead_count(&e.path, &head).unwrap_or(0),
                managed: worktree::is_managed(&e.path),
                session_id: by_path.get(&canonical).copied(),
                path: e.path,
                branch: e.branch,
            })
        })
        .collect();
    Ok(WorktreeListing { root, worktrees })
}

#[derive(serde::Serialize)]
struct SetupScriptInfo {
    path: std::path::PathBuf,
    content: String,
    hash: String,
    consent: Option<bool>,
}

#[tauri::command]
fn worktree_setup_script(state: State<'_, AppState>, repo_root: String) -> Option<SetupScriptInfo> {
    let param = repo::canonicalize_or(std::path::Path::new(&repo_root));
    let root = repo::toplevel(&param).map(|t| repo::canonicalize_or(&t))?;
    let script = worktree::setup_script(&root)?;
    let consent = state
        .store
        .setup_consent(&root.to_string_lossy(), &script.hash)
        .ok()
        .flatten();
    Some(SetupScriptInfo {
        path: script.path,
        content: script.content,
        hash: script.hash,
        consent,
    })
}

#[tauri::command]
fn worktree_set_setup_consent(
    state: State<'_, AppState>,
    repo_root: String,
    hash: String,
    allow: bool,
) -> Result<(), String> {
    let param = repo::canonicalize_or(std::path::Path::new(&repo_root));
    let root = repo::toplevel(&param)
        .map(|t| repo::canonicalize_or(&t))
        .ok_or("fora de repositório git")?;
    state
        .store
        .set_setup_consent(&root.to_string_lossy(), &hash, allow)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn worktree_remove(
    state: State<'_, AppState>,
    path: String,
    delete_branch: bool,
    force: bool,
) -> Result<(), String> {
    let path = std::path::PathBuf::from(&path);
    let canonical = repo::canonicalize_or(&path);
    let busy = state.sessions.list().into_iter().any(|s| {
        matches!(s.status, SessionStatus::Running)
            && s.worktree
                .map(|wt| repo::canonicalize_or(&wt.path) == canonical)
                .unwrap_or(false)
    });
    if busy {
        return Err("worktree tem sessão ativa — encerre a sessão antes de remover".into());
    }
    worktree::remove(&path, delete_branch, force)
}

#[tauri::command]
async fn worktree_gc(state: State<'_, AppState>) -> Result<worktree::GcReport, String> {
    Ok(worktree::gc_orphans(&known_worktree_paths(&state.sessions)))
}

fn session_worktree(state: &AppState, id: SessionId) -> Result<worktree::Worktree, String> {
    state
        .sessions
        .get(id)
        .ok_or_else(|| format!("sessão não encontrada: {id}"))?
        .worktree
        .ok_or_else(|| "sessão não está em worktree".to_string())
}

struct RepoContext {
    path: std::path::PathBuf,
    base_ref: String,
}

fn session_repo_context(state: &AppState, id: SessionId) -> Result<RepoContext, String> {
    let session = state
        .sessions
        .get(id)
        .ok_or_else(|| format!("sessão não encontrada: {id}"))?;
    if let Some(wt) = session.worktree {
        return Ok(RepoContext {
            path: wt.path,
            base_ref: wt.base_ref,
        });
    }
    let pid = state
        .pty_pool
        .leader_pid(id)
        .ok_or("a sessão não tem um processo ativo")?;
    let cwd = repo::process_cwd(pid).ok_or("não foi possível ler o diretório da sessão")?;
    let root = repo::toplevel(&cwd).ok_or("o diretório da sessão não é um repositório git")?;
    Ok(RepoContext {
        path: repo::canonicalize_or(&root),
        base_ref: "HEAD".to_string(),
    })
}

#[derive(serde::Serialize)]
struct SessionGitStatus {
    root: String,
    branch: Option<String>,
    dirty: bool,
}

#[tauri::command]
async fn session_git_status(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Option<SessionGitStatus>, String> {
    let Ok(ctx) = session_repo_context(&state, id) else {
        return Ok(None);
    };
    let dirty = worktree::is_dirty(&ctx.path).unwrap_or(false);
    let branch = repo::branch(&ctx.path);
    Ok(Some(SessionGitStatus {
        root: ctx.path.to_string_lossy().into_owned(),
        branch,
        dirty,
    }))
}

#[tauri::command]
fn open_diff_tab(app: AppHandle, state: State<'_, AppState>, id: SessionId) -> Result<(), String> {
    session_repo_context(&state, id)?;
    state
        .layout
        .open_workspace_side_view(id, &layout::diff_view(id))
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn close_side_view(
    app: AppHandle,
    state: State<'_, AppState>,
    workspace_id: layout::WorkspaceId,
) -> Result<(), String> {
    state
        .layout
        .close_side_view(workspace_id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn set_side_view_expanded(
    app: AppHandle,
    state: State<'_, AppState>,
    workspace_id: layout::WorkspaceId,
    expanded: bool,
) -> Result<(), String> {
    state
        .layout
        .set_side_view_expanded(workspace_id, expanded)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn set_side_view_ratio(
    app: AppHandle,
    state: State<'_, AppState>,
    workspace_id: layout::WorkspaceId,
    ratio: f64,
    commit: bool,
) -> Result<(), String> {
    state
        .layout
        .set_side_view_ratio(workspace_id, ratio, commit)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
async fn worktree_stage(
    state: State<'_, AppState>,
    id: SessionId,
    paths: Vec<String>,
) -> Result<(), String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::ops::stage(&ctx.path, &paths)
}

#[tauri::command]
async fn worktree_unstage(
    state: State<'_, AppState>,
    id: SessionId,
    paths: Vec<String>,
) -> Result<(), String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::ops::unstage(&ctx.path, &paths)
}

#[tauri::command]
async fn worktree_discard(
    state: State<'_, AppState>,
    id: SessionId,
    paths: Vec<String>,
) -> Result<(), String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::ops::discard(&ctx.path, &paths)
}

#[tauri::command]
async fn worktree_commit(
    state: State<'_, AppState>,
    id: SessionId,
    message: String,
) -> Result<(), String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::ops::commit(&ctx.path, &message)
}

#[tauri::command]
async fn worktree_push(state: State<'_, AppState>, id: SessionId) -> Result<String, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    worktree::ops::push(&ctx.path)
}

#[tauri::command]
async fn worktree_merge_preview(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<worktree::ops::MergePreview, AppError> {
    let wt = session_worktree(&state, id).map_err(session_setup_error)?;
    worktree::ops::merge_preview(&wt.path)
}

#[tauri::command]
async fn worktree_merge_materialize(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<(), AppError> {
    let wt = session_worktree(&state, id).map_err(session_setup_error)?;
    worktree::ops::materialize_conflict(&wt.path)
}

#[tauri::command]
async fn worktree_merge_into_base(
    state: State<'_, AppState>,
    id: SessionId,
    strategy: worktree::ops::MergeStrategy,
    message: Option<String>,
) -> Result<String, AppError> {
    let wt = session_worktree(&state, id).map_err(session_setup_error)?;
    worktree::ops::merge_into_base(&wt.path, strategy, message.as_deref())
}

fn session_setup_error(detail: String) -> AppError {
    AppError::new("session.unavailable").with("detail", detail)
}

async fn forge_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::new("forge.task_failed").with("detail", e.to_string()))?
}

#[tauri::command]
async fn forge_status(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Option<forge::ForgeStatus>, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    let root = ctx.path.clone();
    let branch = repo::branch(&ctx.path);
    forge_blocking(move || Ok(forge::status(&root, branch.as_deref()))).await
}

#[tauri::command]
async fn forge_pr_for_session(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Option<forge::PullRequest>, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    let Some(branch) = repo::branch(&ctx.path) else {
        return Ok(None);
    };
    let path = ctx.path;
    forge_blocking(move || forge::pr_for_branch(&path, &branch)).await
}

#[tauri::command]
async fn forge_pr_comments(
    state: State<'_, AppState>,
    id: SessionId,
    number: u64,
) -> Result<Vec<forge::ReviewComment>, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    let path = ctx.path;
    forge_blocking(move || forge::pr_comments(&path, number)).await
}

#[tauri::command]
async fn forge_create_pr(
    state: State<'_, AppState>,
    id: SessionId,
    title: String,
    body: String,
) -> Result<forge::PullRequest, AppError> {
    let wt = session_worktree(&state, id).map_err(session_setup_error)?;
    let path = wt.path.clone();
    forge_blocking(move || forge::create_pr(&path, &title, &body)).await
}

#[tauri::command]
async fn forge_pr_list(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Vec<forge::PullRequest>, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    let path = ctx.path;
    forge_blocking(move || forge::pr_list(&path)).await
}

/// Abre um arquivo do worktree como TEXTO — nunca "executa" o arquivo.
/// `open <arquivo>`/`xdg-open` rodariam um script/app deixado por um
/// agente no worktree com um clique; aqui só editor configurado ou
/// visualizador de texto. Path do webview: resolve dentro do worktree
/// e recusa qualquer coisa fora.
#[tauri::command]
async fn open_worktree_file(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    editor: Option<String>,
) -> Result<(), String> {
    let wt = session_worktree(&state, id)?;
    let root = wt
        .path
        .canonicalize()
        .map_err(|e| format!("worktree inacessível: {e}"))?;
    let full = root
        .join(&path)
        .canonicalize()
        .map_err(|e| format!("arquivo inacessível: {e}"))?;
    if !full.starts_with(&root) {
        return Err("arquivo fora do worktree".into());
    }

    let gui_editor = editor
        .filter(|id| !id.is_empty())
        .and_then(|id| editor::detect().into_iter().find(|e| e.id == id))
        .filter(|e| !e.terminal);
    let mut cmd = match gui_editor {
        Some(e) => {
            let mut c = std::process::Command::new(e.path);
            c.arg(&full);
            c
        }
        None => {
            #[cfg(target_os = "macos")]
            {
                let mut c = std::process::Command::new("open");
                c.arg("-t").arg(&full);
                c
            }
            #[cfg(not(target_os = "macos"))]
            {
                return Err(
                    "configure um editor em Settings → Geral pra abrir arquivos".to_string()
                );
            }
        }
    };
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("não deu pra abrir o arquivo: {e}"))?;
    // Reap fora do caminho do comando: opener sai rápido, mas Child
    // largado vira zumbi no Unix.
    std::thread::Builder::new()
        .name("open-file-reap".into())
        .spawn(move || {
            let _ = child.wait();
        })
        .ok();
    Ok(())
}

#[tauri::command]
async fn session_diff(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<worktree::diff::SessionDiff, String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::diff::session_diff(&ctx.path, &ctx.base_ref)
}

#[tauri::command]
async fn session_diff_hunks(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    scope: worktree::diff::DiffScope,
    old_path: Option<String>,
) -> Result<worktree::diff::FileHunks, String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::diff::file_hunks(&ctx.path, &ctx.base_ref, scope, &path, old_path.as_deref())
}

#[tauri::command]
async fn session_conflicts(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Option<worktree::conflicts::ConflictState>, String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::conflicts::session_conflicts(&ctx.path)
}

#[tauri::command]
async fn session_conflict_choose(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    side: String,
) -> Result<(), AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    worktree::conflicts::choose_side(&ctx.path, &path, &side)
}

#[tauri::command]
async fn session_conflict_mark_resolved(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
) -> Result<(), AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    worktree::conflicts::mark_resolved(&ctx.path, &path)
}

#[tauri::command]
async fn session_branches(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<worktree::branches::BranchList, String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::branches::list(&ctx.path)
}

#[tauri::command]
async fn session_fetch(state: State<'_, AppState>, id: SessionId) -> Result<(), String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::branches::fetch(&ctx.path)
}

#[tauri::command]
async fn suggest_commit_message(
    state: State<'_, AppState>,
    id: SessionId,
    agent: String,
) -> Result<String, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    agent::suggest::suggest_commit_message(&ctx.path, &agent)
}

#[tauri::command]
async fn session_checkout(
    state: State<'_, AppState>,
    id: SessionId,
    branch: String,
    is_remote: bool,
) -> Result<(), AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    worktree::branches::checkout(&ctx.path, &branch, is_remote)
}

#[tauri::command]
async fn session_branch_diff(
    state: State<'_, AppState>,
    id: SessionId,
    branch: String,
) -> Result<worktree::diff::SessionDiff, String> {
    worktree::branches::validate_ref_name(&branch)?;
    let ctx = session_repo_context(&state, id)?;
    let base = worktree::branches::default_base(&ctx.path);
    let merge_base = worktree::branches::merge_base(&ctx.path, &base, &branch)?;
    worktree::diff::branch_diff(&ctx.path, &merge_base, &branch)
}

#[tauri::command]
async fn session_branch_hunks(
    state: State<'_, AppState>,
    id: SessionId,
    branch: String,
    path: String,
    old_path: Option<String>,
) -> Result<worktree::diff::FileHunks, String> {
    worktree::branches::validate_ref_name(&branch)?;
    let ctx = session_repo_context(&state, id)?;
    let base = worktree::branches::default_base(&ctx.path);
    let merge_base = worktree::branches::merge_base(&ctx.path, &base, &branch)?;
    worktree::diff::range_file_hunks(
        &ctx.path,
        &format!("{merge_base}..{branch}"),
        &path,
        old_path.as_deref(),
    )
}

fn known_worktree_paths(
    sessions: &SharedSessionManager,
) -> std::collections::HashSet<std::path::PathBuf> {
    sessions
        .list()
        .into_iter()
        .filter_map(|s| s.worktree.map(|wt| wt.path))
        .collect()
}

#[tauri::command]
fn write_to_session(state: State<'_, AppState>, id: SessionId, data: String) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| e.to_string())?;
    state.pty_pool.write(id, &bytes).map_err(|e| e.to_string())
}

fn session_cwd_of(state: &AppState, id: SessionId) -> Result<std::path::PathBuf, String> {
    let pid = state
        .pty_pool
        .leader_pid(id)
        .ok_or_else(|| format!("sessão sem processo vivo: {id}"))?;
    repo::process_cwd(pid).ok_or_else(|| "cwd da sessão indisponível".to_string())
}

#[tauri::command]
fn submit_rich_input(
    state: State<'_, AppState>,
    id: SessionId,
    text: String,
    submit: bool,
) -> Result<(), String> {
    let bracketed = state
        .pty_pool
        .bracketed_paste(id)
        .ok_or_else(|| format!("sessão não encontrada: {id}"))?;
    let (normalized, _) = rich_input::normalize(&text);
    let payload = rich_input::plan_injection(&normalized, bracketed)?;
    if payload.is_empty() {
        return Ok(());
    }

    let _submitting = state.rich_input_submit.lock();
    state
        .pty_pool
        .write(id, &payload)
        .map_err(|e| e.to_string())?;
    if submit {
        std::thread::sleep(rich_input::SUBMIT_DELAY);
        state.pty_pool.write(id, b"\r").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn set_agent_match_pattern(pattern: String) -> bool {
    rich_input::agent_matcher().set_pattern(&pattern)
}

#[tauri::command]
fn prompt_mentions_sensitive(text: String) -> bool {
    rich_input::mentions_sensitive(&text)
}

#[tauri::command]
fn session_bracketed_paste(state: State<'_, AppState>, id: SessionId) -> bool {
    state.pty_pool.bracketed_paste(id).unwrap_or(false)
}

#[tauri::command]
fn session_rel_path(state: State<'_, AppState>, id: SessionId, path: String) -> String {
    match session_cwd_of(&state, id) {
        Ok(cwd) => rich_input::rel_path(std::path::Path::new(&path), &cwd),
        Err(_) => path,
    }
}

#[tauri::command]
fn list_worktree_files(
    state: State<'_, AppState>,
    id: SessionId,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    let cwd = session_cwd_of(&state, id)?;
    state
        .worktree_files
        .files_for(&cwd, &query, limit.unwrap_or(50).min(500))
}

#[tauri::command]
fn attach_session(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<(), String> {
    state
        .pty_pool
        .attach(&app, window.label(), id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn detach_session(window: tauri::Window, state: State<'_, AppState>, id: SessionId) {
    state.pty_pool.detach(window.label(), id);
}

#[tauri::command]
fn repo_snapshots(state: State<'_, AppState>) -> Vec<repo::RepoSnapshot> {
    state.repos.snapshots()
}

#[tauri::command]
fn session_cwd(state: State<'_, AppState>, id: SessionId) -> Option<pty::SessionCwdPayload> {
    let pid = state.pty_pool.leader_pid(id)?;
    repo::process_cwd(pid).map(|p| pty::SessionCwdPayload::of(&p))
}

#[tauri::command]
fn resize_session(
    state: State<'_, AppState>,
    id: SessionId,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    state
        .pty_pool
        .resize(id, cols, rows)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_sessions(state: State<'_, AppState>) -> Vec<Session> {
    state.sessions.list()
}

#[tauri::command]
fn session_mark_seen(app: AppHandle, state: State<'_, AppState>, id: SessionId) {
    state.sessions.mark_seen(&app, id);
}

#[tauri::command]
fn dispose_session(app: AppHandle, state: State<'_, AppState>, id: SessionId) {
    teardown_agent_session(&app, &state, id);
    state.sessions.dispose(&state.pty_pool, id);
    let _ = state.layout.session_disposed(id);
    emit_layout(&app, &state);
}

#[tauri::command]
fn layout_state(state: State<'_, AppState>) -> layout::LayoutState {
    state.layout.state()
}

#[tauri::command]
fn create_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    repo_root: Option<String>,
    session_id: SessionId,
) -> Result<layout::WorkspaceId, String> {
    let repo_root = repo_root.map(|r| {
        let expanded = session::expand_home(std::path::Path::new(&r));
        repo::canonicalize_or(&expanded)
            .to_string_lossy()
            .into_owned()
    });
    let id = state
        .layout
        .create_workspace(&name, repo_root, session_id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(id)
}

#[tauri::command]
fn close_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::WorkspaceId,
) -> Result<(), String> {
    let bound = state
        .layout
        .close_workspace(id)
        .map_err(|e| e.to_string())?;
    dispose_all(&app, &state, &bound);
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn activate_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::WorkspaceId,
) -> Result<(), String> {
    state
        .layout
        .activate_workspace(id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn create_tab(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: SessionId,
    workspace_id: Option<layout::WorkspaceId>,
) -> Result<layout::TabId, String> {
    let id = state
        .layout
        .create_tab(session_id, workspace_id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(id)
}

#[tauri::command]
fn close_tab(app: AppHandle, state: State<'_, AppState>, id: layout::TabId) -> Result<(), String> {
    let bound = state.layout.close_tab(id).map_err(|e| e.to_string())?;
    dispose_shells(&state, &bound);
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn rename_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::WorkspaceId,
    name: String,
) -> Result<(), String> {
    state
        .layout
        .rename_workspace(id, &name)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn set_workspace_color(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::WorkspaceId,
    color: Option<String>,
) -> Result<(), String> {
    state
        .layout
        .set_workspace_color(id, color)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn set_workspace_group(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::WorkspaceId,
    group: Option<String>,
) -> Result<(), String> {
    state
        .layout
        .set_workspace_group(id, group)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn new_window(app: AppHandle) -> Result<(), String> {
    let label = format!("tyba-{}", uuid::Uuid::new_v4().simple());
    let builder = tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::default())
        .title("Tyba")
        .inner_size(1100.0, 720.0);
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    builder.build().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn list_editors() -> Vec<editor::Editor> {
    editor::detect()
}

#[tauri::command]
fn get_pref(state: State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    if !key.starts_with("pref.") {
        return Err("chave de preferência inválida".into());
    }
    state.store.get_setting(&key).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_pref(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    if !key.starts_with("pref.") {
        return Err("chave de preferência inválida".into());
    }
    state
        .store
        .set_setting(&key, &value)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn activate_tab(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::TabId,
) -> Result<(), String> {
    state.layout.activate_tab(id).map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn move_tab(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::TabId,
    to: usize,
) -> Result<(), String> {
    state.layout.move_tab(id, to).map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn open_session_in_tab(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<(), String> {
    state
        .layout
        .open_session(session_id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn split_pane(
    app: AppHandle,
    state: State<'_, AppState>,
    pane_id: layout::PaneId,
    kind: layout::SplitKind,
    session_id: SessionId,
) -> Result<layout::PaneId, String> {
    let id = state
        .layout
        .split_pane(pane_id, kind, session_id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(id)
}

#[tauri::command]
fn close_pane(
    app: AppHandle,
    state: State<'_, AppState>,
    pane_id: layout::PaneId,
) -> Result<(), String> {
    let unbound = state
        .layout
        .close_pane(pane_id)
        .map_err(|e| e.to_string())?;
    dispose_shells(&state, &unbound);
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn focus_pane(
    app: AppHandle,
    state: State<'_, AppState>,
    pane_id: layout::PaneId,
) -> Result<(), String> {
    state
        .layout
        .focus_pane(pane_id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn set_split_ratio(
    app: AppHandle,
    state: State<'_, AppState>,
    pane_id: layout::PaneId,
    ratio: f64,
    commit: Option<bool>,
) -> Result<(), String> {
    state
        .layout
        .set_split_ratio(pane_id, ratio, commit.unwrap_or(true))
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn docker_available(state: State<'_, AppState>) -> bool {
    state.docker.available()
}

#[tauri::command]
fn docker_list_containers(
    state: State<'_, AppState>,
    repo_root: Option<String>,
    all: bool,
) -> Result<Vec<docker::ContainerInfo>, String> {
    state
        .docker
        .list(repo_root.as_deref(), all)
        .map_err(|e| e.to_string())
}

fn open_container_tab(
    app: &AppHandle,
    state: &State<'_, AppState>,
    container_id: &str,
    tab: docker::ContainerTab,
) -> Result<(), String> {
    let name = state
        .docker
        .container_name(container_id)
        .map_err(|e| e.to_string())?;

    if let Some(existing) = state.docker.tab_session(container_id, tab) {
        if state.sessions.get(existing).is_some() {
            state
                .layout
                .open_session(existing)
                .map_err(|e| e.to_string())?;
            emit_layout(app, state);
            return Ok(());
        }
    }
    let workspace_id = Some(state.layout.docker_workspace().map_err(|e| e.to_string())?);

    let bin = docker::docker_bin().ok_or("binário docker não encontrado")?;
    let (args, title) = match tab {
        docker::ContainerTab::Logs => (
            vec![
                "logs".to_string(),
                "-f".to_string(),
                "--tail".to_string(),
                "200".to_string(),
                container_id.to_string(),
            ],
            format!("logs: {name}"),
        ),
        docker::ContainerTab::Shell => (
            vec![
                "exec".to_string(),
                "-it".to_string(),
                container_id.to_string(),
                "sh".to_string(),
                "-c".to_string(),
                "command -v bash >/dev/null && exec bash || exec sh".to_string(),
            ],
            format!("sh: {name}"),
        ),
    };

    let session = spawn_tab_session(
        app,
        state,
        bin.as_path(),
        &args,
        title,
        None,
        &name,
        workspace_id,
    )?;
    state.docker.remember_tab(container_id, tab, session);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn spawn_tab_session(
    app: &AppHandle,
    state: &State<'_, AppState>,
    program: &std::path::Path,
    args: &[String],
    title: String,
    cwd: Option<&std::path::Path>,
    fallback_workspace: &str,
    workspace_id: Option<layout::WorkspaceId>,
) -> Result<SessionId, String> {
    let handle = app.clone();
    let session = state
        .sessions
        .create_command_session(
            app.clone(),
            &state.pty_pool,
            program,
            args,
            title,
            cwd,
            100,
            30,
            move |id| session_exited(&handle, id),
        )
        .map_err(|e| e.to_string())?;

    if let Err(e) = state.layout.create_tab(session.id, workspace_id) {
        if matches!(e, layout::LayoutError::NoActiveWorkspace) {
            state
                .layout
                .create_workspace(fallback_workspace, None, session.id)
                .map_err(|e| e.to_string())?;
        } else {
            return Err(e.to_string());
        }
    }
    emit_layout(app, state);
    Ok(session.id)
}

#[tauri::command]
fn docker_open_logs(
    app: AppHandle,
    state: State<'_, AppState>,
    container_id: String,
) -> Result<(), String> {
    open_container_tab(&app, &state, &container_id, docker::ContainerTab::Logs)
}

#[tauri::command]
fn docker_open_shell(
    app: AppHandle,
    state: State<'_, AppState>,
    container_id: String,
) -> Result<(), String> {
    open_container_tab(&app, &state, &container_id, docker::ContainerTab::Shell)
}

#[tauri::command]
fn open_view_tab(app: AppHandle, state: State<'_, AppState>, view: String) -> Result<(), String> {
    if view != layout::VIEW_SETTINGS && view != layout::VIEW_WORKSPACE {
        return Err(format!("view desconhecida: {view}"));
    }
    state
        .layout
        .open_view_tab(&view)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn docker_open_dashboard(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state
        .layout
        .open_docker_dashboard()
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn docker_compose_op(
    app: AppHandle,
    state: State<'_, AppState>,
    project: String,
    op: docker::ComposeOp,
) -> Result<(), String> {
    let info = state
        .docker
        .project_info(&project)
        .map_err(|e| e.to_string())?;
    docker::validate_working_dir(&info.working_dir).map_err(|e| e.to_string())?;
    let bin = docker::docker_bin().ok_or("binário docker não encontrado")?;
    let workspace_id = Some(state.layout.docker_workspace().map_err(|e| e.to_string())?);

    let script = format!(
        "'{}' {}; ec=$?; if [ $ec -ne 0 ]; then printf '\\n[falhou — enter para fechar]\\n'; read _; fi",
        bin.display(),
        op.compose_args(),
    );
    let title = format!("compose {}: {}", op.label(), project);
    spawn_tab_session(
        &app,
        &state,
        std::path::Path::new("/bin/sh"),
        &["-c".to_string(), script],
        title,
        Some(std::path::Path::new(&info.working_dir)),
        &project,
        workspace_id,
    )?;
    Ok(())
}

#[tauri::command]
fn docker_open_project(
    app: AppHandle,
    state: State<'_, AppState>,
    project: String,
) -> Result<(), String> {
    let info = state
        .docker
        .project_info(&project)
        .map_err(|e| e.to_string())?;
    docker::validate_working_dir(&info.working_dir).map_err(|e| e.to_string())?;

    let existing = state
        .layout
        .state()
        .workspaces
        .iter()
        .find(|w| w.repo_root.as_deref() == Some(info.working_dir.as_str()))
        .map(|w| w.id);
    if let Some(id) = existing {
        state
            .layout
            .activate_workspace(id)
            .map_err(|e| e.to_string())?;
        emit_layout(&app, &state);
        return Ok(());
    }

    let handle = app.clone();
    let session = state
        .sessions
        .create_shell_session(
            app.clone(),
            &state.pty_pool,
            CreateSessionOpts {
                kind: SessionKind::Shell,
                title: None,
                cwd: Some(std::path::PathBuf::from(&info.working_dir)),
                cols: 100,
                rows: 30,
                worktree_task: None,
            },
            move |id| session_exited(&handle, id),
        )
        .map_err(|e| e.to_string())?;
    state
        .layout
        .create_workspace(&project, Some(info.working_dir), session.id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn docker_open_compose_file(
    app: AppHandle,
    state: State<'_, AppState>,
    project: String,
) -> Result<(), String> {
    let info = state
        .docker
        .project_info(&project)
        .map_err(|e| e.to_string())?;
    let file = info.config_file.ok_or("projeto sem arquivo compose")?;
    docker::validate_compose_file(&file).map_err(|e| e.to_string())?;
    docker::validate_working_dir(&info.working_dir).map_err(|e| e.to_string())?;
    let workspace_id = Some(state.layout.docker_workspace().map_err(|e| e.to_string())?);

    let shell = session::default_shell();
    let script = format!("exec \"${{EDITOR:-vi}}\" '{file}'");
    let title = format!("compose: {project}");
    spawn_tab_session(
        &app,
        &state,
        std::path::Path::new(&shell),
        &["-lc".to_string(), script],
        title,
        Some(std::path::Path::new(&info.working_dir)),
        &project,
        workspace_id,
    )?;
    Ok(())
}

#[tauri::command]
fn docker_remove_container(state: State<'_, AppState>, container_id: String) -> Result<(), String> {
    state
        .docker
        .remove(&container_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn docker_open_desktop() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let attempts: [&[&str]; 2] = [&["-a", "Docker"], &["-b", "com.docker.docker"]];
        let mut last_error = String::new();
        for args in attempts {
            match std::process::Command::new("/usr/bin/open")
                .args(args)
                .output()
            {
                Ok(out) if out.status.success() => return Ok(()),
                Ok(out) => {
                    last_error = String::from_utf8_lossy(&out.stderr).trim().to_string();
                }
                Err(e) => last_error = e.to_string(),
            }
        }
        if last_error.is_empty() {
            last_error = "não foi possível abrir o Docker Desktop".into();
        }
        Err(last_error)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("disponível apenas no macOS".into())
    }
}

#[tauri::command]
fn request_approval(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: SessionId,
    command: String,
    cwd: Option<String>,
    context: Option<String>,
) -> Result<ApprovalRequest, String> {
    state
        .approvals
        .request(&app, session_id, command, cwd, context)
}

#[tauri::command]
fn list_approvals(state: State<'_, AppState>) -> Vec<ApprovalRequest> {
    state.approvals.list_pending()
}

#[tauri::command]
fn resolve_approval(
    app: AppHandle,
    state: State<'_, AppState>,
    id: u64,
    decision: Decision,
    feedback: Option<String>,
) -> Result<(), String> {
    let request = state.approvals.resolve(&app, id, decision, feedback)?;
    agent::session::record_history(
        &state.store,
        request.session_id,
        request.command,
        request.cwd,
        request.risk,
        agent::session::decision_label(decision),
        request.requested_at_ms,
    );
    Ok(())
}

#[derive(serde::Serialize)]
struct AgentConfigInfo {
    hash: String,
    default_agent: Option<String>,
    env_allow: Vec<String>,
    consent: Option<bool>,
}

#[tauri::command]
fn agent_binary_available(runner: crate::session::AgentRunnerKind) -> bool {
    agent::binary_available(&runner)
}

#[tauri::command]
fn agent_repo_config(
    state: State<'_, AppState>,
    repo_root: String,
) -> Result<Option<AgentConfigInfo>, String> {
    let param = repo::canonicalize_or(std::path::Path::new(&repo_root));
    let root = repo::toplevel(&param)
        .map(|t| repo::canonicalize_or(&t))
        .ok_or("fora de repositório git")?;
    let Some((config, hash)) = repo_config::load(&root).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let consent = state
        .store
        .config_consent(&root.to_string_lossy(), &hash)
        .ok()
        .flatten();
    Ok(Some(AgentConfigInfo {
        hash,
        default_agent: config.default_agent,
        env_allow: config.env_allow,
        consent,
    }))
}

#[tauri::command]
fn set_agent_config_consent(
    state: State<'_, AppState>,
    repo_root: String,
    hash: String,
    allowed: bool,
) -> Result<(), String> {
    let param = repo::canonicalize_or(std::path::Path::new(&repo_root));
    let root = repo::toplevel(&param)
        .map(|t| repo::canonicalize_or(&t))
        .ok_or("fora de repositório git")?;
    state
        .store
        .set_config_consent(&root.to_string_lossy(), &hash, allowed)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_themes(state: State<'_, AppState>) -> Vec<theme::Theme> {
    state.themes.list()
}

#[tauri::command]
fn get_theme_state(state: State<'_, AppState>) -> theme::ThemeState {
    state.themes.state()
}

#[tauri::command]
fn set_theme_mode(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: theme::ThemeMode,
) -> Result<(), String> {
    state.themes.set_mode(&app, mode).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_theme_slot(
    app: AppHandle,
    state: State<'_, AppState>,
    base: theme::ThemeBase,
    id: String,
) -> Result<(), String> {
    state
        .themes
        .set_slot(&app, base, &id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn import_theme(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<theme::Theme, String> {
    state.themes.import(&app, &path).map_err(|e| e.to_string())
}

const SCROLLBACK_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

fn open_store(app: &AppHandle) -> session::store::Store {
    let db_path = std::env::var_os("TYBA_DATA_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| app.path().app_data_dir().ok())
        .map(|dir| {
            let _ = std::fs::create_dir_all(&dir);
            dir.join("tyba.db")
        });

    db_path
        .and_then(|path| session::store::Store::open(&path).ok())
        .or_else(|| session::store::Store::open_in_memory().ok())
        .expect("failed to open session store")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(state) = window.try_state::<AppState>() {
                    state.pty_pool.drop_window_attachers(window.label());
                }
            }
        })
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                if let Some(state) = webview.try_state::<AppState>() {
                    state.pty_pool.drop_window_attachers(webview.label());
                }
            }
        })
        .setup(|app| {
            let store = Arc::new(open_store(app.handle()));
            let pty_pool: SharedPtyPool = Arc::new(pty::PtyPool::new());
            let sessions: SharedSessionManager =
                Arc::new(session::SessionManager::new(Arc::clone(&store)));
            let _ = sessions.restore();

            let layout: layout::SharedLayout =
                Arc::new(layout::LayoutManager::new(Arc::clone(&store)));
            let valid: std::collections::HashSet<SessionId> =
                sessions.list().iter().map(|s| s.id).collect();
            layout.load(&valid);

            let themes_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("tyba"))
                .join("themes");
            let themes: theme::SharedThemes =
                Arc::new(theme::ThemeManager::new(Arc::clone(&store), themes_dir));

            let repos: repo::SharedRepoWatcher = Arc::new(repo::RepoWatcher::new());
            let (reconcile_tx, reconcile_rx) = std::sync::mpsc::channel::<()>();

            app.manage(AppState {
                store: Arc::clone(&store),
                pty_pool: Arc::clone(&pty_pool),
                sessions: Arc::clone(&sessions),
                approvals: Arc::new(approvals::ApprovalsManager::new()),
                themes,
                layout,
                docker: Arc::new(docker::DockerManager::new()),
                repos,
                repo_reconcile: reconcile_tx.clone(),
                rich_input_submit: parking_lot::Mutex::new(()),
                worktree_files: rich_input::FilesCache::default(),
                hook_servers: Arc::new(agent::session::HookServerRegistry::default()),
            });

            std::thread::Builder::new()
                .name("scrollback-flush".into())
                .spawn(move || loop {
                    std::thread::sleep(SCROLLBACK_FLUSH_INTERVAL);
                    sessions.flush_scrollback(&pty_pool);
                })
                .expect("failed to spawn scrollback flush thread");

            let gc_sessions = Arc::clone(&app.state::<AppState>().sessions);
            std::thread::Builder::new()
                .name("worktree-gc".into())
                .spawn(move || {
                    let report = worktree::gc_orphans(&known_worktree_paths(&gc_sessions));
                    for removed in &report.removed {
                        eprintln!("[tyba] worktree órfão removido: {}", removed.display());
                    }
                })
                .expect("failed to spawn worktree gc thread");

            let cwd_tx = reconcile_tx.clone();
            app.listen_any(pty::EVENT_CWD_CHANGED, move |_| {
                let _ = cwd_tx.send(());
            });

            let reconcile_handle = app.handle().clone();
            std::thread::Builder::new()
                .name("repo-reconcile".into())
                .spawn(move || {
                    while reconcile_rx.recv().is_ok() {
                        std::thread::sleep(Duration::from_millis(300));
                        while reconcile_rx.try_recv().is_ok() {}
                        reconcile_repo_watchers(&reconcile_handle);
                    }
                })
                .expect("failed to spawn repo reconcile thread");

            let _ = reconcile_tx.send(());

            if let Some(window) = app.get_webview_window("main") {
                let hidden = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = hidden.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            agent_binary_available,
            create_session,
            write_to_session,
            submit_rich_input,
            set_agent_match_pattern,
            prompt_mentions_sensitive,
            session_bracketed_paste,
            session_rel_path,
            list_worktree_files,
            attach_session,
            detach_session,
            worktree_list,
            worktree_setup_script,
            worktree_set_setup_consent,
            worktree_remove,
            worktree_gc,
            session_diff,
            session_diff_hunks,
            session_conflicts,
            session_conflict_choose,
            session_conflict_mark_resolved,
            session_branches,
            session_fetch,
            session_checkout,
            suggest_commit_message,
            session_branch_diff,
            session_branch_hunks,
            open_diff_tab,
            close_side_view,
            set_side_view_expanded,
            set_side_view_ratio,
            worktree_stage,
            worktree_unstage,
            worktree_discard,
            worktree_commit,
            worktree_push,
            worktree_merge_preview,
            worktree_merge_materialize,
            worktree_merge_into_base,
            forge_status,
            forge_pr_for_session,
            forge_pr_comments,
            forge_create_pr,
            forge_pr_list,
            session_git_status,
            open_worktree_file,
            repo_snapshots,
            session_cwd,
            resize_session,
            list_sessions,
            session_mark_seen,
            dispose_session,
            request_approval,
            list_approvals,
            resolve_approval,
            agent_repo_config,
            set_agent_config_consent,
            list_themes,
            get_theme_state,
            set_theme_mode,
            set_theme_slot,
            import_theme,
            layout_state,
            create_workspace,
            close_workspace,
            activate_workspace,
            rename_workspace,
            set_workspace_color,
            set_workspace_group,
            new_window,
            create_tab,
            close_tab,
            activate_tab,
            move_tab,
            open_session_in_tab,
            split_pane,
            close_pane,
            focus_pane,
            set_split_ratio,
            get_pref,
            set_pref,
            list_editors,
            docker_available,
            docker_list_containers,
            docker_open_logs,
            docker_open_shell,
            docker_remove_container,
            docker_open_desktop,
            docker_compose_op,
            docker_open_project,
            docker_open_compose_file,
            docker_open_dashboard,
            open_view_tab,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tyba")
        .run(|_app_handle, _event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                if let Some(window) = _app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });
}
