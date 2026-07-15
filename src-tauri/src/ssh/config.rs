use std::fs;
use std::path::{Path, PathBuf};

use crate::error::AppError;
use crate::ssh::Host;

const HEADER: &str =
    "# Gerado pelo TYBA — não editar à mão. A fonte de verdade é o app.\n# https://github.com/tybadev/tyba-terminal\n\n";
const INCLUDE_TOKEN: &str = "config.d/tyba.conf";
const INCLUDE_LINE: &str = "Include config.d/tyba.conf";

fn ssh_dir(home: &Path) -> PathBuf {
    home.join(".ssh")
}

fn conf_path(home: &Path) -> PathBuf {
    ssh_dir(home).join("config.d").join("tyba.conf")
}

fn config_path(home: &Path) -> PathBuf {
    ssh_dir(home).join("config")
}

fn valid_alias(alias: &str) -> bool {
    !alias.is_empty() && !alias.chars().any(|c| c.is_whitespace())
}

fn sanitize_value(val: &str, alias: &str, field: &str) -> Result<String, AppError> {
    if val.contains(['\n', '\r', '"']) {
        return Err(AppError::new("ssh.field_invalid")
            .with("alias", alias)
            .with("field", field));
    }
    if val.chars().any(|c| c.is_whitespace()) {
        Ok(format!("\"{val}\""))
    } else {
        Ok(val.to_string())
    }
}

fn push_field(out: &mut String, key: &str, val: Option<&str>, alias: &str) -> Result<(), AppError> {
    if let Some(v) = val {
        let v = sanitize_value(v, alias, key)?;
        out.push_str("    ");
        out.push_str(key);
        out.push(' ');
        out.push_str(&v);
        out.push('\n');
    }
    Ok(())
}

/// Renderiza o `tyba.conf` inteiro a partir dos hosts. Função pura — o DB é a
/// fonte de verdade, então cada mutação regenera tudo. Recusa alias/valor com
/// `\n`/`\r`/`"` (guard de injeção: um `\n` num campo quebraria pra fora do bloco
/// e reescreveria o ssh_config do usuário).
pub fn render_tyba_conf(hosts: &[Host]) -> Result<String, AppError> {
    let mut out = String::from(HEADER);
    for h in hosts {
        if !valid_alias(&h.alias) {
            return Err(AppError::new("ssh.alias_invalid").with("alias", h.alias.clone()));
        }
        out.push_str("Host ");
        out.push_str(&h.alias);
        out.push('\n');
        push_field(&mut out, "HostName", Some(&h.hostname), &h.alias)?;
        if let Some(port) = h.port {
            out.push_str(&format!("    Port {port}\n"));
        }
        push_field(&mut out, "User", h.username.as_deref(), &h.alias)?;
        push_field(
            &mut out,
            "IdentityFile",
            h.identity_file.as_deref(),
            &h.alias,
        )?;
        push_field(&mut out, "ProxyJump", h.proxy_jump.as_deref(), &h.alias)?;
        out.push('\n');
    }
    Ok(out)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| AppError::new("ssh.write_failed").with("detail", e.to_string()))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), AppError> {
    Ok(())
}

fn write_failed(e: std::io::Error) -> AppError {
    AppError::new("ssh.write_failed").with("detail", e.to_string())
}

/// Grava o conteúdo em `~/.ssh/config.d/tyba.conf` (0600), criando os diretórios
/// (0700) se faltarem.
pub fn write_tyba_conf(home: &Path, content: &str) -> Result<(), AppError> {
    let dir = conf_path(home);
    let dir = dir.parent().expect("conf_path tem parent");
    fs::create_dir_all(dir).map_err(write_failed)?;
    set_mode(&ssh_dir(home), 0o700)?;
    set_mode(dir, 0o700)?;
    let path = conf_path(home);
    fs::write(&path, content).map_err(write_failed)?;
    set_mode(&path, 0o600)?;
    Ok(())
}

/// Garante `Include config.d/tyba.conf` no topo do `~/.ssh/config`. Idempotente:
/// não duplica; cria o config só com a linha se ele não existir; jamais mexe no
/// resto do que o usuário tem.
pub fn ensure_include_line(home: &Path) -> Result<(), AppError> {
    fs::create_dir_all(ssh_dir(home)).map_err(write_failed)?;
    set_mode(&ssh_dir(home), 0o700)?;
    let path = config_path(home);
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let already = existing.lines().any(|l| {
        let t = l.trim();
        !t.starts_with('#') && t.contains(INCLUDE_TOKEN)
    });
    if already {
        return Ok(());
    }
    let new = if existing.trim().is_empty() {
        format!("{INCLUDE_LINE}\n")
    } else {
        format!("{INCLUDE_LINE}\n\n{existing}")
    };
    fs::write(&path, new).map_err(write_failed)?;
    set_mode(&path, 0o600)?;
    Ok(())
}

