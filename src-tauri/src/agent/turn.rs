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
//! Um snapshot é uma ÁRVORE do git com a pasta inteira (rastreados e não
//! rastreados, sem os ignorados), escrita por um índice TEMPORÁRIO: o índice
//! de verdade do repositório não é escrito nem travado, então o snapshot não
//! disputa `index.lock` com o agente e funciona no meio de um merge. Há um no
//! prompt e outro no fim do turno, e o diff é de árvore contra árvore — não
//! roda filtro de conteúdo nem lê a pasta de novo.
//!
//! O que fica no repositório do usuário: objetos soltos (os blobs do que
//! estava modificado ou não rastreado e as árvores), sem ref nenhuma; o
//! `git gc` leva.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::agent::usage::AgentUsage;
use crate::session::SessionId;
use crate::worktree::diff::{diff_files, FileDiff};
use crate::worktree::{git_in, git_in_rw, run_git};

/// Teto de um snapshot, do primeiro ao último `git` dele. Passou, o filho é
/// morto e o turno fica sem diff.
///
/// No prompt o teto também é o que dá sentido ao snapshot: o hook do agente
/// livre não espera o core (`tyba _hook --observe` manda e sai), então o
/// snapshot corre junto com o começo do turno, e o que protege é a folga até
/// a primeira ferramenta do agente. No fim vale o mesmo número: quem tirou o
/// do prompt a tempo tira o do fim, e um fim que demora mais já estaria
/// medindo o turno seguinte.
pub const SNAPSHOT_MAX: Duration = Duration::from_millis(2000);

/// Teto da lista que vai ao webview: ela viaja em todo `session://status`.
pub const FILES_MAX: usize = 200;

const TEMP_INDEX_PREFIX: &str = "tyba-turn-index-";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnDiff {
    Ok,
    /// A pasta não é um repositório git: só duração e custo.
    NoRepo,
    /// É um repositório, mas um dos snapshots falhou ou estourou o teto.
    Unavailable,
}

/// O que vai ao webview em `Session::last_turn`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TurnDigest {
    /// Ordem dos turnos da sessão: um resumo nunca troca outro de `seq` maior.
    pub seq: u64,
    pub outcome: TurnOutcome,
    /// Motivo da falha (`outcome == failed`), o mesmo texto do status.
    pub failure: Option<String>,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    pub duration_ms: u64,
    /// Redigido e cortado.
    pub prompt: Option<String>,
    pub diff: TurnDiff,
    /// Ordenada por caminho, cortada em [`FILES_MAX`].
    pub files: Vec<FileDiff>,
    pub files_total: u32,
    /// Totais de TODOS os arquivos, não só dos listados.
    pub insertions: u32,
    pub deletions: u32,
    pub commits: u32,
    /// Custo e tokens DO TURNO. `None` quando o agente não reporta uso (Codex).
    pub cost_usd: Option<f64>,
    pub tokens: Option<u64>,
}

impl TurnDigest {
    /// Troca custo e tokens pelo que o uso diz agora (o `statusLine` do fim
    /// do turno chega depois do `Stop`).
    pub fn with_usage(
        mut self,
        at_start: Option<&AgentUsage>,
        at_end: Option<&AgentUsage>,
    ) -> Self {
        self.cost_usd = delta(
            at_start.and_then(|u| u.cost_usd),
            at_end.and_then(|u| u.cost_usd),
        );
        self.tokens = delta(at_start.and_then(tokens), at_end.and_then(tokens));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoTree {
    pub root: PathBuf,
    pub tree: String,
    /// `None` em repositório sem commit.
    pub head: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Snapshot {
    NoRepo,
    Unavailable,
    Tree(RepoTree),
}

enum Ran {
    Out(Vec<u8>),
    Failed(Option<i32>),
    TimedOut,
}

#[cfg(unix)]
fn kill_group(child: &std::process::Child) {
    // pgid == pid do líder (`process_group(0)`).
    unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) };
}

#[cfg(not(unix))]
fn kill_group(_child: &std::process::Child) {}

/// Roda o comando até o prazo; passou, mata o grupo inteiro (princípio 9) e
/// só volta com o filho recolhido. A saída esperada é curta (um sha, dois
/// caminhos): cabe no pipe e é lida depois que o filho sai. `capture` fica
/// de fora no `add`, o único que roda filtro: um filtro que ficasse para
/// trás seguraria o pipe e a leitura junto.
fn run_within(mut cmd: Command, deadline: Instant, capture: bool) -> Ran {
    if Instant::now() >= deadline {
        return Ran::TimedOut;
    }
    cmd.stdout(if capture {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let Ok(mut child) = cmd.spawn() else {
        return Ran::Failed(None);
    };
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            _ => break None,
        }
    };
    let Some(status) = status else {
        // Antes do `wait`: com o líder ainda por recolher o pgid é dele.
        kill_group(&child);
        let _ = child.kill();
        let _ = child.wait();
        return Ran::TimedOut;
    };
    if !status.success() {
        return Ran::Failed(status.code());
    }
    let mut out = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = std::io::Read::read_to_end(&mut stdout, &mut out);
    }
    Ran::Out(out)
}

