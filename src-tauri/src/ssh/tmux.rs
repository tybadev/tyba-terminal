use std::collections::HashSet;
use std::time::Duration;

use uuid::Uuid;

use crate::session::store::{Store, StoreError};

pub const INSTALL_ID_KEY: &str = "ssh.install_id";

/// Identidade desta instalação do TYBA no remoto. Não resolve "qual sessão é
/// esta?" (o `session_id` já resolve, e sobrevive ao restart) e sim "eu posso
/// matá-la?": o GC de uma máquina não tem autoridade sobre a sessão viva de
/// outra. Ver ADR `2026-07-16-ssh-tmux-namespace-por-instalacao`.
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

/// O comando que roda no Host. O fallback mora aqui, e não numa sonda prévia,
/// por dois motivos: não custa round-trip no connect, e nada fica cacheado para
/// mentir depois que alguém instalar tmux no Host.
///
/// A invisibilidade é o resto: `status off` (a tela é do dono), `prefix None` (o
/// TYBA controla pelo CLI, nunca por tecla — então não disputa Ctrl-B) e `env -u
/// TMUX` no **comando do pane**, que é onde o tmux seta a variável: sem isso o
/// tmux do dono recusa aninhar e um fluxo que funcionava antes do wrap quebra.
/// Ver ADR `2026-07-16-ssh-tmux-invisivel-o-do-dono-aninha-dentro`.
pub fn wrap_command(name: &str) -> String {
    format!(
        "command -v tmux >/dev/null 2>&1 && \
         exec tmux new-session -A -s {name} 'exec env -u TMUX \"${{SHELL:-/bin/sh}}\" -l' \\; \
         set-option -t {name} status off \\; \
         set-option -t {name} prefix None \\; \
         set-option -t {name} history-limit 5000 || \
         exec \"${{SHELL:-/bin/sh}}\" -l"
    )
}

/// O veredito do árbitro. O TYBA pergunta ao Host em vez de inferir intenção do
/// exit code do `ssh`: 255 é queda **e** falha de auth, e o detach do tmux sai 0
/// igual ao `exit` do dono — dois sinais ambíguos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// A SSH Session está viva no Host; o que caiu foi o Cano.
    Alive,
    /// O dono deu `exit`: acabou de verdade, sem órfã.
    Gone,
    /// O Host não tem tmux. Sem isto, um Host vivo e sem tmux reconectaria para
    /// sempre — a sonda falharia igual a "não sei".
    NoTmux,
    /// Não deu para perguntar (Host fora do ar).
    Unknown,
}

/// `tmux has-session -t <nome>` → veredito.
///
/// `None` é o processo morto por sinal. "Não sei" cai para o lado de reatar:
/// errar reatando custa uma tentativa; errar para o outro lado descarta trabalho
/// vivo do dono.
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

/// Espera antes da tentativa `attempt` (0-based). `None` = desistiu: vai para
/// `Dropped` com botão manual.
///
/// Desistir não perde nada — a SSH Session não vai a lugar nenhum. Perde-se só o
/// direito de sondar de graça uma VPS desligada, que é o que a regra de
/// performance cobra (o core disputa rede com os agentes).
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

/// Sessões `tyba-*` no Host que esta instalação deve recolher.
///
/// `listed` é o `tmux ls -F '#{session_name}'` do Host. `known` são os
/// `session_id` do SQLite — **inclusive os mortos**: sessão `Exited` com tmux
/// vivo é o caso de reattach, não órfã (o `restore()` preserva a linha, e é dela
/// que o reattach sai).
///
/// Só o próprio namespace entra. Sessão de outra instalação não é "poupada por
/// heurística": ela é invisível.
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

