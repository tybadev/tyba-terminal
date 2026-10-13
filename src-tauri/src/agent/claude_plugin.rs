//! Plugin do TYBA para o Claude Code (M6-B do claude-monitor, modelo Warp).
//!
//! O shim e o wrapper (M6-A) pegam quase todo `claude` digitado num pane. O
//! que escapa — caminho absoluto, `npx`, binário com outro nome — roda cru.
//! Este plugin, instalado só por ação explícita do usuário, faz o próprio
//! Claude avisar o TYBA nesses casos:
//!
//! - Fora do TYBA (sem `TYBA_PANE_TOKEN`) o hook sai na hora com 0: um `sh`
//!   puro, sem `jq` e sem spawn de binário nenhum.
//! - Num claude já com gate (`TYBA_HOOK_SOCKET` do `--settings` da sessão
//!   gerenciada/hospedada, ou `TYBA_HOSTED=1`) também sai: os eventos já
//!   chegam pelos hooks da sessão, e não podem chegar duas vezes.
//! - Num claude cru dentro de um pane: o `SessionStart` avisa o core (o pane
//!   mostra "sem gate" em vermelho) e o `PreToolUse`/`PermissionRequest`
//!   negam escrita, Bash e rede, mandando rodar `claude` direto no pane.
//!   Ferramentas só de leitura passam.
//!
//! Canal: o mesmo socket 0600 do shim→core, com peer cred, mais um token por
//! pane (`TYBA_PANE_TOKEN`) que o core confere. Nada aqui usa
//! `onFailure: "block"`: o hook é global, e um app movido de lugar travaria o
//! Claude da máquina inteira.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

pub const MARKETPLACE: &str = "tyba";
pub const PLUGIN: &str = "tyba";
pub const PLUGIN_ID: &str = "tyba@tyba";

const HOOK_SCRIPT: &str = include_str!("claude_plugin_hook.sh");

/// Os eventos que o plugin escuta (o `SessionStart` em todas as origens).
const SESSION_START_MATCHER: &str = "startup|resume|clear|compact|fork";
const EVENTS: &[&str] = &[
    "SessionStart",
    "PreToolUse",
    "PermissionRequest",
    "Notification",
    "Stop",
];

/// Ferramentas que só leem ou só organizam o próprio Claude: passam num
/// claude cru. Todo o resto — escrita, Bash, rede, MCP, qualquer ferramenta
/// nova que ninguém classificou — é negado. Lista positiva de propósito.
const READ_ONLY_TOOLS: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "LS",
    "NotebookRead",
    "TodoRead",
    "TodoWrite",
    "Task",
    "Agent",
    "TaskOutput",
    "BashOutput",
    "ToolSearch",
    "Skill",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
];

pub const RAW_DENY_MESSAGE: &str = "TYBA: este claude está rodando fora do gate (caminho absoluto, npx ou parecido). Escrita, Bash e rede estão bloqueados. Saia e rode `claude` direto no pane do TYBA para ter aprovação e jaula.";

/// O motivo da recusa, ou `None` se a ferramenta é só de leitura.
pub fn raw_tool_decision(tool: &str) -> Option<&'static str> {
    (!READ_ONLY_TOOLS.contains(&tool)).then_some(RAW_DENY_MESSAGE)
}

/// O que o hook imprime no stdout para o Claude. `None` = nada a dizer
/// (deixa o fluxo normal seguir).
pub fn hook_output(event: &str, deny: Option<&str>) -> Option<String> {
    let reason = deny?;
    let value = match event {
        "PreToolUse" => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        }),
        "PermissionRequest" => json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": { "behavior": "deny", "message": reason },
            }
        }),
        _ => return None,
    };
    Some(value.to_string())
}

/// Decisão do evento inteiro: só `PreToolUse`/`PermissionRequest` decidem.
pub fn event_decision(event: &str, tool: Option<&str>) -> Option<&'static str> {
    match event {
        "PreToolUse" | "PermissionRequest" => raw_tool_decision(tool.unwrap_or("")),
        _ => None,
    }
}

// --- Arquivos do plugin ------------------------------------------------------