/// O índice temporário, apagado em qualquer saída (com o `.lock` que o git
/// cria ao lado enquanto escreve).
struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let mut lock = self.0.clone().into_os_string();
        lock.push(".lock");
        let _ = std::fs::remove_file(lock);
    }
}

/// Copia o índice de verdade (o git o troca por rename, então a cópia é
/// consistente) com o mtime dele: é contra esse relógio que o git decide se
/// uma entrada precisa ser relida.
fn copy_index(real: &Path, temp: &Path) -> std::io::Result<()> {
    if !real.exists() {
        return Ok(());
    }
    std::fs::copy(real, temp)?;
    if let Ok(mtime) = std::fs::metadata(real).and_then(|m| m.modified()) {
        let _ = std::fs::File::options()
            .write(true)
            .open(temp)
            .and_then(|f| f.set_modified(mtime));
    }
    Ok(())
}

/// A pasta de `dir` como árvore do git. Quatro comandos, todos dentro do
/// mesmo prazo:
///
/// 1. `rev-parse --show-toplevel --git-dir` (jaula só-leitura);
/// 2. `add -A` com `GIT_INDEX_FILE` no índice temporário (jaula de escrita:
///    roda filtro `clean`, sem rede);
/// 3. `write-tree` com o mesmo índice (jaula de escrita);
/// 4. `rev-parse --verify HEAD` (jaula só-leitura).
///
/// O índice temporário mora no gitdir do worktree: a jaula de escrita já
/// libera esse diretório, e é lá que um índice dividido acha a metade
/// compartilhada (`core.splitIndex=false` para a cópia nunca gravar outra).
pub fn snapshot(dir: &Path, bound: Duration) -> Snapshot {
    let deadline = Instant::now() + bound;
    let mut place = git_in(dir);
    place.args([
        "rev-parse",
        "--path-format=absolute",
        "--show-toplevel",
        "--git-dir",
    ]);
    let place = match run_within(place, deadline, true) {
        Ran::Out(out) => String::from_utf8_lossy(&out).into_owned(),
        // 128: "not a git repository" (ou repositório bare, sem pasta).
        Ran::Failed(Some(128)) => return Snapshot::NoRepo,
        Ran::Failed(_) | Ran::TimedOut => return Snapshot::Unavailable,
    };
    let mut lines = place.lines();
    let (Some(root), Some(git_dir), None) = (lines.next(), lines.next(), lines.next()) else {
        return Snapshot::Unavailable;
    };
    let root = crate::repo::canonicalize_or(Path::new(root));
    let git_dir = PathBuf::from(git_dir);

    let index = TempIndex(git_dir.join(format!("{TEMP_INDEX_PREFIX}{}", uuid::Uuid::new_v4())));
    if copy_index(&git_dir.join("index"), &index.0).is_err() {
        return Snapshot::Unavailable;
    }
    let mut add = git_in_rw(&root);
    add.args(["-c", "core.splitIndex=false", "add", "-A"]);
    add.env("GIT_INDEX_FILE", &index.0);
    if !matches!(run_within(add, deadline, false), Ran::Out(_)) {
        return Snapshot::Unavailable;
    }
    let mut write = git_in_rw(&root);
    write.arg("write-tree");
    write.env("GIT_INDEX_FILE", &index.0);
    let Ran::Out(tree) = run_within(write, deadline, true) else {
        return Snapshot::Unavailable;
    };
    let tree = String::from_utf8_lossy(&tree).trim().to_string();
    if tree.len() < 40 || !tree.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Snapshot::Unavailable;
    }
    let mut head = git_in(&root);
    head.args(["rev-parse", "--verify", "--quiet", "HEAD"]);
    let head = match run_within(head, deadline, true) {
        Ran::Out(out) => Some(String::from_utf8_lossy(&out).trim().to_string()),
        Ran::Failed(_) => None,
        Ran::TimedOut => return Snapshot::Unavailable,
    };
    Snapshot::Tree(RepoTree { root, tree, head })
}

