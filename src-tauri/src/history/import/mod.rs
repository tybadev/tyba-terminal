//! Import do histórico que o usuário já tem no shell.
//!
//! O motor de frecência nasce sem dado: quem instala o TYBA tem anos em
//! `~/.zsh_history` e abre uma paleta vazia. Aqui esse histórico entra.

//! Nada aqui registra texto de comando em log. O arquivo lido é o que o dono da
//! máquina digitou por anos, e log vaza para lugar que a redação não cobre.

pub mod parser;
pub mod source;

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use sha2::{Digest, Sha256};

use source::{ImportSource, ResolvedSource};

use crate::session::redact::redact;
use crate::session::store::Store;

/// Entradas por transação. O lock do store nunca fica preso pelo arquivo
/// inteiro, e o que já commitou sobrevive a um fechamento no meio do import.
const BATCH: usize = 1_000;

/// Progresso do import para o webview. Sai por evento, não pelo retorno: o
/// retorno só chega quando tudo acabou.
pub const EVENT_PROGRESS: &str = "history:import-progress";

/// Uma entrada lida de arquivo de histórico, antes de virar linha no banco.
///
/// `exit_code` não aparece: nenhuma das fontes de texto grava código, e nulo é
/// o que a frecência trata como desconhecido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEntry {
    pub command: String,
    pub started_at_ms: i64,
    pub duration_ms: Option<i64>,
}

/// Uma entrada pronta para gravar: comando já redigido e chave já calculada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRow {
    pub command: String,
    pub started_at_ms: i64,
    pub duration_ms: Option<i64>,
    pub import_key: String,
}

/// A chave que impede a duplicata ao reimportar.
///
/// **A posição da entrada no arquivo fica de fora, de propósito.** Incluí-la
/// faria o reimport duplicar tudo depois que o zsh apara o arquivo por
/// `SAVEHIST`, porque as posições andam. Deixá-la de fora custa perder uma
/// segunda ocorrência idêntica no mesmo segundo — uma unidade de contagem em um
/// comando que já vai aparecer. Perder contagem é barato; duplicar o corpus
/// corrompe o ranking inteiro.
///
/// O comando entra **já redigido**: a chave precisa casar com o que está no
/// banco, e o que está no banco passou pela redação.
pub fn import_key(source: ImportSource, redacted_command: &str, started_at_ms: i64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(redacted_command.as_bytes());
    hasher.update([0x1f]);
    hasher.update(started_at_ms.to_string().as_bytes());
    format!("{}:{:x}", source.key(), hasher.finalize())
}

/// O que aconteceu com uma fonte. Sem texto de comando: este relatório atravessa
/// o IPC e aparece na tela.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceOutcome {
    pub source: ImportSource,
    pub path: String,
    pub read: usize,
    pub imported: usize,
    pub discarded: usize,
    /// Preenchido quando a fonte inteira foi pulada, com o motivo.
    pub skipped: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub sources: Vec<SourceOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub source: ImportSource,
    pub imported: usize,
    pub total: usize,
}

#[derive(Debug)]
pub enum ImportError {
    AlreadyRunning,
    Store(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::AlreadyRunning => write!(f, "history_import_already_running"),
            ImportError::Store(error) => write!(f, "{error}"),
        }
    }
}

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Garante que a marca de "rodando" cai mesmo se o import falhar no meio.
struct RunningGuard;

impl RunningGuard {
    fn acquire() -> Option<Self> {
        RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| RunningGuard)
    }
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

/// Importa as fontes, uma de cada vez.
///
/// Um import por vez, e o segundo é **recusado**, não enfileirado: enfileirar
/// esconderia do usuário que ele pediu duas vezes.
pub fn run(
    store: &Store,
    sources: &[ResolvedSource],
    on_progress: &mut dyn FnMut(Progress),
) -> Result<ImportReport, ImportError> {
    run_batched(store, sources, BATCH, on_progress)
}

