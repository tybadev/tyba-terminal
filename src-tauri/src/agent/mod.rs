pub mod auth_alert;
pub mod auth_preflight;
pub mod auth_watch;
pub mod browser_bridge;
pub mod codex_hooks;
pub mod conversation;
pub mod credentials;
pub mod disk_observer;
pub mod hooks_settings;
pub mod notify;
pub mod process_probe;
pub mod session;
pub mod subagents;
pub mod suggest;

#[cfg(test)]
mod codex_e2e_tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use portable_pty::CommandBuilder;

use crate::sandbox::policy::{AgentAccess, Rule, RuleSet};
use crate::session::{AgentRunnerKind, SessionKind};

pub struct HookSetup {
    pub settings_path: PathBuf,
    pub hook_command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmitStrategy {
    pub use_bracketed_paste: bool,
    pub delay: Duration,
    pub submit_bytes: &'static [u8],
}

impl Default for SubmitStrategy {
    fn default() -> Self {
        Self {
            use_bracketed_paste: true,
            delay: Duration::from_millis(50),
            submit_bytes: b"\r",
        }
    }
}

pub fn submit_strategy_for(kind: &SessionKind) -> SubmitStrategy {
    match kind {
        SessionKind::Shell | SessionKind::Ssh { .. } | SessionKind::Container { .. } => {
            SubmitStrategy::default()
        }
        SessionKind::Agent { runner } => match runner {
            AgentRunnerKind::ClaudeCode => ClaudeCodeRunner.submit_strategy(),
            AgentRunnerKind::Codex => CodexRunner.submit_strategy(),
            AgentRunnerKind::Custom(_) => SubmitStrategy::default(),
        },
    }
}

pub trait AgentRunner: Send + Sync {
    fn kind(&self) -> AgentRunnerKind;

    /// `resume` é o id nativo da conversa a retomar (ver
    /// [`crate::agent::conversation`]). Quem monta o argv é o runner, e não quem
    /// chama, porque a posição do resume é diferente em cada CLI: no Claude Code
    /// é a opção `--resume <id>`, no Codex é o subcomando `resume <id>`, que tem
    /// de vir antes de qualquer flag.
    fn build_command(
        &self,
        worktree_path: &Path,
        env: &HashMap<String, String>,
        hooks: &HookSetup,
        resume: Option<&str>,
    ) -> CommandBuilder;

    /// Se a CLI deste runner sabe retomar uma conversa por id. `false` não é
    /// falha: é o convite de retomar não aparecer.
    fn resumes_conversations(&self) -> bool {
        false
    }

    fn submit_strategy(&self) -> SubmitStrategy {
        SubmitStrategy::default()
    }

    fn supports_hooks(&self) -> bool {
        false
    }

    fn needs_network(&self) -> bool {
        false
    }

    fn self_sandboxes(&self) -> bool {
        false
    }

