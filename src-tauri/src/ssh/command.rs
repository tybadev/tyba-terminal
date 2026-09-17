//! O único lugar que cria processo `ssh` (e o `ssh-add`).
//!
//! Todo `ssh` do core recebe `PATH` e `SSH_AUTH_SOCK` do login shell do dono:
//! o app lançado pelo Dock herda o ambiente do launchd, e sem isso o `ssh` do
//! TYBA não enxerga o agente que o `ssh` digitado à mão enxerga.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use parking_lot::RwLock;
use portable_pty::CommandBuilder;

use crate::error::AppError;
use crate::shell_path::{login_env, LoginEnv};

const SSH: &str = if cfg!(windows) { "ssh.exe" } else { "ssh" };
const SSH_ADD: &str = if cfg!(windows) {
    "ssh-add.exe"
} else {
    "ssh-add"
};
const SSH_KEYGEN: &str = if cfg!(windows) {
    "ssh-keygen.exe"
} else {
    "ssh-keygen"
};

fn find_in_path(program: &str, path: Option<&str>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// O binário resolvido no PATH do login shell. Não achou? O nome cru, e o SO
/// procura no PATH que o processo já tem.
fn program(name: &str, env: &LoginEnv) -> OsString {
    find_in_path(name, env.path.as_deref())
        .map(PathBuf::into_os_string)
        .unwrap_or_else(|| OsString::from(name))
}

fn env_pairs(env: &LoginEnv) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::new();
    if let Some(path) = &env.path {
        pairs.push(("PATH", path.clone()));
    }
    if let Some(sock) = &env.ssh_auth_sock {
        pairs.push(("SSH_AUTH_SOCK", sock.clone()));
    }
    pairs
}

fn resolved_env() -> &'static LoginEnv {
    static ENV: OnceLock<LoginEnv> = OnceLock::new();
    ENV.get_or_init(login_env)
}

fn std_command_with(name: &str, env: &LoginEnv) -> Command {
    let mut cmd = Command::new(program(name, env));
    for (k, v) in env_pairs(env) {
        cmd.env(k, v);
    }
    crate::repo::no_console_window(&mut cmd);
    cmd
}

/// `ssh` para processo sem terminal: probe, túnel, SFTP, `ssh -G`, teste.
pub fn std_command() -> Command {
    std_command_with(SSH, resolved_env())
}

/// `ssh` para o PTY do Cano.
pub fn pty_command() -> CommandBuilder {
    let env = resolved_env();
    let mut cmd = CommandBuilder::new(program(SSH, env));
    for (k, v) in env_pairs(env) {
        cmd.env(k, v);
    }
    cmd
}

/// `ssh-add` para listar as chaves do agente, com o mesmo ambiente.
pub fn ssh_add_command() -> Command {
    std_command_with(SSH_ADD, resolved_env())
}

/// `ssh-keygen` para inspecionar uma chave (digital, se abre sem passphrase).
pub fn keygen_command() -> Command {
    std_command_with(SSH_KEYGEN, resolved_env())
}

/// Para processo que chama o `ssh` por conta própria — o `docker` com
/// `DOCKER_HOST=ssh://`.
pub fn apply_env(cmd: &mut Command) {
    for (k, v) in env_pairs(resolved_env()) {
        cmd.env(k, v);
    }
}

/// O mesmo, para processo que roda num PTY (aba de container remoto).
pub fn apply_pty_env(cmd: &mut CommandBuilder) {
    for (k, v) in env_pairs(resolved_env()) {
        cmd.env(k, v);
    }
}

/// Existe um master (`ControlMaster`) vivo para este alias?
pub fn master_alive(alias: &str) -> bool {
    std_command()
        .args(["-O", "check", "--", alias])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn password_aliases() -> &'static RwLock<HashSet<String>> {
    static ALIASES: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
    ALIASES.get_or_init(|| RwLock::new(HashSet::new()))
}

/// Atualizado a cada materialização: quem abre conexão de fundo só sabe o alias.
pub fn set_password_aliases(aliases: impl IntoIterator<Item = String>) {
    *password_aliases().write() = aliases.into_iter().collect();
}

