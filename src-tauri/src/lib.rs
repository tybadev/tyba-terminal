pub mod agent;
pub mod approvals;
pub mod docker;
pub mod editor;
pub mod layout;
pub mod pty;
pub mod repo;
pub mod rich_input;
pub mod sandbox;
pub mod session;
pub mod status;
pub mod theme;
pub mod worktree;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use tauri::{AppHandle, Emitter, Listener, Manager, State};

use approvals::{ApprovalRequest, Decision, SharedApprovals};
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

fn dispose_all(state: &State<'_, AppState>, ids: &[SessionId]) {
    for id in ids {
        state.sessions.dispose(&state.pty_pool, *id);
    }
}

fn session_exited(app: &AppHandle, id: SessionId) {
    let state = app.state::<AppState>();
    let Some(session) = state.sessions.get(id) else {
        return;
    };
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

#[tauri::command]
fn create_session(
    app: AppHandle,
    state: State<'_, AppState>,
    opts: CreateSessionOpts,
) -> Result<Session, String> {
    let handle = app.clone();
    let session = state
        .sessions
        .create_shell_session(app.clone(), &state.pty_pool, opts, move |id| {
            session_exited(&handle, id)
        })
        .map_err(|e| e.to_string())?;
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

#[tauri::command]
async fn worktree_list(
    state: State<'_, AppState>,
    repo_root: String,
) -> Result<Vec<WorktreeStatus>, String> {
    let root = repo::canonicalize_or(std::path::Path::new(&repo_root));
    let head = worktree::head_sha(&root)?;
    let by_path: std::collections::HashMap<std::path::PathBuf, SessionId> = state
        .sessions
        .list()
        .into_iter()
        .filter_map(|s| s.worktree.map(|wt| (repo::canonicalize_or(&wt.path), s.id)))
        .collect();

    Ok(worktree::list(&root)?
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
        .collect())
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
    let root = repo::canonicalize_or(std::path::Path::new(&repo_root));
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
    let root = repo::canonicalize_or(std::path::Path::new(&repo_root));
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
fn dispose_session(app: AppHandle, state: State<'_, AppState>, id: SessionId) {
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
    dispose_all(&state, &bound);
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
        .title("TYBA")
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
    if view != layout::VIEW_SETTINGS {
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
) -> Result<(), String> {
    state.approvals.resolve(&app, id, decision)
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
    let db_path = app
        .path()
        .app_data_dir()
        .map(|dir| {
            let _ = std::fs::create_dir_all(&dir);
            dir.join("tyba.db")
        })
        .ok();

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
            repo_snapshots,
            session_cwd,
            resize_session,
            list_sessions,
            dispose_session,
            request_approval,
            list_approvals,
            resolve_approval,
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