/// Commits feitos desde o prompt. `--ancestry-path` zera quando o `HEAD`
/// voltou ou foi para uma história que não descende do `HEAD` do prompt;
/// sem `HEAD` no prompt não há de onde contar.
fn commits_since(start: &RepoTree) -> u32 {
    let Some(head) = &start.head else {
        return 0;
    };
    let mut cmd = git_in(&start.root);
    cmd.args(["rev-list", "--count", "--ancestry-path"]);
    cmd.arg(format!("{head}..HEAD"));
    run_git(cmd, "git rev-list")
        .ok()
        .and_then(|out| String::from_utf8_lossy(&out).trim().parse().ok())
        .unwrap_or(0)
}

/// Árvore do prompt contra árvore do fim, com `-M`: um `mv` puro é um
/// arquivo renomeado, não um apagado e um novo.
fn changed_files(start: &RepoTree, end: &RepoTree) -> Option<Vec<FileDiff>> {
    if start.root != end.root {
        return None;
    }
    if start.tree == end.tree {
        return Some(Vec::new());
    }
    let mut files = diff_files(&end.root, &[&start.tree, &end.tree]).ok()?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Some(files)
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

#[derive(Debug, Clone)]
pub struct TurnStart {
    pub seq: u64,
    pub started_at_ms: u64,
    pub prompt: Option<String>,
    /// Uso acumulado da sessão até o prompt.
    pub usage: Option<AgentUsage>,
    pub repo: Snapshot,
}

/// Como o turno acabou, visto de fora.
#[derive(Debug, Clone)]
pub struct TurnEnding {
    pub outcome: TurnOutcome,
    pub failure: Option<String>,
    pub ended_at_ms: u64,
}

pub fn digest(
    start: &TurnStart,
    end: &Snapshot,
    ending: &TurnEnding,
    usage_end: Option<&AgentUsage>,
) -> TurnDigest {
    let (diff, files, commits) = match (&start.repo, end) {
        (Snapshot::NoRepo, _) | (_, Snapshot::NoRepo) => (TurnDiff::NoRepo, Vec::new(), 0),
        (Snapshot::Tree(from), Snapshot::Tree(to)) => match changed_files(from, to) {
            Some(files) => (TurnDiff::Ok, files, commits_since(from)),
            None => (TurnDiff::Unavailable, Vec::new(), 0),
        },
        _ => (TurnDiff::Unavailable, Vec::new(), 0),
    };
    let sum = |pick: fn(&FileDiff) -> Option<u32>| files.iter().filter_map(pick).sum();
    let (insertions, deletions) = (sum(|f| f.insertions), sum(|f| f.deletions));
    let files_total = files.len() as u32;
    let mut files = files;
    files.truncate(FILES_MAX);
    TurnDigest {
        seq: start.seq,
        outcome: ending.outcome,
        failure: ending.failure.clone(),
        started_at_ms: start.started_at_ms,
        ended_at_ms: ending.ended_at_ms,
        duration_ms: ending.ended_at_ms.saturating_sub(start.started_at_ms),
        prompt: start.prompt.clone(),
        diff,
        files,
        files_total,
        insertions,
        deletions,
        commits,
        cost_usd: None,
        tokens: None,
    }
    .with_usage(start.usage.as_ref(), usage_end)
}

/// As duas árvores de um turno que fechou com diff: é delas que sai o "ver
/// diff do turno". Ficam no core, só em memória.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnTrees {
    pub root: PathBuf,
    pub start: String,
    pub end: String,
}

pub fn trees(start: &TurnStart, end: &Snapshot) -> Option<TurnTrees> {
    match (&start.repo, end) {
        (Snapshot::Tree(from), Snapshot::Tree(to)) if from.root == to.root => Some(TurnTrees {
            root: to.root.clone(),
            start: from.tree.clone(),
            end: to.tree.clone(),
        }),
        _ => None,
    }
}

