use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::error::AppError;
use crate::ssh::{AuthMethod, Host};

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

pub(crate) fn valid_alias(alias: &str) -> bool {
    // `-` inicial vira opção do `ssh` (`-oProxyCommand=...` = exec local); barra
    // aqui também, não só na entrada da UI.
    !alias.is_empty() && !alias.starts_with('-') && !alias.chars().any(|c| c.is_whitespace())
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

/// Multiplexing: uma conexão por host, reusada. Sem isso cada `ssh` (split,
/// tab, e cada `docker ps` do painel) abre conexão nova e o agente de chave
/// (1Password, ssh-agent) pede aprovação **de novo** — autenticar uma vez é
/// normal, a cada comando é bug.
///
/// `%C` é o hash da conexão: o socket precisa ser curto porque caminho de socket
/// unix estoura em ~104 bytes. Ele nasce em `~/.ssh`, que a jaula já nega ao
/// agente nas três plataformas — quem alcança o socket entra no servidor sem
/// re-autenticar, então ele não pode viver num lugar que o agente leia.
///
/// Windows fica de fora: o OpenSSH de lá não implementa ControlMaster.
const MULTIPLEX: &str =
    "    ControlMaster auto\n    ControlPath ~/.ssh/tyba-cm-%C\n    ControlPersist 10m\n";

/// Onde moram os `.pub` das chaves de agente, como o `ssh` os lê no bloco.
const KEY_DIR_IN_CONF: &str = "~/.ssh/config.d/tyba-keys";

const KEEPALIVE: &str = "    ServerAliveInterval 15\n    ServerAliveCountMax 3\n";

/// Nome do `.pub` a partir da digital: base64 sem `/` nem `+`, que não cabem
/// (ou atrapalham) num nome de arquivo.
pub fn key_file_stem(fingerprint: &str) -> String {
    fingerprint
        .trim_start_matches("SHA256:")
        .chars()
        .filter_map(|c| match c {
            '+' => Some('-'),
            '/' => Some('_'),
            '=' => None,
            c => Some(c),
        })
        .collect()
}

/// A chave de agente, conferida, com o nome do arquivo derivado da digital que
/// o core calcula — nunca da que veio gravada.
fn checked_agent_key(h: &Host) -> Result<(crate::ssh::AgentKey, String), AppError> {
    let key = h
        .agent_key
        .as_ref()
        .ok_or_else(|| AppError::new("ssh.agent_key_invalid"))?;
    let key = crate::ssh::agent_keys::validate_agent_key(key)?;
    let stem = key_file_stem(&key.fingerprint);
    Ok((key, stem))
}

fn blank(v: &Option<String>) -> bool {
    v.as_deref().is_none_or(|v| v.trim().is_empty())
}

/// Regras 2–5: cada método com os campos que ele admite.
pub fn validate_auth(h: &Host) -> Result<(), AppError> {
    let conflict = || Err(AppError::new("ssh.auth_fields_conflict").with("alias", h.alias.clone()));
    match h.auth_method {
        AuthMethod::Auto if h.agent_key.is_some() => conflict(),
        AuthMethod::Auto => Ok(()),
        AuthMethod::Agent if !blank(&h.identity_file) => conflict(),
        AuthMethod::Agent => checked_agent_key(h).map(|_| ()),
        AuthMethod::File if h.agent_key.is_some() => conflict(),
        AuthMethod::File if blank(&h.identity_file) => {
            Err(AppError::new("ssh.identity_file_required").with("alias", h.alias.clone()))
        }
        AuthMethod::File => Ok(()),
        AuthMethod::Password if h.agent_key.is_some() || !blank(&h.identity_file) => conflict(),
        AuthMethod::Password => Ok(()),
    }
}

fn push_raw(out: &mut String, line: &str) {
    out.push_str("    ");
    out.push_str(line);
    out.push('\n');
}

/// Um bloco `Host`. Ordem estável (regra 9): `HostName`, `Port`, `User`,
/// autenticação, `ProxyJump`, forwards, keepalive, multiplex.
pub(crate) fn render_host_block(
    h: &Host,
    multiplex: bool,
    key_dir: &str,
) -> Result<String, AppError> {
    if !valid_alias(&h.alias) {
        return Err(AppError::new("ssh.alias_invalid").with("alias", h.alias.clone()));
    }
    validate_auth(h)?;
    let mut out = String::new();
    out.push_str("Host ");
    out.push_str(&h.alias);
    out.push('\n');
    push_field(&mut out, "HostName", Some(&h.hostname), &h.alias)?;
    if let Some(port) = h.port {
        out.push_str(&format!("    Port {port}\n"));
    }
    push_field(&mut out, "User", h.username.as_deref(), &h.alias)?;
    let identity = h.identity_file.as_deref().filter(|v| !v.trim().is_empty());
    match h.auth_method {
        AuthMethod::Auto => push_field(&mut out, "IdentityFile", identity, &h.alias)?,
        AuthMethod::Agent => {
            let (_, stem) = checked_agent_key(h)?;
            let path = format!("{key_dir}/{stem}.pub");
            push_field(&mut out, "IdentityFile", Some(&path), &h.alias)?;
            push_raw(&mut out, "IdentitiesOnly yes");
        }
        AuthMethod::File => {
            push_field(&mut out, "IdentityFile", identity, &h.alias)?;
            push_raw(&mut out, "IdentitiesOnly yes");
            push_raw(&mut out, "AddKeysToAgent yes");
        }
        AuthMethod::Password => {
            push_raw(&mut out, "PubkeyAuthentication no");
            push_raw(
                &mut out,
                "PreferredAuthentications keyboard-interactive,password",
            );
        }
    }
    push_field(&mut out, "ProxyJump", h.proxy_jump.as_deref(), &h.alias)?;
    for t in &h.tunnels {
        out.push_str(&t.config_line()?);
    }
    out.push_str(KEEPALIVE);
    if multiplex {
        out.push_str(MULTIPLEX);
    }
    Ok(out)
}

fn render_with(hosts: &[Host], multiplex: bool) -> Result<String, AppError> {
    let mut out = String::from(HEADER);
    for h in hosts {
        out.push_str(&render_host_block(h, multiplex, KEY_DIR_IN_CONF)?);
        out.push('\n');
    }
    Ok(out)
}

/// Renderiza o `tyba.conf` inteiro a partir dos hosts. Função pura — o DB é a
/// fonte de verdade, então cada mutação regenera tudo. Recusa alias/valor com
/// `\n`/`\r`/`"` (guard de injeção: um `\n` num campo quebraria pra fora do bloco
/// e reescreveria o ssh_config do usuário).
pub fn render_tyba_conf(hosts: &[Host]) -> Result<String, AppError> {
    render_with(hosts, cfg!(unix))
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
    let staged = path.with_extension("conf.staged");
    fs::write(&staged, content).map_err(write_failed)?;
    set_mode(&staged, 0o600)?;
    if let Err(e) = ssh_parses(&staged) {
        let _ = fs::remove_file(&staged);
        return Err(e);
    }
    fs::rename(&staged, &path).map_err(write_failed)?;
    set_mode(&path, 0o600)?;
    Ok(())
}

fn ssh_parses(path: &Path) -> Result<(), AppError> {
    let out = crate::ssh::command::std_command()
        .arg("-F")
        .arg(path)
        .args(["-G", "tyba-config-check"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(AppError::new("ssh.config_invalid").with(
            "detail",
            String::from_utf8_lossy(&o.stderr).trim().to_string(),
        )),
        Err(_) => Ok(()),
    }
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

fn key_dir(home: &Path) -> PathBuf {
    ssh_dir(home).join("config.d").join("tyba-keys")
}

/// Grava o `.pub` de cada chave de agente referenciada e devolve os nomes. Só a
/// parte pública: o `IdentityFile` de um `.pub` faz o `ssh` pedir ao agente
/// justamente essa chave.
pub(crate) fn write_key_files(
    dir: &Path,
    hosts: &[Host],
) -> Result<std::collections::HashSet<String>, AppError> {
    let mut wanted = std::collections::HashSet::new();
    for h in hosts.iter().filter(|h| h.auth_method == AuthMethod::Agent) {
        let (key, stem) = checked_agent_key(h)?;
        if wanted.is_empty() {
            fs::create_dir_all(dir).map_err(write_failed)?;
            set_mode(dir, 0o700)?;
        }
        let name = format!("{stem}.pub");
        crate::session::write_private(dir, &name, &format!("{} {}\n", key.public_key, key.name))
            .map_err(write_failed)?;
        wanted.insert(name);
    }
    Ok(wanted)
}

/// Depois do `tyba.conf` novo instalado: o antigo ainda podia apontar para
/// estes arquivos.
fn prune_key_files(dir: &Path, wanted: &std::collections::HashSet<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !wanted.contains(&name) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Regra 10, primeira metade: renderiza e passa pelo `ssh -G` num arquivo à
/// parte, sem tocar no `tyba.conf` nem nas chaves. Quem chama grava no banco
/// só depois disto.
pub fn validate_hosts(home: &Path, hosts: &[Host]) -> Result<(), AppError> {
    let content = render_tyba_conf(hosts)?;
    let dir = ssh_dir(home).join("config.d");
    fs::create_dir_all(&dir).map_err(write_failed)?;
    set_mode(&ssh_dir(home), 0o700)?;
    set_mode(&dir, 0o700)?;
    let staged = dir.join(format!("tyba.conf.check-{}", uuid::Uuid::new_v4().simple()));
    let result = fs::write(&staged, content)
        .map_err(write_failed)
        .and_then(|()| set_mode(&staged, 0o600))
        .and_then(|()| ssh_parses(&staged));
    let _ = fs::remove_file(&staged);
    result
}

/// Materializa: render + chaves + write + ensure include. Ponto de entrada
/// chamado após cada mutação de Host e no boot.
pub fn materialize(home: &Path, hosts: &[Host]) -> Result<(), AppError> {
    let content = render_tyba_conf(hosts)?;
    let wanted = write_key_files(&key_dir(home), hosts)?;
    write_tyba_conf(home, &content)?;
    prune_key_files(&key_dir(home), &wanted);
    ensure_include_line(home)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::tunnel::{Tunnel, TunnelKind};
    use chrono::Utc;

    #[test]
    fn config_ruim_nunca_substitui_o_arquivo_bom() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        write_tyba_conf(home, "Host bom\n    HostName ok.host\n").unwrap();

        let err = write_tyba_conf(home, "Host x\n    LocalForward 5432:localhost:5432\n");

        assert!(
            err.is_err(),
            "linha invalida no tyba.conf quebra TODO ssh/scp/git da maquina \
             (o arquivo e Included): o ssh -G tem que barrar antes de instalar"
        );
        let vivo = fs::read_to_string(conf_path(home)).unwrap();
        assert!(
            vivo.contains("ok.host"),
            "o arquivo bom tem que sobreviver a tentativa ruim; got:\n{vivo}"
        );
        assert!(
            !conf_path(home).with_extension("conf.staged").exists(),
            "o staged nao pode ficar para tras"
        );
    }

    #[test]
    fn config_bom_e_instalado() {
        let tmp = tempfile::tempdir().unwrap();
        write_tyba_conf(
            tmp.path(),
            "Host bom\n    LocalForward 5432 localhost:5432\n",
        )
        .unwrap();
        let vivo = fs::read_to_string(conf_path(tmp.path())).unwrap();
        assert!(
            vivo.contains("LocalForward 5432 localhost:5432"),
            "got:\n{vivo}"
        );
    }

    fn host(alias: &str, hostname: &str) -> Host {
        Host {
            integration_enabled: true,
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
            tunnels: Vec::new(),
            auth_method: crate::ssh::AuthMethod::Auto,
            agent_key: None,
            created_at: Utc::now(),
            last_connected_at: None,
        }
    }

    fn tunnels() -> Vec<Tunnel> {
        vec![
            Tunnel {
                kind: TunnelKind::Local,
                listen_port: 5432,
                listen_host: None,
                target_host: Some("localhost".into()),
                target_port: Some(5432),
            },
            Tunnel {
                kind: TunnelKind::Remote,
                listen_port: 8000,
                listen_host: None,
                target_host: Some("localhost".into()),
                target_port: Some(3000),
            },
            Tunnel {
                kind: TunnelKind::Dynamic,
                listen_port: 1080,
                listen_host: None,
                target_host: None,
                target_port: None,
            },
        ]
    }

    #[test]
    fn tunel_de_host_sai_dentro_do_bloco_do_host_certo() {
        let mut a = host("a", "a.host");
        a.tunnels = tunnels();
        let out = render_with(&[a, host("b", "b.host")], false).unwrap();

        let expected = "Host a\n    HostName a.host\n    \
             LocalForward 127.0.0.1:5432 localhost:5432\n    \
             RemoteForward 127.0.0.1:8000 localhost:3000\n    \
             DynamicForward 127.0.0.1:1080\n    \
             ServerAliveInterval 15\n    ServerAliveCountMax 3\n\nHost b\n";
        assert!(
            out.contains(expected),
            "os forwards têm que sair ancorados no bloco do host que os declarou: \
             fora do bloco eles valeriam para TODO host do ssh_config. \
             got:\n{out}"
        );
    }

    #[test]
    fn tunel_de_host_vem_antes_do_multiplex() {
        let mut h = host("db", "10.0.0.5");
        h.tunnels = vec![tunnels().remove(0)];
        let out = render_with(&[h], true).unwrap();
        let fwd = out.find("LocalForward").expect("forward na saída");
        let cm = out.find("ControlMaster").expect("multiplex na saída");
        assert!(
            fwd < cm,
            "ordem estável do bloco: forwards do cadastro, depois o multiplex. \
             got:\n{out}"
        );
    }

    #[test]
    fn render_com_tunel_e_idempotente() {
        let mut h = host("db", "10.0.0.5");
        h.tunnels = tunnels();
        let um = render_tyba_conf(std::slice::from_ref(&h)).unwrap();
        let dois = render_tyba_conf(&[h]).unwrap();
        assert_eq!(um, dois, "o writer é puro: mesma entrada, mesma saída");
        assert_eq!(
            um.matches("DynamicForward 127.0.0.1:1080").count(),
            1,
            "cada túnel sai uma vez só; duplicar é config inválida no ~/.ssh"
        );
    }

    #[test]
    #[ignore = "usa rede: exige TYBA_E2E_SSH_ALIAS apontando para um host real e alcançável"]
    fn o_conf_renderizado_abre_um_tunel_que_o_ssh_de_verdade_aceita() {
        let alias = std::env::var("TYBA_E2E_SSH_ALIAS").expect("TYBA_E2E_SSH_ALIAS");
        let home = tmp_home();
        let mut h = host(&alias, "placeholder");
        h.hostname = std::env::var("TYBA_E2E_SSH_HOSTNAME").expect("TYBA_E2E_SSH_HOSTNAME");
        h.username = std::env::var("TYBA_E2E_SSH_USER").ok();
        h.tunnels = vec![Tunnel {
            kind: TunnelKind::Local,
            listen_port: 15432,
            listen_host: Some("127.0.0.1".into()),
            target_host: Some("localhost".into()),
            target_port: Some(22),
        }];
        materialize(home.path(), &[h]).unwrap();
        let conf = conf_path(home.path());

        let mut cmd = crate::ssh::command::std_command();
        cmd.arg("-F")
            .arg(&conf)
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]);
        if let Ok(agent) = std::env::var("TYBA_E2E_SSH_IDENTITY_AGENT") {
            cmd.arg("-o").arg(format!("IdentityAgent=\"{agent}\""));
        }
        let out = cmd.args([&alias, "true"]).output().expect("ssh roda");
        assert!(
            out.status.success(),
            "o ssh recusou o conf com LocalForward: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let g = crate::ssh::command::std_command()
            .arg("-F")
            .arg(&conf)
            .args(["-G", &alias])
            .output()
            .expect("ssh -G roda");
        let rendered = String::from_utf8_lossy(&g.stdout).to_lowercase();
        assert!(
            rendered.contains("localforward [127.0.0.1]:15432 [localhost]:22"),
            "o ssh tem que ENTENDER o forward, não só tolerar a linha — \
             `ssh -G` mostra o que ele de fato aplicaria, já normalizado. got:\n{rendered}"
        );

        let mut piped = crate::ssh::command::std_command();
        piped
            .arg("-F")
            .arg(&conf)
            .args(["-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes"]);
        if let Ok(agent) = std::env::var("TYBA_E2E_SSH_IDENTITY_AGENT") {
            piped.arg("-o").arg(format!("IdentityAgent=\"{agent}\""));
        }
        let mut child = piped
            .args(["-N", &alias])
            .spawn()
            .expect("o cano do túnel sobe");

        let banner = (0..50)
            .find_map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(200));
                let mut s = std::net::TcpStream::connect(("127.0.0.1", 15432)).ok()?;
                s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .ok()?;
                let mut buf = [0u8; 32];
                let n = std::io::Read::read(&mut s, &mut buf).ok()?;
                Some(String::from_utf8_lossy(&buf[..n]).to_string())
            })
            .unwrap_or_default();
        let _ = child.kill();
        let _ = child.wait();

        assert!(
            banner.starts_with("SSH-"),
            "o teste de verdade não é o ssh aceitar a linha, é o byte atravessar: \
             127.0.0.1:15432 tem que entregar o banner do sshd do host remoto. got: {banner:?}"
        );
    }

    #[test]
    fn tunel_invalido_barra_o_render_inteiro() {
        let mut h = host("db", "10.0.0.5");
        h.tunnels = vec![Tunnel {
            kind: TunnelKind::Local,
            listen_port: 5432,
            listen_host: None,
            target_host: Some("localhost\n    RemoteForward 22 localhost:22".into()),
            target_port: Some(5432),
        }];
        assert_eq!(
            render_tyba_conf(&[h]).unwrap_err().code,
            "ssh.tunnel_host_invalid",
            "o alvo do túnel entra num arquivo que é Include do ~/.ssh/config: \
             injeção aqui reescreve o ssh da máquina inteira"
        );
    }

    fn agent_key(seed: u8) -> crate::ssh::AgentKey {
        use base64::Engine;
        let mut blob = Vec::new();
        blob.extend_from_slice(&11u32.to_be_bytes());
        blob.extend_from_slice(b"ssh-ed25519");
        blob.extend_from_slice(&32u32.to_be_bytes());
        blob.extend_from_slice(&[seed; 32]);
        let public_key = format!(
            "ssh-ed25519 {}",
            base64::engine::general_purpose::STANDARD.encode(blob)
        );
        crate::ssh::AgentKey {
            fingerprint: crate::ssh::agent_keys::fingerprint_of(&public_key).unwrap(),
            public_key,
            name: format!("Chave {seed}"),
        }
    }

    fn block(out: &str, alias: &str) -> String {
        let start = out.find(&format!("Host {alias}\n")).expect("bloco do host");
        let rest = &out[start..];
        let end = rest.find("\n\n").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    #[test]
    fn metodo_agente_oferece_so_a_chave_escolhida_pelo_pub() {
        let mut h = host("vps", "vps.example.test");
        h.username = Some("root".into());
        h.auth_method = crate::ssh::AuthMethod::Agent;
        h.agent_key = Some(agent_key(9));
        let out = render_with(&[h.clone()], false).unwrap();
        let stem = key_file_stem(&h.agent_key.unwrap().fingerprint);
        assert_eq!(
            block(&out, "vps"),
            format!(
                "Host vps\n    HostName vps.example.test\n    User root\n    \
                 IdentityFile ~/.ssh/config.d/tyba-keys/{stem}.pub\n    IdentitiesOnly yes\n    \
                 ServerAliveInterval 15\n    ServerAliveCountMax 3"
            )
        );
    }

    #[test]
    fn metodo_arquivo_fixa_a_chave_e_a_entrega_ao_agente() {
        let mut h = host("db", "db.example.test");
        h.auth_method = crate::ssh::AuthMethod::File;
        h.identity_file = Some("/Users/dono/.ssh/id_db".into());
        let out = render_with(&[h], false).unwrap();
        assert_eq!(
            block(&out, "db"),
            "Host db\n    HostName db.example.test\n    IdentityFile /Users/dono/.ssh/id_db\n    \
             IdentitiesOnly yes\n    AddKeysToAgent yes\n    \
             ServerAliveInterval 15\n    ServerAliveCountMax 3"
        );
    }

    #[test]
    fn metodo_senha_desliga_chave_e_pede_senha() {
        let mut h = host("legado", "legado.example.test");
        h.auth_method = crate::ssh::AuthMethod::Password;
        let out = render_with(&[h], false).unwrap();
        assert_eq!(
            block(&out, "legado"),
            "Host legado\n    HostName legado.example.test\n    PubkeyAuthentication no\n    \
             PreferredAuthentications keyboard-interactive,password\n    \
             ServerAliveInterval 15\n    ServerAliveCountMax 3"
        );
    }

    #[test]
    fn auto_legado_so_ganha_keepalive() {
        let mut h = host("velho", "velho.example.test");
        h.identity_file = Some("/Users/dono/.ssh/velho".into());
        let out = render_with(&[h], false).unwrap();
        assert_eq!(
            block(&out, "velho"),
            "Host velho\n    HostName velho.example.test\n    IdentityFile /Users/dono/.ssh/velho\n    \
             ServerAliveInterval 15\n    ServerAliveCountMax 3"
        );
        let out = render_with(&[host("novo", "novo.example.test")], false).unwrap();
        assert_eq!(
            block(&out, "novo"),
            "Host novo\n    HostName novo.example.test\n    \
             ServerAliveInterval 15\n    ServerAliveCountMax 3"
        );
    }

    #[test]
    fn keepalive_vem_depois_dos_forwards_e_antes_do_multiplex() {
        let mut h = host("db", "10.0.0.5");
        h.port = Some(2222);
        h.username = Some("deploy".into());
        h.proxy_jump = Some("bastion".into());
        h.auth_method = crate::ssh::AuthMethod::Password;
        h.tunnels = vec![tunnels().remove(0)];
        let out = render_with(&[h], true).unwrap();
        let order = [
            "HostName",
            "Port",
            "User",
            "PubkeyAuthentication",
            "ProxyJump",
            "LocalForward",
            "ServerAliveInterval 15",
            "ServerAliveCountMax 3",
            "ControlMaster",
        ];
        let positions: Vec<usize> = order.iter().map(|k| out.find(k).expect(k)).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "got:\n{out}");
    }

    #[test]
    fn nenhuma_combinacao_escreve_usekeychain_nem_ignoreunknown() {
        use crate::ssh::AuthMethod::*;
        for method in [Auto, Agent, File, Password] {
            for multiplex in [false, true] {
                let mut h = host("x", "x.example.test");
                h.auth_method = method;
                h.tunnels = tunnels();
                match method {
                    Agent => h.agent_key = Some(agent_key(1)),
                    File => h.identity_file = Some("/k".into()),
                    _ => {}
                }
                let out = render_with(&[h], multiplex).unwrap();
                assert!(!out.contains("UseKeychain"), "{method:?}: {out}");
                assert!(!out.contains("IgnoreUnknown"), "{method:?}: {out}");
                assert!(!out.contains("StrictHostKeyChecking"), "{method:?}: {out}");
            }
        }
    }

    #[test]
    fn campos_que_o_metodo_nao_admite_barram_o_render() {
        use crate::ssh::AuthMethod::*;
        let code = |h: Host| render_with(&[h], false).unwrap_err().code;

        let mut h = host("x", "h");
        h.auth_method = File;
        assert_eq!(code(h.clone()), "ssh.identity_file_required");
        h.identity_file = Some("  ".into());
        assert_eq!(code(h), "ssh.identity_file_required");

        let mut h = host("x", "h");
        h.auth_method = Password;
        h.identity_file = Some("/k".into());
        assert_eq!(code(h), "ssh.auth_fields_conflict");

        let mut h = host("x", "h");
        h.auth_method = Password;
        h.agent_key = Some(agent_key(1));
        assert_eq!(code(h), "ssh.auth_fields_conflict");

        let mut h = host("x", "h");
        h.agent_key = Some(agent_key(1));
        assert_eq!(
            code(h),
            "ssh.auth_fields_conflict",
            "auto não carrega chave de agente"
        );

        let mut h = host("x", "h");
        h.auth_method = Agent;
        assert_eq!(code(h.clone()), "ssh.agent_key_invalid");
        let mut forged = agent_key(1);
        forged.fingerprint = agent_key(2).fingerprint;
        h.agent_key = Some(forged);
        assert_eq!(code(h), "ssh.agent_key_invalid");
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
    fn multiplex_reusa_a_conexao_uma_auth_so() {
        let out = render_with(&[host("web-01", "h")], true).unwrap();
        assert!(out.contains("    ControlMaster auto\n"), "got:\n{out}");
        assert!(out.contains("    ControlPersist 10m\n"));
        // Socket em ~/.ssh: a jaula já nega esse caminho ao agente, e quem lê o
        // socket entra no servidor sem re-autenticar.
        assert!(out.contains("    ControlPath ~/.ssh/tyba-cm-%C\n"));
    }

    #[test]
    fn sem_multiplex_o_bloco_nao_sai() {
        let out = render_with(&[host("web-01", "h")], false).unwrap();
        assert!(!out.contains("ControlMaster"));
        assert!(!out.contains("ControlPath"));
    }

    #[test]
    fn render_rejects_alias_with_whitespace() {
        let err = render_tyba_conf(&[host("bad alias", "h")]).unwrap_err();
        assert_eq!(err.code, "ssh.alias_invalid");
    }

    #[test]
    fn render_rejects_alias_starting_with_dash() {
        // Um alias como `-oProxyCommand=id` seria lido pelo ssh como opção, não
        // host — exec local disfarçado de conexão.
        let err = render_tyba_conf(&[host("-oProxyCommand=id", "h")]).unwrap_err();
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

    #[cfg(unix)]
    #[test]
    fn pub_das_chaves_de_agente_nasce_privado_e_o_que_sobra_sai() {
        use std::os::unix::fs::PermissionsExt;
        let home = tmp_home();
        let mut h = host("vps", "vps.example.test");
        h.auth_method = crate::ssh::AuthMethod::Agent;
        let key = agent_key(5);
        h.agent_key = Some(key.clone());
        let dir = home.path().join(".ssh/config.d/tyba-keys");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("velha.pub"), "ssh-ed25519 AAAA velha\n").unwrap();

        materialize(home.path(), &[h]).unwrap();

        let file = dir.join(format!("{}.pub", key_file_stem(&key.fingerprint)));
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            format!("{} {}\n", key.public_key, key.name)
        );
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600);
        assert_eq!(mode(&dir), 0o700);
        assert!(
            !dir.join("velha.pub").exists(),
            "chave que nenhum Host usa sai"
        );

        materialize(home.path(), &[host("outro", "o.example.test")]).unwrap();
        assert!(
            !file.exists(),
            "Host que trocou de método leva o .pub junto"
        );
    }

    #[test]
    fn validar_nao_instala_nada_e_barra_o_que_o_ssh_recusa() {
        let home = tmp_home();
        materialize(home.path(), &[host("bom", "ok.example.test")]).unwrap();
        let before = fs::read_to_string(conf_path(home.path())).unwrap();

        let mut ruim = host("ruim", "ruim.example.test");
        ruim.port = Some(0);
        let err = validate_hosts(home.path(), &[host("bom", "ok.example.test"), ruim]);
        assert_eq!(err.unwrap_err().code, "ssh.config_invalid");

        let mut novo = host("novo", "novo.example.test");
        novo.auth_method = crate::ssh::AuthMethod::Agent;
        novo.agent_key = Some(agent_key(3));
        validate_hosts(home.path(), &[novo]).unwrap();

        assert_eq!(fs::read_to_string(conf_path(home.path())).unwrap(), before);
        assert!(!home.path().join(".ssh/config.d/tyba-keys").exists());
        let leftovers: Vec<_> = fs::read_dir(home.path().join(".ssh/config.d"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from("tyba.conf")]);
    }

    #[test]
    fn nome_do_pub_vem_da_digital_sem_caracteres_de_caminho() {
        assert_eq!(
            key_file_stem("SHA256:ab+cd/ef=="),
            "ab-cd_ef",
            "barra no nome viraria subdiretório"
        );
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
