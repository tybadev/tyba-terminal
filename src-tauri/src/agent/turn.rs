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
//! rastreados, sem os ignorados). Há um no prompt e outro no fim do turno, e
//! o diff é de árvore contra árvore — não roda filtro de conteúdo nem lê a
//! pasta de novo.
//!
//! Nada é escrito no repositório do usuário. O índice é uma cópia
//! temporária e os objetos (os blobs do que estava modificado ou não
//! rastreado, e as árvores) vão para um diretório privado do TYBA, um por
//! turno, com o banco de objetos do repositório só como alternativa de
//! leitura. O diretório é `0700` e some com o turno: fica o do turno aberto
//! e o do último que fechou (é dele que sai o "ver diff do turno"); o resto
//! é apagado quando um turno fecha, quando a sessão acaba e no boot.
//!
//! Limite: num repositório em que o conteúdo não rastreado não cabe de ser
//! lido dentro do teto do snapshot, nenhum turno tem diff.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::agent::usage::AgentUsage;
use crate::session::SessionId;
use crate::worktree::diff::{
    parse_name_status_z, parse_numstat_z, parse_unified_hunks, FileDiff, FileHunks,
};
use crate::worktree::{git_in, git_in_scratch};

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

/// Teto do diff de árvore contra árvore e da contagem de commits no fim do
/// turno. Mais folgado que o do snapshot: aqui a pasta já está congelada nas
/// duas árvores, e demorar só atrasa o resumo.
pub const DIFF_MAX: Duration = Duration::from_millis(5000);

/// Teto da lista que vai ao webview: ela viaja em todo `session://status`.
pub const FILES_MAX: usize = 200;

const TEMP_INDEX_PREFIX: &str = "index-";

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
    /// É um repositório, mas um dos snapshots (ou o diff) falhou ou estourou
    /// o teto.
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

/// O que do resumo entra no histórico de notificações, que é gravado no
/// SQLite: só números. Caminho, prompt e motivo da falha não cabem aqui.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnCounts {
    pub seq: u64,
    /// `None` quando o turno não foi medido (sem git, ou o snapshot falhou).
    pub files: Option<u32>,
    pub insertions: u32,
    pub deletions: u32,
    pub duration_ms: u64,
    pub cost_usd: Option<f64>,
}

/// "4m 51s", "36s": a duração do turno na linha de números.
fn elapsed(ms: u64) -> String {
    let total = ms / 1000;
    match total / 60 {
        0 => format!("{total}s"),
        minutes => format!("{minutes}m {:02}s", total % 60),
    }
}

impl TurnDigest {
    pub fn counts(&self) -> TurnCounts {
        TurnCounts {
            seq: self.seq,
            files: (self.diff == TurnDiff::Ok).then_some(self.files_total),
            insertions: self.insertions,
            deletions: self.deletions,
            duration_ms: self.duration_ms,
            cost_usd: self.cost_usd,
        }
    }

    /// A linha de números do aviso do sistema (quadro T4 de ResumoDoTurno):
    /// "6 arquivos · +115 −55 · 4m 51s", "sem mudanças · 36s"; só a duração
    /// quando o turno não foi medido.
    pub fn notice_line(&self) -> String {
        let took = elapsed(self.duration_ms);
        match self.counts().files {
            None => took,
            Some(0) => format!("sem mudanças · {took}"),
            Some(n) => format!(
                "{n} {} · +{} −{} · {took}",
                if n == 1 { "arquivo" } else { "arquivos" },
                self.insertions,
                self.deletions
            ),
        }
    }

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
    /// O banco de objetos de verdade do repositório, lido como alternativa.
    pub objects: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Snapshot {
    NoRepo,
    Unavailable,
    Tree(RepoTree),
}

/// `~/.tyba/turns` (`~/.tyba-dev/turns` no build de dev): um diretório por
/// sessão e, dentro dele, um por turno.
pub fn private_root() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(crate::profile::HOME_DIR)
        .join("turns")
}

/// Apaga tudo: no boot não há turno aberto nem resumo que sobreviva, e o
/// que estiver lá é sobra de um processo que morreu.
pub fn sweep(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

/// Cria `dir` e os pais que faltarem, todos `0700` (os que já existem ficam
/// como estão — `~/.tyba` é do dono).
fn create_private(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    if let Some(parent) = dir.parent() {
        create_private(parent)?;
    }
    crate::session::create_private_dir(dir)
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
/// só volta com o filho recolhido. `capture` fica de fora no `add`, o único
/// que roda filtro: um filtro que ficasse para trás seguraria o pipe e a
/// leitura junto.
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
    // Lê enquanto o filho roda: a lista de um diff não cabe no pipe.
    let reader = child.stdout.take().map(|mut stdout| {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = std::io::Read::read_to_end(&mut stdout, &mut out);
            out
        })
    });
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
        // Antes do `wait`: com o líder ainda por recolher o pgid é dele. O
        // leitor termina sozinho quando o grupo morre e o pipe fecha.
        kill_group(&child);
        let _ = child.kill();
        let _ = child.wait();
        return Ran::TimedOut;
    };
    if !status.success() {
        return Ran::Failed(status.code());
    }
    Ran::Out(reader.and_then(|r| r.join().ok()).unwrap_or_default())
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

/// O valor de `GIT_ALTERNATE_OBJECT_DIRECTORIES` para um caminho só. A
/// variável é uma lista separada por `:`; caminho com `:` vai entre aspas,
/// no estilo C que o git aceita.
fn alternates(objects: &Path) -> OsString {
    let raw = objects.as_os_str();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let bytes = raw.as_bytes();
        if bytes.contains(&b':') || bytes.first() == Some(&b'"') {
            let mut quoted = vec![b'"'];
            for b in bytes {
                if matches!(b, b'"' | b'\\') {
                    quoted.push(b'\\');
                }
                quoted.push(*b);
            }
            quoted.push(b'"');
            return OsString::from_vec(quoted);
        }
    }
    raw.to_os_string()
}

/// `git` que lê o repositório e as árvores do turno: os objetos novos moram
/// em `scratch/objects`, os do repositório entram como alternativa. As duas
/// variáveis vão depois do builder, que as limpa.
fn with_objects(mut cmd: Command, scratch: &Path, objects: &Path) -> Command {
    cmd.env("GIT_OBJECT_DIRECTORY", scratch.join("objects"))
        .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", alternates(objects));
    cmd
}

/// Exit 128 do `rev-parse` não é só "não é um repositório": pasta que sumiu,
/// `.git` quebrado e dono suspeito saem igual. Só é pasta sem git a que
/// existe e não tem `.git` nenhum daqui até a raiz.
fn outside_any_repo(dir: &Path) -> bool {
    dir.is_dir()
        && dir
            .ancestors()
            .all(|a| std::fs::symlink_metadata(a.join(".git")).is_err())
}