    fn sandbox_access(&self, _home: &Path, _worktree: &Path) -> AgentAccess {
        AgentAccess::default()
    }
}

pub fn claude_project_dir_name(worktree: &Path) -> String {
    let resolved = std::fs::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    resolved
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

pub fn runner_binary(kind: &AgentRunnerKind) -> Option<&'static str> {
    match kind {
        AgentRunnerKind::ClaudeCode => Some("claude"),
        AgentRunnerKind::Codex => Some("codex"),
        AgentRunnerKind::Custom(_) => None,
    }
}

pub fn binary_available(kind: &AgentRunnerKind) -> bool {
    let Some(name) = runner_binary(kind) else {
        return false;
    };
    std::env::split_paths(&crate::shell_path::agent_path())
        .any(|dir| is_executable(&dir.join(name)))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Diretórios de `~/.claude` que dão ao agente enjaulado poder de executar
/// código ou de mudar a decisão de permissão — hooks, MCP, prompts, comandos
/// (§2.1/§2.2 da entrega B). Ficam somente-LEITURA: `Rule::Node` no except de
/// escrita vira `--ro-bind` do diretório real (bwrap.rs), preservando leitura.
/// `projects/` NÃO entra aqui — é isolamento total (furo F4), tratado à parte
/// como `Rule::Subpath` porque o problema não é execução, é visibilidade
/// cruzada entre repos.
const SENSITIVE_CLAUDE_READONLY_DIRS: [&str; 10] = [
    "plugins",
    "cowork_plugins",
    "hooks",
    "agents",
    "commands",
    "skills",
    "output-styles",
    "rules",
    "workflows",
    "daemon",
];

/// Arquivos sombreados mesmo que ainda não existam — a pré-criação em
/// bwrap.rs os torna sempre presentes (M4: `--ro-bind` de path ausente aborta
/// o bwrap). `daemon.json` é o item mais grave (cron/slash commands rodados
/// pelo daemon no HOST, fora da jaula, com watcher que reconcilia — §2.3/M6).
///
/// `.config.json` NÃO está nesta lista, de propósito (v0.6.2, reabertura da
/// entrega B): é o caminho LEGADO do `.claude.json` — mas, ao contrário do
/// `.claude.json` (que é config só de projeto/MCP), é onde o binário do
/// Claude GRAVA `oauthAccount` no login, e `q()` no binário (2.1.252) prefere
/// `.config.json` sobre `.claude.json` quando os dois existem
/// (legacyPath > configPath). O review r1/r2 anterior promoveu `.config.json`
/// a MANDATORY pelo mesmo motivo do `remote-settings.json` (fechar o vetor
/// `mcpServers`) — mas isso quebrou dois jeitos: (a) `ensure_inert_file`
/// pré-criava um `.config.json` `{}` no HOST quando ausente, e esse `{}`
/// passava a vencer o `.claude.json` real do dono via `q()` — inclusive FORA
/// do TYBA; (b) `--ro-bind` dentro da jaula impedia o login de persistir
/// `oauthAccount` no resume. Tirado da lista: fica dentro do `~/.claude` que
/// já é `--bind` rw (write allow), então o login persiste, e `.config.json`
/// nunca mais nasce sozinho no host. O vetor `mcpServers` continua coberto do
/// MESMO jeito que já cobre `.claude.json` hoje (que também nunca foi
/// sombreado): a aprovação de MCP do próprio Claude + o alarme de deriva
/// (`credentials.rs`) — consistente entre os dois arquivos com o mesmo
/// schema, em vez do read-only que só um dos dois tinha.
///
/// `remote-settings.json` CONTINUA aqui (review de segurança r2, v0.6.2,
/// BLOCKING): o round anterior baixou pra IF_PRESENT pelo raciocínio errado
/// — tratou os dois arquivos como equivalentes só porque os dois foram
/// promovidos juntos no round 1. Mas `remote-settings.json` NÃO é o config
/// global (`q()` só olha `.config.json`/`.claude.json`), então sombreá-lo
/// sempre NUNCA quebra login — e ele carrega hooks/permissions, exatamente
/// o vetor de exec que `daemon.json`/`mcp.json`/`settings.json` fecham. Com
/// IF_PRESENT, no caso comum (ausente no spawn) ele ficava gravável E
/// `is_classified_claude_child` (`credentials.rs`) o marcava "já
/// classificado" — o alarme de deriva NUNCA disparava pra ele. Um agente
/// enjaulado plantando hooks/permissions ali seria silencioso: exec no host
/// na próxima vez que o dono rodasse `claude` fora do TYBA. O `{}` vazio
/// pré-criado é o mesmo trade-off de poluição já aceito pros outros cinco
/// arquivos desta lista (R6) — benigno, porque `remote-settings.json` não
/// tem papel de login. Só `.config.json` devia ter saído de MANDATORY.
const SENSITIVE_CLAUDE_FILES_MANDATORY: [&str; 6] = [
    "settings.json",
    "settings.local.json",
    "daemon.json",
    "mcp.json",
    "CLAUDE.md",
    "remote-settings.json",
];

/// Arquivos sombreados só se já existirem — sem pré-criação, porque não têm
/// papel de segurança forte o bastante para justificar nascer no disco do
/// dono por conta do TYBA (§2.3, §3.4: "ou não é emitido").
const SENSITIVE_CLAUDE_FILES_IF_PRESENT: [&str; 2] = ["keybindings.json", "loop.md"];

/// Extensões que classificam um arquivo do topo de `~/.claude` como "cara de
/// script" mesmo sem nome conhecido (V9: `statusline-command.sh` do dono não
/// está em nenhuma lista fixa — só na configuração dele).
const SCRIPT_EXTENSIONS: [&str; 12] = [
    "sh", "bash", "zsh", "py", "js", "mjs", "cjs", "ts", "rb", "pl", "php", "lua",
];

/// §2.4 — explicitamente NÃO sombreados, de propósito: é estado, fica
/// gravável. Vocabulário de V7 (o que existe hoje em produção) mais o que o
/// design lista à parte com razão escrita (backups/ por causa de M9, jobs/
/// por causa de M6). Usado só pelo alarme de deriva (`credentials.rs`) para
/// distinguir "estado conhecido" de "nome novo que ninguém classificou" — não
/// tem papel na política de sandbox em si (essa é a denylist acima).
///
/// `cfg(linux)`: o único consumidor é `credentials::is_classified_claude_child`
/// (o alarme de deriva, C1/C2/deriva — Linux, §6). As outras quatro listas de
/// nomes deste módulo (`SENSITIVE_CLAUDE_READONLY_DIRS` e afins) continuam
/// sem cfg porque `sensitive_claude_children` — quem elas alimentam — roda
/// nos dois SOs (§3.1: a política de write é compartilhada); só esta é
/// exclusiva do alarme, e por isso fica dead_code fora do Linux sem o gate
/// (achado do clippy no macOS/Windows da CI do PR #299).
#[cfg(target_os = "linux")]
pub(crate) const CLAUDE_STATE_TOP_LEVEL_NAMES: [&str; 43] = [
    // Review de segurança r2 (v0.6.2, NIT): `.config.json` não está em
    // NENHUMA lista de sombreamento (v0.6.2, corrige a regressão de login —
    // ver `SENSITIVE_CLAUDE_FILES_MANDATORY`), então SEM esta entrada
    // `is_classified_claude_child` o via como "não classificado" e o alarme
    // de deriva emitia a linha de "estado benigno" (stderr) a cada spawn,
    // pra TODO usuário logado (que sempre tem `.config.json`). Entra aqui
    // como estado CONHECIDO — mesmo papel de `.credentials.json` logo
    // abaixo — continua gravável, só para de ser "desconhecido".
    ".config.json",
    ".credentials.json",
    "history.jsonl",
    "file-history",
    "dump-prompts",
    "todos",
    "shell-snapshots",
    "session-env",
    "statsig",
    "debug",
    "ide",
    "logs",
    "sessions",
    "backups",
    "cache",
    "chrome",
    "downloads",
    "paste-cache",
    "state",
    "tasks",
    "plans",
    "jobs",
    "telemetry",
    "traces",
    "usage-data",
    "startup-perf",
    "themes",
    "uploads",
    "shares",
    "feedback",
    "feedback-bundles",
    "agent-memory",
    "local",
    "teams",
    "ccr",
    "seed-admin",
    "stats-cache.json",
    "daemon.status.json",
    "daemon.scheduled.status.json",
    ".last-cleanup",
    ".last-update-result.json",
    "mcp-discovery-cache",
    "mcp-needs-auth-cache.json",
];

/// Os filhos de `~/.claude` que ficam sombreados no write (§2 da entrega B).
/// Tudo que NÃO está aqui é estado e fica gravável por default — é o ônus que
/// o alarme de deriva paga (§2.5, `unclassified_claude_children`): o nome novo
/// nasce gravável e denunciado, nunca gravável e silencioso.
pub(crate) fn sensitive_claude_children(claude: &Path) -> Vec<Rule> {
    let mut rules: Vec<Rule> = vec![Rule::Subpath(claude.join("projects"))];
    for dir in SENSITIVE_CLAUDE_READONLY_DIRS {
        rules.push(Rule::Node(claude.join(dir)));
    }
    for file in SENSITIVE_CLAUDE_FILES_MANDATORY {
        rules.push(Rule::Literal(claude.join(file)));
    }

    let mut named_files: std::collections::HashSet<&str> =
        SENSITIVE_CLAUDE_FILES_MANDATORY.into_iter().collect();
    // Review r1 (v0.6.2), MINOR: `.config.json` não está em NENHUMA das duas
    // listas nomeadas de propósito (é o store de login, nunca sombreado —
    // ver o comentário de `SENSITIVE_CLAUDE_FILES_MANDATORY`), o que o
    // deixaria cair na varredura por FORMA logo abaixo (V9) se o dono tiver
    // esse arquivo com bit +x (patológico, mas real) — reabrindo o mesmo bug
    // de login por um caminho diferente. Entra em `named_files` só pra ser
    // PULADO pela varredura por forma, não por ser mandatório ou opcional.
    named_files.insert(".config.json");
    for file in SENSITIVE_CLAUDE_FILES_IF_PRESENT {
        named_files.insert(file);
        let p = claude.join(file);
        if p.is_file() {
            rules.push(Rule::Literal(p));
        }
    }

    if let Ok(entries) = std::fs::read_dir(claude) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if named_files.contains(name.as_ref()) {
                continue;
            }
            let script_ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|ext| SCRIPT_EXTENSIONS.contains(&ext))
                .unwrap_or(false);
            if script_ext || is_executable(&path) {
                rules.push(Rule::Literal(path));
            }
        }
    }
    rules
}

