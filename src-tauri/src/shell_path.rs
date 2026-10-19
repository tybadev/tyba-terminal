//! PATH do shell de login do usuário.
//!
//! Um app lançado pelo Dock/Finder herda o PATH do launchd
//! (`/usr/bin:/bin:/usr/sbin:/sbin`), não o do shell — e os binários de agente
//! moram em `~/.local/bin`, `~/.vite-plus/bin`, `/opt/homebrew/bin` e afins.
//! Sem isso, nenhum agente sobe fora do `tauri dev`.
//!
//! Resolvido uma vez, sob demanda, perguntando ao shell de login do próprio
//! usuário (mesma estratégia de VSCode/Warp). Falhou? Cai no PATH do processo.

//!
//! A mesma invocação devolve o `SSH_AUTH_SOCK` do login shell: é por onde o
//! `ssh` do core alcança o agente que o dono configurou no rc (1Password,
//! Secretive), e sem ele o app lançado pelo Dock só vê o agente do launchd.
//!
//! E devolve o env COMPLETO do login shell (`env -0`), que só vai para o
//! agente livre que o TYBA abre (agente livre: o env do agente é o do shell
//! de login do dono — `ANTHROPIC_API_KEY`, `AWS_PROFILE`, `CLAUDE_CONFIG_DIR`,
//! proxy). O rc roda fora de qualquer repositório: o que é por pasta
//! (direnv, `.nvmrc`) não entra.

use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);
const MARKER: &str = "__TYBA_PATH__";
const SOCK_MARKER: &str = "__TYBA_SOCK__";
const ENV_MARKER: &str = "__TYBA_ENV__";
/// Marca a invocação de resolução; não vai para o env do agente.
const RESOLVING_VAR: &str = "TYBA_RESOLVING_PATH";

/// O que o login shell do dono tem e o processo do app pode não ter. `path` e
/// `ssh_auth_sock` servem o core; `vars` (o env inteiro) só vai para o agente
/// livre que o TYBA abre.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoginEnv {
    pub path: Option<String>,
    pub ssh_auth_sock: Option<String>,
    pub vars: Option<Vec<(String, String)>>,
}

/// Só o sucesso fica em cache. Falha ou prazo estourado tentam de novo depois
/// de [`RETRY_AFTER`] — na próxima sessão, não a cada chamada (o PATH é lido a
/// toda hora).
static RESOLVED: Mutex<Option<LoginEnv>> = Mutex::new(None);
static LAST_FAILURE: Mutex<Option<Instant>> = Mutex::new(None);
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// Variáveis do login shell que não vão para o agente: a pasta onde o rc
/// rodou não é a do agente.
const NOT_FOR_AGENT: &[&str] = &["PWD", "OLDPWD"];

/// O PATH a usar para achar e spawnar binários de agente.
pub fn agent_path() -> String {
    login_env()
        .path
        .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default())
}

/// O mesmo PATH de [`agent_path`], mas só se já foi resolvido — nunca dispara
/// o `$SHELL -lic` (até 3 s). Para quem não pode esperar, como o spawn de um
/// pane.
pub fn agent_path_if_resolved() -> Option<String> {
    let tried = RESOLVED.lock().map(|r| r.is_some()).unwrap_or(false)
        || LAST_FAILURE.lock().map(|f| f.is_some()).unwrap_or(false);
    tried.then(agent_path)
}

/// Tenta resolver de novo agora? Só passado [`RETRY_AFTER`] da última falha.
fn should_retry(last_failure: Option<Instant>, now: Instant) -> bool {
    last_failure.is_none_or(|at| now.saturating_duration_since(at) >= RETRY_AFTER)
}

/// O env do login shell: do cache, ou resolvido agora (sem cache se falhar).
fn resolved() -> Option<LoginEnv> {
    let mut cached = RESOLVED.lock().unwrap_or_else(|e| e.into_inner());
    if cached.is_some() {
        return cached.clone();
    }
    let mut failure = LAST_FAILURE.lock().unwrap_or_else(|e| e.into_inner());
    if !should_retry(*failure, Instant::now()) {
        return None;
    }
    match resolve_login_env() {
        Some(env) => {
            *cached = Some(env.clone());
            *failure = None;
            Some(env)
        }
        None => {
            *failure = Some(Instant::now());
            None
        }
    }
}

/// O env completo do shell de login, resolvido uma vez (até 3 s, com cache).
/// `None` quando o shell não respondeu: quem chama cai no env do processo.
pub fn login_shell_vars() -> Option<std::collections::HashMap<String, String>> {
    let vars = resolved()?.vars?;
    Some(
        vars.into_iter()
            .filter(|(k, _)| k != RESOLVING_VAR && !NOT_FOR_AGENT.contains(&k.as_str()))
            .collect(),
    )
}

