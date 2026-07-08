pub mod redact;
pub mod store;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use portable_pty::CommandBuilder;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::pty::{PtyError, SharedPtyPool};
use crate::session::store::{Store, StoreError};
use crate::worktree::Worktree;

pub type SessionId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionKind {
    Shell,
    Agent { runner: AgentRunnerKind },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunnerKind {
    ClaudeCode,
    Codex,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    AwaitingInput { hint: Option<String> },
    Idle,
    Exited { code: i32 },
    Failed { reason: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub id: SessionId,
    pub kind: SessionKind,
    pub title: String,
    pub repo_root: Option<PathBuf>,
    pub worktree: Option<Worktree>,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionOpts {
    pub kind: SessionKind,
    pub title: Option<String>,
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
}

pub struct SessionManager {
    sessions: RwLock<HashMap<SessionId, Session>>,
    store: Arc<Store>,
}

impl SessionManager {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            store,
        }
    }

    pub fn create_shell_session(
        &self,
        app: AppHandle,
        pty_pool: &SharedPtyPool,
        opts: CreateSessionOpts,
    ) -> Result<Session, PtyError> {
        let id = Uuid::new_v4();

        let shell = default_shell();
        let mut cmd = CommandBuilder::new(&shell);
        if cfg!(unix) {
            cmd.arg("-l");
        }
        if let Some(cwd) = &opts.cwd {
            cmd.cwd(cwd);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("TYBA", "1");
        cmd.env("TYBA_SESSION_ID", id.to_string());

        pty_pool.spawn(app.clone(), id, cmd, None, opts.cols, opts.rows)?;

        let session = Session {
            id,
            kind: opts.kind,
            title: opts.title.unwrap_or_else(|| shell_label(&shell)),
            repo_root: None,
            worktree: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
        };
        self.sessions.write().insert(id, session.clone());
        let _ = self.store.upsert_session(&session);
        emit_status(&app, &session);
        Ok(session)
    }

    pub fn list(&self) -> Vec<Session> {
        let mut v: Vec<_> = self.sessions.read().values().cloned().collect();
        v.sort_by_key(|s| s.created_at);
        v
    }

    pub fn set_status(&self, app: &AppHandle, id: SessionId, status: SessionStatus) {
        let mut sessions = self.sessions.write();
        if let Some(s) = sessions.get_mut(&id) {
            s.status = status;
            let _ = self.store.upsert_session(s);
            emit_status(app, s);
        }
    }

    pub fn dispose(&self, pty_pool: &SharedPtyPool, id: SessionId) {
        let _ = pty_pool.kill(id);
        self.sessions.write().remove(&id);
        let _ = self.store.remove_session(id);
    }

    pub fn flush_scrollback(&self, pty_pool: &SharedPtyPool) {
        let ids: Vec<SessionId> = self.sessions.read().keys().copied().collect();
        for id in ids {
            if let Ok(text) = pty_pool.scrollback_text(id) {
                let _ = self.store.save_scrollback(id, &text);
            }
        }
    }

    pub fn restore(&self) -> Result<(), StoreError> {
        let persisted = self.store.load_sessions()?;
        let mut sessions = self.sessions.write();
        for s in persisted {
            sessions.entry(s.id).or_insert(s);
        }
        Ok(())
    }
}

fn emit_status(app: &AppHandle, session: &Session) {
    let _ = app.emit(&format!("session://status/{}", session.id), session.clone());
}

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".into())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
    }
}

fn shell_label(shell: &str) -> String {
    std::path::Path::new(shell)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| shell.to_string())
}

pub type SharedSessionManager = Arc<SessionManager>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_loads_persisted_sessions_from_store() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let persisted = Session {
            id: SessionId::new_v4(),
            kind: SessionKind::Shell,
            title: "restored".into(),
            repo_root: None,
            worktree: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
        };
        store.upsert_session(&persisted).unwrap();

        let manager = SessionManager::new(store);
        manager.restore().unwrap();

        let listed = manager.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, persisted.id);
        assert_eq!(listed[0].title, "restored");
    }
}
