//! Resumo do turno: o que mudou na pasta entre o prompt do usuário
//! (`UserPromptSubmit` do fio principal) e o fim do turno (`Stop`,
//! `StopFailure`, `Interrupt`). Sem LLM: só git, relógio e o uso que o
//! `statusLine` reportou.
//!
//! A comparação é com o estado da pasta na hora do prompt, não com uma
//! branch base: o que já estava sujo antes fica de fora, o que mudou depois
//! entra — inclusive o que o próprio usuário editou no meio, porque o git
//! não sabe quem escreveu. O resumo carrega caminhos e contagens, nunca
//! conteúdo de arquivo, e não vai para o SQLite.
//!
//! O snapshot é o mesmo do Orbit ([`crate::orbit::agent::snapshot`]): um
//! `git stash create`, que grava objetos soltos no repositório do usuário
//! (sem ref nenhuma; o `git gc` leva) e não mexe na árvore nem no stash.
//!
//! Limite conhecido: arquivo que já era não rastreado no prompt e foi
//! alterado durante o turno não aparece — o snapshot não guarda o conteúdo
//! dele, só o nome. Pelo mesmo motivo, se o turno o adiciona ao git ele
//! também fica de fora (não dá para separar o que já existia do que mudou).

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::agent::usage::AgentUsage;
use crate::orbit::agent::Snapshot;
use crate::worktree::diff::{diff_files, untracked_files, FileDiff, FileStatus};
use crate::worktree::{git_in, run_git};

/// Teto do snapshot. O hook do agente livre não espera o core (`tyba _hook
/// --observe` manda e sai), então o snapshot corre junto com o começo do
/// turno; o que protege é a folga até a primeira ferramenta do agente. Um
/// snapshot que demora mais que isto pode já ter engolido edição do turno:
/// vale menos que nenhum, e o resumo sai sem diff.
pub const SNAPSHOT_MAX: Duration = Duration::from_millis(2000);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    Failed,
    Interrupted,
}

/// O que vai ao webview em `Session::last_turn`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TurnDigest {
    pub outcome: TurnOutcome,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub duration_ms: u64,
    /// Redigido e cortado.
    pub prompt: Option<String>,
    /// `false` = sem diff: a pasta não é um repositório git (ou não tem
    /// commit), ou o snapshot não ficou pronto a tempo. Só duração e custo.
    pub git: bool,
    /// Lista inteira, ordenada por caminho; quem corta é a UI.
    pub files: Vec<FileDiff>,
    pub insertions: u32,
    pub deletions: u32,
    pub commits: u32,
    /// Custo e tokens DO TURNO. `None` quando o agente não reporta uso (Codex).
    pub cost_usd: Option<f64>,
    pub tokens: Option<u64>,
}

/// O repositório na hora do prompt.
#[derive(Debug, Clone)]
pub struct TurnRepo {
    snapshot: Snapshot,
    head: String,
}

#[derive(Debug, Clone)]
pub struct TurnStart {
    pub started_at_ms: u64,
    pub prompt: Option<String>,
    /// Uso acumulado da sessão até o prompt.
    pub usage: Option<AgentUsage>,
    /// `None` fora de um repositório git.
    pub repo: Option<TurnRepo>,
}

pub fn capture_repo(dir: &Path) -> Option<TurnRepo> {
    let snapshot = crate::orbit::agent::snapshot(dir)?;
    let head = crate::worktree::head_sha(&snapshot.root).ok()?;
    Some(TurnRepo { snapshot, head })
}

