//! PATH do shell de login do usuário.
//!
//! Um app lançado pelo Dock/Finder herda o PATH do launchd
//! (`/usr/bin:/bin:/usr/sbin:/sbin`), não o do shell — e os binários de agente
//! moram em `~/.local/bin`, `~/.vite-plus/bin`, `/opt/homebrew/bin` e afins.
//! Sem isso, nenhum agente sobe fora do `tauri dev`.
//!
//! Resolvido uma vez, sob demanda, perguntando ao shell de login do próprio
//! usuário (mesma estratégia de VSCode/Warp). Falhou? Cai no PATH do processo.

use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);
const MARKER: &str = "__TYBA_PATH__";

static RESOLVED: OnceLock<Option<String>> = OnceLock::new();

/// O PATH a usar para achar e spawnar binários de agente.
pub fn agent_path() -> String {
    let login = RESOLVED.get_or_init(resolve_login_path).clone();
    match login {
        Some(path) => path,
        None => std::env::var("PATH").unwrap_or_default(),
    }
}

fn resolve_login_path() -> Option<String> {
    let shell = std::env::var("SHELL").ok()?;
    if !std::path::Path::new(&shell).is_absolute() {
        return None;
    }
    let mut child = Command::new(&shell)
        .arg("-lic")
        .arg(format!("printf '{MARKER}%s{MARKER}' \"$PATH\""))
        .env("TYBA_RESOLVING_PATH", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let deadline = std::time::Instant::now() + RESOLVE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => return None,
        }
    }

    let out = child.wait_with_output().ok()?;
    parse_marked_path(&String::from_utf8_lossy(&out.stdout))
}

/// O rc do usuário pode imprimir qualquer coisa (banner, fastfetch, aviso de
/// update). Os marcadores isolam o PATH desse ruído.
fn parse_marked_path(stdout: &str) -> Option<String> {
    let (_, rest) = stdout.split_once(MARKER)?;
    let (path, _) = rest.split_once(MARKER)?;
    let path = path.trim();
    (!path.is_empty()).then(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