pub struct ClaudeCodeRunner;

impl AgentRunner for ClaudeCodeRunner {
    fn kind(&self) -> AgentRunnerKind {
        AgentRunnerKind::ClaudeCode
    }

    fn build_command(
        &self,
        worktree_path: &Path,
        env: &HashMap<String, String>,
        hooks: &HookSetup,
        resume: Option<&str>,
    ) -> CommandBuilder {
        let mut cmd = CommandBuilder::new("claude");
        cmd.arg("--settings");
        cmd.arg(&hooks.settings_path);
        // `claude --help`: `-r, --resume [value]` retoma a conversa pelo id da
        // sessão. Id desconhecido não sobe agente nenhum — a CLI imprime
        // "No conversation found with session ID" e sai.
        if let Some(id) = resume {
            cmd.arg("--resume");
            cmd.arg(id);
        }
        cmd.cwd(worktree_path);
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd
    }

    fn resumes_conversations(&self) -> bool {
        true
    }

    fn supports_hooks(&self) -> bool {
        true
    }

    fn needs_network(&self) -> bool {
        true
    }

    fn sandbox_access(&self, home: &Path, worktree: &Path) -> AgentAccess {
        let claude = home.join(".claude");
        let project = claude
            .join("projects")
            .join(claude_project_dir_name(worktree));
        AgentAccess {
            read: vec![
                RuleSet {
                    allow: vec![Rule::Subpath(claude.clone())],
                    except: vec![
                        Rule::Subpath(claude.join("projects")),
                        Rule::Subpath(claude.join("file-history")),
                        Rule::Literal(claude.join("history.jsonl")),
                    ],
                },
                RuleSet::allow(vec![
                    Rule::Node(project.clone()),
                    Rule::Family(home.join(".claude.json")),
                ]),
            ],
            write: vec![
                RuleSet {
                    allow: vec![
                        Rule::Node(claude.clone()),
                        Rule::Family(home.join(".claude.json")),
                    ],
                    except: sensitive_claude_children(&claude),
                },
                RuleSet::allow(vec![Rule::Node(project)]),
            ],
        }
    }
}

