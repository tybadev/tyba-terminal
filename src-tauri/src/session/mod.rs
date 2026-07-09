pub mod redact;
pub mod store;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

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

    fn shell_integration_enabled(&self) -> bool {
        self.store
            .get_setting("pref.shell_integration")
            .ok()
            .flatten()
            .map(|v| v != "off")
            .unwrap_or(true)
    }

    pub fn create_shell_session(
        &self,
        app: AppHandle,
        pty_pool: &SharedPtyPool,
        opts: CreateSessionOpts,
        on_exit: impl FnOnce(SessionId) + Send + 'static,
    ) -> Result<Session, PtyError> {
        let id = Uuid::new_v4();

        let shell = default_shell();
        let mut cmd = CommandBuilder::new(&shell);
        if cfg!(unix) {
            cmd.arg("-l");
        }
        cmd.cwd(resolve_cwd(opts.cwd.as_deref()));

        if shell_label(&shell) == "zsh" && self.shell_integration_enabled() {
            if let Some(dir) = zsh_integration_dir() {
                let user_zdotdir = std::env::var("ZDOTDIR")
                    .ok()
                    .filter(|s| !s.is_empty())
                    .or_else(|| std::env::var("HOME").ok())
                    .unwrap_or_default();
                cmd.env("ZDOTDIR", dir);
                cmd.env("TYBA_USER_ZDOTDIR", user_zdotdir);
            }
        }

        let title = opts.title.unwrap_or_else(|| shell_label(&shell));
        self.spawn_session(
            app, pty_pool, id, cmd, opts.kind, title, opts.cols, opts.rows, on_exit,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_command_session(
        &self,
        app: AppHandle,
        pty_pool: &SharedPtyPool,
        program: &std::path::Path,
        args: &[String],
        title: String,
        cwd: Option<&std::path::Path>,
        cols: u16,
        rows: u16,
        on_exit: impl FnOnce(SessionId) + Send + 'static,
    ) -> Result<Session, PtyError> {
        let id = Uuid::new_v4();
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }
        self.spawn_session(
            app,
            pty_pool,
            id,
            cmd,
            SessionKind::Shell,
            title,
            cols,
            rows,
            on_exit,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_session(
        &self,
        app: AppHandle,
        pty_pool: &SharedPtyPool,
        id: SessionId,
        mut cmd: CommandBuilder,
        kind: SessionKind,
        title: String,
        cols: u16,
        rows: u16,
        on_exit: impl FnOnce(SessionId) + Send + 'static,
    ) -> Result<Session, PtyError> {
        cmd.env("TERM", "xterm-256color");
        cmd.env("TYBA", "1");
        cmd.env("TYBA_SESSION_ID", id.to_string());

        pty_pool.spawn(
            app.clone(),
            id,
            cmd,
            None,
            cols,
            rows,
            Box::new(move || on_exit(id)),
        )?;

        let session = Session {
            id,
            kind,
            title,
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

    pub fn get(&self, id: SessionId) -> Option<Session> {
        self.sessions.read().get(&id).cloned()
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
        for mut s in persisted {
            if matches!(s.kind, SessionKind::Shell) {
                let _ = self.store.remove_session(s.id);
                continue;
            }
            if !matches!(
                s.status,
                SessionStatus::Exited { .. } | SessionStatus::Failed { .. }
            ) {
                s.status = SessionStatus::Exited { code: -1 };
                let _ = self.store.upsert_session(&s);
            }
            sessions.entry(s.id).or_insert(s);
        }
        Ok(())
    }
}

fn emit_status(app: &AppHandle, session: &Session) {
    let _ = app.emit(&format!("session://status/{}", session.id), session.clone());
}

pub fn expand_home(path: &Path) -> PathBuf {
    let Ok(home) = std::env::var("HOME") else {
        return path.to_path_buf();
    };
    let raw = path.to_string_lossy();
    if raw == "~" {
        return PathBuf::from(home);
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

pub fn resolve_cwd(requested: Option<&Path>) -> PathBuf {
    let home = || {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"))
    };
    match requested {
        Some(path) => {
            let expanded = expand_home(path);
            if expanded.is_dir() {
                expanded
            } else {
                home()
            }
        }
        None => home(),
    }
}

/// Diretório com os arquivos de shell integration do zsh (ZDOTDIR).
/// Escrito uma vez em temp. Os arquivos sourceiam a config do usuário
/// (via TYBA_USER_ZDOTDIR) e adicionam os hooks OSC 133/633 — padrão
/// consolidado (VS Code/iTerm2): mexe só no ambiente da sessão, nunca
/// nos dotfiles do usuário; se algo falhar, o shell do usuário segue.
fn zsh_integration_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| write_zsh_integration().ok()).as_deref()
}

fn write_zsh_integration() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("tyba-zsh-integration");
    std::fs::create_dir_all(&dir)?;
    let self_dir = dir.to_string_lossy().replace('"', "");

    let chain = |file: &str| {
        format!(
            "if [[ -f \"$TYBA_USER_ZDOTDIR/{file}\" ]]; then\n  \
             ZDOTDIR=\"$TYBA_USER_ZDOTDIR\"\n  \
             source \"$TYBA_USER_ZDOTDIR/{file}\"\n  \
             ZDOTDIR=\"{self_dir}\"\nfi\n"
        )
    };

    std::fs::write(dir.join(".zshenv"), chain(".zshenv"))?;
    std::fs::write(dir.join(".zprofile"), chain(".zprofile"))?;
    std::fs::write(dir.join(".zlogin"), chain(".zlogin"))?;

    let hooks = "\n# TYBA shell integration (OSC 133/633/7)\n\
        if [[ -o interactive ]] && autoload -Uz add-zsh-hook 2>/dev/null; then\n  \
        __tyba_esc() { printf '\\033]%s\\007' \"$1\"; }\n  \
        __tyba_urlencode() { emulate -L zsh; local s=$1 out= i c; for (( i=1; i<=${#s}; i++ )); do c=$s[i]; if [[ $c == [A-Za-z0-9/._~-] ]]; then out+=$c; else out+=$(printf '%%%02X' ${(s: :)$(printf '%s' $c | od -An -tu1)}); fi; done; printf '%s' $out; }\n  \
        __tyba_osc7() { __tyba_esc \"7;file://${HOST}$(__tyba_urlencode \"$PWD\")\"; }\n  \
        __tyba_ps1b() { [[ \"$PS1\" == *$'\\033]133;B'* ]] || PS1=\"$PS1%{$(__tyba_esc '133;B')%}\"; }\n  \
        __tyba_preexec() { __tyba_esc \"633;E;$(print -rn -- \"$1\" | base64 | tr -d '\\n')\"; __tyba_esc \"133;C\"; }\n  \
        __tyba_precmd() { local __c=$?; __tyba_esc \"133;D;$__c\"; __tyba_esc \"133;A\"; __tyba_ps1b; __tyba_osc7; }\n  \
        add-zsh-hook preexec __tyba_preexec\n  \
        add-zsh-hook precmd __tyba_precmd\n  \
        add-zsh-hook chpwd __tyba_osc7\n  \
        __tyba_osc7\n\
        fi\n\
        ZDOTDIR=\"$TYBA_USER_ZDOTDIR\"\n";
    std::fs::write(dir.join(".zshrc"), format!("{}{}", chain(".zshrc"), hooks))?;

    Ok(dir)
}

pub fn default_shell() -> String {
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

    fn make(kind: SessionKind, status: SessionStatus) -> Session {
        Session {
            id: SessionId::new_v4(),
            kind,
            title: "s".into(),
            repo_root: None,
            worktree: None,
            status,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn expand_home_handles_tilde_variants() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_home(Path::new("~")), PathBuf::from(&home));
        assert_eq!(
            expand_home(Path::new("~/projects/x")),
            PathBuf::from(&home).join("projects/x")
        );
        assert_eq!(
            expand_home(Path::new("/abs/path")),
            PathBuf::from("/abs/path")
        );
        assert_eq!(expand_home(Path::new("~user/x")), PathBuf::from("~user/x"));
    }

    #[test]
    fn resolve_cwd_falls_back_to_home_when_missing_or_invalid() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        assert_eq!(resolve_cwd(None), home);
        assert_eq!(
            resolve_cwd(Some(Path::new("~/definitely-not-a-real-dir-xyz"))),
            home
        );
        let tmp = std::env::temp_dir();
        assert_eq!(resolve_cwd(Some(&tmp)), tmp);
        assert_eq!(resolve_cwd(Some(Path::new("~"))), home);
    }

    #[test]
    fn restore_removes_dead_shell_rows() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let shell = make(SessionKind::Shell, SessionStatus::Running);
        store.upsert_session(&shell).unwrap();

        let manager = SessionManager::new(Arc::clone(&store));
        manager.restore().unwrap();

        assert!(manager.list().is_empty());
        assert!(store.load_sessions().unwrap().is_empty());
    }

    #[test]
    fn restore_keeps_agents_downgrading_stale_live_status() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let running = make(
            SessionKind::Agent {
                runner: AgentRunnerKind::ClaudeCode,
            },
            SessionStatus::Running,
        );
        let done = make(
            SessionKind::Agent {
                runner: AgentRunnerKind::ClaudeCode,
            },
            SessionStatus::Exited { code: 0 },
        );
        for s in [&running, &done] {
            store.upsert_session(s).unwrap();
        }

        let manager = SessionManager::new(Arc::clone(&store));
        manager.restore().unwrap();

        let listed = manager.list();
        assert_eq!(listed.len(), 2);
        for s in listed {
            if s.id == done.id {
                assert!(matches!(s.status, SessionStatus::Exited { code: 0 }));
            } else {
                assert!(matches!(s.status, SessionStatus::Exited { code: -1 }));
            }
        }
    }
}
