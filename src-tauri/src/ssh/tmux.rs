use std::collections::HashSet;
use std::time::Duration;

use uuid::Uuid;

use crate::session::store::{Store, StoreError};

pub const INSTALL_ID_KEY: &str = "ssh.install_id";

pub fn install_id(store: &Store) -> Result<String, StoreError> {
    if let Some(existing) = store.get_setting(INSTALL_ID_KEY)? {
        if !existing.trim().is_empty() {
            return Ok(existing);
        }
    }
    let fresh = Uuid::new_v4().simple().to_string()[..12].to_string();
    store.set_setting(INSTALL_ID_KEY, &fresh)?;
    Ok(fresh)
}

pub fn session_name(install_id: &str, session_id: Uuid) -> String {
    format!("tyba-{install_id}-{}", session_id.simple())
}

fn sh_c(script: &str) -> String {
    assert!(
        !script.contains('\''),
        "aspas simples fecham o envelope: {script}"
    );
    format!("sh -c '{script}'")
}

pub fn wrap_command(name: &str) -> String {
    sh_c(&format!(
        "command -v tmux >/dev/null 2>&1 && \
         exec tmux new-session -A -s {name} \"exec env -u TMUX \\\"${{SHELL:-/bin/sh}}\\\" -l\" \\; \
         set-option -t {name} status off \\; \
         set-option -t {name} prefix None \\; \
         set-option -t {name} history-limit 5000 || \
         exec \"${{SHELL:-/bin/sh}}\" -l"
    ))
}

pub fn has_session_command(name: &str) -> String {
    sh_c(&format!("tmux has-session -t {name}"))
}

pub fn list_sessions_command() -> String {
    sh_c("tmux ls -F \"#{session_name}\" 2>/dev/null; true")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Alive,
    Gone,
    NoTmux,
    Unknown,
}

pub fn interpret_has_session(exit_code: Option<i32>) -> Probe {
    match exit_code {
        Some(0) => Probe::Alive,
        Some(1) => Probe::Gone,
        Some(127) => Probe::NoTmux,
        _ => Probe::Unknown,
    }
}

impl Probe {
    pub fn should_reattach(self) -> bool {
        matches!(self, Probe::Alive | Probe::Unknown)
    }
}

const BACKOFF_CEILING: Duration = Duration::from_secs(30);
const GIVE_UP_AFTER: Duration = Duration::from_secs(300);

pub fn retry_delay(attempt: u32) -> Option<Duration> {
    let delay = Duration::from_secs(1u64 << attempt.min(5));
    let delay = delay.min(BACKOFF_CEILING);
    (elapsed_before(attempt) < GIVE_UP_AFTER).then_some(delay)
}

fn elapsed_before(attempt: u32) -> Duration {
    (0..attempt)
        .map(|a| Duration::from_secs(1u64 << a.min(5)).min(BACKOFF_CEILING))
        .sum()
}

pub fn orphans(listed: &[String], install_id: &str, known: &HashSet<Uuid>) -> Vec<String> {
    let prefix = format!("tyba-{install_id}-");
    listed
        .iter()
        .filter(|name| name.starts_with(&prefix))
        .filter(|name| match Uuid::parse_str(&name[prefix.len()..]) {
            Ok(id) => !known.contains(&id),
            Err(_) => false,
        })
        .cloned()
        .collect()
}

pub fn probe(alias: &str, name: &str) -> Probe {
    let status = std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", alias])
        .arg(has_session_command(name))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(s) => interpret_has_session(s.code()),
        Err(_) => Probe::Unknown,
    }
}

pub fn kill_remote(alias: &str, name: &str) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", alias])
        .arg(kill_command(name))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
}