/// Resolvido uma vez. Variável que o login shell não trouxe cai na do processo.
pub fn login_env() -> LoginEnv {
    with_process_fallback(resolved(), |name| std::env::var(name).ok())
}

fn with_process_fallback(
    resolved: Option<LoginEnv>,
    process: impl Fn(&str) -> Option<String>,
) -> LoginEnv {
    let resolved = resolved.unwrap_or_default();
    let non_empty = |v: Option<String>| v.filter(|v| !v.is_empty());
    LoginEnv {
        path: non_empty(resolved.path).or_else(|| non_empty(process("PATH"))),
        ssh_auth_sock: non_empty(resolved.ssh_auth_sock)
            .or_else(|| non_empty(process("SSH_AUTH_SOCK"))),
        vars: resolved.vars,
    }
}

fn resolve_login_env() -> Option<LoginEnv> {
    let shell = std::env::var("SHELL").ok()?;
    resolve_login_env_with(Path::new(&shell), RESOLVE_TIMEOUT)
}

fn resolve_login_env_with(shell: &Path, timeout: Duration) -> Option<LoginEnv> {
    if !shell.is_absolute() {
        return None;
    }
    let mut child = Command::new(shell)
        .arg("-lic")
        .arg(format!(
            "printf '{MARKER}%s{MARKER}{SOCK_MARKER}%s{SOCK_MARKER}' \"$PATH\" \"$SSH_AUTH_SOCK\"; \
             printf '{ENV_MARKER}'; env -0 2>/dev/null; printf '{ENV_MARKER}'"
        ))
        .env(RESOLVING_VAR, "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    // Lê a saída enquanto espera: com o env inteiro passando de 64 KB, o
    // pipe enche e o shell trava escrevendo se ninguém lê.
    let mut pipe = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut out = Vec::new();
        let _ = pipe.read_to_end(&mut out);
        let _ = tx.send(out);
    });
    let Ok(out) = rx.recv_timeout(timeout) else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let _ = child.wait();
    let stdout = String::from_utf8_lossy(&out);
    let path = parse_marked_path(&stdout)?;
    Some(LoginEnv {
        path: Some(path),
        ssh_auth_sock: parse_between(&stdout, SOCK_MARKER),
        vars: parse_env_block(&stdout),
    })
}

/// O `env -0` entre os marcadores: `NOME=valor` separados por NUL. Bloco
/// vazio ou ausente (shell sem `env -0`) é `None`.
fn parse_env_block(stdout: &str) -> Option<Vec<(String, String)>> {
    let (_, rest) = stdout.split_once(ENV_MARKER)?;
    let (block, _) = rest.rsplit_once(ENV_MARKER)?;
    let vars: Vec<(String, String)> = block
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .filter(|(k, _)| !k.is_empty() && !k.contains(['\n', ' ']))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    (!vars.is_empty()).then_some(vars)
}

/// O rc do usuário pode imprimir qualquer coisa (banner, fastfetch, aviso de
/// update). Os marcadores isolam o PATH desse ruído.
fn parse_marked_path(stdout: &str) -> Option<String> {
    parse_between(stdout, MARKER)
}