/// Pergunta ao Host se a SSH Session ainda existe.
///
/// Falha de rede vira `Unknown` (que reata) e não `Gone`: só o Host tem
/// autoridade para dizer que acabou, e um Host que não responde não disse nada.
pub fn probe(alias: &str, name: &str) -> Probe {
    let status = std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", alias])
        .arg(format!("tmux has-session -t {name}"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(s) => interpret_has_session(s.code()),
        Err(_) => Probe::Unknown,
    }
}

/// Encerra a SSH Session no Host. É o gesto deliberado do dono (fechar tab),
/// simétrico ao `killpg` do shell local — não é o Cano caindo.
///
/// `-o BatchMode=yes` porque isto roda sem tela: sem ele, um host que peça senha
/// penduraria a thread esperando input que ninguém vai digitar. O ControlMaster
/// da conexão que acabou de morrer ainda está quente (ControlPersist 10m), então
/// na prática não há handshake novo.
pub fn kill_remote(alias: &str, name: &str) -> std::io::Result<std::process::ExitStatus> {
    std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", alias])
        .arg(kill_command(name))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
}

/// Mata a sessão **e** o cliente que sobrou.
///
/// Achado no primeiro teste real: quando o Cano morre, o processo
/// `tmux new-session` que o `ssh` spawnou não morre com o hangup — reaparece com
/// `ppid=1`, sem tty, e fica pendurado na máquina do dono. O `kill-session`
/// derruba o cliente daquela sessão junto; o `pkill` cobre o que já tinha sido
/// adotado pelo init numa queda anterior. `true` no fim porque não achar nada
/// para matar é sucesso, não erro.
///
/// O `[t]` não é enfeite: o padrão do `pkill` aparece na linha de comando do
/// próprio shell que o executa (o `bash -c` do sshd), e `pkill -f` exclui a si
/// mesmo mas **não ao pai**. Medido na VPS: sem o colchete o comando se suicida
/// no meio — o `true` nunca roda e o ssh devolve 255. Com ele, a regex `[t]mux`
/// casa `tmux` nos alvos, e a linha do shell (que contém `[t]mux` literal) não
/// casa consigo mesma.
pub fn kill_command(name: &str) -> String {
    format!(
        "tmux kill-session -t {name} 2>/dev/null; \
         pkill -f '[t]mux new-session -A -s {name}' 2>/dev/null; \
         true"
    )
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
            cmd.contains(&format!("pkill -f '[t]mux new-session -A -s {name}'")),
            "o cliente vira órfão do init quando o cano morre; kill-session sozinho \
             não o alcança se ele já ficou pendurado antes: {cmd}"
        );
        assert!(
            cmd.ends_with("true"),
            "não achar o que matar é sucesso, não erro: {cmd}"
        );
    }

    /// Medido na VPS: `pkill -f 'tmux new-session…'` casa a linha do próprio
    /// `bash -c` que o executa e mata o pai — o `true` não roda e o ssh volta 255.
    /// O `[t]` faz a regex casar os alvos sem casar a si mesma.
    #[test]
    fn kill_nao_pode_matar_o_shell_que_o_executa() {
        let cmd = kill_command(&session_name("a3f", uuid(0x9f3a)));
        assert!(
            !cmd.contains("pkill -f 'tmux"),
            "sem o [t] o comando se suicida no meio: {cmd}"
        );
        assert!(cmd.contains("pkill -f '[t]mux"), "got: {cmd}");
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
            cmd.ends_with("|| exec \"${SHELL:-/bin/sh}\" -l"),
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

    /// Verificado na VPS real: um `contains("env -u TMUX")` passa com o `env` no
    /// ramo do fallback — onde `$TMUX` nem existe — e o dono ainda leva
    /// "sessions should be nested with care" na cara. O que importa é o ramo: o
    /// `env` tem que ser o **comando do pane**, porque é o tmux que seta `$TMUX`.
    #[test]
    fn wrap_limpa_tmux_no_shell_dentro_do_nosso_tmux() {
        let cmd = wrap_command("tyba-a3f-9f3a");
        assert!(
            cmd.contains("new-session -A -s tyba-a3f-9f3a 'exec env -u TMUX "),
            "o env tem que ser o comando do pane, não o fallback: {cmd}"
        );
        assert!(
            !cmd.contains("|| exec env -u TMUX"),
            "no fallback não há tmux, logo não há $TMUX para limpar: {cmd}"
        );
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
}
