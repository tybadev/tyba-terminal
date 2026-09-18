//! Costuras da conexão SSH contra um host REAL, pela API pública.
//!
//! Todos os casos são `#[ignore]` e conectam de verdade num host alcançável:
//! rodam só com `cargo test --test ssh_real_host -- --ignored` e com o ambiente
//! abaixo. Sem ele, cada caso avisa no stderr e sai cedo, sem falhar.
//!
//! - `TYBA_E2E_SSH_ALIAS` — alias do `~/.ssh/config` que alcança o host.
//! - `TYBA_E2E_SSH_HOSTNAME` — endereço do host (vai no formulário do teste de conexão).
//! - `TYBA_E2E_SSH_USER` — usuário válido no host.
//! - `TYBA_E2E_SSH_AGENT_KEY_NAME` — comentário da chave, na listagem do agente,
//!   que o host aceita para esse usuário.
//! - `TYBA_E2E_SSH_IDENTITY_AGENT` (opcional) — socket do agente, passado como
//!   `IdentityAgent` nas conexões montadas aqui (não no teste de conexão, que usa
//!   só o formulário mais o `~/.ssh/config`).
//!
//! O agente pode pedir aprovação na tela (o do 1Password pede): o dono tem de
//! estar presente. Nenhum endereço, usuário ou chave real mora neste arquivo.
#![cfg(unix)]

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tyba_lib::session::cano::{CanoOutcome, CanoWatch};
use tyba_lib::ssh::classify::{classify, FailureReason};
use tyba_lib::ssh::test_conn::{self, ConnectionTest, TEST_DEADLINE};
use tyba_lib::ssh::tmux::{
    has_session_command, interpret_has_session, kill_command, wrap_command_with_nonce, Probe,
};
use tyba_lib::ssh::{agent_keys, AgentKey, AuthMethod, HostInput};

struct E2eEnv {
    alias: String,
    hostname: String,
    user: String,
    identity_agent: Option<String>,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn e2e_env(case: &str) -> Option<E2eEnv> {
    let found = (|| {
        Some(E2eEnv {
            alias: var("TYBA_E2E_SSH_ALIAS")?,
            hostname: var("TYBA_E2E_SSH_HOSTNAME")?,
            user: var("TYBA_E2E_SSH_USER")?,
            identity_agent: var("TYBA_E2E_SSH_IDENTITY_AGENT"),
        })
    })();
    if found.is_none() {
        eprintln!(
            "{case}: pulado — defina TYBA_E2E_SSH_ALIAS, TYBA_E2E_SSH_HOSTNAME e \
             TYBA_E2E_SSH_USER para conectar num host real"
        );
    }
    found
}

/// Um usuário que não existe no host: Linux diferencia maiúscula de minúscula.
fn wrong_user(user: &str) -> String {
    let mut chars = user.chars();
    let first = chars.next().unwrap_or('x');
    let rest: String = chars.collect();
    let upper: String = first.to_uppercase().chain(rest.chars()).collect();
    if upper != user {
        return upper;
    }
    let lower: String = first.to_lowercase().chain(rest.chars()).collect();
    if lower != user {
        return lower;
    }
    format!("X{user}")
}

fn nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// O `ssh` do core com as opções do Cano, sem multiplexação: uma conexão
/// reaproveitada de um master já aberto pularia a autenticação que o caso mede.
/// `flags` entram antes do destino: depois dele o `ssh` as leria como comando remoto.
fn ssh_to(env: &E2eEnv, user: &str, flags: &[&str]) -> Command {
    let mut cmd = tyba_lib::ssh::command::std_command();
    cmd.args(flags);
    for opt in [
        "BatchMode=yes",
        "ConnectTimeout=10",
        "ControlMaster=no",
        "ControlPath=none",
    ] {
        cmd.arg("-o").arg(opt);
    }
    if let Some(agent) = &env.identity_agent {
        cmd.arg("-o").arg(format!("IdentityAgent=\"{agent}\""));
    }
    cmd.args(["-l", user, &env.alias]);
    cmd
}

fn remote_probe(env: &E2eEnv, name: &str) -> Probe {
    let status = ssh_to(env, &env.user, &[])
        .arg(has_session_command(name))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(s) => interpret_has_session(s.code()),
        Err(_) => Probe::Unknown,
    }
}

