//! Testar conexão com os valores do formulário, sem gravar nada.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Instant;

use crate::error::AppError;
use crate::ssh::classify::FailureReason;
use crate::ssh::{AuthMethod, Host, HostInput};

/// Regra 22: aprovação do agente incluída.
pub const TEST_DEADLINE: Duration = Duration::from_secs(30);
const MAX_STDERR_BYTES: u64 = 256 * 1024;

/// O que `test_host_connection` recusa antes de rodar qualquer coisa.
pub fn validate(input: &HostInput) -> Result<(), AppError> {
    if !crate::ssh::config::valid_alias(&input.alias) {
        return Err(AppError::new("ssh.alias_invalid").with("alias", input.alias.clone()));
    }
    let host = form_host(input, "tyba-test-validate");
    crate::ssh::config::render_host_block(&host, false, "/tyba-test").map(|_| ())
}

fn form_host(input: &HostInput, alias: &str) -> Host {
    let mut host = input
        .clone()
        .into_host(alias.to_string(), 0, chrono::Utc::now());
    host.alias = alias.to_string();
    host
}

/// Diretório privado do teste. Sai no `Drop`, em qualquer caminho.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn create(path: PathBuf) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn failed(detail: impl Into<String>) -> ConnectionTest {
    ConnectionTest::Failed {
        reason: FailureReason::Unknown,
        detail: detail.into(),
    }
}

/// Regra 22: roda o formulário como está, sem gravar nada.
pub fn run(input: &HostInput, deadline: Duration) -> ConnectionTest {
    let home = crate::ssh::home_dir();
    run_with(
        input,
        deadline,
        &std::env::temp_dir(),
        home.as_deref(),
        &crate::ssh::command::std_command,
    )
}

fn quote_if_needed(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if raw.chars().any(char::is_whitespace) {
        format!("\"{raw}\"")
    } else {
        raw.into_owned()
    }
}

fn run_with(
    input: &HostInput,
    deadline: Duration,
    tmp_root: &Path,
    home: Option<&Path>,
    ssh: &dyn Fn() -> Command,
) -> ConnectionTest {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let alias = format!("tyba-test-{nonce}");
    let scratch = match ScratchDir::create(tmp_root.join(&alias)) {
        Ok(dir) => dir,
        Err(e) => return failed(e.to_string()),
    };
    let host = form_host(input, &alias);
    let config = match write_config(&scratch.0, &host, home) {
        Ok(path) => path,
        Err(e) => return failed(e.to_string()),
    };

    let started = Instant::now();
    let mut cmd = ssh();
    cmd.args(argv(&config, &alias, host.auth_method))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return failed(e.to_string()),
    };
    let reader = child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = (&mut stderr).take(MAX_STDERR_BYTES).read_to_end(&mut buf);
            let _ = std::io::copy(&mut stderr, &mut std::io::sink());
            buf
        })
    });
    // No prazo estourado a leitura não é esperada: um neto fora do grupo
    // poderia segurar o pipe aberto.
    let Some(success) = wait_or_kill(&mut child, deadline) else {
        return ConnectionTest::TimedOut;
    };
    let stderr = reader
        .and_then(|r| r.join().ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let finished = Finished {
        success,
        stderr: &stderr,
        elapsed: started.elapsed(),
    };
    let resolved = || resolve_with_g(ssh, &config, &alias);
    interpret_with(
        host.auth_method,
        &finished,
        || match host.username.as_deref().map(str::trim) {
            Some(user) if !user.is_empty() => user.to_string(),
            _ => resolved()
                .lines()
                .find_map(|l| l.strip_prefix("user "))
                .unwrap_or_default()
                .trim()
                .to_string(),
        },
        || {
            let Some(path) = host.identity_file.as_deref() else {
                return false;
            };
            let fingerprints: Vec<String> = crate::ssh::agent_keys::list_from_ssh_g(&resolved())
                .map(|l| l.keys.into_iter().map(|k| k.fingerprint).collect())
                .unwrap_or_default();
            needs_passphrase(&crate::session::expand_home(Path::new(path)), &fingerprints)
        },
    )
}