/// Rastreados: snapshot contra a árvore de trabalho (pega o que foi
/// commitado no turno e o que ficou solto). Não rastreados: só os que
/// surgiram depois do prompt.
fn changed_files(repo: &TurnRepo) -> Result<Vec<FileDiff>, String> {
    let snap = &repo.snapshot;
    let mut files: Vec<FileDiff> = diff_files(&snap.root, &[&snap.base, "--"])?
        .into_iter()
        .filter(|f| !(f.status == FileStatus::Added && snap.untracked.contains(&f.path)))
        .collect();
    // `git rm --cached` no turno: o arquivo já está na lista como apagado.
    let fresh: Vec<FileDiff> = untracked_files(&snap.root)?
        .into_iter()
        .filter(|f| !snap.untracked.contains(&f.path))
        .filter(|f| files.iter().all(|t| t.path != f.path))
        .collect();
    files.extend(fresh);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Commits feitos desde o prompt. `--ancestry-path` zera quando o `HEAD`
/// voltou ou foi para uma história que não descende do `HEAD` do prompt.
fn commits_since(repo: &TurnRepo) -> u32 {
    let mut cmd = git_in(&repo.snapshot.root);
    cmd.args(["rev-list", "--count", "--ancestry-path"]);
    cmd.arg(format!("{}..HEAD", repo.head));
    run_git(cmd, "git rev-list")
        .ok()
        .and_then(|out| String::from_utf8_lossy(&out).trim().parse().ok())
        .unwrap_or(0)
}

/// Contador acumulado → o que é do turno. Sem leitura no prompt o turno é o
/// primeiro da conversa (base zero); contador que diminuiu foi zerado no
/// meio (`/clear`), e o que há é o que veio depois.
fn delta<T: PartialOrd + std::ops::Sub<Output = T> + Default>(
    start: Option<T>,
    end: Option<T>,
) -> Option<T> {
    let end = end?;
    let start = start.unwrap_or_default();
    Some(if end >= start { end - start } else { end })
}

fn tokens(usage: &AgentUsage) -> Option<u64> {
    match (usage.tokens_in, usage.tokens_out) {
        (None, None) => None,
        (i, o) => Some(i.unwrap_or(0) + o.unwrap_or(0)),
    }
}

pub fn digest(
    start: &TurnStart,
    outcome: TurnOutcome,
    ended_at_ms: u64,
    usage_end: Option<&AgentUsage>,
) -> TurnDigest {
    let changed = start
        .repo
        .as_ref()
        .and_then(|repo| Some((changed_files(repo).ok()?, commits_since(repo))));
    let git = changed.is_some();
    let (files, commits) = changed.unwrap_or_default();
    let sum = |pick: fn(&FileDiff) -> Option<u32>| files.iter().filter_map(pick).sum();
    let at_start = start.usage.as_ref();
    TurnDigest {
        outcome,
        started_at_ms: start.started_at_ms,
        ended_at_ms,
        duration_ms: ended_at_ms.saturating_sub(start.started_at_ms),
        prompt: start.prompt.clone(),
        git,
        insertions: sum(|f| f.insertions),
        deletions: sum(|f| f.deletions),
        files,
        commits,
        cost_usd: delta(
            at_start.and_then(|u| u.cost_usd),
            usage_end.and_then(|u| u.cost_usd),
        ),
        tokens: delta(at_start.and_then(tokens), usage_end.and_then(tokens)),
    }
}

/// Um turno aberto: o prompt chegou, o fim ainda não.
pub struct OpenTurn {
    started_at_ms: u64,
    prompt: Option<String>,
    usage: Option<AgentUsage>,
    repo: mpsc::Receiver<Option<TurnRepo>>,
}

impl OpenTurn {
    /// Espera o snapshot (no máximo [`SNAPSHOT_MAX`]). Bloqueia: nunca na
    /// thread que responde o hook.
    pub fn into_start(self) -> TurnStart {
        TurnStart {
            started_at_ms: self.started_at_ms,
            prompt: self.prompt,
            usage: self.usage,
            repo: self.repo.recv_timeout(SNAPSHOT_MAX).ok().flatten(),
        }
    }
}

/// O turno aberto de uma sessão. Sem começo registrado (o TYBA entrou no
/// meio do turno, ou a CLI não manda `UserPromptSubmit`) não há resumo.
#[derive(Default)]
pub struct TurnTracker(Mutex<Option<OpenTurn>>);

impl TurnTracker {
    /// Registra o começo e dispara o snapshot numa thread à parte. Um turno
    /// que ainda estava aberto é descartado: vale o prompt mais recente.
    pub fn begin(
        &self,
        dir: PathBuf,
        started_at_ms: u64,
        prompt: Option<String>,
        usage: Option<AgentUsage>,
    ) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let clock = Instant::now();
            let repo = capture_repo(&dir).filter(|_| clock.elapsed() <= SNAPSHOT_MAX);
            let _ = tx.send(repo);
        });
        *self.0.lock().expect("turn lock") = Some(OpenTurn {
            started_at_ms,
            prompt,
            usage,
            repo: rx,
        });
    }

    /// Fecha o turno aberto, se houver.
    pub fn end(&self) -> Option<OpenTurn> {
        self.0.lock().expect("turn lock").take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Repositório com um commit: `a.txt`, `b.txt`, `some.txt` e um
    /// `.gitignore` que ignora `*.log`. A config global do dono não entra
    /// (`hermetic_git_config` em `lib.rs`).
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        git(p, &["init", "-q"]);
        git(p, &["config", "user.email", "t@example.com"]);
        git(p, &["config", "user.name", "t"]);
        write(p, "a.txt", "um\ndois\ntres\n");
        write(p, "b.txt", "conteudo estavel do b\noutra linha\n");
        write(p, "some.txt", "vai embora\n");
        write(p, ".gitignore", "*.log\n");
        git(p, &["add", "-A"]);
        git(p, &["commit", "-qm", "base"]);
        dir
    }

    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    fn start(dir: &Path) -> TurnStart {
        TurnStart {
            started_at_ms: 1_000,
            prompt: Some("arruma o a.txt".into()),
            usage: None,
            repo: capture_repo(dir),
        }
    }

    fn end(start: &TurnStart) -> TurnDigest {
        digest(start, TurnOutcome::Completed, 4_500, None)
    }

    /// (caminho, status, +, -) de cada arquivo.
    fn rows(d: &TurnDigest) -> Vec<(&str, FileStatus, Option<u32>, Option<u32>)> {
        d.files
            .iter()
            .map(|f| (f.path.as_str(), f.status, f.insertions, f.deletions))
            .collect()
    }

    #[test]
    fn nothing_changed_gives_an_empty_list() {
        let dir = repo();
        let d = end(&start(dir.path()));
        assert!(d.git);
        assert!(d.files.is_empty());
        assert_eq!((d.insertions, d.deletions, d.commits), (0, 0, 0));
        assert_eq!(d.duration_ms, 3_500);
        assert_eq!(d.prompt.as_deref(), Some("arruma o a.txt"));
    }

    #[test]
    fn what_was_dirty_before_the_prompt_stays_out() {
        let dir = repo();
        let p = dir.path();
        write(p, "a.txt", "um\ndois\ntres\nantes\n");
        write(p, "velho.txt", "ja estava aqui\n");
        let s = start(p);
        assert!(end(&s).files.is_empty());

        write(p, "a.txt", "um\ndois\ntres\nantes\ndepois\n");
        let d = end(&s);
        assert_eq!(
            rows(&d),
            [("a.txt", FileStatus::Modified, Some(1), Some(0))]
        );
    }

    #[test]
    fn modified_file_counts_its_lines() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        write(p, "a.txt", "um\nDOIS\ntres\nquatro\n");
        let d = end(&s);
        assert_eq!(
            rows(&d),
            [("a.txt", FileStatus::Modified, Some(2), Some(1))]
        );
        assert_eq!((d.insertions, d.deletions), (2, 1));
    }

    #[test]
    fn new_untracked_file_counts_and_ignored_does_not() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        write(p, "novo.txt", "linha 1\nlinha 2\n");
        write(p, "saida.log", "ignorado\n");
        let d = end(&s);
        assert_eq!(
            rows(&d),
            [("novo.txt", FileStatus::Added, Some(2), Some(0))]
        );
    }

    #[test]
    fn deleted_file_counts() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        std::fs::remove_file(p.join("some.txt")).unwrap();
        let d = end(&s);
        assert_eq!(
            rows(&d),
            [("some.txt", FileStatus::Deleted, Some(0), Some(1))]
        );
    }

    #[test]
    fn rename_keeps_the_old_path() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        git(p, &["mv", "b.txt", "c.txt"]);
        let d = end(&s);
        assert_eq!(rows(&d), [("c.txt", FileStatus::Renamed, Some(0), Some(0))]);
        assert_eq!(d.files[0].old_path.as_deref(), Some("b.txt"));
    }

    #[test]
    fn commit_made_during_the_turn_still_counts_its_files() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        write(p, "novo.txt", "oi\n");
        git(p, &["add", "-A"]);
        git(p, &["commit", "-qm", "do turno"]);
        let d = end(&s);
        assert_eq!(d.commits, 1);
        assert_eq!(
            rows(&d),
            [
                ("a.txt", FileStatus::Modified, Some(1), Some(0)),
                ("novo.txt", FileStatus::Added, Some(1), Some(0)),
            ]
        );
    }

    /// Com a árvore suja o snapshot é um commit solto do `stash create`, não
    /// o `HEAD`: a contagem de commits tem de sair do `HEAD` do prompt.
    #[test]
    fn commit_count_holds_with_a_dirty_tree_at_the_prompt() {
        let dir = repo();
        let p = dir.path();
        write(p, "a.txt", "um\ndois\ntres\nantes\n");
        let s = start(p);
        write(p, "b.txt", "conteudo estavel do b\noutra linha\nmais uma\n");
        git(p, &["commit", "-qm", "so o b", "b.txt"]);
        let d = end(&s);
        assert_eq!(d.commits, 1);
        assert_eq!(
            rows(&d),
            [("b.txt", FileStatus::Modified, Some(1), Some(0))]
        );
    }

    #[test]
    fn head_that_went_back_counts_no_commit() {
        let dir = repo();
        let p = dir.path();
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        git(p, &["commit", "-qam", "segundo"]);
        let s = start(p);
        git(p, &["reset", "-q", "--hard", "HEAD~1"]);
        let d = end(&s);
        assert_eq!(d.commits, 0);
        assert_eq!(
            rows(&d),
            [("a.txt", FileStatus::Modified, Some(0), Some(1))]
        );
    }

    /// Limite conhecido (ver o cabeçalho do módulo).
    #[test]
    fn file_untracked_at_the_prompt_never_shows_up() {
        let dir = repo();
        let p = dir.path();
        write(p, "velho.txt", "ja estava aqui\n");
        let s = start(p);
        write(p, "velho.txt", "ja estava aqui\nmudou no turno\n");
        assert!(end(&s).files.is_empty());
        git(p, &["add", "velho.txt"]);
        assert!(end(&s).files.is_empty());
    }

    #[test]
    fn folder_without_git_has_no_diff() {
        let dir = tempfile::tempdir().unwrap();
        let s = start(dir.path());
        write(dir.path(), "novo.txt", "oi\n");
        let d = end(&s);
        assert!(!d.git);
        assert!(d.files.is_empty());
        assert_eq!(d.duration_ms, 3_500);
    }

    fn usage(cost: f64, tokens_in: u64, tokens_out: u64) -> AgentUsage {
        AgentUsage {
            cost_usd: Some(cost),
            tokens_in: Some(tokens_in),
            tokens_out: Some(tokens_out),
            ..Default::default()
        }
    }

    fn usage_digest(at_start: Option<AgentUsage>, at_end: Option<AgentUsage>) -> TurnDigest {
        let s = TurnStart {
            started_at_ms: 0,
            prompt: None,
            usage: at_start,
            repo: None,
        };
        digest(&s, TurnOutcome::Failed, 10, at_end.as_ref())
    }

    #[test]
    fn usage_is_the_delta_of_the_turn() {
        let d = usage_digest(Some(usage(0.5, 1_000, 200)), Some(usage(0.75, 1_600, 300)));
        assert_eq!(d.cost_usd, Some(0.25));
        assert_eq!(d.tokens, Some(700));
        assert_eq!(d.outcome, TurnOutcome::Failed);
    }

    #[test]
    fn usage_edges() {
        // Codex: ninguém reporta uso.
        let d = usage_digest(None, None);
        assert_eq!((d.cost_usd, d.tokens), (None, None));
        // Primeiro turno da conversa: base zero.
        let d = usage_digest(None, Some(usage(0.25, 100, 20)));
        assert_eq!((d.cost_usd, d.tokens), (Some(0.25), Some(120)));
        // Contador zerado no meio (`/clear`).
        let d = usage_digest(Some(usage(2.0, 9_000, 900)), Some(usage(0.25, 100, 20)));
        assert_eq!((d.cost_usd, d.tokens), (Some(0.25), Some(120)));
    }

    #[test]
    fn turn_end_without_a_start_gives_nothing() {
        let tracker = TurnTracker::default();
        assert!(tracker.end().is_none());
    }

    #[test]
    fn tracker_hands_the_open_turn_over_once() {
        let dir = repo();
        let tracker = TurnTracker::default();
        tracker.begin(dir.path().to_path_buf(), 7, Some("oi".into()), None);
        let s = tracker.end().expect("turno aberto").into_start();
        assert_eq!(s.started_at_ms, 7);
        assert_eq!(s.prompt.as_deref(), Some("oi"));
        assert!(s.repo.is_some());
        assert!(tracker.end().is_none());
    }

    #[test]
    fn digest_serializes_paths_and_counts_only() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nsegredo do arquivo\n");
        let json = serde_json::to_value(end(&s)).unwrap();
        assert_eq!(json["outcome"], "completed");
        assert_eq!(json["files"][0]["path"], "a.txt");
        assert_eq!(json["files"][0]["status"], "modified");
        assert!(!json.to_string().contains("segredo do arquivo"));
    }
}
