pub mod agent;
pub mod pty;
pub mod sandbox;
pub mod session;
pub mod status;
pub mod worktree;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use tauri::{AppHandle, Manager, State};

use pty::SharedPtyPool;
use session::{CreateSessionOpts, Session, SessionId, SharedSessionManager};

struct AppState {
    pty_pool: SharedPtyPool,
    sessions: SharedSessionManager,
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
fn dispose_session(state: State<'_, AppState>, id: SessionId) {
    state.sessions.dispose(&state.pty_pool, id);
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
        .setup(|app| {
            let store = Arc::new(open_store(app.handle()));
            let pty_pool: SharedPtyPool = Arc::new(pty::PtyPool::new());
            let sessions: SharedSessionManager = Arc::new(session::SessionManager::new(store));
            let _ = sessions.restore();

            app.manage(AppState {
                pty_pool: Arc::clone(&pty_pool),
                sessions: Arc::clone(&sessions),
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building tyba")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Reopen { .. } = event {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });
}