fn write_config(dir: &Path, host: &Host, home: Option<&Path>) -> Result<PathBuf, AppError> {
    crate::ssh::config::write_key_files(dir, std::slice::from_ref(host))?;
    let key_dir = dir.to_string_lossy();
    let mut content = crate::ssh::config::render_host_block(host, false, &key_dir)?;
    if let Some(home) = home {
        // Absoluto: dentro de `-F`, `Include` relativo seria relativo a `~/.ssh`
        // de qualquer jeito, mas o caminho explícito não depende disso.
        content.push_str(&format!(
            "\nInclude {}\n",
            quote_if_needed(&home.join(".ssh").join("config"))
        ));
    }
    let path = dir.join("config");
    crate::session::write_private(dir, "config", &content)
        .map_err(|e| AppError::new("ssh.write_failed").with("detail", e.to_string()))?;
    Ok(path)
}

fn resolve_with_g(ssh: &dyn Fn() -> Command, config: &Path, alias: &str) -> String {
    ssh()
        .arg("-F")
        .arg(config)
        .args(["-G", "--", alias])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// `Some(sucesso)` quando o `ssh` terminou; `None` quando o prazo matou o grupo.
fn wait_or_kill(child: &mut Child, deadline: Duration) -> Option<bool> {
    let until = Instant::now() + deadline;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) if Instant::now() >= until => {
                kill_group(child);
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return Some(false),
        }
    }
}