fn parse_between(stdout: &str, marker: &str) -> Option<String> {
    let (_, rest) = stdout.split_once(marker)?;
    let (value, _) = rest.split_once(marker)?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn fake_login_shell(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let shell = dir.join("fake-shell");
        std::fs::write(
            &shell,
            format!(
                "#!/bin/sh\necho call >> \"{}/calls\"\n{body}\n",
                dir.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        shell
    }

    #[cfg(unix)]
    #[test]
    fn uma_invocacao_do_login_shell_devolve_path_e_socket_do_agente() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_login_shell(
            dir.path(),
            "echo 'banner do rc'\n\
             PATH=/opt/fake/bin:/usr/bin SSH_AUTH_SOCK='/tmp/agent dir/agent.sock'\n\
             SYNTH_ONLY_IN_RC='valor com espaço
\
e linha'\n\
             export PATH SSH_AUTH_SOCK SYNTH_ONLY_IN_RC\n\
             eval \"$2\"",
        );
        let env = resolve_login_env_with(&shell, Duration::from_secs(3)).expect("resolveu");
        assert_eq!(env.path.as_deref(), Some("/opt/fake/bin:/usr/bin"));
        let vars: std::collections::HashMap<_, _> =
            env.vars.clone().expect("env").into_iter().collect();
        assert_eq!(vars["PATH"], "/opt/fake/bin:/usr/bin");
        assert_eq!(vars["SYNTH_ONLY_IN_RC"], "valor com espaço\ne linha");
        assert_eq!(
            env.ssh_auth_sock.as_deref(),
            Some("/tmp/agent dir/agent.sock")
        );
        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        assert_eq!(
            calls.lines().count(),
            1,
            "um shell de login só, para as duas variáveis"
        );
    }

    #[cfg(unix)]
    #[test]
    fn login_shell_que_trava_ou_nao_responde_cai_no_env_do_processo() {
        let dir = tempfile::tempdir().unwrap();
        let hangs = fake_login_shell(dir.path(), "sleep 5");
        let started = std::time::Instant::now();
        assert_eq!(
            resolve_login_env_with(&hangs, Duration::from_millis(300)),
            None
        );
        assert!(started.elapsed() < Duration::from_secs(2), "o prazo manda");
        let mute = fake_login_shell(dir.path(), "echo sem marcadores");
        assert_eq!(resolve_login_env_with(&mute, Duration::from_secs(3)), None);

        let process = |name: &str| match name {
            "PATH" => Some("/usr/bin:/bin".to_string()),
            "SSH_AUTH_SOCK" => Some("/private/tmp/launchd/Listeners".to_string()),
            _ => None,
        };
        let env = with_process_fallback(None, process);
        assert_eq!(env.path.as_deref(), Some("/usr/bin:/bin"));
        assert_eq!(
            env.ssh_auth_sock.as_deref(),
            Some("/private/tmp/launchd/Listeners")
        );
        let partial = with_process_fallback(
            Some(LoginEnv {
                path: Some("/opt/fake/bin".into()),
                ssh_auth_sock: None,
                vars: None,
            }),
            process,
        );
        assert_eq!(partial.path.as_deref(), Some("/opt/fake/bin"));
        assert_eq!(
            partial.ssh_auth_sock.as_deref(),
            Some("/private/tmp/launchd/Listeners"),
            "login shell sem agente não apaga o agente do processo"
        );
    }

    /// Env grande (> 64 KB) não trava a leitura.
    #[cfg(unix)]
    #[test]
    fn env_grande_nao_trava_o_login_shell() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_login_shell(
            dir.path(),
            "BIG=$(head -c 200000 /dev/zero | tr '\\0' x)\n\
             export BIG\n\
             eval \"$2\"",
        );
        let env = resolve_login_env_with(&shell, Duration::from_secs(5)).expect("resolveu");
        let vars: std::collections::HashMap<_, _> = env.vars.expect("env").into_iter().collect();
        assert_eq!(vars["BIG"].len(), 200_000);
    }

    #[test]
    fn falha_nao_fica_em_cache_mas_espera_um_pouco_para_tentar_de_novo() {
        let now = Instant::now();
        assert!(should_retry(None, now));
        assert!(!should_retry(Some(now), now + Duration::from_secs(5)));
        assert!(should_retry(Some(now), now + RETRY_AFTER));
    }

    #[test]
    fn extrai_o_path_entre_os_marcadores() {
        let out = format!("banner do rc\n{MARKER}/usr/bin:/opt/homebrew/bin{MARKER}");
        assert_eq!(
            parse_marked_path(&out).as_deref(),
            Some("/usr/bin:/opt/homebrew/bin")
        );
    }

    #[test]
    fn ignora_ruido_depois_do_path() {
        let out = format!("{MARKER}/bin{MARKER}\nnovo update disponível!");
        assert_eq!(parse_marked_path(&out).as_deref(), Some("/bin"));
    }

    #[test]
    fn sem_marcadores_ou_vazio_devolve_none() {
        assert!(parse_marked_path("/usr/bin:/bin").is_none());
        assert!(parse_marked_path(&format!("{MARKER}{MARKER}")).is_none());
        assert!(parse_marked_path(&format!("{MARKER}   {MARKER}")).is_none());
        assert!(parse_marked_path(&format!("só um {MARKER} marcador")).is_none());
    }

    #[test]
    fn agent_path_nunca_e_vazio_quando_o_processo_tem_path() {
        assert!(!agent_path().is_empty());
    }
}

#[cfg(test)]
mod launchd_regression {
    /// Reproduz o cenário do app lançado pelo Dock: PATH do launchd, onde
    /// nenhum binário de agente existe. A detecção precisa achá-los mesmo assim.
    #[test]
    #[ignore = "depende dos agentes instalados na máquina do dev"]
    fn acha_os_agentes_mesmo_com_o_path_do_launchd() {
        use crate::agent::{binary_available, runner_binary};
        use crate::session::AgentRunnerKind;

        let launchd = "/usr/bin:/bin:/usr/sbin:/sbin";
        assert!(
            !std::env::split_paths(launchd)
                .any(|d| d.join("claude").exists() || d.join("codex").exists()),
            "pré-condição: os agentes não moram no PATH do launchd"
        );

        for kind in [AgentRunnerKind::ClaudeCode, AgentRunnerKind::Codex] {
            let binary = runner_binary(&kind).unwrap();
            assert!(
                binary_available(&kind),
                "`{binary}` não foi encontrado — o app lançado pelo Dock não conseguiria subir esse agente"
            );
        }
    }
}