/// Materializa: render + write + ensure include. Ponto de entrada chamado após
/// cada mutação de Host.
pub fn materialize(home: &Path, hosts: &[Host]) -> Result<(), AppError> {
    let content = render_tyba_conf(hosts)?;
    write_tyba_conf(home, &content)?;
    ensure_include_line(home)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn host(alias: &str, hostname: &str) -> Host {
        Host {
            id: alias.to_string(),
            alias: alias.to_string(),
            hostname: hostname.to_string(),
            port: None,
            username: None,
            identity_file: None,
            proxy_jump: None,
            group_id: None,
            color: None,
            notes: None,
            position: 0,
            created_at: Utc::now(),
            last_connected_at: None,
        }
    }

    #[test]
    fn render_omits_optional_fields_when_none() {
        let out = render_tyba_conf(&[host("web-01", "web-01.example.com")]).unwrap();
        assert!(out.contains("Host web-01\n"));
        assert!(out.contains("    HostName web-01.example.com\n"));
        assert!(!out.contains("Port"));
        assert!(!out.contains("User"));
        assert!(!out.contains("IdentityFile"));
        assert!(!out.contains("ProxyJump"));
    }

    #[test]
    fn render_full_host_in_stable_order() {
        let mut h = host("db-01", "10.0.0.5");
        h.port = Some(2222);
        h.username = Some("deploy".into());
        h.identity_file = Some("/home/u/.ssh/prod".into());
        h.proxy_jump = Some("bastion".into());
        let out = render_tyba_conf(&[h]).unwrap();
        let expected = "Host db-01\n    HostName 10.0.0.5\n    Port 2222\n    User deploy\n    IdentityFile /home/u/.ssh/prod\n    ProxyJump bastion\n";
        assert!(out.contains(expected), "got:\n{out}");
    }

    #[test]
    fn render_multiple_hosts() {
        let out = render_tyba_conf(&[host("a", "a.host"), host("b", "b.host")]).unwrap();
        let ia = out.find("Host a\n").unwrap();
        let ib = out.find("Host b\n").unwrap();
        assert!(ia < ib);
    }

    #[test]
    fn render_quotes_values_with_spaces() {
        let mut h = host("x", "x.host");
        h.identity_file = Some("/home/My User/.ssh/key".into());
        let out = render_tyba_conf(&[h]).unwrap();
        assert!(
            out.contains("    IdentityFile \"/home/My User/.ssh/key\"\n"),
            "got:\n{out}"
        );
    }

    #[test]
    fn render_rejects_alias_with_whitespace() {
        let err = render_tyba_conf(&[host("bad alias", "h")]).unwrap_err();
        assert_eq!(err.code, "ssh.alias_invalid");
    }

    #[test]
    fn render_rejects_newline_injection_in_value() {
        let mut h = host("x", "h");
        h.username = Some("deploy\n    ProxyCommand evil".into());
        let err = render_tyba_conf(&[h]).unwrap_err();
        assert_eq!(err.code, "ssh.field_invalid");
        assert_eq!(err.params.get("field").map(String::as_str), Some("User"));
    }

    #[test]
    fn render_rejects_quote_in_value() {
        let mut h = host("x", "h");
        h.identity_file = Some("a\"b".into());
        assert_eq!(
            render_tyba_conf(&[h]).unwrap_err().code,
            "ssh.field_invalid"
        );
    }

    fn tmp_home() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
    }

    #[test]
    fn write_creates_conf_file_with_content() {
        let home = tmp_home();
        write_tyba_conf(home.path(), "Host x\n").unwrap();
        let got = fs::read_to_string(home.path().join(".ssh/config.d/tyba.conf")).unwrap();
        assert_eq!(got, "Host x\n");
    }

    #[cfg(unix)]
    #[test]
    fn write_sets_0600_on_conf() {
        use std::os::unix::fs::PermissionsExt;
        let home = tmp_home();
        write_tyba_conf(home.path(), "Host x\n").unwrap();
        let mode = fs::metadata(home.path().join(".ssh/config.d/tyba.conf"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn ensure_include_creates_config_when_absent() {
        let home = tmp_home();
        ensure_include_line(home.path()).unwrap();
        let got = fs::read_to_string(home.path().join(".ssh/config")).unwrap();
        assert_eq!(got.trim(), INCLUDE_LINE);
    }

    #[test]
    fn ensure_include_is_idempotent() {
        let home = tmp_home();
        ensure_include_line(home.path()).unwrap();
        ensure_include_line(home.path()).unwrap();
        let got = fs::read_to_string(home.path().join(".ssh/config")).unwrap();
        assert_eq!(got.matches(INCLUDE_TOKEN).count(), 1);
    }

    #[test]
    fn ensure_include_preserves_existing_config() {
        let home = tmp_home();
        let ssh = home.path().join(".ssh");
        fs::create_dir_all(&ssh).unwrap();
        fs::write(ssh.join("config"), "Host mine\n    HostName mine.host\n").unwrap();
        ensure_include_line(home.path()).unwrap();
        let got = fs::read_to_string(ssh.join("config")).unwrap();
        assert!(got.contains("Host mine"));
        assert!(got.contains(INCLUDE_LINE));
        assert!(got.find(INCLUDE_LINE).unwrap() < got.find("Host mine").unwrap());
    }

    #[test]
    fn ensure_include_respects_preexisting_user_include() {
        let home = tmp_home();
        let ssh = home.path().join(".ssh");
        fs::create_dir_all(&ssh).unwrap();
        fs::write(ssh.join("config"), "Include config.d/tyba.conf\n").unwrap();
        ensure_include_line(home.path()).unwrap();
        let got = fs::read_to_string(ssh.join("config")).unwrap();
        assert_eq!(got.matches(INCLUDE_TOKEN).count(), 1);
    }

    #[test]
    fn materialize_round_trips() {
        let home = tmp_home();
        materialize(home.path(), &[host("web-01", "web-01.host")]).unwrap();
        let conf = fs::read_to_string(home.path().join(".ssh/config.d/tyba.conf")).unwrap();
        assert!(conf.contains("Host web-01"));
        let config = fs::read_to_string(home.path().join(".ssh/config")).unwrap();
        assert!(config.contains(INCLUDE_LINE));
    }
}
