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

pub const EDITOR_PREF_KEY: &str = "pref.editor";
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AwaitingReason {
    Approval,
    #[default]
    Reply,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    AwaitingInput {
        hint: Option<String>,
        #[serde(default)]
        reason: AwaitingReason,
    },
    Idle,
    Exited { code: i32 },
    Failed { reason: String },
}

impl SessionStatus {
    fn redacted(self) -> Self {
        match self {
            SessionStatus::AwaitingInput { hint, reason } => SessionStatus::AwaitingInput {
                hint: hint.map(|h| redact::redact(&h).into_owned()),
                reason,
            },
            SessionStatus::Failed { reason } => SessionStatus::Failed {
                reason: redact::redact(&reason).into_owned(),
            },
            other => other,
        }
    }

    fn wants_attention(&self) -> bool {
        matches!(
            self,
            SessionStatus::AwaitingInput { .. } | SessionStatus::Idle | SessionStatus::Failed { .. }
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub id: SessionId,
    pub kind: SessionKind,
    pub title: String,
    pub repo_root: Option<PathBuf>,
    pub worktree: Option<Worktree>,
    pub status: SessionStatus,
    pub attention: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionOpts {
    pub kind: SessionKind,
    pub title: Option<String>,
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    #[serde(default)]
    pub worktree_task: Option<String>,
}

pub struct SessionManager {
    sessions: RwLock<HashMap<SessionId, Session>>,
    store: Arc<Store>,
}

impl SessionManager {
    fn preferred_editor_command(&self) -> Option<String> {
        let id = self.store.get_setting(EDITOR_PREF_KEY).ok().flatten()?;
        crate::editor::env_command(&id)
    }

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

        let (worktree, repo_root) = match opts.worktree_task.as_deref() {
            Some(task) => {
                let base = resolve_cwd(opts.cwd.as_deref());
                let root = crate::repo::toplevel(&base).ok_or_else(|| {
                    PtyError::Spawn("a pasta da sessão não é um repositório git".into())
                })?;
                let root = crate::repo::canonicalize_or(&root);
                let wt = crate::worktree::create(&root, task).map_err(PtyError::Spawn)?;
                (Some(wt), Some(root))
            }
            None => (None, None),
        };

        let shell = default_shell();
        let label = shell_label(&shell);
        let integration = self.shell_integration_enabled();
        let mut cmd = CommandBuilder::new(&shell);

        let bash_rc = if cfg!(unix) && label == "bash" && integration {
            bash_integration_file()
        } else {
            None
        };

        if cfg!(unix) {
            if let Some(rc) = bash_rc {
                cmd.arg("--rcfile");
                cmd.arg(rc);
                cmd.arg("-i");
                cmd.env("TYBA_LOGIN_SHELL", "1");
            } else {
                cmd.arg("-l");
            }
        }
        match &worktree {
            Some(wt) => cmd.cwd(&wt.path),
            None => cmd.cwd(resolve_cwd(opts.cwd.as_deref())),
        }

        if label == "zsh" && integration {
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

        let title = opts
            .title
            .or_else(|| opts.worktree_task.clone())
            .unwrap_or_else(|| label.clone());
        let result = self.spawn_session(
            app,
            pty_pool,
            id,
            cmd,
            opts.kind,
            title,
            repo_root,
            worktree.clone(),
            opts.cols,
            opts.rows,
            on_exit,
        );
        if result.is_err() {
            if let Some(wt) = &worktree {
                let _ = crate::worktree::remove(&wt.path, true, true);
            }
        }
        result
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
            None,
            None,
            cols,
            rows,
            on_exit,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_session(
        &self,
        app: AppHandle,
        pty_pool: &SharedPtyPool,
        id: SessionId,
        mut cmd: CommandBuilder,
        kind: SessionKind,
        title: String,
        repo_root: Option<PathBuf>,
        worktree: Option<crate::worktree::Worktree>,
        cols: u16,
        rows: u16,
        on_exit: impl FnOnce(SessionId) + Send + 'static,
    ) -> Result<Session, PtyError> {
        cmd.env("TERM", "xterm-256color");
        cmd.env("TYBA", "1");
        cmd.env("TYBA_SESSION_ID", id.to_string());

        if let Some(command) = self.preferred_editor_command() {
            cmd.env("EDITOR", &command);
            cmd.env("VISUAL", &command);
        }

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
            repo_root,
            worktree,
            status: SessionStatus::Running,
            attention: false,
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
        let status = status.redacted();
        let mut sessions = self.sessions.write();
        if let Some(s) = sessions.get_mut(&id) {
            if s.status == status {
                return;
            }
            s.attention = status.wants_attention() && matches!(s.kind, SessionKind::Agent { .. });
            s.status = status;
            let _ = self.store.upsert_session(s);
            emit_status(app, s);
        }
    }

    pub fn mark_seen(&self, app: &AppHandle, id: SessionId) {
        let mut sessions = self.sessions.write();
        if let Some(s) = sessions.get_mut(&id) {
            if !s.attention {
                return;
            }
            s.attention = false;
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
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    cached_integration_path(&DIR, write_zsh_integration, "zsh")
}

fn cached_integration_path(
    cell: &'static OnceLock<PathBuf>,
    build: fn() -> std::io::Result<PathBuf>,
    shell: &str,
) -> Option<&'static Path> {
    if let Some(path) = cell.get() {
        return Some(path.as_path());
    }
    match build() {
        Ok(path) => {
            let _ = cell.set(path);
            cell.get().map(PathBuf::as_path)
        }
        Err(err) => {
            eprintln!("tyba: shell integration ({shell}) indisponível: {err}");
            None
        }
    }
}

fn integration_dir(name: &str) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("tyba-{name}-{}", current_uid()));
    create_private_dir(&dir)?;
    verify_private_dir(&dir)?;
    Ok(dir)
}

fn integration_denied(reason: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!("diretório de shell integration {reason}"),
    )
}

pub(crate) fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let result = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().mode(0o700).create(dir)
        }
        #[cfg(not(unix))]
        {
            std::fs::create_dir(dir)
        }
    };
    match result {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(unix)]
