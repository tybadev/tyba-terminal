//! Chaves do agente SSH: listagem (`ssh-add -L`) e a digital calculada no core.

use std::io::Read;
use std::path::Path;
use std::process::{Child, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::error::AppError;
use crate::ssh::AgentKey;

use base64::Engine;
use sha2::{Digest, Sha256};

fn invalid() -> AppError {
    AppError::new("ssh.agent_key_invalid")
}

struct PublicKey<'a> {
    key_type: &'a str,
    blob: Vec<u8>,
}

/// `<tipo> <base64>[ <comentário>]` numa linha só, com o tipo declarado igual
/// ao que está dentro do blob.
fn parse_public_key(line: &str) -> Result<PublicKey<'_>, AppError> {
    if line.contains(['\n', '\r', '\0']) {
        return Err(invalid());
    }
    let mut parts = line.trim().splitn(3, ' ');
    let (Some(key_type), Some(b64)) = (parts.next(), parts.next()) else {
        return Err(invalid());
    };
    let blob = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| invalid())?;
    let declared = blob
        .get(..4)
        .map(|n| u32::from_be_bytes([n[0], n[1], n[2], n[3]]) as usize)
        .and_then(|len| blob.get(4..4usize.checked_add(len)?))
        .ok_or_else(invalid)?;
    if key_type.is_empty() || declared != key_type.as_bytes() {
        return Err(invalid());
    }
    Ok(PublicKey { key_type, blob })
}

/// Formato do OpenSSH: `SHA256:` + base64 sem padding do hash do blob.
pub fn fingerprint_of(public_key: &str) -> Result<String, AppError> {
    let key = parse_public_key(public_key)?;
    let digest = Sha256::digest(&key.blob);
    Ok(format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    ))
}

/// A chave como o core a guarda: `<tipo> <base64>`, sem o comentário (que vira
/// `name`), e a digital conferida.
pub fn validate_agent_key(key: &AgentKey) -> Result<AgentKey, AppError> {
    let parsed = parse_public_key(&key.public_key)?;
    let fingerprint = fingerprint_of(&key.public_key)?;
    if fingerprint != key.fingerprint || key.name.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(AgentKey {
        public_key: format!(
            "{} {}",
            parsed.key_type,
            base64::engine::general_purpose::STANDARD.encode(&parsed.blob)
        ),
        name: key.name.clone(),
        fingerprint,
    })
}

pub const MAX_KEYS: usize = 64;
const MAX_LISTING_BYTES: u64 = 256 * 1024;
const LISTING_DEADLINE: Duration = Duration::from_secs(10);
/// Alias que nenhum Host usa: `ssh -G` com ele devolve só o que vale para todos.
const PROBE_ALIAS: &str = "tyba-agent-probe";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentKeyListing {
    pub socket: Option<String>,
    pub keys: Vec<AgentKey>,
}

/// O socket que o `ssh` usaria para o alias: `identityagent` do `ssh -G`, com
/// `~` expandido; `none`, ausente ou a própria variável caem no
/// `SSH_AUTH_SOCK` do login shell.
fn resolve_socket(ssh_g: &str, home: &Path, login_sock: Option<&str>) -> Option<String> {
    let configured = ssh_g
        .lines()
        .find_map(|l| l.strip_prefix("identityagent "))
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "none");
    match configured {
        None | Some("SSH_AUTH_SOCK" | "$SSH_AUTH_SOCK") => login_sock.map(str::to_string),
        Some(v) => Some(match v.strip_prefix("~/") {
            Some(rest) => home.join(rest).to_string_lossy().into_owned(),
            None => v.to_string(),
        }),
    }
}

/// Regra 28. Só `ssh-add -L`: lista as públicas, nunca pede assinatura.
pub fn list(alias: Option<&str>) -> Result<AgentKeyListing, AppError> {
    let alias = alias.unwrap_or(PROBE_ALIAS);
    let g = crate::ssh::command::std_command()
        .args(["-G", "--", alias])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    list_from_ssh_g(&g)
}

/// A mesma listagem a partir de um `ssh -G` já rodado (o teste de conexão roda
/// o seu com `-F` próprio).
pub(crate) fn list_from_ssh_g(g: &str) -> Result<AgentKeyListing, AppError> {
    let home = crate::ssh::home_dir().unwrap_or_default();
    let login = crate::shell_path::login_env().ssh_auth_sock;
    list_from_socket(resolve_socket(g, &home, login.as_deref()).as_deref())
}

fn unreachable(detail: impl Into<String>) -> AppError {
    AppError::new("ssh.agent_unreachable").with("detail", detail)
}