/// Um turno aberto: o prompt chegou, o fim ainda não.
pub struct OpenTurn {
    pub seq: u64,
    /// Onde tirar o snapshot do fim (a mesma pasta do prompt).
    pub dir: PathBuf,
    pub bound: Duration,
    started_at_ms: u64,
    prompt: Option<String>,
    usage: Option<AgentUsage>,
    repo: mpsc::Receiver<Snapshot>,
}

impl OpenTurn {
    /// Espera o snapshot do prompt, que termina sozinho dentro do teto dele.
    /// Bloqueia: nunca na thread que responde o hook.
    pub fn into_start(self) -> TurnStart {
        TurnStart {
            seq: self.seq,
            started_at_ms: self.started_at_ms,
            prompt: self.prompt,
            usage: self.usage,
            repo: self.repo.recv().unwrap_or(Snapshot::Unavailable),
        }
    }
}

#[derive(Default)]
struct Slot {
    seq: u64,
    open: Option<OpenTurn>,
    /// `seq` do turno a que `trees` se refere.
    closed_seq: u64,
    trees: Option<TurnTrees>,
}

/// Os turnos de cada sessão, por id — alcançável de qualquer lugar em que um
/// turno acaba (o handler de hooks e o coordenador que desce a `Idle` a
/// sessão segurada por subagente). Sem começo registrado (o TYBA entrou no
/// meio do turno, ou a CLI não manda `UserPromptSubmit`) não há o que fechar.
#[derive(Default)]
pub struct Turns(Mutex<HashMap<SessionId, Slot>>);

impl Turns {
    /// Abre um turno e dispara o snapshot do prompt numa thread à parte.
    /// Devolve o turno que ainda estava aberto, para quem chamou fechá-lo.
    pub fn begin(
        &self,
        id: SessionId,
        dir: PathBuf,
        bound: Duration,
        started_at_ms: u64,
        prompt: Option<String>,
        usage: Option<AgentUsage>,
    ) -> Option<OpenTurn> {
        let (tx, rx) = mpsc::channel();
        let at = dir.clone();
        std::thread::spawn(move || {
            let _ = tx.send(snapshot(&at, bound));
        });
        let mut slots = self.0.lock().expect("turns lock");
        let slot = slots.entry(id).or_default();
        slot.seq += 1;
        slot.open.replace(OpenTurn {
            seq: slot.seq,
            dir,
            bound,
            started_at_ms,
            prompt,
            usage,
            repo: rx,
        })
    }

    /// Fecha o turno aberto, se houver. Quem chega depois não acha nada: um
    /// turno fecha uma vez só.
    pub fn end(&self, id: SessionId) -> Option<OpenTurn> {
        self.0.lock().expect("turns lock").get_mut(&id)?.open.take()
    }

    pub fn is_open(&self, id: SessionId) -> bool {
        self.0
            .lock()
            .expect("turns lock")
            .get(&id)
            .is_some_and(|slot| slot.open.is_some())
    }

    /// Guarda as árvores do turno `seq` (ou a falta delas). Um turno mais
    /// antigo que termina de calcular depois não troca as de um mais novo.
    pub fn keep_trees(&self, id: SessionId, seq: u64, trees: Option<TurnTrees>) {
        let mut slots = self.0.lock().expect("turns lock");
        let Some(slot) = slots.get_mut(&id) else {
            return;
        };
        if seq >= slot.closed_seq {
            slot.closed_seq = seq;
            slot.trees = trees;
        }
    }

    /// As árvores do último turno que fechou com diff.
    pub fn last_trees(&self, id: SessionId) -> Option<TurnTrees> {
        self.0
            .lock()
            .expect("turns lock")
            .get(&id)
            .and_then(|slot| slot.trees.clone())
    }