pub struct CodexRunner;

impl AgentRunner for CodexRunner {
    fn kind(&self) -> AgentRunnerKind {
        AgentRunnerKind::Codex
    }

    fn submit_strategy(&self) -> SubmitStrategy {
        SubmitStrategy {
            delay: Duration::ZERO,
            ..SubmitStrategy::default()
        }
    }

    fn build_command(
        &self,
        worktree_path: &Path,
        env: &HashMap<String, String>,
        hooks: &HookSetup,
        resume: Option<&str>,
    ) -> CommandBuilder {
        let mut cmd = CommandBuilder::new("codex");
        // `codex resume <SESSION_ID>` é SUBCOMANDO, não opção: precisa vir antes
        // de `--sandbox` e dos `--config`, senão o clap não o reconhece. O
        // sandbox nativo e o `-a on-request` continuam valendo — `codex resume
        // --help` aceita os dois — e é isso que mantém o gate de aprovação de pé
        // numa conversa retomada.
        if let Some(id) = resume {
            cmd.arg("resume");
            cmd.arg(id);
        }
        cmd.arg("--sandbox");
        cmd.arg("workspace-write");
        cmd.arg("--ask-for-approval");
        cmd.arg("on-request");
        for over in codex_hooks::codex_config_overrides(&hooks.hook_command) {
            cmd.arg("--config");
            cmd.arg(over);
        }
        cmd.cwd(worktree_path);
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd
    }

    fn resumes_conversations(&self) -> bool {
        true
    }

    fn supports_hooks(&self) -> bool {
        true
    }

    fn needs_network(&self) -> bool {
        true
    }

    fn self_sandboxes(&self) -> bool {
        true
    }

    fn sandbox_access(&self, home: &Path, _worktree: &Path) -> AgentAccess {
        let codex = home.join(".codex");
        AgentAccess {
            read: vec![RuleSet::allow(vec![Rule::Subpath(codex.clone())])],
            write: vec![RuleSet::allow(vec![
                Rule::Node(codex.join("sessions")),
                Rule::Node(codex.join("archived_sessions")),
                Rule::Node(codex.join("log")),
                Rule::Node(codex.join("tmp")),
                Rule::Literal(codex.join("history.jsonl")),
                Rule::Family(codex.join("auth.json")),
            ])],
        }
    }
}