pub(crate) fn verify_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() {
        return Err(integration_denied("não é um diretório"));
    }
    if meta.uid() != unsafe { libc::getuid() } {
        return Err(integration_denied("pertence a outro usuário"));
    }
    if meta.mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        if std::fs::symlink_metadata(dir)?.permissions().mode() & 0o077 != 0 {
            return Err(integration_denied("acessível a outros usuários"));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn verify_private_dir(dir: &Path) -> std::io::Result<()> {
    if !std::fs::symlink_metadata(dir)?.is_dir() {
        return Err(integration_denied("não é um diretório"));
    }
    Ok(())
}

pub(crate) fn write_private(dir: &Path, name: &str, contents: &str) -> std::io::Result<()> {
    use std::io::Write;

    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    let mut file = opts.open(&tmp)?;
    let written = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(err) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    std::fs::rename(&tmp, dir.join(name))
}

fn zsh_chain(file: &str) -> String {
    format!(
        "__tyba_self_zdotdir=\"$ZDOTDIR\"\n\
         if [[ -f \"$TYBA_USER_ZDOTDIR/{file}\" ]]; then\n  \
         ZDOTDIR=\"$TYBA_USER_ZDOTDIR\"\n  \
         source \"$TYBA_USER_ZDOTDIR/{file}\"\n  \
         ZDOTDIR=\"$__tyba_self_zdotdir\"\nfi\n"
    )
}

fn write_zsh_integration() -> std::io::Result<PathBuf> {
    let dir = integration_dir("zsh")?;

    write_private(&dir, ".zshenv", &zsh_chain(".zshenv"))?;
    write_private(&dir, ".zprofile", &zsh_chain(".zprofile"))?;
    write_private(&dir, ".zlogin", &zsh_chain(".zlogin"))?;

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
    write_private(&dir, ".zshrc", &format!("{}{}", zsh_chain(".zshrc"), hooks))?;

    Ok(dir)
}

/// Arquivo rc de shell integration do bash. Lançado via `bash --rcfile <arquivo>`
/// (sem `-l`, que ignora `--rcfile`). O script reproduz a cadeia de init do
/// usuário e instala os hooks OSC 133/633/7. Verificado em bash 3.2.57 (macOS)
/// e 5.2 (Linux). Ver [[shell-integration]] no cofre.
fn bash_integration_file() -> Option<&'static Path> {
    static FILE: OnceLock<PathBuf> = OnceLock::new();
    cached_integration_path(&FILE, write_bash_integration, "bash")
}

const TYBA_BASH_RC: &str = include_str!("tyba-bash-rc.sh");
const TYBA_BASH_RC_NAME: &str = "tyba-bash-rc.sh";

fn write_bash_integration() -> std::io::Result<PathBuf> {
    let dir = integration_dir("bash")?;
    write_private(&dir, TYBA_BASH_RC_NAME, TYBA_BASH_RC)?;
    Ok(dir.join(TYBA_BASH_RC_NAME))
}