    pub fn forget(&self, id: SessionId) {
        self.0.lock().expect("turns lock").remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::diff::FileStatus;

    /// Folgado: nenhum teste depende do teto de produção.
    const BOUND: Duration = Duration::from_secs(120);

    fn git(dir: &Path, args: &[&str]) {
        assert!(try_git(dir, args), "git {args:?}");
    }

    fn try_git(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("git")
            .status
            .success()
    }

    fn init(dir: &Path) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "t@example.com"]);
        git(dir, &["config", "user.name", "t"]);
    }

    /// Repositório com um commit: `a.txt`, `b.txt`, `some.txt` e um
    /// `.gitignore` que ignora `*.log`. A config global do dono não entra
    /// (`hermetic_git_config` em `lib.rs`).
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init(p);
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
            seq: 1,
            started_at_ms: 1_000,
            prompt: Some("arruma o a.txt".into()),
            usage: None,
            repo: snapshot(dir, BOUND),
        }
    }

    fn ending(outcome: TurnOutcome) -> TurnEnding {
        TurnEnding {
            outcome,
            failure: None,
            ended_at_ms: 4_500,
        }
    }

    fn end(start: &TurnStart, dir: &Path) -> TurnDigest {
        digest(
            start,
            &snapshot(dir, BOUND),
            &ending(TurnOutcome::Completed),
            None,
        )
    }

    /// (caminho, status, +, -) de cada arquivo.
    fn rows(d: &TurnDigest) -> Vec<(&str, FileStatus, Option<u32>, Option<u32>)> {
        d.files
            .iter()
            .map(|f| (f.path.as_str(), f.status, f.insertions, f.deletions))
            .collect()
    }

    fn temp_indexes(git_dir: &Path) -> Vec<String> {
        std::fs::read_dir(git_dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|name| name.starts_with(TEMP_INDEX_PREFIX))
            .collect()
    }

    #[test]
    fn nothing_changed_gives_an_empty_list() {
        let dir = repo();
        let d = end(&start(dir.path()), dir.path());
        assert_eq!(d.diff, TurnDiff::Ok);
        assert!(d.files.is_empty());
        assert_eq!(
            (d.files_total, d.insertions, d.deletions, d.commits),
            (0, 0, 0, 0)
        );
        assert_eq!(d.duration_ms, 3_500);
        assert_eq!(d.prompt.as_deref(), Some("arruma o a.txt"));
    }

    #[test]
    fn what_was_dirty_before_the_prompt_stays_out() {
        let dir = repo();
        let p = dir.path();
        write(p, "a.txt", "um\ndois\ntres\nantes\n");
        write(p, "velho.txt", "ja estava aqui\n");
        git(p, &["add", "a.txt"]);
        let s = start(p);
        assert!(end(&s, p).files.is_empty());

        write(p, "a.txt", "um\ndois\ntres\nantes\ndepois\n");
        let d = end(&s, p);
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
        let d = end(&s, p);
        assert_eq!(
            rows(&d),
            [("a.txt", FileStatus::Modified, Some(2), Some(1))]
        );
        assert_eq!((d.insertions, d.deletions, d.files_total), (2, 1, 1));
    }

    #[test]
    fn new_untracked_file_counts_and_ignored_does_not() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        write(p, "novo.txt", "linha 1\nlinha 2\n");
        write(p, "saida.log", "ignorado\n");
        let d = end(&s, p);
        assert_eq!(
            rows(&d),
            [("novo.txt", FileStatus::Added, Some(2), Some(0))]
        );
    }

    #[test]
    fn untracked_file_from_before_the_prompt_shows_what_changed_in_it() {
        let dir = repo();
        let p = dir.path();
        write(p, "velho.txt", "ja estava aqui\n");
        let s = start(p);
        assert!(end(&s, p).files.is_empty());
        write(p, "velho.txt", "ja estava aqui\nmudou no turno\n");
        let d = end(&s, p);
        assert_eq!(
            rows(&d),
            [("velho.txt", FileStatus::Modified, Some(1), Some(0))]
        );
        // Entrar no git durante o turno não o transforma em arquivo novo.
        git(p, &["add", "velho.txt"]);
        git(p, &["commit", "-qm", "velho"]);
        assert_eq!(rows(&end(&s, p)), rows(&d));
    }

    #[test]
    fn deleted_file_counts() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        std::fs::remove_file(p.join("some.txt")).unwrap();
        let d = end(&s, p);
        assert_eq!(
            rows(&d),
            [("some.txt", FileStatus::Deleted, Some(0), Some(1))]
        );
    }

    #[test]
    fn rename_is_one_file_with_or_without_git_mv() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        std::fs::rename(p.join("b.txt"), p.join("c.txt")).unwrap();
        let d = end(&s, p);
        assert_eq!(rows(&d), [("c.txt", FileStatus::Renamed, Some(0), Some(0))]);
        assert_eq!(d.files[0].old_path.as_deref(), Some("b.txt"));

        git(p, &["mv", "a.txt", "z.txt"]);
        let d = end(&s, p);
        assert_eq!(
            rows(&d),
            [
                ("c.txt", FileStatus::Renamed, Some(0), Some(0)),
                ("z.txt", FileStatus::Renamed, Some(0), Some(0)),
            ]
        );
    }

    #[test]
    fn commit_made_during_the_turn_still_counts_its_files() {
        let dir = repo();
        let p = dir.path();
        write(
            p,
            "b.txt",
            "conteudo estavel do b\noutra linha\nsujo antes\n",
        );
        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        write(p, "novo.txt", "oi\n");
        git(p, &["add", "a.txt", "novo.txt"]);
        git(p, &["commit", "-qm", "do turno"]);
        let d = end(&s, p);
        assert_eq!(d.commits, 1);
        assert_eq!(
            rows(&d),
            [
                ("a.txt", FileStatus::Modified, Some(1), Some(0)),
                ("novo.txt", FileStatus::Added, Some(1), Some(0)),
            ]
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
        let d = end(&s, p);
        assert_eq!(d.commits, 0);
        assert_eq!(
            rows(&d),
            [("a.txt", FileStatus::Modified, Some(0), Some(1))]
        );
    }

    #[test]
    fn folder_without_git_has_no_diff() {
        let dir = tempfile::tempdir().unwrap();
        let s = start(dir.path());
        assert_eq!(s.repo, Snapshot::NoRepo);
        write(dir.path(), "novo.txt", "oi\n");
        let d = end(&s, dir.path());
        assert_eq!(d.diff, TurnDiff::NoRepo);
        assert!(d.files.is_empty());
        assert_eq!(d.duration_ms, 3_500);
    }

    #[test]
    fn repo_without_a_commit_is_a_normal_diff() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        init(p);
        write(p, "antes.txt", "ja estava\n");
        let s = start(p);
        assert!(matches!(&s.repo, Snapshot::Tree(t) if t.head.is_none()));
        write(p, "novo.txt", "um\ndois\n");
        git(p, &["add", "-A"]);
        git(p, &["commit", "-qm", "primeiro"]);
        let d = end(&s, p);
        assert_eq!(d.diff, TurnDiff::Ok);
        assert_eq!(
            rows(&d),
            [("novo.txt", FileStatus::Added, Some(2), Some(0))]
        );
        // Sem `HEAD` no prompt não há de onde contar commits.
        assert_eq!(d.commits, 0);
    }

    /// O snapshot não escreve nem trava o índice de verdade, e não deixa o
    /// temporário para trás.
    #[test]
    fn snapshot_leaves_the_real_index_untouched() {
        let dir = repo();
        let p = dir.path();
        write(p, "a.txt", "um\ndois\ntres\nsujo\n");
        write(p, "staged.txt", "no indice\n");
        git(p, &["add", "staged.txt"]);
        write(p, "solto.txt", "nao rastreado\n");
        let index = p.join(".git/index");
        let before = std::fs::read(&index).unwrap();
        let mtime = std::fs::metadata(&index).unwrap().modified().unwrap();

        assert!(matches!(snapshot(p, BOUND), Snapshot::Tree(_)));

        assert_eq!(std::fs::read(&index).unwrap(), before);
        assert_eq!(
            std::fs::metadata(&index).unwrap().modified().unwrap(),
            mtime
        );
        assert!(temp_indexes(&p.join(".git")).is_empty());
    }

    #[test]
    fn snapshot_works_with_the_index_locked_by_someone_else() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        write(p, ".git/index.lock", "");
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        let d = end(&s, p);
        assert_eq!(d.diff, TurnDiff::Ok);
        assert_eq!(
            rows(&d),
            [("a.txt", FileStatus::Modified, Some(1), Some(0))]
        );
        assert!(p.join(".git/index.lock").exists());
    }

    /// No meio de um merge em conflito o `stash create` se recusa; a árvore
    /// pelo índice temporário sai, e o que já estava em conflito antes do
    /// prompt não conta.
    #[test]
    fn snapshot_works_in_the_middle_of_a_conflicted_merge() {
        let dir = repo();
        let p = dir.path();
        git(p, &["checkout", "-qb", "feature"]);
        write(p, "a.txt", "um\nfeature\ntres\n");
        git(p, &["commit", "-qam", "feature"]);
        git(p, &["checkout", "-q", "-"]);
        write(p, "a.txt", "um\nprincipal\ntres\n");
        git(p, &["commit", "-qam", "principal"]);
        assert!(
            !try_git(p, &["merge", "feature"]),
            "merge deveria conflitar"
        );
        let index = std::fs::read(p.join(".git/index")).unwrap();

        let s = start(p);
        assert!(matches!(s.repo, Snapshot::Tree(_)));
        assert!(end(&s, p).files.is_empty());
        write(p, "b.txt", "conteudo estavel do b\noutra linha\nno turno\n");
        let d = end(&s, p);
        assert_eq!(d.diff, TurnDiff::Ok);
        assert_eq!(
            rows(&d),
            [("b.txt", FileStatus::Modified, Some(1), Some(0))]
        );
        assert_eq!(std::fs::read(p.join(".git/index")).unwrap(), index);
    }

    /// Repositório com filtro de conteúdo: o git roda enjaulado, e a jaula de
    /// escrita tem de deixar o índice temporário nascer no gitdir. Só macOS:
    /// no Linux a jaula depende de user namespace, que a CI não tem.
    #[cfg(target_os = "macos")]
    #[test]
    fn snapshot_works_inside_the_git_cage() {
        let dir = repo();
        let p = dir.path();
        git(p, &["config", "filter.passa.clean", "cat"]);
        write(p, ".gitattributes", "*.txt filter=passa\n");
        assert!(crate::sandbox::git::repo_uses_content_filter(p));
        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        let d = end(&s, p);
        assert_eq!(d.diff, TurnDiff::Ok);
        assert_eq!(
            rows(&d),
            [("a.txt", FileStatus::Modified, Some(1), Some(0))]
        );
        assert!(temp_indexes(&p.join(".git")).is_empty());
    }

    #[test]
    fn snapshot_past_the_bound_is_unavailable() {
        let dir = repo();
        let p = dir.path();
        assert_eq!(snapshot(p, Duration::ZERO), Snapshot::Unavailable);
        assert!(temp_indexes(&p.join(".git")).is_empty());

        let s = start(p);
        let d = digest(
            &s,
            &Snapshot::Unavailable,
            &ending(TurnOutcome::Completed),
            None,
        );
        assert_eq!(d.diff, TurnDiff::Unavailable);
        assert!(trees(&s, &Snapshot::Unavailable).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn child_past_the_deadline_is_killed() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 60 & wait"]);
        cmd.stdin(Stdio::null());
        let clock = Instant::now();
        let ran = run_within(cmd, Instant::now() + Duration::from_millis(100), true);
        assert!(matches!(ran, Ran::TimedOut));
        assert!(clock.elapsed() < Duration::from_secs(30));
    }

    #[test]
    fn list_is_capped_and_totals_are_not() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        for n in 0..FILES_MAX + 5 {
            write(p, &format!("novo-{n:03}.txt"), &format!("arquivo {n}\n"));
        }
        let d = end(&s, p);
        assert_eq!(d.files.len(), FILES_MAX);
        assert_eq!(d.files_total as usize, FILES_MAX + 5);
        assert_eq!(d.insertions as usize, FILES_MAX + 5);
        assert_eq!(d.files[0].path, "novo-000.txt");
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
            seq: 3,
            started_at_ms: 0,
            prompt: None,
            usage: at_start,
            repo: Snapshot::NoRepo,
        };
        let ending = TurnEnding {
            outcome: TurnOutcome::Failed,
            failure: Some("limite de uso atingido".into()),
            ended_at_ms: 10,
        };
        digest(&s, &Snapshot::NoRepo, &ending, at_end.as_ref())
    }

    #[test]
    fn usage_is_the_delta_of_the_turn() {
        let d = usage_digest(Some(usage(0.5, 1_000, 200)), Some(usage(0.75, 1_600, 300)));
        assert_eq!(d.cost_usd, Some(0.25));
        assert_eq!(d.tokens, Some(700));
        assert_eq!(d.outcome, TurnOutcome::Failed);
        assert_eq!(d.failure.as_deref(), Some("limite de uso atingido"));
        assert_eq!(d.seq, 3);
        // O `statusLine` que chega depois do `Stop` só mexe em custo e tokens.
        let late = usage(1.0, 2_000, 400);
        let settled = d
            .clone()
            .with_usage(Some(&usage(0.5, 1_000, 200)), Some(&late));
        assert_eq!((settled.cost_usd, settled.tokens), (Some(0.5), Some(1_200)));
        assert_eq!(settled.failure, d.failure);
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

    fn begin(turns: &Turns, id: SessionId, dir: &Path, at: u64) -> Option<OpenTurn> {
        turns.begin(id, dir.to_path_buf(), BOUND, at, Some("oi".into()), None)
    }

    #[test]
    fn turn_end_without_a_start_gives_nothing() {
        let turns = Turns::default();
        let id = SessionId::new_v4();
        assert!(turns.end(id).is_none());
        assert!(!turns.is_open(id));
    }

    /// O turno fecha uma vez só, venha o fim de onde vier (hook ou
    /// coordenador), e uma sessão não enxerga o turno da outra.
    #[test]
    fn a_turn_closes_once_whoever_gets_there_first() {
        let dir = repo();
        let turns = Turns::default();
        let (id, other) = (SessionId::new_v4(), SessionId::new_v4());
        assert!(begin(&turns, id, dir.path(), 7).is_none());
        assert!(turns.is_open(id));
        assert!(turns.end(other).is_none());

        let open = turns.end(id).expect("turno aberto");
        assert!(!turns.is_open(id));
        assert!(turns.end(id).is_none());
        let s = open.into_start();
        assert_eq!((s.seq, s.started_at_ms), (1, 7));
        assert_eq!(s.prompt.as_deref(), Some("oi"));
        assert!(matches!(s.repo, Snapshot::Tree(_)));
    }

    /// Prompt novo com turno ainda aberto (Esc e negação no prompt nativo
    /// não mandam hook): o aberto volta para ser fechado, e o novo segue.
    #[test]
    fn a_new_prompt_hands_back_the_turn_still_open() {
        let dir = repo();
        let turns = Turns::default();
        let id = SessionId::new_v4();
        assert!(begin(&turns, id, dir.path(), 10).is_none());
        let displaced = begin(&turns, id, dir.path(), 20).expect("turno anterior");
        assert_eq!(displaced.seq, 1);
        assert!(turns.is_open(id));
        assert_eq!(turns.end(id).expect("turno novo").seq, 2);
        assert!(begin(&turns, id, dir.path(), 30).is_none());
        assert_eq!(turns.end(id).unwrap().seq, 3);
    }

    #[test]
    fn trees_of_an_older_turn_never_replace_a_newer_one() {
        let dir = repo();
        let turns = Turns::default();
        let id = SessionId::new_v4();
        begin(&turns, id, dir.path(), 10);
        let pair = |start: &str| TurnTrees {
            root: dir.path().to_path_buf(),
            start: start.into(),
            end: "fim".into(),
        };
        turns.keep_trees(id, 2, Some(pair("dois")));
        turns.keep_trees(id, 1, Some(pair("um")));
        assert_eq!(turns.last_trees(id), Some(pair("dois")));
        // Turno mais novo sem diff: as do anterior não servem mais.
        turns.keep_trees(id, 3, None);
        assert_eq!(turns.last_trees(id), None);
        turns.forget(id);
        assert!(!turns.is_open(id));
    }

    #[test]
    fn trees_come_from_the_two_snapshots() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        let end = snapshot(p, BOUND);
        let pair = trees(&s, &end).expect("duas árvores");
        assert_ne!(pair.start, pair.end);
        assert!(matches!(&s.repo, Snapshot::Tree(t) if t.tree == pair.start));
    }

    #[test]
    fn digest_serializes_paths_and_counts_only() {
        let dir = repo();
        let p = dir.path();
        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nsegredo do arquivo\n");
        let json = serde_json::to_value(end(&s, p)).unwrap();
        assert_eq!(json["outcome"], "completed");
        assert_eq!(json["diff"], "ok");
        assert_eq!(json["seq"], 1);
        assert_eq!(json["files_total"], 1);
        assert_eq!(json["failure"], serde_json::Value::Null);
        assert_eq!(json["files"][0]["path"], "a.txt");
        assert_eq!(json["files"][0]["status"], "modified");
        let text = json.to_string();
        assert!(!text.contains("segredo do arquivo"));
        assert!(!text.contains("tree"));
        assert_eq!(
            serde_json::to_value(TurnDiff::NoRepo).unwrap(),
            serde_json::json!("no_repo")
        );
        assert_eq!(
            serde_json::to_value(TurnDiff::Unavailable).unwrap(),
            serde_json::json!("unavailable")
        );
    }
}
