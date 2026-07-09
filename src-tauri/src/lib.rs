pub mod agent;
pub mod approvals;
pub mod docker;
pub mod layout;
pub mod pty;
pub mod sandbox;
pub mod session;
pub mod status;
pub mod theme;
pub mod worktree;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use tauri::{AppHandle, Emitter, Manager, State};

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
}

fn emit_layout(app: &AppHandle, state: &State<'_, AppState>) {
    let _ = app.emit(layout::EVENT_CHANGED, state.layout.state());
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
    }
}

#[tauri::command]
fn create_session(
    app: AppHandle,
    state: State<'_, AppState>,
    opts: CreateSessionOpts,
) -> Result<Session, String> {
    let handle = app.clone();
    state
        .sessions
        .create_shell_session(app, &state.pty_pool, opts, move |id| {
            session_exited(&handle, id)
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn write_to_session(state: State<'_, AppState>, id: SessionId, data: String) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| e.to_string())?;
    state.pty_pool.write(id, &bytes).map_err(|e| e.to_string())
}

#[tauri::command]
fn attach_session(app: AppHandle, state: State<'_, AppState>, id: SessionId) -> Result<(), String> {
    state.pty_pool.attach(&app, id).map_err(|e| e.to_string())
}

#[tauri::command]
fn detach_session(state: State<'_, AppState>, id: SessionId) {
    state.pty_pool.detach(id);
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
        session::expand_home(std::path::Path::new(&r))
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
fn repo_branch(path: String) -> Option<String> {
    let path = session::expand_home(std::path::Path::new(&path));
    let out = worktree::git_in(&path)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

#[derive(serde::Serialize)]
struct RepoStatus {
    dirty: bool,
    changed: u32,
    insertions: u32,
    deletions: u32,
}

fn diff_numstat(path: &std::path::Path) -> (u32, u32) {
    let out = worktree::git_in(path)
        .args(["diff", "--no-ext-diff", "--numstat", "--no-color", "HEAD"])
        .output();
    let Ok(out) = out else {
        return (0, 0);
    };
    if !out.status.success() {
        return (0, 0);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut insertions = 0;
    let mut deletions = 0;
    for line in text.lines() {
        let mut fields = line.split('\t');
        let added = fields.next().and_then(|v| v.parse::<u32>().ok());
        let removed = fields.next().and_then(|v| v.parse::<u32>().ok());
        insertions += added.unwrap_or(0);
        deletions += removed.unwrap_or(0);
    }
    (insertions, deletions)
}

const UNTRACKED_MAX_BYTES: u64 = 512 * 1024;
const UNTRACKED_MAX_FILES: usize = 500;

fn untracked_insertions(path: &std::path::Path) -> u32 {
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    let list = worktree::git_in(path)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .output();
    let Ok(list) = list else {
        return 0;
    };
    let mut lines = 0u32;
    for file in list
        .stdout
        .split(|b| *b == 0)
        .filter(|f| !f.is_empty())
        .take(UNTRACKED_MAX_FILES)
    {
        #[cfg(unix)]
        let full = path.join(std::ffi::OsStr::from_bytes(file));
        #[cfg(not(unix))]
        let full = path.join(String::from_utf8_lossy(file).as_ref());

        let Ok(meta) = std::fs::metadata(&full) else {
            continue;
        };
        if !meta.is_file() || meta.len() > UNTRACKED_MAX_BYTES {
            continue;
        }
        if let Ok(content) = std::fs::read(&full) {
            if content.contains(&0) {
                continue;
            }
            let mut count = content.iter().filter(|b| **b == b'\n').count();
            if !content.is_empty() && content.last() != Some(&b'\n') {
                count += 1;
            }
            lines += count as u32;
        }
    }
    lines
}

fn count_status_entries(stdout: &[u8]) -> u32 {
    let mut fields = stdout.split(|b| *b == 0).filter(|entry| !entry.is_empty());
    let mut changed = 0u32;
    while let Some(entry) = fields.next() {
        changed += 1;
        if matches!(entry.first(), Some(b'R') | Some(b'C')) {
            fields.next();
        }
    }
    changed
}

#[tauri::command]
fn repo_status(path: String) -> Option<RepoStatus> {
    let path = session::expand_home(std::path::Path::new(&path));
    let out = worktree::git_in(&path)
        .args(["status", "--porcelain", "-z"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let changed = count_status_entries(&out.stdout);
    let (mut insertions, deletions) = if changed > 0 {
        diff_numstat(&path)
    } else {
        (0, 0)
    };
    if changed > 0 {
        insertions += untracked_insertions(&path);
    }
    Some(RepoStatus {
        dirty: changed > 0,
        changed,
        insertions,
        deletions,
    })
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

            app.manage(AppState {
                store: Arc::clone(&store),
                pty_pool: Arc::clone(&pty_pool),
                sessions: Arc::clone(&sessions),
                approvals: Arc::new(approvals::ApprovalsManager::new()),
                themes,
                layout,
                docker: Arc::new(docker::DockerManager::new()),
            });

            std::thread::Builder::new()
                .name("scrollback-flush".into())
                .spawn(move || loop {
                    std::thread::sleep(SCROLLBACK_FLUSH_INTERVAL);
                    sessions.flush_scrollback(&pty_pool);
                })
                .expect("failed to spawn scrollback flush thread");

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
            attach_session,
            detach_session,
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
            repo_branch,
            repo_status,
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

#[cfg(test)]
mod tests {
    use super::count_status_entries;

    #[test]
    fn counts_empty_output_as_zero() {
        assert_eq!(count_status_entries(b""), 0);
    }

    #[test]
    fn counts_plain_entries() {
        assert_eq!(count_status_entries(b" M keep.txt\0?? new.txt\0"), 2);
    }

    #[test]
    fn rename_counts_once_despite_two_paths() {
        assert_eq!(count_status_entries(b"R  new.txt\0old.txt\0"), 1);
    }

    #[test]
    fn copy_counts_once_despite_two_paths() {
        assert_eq!(count_status_entries(b"C  copy.txt\0src.txt\0"), 1);
    }

    #[test]
    fn rename_with_worktree_modification_counts_once() {
        assert_eq!(count_status_entries(b"RM new.txt\0old.txt\0"), 1);
    }

    #[test]
    fn mixed_entries_match_file_count() {
        let out = b" M keep.txt\0R  new.txt\0old.txt\0?? untracked.txt\0";
        assert_eq!(count_status_entries(out), 3);
    }

    #[test]
    fn orig_path_starting_with_r_is_not_treated_as_entry() {
        let out = b"R  new.txt\0Renamed.txt\0 M keep.txt\0";
        assert_eq!(count_status_entries(out), 2);
    }

    #[test]
    fn untracked_path_starting_with_r_counts_normally() {
        assert_eq!(count_status_entries(b"?? Rakefile\0?? README.md\0"), 2);
    }

    #[test]
    fn truncated_rename_record_does_not_panic() {
        assert_eq!(count_status_entries(b"R  new.txt\0"), 1);
    }
}