pub fn marketplace_json() -> Value {
    json!({
        "name": MARKETPLACE,
        "owner": { "name": "TYBA" },
        "plugins": [{
            "name": PLUGIN,
            "source": "./plugin",
            "description": "Avisa o TYBA quando um claude roda fora do gate num pane, e bloqueia escrita, Bash e rede nele.",
        }]
    })
}

pub fn plugin_json() -> Value {
    json!({
        "name": PLUGIN,
        "description": "Integração do TYBA: claude fora do gate num pane do TYBA fica visível e sem escrita, Bash ou rede. Fora do TYBA não faz nada.",
        "version": env!("CARGO_PKG_VERSION"),
        "author": { "name": "TYBA" },
    })
}

pub fn hooks_json() -> Value {
    let command = "${CLAUDE_PLUGIN_ROOT}/scripts/hook.sh";
    let mut hooks = serde_json::Map::new();
    for event in EVENTS {
        let mut entry = json!({ "hooks": [{ "type": "command", "command": command }] });
        if *event == "SessionStart" {
            entry["matcher"] = json!(SESSION_START_MATCHER);
        }
        hooks.insert((*event).to_string(), json!([entry]));
    }
    json!({ "description": "TYBA: gate para claude fora do TYBA num pane", "hooks": hooks })
}

pub fn hook_script() -> &'static str {
    HOOK_SCRIPT
}

/// Escreve o marketplace local em `dir`: `.claude-plugin/marketplace.json` e
/// o plugin em `plugin/`. Reescreve sempre — o conteúdo é do binário atual.
pub fn write_marketplace(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let plugin = dir.join("plugin");
    std::fs::create_dir_all(dir.join(".claude-plugin"))?;
    std::fs::create_dir_all(plugin.join(".claude-plugin"))?;
    std::fs::create_dir_all(plugin.join("hooks"))?;
    std::fs::create_dir_all(plugin.join("scripts"))?;
    let pretty = |v: Value| serde_json::to_string_pretty(&v).unwrap_or_default();
    std::fs::write(
        dir.join(".claude-plugin/marketplace.json"),
        pretty(marketplace_json()),
    )?;
    std::fs::write(
        plugin.join(".claude-plugin/plugin.json"),
        pretty(plugin_json()),
    )?;
    std::fs::write(plugin.join("hooks/hooks.json"), pretty(hooks_json()))?;
    let script = plugin.join("scripts/hook.sh");
    std::fs::write(&script, HOOK_SCRIPT)?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

// --- Instalação ---------------------------------------------------------------

/// Os comandos documentados do Claude Code, como argv (nunca shell): adiciona
/// o marketplace local e instala o plugin, os dois no escopo do usuário.
pub fn install_commands(claude: &Path, marketplace_dir: &Path) -> Vec<Vec<OsString>> {
    vec![
        vec![
            claude.into(),
            "plugin".into(),
            "marketplace".into(),
            "add".into(),
            marketplace_dir.into(),
            "--scope".into(),
            "user".into(),
        ],
        vec![
            claude.into(),
            "plugin".into(),
            "install".into(),
            PLUGIN_ID.into(),
            "--scope".into(),
            "user".into(),
        ],
    ]
}

pub fn uninstall_commands(claude: &Path) -> Vec<Vec<OsString>> {
    vec![
        vec![
            claude.into(),
            "plugin".into(),
            "uninstall".into(),
            PLUGIN_ID.into(),
        ],
        vec![
            claude.into(),
            "plugin".into(),
            "marketplace".into(),
            "remove".into(),
            MARKETPLACE.into(),
        ],
    ]
}

/// Diretório de config do Claude: `CLAUDE_CONFIG_DIR` ou `~/.claude`.
pub fn claude_config_dir(env_config_dir: Option<&str>, home: &Path) -> PathBuf {
    env_config_dir
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"))
}

/// Por que hooks não vão rodar mesmo instalado: `disableAllHooks` em
/// qualquer settings, ou `allowManagedHooksOnly` na config gerenciada
/// (só hooks da empresa rodam). `None` = livre.
pub fn hooks_blocked(user: Option<&Value>, managed: Option<&Value>) -> Option<&'static str> {
    let flag = |v: Option<&Value>, key: &str| {
        v.and_then(|v| v.get(key)).and_then(Value::as_bool) == Some(true)
    };
    if flag(user, "disableAllHooks") || flag(managed, "disableAllHooks") {
        return Some("disable_all_hooks");
    }
    if flag(managed, "allowManagedHooksOnly") {
        return Some("managed_hooks_only");
    }
    None
}