fn require_session_with(
    alias: &str,
    is_password: bool,
    master_alive: impl FnOnce(&str) -> bool,
) -> Result<(), AppError> {
    if !is_password || master_alive(alias) {
        return Ok(());
    }
    Err(AppError::new("ssh.password_needs_session").with("alias", alias))
}

/// Regra 27: conexão de fundo (SFTP, docker, túnel) não tem onde pedir senha.
/// Host de senha só serve por cima do master de uma sessão aberta; sem ele,
/// falha na hora em vez de esperar o timeout.
pub fn require_session_if_password(alias: &str) -> Result<(), AppError> {
    let is_password = password_aliases().read().contains(alias);
    require_session_with(alias, is_password, master_alive)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_env(dir: &Path) -> LoginEnv {
        LoginEnv {
            path: Some(format!("{}:/usr/bin:/bin", dir.display())),
            ssh_auth_sock: Some("/tmp/fake-agent.sock".into()),
        }
    }

    #[cfg(unix)]
    fn executable(dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn ssh_vem_do_path_do_login_shell_com_o_agente_dele() {
        let dir = tempfile::tempdir().unwrap();
        let fake = executable(dir.path(), "ssh");
        let env = fake_env(dir.path());
        let cmd = std_command_with(SSH, &env);
        assert_eq!(cmd.get_program(), fake.as_os_str());
        let envs: Vec<(String, String)> = cmd
            .get_envs()
            .filter_map(|(k, v)| Some((k.to_str()?.to_string(), v?.to_str()?.to_string())))
            .collect();
        assert!(envs.contains(&("SSH_AUTH_SOCK".into(), "/tmp/fake-agent.sock".into())));
        assert!(envs.contains(&("PATH".into(), env.path.clone().unwrap())));
        assert_eq!(
            envs.len(),
            2,
            "nenhuma outra variável do login shell atravessa"
        );
    }

    #[cfg(unix)]
    #[test]
    fn arquivo_sem_permissao_de_execucao_nao_e_o_ssh() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ssh"), "nao executa").unwrap();
        let found = find_in_path("ssh", Some(&format!("{}:/usr/bin", dir.path().display())));
        assert_ne!(found, Some(dir.path().join("ssh")));
    }

    #[test]
    fn sem_path_resolvido_o_nome_cru_fica() {
        assert_eq!(program(SSH, &LoginEnv::default()), OsString::from(SSH));
    }

    #[test]
    fn host_de_senha_sem_master_falha_na_hora() {
        let err = require_session_with("vps-senha", true, |_| false).unwrap_err();
        assert_eq!(err.code, "ssh.password_needs_session");
        assert_eq!(
            err.params.get("alias").map(String::as_str),
            Some("vps-senha")
        );
        assert!(require_session_with("vps-senha", true, |_| true).is_ok());
        let mut asked = false;
        assert!(require_session_with("vps-chave", false, |_| {
            asked = true;
            false
        })
        .is_ok());
        assert!(!asked, "host de chave não paga um `ssh -O check`");
    }

    #[test]
    fn registro_de_hosts_de_senha_segue_a_materializacao() {
        set_password_aliases(["tyba-test-registro-senha".to_string()]);
        let err = require_session_if_password("tyba-test-registro-senha");
        set_password_aliases(Vec::new());
        assert!(
            err.is_err(),
            "alias sem master vivo (nome que não existe) com método senha"
        );
        assert!(require_session_if_password("tyba-test-registro-senha").is_ok());
    }

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// Regra 26: um `ssh` montado fora daqui nasce sem o agente do dono, e o
    /// sintoma é o Host que conecta no terminal e não conecta pelo TYBA.
    #[test]
    fn nenhum_ssh_nasce_fora_do_construtor() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let own = src.join("ssh").join("command.rs");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        assert!(
            files.len() > 50,
            "a varredura tem de alcançar o crate inteiro"
        );
        let needle = ["new(", "\"ssh\")"].concat();
        let offenders: Vec<String> = files
            .iter()
            .filter(|f| **f != own)
            .filter(|f| std::fs::read_to_string(f).unwrap().contains(&needle))
            .map(|f| f.display().to_string())
            .collect();
        assert!(
            offenders.is_empty(),
            "ssh montado fora do construtor: {offenders:?}"
        );
    }
}
