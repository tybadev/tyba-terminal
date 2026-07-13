use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use sha2::Digest;

pub const AGENT_ENV_BASELINE: [&str; 6] = ["PATH", "HOME", "USER", "LANG", "TMPDIR", "SHELL"];

pub const CONFIG_REL: &str = ".tyba/config.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoConfig {
    pub default_agent: Option<String>,
    pub env_allow: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RepoConfigError {
    #[error("falha ao ler .tyba/config.toml: {0}")]
    Io(#[from] std::io::Error),
    #[error("config inválido: {0}")]
    Parse(String),
    #[error("nome de variável de ambiente inválido: {name}")]
    InvalidEnvName { name: String },
}

#[derive(Deserialize)]
struct RawConfig {
    #[serde(default)]
    agent: RawAgent,
}

#[derive(Deserialize, Default)]
struct RawAgent {
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    env: RawEnv,
}

#[derive(Deserialize, Default)]
struct RawEnv {
    #[serde(default)]
    allow: Vec<String>,
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

pub fn parse(content: &str) -> Result<RepoConfig, RepoConfigError> {
    let raw: RawConfig =
        toml::from_str(content).map_err(|e| RepoConfigError::Parse(e.to_string()))?;
    for name in &raw.agent.env.allow {
        if !valid_env_name(name) {
            return Err(RepoConfigError::InvalidEnvName { name: name.clone() });
        }
    }
    Ok(RepoConfig {
        default_agent: raw.agent.default,
        env_allow: raw.agent.env.allow,
    })
}

pub fn config_hash(content: &str) -> String {
    format!("{:x}", sha2::Sha256::digest(content.as_bytes()))
}

pub fn load(repo_root: &Path) -> Result<Option<(RepoConfig, String)>, RepoConfigError> {
    let path = repo_root.join(CONFIG_REL);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RepoConfigError::Io(e)),
    };
    let config = parse(&content)?;
    let hash = config_hash(&content);
    Ok(Some((config, hash)))
}

