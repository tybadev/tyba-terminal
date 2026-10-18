//! Histórico de comandos.
//!
//! O que o usuário digitou já chega no core pelo `OSC 633;E` e o exit code pelo
//! `133;D` — ver `status/mod.rs`. Até aqui isso era usado para detectar agente e
//! descartado. Este módulo guarda.
//!
//! A gravação sai do caminho quente: a thread emitter do PTY empurra num canal
//! limitado e segue; uma thread escritora é a única que toca SQLite. Sob pressão
//! o registro é descartado — perder uma linha de histórico é melhor do que
//! segurar o flush de output do terminal.

pub mod import;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, OnceLock};

use crate::session::store::Store;

const QUEUE_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRecord {
    pub session_id: String,
    pub cwd: Option<String>,
    pub command: String,
    pub exit_code: Option<i32>,
    pub started_at_ms: i64,
    pub duration_ms: Option<i64>,
    /// O Host em cujo servidor o comando foi digitado (regra 18). `None` é o
    /// comando local — e é o que separa os dois mundos na regra 19.
    pub host_id: Option<String>,
}

/// Uma execução gravada, como o painel Histórico mostra.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HistoryRun {
    pub command: String,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub started_at_ms: i64,
    pub duration_ms: Option<i64>,
}

struct Recorder {
    tx: SyncSender<CommandRecord>,
    enabled: AtomicBool,
}

static RECORDER: OnceLock<Recorder> = OnceLock::new();

pub fn install(store: Arc<Store>, enabled: bool) {
    let (tx, rx) = sync_channel::<CommandRecord>(QUEUE_CAPACITY);
    let installed = RECORDER.set(Recorder {
        tx,
        enabled: AtomicBool::new(enabled),
    });
    if installed.is_err() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("command-history-writer".into())
        .spawn(move || {
            while let Ok(record) = rx.recv() {
                let _ = store.insert_command(&record);
            }
        });
}

pub fn set_enabled(enabled: bool) {
    if let Some(recorder) = RECORDER.get() {
        recorder.enabled.store(enabled, Ordering::Relaxed);
    }
}

/// Nunca bloqueia: fila cheia descarta o registro.
pub fn record(record: CommandRecord) {
    let Some(recorder) = RECORDER.get() else {
        return;
    };
    if !recorder.enabled.load(Ordering::Relaxed) {
        return;
    }
    let _ = recorder.tx.try_send(record);
}

/// Quantos comandos a sessão efêmera guarda em memória.
pub const LIVE_LIMIT: usize = 500;

/// O histórico de cada sessão efêmera (shell do nó Terminal do Orbit), só em
/// memória — o par de `blocks::LIVE`.
///
/// A sessão efêmera não grava `command_history` (nada dela vai ao disco), e
/// sem isto o ↑, o cinza e a lista da linha do TYBA respondiam com o
/// histórico da GRADE: ↑ e Enter rodavam um comando que nunca foi digitado no
/// nó. Mesma redação do banco (`insert_command`), mesmo descarte de repetição
/// seguida, e mesma porta: só guarda sessão aberta (`PtyPool::set_ephemeral`),
/// e o `dispose` a fecha.
#[derive(Default)]
pub struct LiveHistory {
    /// Presença da chave = sessão aberta. Do mais antigo ao mais novo.
    by_session: parking_lot::Mutex<HashMap<String, std::collections::VecDeque<CommandRecord>>>,
}

impl LiveHistory {
    pub fn open(&self, session: &str) {
        self.by_session
            .lock()
            .entry(session.to_string())
            .or_default();
    }

    /// Respeita a preferência de histórico desligado, como o [`record`].
    pub fn remember(&self, mut record: CommandRecord) {
        if RECORDER
            .get()
            .is_some_and(|r| !r.enabled.load(Ordering::Relaxed))
        {
            return;
        }
        record.command = crate::session::redact::redact(&record.command).into_owned();
        record.cwd = record
            .cwd
            .map(|cwd| crate::session::redact::redact(&cwd).into_owned());
        let mut map = self.by_session.lock();
        let Some(list) = map.get_mut(&record.session_id) else {
            return;
        };
        if list
            .back()
            .is_some_and(|last| last.command == record.command)
        {
            return;
        }
        list.push_back(record);
        while list.len() > LIVE_LIMIT {
            list.pop_front();
        }
    }