#[cfg(unix)]
fn kill_group(child: &mut Child) {
    // O `ssh` pode ter filhos (ProxyJump, ProxyCommand): o grupo inteiro sai.
    if let Ok(pid) = libc::pid_t::try_from(child.id()) {
        unsafe {
            libc::killpg(pid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
}

#[cfg(not(unix))]
fn kill_group(child: &mut Child) {
    let _ = child.kill();
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ConnectionTest {
    Ok {
        user: String,
        elapsed_ms: u64,
    },
    HostUnknown {
        key_type: String,
        fingerprint: String,
    },
    PassphraseRequired,
    PasswordAccepted {
        elapsed_ms: u64,
    },
    PasswordNotOffered {
        methods: Vec<String>,
    },
    Failed {
        reason: FailureReason,
        detail: String,
    },
    TimedOut,
}

/// O que o `ssh` deixou, já coletado.
struct Finished<'a> {
    success: bool,
    stderr: &'a str,
    elapsed: Duration,
}

fn elapsed_ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Regra 25. `-y -P ""` só diz se a chave abre sem passphrase; o que ele
/// imprime (a pública) é descartado sem ler.
fn needs_passphrase(path: &Path, agent_fingerprints: &[String]) -> bool {
    let opens = crate::ssh::command::keygen_command()
        .args(["-y", "-P", "", "-f"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !matches!(opens, Ok(s) if !s.success()) {
        return false;
    }
    key_fingerprint(path).is_some_and(|fp| !agent_fingerprints.contains(&fp))
}

/// `ssh-keygen -l` lê a digital da parte pública, que o formato OpenSSH guarda
/// em claro mesmo na chave cifrada.
fn key_fingerprint(path: &Path) -> Option<String> {
    let out = crate::ssh::command::keygen_command()
        .args(["-l", "-E", "sha256", "-f"])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .filter(|fp| fp.starts_with("SHA256:"))
        .map(str::to_string)
}

fn interpret_with(
    method: AuthMethod,
    run: &Finished<'_>,
    user: impl FnOnce() -> String,
    passphrase_locked: impl FnOnce() -> bool,
) -> ConnectionTest {
    if run.success {
        return ConnectionTest::Ok {
            user: user(),
            elapsed_ms: elapsed_ms(run.elapsed),
        };
    }
    if run.stderr.lines().any(is_unknown_host_line) {
        if let Some((key_type, fingerprint)) = server_host_key(run.stderr) {
            return ConnectionTest::HostUnknown {
                key_type,
                fingerprint,
            };
        }
    }
    if method == AuthMethod::Password {
        if let Some(methods) = denied_methods(run.stderr) {
            return if methods
                .iter()
                .any(|m| m == "password" || m == "keyboard-interactive")
            {
                ConnectionTest::PasswordAccepted {
                    elapsed_ms: elapsed_ms(run.elapsed),
                }
            } else {
                ConnectionTest::PasswordNotOffered { methods }
            };
        }
    }
    let failure = crate::ssh::classify::classify(&without_debug(run.stderr));
    if method == AuthMethod::File
        && failure.reason == FailureReason::AuthRefused
        && passphrase_locked()
    {
        return ConnectionTest::PassphraseRequired;
    }
    ConnectionTest::Failed {
        reason: failure.reason,
        detail: failure.detail,
    }
}

/// `user@host: Permission denied (publickey,password).`
fn denied_methods(stderr: &str) -> Option<Vec<String>> {
    let line = stderr
        .lines()
        .rev()
        .find(|l| !l.starts_with("debug") && l.contains("Permission denied ("))?;
    let (_, rest) = line.split_once("Permission denied (")?;
    let (list, _) = rest.split_once(')')?;
    Some(
        list.split(',')
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn is_unknown_host_line(line: &str) -> bool {
    line.starts_with("No ") && line.contains(" host key is known for ")
}

/// `debug1: Server host key: ssh-ed25519 SHA256:...`
fn server_host_key(stderr: &str) -> Option<(String, String)> {
    let line = stderr
        .lines()
        .find_map(|l| l.trim().strip_prefix("debug1: Server host key: "))?;
    let mut parts = line.split_whitespace();
    let key_type = parts.next()?.to_string();
    let fingerprint = parts.next()?.to_string();
    Some((key_type, fingerprint))
}

fn without_debug(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| !l.starts_with("debug") && !l.starts_with("OpenSSH_"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Regra 22. `StrictHostKeyChecking=yes` não está na lista da regra, e entra
/// pelo "known_hosts nunca é gravado": um `accept-new` no config do dono faria
/// o teste gravar a digital de um host novo em silêncio.
const BASE_OPTIONS: &[&str] = &[
    "BatchMode=yes",
    "ConnectTimeout=10",
    "ConnectionAttempts=1",
    "ControlMaster=no",
    "ControlPath=none",
    "UpdateHostKeys=no",
    "StrictHostKeyChecking=yes",
];

/// Regra 24: nenhuma chave e nenhuma senha oferecida; a recusa lista os
/// métodos que o servidor aceita.
const PASSWORD_PROBE_OPTIONS: &[&str] =
    &["PreferredAuthentications=none", "PubkeyAuthentication=no"];

fn argv(config: &Path, alias: &str, method: AuthMethod) -> Vec<String> {
    let mut args = vec!["-F".to_string(), config.to_string_lossy().into_owned()];
    let extra = if method == AuthMethod::Password {
        PASSWORD_PROBE_OPTIONS
    } else {
        &[]
    };
    for opt in BASE_OPTIONS.iter().chain(extra) {
        args.push("-o".into());
        args.push((*opt).into());
    }
    args.extend(["-T", "-v", "--", alias, "true"].map(String::from));
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(args: &[String]) -> Vec<String> {
        args.windows(2)
            .filter(|w| w[0] == "-o")
            .map(|w| w[1].clone())
            .collect()
    }

    #[test]
    fn argv_leva_todas_as_opcoes_da_regra_22() {
        let args = argv(
            Path::new("/tmp/x/config"),
            "tyba-test-abc",
            AuthMethod::Agent,
        );
        let opts = pairs(&args);
        for want in [
            "BatchMode=yes",
            "ConnectTimeout=10",
            "ConnectionAttempts=1",
            "ControlMaster=no",
            "ControlPath=none",
            "UpdateHostKeys=no",
        ] {
            assert!(opts.iter().any(|o| o == want), "falta {want}: {args:?}");
        }
        // Desvio aceito da regra 22, não remover: sem `StrictHostKeyChecking=yes`
        // um `accept-new` no config do dono grava `known_hosts` e o desfecho
        // `host_unknown` nunca aparece; `-T` impede que um `RequestTTY` do
        // config do dono mude o teste.
        assert!(
            opts.iter().any(|o| o == "StrictHostKeyChecking=yes"),
            "{args:?}"
        );
        assert!(args.iter().any(|a| a == "-T"), "{args:?}");
        assert!(
            args.windows(2).any(|w| w == ["-F", "/tmp/x/config"]),
            "{args:?}"
        );
        assert!(args.iter().any(|a| a == "-v"));
        assert_eq!(&args[args.len() - 2..], ["tyba-test-abc", "true"]);
        assert!(!opts
            .iter()
            .any(|o| o.starts_with("PreferredAuthentications")));
    }

    fn interpret(
        method: AuthMethod,
        run: &Finished<'_>,
        user: impl FnOnce() -> String,
    ) -> ConnectionTest {
        interpret_with(method, run, user, || false)
    }

    fn finished(success: bool, stderr: &str) -> Finished<'_> {
        Finished {
            success,
            stderr,
            elapsed: Duration::from_millis(842),
        }
    }

    const DEBUG_PREFIX: &str = "OpenSSH_10.0p2, LibreSSL 3.3.6\n\
        debug1: Reading configuration data /tmp/tyba-test-x/config\n\
        debug1: Connecting to vps.example.test [192.0.2.10] port 22.\n\
        debug1: Connection established.\n\
        debug1: Server host key: ssh-ed25519 SHA256:q1w2e3r4t5y6u7i8o9p0AbCdEfGhIjKlMnOpQrStUvW\n";

    #[test]
    fn sucesso_devolve_usuario_e_tempo() {
        let stderr = format!("{DEBUG_PREFIX}debug1: Authentication succeeded (publickey).\n");
        assert_eq!(
            interpret(AuthMethod::Agent, &finished(true, &stderr), || "root"
                .into()),
            ConnectionTest::Ok {
                user: "root".into(),
                elapsed_ms: 842
            }
        );
    }

    #[test]
    fn host_novo_devolve_tipo_e_digital_para_o_dono_conferir() {
        let stderr = format!(
            "{DEBUG_PREFIX}No ED25519 host key is known for vps.example.test and you have requested strict checking.\n\
             Host key verification failed.\n"
        );
        assert_eq!(
            interpret(AuthMethod::Agent, &finished(false, &stderr), String::new),
            ConnectionTest::HostUnknown {
                key_type: "ssh-ed25519".into(),
                fingerprint: "SHA256:q1w2e3r4t5y6u7i8o9p0AbCdEfGhIjKlMnOpQrStUvW".into()
            }
        );
    }

    #[test]
    fn falha_comum_sai_classificada_sem_as_linhas_de_debug() {
        let stderr = format!(
            "{DEBUG_PREFIX}debug1: Authentications that can continue: publickey\n\
             debug1: No more authentication methods to try.\n\
             root@vps.example.test: Permission denied (publickey).\n\
             debug1: Exit status 255\n"
        );
        assert_eq!(
            interpret(AuthMethod::Agent, &finished(false, &stderr), String::new),
            ConnectionTest::Failed {
                reason: FailureReason::AuthRefused,
                detail: "root@vps.example.test: Permission denied (publickey).".into()
            }
        );
    }

    fn denied(methods: &str) -> String {
        format!(
            "{DEBUG_PREFIX}debug1: Authentications that can continue: {methods}\n\
             legado@vps.example.test: Permission denied ({methods}).\n"
        )
    }

    #[test]
    fn senha_aceita_quando_o_servidor_lista_password_ou_keyboard_interactive() {
        for methods in ["publickey,password", "publickey,keyboard-interactive"] {
            assert_eq!(
                interpret(
                    AuthMethod::Password,
                    &finished(false, &denied(methods)),
                    String::new
                ),
                ConnectionTest::PasswordAccepted { elapsed_ms: 842 },
                "{methods}"
            );
        }
    }

    #[test]
    fn senha_nao_oferecida_devolve_os_metodos_do_servidor() {
        assert_eq!(
            interpret(
                AuthMethod::Password,
                &finished(false, &denied("publickey,gssapi-with-mic")),
                String::new
            ),
            ConnectionTest::PasswordNotOffered {
                methods: vec!["publickey".into(), "gssapi-with-mic".into()]
            }
        );
    }

    #[test]
    fn senha_com_rede_fora_e_falha_comum() {
        let stderr = "ssh: connect to host vps.example.test port 22: Connection refused\n";
        assert!(matches!(
            interpret(AuthMethod::Password, &finished(false, stderr), String::new),
            ConnectionTest::Failed {
                reason: FailureReason::NoRoute,
                ..
            }
        ));
    }

    #[cfg(unix)]
    fn keygen(dir: &Path, name: &str, passphrase: &str) -> Option<std::path::PathBuf> {
        let path = dir.join(name);
        let status = std::process::Command::new("ssh-keygen")
            .args([
                "-q",
                "-t",
                "ed25519",
                "-C",
                "descartavel",
                "-N",
                passphrase,
                "-f",
            ])
            .arg(&path)
            .status()
            .ok()?;
        assert!(status.success());
        // Sem o `.pub` ao lado: é o caso que obriga a ler a digital da privada.
        std::fs::remove_file(path.with_extension("pub")).unwrap();
        Some(path)
    }

    #[cfg(unix)]
    #[test]
    fn chave_cifrada_fora_do_agente_pede_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let Some(locked) = keygen(dir.path(), "cifrada", "senha-descartavel-de-teste") else {
            eprintln!("ssh-keygen ausente: teste pulado");
            return;
        };
        let open = keygen(dir.path(), "aberta", "").unwrap();

        assert!(needs_passphrase(&locked, &[]));
        assert!(
            !needs_passphrase(&open, &[]),
            "chave sem passphrase não é esse o motivo"
        );
        let fp = key_fingerprint(&locked).expect("digital da privada cifrada");
        assert!(fp.starts_with("SHA256:"), "{fp}");
        assert!(
            !needs_passphrase(&locked, &[fp]),
            "no agente a chave já está destravada: o motivo da recusa é outro"
        );
        assert!(!needs_passphrase(&dir.path().join("nao-existe"), &[]));
    }

    #[test]
    fn arquivo_recusado_com_chave_trancada_vira_passphrase_required() {
        let stderr = denied("publickey");
        let run = finished(false, &stderr);
        assert_eq!(
            interpret_with(AuthMethod::File, &run, String::new, || true),
            ConnectionTest::PassphraseRequired
        );
        assert!(matches!(
            interpret_with(AuthMethod::File, &run, String::new, || false),
            ConnectionTest::Failed {
                reason: FailureReason::AuthRefused,
                ..
            }
        ));
        let mut asked = false;
        interpret_with(AuthMethod::Agent, &run, String::new, || {
            asked = true;
            true
        });
        assert!(!asked, "só o método arquivo tem arquivo para inspecionar");
    }

    #[cfg(unix)]
    struct Harness {
        tmp_root: tempfile::TempDir,
        home: tempfile::TempDir,
        record: tempfile::TempDir,
        script: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl Harness {
        /// `behavior` é o corpo do `ssh` falso quando não é `-G`.
        fn new(behavior: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let record = tempfile::tempdir().unwrap();
            let home = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(home.path().join(".ssh/config.d")).unwrap();
            std::fs::write(home.path().join(".ssh/config.d/tyba.conf"), "Host antigo\n").unwrap();
            let script = record.path().join("fake-ssh");
            let rec = record.path().display();
            std::fs::write(
                &script,
                format!(
                    "#!/bin/sh\n\
                     for a in \"$@\"; do [ \"$a\" = -G ] && {{ echo 'user usuario-resolvido'; echo 'identityagent none'; exit 0; }}; done\n\
                     echo \"$@\" > {rec}/argv\n\
                     cat \"$2\" > {rec}/config\n\
                     cat \"$(dirname \"$2\")\"/*.pub > {rec}/pubs 2>/dev/null\n\
                     {behavior}\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self {
                tmp_root: tempfile::tempdir().unwrap(),
                home,
                record,
                script,
            }
        }

        fn run(&self, input: &HostInput, deadline: Duration) -> ConnectionTest {
            let script = self.script.clone();
            run_with(
                input,
                deadline,
                self.tmp_root.path(),
                Some(self.home.path()),
                &move || std::process::Command::new(&script),
            )
        }

        fn recorded(&self, name: &str) -> String {
            std::fs::read_to_string(self.record.path().join(name)).unwrap_or_default()
        }

        fn assert_clean(&self) {
            let left: Vec<_> = std::fs::read_dir(self.tmp_root.path()).unwrap().collect();
            assert!(
                left.is_empty(),
                "diretório temporário ficou para trás: {left:?}"
            );
            assert_eq!(
                std::fs::read_to_string(self.home.path().join(".ssh/config.d/tyba.conf")).unwrap(),
                "Host antigo\n",
                "tyba.conf não é tocado pelo teste"
            );
            assert!(!self.home.path().join(".ssh/known_hosts").exists());
        }
    }

    fn form(method: AuthMethod) -> HostInput {
        HostInput {
            alias: "vps".into(),
            hostname: "vps.example.test".into(),
            port: None,
            username: None,
            identity_file: None,
            proxy_jump: None,
            group_id: None,
            color: None,
            notes: None,
            tunnels: Vec::new(),
            auth_method: method,
            agent_key: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn sucesso_usa_o_formulario_num_alias_descartavel_e_limpa_tudo() {
        let h = Harness::new("exit 0");
        let outcome = h.run(&form(AuthMethod::Auto), Duration::from_secs(10));
        assert!(
            matches!(&outcome, ConnectionTest::Ok { user, .. } if user == "usuario-resolvido"),
            "{outcome:?}"
        );
        let config = h.recorded("config");
        assert!(config.starts_with("Host tyba-test-"), "{config}");
        assert!(
            config.contains("    HostName vps.example.test\n"),
            "{config}"
        );
        let include = format!("Include {}/.ssh/config", h.home.path().display());
        assert!(config.contains(&include), "{config}");
        assert!(
            h.recorded("argv").contains(" -- tyba-test-"),
            "{}",
            h.recorded("argv")
        );
        h.assert_clean();
    }

    #[cfg(unix)]
    #[test]
    fn agente_oferece_so_a_chave_escolhida_por_um_pub_temporario() {
        let h = Harness::new("exit 0");
        let mut input = form(AuthMethod::Agent);
        input.username = Some("root".into());
        let key = crate::ssh::agent_keys::tests::synthetic_key(4);
        input.agent_key = Some(crate::ssh::AgentKey {
            fingerprint: crate::ssh::agent_keys::fingerprint_of(&key).unwrap(),
            public_key: key.clone(),
            name: "Chave".into(),
        });
        let outcome = h.run(&input, Duration::from_secs(10));
        assert!(
            matches!(&outcome, ConnectionTest::Ok { user, .. } if user == "root"),
            "{outcome:?}"
        );
        assert!(h.recorded("config").contains("IdentitiesOnly yes"));
        assert_eq!(h.recorded("pubs"), format!("{key} Chave\n"));
        h.assert_clean();
    }

    #[cfg(unix)]
    #[test]
    fn falha_limpa_tudo_e_devolve_o_motivo() {
        let h = Harness::new(
            "echo 'root@vps.example.test: Permission denied (publickey).' >&2\nexit 255",
        );
        let outcome = h.run(&form(AuthMethod::Auto), Duration::from_secs(10));
        assert!(
            matches!(
                outcome,
                ConnectionTest::Failed {
                    reason: FailureReason::AuthRefused,
                    ..
                }
            ),
            "{outcome:?}"
        );
        h.assert_clean();
    }

    #[cfg(unix)]
    #[test]
    fn prazo_estourado_mata_o_ssh_e_limpa_tudo() {
        let h = Harness::new("sleep 30");
        let started = std::time::Instant::now();
        let outcome = h.run(&form(AuthMethod::Auto), Duration::from_millis(300));
        assert_eq!(outcome, ConnectionTest::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
        h.assert_clean();
    }

    #[test]
    fn formulario_invalido_e_recusado_antes_de_rodar() {
        let code = |input: HostInput| validate(&input).unwrap_err().code;
        let mut input = form(AuthMethod::Auto);
        input.alias = "-oProxyCommand=id".into();
        assert_eq!(code(input), "ssh.alias_invalid");
        let mut input = form(AuthMethod::Auto);
        input.hostname = "vps\n    ProxyCommand id".into();
        assert_eq!(code(input), "ssh.field_invalid");
        assert_eq!(code(form(AuthMethod::File)), "ssh.identity_file_required");
        assert_eq!(code(form(AuthMethod::Agent)), "ssh.agent_key_invalid");
        assert!(validate(&form(AuthMethod::Password)).is_ok());
    }

    #[test]
    fn formulario_com_campos_de_outro_metodo_e_recusado_como_conflito() {
        let mut input = form(AuthMethod::Password);
        input.identity_file = Some("~/.ssh/id_ed25519".into());
        assert_eq!(
            validate(&input).unwrap_err().code,
            "ssh.auth_fields_conflict"
        );
    }

    #[test]
    fn desfecho_serializa_com_a_tag_outcome() {
        let json = serde_json::to_value(ConnectionTest::Failed {
            reason: FailureReason::NoRoute,
            detail: "x".into(),
        })
        .unwrap();
        assert_eq!(json["outcome"], "failed");
        assert_eq!(json["reason"], "no_route");
        let json = serde_json::to_value(ConnectionTest::PasswordNotOffered {
            methods: vec!["publickey".into()],
        })
        .unwrap();
        assert_eq!(json["outcome"], "password_not_offered");
        assert_eq!(
            serde_json::to_value(ConnectionTest::TimedOut).unwrap()["outcome"],
            "timed_out"
        );
    }

    #[test]
    fn argv_de_senha_nao_oferece_chave_nem_senha() {
        let opts = pairs(&argv(
            Path::new("/c"),
            "tyba-test-abc",
            AuthMethod::Password,
        ));
        assert!(opts.iter().any(|o| o == "PreferredAuthentications=none"));
        assert!(opts.iter().any(|o| o == "PubkeyAuthentication=no"));
        assert!(opts.iter().any(|o| o == "BatchMode=yes"));
    }
}
