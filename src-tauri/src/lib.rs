//! TYBA core — bootstrap Tauri e commands IPC.

pub mod agent;
pub mod pty;
pub mod sandbox;
pub mod session;
pub mod status;
pub mod worktree;

use std::sync::Arc;

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
    // Teclado chega como base64 do frontend (simetria com o output;
    // preserva bytes de sequências de controle).
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| e.to_string())?;
    state.pty_pool.write(id, &bytes).map_err(|e| e.to_string())
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            app.manage(AppState {
                pty_pool: Arc::new(pty::PtyPool::new()),
                sessions: Arc::new(session::SessionManager::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_session,
            write_to_session,
            resize_session,
            list_sessions,
            dispose_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tyba");
}