    /// Os comandos da sessão, do mais antigo ao mais novo. `None` quando ela
    /// não é efêmera aberta.
    pub fn list(&self, session: &str) -> Option<Vec<CommandRecord>> {
        self.by_session
            .lock()
            .get(session)
            .map(|list| list.iter().cloned().collect())
    }

    pub fn close(&self, session: &str) {
        self.by_session.lock().remove(session);
    }
}

pub static LIVE: std::sync::LazyLock<LiveHistory> = std::sync::LazyLock::new(LiveHistory::default);

/// Os candidatos de uma sessão efêmera, agrupados por comando como os do
/// banco, com o mesmo filtro (escopo, desfecho, duração, período). `records`
/// do mais antigo ao mais novo, como [`LiveHistory::list`].
pub fn live_candidates(
    records: &[CommandRecord],
    cwd: Option<&str>,
    repo_root: Option<&str>,
    filter: &HistoryFilter,
) -> Vec<HistoryCandidate> {
    let root = repo_root.map(|r| r.trim_end_matches('/'));
    let in_repo = |at: Option<&str>| matches!((at, root), (Some(at), Some(root)) if at == root || at.starts_with(&format!("{root}/")));
    let mut out: Vec<HistoryCandidate> = Vec::new();
    for record in records {
        let at = record.cwd.as_deref();
        let in_cwd = cwd.is_some() && at == cwd;
        let scoped = match filter.scope {
            HistoryScope::Cwd => in_cwd,
            HistoryScope::Repo => in_repo(at),
            HistoryScope::Global | HistoryScope::Session => true,
        };
        let outcome = match filter.outcome {
            Some(HistoryOutcome::Failed) => record.exit_code.is_some_and(|c| c != 0),
            Some(HistoryOutcome::Succeeded) => record.exit_code == Some(0),
            None => true,
        };
        let long = filter
            .min_duration_ms
            .is_none_or(|min| record.duration_ms.is_some_and(|d| d >= min));
        let recent = filter
            .since_ms
            .is_none_or(|since| record.started_at_ms >= since);
        if !(scoped && outcome && long && recent) {
            continue;
        }
        let candidate = match out.iter_mut().find(|c| c.command == record.command) {
            Some(found) => found,
            None => {
                out.push(HistoryCandidate {
                    command: record.command.clone(),
                    cwd: None,
                    last_used_at_ms: record.started_at_ms,
                    uses: 0,
                    successes: 0,
                    known_exit_codes: 0,
                    in_cwd: false,
                    in_repo: false,
                    in_host: false,
                });
                out.last_mut().expect("acabou de entrar")
            }
        };
        candidate.uses += 1;
        candidate.successes += u32::from(record.exit_code == Some(0));
        candidate.known_exit_codes += u32::from(record.exit_code.is_some());
        candidate.in_cwd |= in_cwd;
        candidate.in_repo |= in_repo(at);
        if record.started_at_ms >= candidate.last_used_at_ms {
            candidate.last_used_at_ms = record.started_at_ms;
        }
        candidate.cwd = record.cwd.clone();
    }
    out
}

/// De qual Host é cada sessão SSH integrada.
///
/// Registro à parte, e não um campo a mais no [`Tracker`], porque quem constrói
/// o tracker é a máquina de captura dentro do PTY — ela só conhece o id da
/// sessão. O `SessionManager` é quem sabe o Host, e é ele quem registra aqui no
/// spawn.
static SESSION_HOST: std::sync::LazyLock<parking_lot::RwLock<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| parking_lot::RwLock::new(HashMap::new()));

pub fn bind_session_host(session_id: &str, host_id: &str) {
    SESSION_HOST
        .write()
        .insert(session_id.to_string(), host_id.to_string());
}

pub fn forget_session_host(session_id: &str) {
    SESSION_HOST.write().remove(session_id);
}

pub fn session_host(session_id: &str) -> Option<String> {
    SESSION_HOST.read().get(session_id).cloned()
}

/// Comando que não entra no histórico.
///
/// Linha começando com espaço é a convenção `HISTCONTROL=ignorespace`: é como o
/// usuário de shell já diz "não guarde este". Respeitar isso é o mínimo antes de
/// gravar linha de comando em disco.
pub fn should_record(raw: &str) -> bool {
    if raw.starts_with(' ') || raw.starts_with('\t') {
        return false;
    }
    !raw.trim().is_empty()
}

