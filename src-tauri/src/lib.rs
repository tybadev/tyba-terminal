pub mod agent;
pub mod approvals;
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
use session::{CreateSessionOpts, Session, SessionId, SessionKind, SharedSessionManager};

struct AppState {
    store: Arc<Store>,
    pty_pool: SharedPtyPool,
    sessions: SharedSessionManager,
    approvals: SharedApprovals,
    themes: theme::SharedThemes,
    layout: layout::SharedLayout,
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

#[tauri::command]
fn create_session(
    app: AppHandle,
    state: State<'_, AppState>,
    opts: CreateSessionOpts,
) -> Result<Session, String> {
    state
        .sessions
        .create_shell_session(app, &state.pty_pool, opts)
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
fn session_scrollback(state: State<'_, AppState>, id: SessionId) -> Result<String, String> {
    let bytes = state.pty_pool.scrollback(id).map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
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
fn close_tab(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::TabId,
) -> Result<(), String> {
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
    let out = std::process::Command::new("git")
        .args(["-C", &path, "rev-parse", "--abbrev-ref", "HEAD"])
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
) -> Result<(), String> {
    state
        .layout
        .set_split_ratio(pane_id, ratio)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
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
    state
        .themes
        .import(&app, &path)
        .map_err(|e| e.to_string())
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
            session_scrollback,
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
