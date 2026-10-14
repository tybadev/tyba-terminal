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
    home.join(crate::profile::HOME_DIR).join("config.toml")
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
    // Só a linha: a mensagem do `toml` cita o trecho, que pode ser um segredo.
    let raw: RawUserConfig = toml::from_str(content).map_err(|e: toml::de::Error| {
        format!(
            "~/.tyba/config.toml: {}",
            crate::config::schema::toml_error(content, e.span())
        )
    })?;
    let mut sandbox_read_allow = Vec::new();
    for entry in &raw.sandbox.read_allow {
        sandbox_read_allow.push(expand(entry, home)?);
    }
    Ok(UserConfig { sandbox_read_allow })
}

/// Por que o `read_allow` de um `config.toml` de dotfiles não vale.
pub fn linked_read_allow_warning() -> String {
    format!(
        "read_allow ignorado: config.toml de dotfiles (symlink para fora de ~/{}) não relaxa a jaula",
        crate::profile::HOME_DIR
    )
}

pub fn load(home: &Path) -> Result<UserConfig, String> {
    let path = config_path(home);
    let root = home.join(crate::profile::HOME_DIR);
    if !root.exists() {
        return Ok(UserConfig::default());
    }
    // O `read_allow` abre a jaula: só vale de um arquivo que mora de fato em
    // ~/.tyba. Raiz ou arquivo ligados para fora (dotfiles) podem ser
    // alcançados e editados por um agente preso por um caminho que a jaula não
    // esconde.
    if crate::config::fsguard::linked_out_of_root(&root, &path) {
        return Err(linked_read_allow_warning());
    }
    // Raiz recusada (de outro usuário, gravável por outros) não vale nem para
    // o arquivo dentro dela.
    crate::config::fsguard::check_root(&root)
        .map_err(|e| format!("leitura de ~/.tyba/config.toml: {e}"))?;
    // Mesma leitura guardada do resto do arquivo (teto, symlink só para dentro,
    // dono): é o último lugar para aceitar arquivo alheio.
    match crate::config::fsguard::read_capped(&root, &path, crate::config::MAX_BYTES) {
        Ok(Some(content)) => parse(&content, home),
        Ok(None) => Ok(UserConfig::default()),
        Err(e) => Err(format!("leitura de ~/.tyba/config.toml: {e}")),
    }
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
    fn malformed_toml_error_never_echoes_the_text() {
        let err = parse(
            "[sandbox]\nread_allow = sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA\n",
            Path::new("/h"),
        )
        .unwrap_err();
        assert!(!err.contains("sk-ant"), "{err}");
        assert!(err.contains("linha 2"), "{err}");
        let err = parse(
            "[sandbox]\nread_allow = \"sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA\"\n",
            Path::new("/h"),
        )
        .unwrap_err();
        assert!(!err.contains("sk-ant"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn refused_root_is_an_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".tyba");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("config.toml"),
            "[sandbox]\nread_allow = [\"/x\"]\n",
        )
        .unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(load(dir.path()).is_err());
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn dotfiles_symlinked_config_does_not_relax_the_jail() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".tyba");
        std::fs::create_dir_all(&root).unwrap();
        let outside = dir.path().join("dotfiles.toml");
        std::fs::write(&outside, "[sandbox]\nread_allow = [\"/x\"]\n").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("config.toml")).unwrap();
        let err = load(dir.path()).unwrap_err();
        assert!(err.contains("não relaxa a jaula"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_root_does_not_relax_the_jail() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles-tyba");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(
            real.join("config.toml"),
            "[sandbox]\nread_allow = [\"/x\"]\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&real, dir.path().join(".tyba")).unwrap();
        let err = load(dir.path()).unwrap_err();
        assert!(err.contains("não relaxa a jaula"), "{err}");
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
