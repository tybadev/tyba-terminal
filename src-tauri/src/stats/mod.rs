//! Painel de estatísticas de sessão de agente.
//!
//! O TYBA já gravava dois acervos que ninguém lia: `approval_history` (todo
//! pedido de aprovação, com risco, decisão e os dois timestamps) e `block`
//! (todo comando executado, com início e fim). A pergunta que este módulo
//! responde não é "qual comando eu mais uso" — é **quanto os agentes custam de
//! atenção humana**.
//!
//! Os tipos daqui são só o formato do resultado. A agregação inteira acontece
//! em SQL, dentro do [`crate::session::store::Store`] (princípio #1): o webview
//! recebe número pronto e desenha.

use serde::Serialize;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// Quantas linhas cada tabela devolve.
///
/// O painel é leitura de olho humano, não export: sem teto, um banco com meses
/// de uso manda milhares de linhas pelo IPC para preencher uma tela que mostra
/// vinte.
pub const COMMAND_ROWS: usize = 20;
pub const SESSION_ROWS: usize = 50;

/// Decisões em que ninguém foi perguntado.
///
/// `auto_allowed` é o caminho de risco verde. `session_allowed` **não** é
/// verde: é um "aprovar sempre" que o usuário já deu antes, nesta sessão, sendo
/// reusado. Contam juntos porque a pergunta do painel é quanto custou de
/// atenção — e nenhum dos dois interrompeu ninguém.
pub(crate) const AUTO_DECISIONS: &str = "'auto_allowed','session_allowed'";

/// Decisões que passaram por um humano na hora.
///
/// `refused` e `expired` ficam de fora de propósito: são pedidos que morreram
/// sem decisão (canal caiu, sessão encerrou com a aprovação pendente). Entram
/// no total pedido e em nenhum dos dois lados — somar um pedido esquecido como
/// "decidido por humano" inflaria justamente a métrica de atenção.
pub(crate) const HUMAN_DECISIONS: &str = "'approved','approved_always','denied'";

/// Decisões que deixaram o agente rodar.
pub(crate) const APPROVING_DECISIONS: &str =
    "'auto_allowed','session_allowed','approved','approved_always'";

/// Começo da janela do painel, em epoch ms.
///
/// `None` é "tudo" e vira 0 em vez de um `Option` que cada consulta teria de
/// tratar: `WHERE ts >= 0` é a mesma coisa que sem filtro, e nenhum timestamp
/// gravado é negativo.
///
/// `saturating_sub` porque uma janela maior que o próprio relógio (máquina com
/// data errada, banco recém-criado) daria a volta no u64 e o painel mostraria
/// vazio com o banco cheio.
pub fn window_start_ms(days: Option<u32>, now_ms: u64) -> u64 {
    match days {
        None => 0,
        Some(days) => now_ms.saturating_sub(u64::from(days).saturating_mul(DAY_MS)),
    }
}

/// Percentual de `part` sobre `total`, arredondado a uma casa.
///
/// O período vazio é o caso normal deste painel, não a exceção: banco novo,
/// repo sem agente, recorte de 7 dias numa semana parada. `total == 0` devolve
/// zero em vez de `0.0 / 0.0` — NaN atravessa o IPC como `null` e a tela mostra
/// "NaN%" ou some com o cartão.
pub fn percent(part: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    round1(part as f64 * 100.0 / total as f64)
}

/// Arredonda a uma casa decimal.
///
/// Feito aqui e não no React porque o número que sai do core é o número que a
/// tela mostra — `33.33333333333333` no JSON é ruído que cada view teria de
/// aparar por conta própria.
pub fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// Risco a partir da severidade calculada em SQL (3 = red, 2 = yellow).
///
/// Um mesmo comando pode ter sido classificado diferente em momentos
/// diferentes (o risco de escrita depende do caminho, e o caminho muda). A
/// linha da tabela mostra o PIOR que aquele comando já teve: dizer "verde" para
/// algo que um dia foi vermelho é o erro caro dos dois.
pub fn risk_from_severity(severity: i64) -> &'static str {
    match severity {
        3 => "red",
        2 => "yellow",
        _ => "green",
    }
}