pub fn agent_env(
    config: Option<&RepoConfig>,
    user_env: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    for key in AGENT_ENV_BASELINE {
        if let Some(value) = user_env.get(key) {
            env.insert(key.to_string(), value.clone());
        }
    }
    if let Some(config) = config {
        for key in &config.env_allow {
            if let Some(value) = user_env.get(key) {
                env.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }
    // Lançado pelo Dock, o processo herda o PATH do launchd, onde nenhum
    // binário de agente existe. O PATH do shell de login é o que o usuário
    // realmente tem (ver shell_path).
    env.insert("PATH".to_string(), crate::shell_path::agent_path());
    if let Some(dir) = developer_dir(user_env) {
        env.insert("DEVELOPER_DIR".to_string(), dir);
    }
    env
}

/// No macOS `/usr/bin/git` é um **shim**: ele procura o git de verdade dentro do
/// `DEVELOPER_DIR` e, **se não achar lá**, chama `xcodebuild` pra perguntar onde
/// está. Dentro do sandbox essa segunda etapa morre, e o agente fica sem git.
///
/// Não basta apontar pro dir do `xcode-select`: em Xcode recente o git **não vem**
/// no toolchain (`Contents/Developer/usr/bin/git` não existe), e aí o shim cai no
/// `xcodebuild` de novo. Escolhemos o primeiro dir que realmente **contém** o git
/// — normalmente as Command Line Tools — resolvendo tudo fora da jaula.
fn developer_dir(user_env: &HashMap<String, String>) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let mut candidates: Vec<String> = Vec::new();
    if let Some(dir) = user_env.get("DEVELOPER_DIR") {
        candidates.push(dir.clone());
    }
    if let Ok(out) = std::process::Command::new("/usr/bin/xcode-select")
        .arg("-p")
        .output()
    {
        if out.status.success() {
            let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !dir.is_empty() {
                candidates.push(dir);
            }
        }
    }
    candidates.push("/Library/Developer/CommandLineTools".to_string());

    candidates
        .into_iter()
        .find(|dir| std::path::Path::new(dir).join("usr/bin/git").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn parses_full_valid_toml() {
        let content = "[agent]\ndefault = \"claude\"\n\n[agent.env]\nallow = [\"DATABASE_URL\", \"BUN_INSTALL\"]\n";
        let config = parse(content).unwrap();
        assert_eq!(config.default_agent.as_deref(), Some("claude"));
        assert_eq!(config.env_allow, vec!["DATABASE_URL", "BUN_INSTALL"]);
    }

    #[test]
    fn parses_only_env_section() {
        let content = "[agent.env]\nallow = [\"FOO\"]\n";
        let config = parse(content).unwrap();
        assert_eq!(config.default_agent, None);
        assert_eq!(config.env_allow, vec!["FOO"]);
    }

    #[test]
    fn parses_empty_content_as_defaults() {
        let config = parse("").unwrap();
        assert_eq!(config.default_agent, None);
        assert!(config.env_allow.is_empty());
    }

    #[test]
    fn rejects_invalid_env_names() {
        for bad in ["FOO-BAR", "1FOO", "", "FOO BAR", "FÜÜ"] {
            let content = format!("[agent.env]\nallow = [\"{bad}\"]\n");
            match parse(&content) {
                Err(RepoConfigError::InvalidEnvName { name }) => assert_eq!(name, bad),
                other => panic!("esperado InvalidEnvName para {bad:?}, veio {other:?}"),
            }
        }
    }

    #[test]
    fn accepts_valid_env_names() {
        let content = "[agent.env]\nallow = [\"_FOO\", \"F1\", \"a_b_2\"]\n";
        assert!(parse(content).is_ok());
    }

    #[test]
    fn rejects_wrong_type_for_allow() {
        let content = "[agent.env]\nallow = \"DATABASE_URL\"\n";
        assert!(matches!(parse(content), Err(RepoConfigError::Parse(_))));
    }

    #[test]
    fn rejects_non_string_item_in_allow() {
        let content = "[agent.env]\nallow = [1, 2]\n";
        assert!(matches!(parse(content), Err(RepoConfigError::Parse(_))));
    }

    #[test]
    fn rejects_malformed_toml() {
        assert!(matches!(parse("[agent"), Err(RepoConfigError::Parse(_))));
    }

    #[test]
    fn tolerates_unknown_keys() {
        let content = "[agent]\ndefault = \"claude\"\nfuture = 42\n\n[future_section]\nx = true\n";
        let config = parse(content).unwrap();
        assert_eq!(config.default_agent.as_deref(), Some("claude"));
    }

    /// O bug que só a CI pegou: apontar o DEVELOPER_DIR pra um dir SEM git faz o
    /// shim do /usr/bin/git cair no xcodebuild — que morre dentro da jaula. Um dir
    /// que não tem `usr/bin/git` nunca pode ser escolhido.
    #[cfg(target_os = "macos")]
    #[test]
    fn developer_dir_never_points_at_a_toolchain_without_git() {
        let empty = tempfile::tempdir().unwrap();
        let user = env(&[("DEVELOPER_DIR", empty.path().to_str().unwrap())]);

        let chosen = developer_dir(&user);
        assert_ne!(
            chosen.as_deref(),
            Some(empty.path().to_str().unwrap()),
            "dir sem usr/bin/git manda o shim pro xcodebuild, que a jaula bloqueia"
        );
        if let Some(dir) = chosen {
            assert!(
                std::path::Path::new(&dir).join("usr/bin/git").is_file(),
                "o dir escolhido precisa conter o git de verdade: {dir}"
            );
        }
    }

    #[test]
    fn agent_env_baseline_always_present() {
        let user = env(&[("PATH", "/bin"), ("HOME", "/home/x"), ("IGNORED", "y")]);
        let out = agent_env(None, &user);
        assert!(out.contains_key("PATH"));
        assert_eq!(out.get("HOME").map(String::as_str), Some("/home/x"));
        assert!(!out.contains_key("IGNORED"));
    }

    /// Lançado pelo Dock, o processo herda o PATH do launchd — onde nenhum
    /// binário de agente existe. O env do agente tem que carregar o PATH do
    /// shell do usuário, não o do processo.
    #[test]
    fn agent_env_usa_o_path_do_shell_e_nao_o_do_launchd() {
        let launchd = "/usr/bin:/bin:/usr/sbin:/sbin";
        let user = env(&[("PATH", launchd)]);
        let out = agent_env(None, &user);
        assert_eq!(
            out.get("PATH").map(String::as_str),
            Some(crate::shell_path::agent_path().as_str())
        );
    }

    #[test]
    fn agent_env_none_config_only_baseline() {
        let user = env(&[("PATH", "/bin"), ("DATABASE_URL", "postgres://")]);
        let out = agent_env(None, &user);
        assert!(out.contains_key("PATH"));
        // O que importa é que NADA do env do usuário vaze fora da baseline —
        // contar chaves era um atalho que quebra sempre que a baseline cresce.
        assert!(!out.contains_key("DATABASE_URL"));
        for key in out.keys() {
            assert!(
                AGENT_ENV_BASELINE.contains(&key.as_str()) || key == "DEVELOPER_DIR",
                "chave fora da baseline vazou: {key}"
            );
        }
    }

    #[test]
    fn agent_env_allowlist_adds_on_top() {
        let user = env(&[("PATH", "/bin"), ("DATABASE_URL", "postgres://")]);
        let config = RepoConfig {
            default_agent: None,
            env_allow: vec!["DATABASE_URL".into()],
        };
        let out = agent_env(Some(&config), &user);
        assert_eq!(
            out.get("DATABASE_URL").map(String::as_str),
            Some("postgres://")
        );
        assert!(out.contains_key("PATH"));
    }

    /// Um repo hostil que liste `PATH` no `env_allow` não pode sequestrar o
    /// PATH do agente — ele vem sempre do shell do usuário.
    #[test]
    fn agent_env_allowlist_nunca_sequestra_o_path() {
        let user = env(&[("PATH", "/repo/injetado")]);
        let config = RepoConfig {
            default_agent: None,
            env_allow: vec!["PATH".into()],
        };
        let out = agent_env(Some(&config), &user);
        assert_eq!(
            out.get("PATH").map(String::as_str),
            Some(crate::shell_path::agent_path().as_str()),
            "o PATH do core sempre vence o do repo — senão o repo escolhe o binário do agente"
        );
        assert!(!out.values().any(|v| v == "/repo/injetado"));
    }

    #[test]
    fn agent_env_ignores_allow_var_absent_from_user_env() {
        let user = env(&[("PATH", "/bin")]);
        let config = RepoConfig {
            default_agent: None,
            env_allow: vec!["MISSING".into()],
        };
        let out = agent_env(Some(&config), &user);
        assert!(!out.contains_key("MISSING"));
    }

    #[test]
    fn config_hash_is_stable() {
        let content = "[agent]\ndefault = \"claude\"\n";
        assert_eq!(config_hash(content), config_hash(content));
        assert_eq!(config_hash(content).len(), 64);
    }

    #[test]
    fn config_hash_sensitive_to_one_byte() {
        let a = config_hash("[agent]\ndefault = \"claude\"\n");
        let b = config_hash("[agent]\ndefault = \"claudf\"\n");
        assert_ne!(a, b);
    }

    #[test]
    fn load_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), None);
    }

    #[test]
    fn load_reads_config_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".tyba")).unwrap();
        let content = "[agent]\ndefault = \"codex\"\n\n[agent.env]\nallow = [\"BUN_INSTALL\"]\n";
        std::fs::write(dir.path().join(CONFIG_REL), content).unwrap();
        let (config, hash) = load(dir.path()).unwrap().unwrap();
        assert_eq!(config.default_agent.as_deref(), Some("codex"));
        assert_eq!(config.env_allow, vec!["BUN_INSTALL"]);
        assert_eq!(hash, config_hash(content));
    }

    #[test]
    fn load_invalid_config_is_err() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".tyba")).unwrap();
        std::fs::write(
            dir.path().join(CONFIG_REL),
            "[agent.env]\nallow = [\"BAD-NAME\"]\n",
        )
        .unwrap();
        assert!(load(dir.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn load_unreadable_file_is_err() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".tyba")).unwrap();
        let path = dir.path().join(CONFIG_REL);
        std::fs::write(&path, "[agent]\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = load(dir.path());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(result, Err(RepoConfigError::Io(_))));
    }
}