fn list_from_socket(socket: Option<&str>) -> Result<AgentKeyListing, AppError> {
    // Sem agente não é falha: o front tem um estado próprio para `socket: None`.
    // `agent_unreachable` fica para quando há socket e o ssh-add não fala com ele.
    let Some(socket) = socket else {
        return Ok(AgentKeyListing {
            socket: None,
            keys: Vec::new(),
        });
    };
    let mut child = crate::ssh::command::ssh_add_command()
        .arg("-L")
        .env("SSH_AUTH_SOCK", socket)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| unreachable(e.to_string()))?;
    let mut stdout = child.stdout.take().ok_or_else(|| unreachable("stdout"))?;
    let mut stderr = child.stderr.take().ok_or_else(|| unreachable("stderr"))?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut stdout).take(MAX_LISTING_BYTES).read_to_end(&mut buf);
        // O resto é drenado para o ssh-add não travar com o pipe cheio.
        let _ = std::io::copy(&mut stdout, &mut std::io::sink());
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut stderr).take(4096).read_to_end(&mut buf);
        buf
    });
    let status = wait_with_deadline(&mut child, LISTING_DEADLINE);
    let out = reader.join().unwrap_or_default();
    let err = err_reader.join().unwrap_or_default();
    let Some(status) = status else {
        return Err(unreachable("ssh-add não respondeu"));
    };
    let listing = AgentKeyListing {
        socket: Some(socket.to_string()),
        keys: parse_listing(&String::from_utf8_lossy(&out)),
    };
    // `ssh-add -L` sai 1 para agente sem chave e 2 para agente inalcançável.
    match status.code() {
        Some(0) | Some(1) => Ok(listing),
        _ => Err(unreachable(
            String::from_utf8_lossy(&err).trim().to_string(),
        )),
    }
}

fn wait_with_deadline(child: &mut Child, deadline: Duration) -> Option<ExitStatus> {
    let until = Instant::now() + deadline;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() >= until => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
}

