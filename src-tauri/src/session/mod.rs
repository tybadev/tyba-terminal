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
    Idle {
        #[serde(default)]
        summary: Option<String>,
    },
    Exited {
        code: i32,
    },
    Failed {
        reason: String,
    },
}

impl SessionStatus {
    fn redacted(self) -> Self {
        match self {
            SessionStatus::AwaitingInput { hint, reason } => SessionStatus::AwaitingInput {
                hint: hint.map(|h| redact::redact(&h).into_owned()),
                reason,
            },
            SessionStatus::Idle { summary } => SessionStatus::Idle {
                summary: summary.map(|s| redact::redact(&s).into_owned()),
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
            SessionStatus::AwaitingInput { .. }
                | SessionStatus::Idle { .. }
                | SessionStatus::Failed { .. }
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
    /// Pasta onde o processo subiu. É o que permite reabrir a sessão no mesmo
    /// lugar depois de fechar o app — sem ela o PTY morre e o pane fica órfão.
    pub cwd: Option<PathBuf>,
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
    /// Sessão de agente na pasta de `cwd`, que já existe (review, conflitos), em
    /// vez de num worktree novo. O agente continua passando pelo sandbox e pelo
    /// gate de aprovação — é só a origem da pasta que muda.
    #[serde(default)]
    pub attach_existing: bool,
    /// Id do shell escolhido no picker (ver [`available_shells`]). `None` cai no
    /// [`default_shell`]. Só vale para sessão de shell.
    #[serde(default)]
    pub shell: Option<String>,
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

        let integration = self.shell_integration_enabled();
        let (program, pre_args, label): (PathBuf, Vec<String>, String) =
            match opts.shell.as_deref().and_then(resolve_shell) {
                Some(sh) => (sh.program, sh.args, sh.label),
                None => {
                    let program = default_shell();
                    let label = shell_label(&program);
                    (PathBuf::from(program), Vec::new(), label)
                }
            };
        let mut cmd = CommandBuilder::new(&program);
        for arg in &pre_args {
            cmd.arg(arg);
        }

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
        let cwd = match &worktree {
            Some(wt) => wt.path.clone(),
            None => resolve_cwd(opts.cwd.as_deref()),
        };
        cmd.cwd(strip_verbatim_prefix(&cwd));

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
            None,
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
        jail: Option<Box<dyn crate::pty::JailedSpawner>>,
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

        // Lido do próprio comando: pega todo caminho de spawn (shell, agente,
        // tab de container) sem espalhar mais um parâmetro por todos eles.
        // Sem o prefixo verbatim do Windows (`\\?\`) — é o cwd que a UI exibe.
        let cwd = cmd.get_cwd().map(|c| strip_verbatim_prefix(Path::new(c)));

        pty_pool.spawn(
            app.clone(),
            id,
            cmd,
            None,
            jail,
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
            cwd,
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
            let attention = status.wants_attention() && matches!(s.kind, SessionKind::Agent { .. });
            if s.status == status && s.attention == attention {
                return;
            }
            s.attention = attention;
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

    /// Traz de volta as sessões persistidas, todas mortas: o PTY não sobrevive ao
    /// processo. Antes as linhas de shell eram **apagadas** aqui — e com elas o
    /// cwd, que é justamente o que permite reabrir a sessão no mesmo lugar. Quem
    /// decide o que fazer com as mortas é o `resume_startup`, conforme a pref.
    pub fn restore(&self) -> Result<(), StoreError> {
        let persisted = self.store.load_sessions()?;
        let mut sessions = self.sessions.write();
        for mut s in persisted {
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

    /// Sessões mortas herdadas do processo anterior, na ordem de criação.
    pub fn dead_sessions(&self) -> Vec<Session> {
        let sessions = self.sessions.read();
        let mut dead: Vec<Session> = sessions
            .values()
            .filter(|s| {
                matches!(
                    s.status,
                    SessionStatus::Exited { .. } | SessionStatus::Failed { .. }
                )
            })
            .cloned()
            .collect();
        dead.sort_by_key(|s| s.created_at);
        dead
    }

    pub fn forget(&self, id: SessionId) {
        self.sessions.write().remove(&id);
        let _ = self.store.remove_session(id);
    }
}

/// O que fazer com as sessões que não sobreviveram ao fechamento do app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartupMode {
    /// Recria o shell no mesmo cwd e devolve a tab. É o que um terminal faz.
    #[default]
    Resume,
    /// Mantém o layout, sem processo: a tab abre morta até o usuário interagir.
    KeepLayout,
    /// Janela nova, do zero.
    Fresh,
}

pub const STARTUP_PREF_KEY: &str = "pref.startup";

impl StartupMode {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw {
            Some("keep_layout") => StartupMode::KeepLayout,
            Some("fresh") => StartupMode::Fresh,
            _ => StartupMode::Resume,
        }
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
    write_private_bytes(dir, name, contents.as_bytes())
}

pub(crate) fn write_private_bytes(dir: &Path, name: &str, contents: &[u8]) -> std::io::Result<()> {
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
    let written = file.write_all(contents).and_then(|()| file.sync_all());
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
    #[cfg(windows)]
    {
        windows_default_shell()
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
    }
}

/// PowerShell é o default esperado no Windows moderno (o Windows Terminal também
/// abre nele). O código antigo lia `COMSPEC` — que aponta SEMPRE para `cmd.exe` —,
/// então o fallback para PowerShell era código morto e a sessão caía sempre no cmd.
/// Ordem: PowerShell 7 (`pwsh`, se instalado) → Windows PowerShell 5.1 (sempre
/// presente) → `cmd.exe` (último recurso, via COMSPEC).
#[cfg(windows)]
fn windows_default_shell() -> String {
    if let Some(pwsh) = find_on_path("pwsh.exe") {
        return pwsh.to_string_lossy().into_owned();
    }
    if let Some(root) = std::env::var_os("SystemRoot") {
        let ps5 =
            std::path::Path::new(&root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        if ps5.is_file() {
            return ps5.to_string_lossy().into_owned();
        }
    }
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
}

fn shell_label(shell: &str) -> String {
    std::path::Path::new(shell)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| shell.to_string())
}

/// Um shell que o usuário pode escolher ao abrir uma sessão. `id` é estável e
/// reversível por [`resolve_shell`]; `program`/`args` acompanham só para o
/// picker exibir/depurar — quem spawna é sempre o core, a partir do `id`.
#[derive(Debug, Clone, Serialize)]
pub struct ShellOption {
    pub id: String,
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
}

/// Programa + args já resolvidos a partir de um `id` de shell.
pub struct ResolvedShell {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub label: String,
}

/// Shells disponíveis nesta máquina, na ordem que o picker mostra.
#[cfg(windows)]
pub fn available_shells() -> Vec<ShellOption> {
    let mut out = Vec::new();
    if let Some(pwsh) = find_on_path("pwsh.exe") {
        out.push(ShellOption {
            id: "pwsh".into(),
            label: "PowerShell 7".into(),
            program: pwsh.to_string_lossy().into_owned(),
            args: Vec::new(),
        });
    }
    if let Some(ps) = windows_powershell() {
        out.push(ShellOption {
            id: "powershell".into(),
            label: "Windows PowerShell".into(),
            program: ps.to_string_lossy().into_owned(),
            args: Vec::new(),
        });
    }
    let cmd = windows_cmd();
    out.push(ShellOption {
        id: "cmd".into(),
        label: "Prompt de Comando".into(),
        program: cmd.to_string_lossy().into_owned(),
        args: Vec::new(),
    });
    for distro in wsl_distros() {
        out.push(ShellOption {
            id: format!("wsl:{distro}"),
            label: format!("WSL: {distro}"),
            program: "wsl.exe".into(),
            args: vec!["-d".into(), distro],
        });
    }
    out
}

#[cfg(not(windows))]
pub fn available_shells() -> Vec<ShellOption> {
    let program = default_shell();
    let label = shell_label(&program);
    vec![ShellOption {
        id: "default".into(),
        label,
        program,
        args: Vec::new(),
    }]
}

/// Reconstrói o programa a spawnar a partir de um `id` do picker. Devolve `None`
/// se o `id` não corresponde a nada disponível — a sessão então cai no
/// [`default_shell`] (fail-safe, nunca spawna algo arbitrário).
#[cfg(windows)]
pub fn resolve_shell(id: &str) -> Option<ResolvedShell> {
    if let Some(distro) = id.strip_prefix("wsl:") {
        if distro.is_empty() {
            return None;
        }
        return Some(ResolvedShell {
            program: PathBuf::from("wsl.exe"),
            args: vec!["-d".into(), distro.to_string()],
            label: format!("WSL: {distro}"),
        });
    }
    match id {
        "pwsh" => find_on_path("pwsh.exe").map(|program| ResolvedShell {
            program,
            args: Vec::new(),
            label: "PowerShell 7".into(),
        }),
        "powershell" => windows_powershell().map(|program| ResolvedShell {
            program,
            args: Vec::new(),
            label: "Windows PowerShell".into(),
        }),
        "cmd" => Some(ResolvedShell {
            program: windows_cmd(),
            args: Vec::new(),
            label: "Prompt de Comando".into(),
        }),
        _ => None,
    }
}

#[cfg(not(windows))]
pub fn resolve_shell(id: &str) -> Option<ResolvedShell> {
    if id != "default" {
        return None;
    }
    let program = default_shell();
    let label = shell_label(&program);
    Some(ResolvedShell {
        program: PathBuf::from(program),
        args: Vec::new(),
        label,
    })
}

#[cfg(windows)]
fn system_root() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

#[cfg(windows)]
fn windows_powershell() -> Option<PathBuf> {
    let p = system_root().join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    p.is_file().then_some(p)
}

#[cfg(windows)]
fn windows_cmd() -> PathBuf {
    std::env::var_os("COMSPEC")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .unwrap_or_else(|| system_root().join(r"System32\cmd.exe"))
}

#[cfg(windows)]
fn find_on_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe))
        .find(|p| p.is_file())
}

/// Distros WSL instaladas, via `wsl.exe -l -q`. Sem WSL (ou erro), lista vazia.
#[cfg(windows)]
fn wsl_distros() -> Vec<String> {
    let mut cmd = std::process::Command::new("wsl.exe");
    cmd.args(["-l", "-q"]);
    crate::repo::no_console_window(&mut cmd);
    let Ok(out) = cmd.output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    decode_wsl_list(&out.stdout)
}

/// `wsl.exe -l -q` emite UTF-16LE (com BOM, CR e às vezes NUL). Decodifica e
/// limpa em nomes de distro. Parser frágil → tem teste.
#[cfg(windows)]
fn decode_wsl_list(bytes: &[u8]) -> Vec<String> {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .lines()
        .map(|line| line.trim().trim_matches('\u{feff}').trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Remove o prefixo verbatim do Windows (`\\?\`, `\\?\UNC\`) de um caminho para
/// exibição. Caminhos canonicalizados no Windows vêm com esse prefixo: feio na
/// UI e desnecessário fora de chamadas de FS. No-op em qualquer outro caminho.
pub fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = raw.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
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
            cwd: Some(PathBuf::from("/tmp")),
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
    fn legacy_idle_json_defaults_to_no_summary() {
        let status: SessionStatus = serde_json::from_str(r#"{"state":"idle"}"#).unwrap();
        assert_eq!(status, SessionStatus::Idle { summary: None });
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
        assert!(SessionStatus::Idle { summary: None }.wants_attention());
        assert!(SessionStatus::AwaitingInput {
            hint: None,
            reason: AwaitingReason::Reply
        }
        .wants_attention());
        assert!(SessionStatus::Failed { reason: "x".into() }.wants_attention());
        assert!(!SessionStatus::Running.wants_attention());
        assert!(!SessionStatus::Exited { code: 0 }.wants_attention());
    }

    /// Antes o `restore` apagava a linha do shell morto — e com ela o cwd, que é
    /// justamente o que permite reabrir a sessão onde ela estava. Era isso que
    /// deixava o pane órfão e a tab vazia no reopen (#50). A linha agora sobrevive
    /// morta; quem a descarta é o `resume_startup`, conforme a pref.
    #[test]
    fn restore_keeps_the_dead_shell_and_its_cwd_for_the_reopen() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let shell = make(SessionKind::Shell, SessionStatus::Running);
        store.upsert_session(&shell).unwrap();

        let manager = SessionManager::new(Arc::clone(&store));
        manager.restore().unwrap();

        let restored = manager.list();
        assert_eq!(restored.len(), 1);
        assert!(matches!(
            restored[0].status,
            SessionStatus::Exited { .. } | SessionStatus::Failed { .. }
        ));
        assert_eq!(
            restored[0].cwd, shell.cwd,
            "sem o cwd não há como reabrir a sessão no mesmo lugar"
        );
        assert_eq!(manager.dead_sessions().len(), 1);
    }

    #[test]
    fn forget_drops_the_session_from_memory_and_disk() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let shell = make(SessionKind::Shell, SessionStatus::Exited { code: 0 });
        store.upsert_session(&shell).unwrap();
        let manager = SessionManager::new(Arc::clone(&store));
        manager.restore().unwrap();

        manager.forget(shell.id);

        assert!(manager.list().is_empty());
        assert!(store.load_sessions().unwrap().is_empty());
    }

    #[cfg(windows)]
    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    #[test]
    #[cfg(windows)]
    fn decode_wsl_list_reads_utf16le_and_drops_noise() {
        // BOM + duas distros, terminador CRLF como o wsl.exe emite.
        let raw = utf16le("\u{feff}Ubuntu\r\nDebian\r\n");
        assert_eq!(decode_wsl_list(&raw), vec!["Ubuntu", "Debian"]);
        assert!(decode_wsl_list(&utf16le("")).is_empty());
        assert!(decode_wsl_list(&utf16le("   \r\n")).is_empty());
    }

    #[test]
    fn strip_verbatim_prefix_cleans_windows_paths_and_leaves_others() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\C:\Users\a")),
            PathBuf::from(r"C:\Users\a")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\UNC\server\share")),
            PathBuf::from(r"\\server\share")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new("/home/user/proj")),
            PathBuf::from("/home/user/proj")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"C:\Users\a")),
            PathBuf::from(r"C:\Users\a")
        );
    }

    #[test]
    fn startup_mode_defaults_to_resume_and_parses_the_rest() {
        assert_eq!(StartupMode::parse(None), StartupMode::Resume);
        assert_eq!(StartupMode::parse(Some("resume")), StartupMode::Resume);
        assert_eq!(StartupMode::parse(Some("lixo")), StartupMode::Resume);
        assert_eq!(
            StartupMode::parse(Some("keep_layout")),
            StartupMode::KeepLayout
        );
        assert_eq!(StartupMode::parse(Some("fresh")), StartupMode::Fresh);
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
