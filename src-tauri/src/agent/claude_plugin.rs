//! Plugin do TYBA para o Claude Code (modelo Warp: o agente roda livre, o
//! TYBA observa).
//!
//! O claude que o usuário roda num pane pede permissão pelo prompt dele
//! mesmo; o plugin, instalado só por ação explícita do usuário, não nega
//! ferramenta nenhuma. Ele faz duas coisas:
//!
//! - Fora do TYBA (sem `TYBA_PANE_TOKEN`) o hook sai na hora com 0: um `sh`
//!   puro, sem `jq` e sem spawn de binário nenhum.
//! - Num claude já com gate (`TYBA_HOOK_SOCKET` do `--settings` da sessão
//!   gerenciada/hospedada, ou `TYBA_HOSTED=1`) também sai: os eventos já
//!   chegam pelos hooks da sessão, e não podem chegar duas vezes.
//! - Num claude cru dentro de um pane: `SessionStart`, `Notification`,
//!   `PermissionRequest`, `Stop` e `Subagent*` só avisam o core — nenhum
//!   responde nada ao Claude. O `PreToolUse` (só `Bash`) aplica a parte de
//!   texto do guarda-corpo ([`crate::agent::guard_rail`]): merge e aprovação
//!   de PR/MR e desligar o hook do git são recusados com motivo; todo o resto
//!   fica sem decisão, e o prompt nativo do Claude decide. O hook `pre-push`
//!   do git (push para main/master) não alcança este claude: ele vem do env
//!   das sessões do TYBA, e o claude cru escapou delas.
//!
//! O guarda-corpo é texto de comando: pega engano, não agente determinado
//! (script, alias contornam). Fronteira de verdade não mora aqui.
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

/// Os eventos que o plugin só repassa ao core (o `SessionStart` em todas as
/// origens). O `PreToolUse` fica de fora: é local, só o guarda-corpo.
const SESSION_START_MATCHER: &str = "startup|resume|clear|compact|fork";
const OBSERVED_EVENTS: &[&str] = &[
    "SessionStart",
    "Notification",
    "PermissionRequest",
    "Stop",
    "SubagentStart",
    "SubagentStop",
];

use crate::agent::guard_rail::guard_rail_output;

// --- Arquivos do plugin ------------------------------------------------------

pub fn marketplace_json() -> Value {
    json!({
        "name": MARKETPLACE,
        "owner": { "name": "TYBA" },
        "plugins": [{
            "name": PLUGIN,
            "source": "./plugin",
            "description": "Avisa o TYBA do que o claude faz num pane (início, pedido de permissão, fim) e recusa merge e aprovação de PR.",
        }]
    })
}

pub fn plugin_json() -> Value {
    json!({
        "name": PLUGIN,
        "description": "Integração do TYBA: o claude num pane do TYBA avisa o app quando precisa de você, e merge e aprovação de PR são recusados. Fora do TYBA não faz nada.",
        "version": env!("CARGO_PKG_VERSION"),
        "author": { "name": "TYBA" },
    })
}