fn remote_kill(env: &E2eEnv, name: &str) {
    let _ = ssh_to(env, &env.user, &[])
        .arg(kill_command(name))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Cano de teste: o `ssh` num grupo próprio e a saída crua (stdout e stderr,
/// como o PTY entrega) num canal só.
struct Cano {
    child: Child,
    // Segurado aberto: EOF no stdin encerraria a sessão remota antes da hora.
    _stdin: Option<ChildStdin>,
    output: mpsc::Receiver<Vec<u8>>,
}

impl Cano {
    fn spawn(mut cmd: Command) -> Cano {
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = cmd.spawn().expect("o ssh sobe");
        let (tx, output) = mpsc::channel();
        for stream in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let tx = tx.clone();
            std::thread::spawn(move || pump(stream, tx));
        }
        let stdin = child.stdin.take();
        Cano {
            child,
            _stdin: stdin,
            output,
        }
    }

    /// Alimenta o observador até o marco, o fim da saída ou o prazo.
    fn watch_until_login(&self, watch: &mut CanoWatch, deadline: Duration) -> bool {
        let until = Instant::now() + deadline;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(left) {
                Ok(chunk) if watch.feed(&chunk) => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    }

    fn kill(&mut self) {
        if let Ok(pid) = libc::pid_t::try_from(self.child.id()) {
            // SAFETY: sinal para o grupo que este teste criou com `process_group(0)`.
            unsafe {
                libc::killpg(pid, libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Cano {
    fn drop(&mut self) {
        self.kill();
    }
}

fn pump(mut stream: Box<dyn Read + Send>, tx: mpsc::Sender<Vec<u8>>) {
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if tx.send(buf[..n].to_vec()).is_err() {
                    return;
                }
            }
        }
    }
}

/// Mata a sessão tmux do teste no host em qualquer caminho, pânico incluído.
struct RemoteSession<'a> {
    env: &'a E2eEnv,
    name: String,
}

impl Drop for RemoteSession<'_> {
    fn drop(&mut self) {
        remote_kill(self.env, &self.name);
    }
}

#[test]
#[ignore = "conecta num host real: exige TYBA_E2E_SSH_ALIAS, TYBA_E2E_SSH_HOSTNAME e TYBA_E2E_SSH_USER"]
fn login_concluido_no_host_real_emite_o_marco_do_nonce() {
    let Some(env) = e2e_env("login_concluido") else {
        return;
    };
    let nonce = nonce();
    let remote = RemoteSession {
        env: &env,
        name: format!("tyba-e2e-{}", &nonce[..12]),
    };
    let mut cmd = ssh_to(&env, &env.user, &["-tt"]);
    // O wrap faz `exec tmux new-session -A`, que exige terminal; o TERM é o
    // que o PTY do app entrega, e não o do processo que roda o `cargo test`.
    cmd.env("TERM", "xterm-256color")
        .arg(wrap_command_with_nonce(&remote.name, &nonce));
    let mut cano = Cano::spawn(cmd);
    let mut watch = CanoWatch::new(&nonce);

    let seen = cano.watch_until_login(&mut watch, TEST_DEADLINE);
    // O marco sai antes do `exec tmux`: esperar a primeira saída depois dele
    // dá tempo de a sessão existir, e o kill remoto abaixo tem o que matar.
    if seen {
        let _ = cano.output.recv_timeout(Duration::from_secs(5));
    }
    cano.kill();
    assert!(seen, "o marco do nonce não chegou em {TEST_DEADLINE:?}");
    assert_eq!(watch.finish(), CanoOutcome::LoggedIn);

    remote_kill(&env, &remote.name);
    assert!(
        matches!(
            remote_probe(&env, &remote.name),
            Probe::Gone | Probe::NoTmux
        ),
        "a sessão tmux {} ficou viva no host",
        remote.name
    );
}

#[test]
#[ignore = "conecta num host real: exige TYBA_E2E_SSH_ALIAS, TYBA_E2E_SSH_HOSTNAME e TYBA_E2E_SSH_USER"]
fn usuario_errado_sai_sem_marco_e_classificado_como_auth_refused() {
    let Some(env) = e2e_env("usuario_errado") else {
        return;
    };
    let nonce = nonce();
    let remote = RemoteSession {
        env: &env,
        name: format!("tyba-e2e-{}", &nonce[..12]),
    };
    let mut cmd = ssh_to(&env, &wrong_user(&env.user), &["-tt"]);
    cmd.env("TERM", "xterm-256color")
        .arg(wrap_command_with_nonce(&remote.name, &nonce));
    let mut cano = Cano::spawn(cmd);
    let mut watch = CanoWatch::new(&nonce);

    let seen = cano.watch_until_login(&mut watch, TEST_DEADLINE);
    // Pipes fechados não garantem o processo já colhido.
    let reaped_by = Instant::now() + Duration::from_secs(5);
    let exited = loop {
        match cano.child.try_wait() {
            Ok(None) if Instant::now() < reaped_by => std::thread::sleep(Duration::from_millis(20)),
            other => break other.ok().flatten(),
        }
    };
    cano.kill();
    assert!(!seen, "usuário inexistente não pode concluir login");
    assert!(
        exited.is_some(),
        "o ssh tinha de sair sozinho antes do prazo"
    );

    let CanoOutcome::NotLoggedIn { tail } = watch.finish() else {
        panic!("sem marco não há login");
    };
    let failure = classify(&String::from_utf8_lossy(&tail));
    assert_eq!(
        failure.reason,
        FailureReason::AuthRefused,
        "saída pré-login: {}",
        failure.detail
    );
}

fn form(env: &E2eEnv, user: &str, method: AuthMethod, agent_key: Option<AgentKey>) -> HostInput {
    HostInput {
        alias: "tyba-e2e-form".into(),
        hostname: env.hostname.clone(),
        port: None,
        username: Some(user.to_string()),
        identity_file: None,
        proxy_jump: None,
        group_id: None,
        color: None,
        notes: None,
        tunnels: Vec::new(),
        auth_method: method,
        agent_key,
    }
}

fn chosen_agent_key(env: &E2eEnv) -> Option<AgentKey> {
    let Some(name) = var("TYBA_E2E_SSH_AGENT_KEY_NAME") else {
        eprintln!("teste_de_conexao: pulado — defina TYBA_E2E_SSH_AGENT_KEY_NAME");
        return None;
    };
    let listing = agent_keys::list(Some(&env.alias)).expect("o agente responde a listagem");
    let key = listing.keys.into_iter().find(|k| k.name == name);
    assert!(
        key.is_some(),
        "o agente não lista uma chave chamada {name:?}"
    );
    key
}

#[test]
#[ignore = "conecta num host real: exige TYBA_E2E_SSH_ALIAS, TYBA_E2E_SSH_HOSTNAME, TYBA_E2E_SSH_USER e TYBA_E2E_SSH_AGENT_KEY_NAME"]
fn teste_de_conexao_no_host_real_distingue_ok_recusa_e_senha() {
    let Some(env) = e2e_env("teste_de_conexao") else {
        return;
    };
    let Some(key) = chosen_agent_key(&env) else {
        return;
    };

    let ok = test_conn::run(
        &form(&env, &env.user, AuthMethod::Agent, Some(key.clone())),
        TEST_DEADLINE,
    );
    assert!(
        matches!(&ok, ConnectionTest::Ok { user, .. } if *user == env.user),
        "{ok:?}"
    );

    let refused = test_conn::run(
        &form(&env, &wrong_user(&env.user), AuthMethod::Agent, Some(key)),
        TEST_DEADLINE,
    );
    assert!(
        matches!(
            refused,
            ConnectionTest::Failed {
                reason: FailureReason::AuthRefused,
                ..
            }
        ),
        "{refused:?}"
    );

    let password = test_conn::run(
        &form(&env, &env.user, AuthMethod::Password, None),
        TEST_DEADLINE,
    );
    // `Ok` aqui seria o servidor aceitando algo que a sonda de senha não pode
    // oferecer: nem chave nem senha saem, só a lista de métodos volta.
    assert!(
        matches!(
            password,
            ConnectionTest::PasswordAccepted { .. } | ConnectionTest::PasswordNotOffered { .. }
        ),
        "{password:?}"
    );
}

#[test]
fn usuario_errado_difere_do_real_so_na_caixa() {
    assert_eq!(wrong_user("operador"), "Operador");
    assert_eq!(wrong_user("Operador"), "operador");
    assert_eq!(wrong_user("1op"), "X1op");
}