pub fn resolved_binary(kind: &AgentRunnerKind) -> Option<PathBuf> {
    let name = runner_binary(kind)?;
    std::env::split_paths(&crate::shell_path::agent_path())
        .map(|dir| dir.join(name))
        .find(|p| is_executable(p))
        .and_then(|p| std::fs::canonicalize(p).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn argv_strings(cmd: &CommandBuilder) -> Vec<String> {
        cmd.get_argv()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn hooks() -> HookSetup {
        HookSetup {
            settings_path: PathBuf::from("/tmp/hooks.json"),
            hook_command: "'/usr/bin/tyba' _hook".to_string(),
        }
    }

    #[test]
    fn runner_binary_maps_kinds() {
        assert_eq!(runner_binary(&AgentRunnerKind::ClaudeCode), Some("claude"));
        assert_eq!(runner_binary(&AgentRunnerKind::Codex), Some("codex"));
        assert_eq!(runner_binary(&AgentRunnerKind::Custom("x".into())), None);
    }

    #[test]
    fn custom_runner_binary_is_never_available() {
        assert!(!binary_available(&AgentRunnerKind::Custom("x".into())));
    }

    #[test]
    fn claude_kind_is_claude_code() {
        assert!(matches!(
            ClaudeCodeRunner.kind(),
            AgentRunnerKind::ClaudeCode
        ));
    }

    #[test]
    fn claude_supports_hooks() {
        assert!(ClaudeCodeRunner.supports_hooks());
    }

    #[test]
    fn claude_needs_network() {
        assert!(ClaudeCodeRunner.needs_network());
    }

    #[test]
    fn claude_submit_strategy_waits_50ms_before_carriage_return() {
        let strategy = ClaudeCodeRunner.submit_strategy();
        assert!(strategy.use_bracketed_paste);
        assert_eq!(strategy.delay, Duration::from_millis(50));
        assert_eq!(strategy.submit_bytes, b"\r");
    }

    #[test]
    fn codex_submit_strategy_sends_carriage_return_without_delay() {
        let strategy = CodexRunner.submit_strategy();
        assert!(strategy.use_bracketed_paste);
        assert_eq!(strategy.delay, Duration::ZERO);
        assert_eq!(strategy.submit_bytes, b"\r");
    }

    #[test]
    fn submit_strategy_for_maps_session_kinds() {
        assert_eq!(
            submit_strategy_for(&SessionKind::Shell),
            SubmitStrategy::default()
        );
        assert_eq!(
            submit_strategy_for(&SessionKind::Agent {
                runner: AgentRunnerKind::ClaudeCode
            }),
            ClaudeCodeRunner.submit_strategy()
        );
        assert_eq!(
            submit_strategy_for(&SessionKind::Agent {
                runner: AgentRunnerKind::Codex
            }),
            CodexRunner.submit_strategy()
        );
        assert_eq!(
            submit_strategy_for(&SessionKind::Agent {
                runner: AgentRunnerKind::Custom("aider".into())
            }),
            SubmitStrategy::default()
        );
    }

    #[test]
    fn build_command_uses_settings_flag_with_hook_path() {
        let env = HashMap::new();
        let cmd = ClaudeCodeRunner.build_command(Path::new("/wt"), &env, &hooks(), None);
        let argv = argv_strings(&cmd);
        assert_eq!(argv, vec!["claude", "--settings", "/tmp/hooks.json"]);
    }

    /// Sintaxe conferida contra o binário desta máquina (`claude --help`):
    /// `-r, --resume [value]` retoma pelo id da sessão.
    #[test]
    fn claude_resume_passes_the_conversation_id_as_an_option() {
        let env = HashMap::new();
        let cmd = ClaudeCodeRunner.build_command(Path::new("/wt"), &env, &hooks(), Some("abc-123"));
        let argv = argv_strings(&cmd);
        assert_eq!(
            argv,
            vec![
                "claude",
                "--settings",
                "/tmp/hooks.json",
                "--resume",
                "abc-123"
            ]
        );
    }

    /// `codex resume <SESSION_ID>` é subcomando: fora da primeira posição o clap
    /// não o reconhece e a conversa não volta.
    #[test]
    fn codex_resume_is_the_first_argument_and_keeps_sandbox_and_approval() {
        let env = HashMap::new();
        let cmd = CodexRunner.build_command(Path::new("/wt"), &env, &hooks(), Some("abc-123"));
        let argv = argv_strings(&cmd);
        assert_eq!(&argv[..3], ["codex", "resume", "abc-123"]);
        let sandbox = argv.iter().position(|a| a == "--sandbox").unwrap();
        assert_eq!(argv[sandbox + 1], "workspace-write");
        let approval = argv.iter().position(|a| a == "--ask-for-approval").unwrap();
        assert_eq!(argv[approval + 1], "on-request");
        assert!(argv.iter().filter(|a| *a == "--config").count() == 5);
    }

    #[test]
    fn both_shipped_runners_resume_conversations() {
        assert!(ClaudeCodeRunner.resumes_conversations());
        assert!(CodexRunner.resumes_conversations());
    }

    #[test]
    fn build_command_sets_cwd_to_worktree() {
        let env = HashMap::new();
        let cmd = ClaudeCodeRunner.build_command(Path::new("/wt"), &env, &hooks(), None);
        assert_eq!(cmd.get_cwd().map(OsStr::new), Some(OsStr::new("/wt")));
    }

    #[test]
    fn build_command_applies_env_allowlist() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        let cmd = ClaudeCodeRunner.build_command(Path::new("/wt"), &env, &hooks(), None);
        assert_eq!(cmd.get_env("PATH"), Some(OsStr::new("/usr/bin")));
    }

    #[test]
    fn build_command_never_bypasses_permissions() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        let cmd = ClaudeCodeRunner.build_command(Path::new("/wt"), &env, &hooks(), None);
        let argv = argv_strings(&cmd);
        for arg in &argv {
            assert_ne!(arg, "--dangerously-skip-permissions");
            assert_ne!(arg, "--permission-mode");
            assert_ne!(arg, "-p");
        }
    }

    #[test]
    fn codex_kind_supports_hooks_and_network() {
        assert!(matches!(CodexRunner.kind(), AgentRunnerKind::Codex));
        assert!(CodexRunner.supports_hooks());
        assert!(CodexRunner.needs_network());
    }

    #[test]
    fn codex_command_keeps_sandbox_on_and_tui_silent() {
        let env = HashMap::new();
        let cmd = CodexRunner.build_command(Path::new("/wt"), &env, &hooks(), None);
        let argv = argv_strings(&cmd);
        assert_eq!(argv[0], "codex");
        let sandbox = argv.iter().position(|a| a == "--sandbox").unwrap();
        assert_eq!(argv[sandbox + 1], "workspace-write");
        let approval = argv.iter().position(|a| a == "--ask-for-approval").unwrap();
        assert_eq!(argv[approval + 1], "on-request");
        assert_eq!(cmd.get_cwd().map(OsStr::new), Some(OsStr::new("/wt")));
    }

    #[test]
    fn codex_command_injects_hooks_and_trust_via_config_overrides() {
        let env = HashMap::new();
        let cmd = CodexRunner.build_command(Path::new("/wt"), &env, &hooks(), None);
        let argv = argv_strings(&cmd);
        let overrides: Vec<&String> = argv
            .iter()
            .enumerate()
            .filter(|(i, _)| *i > 0 && argv[i - 1] == "--config")
            .map(|(_, a)| a)
            .collect();
        assert_eq!(overrides.len(), 5);
        for key in [
            "hooks.PreToolUse=",
            "hooks.PermissionRequest=",
            "hooks.SessionStart=",
            "hooks.Stop=",
            "hooks.state=",
        ] {
            assert!(
                overrides.iter().any(|o| o.starts_with(key)),
                "faltou override {key}"
            );
        }
        assert!(overrides
            .iter()
            .all(|o| !o.starts_with("hooks.state=") || o.contains("trusted_hash")));
    }

    #[test]
    fn claude_project_dir_name_replaces_non_alphanumerics() {
        assert_eq!(
            claude_project_dir_name(Path::new("/Users/g/.tyba/worktrees/repo/task-1")),
            "-Users-g--tyba-worktrees-repo-task-1"
        );
    }

    #[test]
    fn claude_sandbox_access_scopes_projects_to_current_worktree() {
        let access =
            ClaudeCodeRunner.sandbox_access(Path::new("/Users/x"), Path::new("/private/wt/a"));
        let broad = &access.read[0];
        assert!(broad
            .except
            .contains(&Rule::Subpath(PathBuf::from("/Users/x/.claude/projects"))));
        assert!(access.read[1].allow.contains(&Rule::Node(PathBuf::from(
            "/Users/x/.claude/projects/-private-wt-a"
        ))));
    }

    fn rule_matches(rule: &Rule, candidate: &Path) -> bool {
        let p = rule.path();
        match rule {
            Rule::Literal(_) => candidate == p,
            Rule::Subpath(_) | Rule::Node(_) => candidate.starts_with(p),
            Rule::Prefix(_) => candidate
                .to_string_lossy()
                .starts_with(&*p.to_string_lossy()),
            Rule::Family(_) => {
                let cs = candidate.to_string_lossy();
                let ps = p.to_string_lossy();
                cs == ps
                    || cs
                        .strip_prefix(&*ps)
                        .and_then(|rest| rest.chars().next())
                        .is_some_and(|c| matches!(c, '-' | '.' | '~'))
            }
        }
    }

    fn write_grants(access: &AgentAccess, candidate: &Path) -> bool {
        access.write.iter().any(|set| {
            set.allow.iter().any(|r| rule_matches(r, candidate))
                && !set.except.iter().any(|r| rule_matches(r, candidate))
        })
    }

    /// Entrega B (§2/§3.1): o write vira denylist — `~/.claude` inteiro é
    /// gravável por default, e só o que dá poder de execução ou muda a decisão
    /// de permissão fica sombreado. Substitui
    /// `claude_sandbox_access_never_writes_settings_or_plugins`: com a
    /// allowlist antiga, `unknown-dir` e `session-env-evil` NÃO eram
    /// graváveis por não estarem listados — esse era exatamente o bug (V8:
    /// backups/cache/chrome/... falhavam EROFS em silêncio por não estar na
    /// lista). Sob a nova política, estado não-classificado é gravável por
    /// default (o ônus é o alarme de deriva, testado à parte), então essas
    /// duas asserções mudaram de sentido — a promessa desta entrega é a razão.
    #[test]
    fn claude_write_access_shadows_config_and_hook_surfaces_but_keeps_state_writable() {
        let access =
            ClaudeCodeRunner.sandbox_access(Path::new("/Users/x"), Path::new("/private/wt/a"));
        let claude = Path::new("/Users/x/.claude");

        for forbidden in [
            "settings.json",
            "settings.local.json",
            "daemon.json",
            "daemon",
            "daemon/schedule.json",
            "plugins",
            "plugins/x/hook.sh",
            "cowork_plugins",
            "cowork_plugins/x/hook.sh",
            "hooks",
            "hooks/pre-commit.sh",
            "mcp.json",
            "agents",
            "agents/reviewer.md",
            "commands",
            "commands/deploy.md",
            "skills",
            "skills/x/SKILL.md",
            "output-styles",
            "output-styles/terse.md",
            "rules",
            "rules/security.md",
            "workflows",
            "workflows/ci.yaml",
            "CLAUDE.md",
            // Review de segurança r2 (v0.6.2, BLOCKING): remote-settings.json
            // voltou pra MANDATORY — carrega hooks/permissions, e IF_PRESENT
            // deixava o caso comum (ausente no spawn) gravável E classificado
            // (o alarme de deriva nunca disparava pra ele). Nunca gravável,
            // nem quando ainda não existia no spawn — ver o comentário de
            // `SENSITIVE_CLAUDE_FILES_MANDATORY`.
            "remote-settings.json",
        ] {
            assert!(
                !write_grants(&access, &claude.join(forbidden)),
                "config/hook nunca gravável, mesmo pré-existente: {forbidden}"
            );
        }

        for still_writable in [
            "session-env",
            "session-env/8f3-uuid",
            "backups",
            "backups/.claude.json.backup.123",
            "jobs",
            "jobs/abc",
            "cache",
            "cache/x",
            "paste-cache",
            "sessions",
            "chrome",
            "downloads",
            "todos",
            "shell-snapshots",
            "statsig",
            "debug",
            "ide",
            "logs",
            ".credentials.json",
            "unknown-dir",
            "unknown-dir/deep/file",
            // v0.6.2: `.config.json` saiu de MANDATORY (causa raiz do bug de
            // login — ver o comentário de `SENSITIVE_CLAUDE_FILES_MANDATORY`)
            // e nunca é sombreado, presente ou não.
            ".config.json",
        ] {
            assert!(
                write_grants(&access, &claude.join(still_writable)),
                "estado do Claude Code precisa continuar gravável (senão volta o EROFS \
                 silencioso de hoje, V8): {still_writable}"
            );
        }

        assert!(
            !write_grants(&access, &claude.join("projects").join("-other-repo")),
            "projects/<outro> não pode ser gravável (furo F4)"
        );
        assert!(
            write_grants(&access, &claude.join("projects").join("-private-wt-a")),
            "projects/<este> precisa continuar gravável, via re-grant"
        );
    }

    #[test]
    fn sensitive_claude_children_lists_the_classified_dirs_and_mandatory_files() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        let rules = sensitive_claude_children(&claude);

        assert!(rules.contains(&Rule::Subpath(claude.join("projects"))));
        for dir in [
            "plugins",
            "cowork_plugins",
            "hooks",
            "agents",
            "commands",
            "skills",
            "output-styles",
            "rules",
            "workflows",
            "daemon",
        ] {
            assert!(
                rules.contains(&Rule::Node(claude.join(dir))),
                "{dir} precisa estar classificado como Node (somente-leitura)"
            );
        }
        for file in [
            "settings.json",
            "settings.local.json",
            "daemon.json",
            "mcp.json",
            "CLAUDE.md",
            // Review de segurança r2 (v0.6.2, BLOCKING): de volta a
            // MANDATORY — ver o comentário de
            // `SENSITIVE_CLAUDE_FILES_MANDATORY`.
            "remote-settings.json",
        ] {
            assert!(
                rules.contains(&Rule::Literal(claude.join(file))),
                "{file} precisa estar classificado mesmo sem existir ainda (pré-criação, M4)"
            );
        }
    }

    /// v0.6.2, item 1/5 do contrato de cobertura da Track A: `.config.json` é
    /// o store de login do Claude (`oauthAccount`) — não pode ser
    /// pré-criado nem sombreado, nem quando já existe no disco (é exatamente
    /// o caso do dono depois do fork: `.config.json` já presente, precisa
    /// continuar gravável para o resume funcionar). Contraste com
    /// `sensitive_claude_children_only_lists_optional_files_when_they_already_exist`,
    /// onde "já existir" LIGA a sombra — para `.config.json` isso nunca
    /// acontece, em nenhum dos dois estados.
    #[test]
    fn sensitive_claude_children_never_shadows_config_json_present_or_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();

        let absent = sensitive_claude_children(&claude);
        assert!(
            !absent.contains(&Rule::Literal(claude.join(".config.json"))),
            ".config.json ausente não pode virar Rule::Literal (isso pré-criaria {{}} no host)"
        );

        std::fs::write(claude.join(".config.json"), r#"{"oauthAccount":{}}"#).unwrap();
        let present = sensitive_claude_children(&claude);
        assert!(
            !present.contains(&Rule::Literal(claude.join(".config.json"))),
            ".config.json já existente não pode ser sombreado — o login precisa persistir nele"
        );
    }

    /// Review r1 (v0.6.2), MINOR: `.config.json` não está em nenhuma das
    /// duas listas nomeadas (MANDATORY/IF_PRESENT), então cairia na
    /// varredura genérica por FORMA (V9) igual a `statusline-command.sh` do
    /// dono -- se o dono tiver `.config.json` com bit +x (patológico, mas
    /// real), a varredura o classificaria como "cara de script" e voltaria
    /// a sombreá-lo, reabrindo o bug de login que a remoção de
    /// `SENSITIVE_CLAUDE_FILES_MANDATORY` corrigiu.
    #[test]
    #[cfg(unix)]
    fn sensitive_claude_children_never_shadows_config_json_even_with_exec_bit() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(claude.join(".config.json"), r#"{"oauthAccount":{}}"#).unwrap();
        std::fs::set_permissions(
            claude.join(".config.json"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        let rules = sensitive_claude_children(&claude);
        assert!(
            !rules.contains(&Rule::Literal(claude.join(".config.json"))),
            ".config.json com +x não pode virar --ro-bind pela varredura por forma: {rules:?}"
        );
    }

    #[test]
    fn sensitive_claude_children_only_lists_optional_files_when_they_already_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        let absent = sensitive_claude_children(&claude);
        for file in ["keybindings.json", "loop.md"] {
            assert!(
                !absent.contains(&Rule::Literal(claude.join(file))),
                "{file} não é mandatório: sem pré-criação, então só entra se já existir"
            );
        }

        std::fs::write(claude.join("keybindings.json"), "{}").unwrap();
        let present = sensitive_claude_children(&claude);
        assert!(present.contains(&Rule::Literal(claude.join("keybindings.json"))));
    }

    /// V9: o script do statusline do dono não tem nome fixo — a classificação
    /// tem que achá-lo por forma (extensão de script ou bit de execução), não
    /// por lista de nomes.
    #[test]
    fn sensitive_claude_children_detects_script_looking_files_at_the_top_by_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(claude.join("statusline-command.sh"), "#!/bin/sh\n").unwrap();
        std::fs::write(claude.join("notes.txt"), "não é script").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let exec_no_ext = claude.join("run-me");
            std::fs::write(&exec_no_ext, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&exec_no_ext, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let rules = sensitive_claude_children(&claude);
        assert!(rules.contains(&Rule::Literal(claude.join("statusline-command.sh"))));
        assert!(!rules.contains(&Rule::Literal(claude.join("notes.txt"))));
        #[cfg(unix)]
        assert!(rules.contains(&Rule::Literal(claude.join("run-me"))));
    }

    /// §5.5/R7: se algum dia o TYBA passar a subir o claude em modo attach ou
    /// background, o shim BROWSER é contornado em silêncio (M8 — o Claude
    /// injeta BROWSER:"true" nesse modo, tratado como não-setado). O único
    /// lugar que monta o argv é `build_command`; esta guarda pina que ele
    /// nunca emite flag de attach/background.
    #[test]
    fn build_command_never_enables_attach_or_background_mode() {
        let env = HashMap::new();
        let cmd = ClaudeCodeRunner.build_command(Path::new("/wt"), &env, &hooks(), None);
        let argv = argv_strings(&cmd);
        for flag in ["--attach", "--background", "-a", "--daemon"] {
            assert!(
                !argv.contains(&flag.to_string()),
                "argv não pode conter {flag}: {argv:?}"
            );
        }
    }

    #[test]
    fn codex_sandbox_access_never_writes_config_or_hooks() {
        let access = CodexRunner.sandbox_access(Path::new("/Users/x"), Path::new("/private/wt/a"));
        for set in &access.write {
            for rule in &set.allow {
                let s = rule.path().to_string_lossy();
                assert!(!s.ends_with("config.toml"), "{s}");
                assert!(!s.ends_with("hooks.json"), "{s}");
                assert!(!s.ends_with(".codex"), "{s}");
            }
        }
    }

    #[test]
    fn codex_self_sandboxes_claude_does_not() {
        assert!(
            CodexRunner.self_sandboxes(),
            "o Codex aplica o Seatbelt nativo dele por comando; envolvê-lo no Seatbelt do TYBA \
             faz o sandbox_apply aninhado falhar e quebra toda execução de tool"
        );
        assert!(!ClaudeCodeRunner.self_sandboxes());
    }

    #[test]
    fn codex_sandbox_access_reads_own_home_only() {
        let access = CodexRunner.sandbox_access(Path::new("/Users/x"), Path::new("/private/wt/a"));
        assert_eq!(
            access.read[0].allow,
            vec![Rule::Subpath(PathBuf::from("/Users/x/.codex"))]
        );
    }

    #[test]
    fn codex_command_never_bypasses_sandbox_or_hook_trust() {
        let env = HashMap::new();
        let cmd = CodexRunner.build_command(Path::new("/wt"), &env, &hooks(), None);
        for arg in argv_strings(&cmd) {
            assert_ne!(arg, "--dangerously-bypass-approvals-and-sandbox");
            assert_ne!(arg, "--dangerously-bypass-hook-trust");
            assert_ne!(arg, "danger-full-access");
        }
    }
}