fn current_uid() -> String {
    #[cfg(unix)]
    {
        unsafe { libc::getuid() }.to_string()
    }
    #[cfg(not(unix))]
    {
        "user".to_string()
    }
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
            attention: false,
            created_at: Utc::now(),
        }
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("tyba-test-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    #[cfg(unix)]
    fn verify_private_dir_rejects_symlink() {
        let real = scratch_dir("real");
        let link = scratch_dir("link");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let err = verify_private_dir(&link).expect_err("symlink deve falhar fechado");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);

        std::fs::remove_file(&link).unwrap();
        std::fs::remove_dir_all(&real).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn verify_private_dir_tightens_world_writable_dir() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch_dir("loose");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();

        verify_private_dir(&dir).expect("dir nosso deve ser endurecido, não recusado");
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_private_publishes_atomically_without_leftovers() {
        let dir = scratch_dir("atomic");
        create_private_dir(&dir).unwrap();

        write_private(&dir, "f", "primeiro").unwrap();
        write_private(&dir, "f", "segundo").unwrap();

        assert_eq!(std::fs::read_to_string(dir.join("f")).unwrap(), "segundo");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp file vazou: {leftovers:?}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("f"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn zsh_integration_dir_is_private_and_does_not_interpolate_its_path() {
        use std::os::unix::fs::PermissionsExt;

        let dir = write_zsh_integration().expect("write zsh integration");
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);

        let zshenv = std::fs::read_to_string(dir.join(".zshenv")).unwrap();
        assert!(zshenv.contains("__tyba_self_zdotdir"));
        assert!(!zshenv.contains(dir.to_str().unwrap()));

        let zshrc = std::fs::read_to_string(dir.join(".zshrc")).unwrap();
        assert!(zshrc.contains("133;B"));
        assert!(zshrc.contains("add-zsh-hook chpwd __tyba_osc7"));
        assert!(!zshrc.contains(dir.to_str().unwrap()));
    }

    #[test]
    fn bash_integration_writes_rcfile_with_hooks() {
        let rc = write_bash_integration().expect("write bash rc");
        assert!(rc.is_file());
        let body = std::fs::read_to_string(&rc).unwrap();
        assert!(body.contains("__tyba_preexec"));
        assert!(body.contains("__tyba_precmd"));
        assert!(body.contains("trap '__tyba_preexec' DEBUG"));
        assert!(body.contains("133;C"));
        assert!(body.contains("TYBA_LOGIN_SHELL"));
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
    fn legacy_awaiting_input_json_defaults_to_reply() {
        let status: SessionStatus =
            serde_json::from_str(r#"{"state":"awaiting_input","hint":"npm test"}"#).unwrap();
        assert!(matches!(
            status,
            SessionStatus::AwaitingInput {
                hint: Some(_),
                reason: AwaitingReason::Reply
            }
        ));
    }

    #[test]
    fn awaiting_reason_round_trips() {
        let status = SessionStatus::AwaitingInput {
            hint: Some("git push".into()),
            reason: AwaitingReason::Approval,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains(r#""reason":"approval""#));
        let back: SessionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, status);
    }

    #[test]
    fn redacted_scrubs_hint_and_failure_reason() {
        let hint = SessionStatus::AwaitingInput {
            hint: Some("export KEY=sk-abcdef1234567890ABCDEFghijkl".into()),
            reason: AwaitingReason::Approval,
        }
        .redacted();
        let SessionStatus::AwaitingInput {
            hint: Some(hint), ..
        } = hint
        else {
            panic!("variante preservada");
        };
        assert!(!hint.contains("sk-abcdef"));
        assert!(hint.contains(redact::REDACTION_MARK));

        let failed = SessionStatus::Failed {
            reason: "token AKIAIOSFODNN7EXAMPLE vazou".into(),
        }
        .redacted();
        let SessionStatus::Failed { reason } = failed else {
            panic!("variante preservada");
        };
        assert!(!reason.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn attention_follows_state_semantics() {
        assert!(SessionStatus::Idle.wants_attention());
        assert!(SessionStatus::AwaitingInput {
            hint: None,
            reason: AwaitingReason::Reply
        }
        .wants_attention());
        assert!(SessionStatus::Failed { reason: "x".into() }.wants_attention());
        assert!(!SessionStatus::Running.wants_attention());
        assert!(!SessionStatus::Exited { code: 0 }.wants_attention());
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