pub fn managed_settings_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.json")
    } else {
        PathBuf::from("/etc/claude-code/managed-settings.json")
    }
}

/// Se o plugin aparece no registro de instalados do Claude.
pub fn is_installed(installed_plugins: Option<&Value>) -> bool {
    installed_plugins
        .and_then(|v| v.get("plugins").or(Some(v)))
        .and_then(Value::as_object)
        .is_some_and(|map| map.contains_key(PLUGIN_ID))
}

pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// O que a tela de Integrações mostra.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginStatus {
    /// O `claude` está instalado (dá para instalar a integração).
    pub available: bool,
    pub installed: bool,
    /// `disable_all_hooks` / `managed_hooks_only`: instalado ou não, os hooks
    /// não rodam — a tela diz que está bloqueado.
    pub blocked: Option<&'static str>,
}

pub fn status(home: &Path) -> PluginStatus {
    let env_dir = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let config = claude_config_dir(env_dir.as_deref(), home);
    PluginStatus {
        available: crate::agent::resolved_binary(&crate::session::AgentRunnerKind::ClaudeCode)
            .is_some(),
        installed: is_installed(read_json(&config.join("plugins/installed_plugins.json")).as_ref()),
        blocked: hooks_blocked(
            read_json(&config.join("settings.json")).as_ref(),
            read_json(&managed_settings_path()).as_ref(),
        ),
    }
}

/// Roda os comandos do Claude Code em sequência, parando no primeiro que
/// falha. Só por clique do usuário (Configurações → Integrações).
pub fn run_commands(commands: &[Vec<OsString>]) -> Result<(), String> {
    for argv in commands {
        let Some((program, args)) = argv.split_first() else {
            continue;
        };
        let out = std::process::Command::new(program)
            .args(args)
            .env("PATH", crate::shell_path::agent_path())
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("claude não rodou: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let msg: String = stderr.trim().chars().take(400).collect();
            return Err(crate::session::redact::redact(&msg).into_owned());
        }
    }
    Ok(())
}

pub fn install(marketplace_dir: &Path) -> Result<(), String> {
    let claude = crate::agent::resolved_binary(&crate::session::AgentRunnerKind::ClaudeCode)
        .ok_or("claude não encontrado no PATH")?;
    write_marketplace(marketplace_dir).map_err(|e| format!("arquivos do plugin: {e}"))?;
    run_commands(&install_commands(&claude, marketplace_dir))
}

pub fn uninstall() -> Result<(), String> {
    let claude = crate::agent::resolved_binary(&crate::session::AgentRunnerKind::ClaudeCode)
        .ok_or("claude não encontrado no PATH")?;
    run_commands(&uninstall_commands(&claude))
}

// --- Cliente: `tyba _plugin-hook` ---------------------------------------------

const MAX_STDIN: u64 = 1024 * 1024;

/// Um resumo curto da chamada para o histórico: o comando do Bash, o
/// arquivo, a URL — já redigido (princípio #10). Nunca o payload inteiro.
pub fn tool_summary(tool: &str, input: Option<&Value>) -> String {
    let field = |k: &str| input.and_then(|i| i.get(k)).and_then(Value::as_str);
    let detail = field("command")
        .or_else(|| field("file_path"))
        .or_else(|| field("url"))
        .or_else(|| field("path"))
        .unwrap_or("");
    let line = if detail.is_empty() {
        tool.to_string()
    } else {
        format!("{tool}: {detail}")
    };
    let short: String = line.chars().take(400).collect();
    crate::session::redact::redact(&short).into_owned()
}

