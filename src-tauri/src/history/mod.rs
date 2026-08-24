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
pub fn frecency(now_ms: i64, candidate: &HistoryCandidate) -> f64 {
    let age_days = ((now_ms - candidate.last_used_at_ms).max(0) as f64) / DAY_MS;
    let recency = 1.0 / (1.0 + age_days);
    let frequency = 1.0 + (1.0 + candidate.uses as f64).ln();
    let success = if candidate.known_exit_codes > 0 && candidate.successes == 0 {
        0.5
    } else {
        1.0
    };
    let scope =
        1.0 + if candidate.in_cwd { 1.0 } else { 0.0 } + if candidate.in_repo { 0.5 } else { 0.0 };
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
        }
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