/// Saída do `ssh-add -L`: `<tipo> <base64> <comentário>` por linha. Linha que
/// não é chave é ignorada.
pub fn parse_listing(stdout: &str) -> Vec<AgentKey> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let parsed = parse_public_key(line).ok()?;
            let name = line.splitn(3, ' ').nth(2).unwrap_or_default().trim();
            Some(AgentKey {
                public_key: format!(
                    "{} {}",
                    parsed.key_type,
                    base64::engine::general_purpose::STANDARD.encode(&parsed.blob)
                ),
                name: name.chars().filter(|c| !c.is_control()).collect(),
                fingerprint: fingerprint_of(line).ok()?,
            })
        })
        .take(MAX_KEYS)
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn digital_forjada_e_recusada() {
        let key = AgentKey {
            public_key: synthetic_key(7),
            name: "teste".into(),
            fingerprint: "SHA256:qualquercoisa".into(),
        };
        assert_eq!(
            validate_agent_key(&key).unwrap_err().code,
            "ssh.agent_key_invalid",
            "a digital que veio do front tem de bater com a que o core calcula"
        );
        let honest = AgentKey {
            fingerprint: fingerprint_of(&key.public_key).unwrap(),
            ..key
        };
        assert_eq!(validate_agent_key(&honest).unwrap(), honest);
    }

    fn honest(public_key: String) -> AgentKey {
        AgentKey {
            fingerprint: fingerprint_of(&public_key).unwrap_or_default(),
            public_key,
            name: "teste".into(),
        }
    }

    #[test]
    fn chave_em_mais_de_uma_linha_ou_fora_do_formato_e_recusada() {
        let good = synthetic_key(1);
        for bad in [
            format!("{good}\nssh-ed25519 AAAA"),
            format!("{good}\r"),
            "ssh-ed25519".to_string(),
            "ssh-ed25519 !!!nao-e-base64".to_string(),
            good.replacen("ssh-ed25519", "ssh-rsa", 1),
            String::new(),
        ] {
            let mut key = honest(good.clone());
            key.public_key = bad.clone();
            assert_eq!(
                validate_agent_key(&key).map_err(|e| e.code),
                Err("ssh.agent_key_invalid".to_string()),
                "{bad:?}"
            );
        }
        let mut key = honest(good);
        key.name = "nome\nIdentityFile /etc/passwd".into();
        assert!(
            validate_agent_key(&key).is_err(),
            "o nome vai para o arquivo .pub"
        );
    }

    #[test]
    fn comentario_da_linha_nao_entra_na_chave_guardada() {
        let key = honest(format!("{} laptop@example.test", synthetic_key(2)));
        assert_eq!(
            validate_agent_key(&key).unwrap().public_key,
            synthetic_key(2)
        );
    }

    #[cfg(unix)]
    #[test]
    fn digital_bate_com_a_do_ssh_keygen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("id_teste");
        let made = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "descartavel", "-f"])
            .arg(&path)
            .status();
        let Ok(status) = made else {
            eprintln!("ssh-keygen ausente: teste pulado");
            return;
        };
        assert!(status.success());
        let public = std::fs::read_to_string(path.with_extension("pub")).unwrap();
        let out = std::process::Command::new("ssh-keygen")
            .args(["-l", "-E", "sha256", "-f"])
            .arg(path.with_extension("pub"))
            .output()
            .unwrap();
        let listed = String::from_utf8_lossy(&out.stdout);
        let expected = listed.split_whitespace().nth(1).unwrap();
        assert_eq!(fingerprint_of(public.trim()).unwrap(), expected);
    }

    #[test]
    fn listagem_do_ssh_add_vira_chaves_com_nome_e_digital() {
        let out = format!(
            "{} Chave pessoal (1Password)\nlinha que não é chave\n{}\n",
            synthetic_key(3),
            synthetic_key(4)
        );
        let keys = parse_listing(&out);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].name, "Chave pessoal (1Password)");
        assert_eq!(keys[0].public_key, synthetic_key(3));
        assert_eq!(
            keys[0].fingerprint,
            fingerprint_of(&synthetic_key(3)).unwrap()
        );
        assert_eq!(keys[1].name, "");
    }

    #[test]
    fn listagem_para_em_64_chaves() {
        let out: String = (0..100u8)
            .map(|i| format!("{} k{i}\n", synthetic_key(i)))
            .collect();
        assert_eq!(parse_listing(&out).len(), 64);
    }

    #[test]
    fn socket_vem_do_identityagent_do_alias_ou_do_login_shell() {
        let home = std::path::Path::new("/Users/dono");
        let login = Some("/private/tmp/login.sock");
        let g = "user dono\nidentityagent ~/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock\nport 22\n";
        assert_eq!(
            resolve_socket(g, home, login).as_deref(),
            Some("/Users/dono/Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock")
        );
        for g in [
            "user dono\nidentityagent none\n",
            "user dono\n",
            "identityagent SSH_AUTH_SOCK\n",
            "identityagent $SSH_AUTH_SOCK\n",
        ] {
            assert_eq!(resolve_socket(g, home, login).as_deref(), login, "{g}");
        }
        assert_eq!(
            resolve_socket("identityagent /tmp/agent.sock\n", home, None).as_deref(),
            Some("/tmp/agent.sock")
        );
    }

    #[cfg(unix)]
    struct Agent {
        child: std::process::Child,
        socket: std::path::PathBuf,
        _dir: tempfile::TempDir,
    }

    #[cfg(unix)]
    impl Agent {
        fn start() -> Option<Self> {
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("agent.sock");
            let child = std::process::Command::new("ssh-agent")
                .arg("-D")
                .arg("-a")
                .arg(&socket)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok()?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !socket.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Some(Self {
                child,
                socket,
                _dir: dir,
            })
        }

        fn add_throwaway_key(&self, comment: &str) -> String {
            let path = self._dir.path().join(format!("id_{}", comment.len()));
            let ok = std::process::Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-C", comment, "-f"])
                .arg(&path)
                .status()
                .unwrap()
                .success();
            assert!(ok);
            let ok = std::process::Command::new("ssh-add")
                .arg(&path)
                .env("SSH_AUTH_SOCK", &self.socket)
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success();
            assert!(ok);
            std::fs::read_to_string(path.with_extension("pub")).unwrap()
        }
    }

    #[cfg(unix)]
    impl Drop for Agent {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[cfg(unix)]
    #[test]
    fn agente_descartavel_lista_vazio_e_depois_a_chave() {
        let Some(agent) = Agent::start() else {
            eprintln!("ssh-agent ausente: teste pulado");
            return;
        };
        let sock = agent.socket.to_string_lossy().to_string();
        let empty = list_from_socket(Some(&sock)).expect("agente sem chave não é erro");
        assert!(empty.keys.is_empty());
        assert_eq!(empty.socket.as_deref(), Some(sock.as_str()));

        let public = agent.add_throwaway_key("chave-descartavel");
        let listing = list_from_socket(Some(&sock)).unwrap();
        assert_eq!(listing.keys.len(), 1);
        assert_eq!(listing.keys[0].name, "chave-descartavel");
        assert_eq!(
            listing.keys[0].fingerprint,
            fingerprint_of(public.trim()).unwrap()
        );
    }

    #[test]
    fn sem_socket_resolvido_e_lista_vazia_sem_socket() {
        assert_eq!(
            list_from_socket(None).map_err(|e| e.code),
            Ok(AgentKeyListing {
                socket: None,
                keys: vec![],
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn socket_que_nao_responde_e_agent_unreachable() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("nada.sock").to_string_lossy().to_string();
        let err = list_from_socket(Some(&sock)).unwrap_err();
        assert_eq!(err.code, "ssh.agent_unreachable");
        assert!(err.params.contains_key("detail"));
    }

    /// Chave ed25519 sintética: o formato é o real, os 32 bytes não são de
    /// chave nenhuma.
    pub(crate) fn synthetic_key(seed: u8) -> String {
        use base64::Engine;
        let mut blob = Vec::new();
        blob.extend_from_slice(&11u32.to_be_bytes());
        blob.extend_from_slice(b"ssh-ed25519");
        blob.extend_from_slice(&32u32.to_be_bytes());
        blob.extend_from_slice(&[seed; 32]);
        format!(
            "ssh-ed25519 {}",
            base64::engine::general_purpose::STANDARD.encode(blob)
        )
    }
}