fn run_batched(
    store: &Store,
    sources: &[ResolvedSource],
    batch: usize,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<ImportReport, ImportError> {
    let _guard = RunningGuard::acquire().ok_or(ImportError::AlreadyRunning)?;

    let mut report = ImportReport {
        sources: Vec::with_capacity(sources.len()),
    };
    for resolved in sources {
        report
            .sources
            .push(import_one(store, resolved, batch, on_progress)?);
    }
    store
        .evict_command_history()
        .map_err(|error| ImportError::Store(error.to_string()))?;
    Ok(report)
}

/// Fonte quebrada não derruba o import: ela é pulada com o motivo e as outras
/// seguem. Erro de banco, sim, aborta — insistir depois dele é escrever no
/// escuro.
fn import_one(
    store: &Store,
    resolved: &ResolvedSource,
    batch: usize,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<SourceOutcome, ImportError> {
    let path = resolved.path.to_string_lossy().into_owned();
    let mut outcome = SourceOutcome {
        source: resolved.source,
        path,
        read: 0,
        imported: 0,
        discarded: 0,
        skipped: None,
    };

    let total = match source::scan(resolved) {
        Ok(scan) => scan.entries,
        Err(error) => {
            outcome.skipped = Some(error.to_string());
            return Ok(outcome);
        }
    };

    let mut pending: Vec<ImportRow> = Vec::with_capacity(batch.min(total.max(1)));
    let mut rows_read = 0usize;
    let mut flushed = 0usize;
    let mut store_error: Option<String> = None;

    let read = source::read(resolved, total, &mut |entry| {
        if store_error.is_some() {
            return;
        }
        rows_read += 1;
        // `ignorespace` vale no import como vale na captura ao vivo: linha
        // iniciada por espaço é o usuário dizendo "não guarde este".
        if !crate::history::should_record(&entry.command) {
            return;
        }
        let command = redact(entry.command.trim()).into_owned();
        let import_key = import_key(resolved.source, &command, entry.started_at_ms);
        pending.push(ImportRow {
            command,
            started_at_ms: entry.started_at_ms,
            duration_ms: entry.duration_ms,
            import_key,
        });
        if pending.len() >= batch {
            match store.insert_imported_batch(&pending) {
                Ok(inserted) => {
                    flushed += inserted;
                    on_progress(Progress {
                        source: resolved.source,
                        imported: flushed,
                        total,
                    });
                }
                Err(error) => store_error = Some(error.to_string()),
            }
            pending.clear();
        }
    });

    if let Some(error) = store_error {
        return Err(ImportError::Store(error));
    }
    let discarded_by_parser = match read {
        Ok(discarded) => discarded,
        Err(error) => {
            outcome.skipped = Some(error.to_string());
            return Ok(outcome);
        }
    };
    if !pending.is_empty() {
        flushed += store
            .insert_imported_batch(&pending)
            .map_err(|error| ImportError::Store(error.to_string()))?;
    }

    outcome.read = rows_read + discarded_by_parser;
    outcome.imported = flushed;
    outcome.discarded = outcome.read - flushed;
    on_progress(Progress {
        source: resolved.source,
        imported: flushed,
        total,
    });
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    /// O guarda de "um import por vez" é do processo, e o `cargo test` roda os
    /// testes em paralelo dentro dele: sem serializar, um teste recusaria o
    /// import do outro. A serialização é do teste, não do produto.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn home_with(name: &str, contents: &str) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
        home
    }

    fn sources_of(home: &Path) -> Vec<ResolvedSource> {
        source::resolve(home, &|_| None)
    }

    fn import(store: &Store, home: &Path) -> ImportReport {
        run(store, &sources_of(home), &mut |_| {}).expect("import")
    }

    fn commands(store: &Store) -> Vec<String> {
        store
            .history_candidates(None, None, None)
            .unwrap()
            .into_iter()
            .map(|candidate| candidate.command)
            .collect()
    }

    /// Fixture sintética: o segredo é inventado aqui, nunca copiado de arquivo
    /// de alguém.
    #[test]
    fn a_secret_in_the_history_file_is_redacted_before_it_lands() {
        let _serial = serial();
        let home = home_with(
            ".zsh_history",
            ": 1:0;export TOKEN=sk-abcdefghijklmnopqrstuvwxyz0123456789ABCD\n",
        );
        let store = Store::open_in_memory().unwrap();
        import(&store, home.path());

        let found = commands(&store);
        assert_eq!(found.len(), 1);
        assert!(!found[0].contains("sk-abcdefghijklmnopqrstuvwxyz0123456789ABCD"));
        assert!(found[0].contains("[REDACTED]"));
    }

    #[test]
    fn a_command_starting_with_a_space_is_not_imported() {
        let _serial = serial();
        let home = home_with(".zsh_history", ": 1:0; export TOKEN=abc\n: 2:0;pwd\n");
        let store = Store::open_in_memory().unwrap();
        let report = import(&store, home.path());

        assert_eq!(commands(&store), vec!["pwd"]);
        assert_eq!(report.sources[0].read, 2);
        assert_eq!(report.sources[0].imported, 1);
        assert_eq!(report.sources[0].discarded, 1);
    }

    #[test]
    fn the_report_counts_read_imported_and_discarded_per_source() {
        let _serial = serial();
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join(".zsh_history"), ": 1:0;ls\n: 2:0;pwd\n").unwrap();
        fs::write(home.path().join(".bash_history"), "cargo test\n").unwrap();
        let store = Store::open_in_memory().unwrap();
        let report = import(&store, home.path());

        assert_eq!(report.sources.len(), 2);
        assert_eq!(report.sources[0].source, ImportSource::Zsh);
        assert_eq!(report.sources[0].read, 2);
        assert_eq!(report.sources[0].imported, 2);
        assert_eq!(report.sources[1].source, ImportSource::Bash);
        assert_eq!(report.sources[1].imported, 1);
        assert!(report.sources.iter().all(|s| s.skipped.is_none()));
    }

    /// Fonte quebrada não pode derrubar o import inteiro.
    #[test]
    fn an_unreadable_source_is_skipped_and_the_others_continue() {
        let _serial = serial();
        let home = tempfile::tempdir().unwrap();
        let broken = home.path().join(".zsh_history");
        fs::write(&broken, ": 1:0;ls\n").unwrap();
        fs::write(home.path().join(".bash_history"), "cargo test\n").unwrap();
        let store = Store::open_in_memory().unwrap();

        let sources = sources_of(home.path());
        // Some com o arquivo depois de resolvido: é o que acontece quando o
        // usuário apaga ou troca a permissão entre a contagem e o import.
        fs::remove_file(&broken).unwrap();
        let report = run(&store, &sources, &mut |_| {}).expect("import");

        assert!(report.sources[0].skipped.is_some());
        assert_eq!(report.sources[0].imported, 0);
        assert_eq!(report.sources[1].imported, 1);
        assert_eq!(commands(&store), vec!["cargo test"]);
    }

    #[test]
    fn a_second_import_while_one_runs_is_refused() {
        let _serial = serial();
        let home = home_with(".zsh_history", ": 1:0;ls\n");
        let store = Store::open_in_memory().unwrap();
        let sources = sources_of(home.path());

        let mut nested = Ok(ImportReport { sources: vec![] });
        run(&store, &sources, &mut |_| {
            if nested.is_ok() {
                nested = run(&store, &sources, &mut |_| {});
            }
        })
        .expect("import");

        assert!(matches!(nested, Err(ImportError::AlreadyRunning)));
    }

    /// O lote existe para não segurar o lock do store pelo arquivo inteiro; o
    /// resultado não pode depender do tamanho dele.
    #[test]
    fn a_file_larger_than_one_batch_is_imported_whole() {
        let _serial = serial();
        let mut file = String::new();
        for i in 0..5 {
            file.push_str(&format!(": {i}:0;comando{i}\n"));
        }
        let home = home_with(".zsh_history", &file);
        let store = Store::open_in_memory().unwrap();

        let report = run_batched(&store, &sources_of(home.path()), 2, &mut |_| {}).expect("import");
        assert_eq!(report.sources[0].imported, 5);
        assert_eq!(commands(&store).len(), 5);
    }

    /// O caso real do reimport: semanas depois, o arquivo cresceu com o que foi
    /// digitado fora do TYBA. Só isso pode entrar.
    #[test]
    fn a_file_that_grew_since_the_last_import_brings_only_the_new_entries() {
        let _serial = serial();
        let home = home_with(".zsh_history", ": 1:0;ls\n");
        let store = Store::open_in_memory().unwrap();
        assert_eq!(import(&store, home.path()).sources[0].imported, 1);

        let path = home.path().join(".zsh_history");
        let grown = fs::read_to_string(&path).unwrap() + ": 2:0;pwd\n";
        fs::write(&path, grown).unwrap();

        let second = import(&store, home.path());
        assert_eq!(second.sources[0].read, 2);
        assert_eq!(second.sources[0].imported, 1);
        assert_eq!(commands(&store).len(), 2);
    }

    #[test]
    fn importing_twice_does_not_duplicate() {
        let _serial = serial();
        let home = home_with(".zsh_history", ": 1:0;ls\n: 2:0;pwd\n");
        let store = Store::open_in_memory().unwrap();

        let first = import(&store, home.path());
        let second = import(&store, home.path());
        assert_eq!(first.sources[0].imported, 2);
        assert_eq!(second.sources[0].imported, 0);
        assert_eq!(commands(&store).len(), 2);
    }
}
