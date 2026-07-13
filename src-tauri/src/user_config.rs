use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserConfig {
    pub sandbox_read_allow: Vec<PathBuf>,
}

#[derive(Deserialize, Default)]
struct RawUserConfig {
    #[serde(default)]
    sandbox: RawSandbox,
}

#[derive(Deserialize, Default)]
struct RawSandbox {
    #[serde(default)]
    read_allow: Vec<String>,
}

pub fn config_path(home: &Path) -> PathBuf {
    home.join(".tyba").join("config.toml")
}

fn expand(entry: &str, home: &Path) -> Result<PathBuf, String> {
    if entry == "~" {
        return Ok(home.to_path_buf());
    }
    if let Some(rest) = entry.strip_prefix("~/") {
        return Ok(home.join(rest));
    }
    let path = PathBuf::from(entry);
    if !path.is_absolute() {
        return Err(format!("read_allow exige path absoluto ou ~/: {entry}"));
    }
    Ok(path)
}

pub fn parse(content: &str, home: &Path) -> Result<UserConfig, String> {
    let raw: RawUserConfig =
        toml::from_str(content).map_err(|e| format!("~/.tyba/config.toml inválido: {e}"))?;
    let mut sandbox_read_allow = Vec::new();
    for entry in &raw.sandbox.read_allow {
        sandbox_read_allow.push(expand(entry, home)?);
    }
    Ok(UserConfig { sandbox_read_allow })
}

pub fn load(home: &Path) -> Result<UserConfig, String> {
    let path = config_path(home);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(UserConfig::default()),
        Err(e) => return Err(format!("leitura de ~/.tyba/config.toml: {e}")),
    };
    parse(&content, home)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), UserConfig::default());
    }

    #[test]
    fn parses_read_allow_with_tilde_expansion() {
        let home = Path::new("/Users/x");
        let config = parse(
            "[sandbox]\nread_allow = [\"~/.kube\", \"/opt/data\"]\n",
            home,
        )
        .unwrap();
        assert_eq!(
            config.sandbox_read_allow,
            vec![PathBuf::from("/Users/x/.kube"), PathBuf::from("/opt/data")]
        );
    }

    #[test]
    fn rejects_relative_paths() {
        let err = parse(
            "[sandbox]\nread_allow = [\"relative/path\"]\n",
            Path::new("/h"),
        )
        .unwrap_err();
        assert!(err.contains("absoluto"));
    }

    #[test]
    fn rejects_malformed_toml() {
        assert!(parse("[sandbox", Path::new("/h")).is_err());
    }

    #[test]
    fn tolerates_unknown_sections() {
        let config = parse("[future]\nx = 1\n", Path::new("/h")).unwrap();
        assert_eq!(config, UserConfig::default());
    }

    #[test]
    fn empty_content_is_default() {
        assert_eq!(parse("", Path::new("/h")).unwrap(), UserConfig::default());
    }

    #[test]
    fn load_reads_real_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".tyba")).unwrap();
        std::fs::write(
            dir.path().join(".tyba/config.toml"),
            "[sandbox]\nread_allow = [\"~/.terraform.d\"]\n",
        )
        .unwrap();
        let config = load(dir.path()).unwrap();
        assert_eq!(
            config.sandbox_read_allow,
            vec![dir.path().join(".terraform.d")]
        );
    }
}