pub fn hooks_json() -> Value {
    let command = "\"${CLAUDE_PLUGIN_ROOT}/scripts/hook.sh\"";
    let mut hooks = serde_json::Map::new();
    for event in OBSERVED_EVENTS.iter().chain(&["PreToolUse"]) {
        let mut entry = json!({ "hooks": [{ "type": "command", "command": command }] });
        match *event {
            "SessionStart" => entry["matcher"] = json!(SESSION_START_MATCHER),
            // O guarda-corpo só olha `Bash`: as outras ferramentas nem chamam o hook.
            "PreToolUse" => entry["matcher"] = json!("Bash"),
            _ => {}
        }
        hooks.insert((*event).to_string(), json!([entry]));
    }
    json!({ "description": "TYBA: observa o claude num pane e recusa merge e aprovação de PR", "hooks": hooks })
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

pub fn status(home: &Path, config_dir: Option<&str>) -> PluginStatus {
    let env_dir = config_dir
        .map(str::to_string)
        .or_else(|| std::env::var("CLAUDE_CONFIG_DIR").ok());
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

/// Fatia 4 do agente livre: o chip "Instalar plugin" no pane. Só aparece
/// com o plugin ausente e os hooks livres — bloqueado, instalar não adianta
/// nada, e a tela de Integrações já diz por quê.
pub fn should_suggest(installed: bool, blocked: Option<&str>) -> bool {
    !installed && blocked.is_none()
}

/// O mesmo [`status`] sem resolver o binário: quem pergunta já achou um
/// claude rodando no pane. Só lê dois JSON, então cabe no poll do probe.
/// `config_dir` é o `CLAUDE_CONFIG_DIR` do claude do pane — não o do TYBA,
/// que aberto pelo Dock não tem o rc do shell.
pub fn suggest_install(home: &Path, config_dir: Option<&str>) -> bool {
    suggest_in(
        &claude_config_dir(config_dir, home),
        read_json(&managed_settings_path()).as_ref(),
    )
}

fn suggest_in(config: &Path, managed: Option<&Value>) -> bool {
    should_suggest(
        is_installed(read_json(&config.join("plugins/installed_plugins.json")).as_ref()),
        hooks_blocked(read_json(&config.join("settings.json")).as_ref(), managed),
    )
}

/// Roda os comandos do Claude Code em sequência, parando no primeiro que
/// falha. Só por clique do usuário (Configurações → Integrações).
/// `config_dir`: o `CLAUDE_CONFIG_DIR` do claude do pane (chip); `None`
/// herda o do TYBA (Configurações).
pub fn run_commands(commands: &[Vec<OsString>], config_dir: Option<&str>) -> Result<(), String> {
    for argv in commands {
        let Some((program, args)) = argv.split_first() else {
            continue;
        };
        let mut cmd = std::process::Command::new(program);
        cmd.args(args)
            .env("PATH", crate::shell_path::agent_path())
            .stdin(std::process::Stdio::null());
        if let Some(dir) = config_dir.filter(|d| !d.is_empty()) {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        let out = cmd.output().map_err(|e| format!("claude não rodou: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let msg: String = stderr.trim().chars().take(400).collect();
            return Err(crate::session::redact::redact(&msg).into_owned());
        }
    }
    Ok(())
}

pub fn install(marketplace_dir: &Path, config_dir: Option<&str>) -> Result<(), String> {
    let claude = crate::agent::resolved_binary(&crate::session::AgentRunnerKind::ClaudeCode)
        .ok_or("claude não encontrado no PATH")?;
    write_marketplace(marketplace_dir).map_err(|e| format!("arquivos do plugin: {e}"))?;
    run_commands(&install_commands(&claude, marketplace_dir), config_dir)
}

pub fn uninstall() -> Result<(), String> {
    let claude = crate::agent::resolved_binary(&crate::session::AgentRunnerKind::ClaudeCode)
        .ok_or("claude não encontrado no PATH")?;
    run_commands(&uninstall_commands(&claude), None)
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
/// só num claude cru dentro de um pane. Sempre sai com 0. O guarda-corpo do
/// `PreToolUse` é decidido aqui, sem o core (o comando inteiro não atravessa
/// o canal); os eventos observados vão ao core e não respondem nada.
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
    if let Some(out) = guard_rail_output(&payload) {
        println!("{out}");
    }
    let event = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    if OBSERVED_EVENTS.contains(&event) {
        let tool = payload
            .get("tool_name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let summary = tool
            .as_deref()
            .map(|t| tool_summary(t, payload.get("tool_input")));
        let _ = crate::hook_ipc::channel::send_plugin_event(
            token,
            event.to_string(),
            tool,
            summary,
            notification_type(&payload),
        );
    }
    Some(0)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn maybe_run_plugin_hook() -> Option<i32> {
    (std::env::args().nth(1).as_deref() == Some("_plugin-hook")).then_some(0)
}

// --- Core: evento do plugin ---------------------------------------------------

/// Valida o evento: o token tem que ser o de um pane, e o peer tem que
/// descender do shell DESSE pane. Devolve o pane; qualquer falha é `None` —
/// o evento é ignorado.
pub fn resolve_plugin_event(
    token: &str,
    peer_session: Option<crate::session::SessionId>,
    session_for_token: impl Fn(&str) -> Option<crate::session::SessionId>,
) -> Option<crate::session::SessionId> {
    let session = session_for_token(token)?;
    (peer_session == Some(session)).then_some(session)
}

/// O que um evento do plugin faz com o "pedindo" do pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginSignal {
    /// Pedido de permissão: marca e avisa.
    Asking,
    /// O pedido não está mais aberto: o turno fechou (`Stop`) ou a sessão
    /// recomeçou (`SessionStart`). Sem veredito.
    Settled,
    None,
}

/// Só permissão conta como pedido. O `Notification` chega também como
/// `idle_prompt` (~60 s depois de todo turno; decisão do dono de 14/10) e
/// `auth_success`: nenhum dos dois avisa.
pub fn plugin_signal(event: &str, notification_type: Option<&str>) -> PluginSignal {
    match (event, notification_type) {
        ("PermissionRequest", _) => PluginSignal::Asking,
        ("Notification", Some("permission_prompt")) => PluginSignal::Asking,
        ("Stop" | "SessionStart", _) => PluginSignal::Settled,
        _ => PluginSignal::None,
    }
}

/// Casa só o par `PermissionRequest` + `Notification(permission_prompt)` do
/// MESMO prompt, que chegam quase juntos.
pub const ASK_DEBOUNCE: std::time::Duration = std::time::Duration::from_secs(2);

pub fn ask_due(last: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    last.is_none_or(|at| now.saturating_duration_since(at) >= ASK_DEBOUNCE)
}

type AskLog = std::collections::HashMap<crate::session::SessionId, std::time::Instant>;

static LAST_ASK: std::sync::Mutex<Option<AskLog>> = std::sync::Mutex::new(None);

/// Roda `show` se a janela do último aviso deste pane já passou, e só gasta
/// a vaga quando o aviso saiu de fato. O lock fica preso durante o `show`: o
/// par do mesmo prompt chega em duas threads e não pode sair duas vezes.
pub fn with_ask_slot(
    session: crate::session::SessionId,
    now: std::time::Instant,
    show: impl FnOnce() -> bool,
) -> bool {
    let Ok(mut guard) = LAST_ASK.lock() else {
        return show();
    };
    let log = guard.get_or_insert_with(Default::default);
    if !ask_due(log.get(&session).copied(), now) {
        return false;
    }
    let shown = show();
    log.retain(|_, at| now.saturating_duration_since(*at) < ASK_DEBOUNCE);
    if shown {
        log.insert(session, now);
    }
    shown
}

/// A linha deduzida do histórico (`approval_history` id + chave) de um pedido
/// do plugin; `None` quando a gravação falhou.
pub type PluginInferred = Option<(i64, String)>;

type PluginAsks = std::collections::HashMap<crate::session::SessionId, PluginInferred>;

static PLUGIN_ASKING: std::sync::Mutex<Option<PluginAsks>> = std::sync::Mutex::new(None);

/// Panes cujo "pedindo" veio do plugin (claude cru). Só esses a tela pode
/// limpar: o hospedado tem os próprios eventos.
pub fn mark_plugin_ask(session: crate::session::SessionId, inferred: PluginInferred) {
    if let Ok(mut map) = PLUGIN_ASKING.lock() {
        map.get_or_insert_with(Default::default)
            .insert(session, inferred);
    }
}

/// O par do mesmo prompt chegou com a sessão já "pedindo": mantém a linha
/// que o primeiro gravou.
pub fn mark_plugin_ask_kept(session: crate::session::SessionId) {
    if let Ok(mut map) = PLUGIN_ASKING.lock() {
        map.get_or_insert_with(Default::default)
            .entry(session)
            .or_insert(None);
    }
}

/// Tira a marca do pane: `None` quando ele não estava pedindo pelo plugin.
pub fn take_plugin_ask(session: crate::session::SessionId) -> Option<PluginInferred> {
    PLUGIN_ASKING
        .lock()
        .ok()
        .and_then(|mut map| map.as_mut().and_then(|m| m.remove(&session)))
}

/// O `notification_type` do payload, só se tiver cara de identificador.
pub fn notification_type(payload: &Value) -> Option<String> {
    payload
        .get("notification_type")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty() && t.len() <= 40)
        .filter(|t| t.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .map(str::to_string)
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
    fn hooks_json_never_blocks_on_failure_and_covers_the_events() {
        let v = hooks_json();
        let text = v.to_string();
        assert!(
            !text.contains("onFailure"),
            "hook global nunca falha fechado"
        );
        for event in OBSERVED_EVENTS.iter().chain(&["PreToolUse"]) {
            assert!(v["hooks"][event].is_array(), "{event}");
        }
        assert_eq!(
            v["hooks"]["SessionStart"][0]["matcher"],
            "startup|resume|clear|compact|fork"
        );
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        assert_eq!(
            v["hooks"]["Stop"][0]["hooks"][0]["command"],
            "\"${CLAUDE_PLUGIN_ROOT}/scripts/hook.sh\"",
            "caminho do plugin entre aspas"
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

        let err = run_commands(&install_commands(&fake, &mkt), None).unwrap_err();
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
        run_commands(&uninstall_commands(&fake), None).expect("desinstala");
        let lines = std::fs::read_to_string(&log).unwrap();
        assert!(lines.contains("plugin|uninstall|tyba@tyba|"));
        assert!(lines.contains("plugin|marketplace|remove|tyba|"));
    }

    /// Pelo chip, o `claude plugin …` roda com o `CLAUDE_CONFIG_DIR` do
    /// claude do pane.
    #[test]
    fn run_commands_passes_the_pane_config_dir() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("env.log");
        let fake = dir.path().join("claude");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\necho \"${{CLAUDE_CONFIG_DIR:-}}\" >> '{log}'\n",
                log = log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        run_commands(&uninstall_commands(&fake), Some("/tmp/cfg-do-pane")).unwrap();
        let seen = std::fs::read_to_string(&log).unwrap();
        assert!(seen.lines().all(|l| l == "/tmp/cfg-do-pane"), "{seen}");
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
    fn chip_reads_the_config_dir_of_the_pane_claude() {
        let home = tempfile::tempdir().unwrap();
        let work = home.path().join(".claude-work");
        std::fs::create_dir_all(work.join("plugins")).unwrap();
        std::fs::write(
            work.join("plugins/installed_plugins.json"),
            r#"{"plugins":{"tyba@tyba":[]}}"#,
        )
        .unwrap();
        // Instalado no config do pane: sem chip. O ~/.claude padrão (vazio)
        // pediria o chip.
        assert!(!suggest_in(
            &claude_config_dir(work.to_str(), home.path()),
            None
        ));
        assert!(suggest_in(&claude_config_dir(None, home.path()), None));
    }

    #[test]
    fn only_permission_counts_as_asking() {
        assert_eq!(
            plugin_signal("PermissionRequest", None),
            PluginSignal::Asking
        );
        assert_eq!(
            plugin_signal("Notification", Some("permission_prompt")),
            PluginSignal::Asking
        );
        assert_eq!(
            plugin_signal("Notification", Some("idle_prompt")),
            PluginSignal::None
        );
        assert_eq!(
            plugin_signal("Notification", Some("auth_success")),
            PluginSignal::None
        );
        assert_eq!(plugin_signal("Notification", None), PluginSignal::None);
        assert_eq!(plugin_signal("Stop", None), PluginSignal::Settled);
        assert_eq!(plugin_signal("SessionStart", None), PluginSignal::Settled);
        assert_eq!(plugin_signal("SubagentStop", None), PluginSignal::None);
    }

    #[test]
    fn notification_type_is_only_an_identifier() {
        let p = |t: &str| json!({ "notification_type": t });
        assert_eq!(
            notification_type(&p("permission_prompt")).as_deref(),
            Some("permission_prompt")
        );
        assert_eq!(notification_type(&p("rm -rf /")), None);
        assert_eq!(notification_type(&json!({})), None);
    }

    #[test]
    fn ask_slot_pairs_one_prompt_and_spends_only_when_shown() {
        use std::time::{Duration, Instant};
        let at = Instant::now();
        let a = crate::session::SessionId::new_v4();
        let b = crate::session::SessionId::new_v4();
        // Aviso que não saiu (calado, desligado) não gasta a vaga.
        assert!(!with_ask_slot(a, at, || false));
        assert!(with_ask_slot(a, at, || true));
        // O par do mesmo prompt, logo em seguida: um aviso só.
        let mut called = false;
        assert!(!with_ask_slot(a, at + Duration::from_millis(300), || {
            called = true;
            true
        }));
        assert!(!called);
        // Outro pane não espera; o próximo pedido real do mesmo pane também
        // não se perde (aprovar A e sair → B em poucos segundos).
        assert!(with_ask_slot(b, at + Duration::from_millis(300), || true));
        assert!(with_ask_slot(a, at + ASK_DEBOUNCE, || true));
    }

    #[test]
    fn plugin_ask_mark_is_taken_once() {
        let s = crate::session::SessionId::new_v4();
        assert_eq!(take_plugin_ask(s), None);
        mark_plugin_ask(s, Some((7, "plugin:x".into())));
        // O par do mesmo prompt não apaga a linha gravada pelo primeiro.
        mark_plugin_ask_kept(s);
        assert_eq!(take_plugin_ask(s), Some(Some((7, "plugin:x".into()))));
        assert_eq!(take_plugin_ask(s), None);
    }

    #[test]
    fn chip_only_when_missing_and_hooks_run() {
        assert!(should_suggest(false, None));
        assert!(!should_suggest(true, None));
        assert!(!should_suggest(false, Some("disable_all_hooks")));
        assert!(!should_suggest(false, Some("managed_hooks_only")));
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

        assert_eq!(
            resolve_plugin_event("segredo-do-pane", Some(pane), lookup),
            Some(pane)
        );
        assert_eq!(resolve_plugin_event("errado", Some(pane), lookup), None);
        assert_eq!(
            resolve_plugin_event("segredo-do-pane", Some(other), lookup),
            None,
            "token certo vindo de outro pane é recusado"
        );
        assert_eq!(resolve_plugin_event("segredo-do-pane", None, lookup), None);
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
        assert!(out.stdout.is_empty(), "o hook.sh nunca escreve no stdout");
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