pub fn list_sessions(alias: &str) -> Vec<String> {
    let out = std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", alias])
        .arg(list_sessions_command())
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub fn collect_orphans(alias: &str, install_id: &str, known: &HashSet<Uuid>) -> Vec<String> {
    let listed = list_sessions(alias);
    let doomed = orphans(&listed, install_id, known);
    for name in &doomed {
        let _ = kill_remote(alias, name);
    }
    doomed
}

pub fn kill_command(name: &str) -> String {
    sh_c(&format!(
        "tmux kill-session -t {name} 2>/dev/null; \
         pkill -f \"[t]mux new-session -A -s {name}\" 2>/dev/null; \
         true"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn kill_mata_a_sessao_e_o_cliente_orfao() {
        let name = session_name("a3f", uuid(0x9f3a));
        let cmd = kill_command(&name);
        assert!(
            cmd.contains(&format!("kill-session -t {name}")),
            "got: {cmd}"
        );
        assert!(
            cmd.contains(&format!("pkill -f \"[t]mux new-session -A -s {name}\"")),
            "o cliente vira órfão do init quando o cano morre; kill-session sozinho \
             não o alcança se ele já ficou pendurado antes: {cmd}"
        );
        assert!(
            cmd.ends_with("true'"),
            "não achar o que matar é sucesso, não erro: {cmd}"
        );
    }

    #[test]
    fn kill_e_especifico_da_sessao_nunca_do_servidor() {
        let cmd = kill_command(&session_name("a3f", uuid(0x9f3a)));
        assert!(
            !cmd.contains("kill-server"),
            "kill-server derrubaria as sessões do dono e as de outras instalações: {cmd}"
        );
    }

    #[test]
    fn kill_nao_pode_matar_o_shell_que_o_executa() {
        let cmd = kill_command(&session_name("a3f", uuid(0x9f3a)));
        assert!(
            !cmd.contains("pkill -f \"tmux"),
            "medido na VPS: sem o [t] o pkill casa a linha do bash -c que o roda, \
             mata o próprio pai, o true não executa e o ssh volta 255: {cmd}"
        );
        assert!(cmd.contains("pkill -f \"[t]mux"), "got: {cmd}");
    }

    #[test]
    fn install_id_e_estavel_entre_chamadas() {
        let store = Store::open_in_memory().unwrap();
        let first = install_id(&store).unwrap();
        let second = install_id(&store).unwrap();
        assert_eq!(first, second, "install_id não pode mudar entre boots");
        assert!(!first.is_empty());
    }

    #[test]
    fn install_id_difere_entre_instalacoes() {
        let laptop = Store::open_in_memory().unwrap();
        let desktop = Store::open_in_memory().unwrap();
        assert_ne!(
            install_id(&laptop).unwrap(),
            install_id(&desktop).unwrap(),
            "duas instalações não podem colidir: é o que separa a autoridade do GC"
        );
    }

    #[test]
    fn nome_carrega_instalacao_e_sessao() {
        let name = session_name("a3f", uuid(0x9f3a));
        assert!(name.starts_with("tyba-a3f-"), "got: {name}");
        assert!(name.contains(&uuid(0x9f3a).simple().to_string()));
    }

    #[test]
    fn wrap_cai_no_shell_quando_nao_ha_tmux() {
        let cmd = wrap_command("tyba-a3f-9f3a");
        assert!(cmd.contains("command -v tmux"), "got: {cmd}");
        assert!(
            cmd.ends_with("|| exec \"${SHELL:-/bin/sh}\" -l'"),
            "sem tmux o dono recebe o login shell dele, e mais nada: {cmd}"
        );
    }

    #[test]
    fn wrap_e_invisivel_para_o_dono() {
        let cmd = wrap_command("tyba-a3f-9f3a");
        assert!(cmd.contains("status off"), "status bar é do dono: {cmd}");
        assert!(
            cmd.contains("prefix None"),
            "sem prefixo: o TYBA controla pelo CLI e não disputa Ctrl-B: {cmd}"
        );
    }

    #[test]
    fn wrap_limpa_tmux_no_shell_dentro_do_nosso_tmux() {
        let cmd = wrap_command("tyba-a3f-9f3a");
        assert!(
            cmd.contains("new-session -A -s tyba-a3f-9f3a \"exec env -u TMUX "),
            "verificado na VPS: o env tem que ser o comando do pane, porque é o \
             tmux que seta $TMUX; no fallback não há tmux para setar nada: {cmd}"
        );
        assert!(!cmd.contains("|| exec env -u TMUX"), "got: {cmd}");
    }

    #[test]
    fn wrap_reata_em_vez_de_criar_outra() {
        let cmd = wrap_command("tyba-a3f-9f3a");
        assert!(
            cmd.contains("new-session -A -s tyba-a3f-9f3a"),
            "-A reata a existente; sem ele o reattach criaria uma sessão nova: {cmd}"
        );
    }

    #[test]
    fn has_session_zero_e_sessao_viva() {
        assert_eq!(interpret_has_session(Some(0)), Probe::Alive);
        assert!(interpret_has_session(Some(0)).should_reattach());
    }

    #[test]
    fn has_session_um_e_o_dono_tendo_saido() {
        assert_eq!(interpret_has_session(Some(1)), Probe::Gone);
        assert!(
            !interpret_has_session(Some(1)).should_reattach(),
            "o dono deu exit: reatar seria ressuscitar o que ele encerrou"
        );
    }

    #[test]
    fn has_session_127_e_host_sem_tmux_e_nao_reata() {
        assert_eq!(interpret_has_session(Some(127)), Probe::NoTmux);
        assert!(
            !interpret_has_session(Some(127)).should_reattach(),
            "sem distinguir 127 de 'não sei', um Host vivo e sem tmux reconectaria para sempre"
        );
    }

    #[test]
    fn has_session_indeterminado_reata() {
        for code in [Some(255), Some(2), None] {
            assert_eq!(
                interpret_has_session(code),
                Probe::Unknown,
                "code: {code:?}"
            );
            assert!(
                interpret_has_session(code).should_reattach(),
                "não sei cai para o lado de reatar: errar reatando custa uma tentativa; \
                 errar para o outro lado descarta trabalho vivo"
            );
        }
    }

    #[test]
    fn backoff_dobra_ate_o_teto() {
        let seen: Vec<u64> = (0..6).map(|a| retry_delay(a).unwrap().as_secs()).collect();
        assert_eq!(seen, vec![1, 2, 4, 8, 16, 30]);
    }

    #[test]
    fn backoff_desiste_e_deixa_o_botao() {
        let mut attempt = 0;
        while retry_delay(attempt).is_some() {
            attempt += 1;
            assert!(attempt < 100, "backoff não pode insistir para sempre");
        }
        let total: u64 = elapsed_before(attempt).as_secs();
        assert!(
            (240..=360).contains(&total),
            "desiste perto de ~5min, cobrindo sleep de laptop; got {total}s"
        );
    }

    #[test]
    fn gc_nao_toca_na_sessao_de_outra_instalacao() {
        let viva_do_desktop = session_name("b71", uuid(0x1c8e));
        let listed = vec![viva_do_desktop.clone()];

        let recolher = orphans(&listed, "a3f", &HashSet::new());

        assert!(
            recolher.is_empty(),
            "o laptop não tem autoridade sobre a sessão viva do desktop — \
             o SQLite dele não prova nada sobre ela"
        );
    }

    #[test]
    fn gc_nao_toca_no_tmux_do_dono() {
        let listed = vec!["work".to_string(), "0".to_string(), "tyba".to_string()];
        assert!(orphans(&listed, "a3f", &HashSet::new()).is_empty());
    }

    #[test]
    fn gc_recolhe_orfa_do_proprio_namespace() {
        let orfa = session_name("a3f", uuid(0x9f3a));
        let listed = vec![orfa.clone(), session_name("b71", uuid(0x1c8e))];

        assert_eq!(orphans(&listed, "a3f", &HashSet::new()), vec![orfa]);
    }

    #[test]
    fn gc_poupa_sessao_conhecida_mesmo_morta() {
        let id = uuid(0x9f3a);
        let listed = vec![session_name("a3f", id)];
        let known = HashSet::from([id]);

        assert!(
            orphans(&listed, "a3f", &known).is_empty(),
            "sessão Exited com tmux vivo é o caso de reattach, não órfã: \
             matá-la seria matar a feature"
        );
    }

    #[test]
    fn gc_ignora_nome_que_nao_casa_o_formato() {
        let listed = vec!["tyba-a3f-nao-e-uuid".to_string()];
        assert!(
            orphans(&listed, "a3f", &HashSet::new()).is_empty(),
            "na dúvida sobre o que é o nome, não matar"
        );
    }

    fn remote_commands() -> Vec<(&'static str, String)> {
        let name = session_name("a3f", uuid(0x9f3a));
        vec![
            ("wrap", wrap_command(&name)),
            ("has_session", has_session_command(&name)),
            ("list", list_sessions_command()),
            ("kill", kill_command(&name)),
        ]
    }

    #[test]
    fn comando_remoto_e_sh_c_sem_aspas_simples_no_script() {
        for (label, cmd) in remote_commands() {
            assert!(
                cmd.starts_with("sh -c '") && cmd.ends_with('\''),
                "{label}: o login shell remoto (fish, csh, o que for) só pode ver \
                 `sh -c '...'` — qualquer sintaxe além disso é POSIX que ele pode \
                 não falar: {cmd}"
            );
            let script = &cmd["sh -c '".len()..cmd.len() - 1];
            assert!(
                !script.contains('\''),
                "{label}: aspas simples dentro do script fecham o envelope no meio \
                 e devolvem o resto para o login shell: {cmd}"
            );
            assert!(
                !cmd.contains('\n') && !cmd.contains('!') && !cmd.contains("\\\\"),
                "{label}: newline quebra csh, `!` liga history expansion, `\\\\` é \
                 escape especial dentro de aspas simples no fish: {cmd}"
            );
        }
    }

    #[cfg(unix)]
    mod login_shells {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        use std::process::{Command, Stdio};

        fn run_as_login_shell(login_shell: &str, cmd: &str) -> Option<i32> {
            let dir = tempfile::tempdir().unwrap();
            let bin = dir.path().join("bin");
            std::fs::create_dir(&bin).unwrap();
            std::os::unix::fs::symlink("/bin/sh", bin.join("sh")).unwrap();
            let fake = dir.path().join("login-shell");
            std::fs::write(&fake, "#!/bin/sh\nexit 42\n").unwrap();
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
            Command::new(login_shell)
                .arg("-c")
                .arg(cmd)
                .env_clear()
                .env("PATH", &bin)
                .env("SHELL", &fake)
                .env("HOME", dir.path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok()?
                .code()
        }

        fn installed_login_shells() -> Vec<String> {
            let mut found = vec!["/bin/sh".to_string()];
            for shell in ["bash", "zsh", "dash", "fish", "csh", "tcsh"] {
                let out = Command::new("/bin/sh")
                    .arg("-c")
                    .arg(format!("command -v {shell}"))
                    .output();
                if let Ok(o) = out {
                    let path = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if o.status.success() && !path.is_empty() {
                        found.push(path);
                    }
                }
            }
            found
        }

        #[test]
        #[cfg(target_os = "macos")]
        fn cobertura_inclui_login_shell_nao_posix() {
            let shells = installed_login_shells();
            assert!(
                shells
                    .iter()
                    .any(|s| s.ends_with("/csh") || s.ends_with("/tcsh") || s.ends_with("/fish")),
                "csh e tcsh vêm com o macOS: sem nenhum shell não-POSIX na lista, \
                 estes testes ficam verdes exercitando só os shells onde o bug \
                 nunca aparecia; got {shells:?}"
            );
        }

        #[test]
        fn wrap_abre_o_shell_sob_qualquer_login_shell_sem_tmux() {
            let name = session_name("a3f", uuid(0x9f3a));
            for shell in installed_login_shells() {
                assert_eq!(
                    run_as_login_shell(&shell, &wrap_command(&name)),
                    Some(42),
                    "medido no Arch do usuário: com login shell fish o wrap morria \
                     no parse (`${{` não é fish) e a sessão sumia — o shell do host \
                     nunca abria; sob {shell} o fallback tem que chegar ao $SHELL"
                );
            }
        }

        #[test]
        fn probe_ve_127_sob_qualquer_login_shell_sem_tmux() {
            let name = session_name("a3f", uuid(0x9f3a));
            for shell in installed_login_shells() {
                let code = run_as_login_shell(&shell, &has_session_command(&name));
                assert_eq!(
                    interpret_has_session(code),
                    Probe::NoTmux,
                    "csh devolve 1 para comando inexistente, não 127 — sem o sh -c \
                     o probe classificaria NoTmux como Gone; got {code:?} sob {shell}"
                );
            }
        }

        #[test]
        fn gc_e_kill_degradam_para_sucesso_sob_qualquer_login_shell_sem_tmux() {
            let name = session_name("a3f", uuid(0x9f3a));
            for shell in installed_login_shells() {
                for (label, cmd) in [
                    ("kill", kill_command(&name)),
                    ("list", list_sessions_command()),
                ] {
                    assert_eq!(
                        run_as_login_shell(&shell, &cmd),
                        Some(0),
                        "{label}: sem tmux não há o que recolher — csh quebrava no \
                         `2>/dev/null` antes do `; true` final; got sob {shell}"
                    );
                }
            }
        }
    }
}