/// Ponto de entrada de `tyba _plugin-hook`, chamado pelo `hook.sh` do plugin
/// só num claude cru dentro de um pane. Sempre sai com 0: a decisão vai no
/// stdout. Sem o core, decide sozinho com a mesma regra.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn maybe_run_plugin_hook() -> Option<i32> {
    use std::io::Read;

    if std::env::args().nth(1).as_deref() != Some("_plugin-hook") {
        return None;
    }
    let gated = std::env::var_os("TYBA_HOOK_SOCKET").is_some()
        || std::env::var("TYBA_HOSTED").as_deref() == Ok("1");
    let Some(token) = std::env::var("TYBA_PANE_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
    else {
        return Some(0);
    };
    if gated {
        return Some(0);
    }
    let mut raw = Vec::new();
    let _ = std::io::stdin().take(MAX_STDIN).read_to_end(&mut raw);
    let payload: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    let event = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let tool = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let summary = tool
        .as_deref()
        .map(|t| tool_summary(t, payload.get("tool_input")));

    let local = event_decision(&event, tool.as_deref());
    let core = crate::hook_ipc::channel::send_plugin_event(token, event.clone(), tool, summary);
    let deny = match core {
        Some(crate::hook_ipc::channel::ChannelResponse::Plugin { deny }) => deny,
        _ => local.map(str::to_string),
    };
    if let Some(out) = hook_output(&event, deny.as_deref()) {
        println!("{out}");
    }
    Some(0)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn maybe_run_plugin_hook() -> Option<i32> {
    (std::env::args().nth(1).as_deref() == Some("_plugin-hook")).then_some(0)
}

// --- Core: evento do plugin ---------------------------------------------------

/// O que o core decidiu sobre um evento do plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginVerdict {
    pub session: crate::session::SessionId,
    pub deny: Option<String>,
    /// `SessionStart`: marcar o pane como "claude sem gate".
    pub mark_raw: bool,
}

/// Valida o evento: o token tem que ser o de um pane, e o peer tem que
/// descender do shell DESSE pane. Qualquer falha é `None` — o evento é
/// ignorado (e o cliente aplica a regra local, que só nega).
pub fn resolve_plugin_event(
    token: &str,
    peer_session: Option<crate::session::SessionId>,
    session_for_token: impl Fn(&str) -> Option<crate::session::SessionId>,
    event: &str,
    tool: Option<&str>,
) -> Option<PluginVerdict> {
    let session = session_for_token(token)?;
    if peer_session != Some(session) {
        return None;
    }
    Some(PluginVerdict {
        session,
        deny: event_decision(event, tool).map(str::to_string),
        mark_raw: event == "SessionStart",
    })
}