/// A pasta de `dir` como árvore do git, sem escrever no repositório. Quatro
/// comandos, todos dentro do mesmo prazo:
///
/// 1. `rev-parse --show-toplevel --git-dir --git-path objects` (jaula
///    só-leitura);
/// 2. `add -A --ignore-errors` — jaula [`git_in_scratch`]: roda filtro
///    `clean`, sem rede, e só escreve em `scratch`;
/// 3. `write-tree` (mesma jaula);
/// 4. `rev-parse --verify HEAD` (jaula só-leitura).
///
/// O 2 e o 3 rodam com `GIT_INDEX_FILE` numa cópia do índice dentro de
/// `scratch`, `GIT_OBJECT_DIRECTORY` em `scratch/objects` e o banco de
/// verdade em `GIT_ALTERNATE_OBJECT_DIRECTORIES`, e com
/// `core.splitIndex=false` (um índice dividido gravaria outra metade
/// compartilhada no gitdir).
///
/// Um arquivo que o `add` não consegue indexar (ilegível, repositório
/// aninhado sem commit, fora do cone do sparse-checkout) faz o `add` sair
/// com erro, mas o `--ignore-errors` indexa o resto: a árvore sai sem ele.
pub fn snapshot(dir: &Path, scratch: &Path, bound: Duration) -> Snapshot {
    let deadline = Instant::now() + bound;
    let mut place = git_in(dir);
    place.args([
        "rev-parse",
        "--path-format=absolute",
        "--show-toplevel",
        "--git-dir",
        "--git-path",
        "objects",
    ]);
    let place = match run_within(place, deadline, true) {
        Ran::Out(out) => String::from_utf8_lossy(&out).into_owned(),
        Ran::Failed(Some(128)) if outside_any_repo(dir) => return Snapshot::NoRepo,
        Ran::Failed(_) | Ran::TimedOut => return Snapshot::Unavailable,
    };
    let mut lines = place.lines();
    let (Some(root), Some(git_dir), Some(objects), None) =
        (lines.next(), lines.next(), lines.next(), lines.next())
    else {
        return Snapshot::Unavailable;
    };
    let root = crate::repo::canonicalize_or(Path::new(root));
    let objects = PathBuf::from(objects);

    if create_private(&scratch.join("objects")).is_err() {
        return Snapshot::Unavailable;
    }
    let index = TempIndex(scratch.join(format!("{TEMP_INDEX_PREFIX}{}", uuid::Uuid::new_v4())));
    if copy_index(&Path::new(git_dir).join("index"), &index.0).is_err() {
        return Snapshot::Unavailable;
    }
    let indexed = |args: &[&str]| {
        let mut cmd = with_objects(git_in_scratch(&root, scratch), scratch, &objects);
        cmd.args(["-c", "core.splitIndex=false"]).args(args);
        cmd.env("GIT_INDEX_FILE", &index.0);
        cmd
    };
    let add = indexed(&["add", "-A", "--ignore-errors"]);
    if matches!(run_within(add, deadline, false), Ran::TimedOut) {
        return Snapshot::Unavailable;
    }
    let Ran::Out(tree) = run_within(indexed(&["write-tree"]), deadline, true) else {
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
    Snapshot::Tree(RepoTree {
        root,
        tree,
        head,
        objects,
    })
}

/// Commits entre o `HEAD` do prompt e o `HEAD` do snapshot do fim.
/// `--ancestry-path` zera quando o `HEAD` voltou ou foi para uma história
/// que não descende do `HEAD` do prompt; sem `HEAD` no prompt não há de onde
/// contar.
fn commits_between(from: &RepoTree, to: &RepoTree, deadline: Instant) -> u32 {
    let (Some(old), Some(new)) = (&from.head, &to.head) else {
        return 0;
    };
    if old == new {
        return 0;
    }
    let mut cmd = git_in(&to.root);
    cmd.args(["rev-list", "--count", "--ancestry-path"]);
    cmd.arg(format!("{old}..{new}"));
    match run_within(cmd, deadline, true) {
        Ran::Out(out) => String::from_utf8_lossy(&out).trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// Árvore do prompt contra árvore do fim, com `-M`: um `mv` puro é um
/// arquivo renomeado, não um apagado e um novo. Mesma dupla
/// numstat + name-status de [`crate::worktree::diff`], aqui com prazo e com
/// os objetos do turno.
fn changed_files(
    from: &RepoTree,
    to: &RepoTree,
    scratch: &Path,
    deadline: Instant,
) -> Option<Vec<FileDiff>> {
    if from.root != to.root {
        return None;
    }
    if from.tree == to.tree {
        return Some(Vec::new());
    }
    let run = |format: &str| {
        let mut cmd = with_objects(git_in(&to.root), scratch, &to.objects);
        cmd.args(["diff", "--no-ext-diff", "--no-color", format, "-z", "-M"]);
        cmd.args([&from.tree, &to.tree]);
        match run_within(cmd, deadline, true) {
            Ran::Out(out) => Some(out),
            _ => None,
        }
    };
    let counts: HashMap<String, (Option<u32>, Option<u32>)> = parse_numstat_z(&run("--numstat")?)
        .into_iter()
        .map(|e| (e.path, (e.insertions, e.deletions)))
        .collect();
    let mut files: Vec<FileDiff> = parse_name_status_z(&run("--name-status")?)
        .into_iter()
        .map(|(status, old_path, path)| {
            let (insertions, deletions) = counts.get(&path).copied().unwrap_or((Some(0), Some(0)));
            FileDiff {
                path,
                old_path,
                status,
                insertions,
                deletions,
            }
        })
        .collect();
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
    /// O diretório privado do turno, onde estão os objetos das duas árvores.
    pub scratch: PathBuf,
}

/// Como o turno acabou, visto de fora.
#[derive(Debug, Clone)]
pub struct TurnEnding {
    pub outcome: TurnOutcome,
    pub failure: Option<String>,
    pub ended_at_ms: u64,
}

/// `bound` é o teto do diff e da contagem de commits ([`DIFF_MAX`]).
pub fn digest(
    start: &TurnStart,
    end: &Snapshot,
    ending: &TurnEnding,
    usage_end: Option<&AgentUsage>,
    bound: Duration,
) -> TurnDigest {
    let deadline = Instant::now() + bound;
    let (diff, files, commits) = match (&start.repo, end) {
        (Snapshot::NoRepo, _) | (_, Snapshot::NoRepo) => (TurnDiff::NoRepo, Vec::new(), 0),
        (Snapshot::Tree(from), Snapshot::Tree(to)) => {
            match changed_files(from, to, &start.scratch, deadline) {
                Some(files) => (TurnDiff::Ok, files, commits_between(from, to, deadline)),
                None => (TurnDiff::Unavailable, Vec::new(), 0),
            }
        }
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

/// As duas árvores de um turno que fechou com diff, e onde lê-las: é delas
/// que sai o "ver diff do turno". Ficam no core, só em memória.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnTrees {
    pub root: PathBuf,
    pub start: String,
    pub end: String,
    /// Diretório privado do turno (`GIT_OBJECT_DIRECTORY` é `scratch/objects`).
    pub scratch: PathBuf,
    /// Banco de objetos do repositório (`GIT_ALTERNATE_OBJECT_DIRECTORIES`).
    pub objects: PathBuf,
}

pub fn trees(start: &TurnStart, end: &Snapshot) -> Option<TurnTrees> {
    match (&start.repo, end) {
        (Snapshot::Tree(from), Snapshot::Tree(to)) if from.root == to.root => Some(TurnTrees {
            root: to.root.clone(),
            start: from.tree.clone(),
            end: to.tree.clone(),
            scratch: start.scratch.clone(),
            objects: to.objects.clone(),
        }),
        _ => None,
    }
}

impl TurnTrees {
    fn side(&self, tree: &str) -> RepoTree {
        RepoTree {
            root: self.root.clone(),
            tree: tree.to_string(),
            head: None,
            objects: self.objects.clone(),
        }
    }

    /// Todos os arquivos do turno, sem o teto de [`FILES_MAX`]: a lista do
    /// "ver diff do turno". `None` quando o git falha ou estoura `bound` (o
    /// diretório privado some quando o turno seguinte fecha).
    pub fn files(&self, bound: Duration) -> Option<Vec<FileDiff>> {
        changed_files(
            &self.side(&self.start),
            &self.side(&self.end),
            &self.scratch,
            Instant::now() + bound,
        )
    }

    /// Os trechos de um arquivo entre as duas árvores. Árvore contra árvore:
    /// não lê a pasta nem roda filtro; `--no-textconv` e `--no-ext-diff`
    /// para nenhum comando definido pelo repositório rodar aqui.
    pub fn file_hunks(
        &self,
        path: &str,
        old_path: Option<&str>,
        bound: Duration,
    ) -> Option<FileHunks> {
        let mut cmd = with_objects(git_in(&self.root), &self.scratch, &self.objects);
        cmd.args(["diff", "--no-ext-diff", "--no-textconv", "--no-color", "-M"]);
        cmd.args([&self.start, &self.end]);
        cmd.arg("--").arg(format!(":(literal){path}"));
        if let Some(old) = old_path {
            cmd.arg(format!(":(literal){old}"));
        }
        match run_within(cmd, Instant::now() + bound, true) {
            Ran::Out(out) => Some(parse_unified_hunks(&String::from_utf8_lossy(&out))),
            _ => None,
        }
    }
}

/// Um turno aberto: o prompt chegou, o fim ainda não.
pub struct OpenTurn {
    pub seq: u64,
    /// Onde tirar o snapshot do fim (a mesma pasta do prompt).
    pub dir: PathBuf,
    /// O diretório privado do turno.
    pub scratch: PathBuf,
    pub bound: Duration,
    /// Quando o `Stop` do orquestrador chegou e ficou segurado (subagente
    /// vivo, pendência em background). `None` enquanto ele trabalha.
    pub stop_seen_at_ms: Option<u64>,
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
            scratch: self.scratch,
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
/// turno acaba (o handler de hooks, o coordenador que desce a `Idle` a
/// sessão segurada por subagente, a saída do processo). Sem começo
/// registrado (o TYBA entrou no meio do turno, ou a CLI não manda
/// `UserPromptSubmit`) não há o que fechar.
pub struct Turns {
    /// Raiz dos diretórios privados: `root/<sessão>/<seq>`.
    root: PathBuf,
    slots: Mutex<HashMap<SessionId, Slot>>,
}

impl Default for Turns {
    fn default() -> Self {
        Self::at(private_root())
    }
}

impl Turns {
    pub fn at(root: PathBuf) -> Self {
        Self {
            root,
            slots: Mutex::new(HashMap::new()),
        }
    }

    fn session_dir(&self, id: SessionId) -> PathBuf {
        self.root.join(id.to_string())
    }

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
        let mut slots = self.slots.lock().expect("turns lock");
        let slot = slots.entry(id).or_default();
        slot.seq += 1;
        let scratch = self.session_dir(id).join(slot.seq.to_string());
        let (tx, rx) = mpsc::channel();
        let (at, into) = (dir.clone(), scratch.clone());
        std::thread::spawn(move || {
            let _ = tx.send(snapshot(&at, &into, bound));
        });
        slot.open.replace(OpenTurn {
            seq: slot.seq,
            dir,
            scratch,
            bound,
            stop_seen_at_ms: None,
            started_at_ms,
            prompt,
            usage,
            repo: rx,
        })
    }

    /// Fecha o turno aberto, se houver. Quem chega depois não acha nada: um
    /// turno fecha uma vez só.
    pub fn end(&self, id: SessionId) -> Option<OpenTurn> {
        self.slots
            .lock()
            .expect("turns lock")
            .get_mut(&id)?
            .open
            .take()
    }

    /// Fecha o turno só se o `Stop` do orquestrador já foi visto. É o que
    /// usa quem desce a sessão a `Idle` quando o último subagente termina:
    /// um turno que acabou de abrir não é dele para fechar.
    pub fn end_if_stopped(&self, id: SessionId) -> Option<OpenTurn> {
        let mut slots = self.slots.lock().expect("turns lock");
        let slot = slots.get_mut(&id)?;
        slot.open.as_ref()?.stop_seen_at_ms?;
        slot.open.take()
    }

    /// O `Stop` do orquestrador chegou e ficou segurado.
    pub fn note_stop(&self, id: SessionId, at_ms: u64) {
        self.note(id, Some(at_ms));
    }

    /// O orquestrador voltou a trabalhar (acordou de uma pendência): o `Stop`
    /// anterior não é mais o fim do turno.
    pub fn note_working(&self, id: SessionId) {
        self.note(id, None);
    }

    fn note(&self, id: SessionId, stop_seen_at_ms: Option<u64>) {
        let mut slots = self.slots.lock().expect("turns lock");
        if let Some(open) = slots.get_mut(&id).and_then(|slot| slot.open.as_mut()) {
            open.stop_seen_at_ms = stop_seen_at_ms;
        }
    }

    pub fn is_open(&self, id: SessionId) -> bool {
        self.slots
            .lock()
            .expect("turns lock")
            .get(&id)
            .is_some_and(|slot| slot.open.is_some())
    }

    /// Guarda as árvores do turno `seq` (ou a falta delas) e apaga o
    /// diretório privado dos turnos anteriores ao último que fechou. Um
    /// turno mais antigo que termina de calcular depois não troca as de um
    /// mais novo.
    pub fn keep_trees(&self, id: SessionId, seq: u64, trees: Option<TurnTrees>) {
        let mut slots = self.slots.lock().expect("turns lock");
        let Some(slot) = slots.get_mut(&id) else {
            return;
        };
        if seq >= slot.closed_seq {
            slot.closed_seq = seq;
            slot.trees = trees;
        }
        let Ok(entries) = std::fs::read_dir(self.session_dir(id)) else {
            return;
        };
        for entry in entries.flatten() {
            let older = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u64>().ok())
                .is_some_and(|n| n < slot.closed_seq);
            if older {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    /// As árvores do último turno que fechou com diff.
    pub fn last_trees(&self, id: SessionId) -> Option<TurnTrees> {
        self.slots
            .lock()
            .expect("turns lock")
            .get(&id)
            .and_then(|slot| slot.trees.clone())
    }

    /// As árvores do turno `seq`, só enquanto ele for o último que fechou:
    /// turno antigo guarda só os números.
    pub fn trees_of(&self, id: SessionId, seq: u64) -> Option<TurnTrees> {
        self.slots
            .lock()
            .expect("turns lock")
            .get(&id)
            .filter(|slot| slot.closed_seq == seq)
            .and_then(|slot| slot.trees.clone())
    }

    /// A sessão acabou (ou vai subir de novo): some o turno aberto, as
    /// árvores e o diretório privado dela.
    pub fn forget(&self, id: SessionId) {
        self.slots.lock().expect("turns lock").remove(&id);
        let _ = std::fs::remove_dir_all(self.session_dir(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::diff::FileStatus;
    use std::collections::BTreeSet;

    /// Folgado: nenhum teste depende dos tetos de produção.
    const BOUND: Duration = Duration::from_secs(120);

    fn git(dir: &Path, args: &[&str]) {
        assert!(try_git(dir, args), "git {args:?}");
    }

    fn try_git(dir: &Path, args: &[&str]) -> bool {
        git_out(dir, args).is_some()
    }

    fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("git");
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Uma pasta de trabalho e, ao lado dela (fora da pasta), a raiz dos
    /// diretórios privados do teste.
    struct Fx {
        tmp: tempfile::TempDir,
        name: &'static str,
    }

    impl Fx {
        fn path(&self) -> &Path {
            Box::leak(self.tmp.path().join(self.name).into_boxed_path())
        }

        fn private(&self) -> PathBuf {
            self.tmp.path().join("privado")
        }
    }

    fn plain_named(name: &'static str) -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(name)).unwrap();
        Fx { tmp, name }
    }

    /// Pasta sem git.
    fn plain() -> Fx {
        plain_named("pasta")
    }

    fn init(dir: &Path) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "t@example.com"]);
        git(dir, &["config", "user.name", "t"]);
    }

    fn seed(p: &Path) {
        init(p);
        write(p, "a.txt", "um\ndois\ntres\n");
        write(p, "b.txt", "conteudo estavel do b\noutra linha\n");
        write(p, "some.txt", "vai embora\n");
        write(p, ".gitignore", "*.log\n");
        git(p, &["add", "-A"]);
        git(p, &["commit", "-qm", "base"]);
    }

    /// Repositório com um commit: `a.txt`, `b.txt`, `some.txt` e um
    /// `.gitignore` que ignora `*.log`. A config global do dono não entra
    /// (`hermetic_git_config` em `lib.rs`).
    fn repo() -> Fx {
        let fx = plain();
        seed(fx.path());
        fx
    }

    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    /// O diretório privado do turno de teste: ao lado da pasta.
    fn scratch_of(dir: &Path) -> PathBuf {
        dir.parent().unwrap().join("privado").join("turno")
    }

    fn start(dir: &Path) -> TurnStart {
        let scratch = scratch_of(dir);
        TurnStart {
            seq: 1,
            started_at_ms: 1_000,
            prompt: Some("arruma o a.txt".into()),
            usage: None,
            repo: snapshot(dir, &scratch, BOUND),
            scratch,
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
        let end = snapshot(dir, &start.scratch, BOUND);
        digest(start, &end, &ending(TurnOutcome::Completed), None, BOUND)
    }

    /// (caminho, status, +, -) de cada arquivo.
    fn rows(d: &TurnDigest) -> Vec<(&str, FileStatus, Option<u32>, Option<u32>)> {
        d.files
            .iter()
            .map(|f| (f.path.as_str(), f.status, f.insertions, f.deletions))
            .collect()
    }

    /// Todo caminho debaixo de `dir`, relativo a ele.
    fn listing(dir: &Path) -> BTreeSet<String> {
        fn walk(base: &Path, dir: &Path, out: &mut BTreeSet<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                out.insert(path.strip_prefix(base).unwrap().display().to_string());
                if entry.file_type().unwrap().is_dir() {
                    walk(base, &path, out);
                }
            }
        }
        let mut out = BTreeSet::new();
        if dir.is_dir() {
            walk(dir, dir, &mut out);
        }
        out
    }

    /// Onde o blob de `file` moraria num banco de objetos.
    fn loose(objects: &Path, repo: &Path, file: &str) -> PathBuf {
        let sha = git_out(repo, &["hash-object", file]).unwrap();
        objects.join(&sha[..2]).join(&sha[2..])
    }

    #[test]
    fn nothing_changed_gives_an_empty_list() {
        let fx = repo();
        let d = end(&start(fx.path()), fx.path());
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
        let fx = repo();
        let p = fx.path();
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
        let fx = repo();
        let p = fx.path();
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
        let fx = repo();
        let p = fx.path();
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
        let fx = repo();
        let p = fx.path();
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
        let fx = repo();
        let p = fx.path();
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
        let fx = repo();
        let p = fx.path();
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
        let fx = repo();
        let p = fx.path();
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

    /// Os commits contam até o `HEAD` do snapshot do fim, não até o `HEAD`
    /// de quando o resumo é calculado.
    #[test]
    fn commits_count_up_to_the_end_snapshot() {
        let fx = repo();
        let p = fx.path();
        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        git(p, &["commit", "-qam", "do turno"]);
        let end = snapshot(p, &s.scratch, BOUND);
        write(p, "a.txt", "um\ndois\ntres\nquatro\ncinco\n");
        git(p, &["commit", "-qam", "depois do turno"]);
        let d = digest(&s, &end, &ending(TurnOutcome::Completed), None, BOUND);
        assert_eq!(d.commits, 1);
        assert_eq!(
            rows(&d),
            [("a.txt", FileStatus::Modified, Some(1), Some(0))]
        );
    }

    #[test]
    fn head_that_went_back_counts_no_commit() {
        let fx = repo();
        let p = fx.path();
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
        let fx = plain();
        let p = fx.path();
        let s = start(p);
        assert_eq!(s.repo, Snapshot::NoRepo);
        write(p, "novo.txt", "oi\n");
        let d = end(&s, p);
        assert_eq!(d.diff, TurnDiff::NoRepo);
        assert!(d.files.is_empty());
        assert_eq!(d.duration_ms, 3_500);
        // Nem o diretório privado nasce.
        assert!(listing(&fx.private()).is_empty());
    }

    /// Exit 128 que não é "pasta sem git": a pasta sumiu, ou o `.git` aponta
    /// para lugar nenhum. É `unavailable`, não `no_repo`.
    #[test]
    fn broken_repo_and_missing_folder_are_unavailable() {
        let fx = plain();
        let p = fx.path();
        let gone = p.join("nao-existe");
        assert_eq!(
            snapshot(&gone, &scratch_of(p), BOUND),
            Snapshot::Unavailable
        );

        write(p, ".git", "gitdir: /caminho/que/nao/existe\n");
        let s = start(p);
        assert_eq!(s.repo, Snapshot::Unavailable);
        assert_eq!(end(&s, p).diff, TurnDiff::Unavailable);
        // Um subdiretório dele também não vira "pasta sem git".
        std::fs::create_dir(p.join("sub")).unwrap();
        assert_eq!(
            snapshot(&p.join("sub"), &scratch_of(p), BOUND),
            Snapshot::Unavailable
        );
    }

    #[test]
    fn repo_without_a_commit_is_a_normal_diff() {
        let fx = plain();
        let p = fx.path();
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

    /// O snapshot não escreve nada no gitdir do usuário: nem índice, nem
    /// trava, nem objeto. O conteúdo do arquivo não rastreado só existe no
    /// diretório privado.
    #[test]
    fn snapshot_writes_nothing_under_the_users_gitdir() {
        let fx = repo();
        let p = fx.path();
        write(p, "a.txt", "um\ndois\ntres\nsujo\n");
        write(p, "staged.txt", "no indice\n");
        git(p, &["add", "staged.txt"]);
        write(p, ".env", "TOKEN=valor-de-mentira-do-teste\n");
        let git_dir = p.join(".git");
        let index = git_dir.join("index");
        let before = (listing(&git_dir), std::fs::read(&index).unwrap());
        let mtime = std::fs::metadata(&index).unwrap().modified().unwrap();

        let s = start(p);
        assert!(matches!(s.repo, Snapshot::Tree(_)));
        write(p, ".env", "TOKEN=outro-valor-de-mentira\n");
        assert_eq!(
            rows(&end(&s, p)),
            [(".env", FileStatus::Modified, Some(1), Some(1))]
        );

        assert_eq!(listing(&git_dir), before.0);
        assert_eq!(std::fs::read(&index).unwrap(), before.1);
        assert_eq!(
            std::fs::metadata(&index).unwrap().modified().unwrap(),
            mtime
        );
        assert!(!loose(&git_dir.join("objects"), p, ".env").exists());
        assert!(loose(&s.scratch.join("objects"), p, ".env").exists());
        // O índice temporário não fica nem no diretório privado.
        assert!(listing(&s.scratch)
            .iter()
            .all(|f| !f.starts_with(TEMP_INDEX_PREFIX)));
    }

    #[cfg(unix)]
    #[test]
    fn private_dirs_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let fx = repo();
        let s = start(fx.path());
        for dir in [fx.private(), s.scratch.clone(), s.scratch.join("objects")] {
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{}", dir.display());
        }
    }

    /// Índice dividido: sem `core.splitIndex=false` em todo comando que usa
    /// a cópia, cada snapshot gravava um `sharedindex.*` novo no gitdir.
    #[test]
    fn snapshot_with_split_index_adds_no_shared_index() {
        let fx = repo();
        let p = fx.path();
        git(p, &["config", "core.splitIndex", "true"]);
        git(p, &["update-index", "--split-index"]);
        write(p, "novo.txt", "nao rastreado\n");
        let git_dir = p.join(".git");
        let before = listing(&git_dir);
        assert!(before.iter().any(|f| f.starts_with("sharedindex.")));

        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        for _ in 0..3 {
            assert_eq!(
                rows(&end(&s, p)),
                [("a.txt", FileStatus::Modified, Some(1), Some(0))]
            );
        }
        assert_eq!(listing(&git_dir), before);
    }

    /// Worktree vinculado: os objetos de verdade moram no `.git` do
    /// repositório principal, e é de lá que a alternativa tem de sair.
    #[test]
    fn snapshot_works_in_a_linked_worktree() {
        let fx = repo();
        let p = fx.path();
        let wt = p.parent().unwrap().join("vinculado");
        git(p, &["worktree", "add", "-q", wt.to_str().unwrap()]);
        let before = listing(&p.join(".git"));
        let scratch = scratch_of(p);
        let s = TurnStart {
            repo: snapshot(&wt, &scratch, BOUND),
            ..start(p)
        };
        write(&wt, "a.txt", "um\ndois\ntres\nquatro\n");
        write(&wt, "novo.txt", "oi\n");
        let d = end(&s, &wt);
        assert_eq!(
            rows(&d),
            [
                ("a.txt", FileStatus::Modified, Some(1), Some(0)),
                ("novo.txt", FileStatus::Added, Some(1), Some(0)),
            ]
        );
        assert_eq!(listing(&p.join(".git")), before);
    }

    /// Caminho com `:` quebraria a lista de `GIT_ALTERNATE_OBJECT_DIRECTORIES`
    /// sem as aspas.
    #[cfg(unix)]
    #[test]
    fn repo_path_with_a_colon_still_reads_its_objects() {
        let fx = plain_named("re:po");
        let p = fx.path();
        seed(p);
        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        assert_eq!(
            rows(&end(&s, p)),
            [("a.txt", FileStatus::Modified, Some(1), Some(0))]
        );
    }

    #[test]
    fn snapshot_works_with_the_index_locked_by_someone_else() {
        let fx = repo();
        let p = fx.path();
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
        let fx = repo();
        let p = fx.path();
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
        let before = (
            listing(&p.join(".git")),
            std::fs::read(p.join(".git/index")).unwrap(),
        );

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
        assert_eq!(listing(&p.join(".git")), before.0);
        assert_eq!(std::fs::read(p.join(".git/index")).unwrap(), before.1);
    }

    /// Um arquivo que o `add` não lê não derruba o diff do turno: a árvore
    /// sai sem ele.
    #[cfg(unix)]
    #[test]
    fn unreadable_file_does_not_cost_the_turn_its_diff() {
        use std::os::unix::fs::PermissionsExt;
        let fx = repo();
        let p = fx.path();
        write(p, "fechado.txt", "ninguem le\n");
        let closed = std::fs::Permissions::from_mode(0o000);
        std::fs::set_permissions(p.join("fechado.txt"), closed).unwrap();
        if std::fs::read(p.join("fechado.txt")).is_ok() {
            return; // root lê tudo: não há o que provar aqui.
        }
        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        write(p, "zeta.txt", "depois do fechado na ordem\n");
        let d = end(&s, p);
        assert_eq!(d.diff, TurnDiff::Ok);
        assert_eq!(
            rows(&d),
            [
                ("a.txt", FileStatus::Modified, Some(1), Some(0)),
                ("zeta.txt", FileStatus::Added, Some(1), Some(0)),
            ]
        );
    }

    /// `git init` aninhado sem commit: o `add -A` morre com 128.
    #[test]
    fn nested_repo_without_a_commit_does_not_cost_the_turn_its_diff() {
        let fx = repo();
        let p = fx.path();
        std::fs::create_dir(p.join("meio")).unwrap();
        init(&p.join("meio"));
        write(&p.join("meio"), "dentro.txt", "do outro repo\n");
        let s = start(p);
        assert!(matches!(s.repo, Snapshot::Tree(_)));
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        write(p, "zeta.txt", "depois do aninhado na ordem\n");
        let d = end(&s, p);
        assert_eq!(d.diff, TurnDiff::Ok);
        assert_eq!(
            rows(&d),
            [
                ("a.txt", FileStatus::Modified, Some(1), Some(0)),
                ("zeta.txt", FileStatus::Added, Some(1), Some(0)),
            ]
        );
    }

    /// Arquivo novo fora do cone do sparse-checkout: o `add -A` sai com 1.
    #[test]
    fn file_outside_the_sparse_cone_does_not_cost_the_turn_its_diff() {
        let fx = repo();
        let p = fx.path();
        std::fs::create_dir(p.join("dentro")).unwrap();
        write(p, "dentro/x.txt", "no cone\n");
        git(p, &["add", "-A"]);
        git(p, &["commit", "-qm", "dentro"]);
        git(p, &["sparse-checkout", "set", "--cone", "dentro"]);
        std::fs::create_dir(p.join("fora")).unwrap();
        write(p, "fora/y.txt", "fora do cone\n");
        let s = start(p);
        assert!(matches!(s.repo, Snapshot::Tree(_)));
        write(p, "dentro/x.txt", "no cone\nmudou\n");
        let d = end(&s, p);
        assert_eq!(d.diff, TurnDiff::Ok);
        assert!(rows(&d).contains(&("dentro/x.txt", FileStatus::Modified, Some(1), Some(0))));
    }

    /// Repositório com filtro de conteúdo: o git roda enjaulado. A jaula do
    /// snapshot deixa escrever no diretório privado e em mais lugar nenhum —
    /// um filtro `clean` do repositório não consegue criar arquivo na pasta.
    /// Só macOS: no Linux a jaula depende de user namespace, que a CI não tem.
    #[cfg(target_os = "macos")]
    #[test]
    fn clean_filter_cannot_write_into_the_repo_during_a_snapshot() {
        let fx = repo();
        let p = fx.path();
        let planted = p.join("plantado-pelo-filtro");
        let filter = format!("touch '{}'; cat", planted.display());
        git(p, &["config", "filter.espiao.clean", &filter]);
        write(p, ".gitattributes", "*.txt filter=espiao\n");
        assert!(crate::sandbox::git::repo_uses_content_filter(p));
        let before = listing(&p.join(".git"));

        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        let d = end(&s, p);
        assert_eq!(d.diff, TurnDiff::Ok);
        assert_eq!(
            rows(&d),
            [("a.txt", FileStatus::Modified, Some(1), Some(0))]
        );
        assert!(!planted.exists(), "o filtro escreveu na pasta do usuário");
        assert_eq!(listing(&p.join(".git")), before);
    }

    #[test]
    fn snapshot_past_the_bound_is_unavailable() {
        let fx = repo();
        let p = fx.path();
        let scratch = scratch_of(p);
        assert_eq!(snapshot(p, &scratch, Duration::ZERO), Snapshot::Unavailable);

        let s = start(p);
        let gone = Snapshot::Unavailable;
        let d = digest(&s, &gone, &ending(TurnOutcome::Completed), None, BOUND);
        assert_eq!(d.diff, TurnDiff::Unavailable);
        assert!(trees(&s, &gone).is_none());
        // Diff que estoura o teto dele: sem diff, sem travar.
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        let end = snapshot(p, &s.scratch, BOUND);
        let d = digest(
            &s,
            &end,
            &ending(TurnOutcome::Completed),
            None,
            Duration::ZERO,
        );
        assert_eq!(d.diff, TurnDiff::Unavailable);
    }

    #[cfg(unix)]
    fn alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[cfg(unix)]
    fn wait_gone(pid: i32) -> bool {
        let clock = Instant::now();
        while alive(pid) {
            if clock.elapsed() > Duration::from_secs(10) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    /// O prazo mata o GRUPO: o neto (o `sleep` que o `sh` deixou em segundo
    /// plano) morre junto, não só o filho direto.
    #[cfg(unix)]
    #[test]
    fn child_past_the_deadline_is_killed_with_its_group() {
        let tmp = tempfile::tempdir().unwrap();
        let pidfile = tmp.path().join("neto.pid");
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("sleep 600 & echo $! > \"$0\"; wait")
            .arg(&pidfile);
        cmd.stdin(Stdio::null());
        let clock = Instant::now();
        let ran = run_within(cmd, Instant::now() + Duration::from_millis(1500), true);
        assert!(matches!(ran, Ran::TimedOut));
        assert!(clock.elapsed() < Duration::from_secs(30));
        let grandchild: i32 = std::fs::read_to_string(&pidfile)
            .expect("o sh não chegou a gravar o pid do neto")
            .trim()
            .parse()
            .unwrap();
        assert!(wait_gone(grandchild), "o neto sobreviveu ao prazo");
    }

    /// Snapshot morto no meio do `add` (um filtro `clean` que não volta): o
    /// filtro morre com o grupo, o gitdir do usuário fica como estava, o
    /// índice temporário some e o que sobrou no diretório privado vai embora
    /// com a sessão.
    #[cfg(unix)]
    #[test]
    fn snapshot_killed_mid_add_leaves_nothing_behind() {
        let fx = repo();
        let p = fx.path();
        let id = SessionId::new_v4();
        // Dentro do diretório privado do turno: é o único lugar em que a
        // jaula do snapshot deixa o filtro escrever.
        let pidfile = fx.private().join(id.to_string()).join("1/filtro.pid");
        let filter = format!("echo $$ > '{}'; exec sleep 600", pidfile.display());
        git(p, &["config", "filter.lento.clean", &filter]);
        write(p, ".gitattributes", "*.txt filter=lento\n");
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        let before = listing(&p.join(".git"));

        let turns = Turns::at(fx.private());
        let bound = Duration::from_secs(4);
        turns.begin(id, p.to_path_buf(), bound, 1, None, None);
        let s = turns.end(id).unwrap().into_start();
        assert_eq!(s.repo, Snapshot::Unavailable);
        assert_eq!(listing(&p.join(".git")), before);
        assert!(listing(&s.scratch)
            .iter()
            .all(|f| !f.starts_with(TEMP_INDEX_PREFIX)));
        // No Linux sem user namespace a jaula recusa o git antes do filtro
        // rodar: aí não há filtro para conferir, e o resto vale igual.
        if let Ok(pid) = std::fs::read_to_string(&pidfile) {
            assert!(
                wait_gone(pid.trim().parse().unwrap()),
                "o filtro sobreviveu ao prazo"
            );
        }
        turns.forget(id);
        assert!(listing(&fx.private()).is_empty());
    }

    #[test]
    fn list_is_capped_and_totals_are_not() {
        let fx = repo();
        let p = fx.path();
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
            scratch: PathBuf::new(),
        };
        let ending = TurnEnding {
            outcome: TurnOutcome::Failed,
            failure: Some("limite de uso atingido".into()),
            ended_at_ms: 10,
        };
        digest(&s, &Snapshot::NoRepo, &ending, at_end.as_ref(), BOUND)
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
        let fx = plain();
        let turns = Turns::at(fx.private());
        let id = SessionId::new_v4();
        assert!(turns.end(id).is_none());
        assert!(turns.end_if_stopped(id).is_none());
        assert!(!turns.is_open(id));
    }

    /// O turno fecha uma vez só, venha o fim de onde vier (hook, coordenador
    /// ou saída do processo), e uma sessão não enxerga o turno da outra.
    #[test]
    fn a_turn_closes_once_whoever_gets_there_first() {
        let fx = repo();
        let turns = Turns::at(fx.private());
        let (id, other) = (SessionId::new_v4(), SessionId::new_v4());
        assert!(begin(&turns, id, fx.path(), 7).is_none());
        assert!(turns.is_open(id));
        assert!(turns.end(other).is_none());

        let open = turns.end(id).expect("turno aberto");
        assert!(!turns.is_open(id));
        assert!(turns.end(id).is_none());
        let s = open.into_start();
        assert_eq!((s.seq, s.started_at_ms), (1, 7));
        assert_eq!(s.prompt.as_deref(), Some("oi"));
        assert!(matches!(s.repo, Snapshot::Tree(_)));
        assert_eq!(s.scratch, fx.private().join(id.to_string()).join("1"));
    }

    /// Prompt novo com turno ainda aberto (Esc e negação no prompt nativo
    /// não mandam hook): o aberto volta para ser fechado, e o novo segue.
    #[test]
    fn a_new_prompt_hands_back_the_turn_still_open() {
        let fx = repo();
        let turns = Turns::at(fx.private());
        let id = SessionId::new_v4();
        assert!(begin(&turns, id, fx.path(), 10).is_none());
        let displaced = begin(&turns, id, fx.path(), 20).expect("turno anterior");
        assert_eq!((displaced.seq, displaced.stop_seen_at_ms), (1, None));
        assert!(turns.is_open(id));
        assert_eq!(turns.end(id).expect("turno novo").seq, 2);
        assert!(begin(&turns, id, fx.path(), 30).is_none());
        assert_eq!(turns.end(id).unwrap().seq, 3);
    }

    /// `Stop` segurado (subagente vivo, pendência): o turno segue aberto,
    /// mas quem o fechar depois sabe que o orquestrador terminou, e quando.
    /// Quem só fecha turno com `Stop` visto não leva um turno recém-aberto.
    #[test]
    fn held_stop_is_remembered_until_the_orchestrator_works_again() {
        let fx = repo();
        let turns = Turns::at(fx.private());
        let id = SessionId::new_v4();
        begin(&turns, id, fx.path(), 10);
        assert!(turns.end_if_stopped(id).is_none());
        assert!(turns.is_open(id));

        turns.note_stop(id, 50);
        let displaced = begin(&turns, id, fx.path(), 90).expect("turno anterior");
        assert_eq!(displaced.stop_seen_at_ms, Some(50));
        // O turno novo não herda o `Stop` do anterior.
        assert!(turns.end_if_stopped(id).is_none());

        turns.note_stop(id, 120);
        turns.note_working(id);
        assert!(turns.end_if_stopped(id).is_none());
        turns.note_stop(id, 150);
        let open = turns.end_if_stopped(id).expect("Stop visto");
        assert_eq!((open.seq, open.stop_seen_at_ms), (2, Some(150)));
        assert!(!turns.is_open(id));
    }

    fn pair(start: &str) -> TurnTrees {
        TurnTrees {
            root: PathBuf::from("/repo"),
            start: start.into(),
            end: "fim".into(),
            scratch: PathBuf::from("/privado"),
            objects: PathBuf::from("/repo/.git/objects"),
        }
    }

    #[test]
    fn trees_of_an_older_turn_never_replace_a_newer_one() {
        let fx = repo();
        let turns = Turns::at(fx.private());
        let id = SessionId::new_v4();
        begin(&turns, id, fx.path(), 10);
        turns.keep_trees(id, 2, Some(pair("dois")));
        turns.keep_trees(id, 1, Some(pair("um")));
        assert_eq!(turns.last_trees(id), Some(pair("dois")));
        assert_eq!(turns.trees_of(id, 2), Some(pair("dois")));
        // Só o último turno tem diff: o anterior guarda só os números.
        assert_eq!(turns.trees_of(id, 1), None);
        // Turno mais novo sem diff: as do anterior não servem mais.
        turns.keep_trees(id, 3, None);
        assert_eq!(turns.last_trees(id), None);
        assert_eq!(turns.trees_of(id, 2), None);
        assert_eq!(turns.trees_of(id, 3), None);
    }

    /// As árvores de um turno de verdade: o que já estava sujo fica de fora
    /// da lista e dos trechos, e arquivo sem rastreio entra.
    #[test]
    fn turn_diff_lists_files_and_reads_hunks_from_the_two_trees() {
        let fx = repo();
        let p = fx.path();
        write(p, "b.txt", "sujo antes do prompt\n");
        let s = start(p);
        write(p, "a.txt", "um\nDOIS\ntres\n");
        write(p, "novo.txt", "sem rastreio\n");
        git(p, &["mv", "some.txt", "foi.txt"]);
        let end = snapshot(p, &s.scratch, BOUND);
        let pair = trees(&s, &end).expect("duas árvores");

        let files = pair.files(BOUND).expect("lista");
        let got: Vec<_> = files
            .iter()
            .map(|f| (f.path.as_str(), f.status, f.old_path.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("a.txt", FileStatus::Modified, None),
                ("foi.txt", FileStatus::Renamed, Some("some.txt")),
                ("novo.txt", FileStatus::Added, None),
            ]
        );

        let lines = |h: &FileHunks| -> Vec<String> {
            h.hunks
                .iter()
                .flat_map(|hunk| hunk.lines.iter().map(|l| l.text.clone()))
                .collect()
        };
        // A pasta muda depois do fim do turno: os trechos são os do turno.
        write(p, "a.txt", "mexido depois do turno\n");
        let a = pair.file_hunks("a.txt", None, BOUND).expect("trechos");
        assert!(lines(&a).contains(&"DOIS".to_string()), "{a:?}");
        assert!(!lines(&a).iter().any(|l| l.contains("depois do turno")));
        let novo = pair.file_hunks("novo.txt", None, BOUND).expect("trechos");
        assert_eq!(lines(&novo), ["sem rastreio"]);
        // Sujo antes do prompt e igual no fim: nada a mostrar.
        let b = pair.file_hunks("b.txt", None, BOUND).expect("trechos");
        assert!(b.hunks.is_empty());
    }

    /// `textconv` é comando definido pelo repositório: não roda no diff do
    /// turno. Sem o `--no-textconv`, onde há jaula o git falha ao criar o
    /// temporário do textconv e os trechos não vêm (quem acusa é o
    /// `expect("trechos")`); onde não há, o comando roda e deixa o marcador.
    #[test]
    fn turn_diff_does_not_run_textconv() {
        let fx = repo();
        let p = fx.path();
        let marker = fx.tmp.path().join("rodou");
        write(p, ".gitattributes", "*.txt diff=espia\n");
        git(
            p,
            &[
                "config",
                "diff.espia.textconv",
                &format!("sh -c 'touch {}; cat \"$1\"' --", marker.display()),
            ],
        );
        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        let end = snapshot(p, &s.scratch, BOUND);
        let pair = trees(&s, &end).expect("duas árvores");
        assert_eq!(pair.files(BOUND).expect("lista").len(), 1);
        let hunks = pair.file_hunks("a.txt", None, BOUND).expect("trechos");
        assert!(!hunks.hunks.is_empty());
        assert!(!marker.exists(), "textconv rodou");
    }

    /// Diretório privado apagado (o turno seguinte fechou): sem diff, sem
    /// pânico.
    #[test]
    fn turn_diff_without_its_private_dir_is_none() {
        let fx = repo();
        let p = fx.path();
        let s = start(p);
        write(p, "novo.txt", "sem rastreio\n");
        let end = snapshot(p, &s.scratch, BOUND);
        let pair = trees(&s, &end).expect("duas árvores");
        std::fs::remove_dir_all(&pair.scratch).unwrap();
        assert_eq!(pair.files(BOUND), None);
        assert_eq!(pair.file_hunks("novo.txt", None, BOUND), None);
    }

    /// Só o diretório do turno aberto e o do último que fechou ficam; o da
    /// sessão inteira some com ela, sem tocar no das outras.
    #[test]
    fn private_dirs_go_away_as_turns_finish_and_with_the_session() {
        let fx = repo();
        let turns = Turns::at(fx.private());
        let (id, other) = (SessionId::new_v4(), SessionId::new_v4());
        let session = fx.private().join(id.to_string());
        let dirs = || {
            let mut names: Vec<String> = std::fs::read_dir(&session)
                .map(|d| {
                    d.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        };
        let finish = |seq: u64| {
            let s = turns.end(id).unwrap().into_start();
            assert_eq!(s.seq, seq);
            turns.keep_trees(id, seq, Some(pair("x")));
        };
        begin(&turns, other, fx.path(), 1);
        turns.end(other).unwrap().into_start();

        begin(&turns, id, fx.path(), 10);
        finish(1);
        assert_eq!(dirs(), ["1"]);
        begin(&turns, id, fx.path(), 20);
        let open = turns.end(id).unwrap().into_start();
        begin(&turns, id, fx.path(), 30);
        // O 2 fechou e o 3 está aberto: o 1 vai embora.
        turns.keep_trees(id, open.seq, None);
        turns.end(id).unwrap().into_start();
        assert_eq!(dirs(), ["2", "3"]);
        // Resumo atrasado de um turno antigo não ressuscita nem apaga o novo.
        turns.keep_trees(id, 1, None);
        assert_eq!(dirs(), ["2", "3"]);
        turns.keep_trees(id, 3, None);
        assert_eq!(dirs(), ["3"]);

        turns.forget(id);
        assert!(!session.exists());
        assert!(!turns.is_open(id));
        assert!(fx.private().join(other.to_string()).is_dir());

        sweep(&fx.private());
        assert!(!fx.private().exists());
    }

    #[test]
    fn trees_come_from_the_two_snapshots() {
        let fx = repo();
        let p = fx.path();
        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        let end = snapshot(p, &s.scratch, BOUND);
        let pair = trees(&s, &end).expect("duas árvores");
        assert_ne!(pair.start, pair.end);
        assert!(matches!(&s.repo, Snapshot::Tree(t) if t.tree == pair.start));
        assert_eq!(pair.scratch, s.scratch);
        assert_eq!(
            crate::repo::canonicalize_or(&pair.objects),
            crate::repo::canonicalize_or(&p.join(".git/objects"))
        );
    }

    #[test]
    fn counts_and_notice_line_carry_numbers_only() {
        let fx = repo();
        let p = fx.path();
        let s = start(p);
        write(p, "a.txt", "um\nDOIS\ntres\nquatro\n");
        write(p, "segredo.env", "TOKEN=abc\n");
        let mut d = end(&s, p);
        d.duration_ms = 291_000;
        d.cost_usd = Some(0.41);
        assert_eq!(d.notice_line(), "2 arquivos · +3 −1 · 4m 51s");
        let counts = d.counts();
        assert_eq!(
            (counts.files, counts.insertions, counts.deletions),
            (Some(2), 3, 1)
        );
        let json = serde_json::to_string(&counts).unwrap();
        for leak in ["a.txt", "segredo", "arruma"] {
            assert!(!json.contains(leak), "{leak} nas contagens: {json}");
        }

        d.files_total = 1;
        assert!(d.notice_line().starts_with("1 arquivo · "));
        d.files_total = 0;
        d.duration_ms = 36_000;
        assert_eq!(d.notice_line(), "sem mudanças · 36s");
        d.diff = TurnDiff::Unavailable;
        assert_eq!(d.notice_line(), "36s");
        assert_eq!(d.counts().files, None);
    }

    /// O aviso do sistema só leva os números do turno que acabou de fechar.
    #[test]
    fn notice_body_only_carries_this_turns_numbers() {
        use crate::agent::session::notice_body;
        let fx = repo();
        let p = fx.path();
        let s = start(p);
        write(p, "a.txt", "um\ndois\ntres\nquatro\n");
        let d = end(&s, p);
        assert_eq!(d.ended_at_ms, 4_500);
        assert_eq!(
            notice_body("Pronto.", Some(&d), 4_600),
            "Pronto.\n1 arquivo · +1 −0 · 3s"
        );
        // Resumo do turno anterior (o deste ainda não ficou pronto) e sessão
        // sem resumo: o aviso sai como antes.
        assert_eq!(notice_body("Pronto.", Some(&d), 60_000), "Pronto.");
        assert_eq!(notice_body("Pronto.", None, 4_600), "Pronto.");
    }

    #[test]
    fn digest_serializes_paths_and_counts_only() {
        let fx = repo();
        let p = fx.path();
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
        assert!(!text.contains("privado"));
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