/// Cartões do topo.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApprovalTotals {
    pub requested: u64,
    pub auto_approved: u64,
    pub human_decided: u64,
    pub denied: u64,
    pub auto_approved_pct: f64,
    pub human_decided_pct: f64,
    pub denied_pct: f64,
    /// Mediana — não média — do tempo entre pedir e decidir, só nas que
    /// exigiram humano. `None` quando ninguém decidiu nada no período.
    ///
    /// Uma aprovação esquecida por duas horas puxa a média para um número que
    /// não descreve nenhum dia real; a mediana continua descrevendo o dia a
    /// dia. `None` em vez de 0 porque "ninguém decidiu nada" e "todo mundo
    /// decidiu na hora" são fatos diferentes.
    pub median_human_ms: Option<u64>,
}

/// Uma linha da tabela de comandos mais pedidos.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommandStat {
    pub command: String,
    pub requests: u64,
    pub risk: String,
    pub approved: u64,
    pub approval_rate: f64,
}

/// Uma linha da tabela de sessões.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionStat {
    pub session_id: String,
    pub title: String,
    pub commands: u64,
    pub approvals: u64,
    /// Soma da duração dos blocos do período — tempo com comando rodando, não
    /// o intervalo entre o primeiro e o último.
    pub total_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentStats {
    pub totals: ApprovalTotals,
    pub commands: Vec<CommandStat>,
    pub sessions: Vec<SessionStat>,
    /// Repositórios com atividade no período, para o filtro.
    ///
    /// Sai da mesma janela e ignora o escopo escolhido — filtrar a lista pelo
    /// filtro deixaria a pessoa presa no repo que acabou de escolher.
    pub repos: Vec<String>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;

    use super::*;
    use crate::blocks::Block;
    use crate::session::store::{ApprovalHistoryEntry, Store};
    use crate::session::{Session, SessionId, SessionKind, SessionStatus};

    fn agent_session(store: &Store, repo: &str, title: &str) -> SessionId {
        let id = SessionId::new_v4();
        let session = Session {
            id,
            kind: SessionKind::Agent {
                runner: crate::session::AgentRunnerKind::ClaudeCode,
            },
            title: title.to_string(),
            repo_root: Some(PathBuf::from(repo)),
            worktree: None,
            status: SessionStatus::Running,
            attention: false,
            agent_conversation_id: None,
            created_at: Utc::now(),
            cwd: Some(PathBuf::from(repo)),
            connection: crate::session::ConnectionState::default(),
        };
        store.upsert_session(&session).unwrap();
        id
    }

    fn approval(
        store: &Store,
        session: SessionId,
        command: &str,
        risk: &str,
        decision: &str,
        requested_at_ms: u64,
        elapsed_ms: u64,
    ) {
        store
            .insert_approval_history(&ApprovalHistoryEntry {
                session_id: session.to_string(),
                command: command.to_string(),
                cwd: None,
                risk: risk.to_string(),
                decision: decision.to_string(),
                requested_at_ms,
                resolved_at_ms: requested_at_ms + elapsed_ms,
            })
            .unwrap();
    }

    fn block(store: &Store, session: SessionId, started_at_ms: i64, duration_ms: i64) {
        store
            .insert_block(&Block {
                id: 0,
                session_id: session.to_string(),
                command: "cargo test".into(),
                exit_code: Some(0),
                cwd: None,
                started_at_ms,
                finished_at_ms: started_at_ms + duration_ms,
                lines: Vec::new(),
                truncated: 0,
                alt_screen: false,
            })
            .unwrap();
    }

    /// Cinco decisões humanas: a mediana é a do meio, e não a média — que aqui
    /// daria 2 220 ms por causa da única esquecida por dez segundos.
    #[test]
    fn median_with_an_odd_number_of_samples_is_the_middle_one() {
        let store = Store::open_in_memory().unwrap();
        let session = agent_session(&store, "/repo", "agente");
        for elapsed in [100, 200, 300, 500, 10_000] {
            approval(
                &store, session, "git push", "red", "approved", 1_000, elapsed,
            );
        }

        let stats = store.agent_stats(0, None).unwrap();

        assert_eq!(stats.totals.median_human_ms, Some(300));
    }

    /// Com número par não existe "a do meio": a mediana é a média das duas
    /// centrais. Sem isso a conta escolheria uma das duas e o resultado
    /// dependeria da ordem em que as linhas entraram.
    #[test]
    fn median_with_an_even_number_of_samples_averages_the_two_middle_ones() {
        let store = Store::open_in_memory().unwrap();
        let session = agent_session(&store, "/repo", "agente");
        for elapsed in [100, 200, 300, 500] {
            approval(&store, session, "git push", "red", "denied", 1_000, elapsed);
        }

        let stats = store.agent_stats(0, None).unwrap();

        assert_eq!(stats.totals.median_human_ms, Some(250));
    }

    /// Só as que exigiram humano entram na mediana: as automáticas resolvem em
    /// zero e puxariam o número para baixo descrevendo uma espera que não
    /// existiu.
    #[test]
    fn the_median_ignores_what_no_human_decided() {
        let store = Store::open_in_memory().unwrap();
        let session = agent_session(&store, "/repo", "agente");
        approval(&store, session, "ls", "green", "auto_allowed", 1_000, 0);
        approval(&store, session, "ls", "green", "auto_allowed", 1_000, 0);
        approval(
            &store, session, "rm -rf x", "red", "expired", 1_000, 900_000,
        );
        approval(&store, session, "git push", "red", "approved", 1_000, 4_000);

        let stats = store.agent_stats(0, None).unwrap();

        assert_eq!(stats.totals.median_human_ms, Some(4_000));
        assert_eq!(stats.totals.requested, 4);
        assert_eq!(stats.totals.auto_approved, 2);
        assert_eq!(stats.totals.human_decided, 1);
    }

    /// Período sem nada é o estado normal de banco novo. Nenhum percentual pode
    /// virar NaN e nenhuma mediana pode virar zero fingindo decisão instantânea.
    #[test]
    fn an_empty_window_reports_zeroes_and_never_divides_by_zero() {
        let store = Store::open_in_memory().unwrap();
        let session = agent_session(&store, "/repo", "agente");
        approval(&store, session, "git push", "red", "approved", 1_000, 4_000);
        block(&store, session, 1_000, 500);

        // Janela que começa depois de tudo que existe.
        let stats = store.agent_stats(50_000, None).unwrap();

        assert_eq!(stats.totals.requested, 0);
        assert_eq!(stats.totals.auto_approved, 0);
        assert_eq!(stats.totals.human_decided, 0);
        assert_eq!(stats.totals.denied, 0);
        assert_eq!(stats.totals.median_human_ms, None);
        for pct in [
            stats.totals.auto_approved_pct,
            stats.totals.human_decided_pct,
            stats.totals.denied_pct,
        ] {
            assert!(pct.is_finite(), "percentual virou {pct}");
            assert_eq!(pct, 0.0);
        }
        assert!(stats.commands.is_empty());
        assert!(stats.sessions.is_empty());
        assert!(stats.repos.is_empty());
    }

    /// A janela corta pelo timestamp do pedido, não pelo da resolução: o que
    /// vale é quando o agente pediu.
    #[test]
    fn the_window_keeps_only_what_happened_inside_it() {
        let store = Store::open_in_memory().unwrap();
        let session = agent_session(&store, "/repo", "agente");
        approval(&store, session, "antigo", "red", "approved", 1_000, 10);
        approval(&store, session, "recente", "red", "approved", 90_000, 20);

        let stats = store.agent_stats(50_000, None).unwrap();

        assert_eq!(stats.totals.requested, 1);
        assert_eq!(stats.commands.len(), 1);
        assert_eq!(stats.commands[0].command, "recente");
    }

    /// `approval_history` e `block` não guardam repo: o escopo vem da sessão
    /// dona da linha. Se o `EXISTS` cair, o filtro vira decoração e o painel
    /// mostra o repo errado sem avisar.
    #[test]
    fn the_repository_scope_actually_filters() {
        let store = Store::open_in_memory().unwrap();
        let here = agent_session(&store, "/repo/tyba", "tyba");
        let there = agent_session(&store, "/repo/outro", "outro");
        approval(&store, here, "git push", "red", "approved", 1_000, 100);
        approval(&store, here, "git push", "red", "approved", 1_000, 300);
        approval(&store, there, "rm -rf /", "red", "denied", 1_000, 5_000);
        block(&store, here, 1_000, 40);
        block(&store, there, 1_000, 80);

        let scoped = store.agent_stats(0, Some("/repo/tyba")).unwrap();

        assert_eq!(scoped.totals.requested, 2);
        assert_eq!(scoped.totals.denied, 0);
        assert_eq!(scoped.totals.median_human_ms, Some(200));
        assert_eq!(scoped.commands.len(), 1);
        assert_eq!(scoped.commands[0].command, "git push");
        assert_eq!(scoped.sessions.len(), 1);
        assert_eq!(scoped.sessions[0].title, "tyba");
        assert_eq!(scoped.sessions[0].commands, 1);
        assert_eq!(scoped.sessions[0].total_ms, 40);
        // O filtro conhece os dois repos, senão não dá para voltar.
        assert_eq!(scoped.repos, vec!["/repo/outro", "/repo/tyba"]);

        let all = store.agent_stats(0, None).unwrap();
        assert_eq!(all.totals.requested, 3);
        assert_eq!(all.sessions.len(), 2);
    }

    /// Um repo que nunca teve agente não some da conta por acaso: ele
    /// simplesmente não tem linha nenhuma.
    #[test]
    fn scoping_to_a_repository_without_activity_is_empty_not_broken() {
        let store = Store::open_in_memory().unwrap();
        let session = agent_session(&store, "/repo/tyba", "tyba");
        approval(&store, session, "git push", "red", "approved", 1_000, 100);

        let stats = store.agent_stats(0, Some("/repo/vazio")).unwrap();

        assert_eq!(stats.totals.requested, 0);
        assert_eq!(stats.totals.auto_approved_pct, 0.0);
        assert_eq!(stats.totals.median_human_ms, None);
        assert!(stats.sessions.is_empty());
    }

    #[test]
    fn totals_split_automatic_from_human_and_count_the_denials() {
        let store = Store::open_in_memory().unwrap();
        let session = agent_session(&store, "/repo", "agente");
        approval(&store, session, "ls", "green", "auto_allowed", 1_000, 0);
        approval(&store, session, "ls", "green", "auto_allowed", 1_000, 0);
        approval(
            &store,
            session,
            "cargo test",
            "yellow",
            "session_allowed",
            1_000,
            0,
        );
        approval(&store, session, "git push", "red", "approved", 1_000, 500);
        approval(&store, session, "rm -rf /", "red", "denied", 1_000, 700);

        let stats = store.agent_stats(0, None).unwrap();

        assert_eq!(stats.totals.requested, 5);
        assert_eq!(stats.totals.auto_approved, 3);
        assert_eq!(stats.totals.human_decided, 2);
        assert_eq!(stats.totals.denied, 1);
        assert_eq!(stats.totals.auto_approved_pct, 60.0);
        assert_eq!(stats.totals.human_decided_pct, 40.0);
        assert_eq!(stats.totals.denied_pct, 20.0);
    }

    /// A tabela é "mais pedidos": a ordem é por número de pedidos, e o risco da
    /// linha é o pior que aquele comando já teve.
    #[test]
    fn the_command_table_ranks_by_requests_and_shows_the_worst_risk() {
        let store = Store::open_in_memory().unwrap();
        let session = agent_session(&store, "/repo", "agente");
        approval(&store, session, "git push", "red", "approved", 1_000, 10);
        for _ in 0..3 {
            approval(
                &store,
                session,
                "cargo test",
                "green",
                "auto_allowed",
                1_000,
                0,
            );
        }
        approval(&store, session, "cargo test", "yellow", "denied", 1_000, 10);

        let stats = store.agent_stats(0, None).unwrap();

        assert_eq!(stats.commands.len(), 2);
        assert_eq!(stats.commands[0].command, "cargo test");
        assert_eq!(stats.commands[0].requests, 4);
        assert_eq!(stats.commands[0].risk, "yellow");
        assert_eq!(stats.commands[0].approved, 3);
        assert_eq!(stats.commands[0].approval_rate, 75.0);
        assert_eq!(stats.commands[1].command, "git push");
        assert_eq!(stats.commands[1].risk, "red");
        assert_eq!(stats.commands[1].approval_rate, 100.0);
    }

    /// Sessão de shell não é sessão de agente: ela executa comando e nunca pede
    /// aprovação, então entraria na tabela como uma linha de zeros que empurra
    /// as de agente para baixo.
    #[test]
    fn the_session_table_leaves_out_shell_sessions() {
        let store = Store::open_in_memory().unwrap();
        let shell = SessionId::new_v4();
        store
            .upsert_session(&Session {
                id: shell,
                kind: SessionKind::Shell,
                title: "zsh".into(),
                repo_root: Some(PathBuf::from("/repo")),
                worktree: None,
                status: SessionStatus::Running,
                attention: false,
                agent_conversation_id: None,
                created_at: Utc::now(),
                cwd: None,
                connection: crate::session::ConnectionState::default(),
            })
            .unwrap();
        block(&store, shell, 1_000, 100);
        let agent = agent_session(&store, "/repo", "agente");
        block(&store, agent, 1_000, 250);
        approval(&store, agent, "git push", "red", "approved", 1_000, 10);

        let stats = store.agent_stats(0, None).unwrap();

        assert_eq!(stats.sessions.len(), 1);
        assert_eq!(stats.sessions[0].title, "agente");
        assert_eq!(stats.sessions[0].commands, 1);
        assert_eq!(stats.sessions[0].approvals, 1);
        assert_eq!(stats.sessions[0].total_ms, 250);
    }

    /// `remove_session` apaga a sessão e os blocos dela, mas não o
    /// `approval_history`: a linha sobrevive sem dono. Ela ainda conta — some
    /// da tela seria perder atenção humana que de fato foi gasta.
    #[test]
    fn approvals_from_a_discarded_session_still_count() {
        let store = Store::open_in_memory().unwrap();
        let session = agent_session(&store, "/repo", "agente");
        approval(&store, session, "git push", "red", "approved", 1_000, 100);
        store.remove_session(session).unwrap();

        let stats = store.agent_stats(0, None).unwrap();

        assert_eq!(stats.totals.requested, 1);
        assert_eq!(stats.sessions.len(), 1);
        // Sem linha em `sessions` não há título: o id é o que sobrou.
        assert_eq!(stats.sessions[0].title, session.to_string());
        // E sem sessão não há repo: só aparece em "todos".
        assert!(store
            .agent_stats(0, Some("/repo"))
            .unwrap()
            .sessions
            .is_empty());
    }

    /// O comando já entra redigido no banco, mas o painel redige de novo na
    /// saída (princípio #10). A linha crua aqui é escrita por fora do
    /// `insert_approval_history` de propósito: é o único jeito de provar que a
    /// redação da consulta existe — pelo caminho normal ela já veio redigida e
    /// o teste passaria mesmo sem redação nenhuma.
    #[test]
    fn the_command_column_never_ships_a_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tyba.db");
        let store = Store::open(&path).unwrap();
        let session = agent_session(&store, "/repo", "agente");
        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute(
            "INSERT INTO approval_history
                 (session_id, command, cwd, risk, decision, requested_at_ms, resolved_at_ms)
             VALUES (?1, ?2, NULL, 'red', 'approved', 1000, 1100)",
            rusqlite::params![
                session.to_string(),
                "curl -H 'x: sk-abcdefghijklmnopqrstuvwxyz0123'"
            ],
        )
        .unwrap();

        let stats = store.agent_stats(0, None).unwrap();

        assert_eq!(stats.commands.len(), 1);
        assert!(
            !stats.commands[0].command.contains("sk-abcdefghij"),
            "{}",
            stats.commands[0].command
        );
        assert!(stats.commands[0].command.contains("[REDACTED]"));
    }

    #[test]
    fn the_window_start_never_wraps_around() {
        assert_eq!(window_start_ms(None, 1_000), 0);
        assert_eq!(window_start_ms(Some(7), 10 * DAY_MS), 3 * DAY_MS);
        // Janela maior que o próprio relógio: o começo é 0, não um u64 que deu
        // a volta e esconderia tudo que existe.
        assert_eq!(window_start_ms(Some(30), 1_000), 0);
        assert_eq!(window_start_ms(Some(u32::MAX), 1_000), 0);
    }
}