/// Comparação em tempo constante para o token — não vaza por tempo quantos
/// caracteres batem.
pub fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionId;

    #[test]
    fn read_only_tools_pass_and_everything_else_is_denied() {
        for tool in ["Read", "Glob", "Grep", "TodoWrite", "Task"] {
            assert_eq!(raw_tool_decision(tool), None, "{tool}");
        }
        for tool in [
            "Bash",
            "Write",
            "Edit",
            "MultiEdit",
            "NotebookEdit",
            "WebFetch",
            "WebSearch",
            "mcp__github__create_issue",
            "FerramentaNova",
            "",
        ] {
            assert_eq!(raw_tool_decision(tool), Some(RAW_DENY_MESSAGE), "{tool}");
        }
    }

    #[test]
    fn hook_output_speaks_the_claude_hook_protocol() {
        let pre: Value =
            serde_json::from_str(&hook_output("PreToolUse", Some("não")).unwrap()).unwrap();
        assert_eq!(pre["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(pre["hookSpecificOutput"]["permissionDecisionReason"], "não");
        let perm: Value =
            serde_json::from_str(&hook_output("PermissionRequest", Some("não")).unwrap()).unwrap();
        assert_eq!(perm["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(hook_output("PreToolUse", None), None);
        assert_eq!(hook_output("Stop", Some("x")), None);
    }

    #[test]
    fn hooks_json_never_blocks_on_failure_and_covers_the_events() {
        let v = hooks_json();
        let text = v.to_string();
        assert!(
            !text.contains("onFailure"),
            "hook global nunca falha fechado"
        );
        for event in EVENTS {
            assert!(v["hooks"][event].is_array(), "{event}");
        }
        assert_eq!(
            v["hooks"]["SessionStart"][0]["matcher"],
            "startup|resume|clear|compact|fork"
        );
    }

    #[test]
    fn install_commands_are_argv_with_the_user_scope() {
        let cmds = install_commands(Path::new("/x/claude"), Path::new("/data/tyba mkt"));
        let as_str: Vec<Vec<String>> = cmds
            .iter()
            .map(|c| c.iter().map(|a| a.to_string_lossy().into_owned()).collect())
            .collect();
        assert_eq!(
            as_str[0],
            [
                "/x/claude",
                "plugin",
                "marketplace",
                "add",
                "/data/tyba mkt",
                "--scope",
                "user"
            ]
        );
        assert_eq!(
            as_str[1],
            [
                "/x/claude",
                "plugin",
                "install",
                "tyba@tyba",
                "--scope",
                "user"
            ]
        );
        let un: Vec<Vec<String>> = uninstall_commands(Path::new("/x/claude"))
            .iter()
            .map(|c| c.iter().map(|a| a.to_string_lossy().into_owned()).collect())
            .collect();
        assert_eq!(un[0], ["/x/claude", "plugin", "uninstall", "tyba@tyba"]);
        assert_eq!(
            un[1],
            ["/x/claude", "plugin", "marketplace", "remove", "tyba"]
        );
    }

    /// O instalador roda os comandos como argv, em ordem, e para no primeiro
    /// que falha — com um `claude` falso num HOME temporário, nunca o de
    /// verdade desta máquina.
    #[test]
    fn run_commands_runs_in_order_and_stops_at_the_first_failure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("argv.log");
        let fake = dir.path().join("claude");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\nprintf '%s|' \"$@\" >> '{log}'\necho >> '{log}'\n[ \"$2\" = install ] && {{ echo 'falhou token sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789' >&2; exit 3; }}\nexit 0\n",
                log = log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mkt = dir.path().join("mkt dir");

        let err = run_commands(&install_commands(&fake, &mkt)).unwrap_err();
        let lines: Vec<String> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(
            lines[0],
            format!("plugin|marketplace|add|{}|--scope|user|", mkt.display())
        );
        assert_eq!(lines[1], "plugin|install|tyba@tyba|--scope|user|");
        assert!(err.starts_with("falhou token"), "{err}");
        assert!(
            !err.contains("sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789"),
            "o stderr do claude passa pela redação: {err}"
        );

        std::fs::remove_file(&log).unwrap();
        run_commands(&uninstall_commands(&fake)).expect("desinstala");
        let lines = std::fs::read_to_string(&log).unwrap();
        assert!(lines.contains("plugin|uninstall|tyba@tyba|"));
        assert!(lines.contains("plugin|marketplace|remove|tyba|"));
    }

    #[test]
    fn hooks_blocked_detects_both_settings() {
        assert_eq!(hooks_blocked(None, None), None);
        let off = json!({"disableAllHooks": true});
        assert_eq!(hooks_blocked(Some(&off), None), Some("disable_all_hooks"));
        assert_eq!(hooks_blocked(None, Some(&off)), Some("disable_all_hooks"));
        let managed = json!({"allowManagedHooksOnly": true});
        assert_eq!(
            hooks_blocked(None, Some(&managed)),
            Some("managed_hooks_only")
        );
        // `allowManagedHooksOnly` só vale na config gerenciada.
        assert_eq!(hooks_blocked(Some(&managed), None), None);
    }

    #[test]
    fn is_installed_reads_the_registry() {
        assert!(is_installed(Some(
            &json!({"version": 2, "plugins": {"tyba@tyba": []}})
        )));
        assert!(!is_installed(Some(
            &json!({"version": 2, "plugins": {"warp@x": []}})
        )));
        assert!(!is_installed(None));
    }

    #[test]
    fn config_dir_honors_claude_config_dir() {
        let home = Path::new("/Users/x");
        assert_eq!(claude_config_dir(None, home), home.join(".claude"));
        assert_eq!(claude_config_dir(Some(""), home), home.join(".claude"));
        assert_eq!(
            claude_config_dir(Some("/tmp/cfg"), home),
            PathBuf::from("/tmp/cfg")
        );
    }

    #[test]
    fn tool_summary_is_short_and_redacted() {
        let input = json!({"command": "curl -H 'Authorization: Bearer sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789' x"});
        let s = tool_summary("Bash", Some(&input));
        assert!(s.starts_with("Bash: curl"));
        assert!(
            !s.contains("sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789"),
            "{s}"
        );
        let long = json!({"command": "x".repeat(5000)});
        assert!(tool_summary("Bash", Some(&long)).chars().count() <= 400);
    }

    #[test]
    fn token_must_match_and_the_peer_must_be_in_that_pane() {
        let pane = SessionId::new_v4();
        let other = SessionId::new_v4();
        let lookup = |t: &str| token_eq(t, "segredo-do-pane").then_some(pane);

        let ok = resolve_plugin_event(
            "segredo-do-pane",
            Some(pane),
            lookup,
            "PreToolUse",
            Some("Bash"),
        )
        .expect("aceita");
        assert_eq!(ok.deny.as_deref(), Some(RAW_DENY_MESSAGE));
        assert!(!ok.mark_raw);

        assert_eq!(
            resolve_plugin_event("errado", Some(pane), lookup, "SessionStart", None),
            None
        );
        assert_eq!(
            resolve_plugin_event("segredo-do-pane", Some(other), lookup, "SessionStart", None),
            None,
            "token certo vindo de outro pane é recusado"
        );
        assert_eq!(
            resolve_plugin_event("segredo-do-pane", None, lookup, "SessionStart", None),
            None
        );
        let start =
            resolve_plugin_event("segredo-do-pane", Some(pane), lookup, "SessionStart", None)
                .unwrap();
        assert!(start.mark_raw);
        assert_eq!(start.deny, None);
    }

    #[test]
    fn token_eq_is_exact() {
        assert!(token_eq("abc", "abc"));
        assert!(!token_eq("abc", "abd"));
        assert!(!token_eq("abc", "abcd"));
        assert!(!token_eq("", "a"));
    }

    #[test]
    fn write_marketplace_lays_out_the_documented_structure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        write_marketplace(dir.path()).unwrap();
        let mkt = read_json(&dir.path().join(".claude-plugin/marketplace.json")).unwrap();
        assert_eq!(mkt["plugins"][0]["source"], "./plugin");
        assert!(dir
            .path()
            .join("plugin/.claude-plugin/plugin.json")
            .is_file());
        assert!(dir.path().join("plugin/hooks/hooks.json").is_file());
        let script = dir.path().join("plugin/scripts/hook.sh");
        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    /// Roda o `hook.sh` de verdade num `sh`, com um `tyba` falso que registra
    /// se foi chamado. Os três casos de env do M6-B.
    fn run_hook(envs: &[(&str, &str)]) -> (i32, bool) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("hook.sh");
        std::fs::write(&script, HOOK_SCRIPT).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let log = dir.path().join("called");
        let fake = dir.path().join("tyba");
        std::fs::write(
            &fake,
            format!("#!/bin/sh\necho \"$@\" > '{}'\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cmd = std::process::Command::new(&script);
        cmd.env_clear().env("PATH", "/usr/bin:/bin");
        for (k, v) in envs {
            let v = if *v == "$FAKE" {
                fake.to_str().unwrap()
            } else {
                v
            };
            cmd.env(k, v);
        }
        let out = cmd
            .stdin(std::process::Stdio::null())
            .output()
            .expect("hook roda");
        (out.status.code().unwrap_or(-1), log.exists())
    }

    #[test]
    fn hook_script_is_a_silent_no_op_outside_tyba() {
        assert_eq!(run_hook(&[("TYBA_BIN", "$FAKE")]), (0, false));
    }

    #[test]
    fn hook_script_is_a_no_op_in_an_already_gated_claude() {
        let base = [("TYBA_BIN", "$FAKE"), ("TYBA_PANE_TOKEN", "t")];
        let mut hosted = base.to_vec();
        hosted.push(("TYBA_HOSTED", "1"));
        assert_eq!(run_hook(&hosted), (0, false));
        let mut managed = base.to_vec();
        managed.push(("TYBA_HOOK_SOCKET", "/tmp/x.sock"));
        assert_eq!(run_hook(&managed), (0, false));
    }

    #[test]
    fn hook_script_hands_a_raw_claude_in_a_pane_to_tyba() {
        assert_eq!(
            run_hook(&[("TYBA_BIN", "$FAKE"), ("TYBA_PANE_TOKEN", "t")]),
            (0, true)
        );
        // Sem o binário do TYBA (app movido), sai 0 e não trava o Claude.
        assert_eq!(
            run_hook(&[("TYBA_BIN", "/nao/existe"), ("TYBA_PANE_TOKEN", "t")]),
            (0, false)
        );
    }
}