/// Máquina de estado do ciclo `633;E` → `133;C` → `133;D`.
///
/// Mora aqui, e não dentro da thread do PTY, para ser testável sem GUI e sem
/// pty: é onde estão as decisões que importam (o que grava, o que descarta, o
/// que fazer com comando que nunca terminou).
#[derive(Debug, Default)]
pub struct Tracker {
    session_id: String,
    raw: Option<String>,
    started_at: Option<i64>,
}

impl Tracker {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            raw: None,
            started_at: None,
        }
    }

    /// O comando limpo, para o evento que a UI consome.
    pub fn command(&self) -> Option<&str> {
        self.raw.as_deref().map(str::trim_start)
    }

    pub fn on_command_line(&mut self, raw: String) {
        self.raw = Some(raw);
    }

    pub fn started_at(&self) -> Option<i64> {
        self.started_at
    }

    pub fn on_start(&mut self, now_ms: i64) {
        self.started_at = Some(now_ms);
    }

    /// Fecha o comando corrente. `exit_code` é `None` quando o fim veio de um
    /// prompt novo em vez de um `133;D` — Ctrl+C no meio, shell reiniciado.
    pub fn on_end(
        &mut self,
        exit_code: Option<i32>,
        now_ms: i64,
        cwd: Option<&std::path::Path>,
    ) -> Option<CommandRecord> {
        let raw = self.raw.take()?;
        let started = self.started_at.take().unwrap_or(now_ms);
        if !should_record(&raw) {
            return None;
        }
        Some(CommandRecord {
            host_id: session_host(&self.session_id),
            session_id: self.session_id.clone(),
            cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
            command: raw.trim().to_string(),
            exit_code,
            started_at_ms: started,
            duration_ms: Some((now_ms - started).max(0)),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryCandidate {
    pub command: String,
    pub cwd: Option<String>,
    pub last_used_at_ms: i64,
    pub uses: u32,
    pub successes: u32,
    /// Quantas entradas trazem exit code. Menor que `uses` quando o comando veio
    /// de fonte que não grava código — histórico importado de bash e fish, ou
    /// comando interrompido antes do `133;D`.
    pub known_exit_codes: u32,
    pub in_cwd: bool,
    pub in_repo: bool,
    /// O comando veio do histórico do Host desta sessão (regra 19).
    pub in_host: bool,
}

const DAY_MS: f64 = 86_400_000.0;

/// Frecência: recência × frequência, com o escopo e o exit code pesando.
///
/// Comando que só falhou vale metade — repetir o que não funcionou é o oposto do
/// que se quer sugerir. Rodar neste diretório vale mais do que rodar neste repo,
/// que vale mais do que ter rodado em qualquer lugar.
///
/// **Exit code ausente é desconhecido, não fracasso.** O demérito exige ao menos
/// um código conhecido: sem isso, todo comando vindo de fonte que não grava
/// código nasceria valendo metade. Ver a decisão de 2026-08-15 no cofre.
/// Onde a busca procura.
///
/// A paleta nasceu só com "tudo", e num terminal de agentes isso é pouco: o
/// mesmo `cargo test` aparece de cinco worktrees e a lista deixa de responder
/// "o que eu rodei **aqui**". O escopo é ciclado pela UI, não configurado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryScope {
    #[default]
    Global,
    /// Qualquer lugar dentro do repositório atual, worktrees inclusive.
    Repo,
    /// Só o diretório exato.
    Cwd,
    /// Só a sessão atual.
    Session,
}

/// Filtrar por como o comando terminou.
///
/// `Failed` não é "tem exit code diferente de zero": é isso **e** ter exit code
/// conhecido. Histórico importado de bash e de fish não grava código, e tratar
/// desconhecido como falha encheria a lista de comandos que ninguém sabe se
/// falharam — o mesmo erro que o demérito da frecência já evita.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryOutcome {
    Failed,
    Succeeded,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HistoryFilter {
    pub scope: HistoryScope,
    pub outcome: Option<HistoryOutcome>,
    /// Só o que demorou pelo menos isto. Responde "o que está me fazendo
    /// esperar", que é a pergunta que o dado de duração existe para responder.
    pub min_duration_ms: Option<i64>,
    /// Só a partir deste instante, em epoch ms.
    pub since_ms: Option<i64>,
}

impl HistoryFilter {
    /// O valor que o SQL compara com `exit_code`, ou `None` para não filtrar.
    ///
    /// Existe para o `store` não precisar conhecer o enum: ele recebe um número
    /// e compara. Mantém a decisão aqui, junto do comentário que a explica.
    pub fn outcome_tag(&self) -> Option<i64> {
        match self.outcome {
            Some(HistoryOutcome::Failed) => Some(1),
            Some(HistoryOutcome::Succeeded) => Some(0),
            None => None,
        }
    }

    /// A janela pedida ainda faz sentido no instante `now_ms`?
    ///
    /// Janela que começa no futuro devolveria lista vazia sem explicação. É erro
    /// de quem chamou, não estado legítimo — e a UI trata como "sem filtro" em
    /// vez de mostrar um vazio que parece bug.
    pub fn since_is_sane(&self, now_ms: i64) -> bool {
        self.since_ms.is_none_or(|since| since <= now_ms)
    }
}

/// Por que a lista está ordenada assim.
///
/// As duas superfícies que consultam o histórico fazem perguntas diferentes, e
/// responder as duas com a mesma conta foi o que colocou um comando recém
/// executado no meio da lista.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ordem {
    /// **O ghost prevê**: "o que você provavelmente vai rodar?". O que se usa
    /// todo dia vale mais que o que rodou uma vez.
    Frecencia,
    /// **A paleta registra**: "o que eu rodei?". Cronológico, como o `Ctrl-R`
    /// do shell e como o Atuin — que ordena por `timestamp` deduplicado por
    /// comando. Aqui o fuzzy FILTRA quais aparecem; não reordena.
    Recencia,
}

/// Ordena a lista no lugar, pela pergunta que a superfície faz.
///
/// O desempate por comando não é enfeite: import de histórico grava lotes com o
/// mesmo carimbo, e sem ele a ordem entre eles muda a cada consulta — a lista
/// dança sob o dedo de quem já estava mirando um item.
pub fn ordenar(lista: &mut [HistoryCandidate], ordem: Ordem, now_ms: i64) {
    match ordem {
        Ordem::Frecencia => lista.sort_by(|a, b| {
            frecency(now_ms, b)
                .total_cmp(&frecency(now_ms, a))
                .then_with(|| a.command.cmp(&b.command))
        }),
        Ordem::Recencia => lista.sort_by(|a, b| {
            b.last_used_at_ms
                .cmp(&a.last_used_at_ms)
                .then_with(|| a.command.cmp(&b.command))
        }),
    }
}

pub fn frecency(now_ms: i64, candidate: &HistoryCandidate) -> f64 {
    let age_days = ((now_ms - candidate.last_used_at_ms).max(0) as f64) / DAY_MS;
    let recency = 1.0 / (1.0 + age_days);
    let frequency = 1.0 + (1.0 + candidate.uses as f64).ln();
    let success = if candidate.known_exit_codes > 0 && candidate.successes == 0 {
        0.5
    } else {
        1.0
    };
    // Regra 19: na sessão de um Host, o histórico DELE vem primeiro. O peso é o
    // maior da conta de escopo de propósito — "rodei isto neste servidor" prevê
    // melhor que "rodei isto nesta pasta", porque a pasta local e a remota não
    // são a mesma máquina.
    let scope = 1.0
        + if candidate.in_cwd { 1.0 } else { 0.0 }
        + if candidate.in_repo { 0.5 } else { 0.0 }
        + if candidate.in_host { 2.0 } else { 0.0 };
    recency * frequency * success * scope
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(command: &str) -> HistoryCandidate {
        HistoryCandidate {
            command: command.to_string(),
            cwd: None,
            last_used_at_ms: 0,
            uses: 1,
            successes: 1,
            known_exit_codes: 1,
            in_cwd: false,
            in_repo: false,
            in_host: false,
        }
    }

    fn live_record(session: &str, command: &str, at: i64, exit: Option<i32>) -> CommandRecord {
        CommandRecord {
            session_id: session.into(),
            cwd: Some("/repo/app".into()),
            command: command.into(),
            exit_code: exit,
            started_at_ms: at,
            duration_ms: Some(5),
            host_id: None,
        }
    }

    /// O histórico da sessão efêmera: só de sessão aberta, redigido como o do
    /// banco, sem repetição seguida, com teto, e some no `close` — o que
    /// chegar depois (o EOF do kill) é descartado.
    #[test]
    fn live_history_is_gated_redacted_bounded_and_closed() {
        let live = LiveHistory::default();
        live.remember(live_record("grade", "ls", 1, Some(0)));
        assert!(live.list("grade").is_none(), "sessão não aberta não guarda");

        live.open("no");
        live.remember(live_record(
            "no",
            "export K=sk-abcdef1234567890ABCDEFghijkl",
            1,
            Some(0),
        ));
        live.remember(live_record("no", "ls", 2, Some(0)));
        live.remember(live_record("no", "ls", 3, Some(0)));
        let got = live.list("no").unwrap();
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(!got[0].command.contains("sk-abcdef"), "{got:?}");
        assert!(got[0].command.contains("[REDACTED]"), "{got:?}");

        for at in 0..(LIVE_LIMIT as i64 + 10) {
            live.remember(live_record("no", &format!("echo {at}"), 10 + at, Some(0)));
        }
        let got = live.list("no").unwrap();
        assert_eq!(got.len(), LIVE_LIMIT);
        assert_eq!(
            got.last().map(|r| r.command.as_str()),
            Some(format!("echo {}", LIVE_LIMIT + 9).as_str())
        );

        live.close("no");
        live.remember(live_record("no", "make", 9_999, Some(0)));
        assert!(live.list("no").is_none());
    }

    /// Agrupa por comando como o banco, com o filtro, e a `Recencia` põe o
    /// último executado primeiro — o item que o ↑ escolhe.
    #[test]
    fn live_candidates_group_filter_and_order_most_recent_first() {
        let records = vec![
            live_record("no", "cargo test", 1, Some(1)),
            live_record("no", "ls", 2, Some(0)),
            live_record("no", "cargo test", 3, Some(0)),
            live_record("no", "make", 4, None),
        ];
        let mut all = live_candidates(
            &records,
            Some("/repo/app"),
            Some("/repo"),
            &HistoryFilter::default(),
        );
        ordenar(&mut all, Ordem::Recencia, 10);
        let order: Vec<&str> = all.iter().map(|c| c.command.as_str()).collect();
        assert_eq!(order, ["make", "cargo test", "ls"]);
        let cargo = &all[1];
        assert_eq!(
            (cargo.uses, cargo.successes, cargo.known_exit_codes),
            (2, 1, 2)
        );
        assert!(cargo.in_cwd && cargo.in_repo);
        assert_eq!(cargo.last_used_at_ms, 3);

        let failed = HistoryFilter {
            outcome: Some(HistoryOutcome::Failed),
            ..Default::default()
        };
        let only: Vec<String> = live_candidates(&records, None, None, &failed)
            .into_iter()
            .map(|c| c.command)
            .collect();
        assert_eq!(only, ["cargo test"]);

        let elsewhere = HistoryFilter {
            scope: HistoryScope::Cwd,
            ..Default::default()
        };
        assert!(live_candidates(&records, Some("/outro"), None, &elsewhere).is_empty());
    }

    #[test]
    fn ignorespace_keeps_the_command_out() {
        assert!(!should_record(" export TOKEN=abc"));
        assert!(!should_record("\texport TOKEN=abc"));
        assert!(should_record("cargo test"));
    }

    #[test]
    fn blank_lines_are_not_history() {
        assert!(!should_record(""));
        assert!(!should_record("   \t "));
    }

    /// Percorre um ciclo real do hook, como o `OscParser` o entrega.
    fn drive(stream: &[ShellEvent], cwd: Option<&str>) -> Vec<CommandRecord> {
        let mut tracker = Tracker::new("s1".into());
        let cwd = cwd.map(std::path::Path::new);
        let mut clock = 0i64;
        let mut out = Vec::new();
        for event in stream {
            clock += 1_000;
            match event {
                ShellEvent::CommandLine(raw) => tracker.on_command_line(raw.clone()),
                ShellEvent::CommandStart => tracker.on_start(clock),
                ShellEvent::CommandEnd(code) => out.extend(tracker.on_end(Some(*code), clock, cwd)),
                ShellEvent::PromptStart => out.extend(tracker.on_end(None, clock, cwd)),
                _ => {}
            }
        }
        out
    }

    use crate::status::ShellEvent;

    #[test]
    fn full_cycle_records_command_exit_and_duration() {
        let records = drive(
            &[
                ShellEvent::PromptStart,
                ShellEvent::CommandLine("cargo test".into()),
                ShellEvent::CommandStart,
                ShellEvent::CommandEnd(0),
            ],
            Some("/repo"),
        );
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].command, "cargo test");
        assert_eq!(records[0].exit_code, Some(0));
        assert_eq!(records[0].cwd.as_deref(), Some("/repo"));
        assert_eq!(records[0].duration_ms, Some(1_000));
    }

    #[test]
    fn prompt_before_the_first_command_records_nothing() {
        assert!(drive(&[ShellEvent::PromptStart, ShellEvent::PromptStart], None).is_empty());
    }

    #[test]
    fn command_killed_before_finishing_is_recorded_without_exit_code() {
        // Ctrl+C entre o `133;C` e o `133;D`: o prompt volta sem exit code. Sumir
        // com o comando seria pior do que guardá-lo sem o código.
        let records = drive(
            &[
                ShellEvent::CommandLine("sleep 500".into()),
                ShellEvent::CommandStart,
                ShellEvent::PromptStart,
            ],
            None,
        );
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].command, "sleep 500");
        assert_eq!(records[0].exit_code, None);
    }

    #[test]
    fn ignorespace_survives_the_whole_pipeline() {
        let records = drive(
            &[
                ShellEvent::CommandLine(" export TOKEN=abc".into()),
                ShellEvent::CommandStart,
                ShellEvent::CommandEnd(0),
            ],
            None,
        );
        assert!(records.is_empty());
    }

    #[test]
    fn command_end_without_a_command_line_records_nothing() {
        // Saída solta no terminal (um `cat` de arquivo com OSC forjado) não pode
        // virar entrada de histórico sem que exista comando.
        assert!(drive(&[ShellEvent::CommandEnd(0)], None).is_empty());
    }

    #[test]
    fn tracker_exposes_the_clean_command_for_the_ui() {
        let mut tracker = Tracker::new("s1".into());
        tracker.on_command_line("  claude --continue".into());
        assert_eq!(tracker.command(), Some("claude --continue"));
    }

    #[test]
    fn a_paleta_ordena_pelo_que_foi_rodado_por_ultimo() {
        // A paleta é um REGISTRO, não uma previsão: a pergunta ali é "o que eu
        // rodei", e a resposta é cronológica. O `Ctrl-R` do shell e o Atuin
        // fazem assim.
        //
        // Sem isto, um comando rodado 50 vezes ontem passa na frente do que
        // acabou de rodar — foi o que o dono viu: `cd` no meio da lista logo
        // depois de executá-lo.
        let agora = 1_000_000_000_i64;
        let recem = HistoryCandidate {
            last_used_at_ms: agora,
            uses: 1,
            ..candidate("cd algo")
        };
        // 200 usos, não 50: a conta importa. Com `uses` baixo a frecência já
        // põe o recente na frente sozinha, e o teste passaria sem provar nada —
        // foi o que a matriz de mutação flagrou. O ponto de virada fica perto
        // de 59 usos, então o cenário precisa estar claramente acima dele.
        let antigo_e_usado = HistoryCandidate {
            last_used_at_ms: agora - 2 * DAY_MS as i64,
            uses: 200,
            ..candidate("ls -la")
        };

        // O cenário do dono, confirmado pela conta: pela frecência o antigo e
        // muito usado GANHA do que acabou de rodar. É por isso que o `cd`
        // aparecia no meio da lista.
        assert!(
            frecency(agora, &antigo_e_usado) > frecency(agora, &recem),
            "o cenário do dono não se reproduz: a frecência já separaria os dois"
        );

        // Pela recência, quem rodou por último vem primeiro. Sempre.
        let mut lista = vec![antigo_e_usado.clone(), recem.clone()];
        ordenar(&mut lista, Ordem::Recencia, agora);
        assert_eq!(lista[0].command, "cd algo");
    }

    #[test]
    fn o_ghost_continua_prevendo_pela_frecencia() {
        // A outra metade da separação: o ghost responde "o que você
        // provavelmente vai rodar", e ali o que se usa todo dia vale mais que o
        // que rodou uma vez. Trocar isto por recência faria o cinza sugerir o
        // último comando em vez do provável.
        let agora = 1_000_000_000_i64;
        let raro = HistoryCandidate {
            last_used_at_ms: agora,
            uses: 1,
            ..candidate("comando-raro")
        };
        let diario = HistoryCandidate {
            last_used_at_ms: agora - 3_600_000,
            uses: 200,
            ..candidate("git status")
        };
        let mut lista = vec![raro, diario];
        ordenar(&mut lista, Ordem::Frecencia, agora);
        assert_eq!(lista[0].command, "git status");
    }

    #[test]
    fn empate_de_instante_nao_embaralha_a_lista() {
        // Import de histórico grava lotes com o mesmo carimbo. Sem desempate, a
        // ordem entre eles muda a cada consulta e a lista dança sob o dedo.
        let agora = 1_000_000_000_i64;
        let mut lista = vec![
            HistoryCandidate {
                last_used_at_ms: agora,
                ..candidate("b")
            },
            HistoryCandidate {
                last_used_at_ms: agora,
                ..candidate("a")
            },
        ];
        ordenar(&mut lista, Ordem::Recencia, agora);
        assert_eq!(lista[0].command, "a");
    }

    #[test]
    fn recent_beats_old_at_equal_use() {
        let now = 10 * DAY_MS as i64;
        let mut fresh = candidate("a");
        fresh.last_used_at_ms = now;
        let mut old = candidate("b");
        old.last_used_at_ms = now - 7 * DAY_MS as i64;
        assert!(frecency(now, &fresh) > frecency(now, &old));
    }

    #[test]
    fn frequent_beats_rare_at_equal_age() {
        let now = DAY_MS as i64;
        let mut often = candidate("a");
        often.uses = 40;
        often.successes = 40;
        often.last_used_at_ms = now;
        let mut once = candidate("b");
        once.last_used_at_ms = now;
        assert!(frecency(now, &often) > frecency(now, &once));
    }

    #[test]
    fn command_that_only_ever_failed_is_demoted() {
        let now = DAY_MS as i64;
        let mut broken = candidate("a");
        broken.successes = 0;
        broken.last_used_at_ms = now;
        let mut works = candidate("b");
        works.last_used_at_ms = now;
        assert!(frecency(now, &broken) < frecency(now, &works));
    }

    /// Histórico importado de bash e fish não traz exit code. Sem esta regra o
    /// corpus inteiro nasceria valendo metade, como se só tivesse falhado.
    #[test]
    fn command_without_known_exit_code_is_not_demoted() {
        let now = DAY_MS as i64;
        let mut unknown = candidate("a");
        unknown.uses = 4;
        unknown.successes = 0;
        unknown.known_exit_codes = 0;
        unknown.last_used_at_ms = now;
        let mut works = candidate("b");
        works.uses = 4;
        works.successes = 4;
        works.known_exit_codes = 4;
        works.last_used_at_ms = now;
        assert_eq!(frecency(now, &unknown), frecency(now, &works));
    }

    #[test]
    fn unknown_codes_mixed_with_failures_stay_demoted() {
        let now = DAY_MS as i64;
        let mut broken = candidate("a");
        broken.uses = 5;
        broken.successes = 0;
        broken.known_exit_codes = 2;
        broken.last_used_at_ms = now;
        let mut works = candidate("b");
        works.uses = 5;
        works.successes = 5;
        works.known_exit_codes = 5;
        works.last_used_at_ms = now;
        assert_eq!(frecency(now, &broken), frecency(now, &works) * 0.5);
    }

    #[test]
    fn one_known_success_among_unknowns_is_not_demoted() {
        let now = DAY_MS as i64;
        let mut mixed = candidate("a");
        mixed.uses = 5;
        mixed.successes = 1;
        mixed.known_exit_codes = 2;
        mixed.last_used_at_ms = now;
        let mut works = candidate("b");
        works.uses = 5;
        works.successes = 5;
        works.known_exit_codes = 5;
        works.last_used_at_ms = now;
        assert_eq!(frecency(now, &mixed), frecency(now, &works));
    }

    #[test]
    fn same_directory_outranks_same_repo_outranks_anywhere() {
        let now = DAY_MS as i64;
        let mut here = candidate("a");
        here.in_cwd = true;
        here.in_repo = true;
        here.last_used_at_ms = now;
        let mut repo = candidate("b");
        repo.in_repo = true;
        repo.last_used_at_ms = now;
        let mut anywhere = candidate("c");
        anywhere.last_used_at_ms = now;
        assert!(frecency(now, &here) > frecency(now, &repo));
        assert!(frecency(now, &repo) > frecency(now, &anywhere));
    }

    #[test]
    fn future_timestamp_never_produces_a_negative_score() {
        let now = 0;
        let mut skewed = candidate("a");
        skewed.last_used_at_ms = 10 * DAY_MS as i64;
        assert!(frecency(now, &skewed) > 0.0);
    }
}
