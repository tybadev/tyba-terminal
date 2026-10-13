//! Histórico do repo (spec: git-completo, fatias 1–2).
//!
//! Regras de performance da spec:
//! - log SEMPRE paginado: `log -z --topo-order --max-count=N`, nunca o repo
//!   inteiro numa chamada e nunca `--graph`;
//! - paginação sem `--skip`: o cursor guarda a **fronteira** (pais já vistos
//!   e ainda não mostrados) e as pontas ainda não alcançadas. Como o
//!   `--topo-order` só mostra um commit depois de todos os filhos dele, o que
//!   já saiu é fechado para descendentes, e o que falta é exatamente o que se
//!   alcança a partir da fronteira + pontas não mostradas. A página seguinte é
//!   um `log` novo a partir daí — custo proporcional à página, não à posição;
//! - com filtro (autor, caminho, busca) o grafo não tem significado (os pais
//!   reais somem da lista) e a fronteira não basta: cai para `--skip`, que só
//!   custa em resultado filtrado — pequeno por natureza.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use super::diff::FileDiff;
use super::graph::{GraphRow, GraphState};
use super::{git_in, git_in_rw, run_git};
use crate::error::AppError;

pub const PAGE_DEFAULT: usize = 200;
pub const PAGE_MAX: usize = 500;
/// Subject é uma linha, mas nada impede uma linha de 50 KB.
const SUBJECT_MAX: usize = 300;
/// Arquivos de um commit no detalhe — lockfile regenerado vira milhares.
pub const DETAIL_FILES_MAX: usize = 1000;
/// Corpo da mensagem no detalhe.
const BODY_MAX: usize = 16 * 1024;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFilter {
    /// Branch/ref de partida (default `HEAD`). Ignorado com `all_branches`.
    #[serde(default)]
    pub ref_name: Option<String>,
    #[serde(default)]
    pub all_branches: bool,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    /// Texto da mensagem, ou um hash (prefixo de 4+ hex).
    #[serde(default)]
    pub query: Option<String>,
}

fn non_empty(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

impl LogFilter {
    pub fn is_filtered(&self) -> bool {
        non_empty(&self.author).is_some()
            || non_empty(&self.path).is_some()
            || non_empty(&self.query).is_some()
    }
}

/// Estado para continuar o log. Opaco para a UI: ela só devolve o que
/// recebeu. Tudo que vira argv é revalidado ao voltar (é fronteira de IPC).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LogCursor {
    /// Pais já vistos e ainda não mostrados (modo sem filtro).
    #[serde(default)]
    pub frontier: Vec<String>,
    /// Pontas ainda não mostradas (modo sem filtro).
    #[serde(default)]
    pub tips: Vec<String>,
    /// Modo filtrado: quantos já saíram.
    #[serde(default)]
    pub skip: usize,
    /// Lanes do grafo no fim da página anterior (modo sem filtro).
    #[serde(default)]
    pub graph: GraphState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    /// `HEAD` destacado (sem branch).
    Head,
    Branch,
    Remote,
    Tag,
    /// Branch de worktree de agente (`tyba/*`).
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
    /// `HEAD -> <esta branch>`.
    pub is_head: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogRow {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub authored_at: String,
    pub refs: Vec<RefLabel>,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogPage {
    pub rows: Vec<LogRow>,
    /// `None` = acabou o histórico.
    pub cursor: Option<LogCursor>,
    /// Página filtrada: sem grafo (os pais reais não estão na lista).
    pub filtered: bool,
    /// Uma linha de grafo por `rows[i]`; vazio na página filtrada.
    pub graph: Vec<GraphRow>,
}

/// `%D` com `--decorate=full`: `HEAD -> refs/heads/main, refs/remotes/origin/main,
/// tag: refs/tags/v1`. Full para não confundir uma branch local chamada
/// `origin/x` com a remota.
pub fn parse_decorations(raw: &str) -> Vec<RefLabel> {
    let mut out = Vec::new();
    for part in raw.split(", ").map(str::trim).filter(|p| !p.is_empty()) {
        let (is_head, part) = match part.strip_prefix("HEAD -> ") {
            Some(rest) => (true, rest),
            None => (false, part),
        };
        let label = if part == "HEAD" {
            RefLabel {
                name: "HEAD".into(),
                kind: RefKind::Head,
                is_head: true,
            }
        } else if let Some(tag) = part
            .strip_prefix("tag: ")
            .map(|t| t.strip_prefix("refs/tags/").unwrap_or(t))
        {
            RefLabel {
                name: tag.into(),
                kind: RefKind::Tag,
                is_head: false,
            }
        } else if let Some(branch) = part.strip_prefix("refs/heads/") {
            RefLabel {
                name: branch.into(),
                kind: if branch.starts_with("tyba/") {
                    RefKind::Agent
                } else {
                    RefKind::Branch
                },
                is_head,
            }
        } else if let Some(remote) = part.strip_prefix("refs/remotes/") {
            // `origin/HEAD` é ponteiro, não trabalho — ruído no chip.
            if remote.ends_with("/HEAD") {
                continue;
            }
            RefLabel {
                name: remote.into(),
                kind: RefKind::Remote,
                is_head: false,
            }
        } else {
            // refs/stash, notes, refs de forja: fora do histórico.
            continue;
        };
        out.push(label);
    }
    out
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// `git log -z --format=%H%x1f%P%x1f%an%x1f%aI%x1f%D%x1f%s`: registros por
/// NUL, campos por US. O subject é o ÚLTIMO campo e engole o que sobrar —
/// ele pode conter o próprio separador.
pub fn parse_log_rows(raw: &[u8]) -> Vec<LogRow> {
    raw.split(|b| *b == 0)
        .filter(|r| !r.iter().all(u8::is_ascii_whitespace))
        .filter_map(|record| {
            let text = String::from_utf8_lossy(record);
            let text = text.trim_start_matches('\n');
            let mut parts = text.splitn(6, '\x1f');
            let sha = parts.next()?.trim().to_string();
            let parents = parts
                .next()?
                .split_whitespace()
                .map(str::to_string)
                .collect();
            let author = parts.next()?.to_string();
            let authored_at = parts.next()?.to_string();
            let refs = parse_decorations(parts.next()?);
            let subject = truncate_chars(parts.next()?, SUBJECT_MAX);
            if !is_object_id(&sha) {
                return None;
            }
            Some(LogRow {
                sha,
                parents,
                author,
                authored_at,
                refs,
                subject,
            })
        })
        .collect()
}

/// SHA-1 ou SHA-256 completo em hex minúsculo — o que o git emite e o único
/// formato que o cursor aceita de volta.
pub fn is_object_id(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn looks_like_hash_prefix(s: &str) -> bool {
    (4..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn invalid(detail: impl Into<String>) -> String {
    format!("histórico: {}", detail.into())
}

/// `git <args> --stdin` com as revisões no stdin: centenas de branches não
/// estouram argv, e nenhum sha vira flag.
pub(super) fn run_git_stdin(mut cmd: Command, input: &[u8], what: &str) -> Result<Vec<u8>, String> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("{what}: {e}"))?;
    let mut stdin = child.stdin.take().ok_or_else(|| format!("{what}: stdin"))?;
    // Escreve numa thread: o git começa a cuspir saída antes de ler tudo, e
    // com pipe cheio dos dois lados ninguém anda.
    let input = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let out = child
        .wait_with_output()
        .map_err(|e| format!("{what}: {e}"))?;
    let _ = writer.join();
    if !out.status.success() {
        return Err(format!(
            "{what}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

const LOG_FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%aI%x1f%D%x1f%s";

fn base_log(repo: &Path, limit: usize) -> Command {
    let mut c = git_in(repo);
    c.args([
        "-c",
        "log.showSignature=false",
        "log",
        "-z",
        "--no-color",
        "--topo-order",
        "--decorate=full",
        LOG_FORMAT,
    ]);
    c.arg(format!("--max-count={limit}"));
    c
}

/// Shas das pontas de partida, resolvidos UMA vez: a página 1 e as seguintes
/// andam sobre o mesmo retrato, mesmo que uma branch mude no meio.
fn resolve_tips(repo: &Path, filter: &LogFilter) -> Result<Vec<String>, String> {
    if filter.all_branches {
        let raw = run_git(
            {
                let mut c = git_in(repo);
                c.args([
                    "for-each-ref",
                    "--format=%(objecttype) %(objectname) %(*objecttype) %(*objectname)",
                    "refs/heads",
                    "refs/remotes",
                    "refs/tags",
                ]);
                c
            },
            "git for-each-ref",
        )?;
        let mut tips = Vec::new();
        let mut seen = HashSet::new();
        if let Ok(head) = resolve_commit(repo, "HEAD") {
            seen.insert(head.clone());
            tips.push(head);
        }
        for sha in parse_ref_tips(&String::from_utf8_lossy(&raw)) {
            if seen.insert(sha.clone()) {
                tips.push(sha);
            }
        }
        return Ok(tips);
    }
    let name = non_empty(&filter.ref_name).unwrap_or("HEAD");
    super::branches::validate_ref_name(name)?;
    Ok(vec![resolve_commit(repo, name)?])
}

/// Linhas `tipo sha tipo-descascado sha-descascado` do for-each-ref: tag
/// anotada vale pelo commit para onde aponta; tag de árvore/blob fica fora.
pub fn parse_ref_tips(raw: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(' ').collect();
            match f.as_slice() {
                ["commit", sha, ..] => Some(sha.to_string()),
                ["tag", _, "commit", peeled] => Some(peeled.to_string()),
                _ => None,
            }
        })
        .filter(|s| is_object_id(s))
        .collect()
}

pub fn resolve_commit(repo: &Path, rev: &str) -> Result<String, String> {
    let out = run_git(
        {
            let mut c = git_in(repo);
            c.args(["rev-parse", "--verify", "--quiet", "--end-of-options"]);
            c.arg(format!("{rev}^{{commit}}"));
            c
        },
        "git rev-parse",
    )
    .map_err(|_| invalid(format!("referência não encontrada: {rev}")))?;
    let sha = String::from_utf8_lossy(&out).trim().to_string();
    if is_object_id(&sha) {
        Ok(sha)
    } else {
        Err(invalid(format!("referência não encontrada: {rev}")))
    }
}

fn validate_cursor(cursor: &LogCursor) -> Result<(), String> {
    if cursor
        .frontier
        .iter()
        .map(String::as_str)
        .chain(cursor.tips.iter().map(String::as_str))
        .chain(cursor.graph.pending())
        .all(is_object_id)
    {
        Ok(())
    } else {
        Err(invalid("cursor inválido"))
    }
}

/// Próxima fronteira: (fronteira ∪ pais do que saiu) − o que saiu. Ordem
/// estável (a de chegada) para o próximo `log` ser determinístico.
pub fn advance_cursor(cursor: &LogCursor, rows: &[LogRow]) -> LogCursor {
    let shown: HashSet<&str> = rows.iter().map(|r| r.sha.as_str()).collect();
    let mut seen = HashSet::new();
    let mut frontier = Vec::new();
    let candidates = cursor
        .frontier
        .iter()
        .chain(rows.iter().flat_map(|r| r.parents.iter()));
    for sha in candidates {
        if !shown.contains(sha.as_str()) && seen.insert(sha.as_str()) {
            frontier.push(sha.clone());
        }
    }
    let tips = cursor
        .tips
        .iter()
        .filter(|t| !shown.contains(t.as_str()) && !seen.contains(t.as_str()))
        .cloned()
        .collect();
    LogCursor {
        frontier,
        tips,
        skip: 0,
        graph: GraphState::default(),
    }
}

/// Uma página do histórico. `cursor: None` = primeira página.
pub fn log_page(
    repo: &Path,
    filter: &LogFilter,
    cursor: Option<&LogCursor>,
    limit: usize,
) -> Result<LogPage, String> {
    let limit = limit.clamp(1, PAGE_MAX);
    if super::diff::head_is_unborn(repo) && !filter.all_branches {
        return Ok(LogPage {
            rows: Vec::new(),
            cursor: None,
            filtered: filter.is_filtered(),
            graph: Vec::new(),
        });
    }
    if filter.is_filtered() {
        return filtered_page(repo, filter, cursor.map_or(0, |c| c.skip), limit);
    }

    let start = match cursor {
        Some(c) => {
            validate_cursor(c)?;
            c.clone()
        }
        None => LogCursor {
            tips: resolve_tips(repo, filter)?,
            ..Default::default()
        },
    };
    let mut revs = String::new();
    for sha in start.tips.iter().chain(start.frontier.iter()) {
        revs.push_str(sha);
        revs.push('\n');
    }
    if revs.is_empty() {
        return Ok(LogPage {
            rows: Vec::new(),
            cursor: None,
            filtered: false,
            graph: Vec::new(),
        });
    }
    // Pede um a mais: se vier, existe próxima página — sem `rev-list --count`.
    let mut cmd = base_log(repo, limit + 1);
    cmd.arg("--stdin");
    let raw = run_git_stdin(cmd, revs.as_bytes(), "git log")?;
    let mut rows = parse_log_rows(&raw);
    let more = rows.len() > limit;
    rows.truncate(limit);
    let mut lanes = start.graph.clone();
    let graph = rows
        .iter()
        .map(|r| {
            let agent = r.refs.iter().any(|x| x.kind == RefKind::Agent);
            lanes.row(&r.sha, &r.parents, agent)
        })
        .collect();
    let cursor = more.then(|| LogCursor {
        graph: lanes,
        ..advance_cursor(&start, &rows)
    });
    Ok(LogPage {
        rows,
        cursor,
        filtered: false,
        graph,
    })
}

fn filtered_page(
    repo: &Path,
    filter: &LogFilter,
    skip: usize,
    limit: usize,
) -> Result<LogPage, String> {
    let query = non_empty(&filter.query);
    // Hash colado na busca: o commit em si, se existir.
    if let Some(q) = query.filter(|q| looks_like_hash_prefix(q)) {
        if let Ok(sha) = resolve_commit(repo, &q.to_ascii_lowercase()) {
            if skip > 0 {
                return Ok(LogPage {
                    rows: Vec::new(),
                    cursor: None,
                    filtered: true,
                    graph: Vec::new(),
                });
            }
            let mut c = base_log(repo, 1);
            c.args(["--no-walk", "--end-of-options", &sha]);
            return Ok(LogPage {
                rows: parse_log_rows(&run_git(c, "git log")?),
                cursor: None,
                filtered: true,
                graph: Vec::new(),
            });
        }
    }
    let mut c = base_log(repo, limit + 1);
    c.arg(format!("--skip={skip}"));
    c.args(["-i", "--fixed-strings"]);
    if let Some(author) = non_empty(&filter.author) {
        c.arg(format!("--author={author}"));
    }
    if let Some(q) = query {
        c.arg(format!("--grep={q}"));
    }
    if filter.all_branches {
        c.args(["HEAD", "--branches", "--remotes", "--tags"]);
    } else {
        let name = non_empty(&filter.ref_name).unwrap_or("HEAD");
        super::branches::validate_ref_name(name)?;
        c.args(["--end-of-options", name]);
    }
    if let Some(path) = non_empty(&filter.path) {
        super::ops::ensure_relative(path)?;
        c.arg("--").arg(format!(":(literal){path}"));
    }
    let mut rows = parse_log_rows(&run_git(c, "git log")?);
    let more = rows.len() > limit;
    rows.truncate(limit);
    let cursor = more.then(|| LogCursor {
        skip: skip + rows.len(),
        ..Default::default()
    });
    Ok(LogPage {
        rows,
        cursor,
        filtered: true,
        graph: Vec::new(),
    })
}

// --- tamanho do histórico e commit-graph -----------------------------------

/// Acima disto, sem commit-graph, a aba sugere (sem insistir) gravá-lo.
pub const COMMIT_GRAPH_SUGGEST_ABOVE: u64 = 5000;

#[derive(Debug, Clone, Serialize)]
pub struct LogStats {
    /// Commits alcançáveis da partida atual; `None` com filtro.
    pub total: Option<u64>,
    pub has_commit_graph: bool,
    /// Repo grande sem commit-graph: a UI mostra o convite discreto.
    pub suggest_commit_graph: bool,
}

/// `objects/info/commit-graph` (arquivo único) ou a cadeia incremental
/// (`commit-graphs/commit-graph-chain`), no git dir COMUM — worktree de
/// agente usa o do repo principal. Em dúvida, diz que tem: não sugerir
/// escrita no repo por engano vale mais que a sugestão.
pub fn has_commit_graph(repo: &Path) -> bool {
    let Ok(dirs) = super::resolved_git_dirs(repo) else {
        return true;
    };
    let info = dirs.common_dir.join("objects").join("info");
    info.join("commit-graph").is_file()
        || info
            .join("commit-graphs")
            .join("commit-graph-chain")
            .is_file()
}

/// Separado da página de propósito: o `rev-list --count` anda o histórico
/// inteiro, e a primeira página não pode esperar por isso.
pub fn log_stats(repo: &Path, filter: &LogFilter) -> Result<LogStats, String> {
    let has = has_commit_graph(repo);
    let total = if filter.is_filtered() || super::diff::head_is_unborn(repo) {
        None
    } else {
        let mut c = git_in(repo);
        c.args(["rev-list", "--count"]);
        if filter.all_branches {
            c.args(["HEAD", "--branches", "--remotes", "--tags"]);
        } else {
            let name = non_empty(&filter.ref_name).unwrap_or("HEAD");
            super::branches::validate_ref_name(name)?;
            c.args(["--end-of-options", name]);
        }
        let out = run_git(c, "git rev-list --count")?;
        String::from_utf8_lossy(&out).trim().parse().ok()
    };
    Ok(LogStats {
        total,
        has_commit_graph: has,
        suggest_commit_graph: !has && total.is_some_and(|n| n > COMMIT_GRAPH_SUGGEST_ABOVE),
    })
}

/// `git commit-graph write --reachable`: ESCREVE no repo (um índice em
/// `objects/info`, sem tocar commit nem ref) — por isso jaula rw, e só por
/// clique explícito na UI.
pub fn write_commit_graph(repo: &Path) -> Result<(), String> {
    let mut c = git_in_rw(repo);
    c.args(["commit-graph", "write", "--reachable"]);
    run_git(c, "git commit-graph write")?;
    Ok(())
}

// --- detalhe de um commit ---------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CommitDetail {
    pub sha: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
    pub committer_name: String,
    pub committed_at: String,
    pub refs: Vec<RefLabel>,
    pub subject: String,
    pub body: String,
    /// Contra o primeiro pai (o que o commit trouxe para a linha dele).
    pub files: Vec<FileDiff>,
    pub files_truncated: usize,
}

/// `show -s -z --format=%H%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%cn%x1f%cI%x1f%D%x1f%B`.
/// `%B` é o último campo e leva o resto (pode ter qualquer coisa menos NUL).
pub fn parse_commit_meta(raw: &[u8]) -> Option<CommitDetail> {
    let record = raw.split(|b| *b == 0).find(|r| !r.is_empty())?;
    let text = String::from_utf8_lossy(record);
    let mut parts = text.splitn(9, '\x1f');
    let sha = parts.next()?.trim().to_string();
    if !is_object_id(&sha) {
        return None;
    }
    let parents = parts
        .next()?
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let author_name = parts.next()?.to_string();
    let author_email = parts.next()?.to_string();
    let authored_at = parts.next()?.to_string();
    let committer_name = parts.next()?.to_string();
    let committed_at = parts.next()?.to_string();
    let refs = parse_decorations(parts.next()?);
    let message = parts.next()?.trim_end();
    let (subject, body) = match message.split_once('\n') {
        Some((s, b)) => (s.to_string(), b.trim_start_matches('\n').to_string()),
        None => (message.to_string(), String::new()),
    };
    Some(CommitDetail {
        sha,
        parents,
        author_name,
        author_email,
        authored_at,
        committer_name,
        committed_at,
        refs,
        subject: truncate_chars(&subject, SUBJECT_MAX * 4),
        body: truncate_chars(&body, BODY_MAX),
        files: Vec::new(),
        files_truncated: 0,
    })
}

/// Árvore vazia do formato do repo (SHA-1 ou SHA-256) — o "pai" do commit
/// raiz. `hash-object` sem `-w` não escreve nada.
fn empty_tree(repo: &Path) -> Result<String, String> {
    let out = run_git(
        {
            let mut c = git_in(repo);
            c.args(["hash-object", "-t", "tree", "--stdin"]);
            c
        },
        "git hash-object",
    )?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

/// Range `<primeiro pai>..<sha>` (ou árvore vazia para o raiz) de um commit
/// já validado — base do detalhe e dos hunks.
pub fn commit_range(repo: &Path, sha: &str) -> Result<String, String> {
    let sha = resolve_commit(repo, sha)?;
    let parent = run_git(
        {
            let mut c = git_in(repo);
            c.args(["rev-list", "--parents", "-n", "1", &sha]);
            c
        },
        "git rev-list --parents",
    )?;
    let parent = String::from_utf8_lossy(&parent)
        .split_whitespace()
        .nth(1)
        .map(str::to_string);
    let base = match parent {
        Some(p) => p,
        None => empty_tree(repo)?,
    };
    Ok(format!("{base}..{sha}"))
}

fn validate_sha_arg(sha: &str) -> Result<(), String> {
    if looks_like_hash_prefix(sha) {
        Ok(())
    } else {
        Err(invalid(format!("hash inválido: {sha}")))
    }
}

pub fn commit_detail(repo: &Path, sha: &str) -> Result<CommitDetail, String> {
    validate_sha_arg(sha)?;
    let sha = resolve_commit(repo, sha)?;
    let raw = run_git(
        {
            let mut c = git_in(repo);
            c.args([
                "-c",
                "log.showSignature=false",
                "show",
                "-s",
                "-z",
                "--no-color",
                "--decorate=full",
                "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%cn%x1f%cI%x1f%D%x1f%B",
                "--end-of-options",
                &sha,
            ]);
            c
        },
        "git show",
    )?;
    let mut detail = parse_commit_meta(&raw).ok_or_else(|| invalid("commit ilegível"))?;
    let range = commit_range(repo, &sha)?;
    let mut files = super::diff::diff_files(repo, &[&range])?;
    detail.files_truncated = files.len().saturating_sub(DETAIL_FILES_MAX);
    files.truncate(DETAIL_FILES_MAX);
    detail.files = files;
    Ok(detail)
}

pub fn commit_hunks(
    repo: &Path,
    sha: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<super::diff::FileHunks, String> {
    validate_sha_arg(sha)?;
    super::ops::ensure_relative(path)?;
    if let Some(old) = old_path {
        super::ops::ensure_relative(old)?;
    }
    let range = commit_range(repo, sha)?;
    super::diff::range_file_hunks(repo, &range, path, old_path)
}

// --- ações sobre um commit (escrevem no repo: jaula rw) ---------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitAction {
    /// `checkout --detach <sha>` (HEAD destacado; sem branch nova).
    Checkout,
    CherryPick,
    Revert,
}

fn history_failed(detail: impl Into<String>) -> AppError {
    AppError::new("history.failed").with("detail", detail.into())
}

/// Checkout/cherry-pick/revert de um commit. Árvore suja só passa com
/// `allow_dirty` — a UI pede confirmação antes; o próprio git ainda recusa o
/// que sobrescreveria alteração local (sem `-f`, sem stash). Commit de merge
/// fica de fora do cherry-pick/revert: escolher o `-m` é decisão de terminal.
/// Nunca há rede aqui: nada de push.
pub fn apply_commit_action(
    repo: &Path,
    sha: &str,
    action: CommitAction,
    allow_dirty: bool,
) -> Result<(), AppError> {
    validate_sha_arg(sha).map_err(history_failed)?;
    let sha = resolve_commit(repo, sha).map_err(history_failed)?;
    if !allow_dirty && super::is_dirty(repo).map_err(history_failed)? {
        return Err(AppError::new("history.dirty"));
    }
    if action != CommitAction::Checkout {
        let parents = run_git(
            {
                let mut c = git_in(repo);
                c.args(["rev-list", "--parents", "-n", "1", &sha]);
                c
            },
            "git rev-list --parents",
        )
        .map_err(history_failed)?;
        if String::from_utf8_lossy(&parents).split_whitespace().count() > 2 {
            return Err(AppError::new("history.merge_commit"));
        }
    }
    let mut c = git_in_rw(repo);
    match action {
        CommitAction::Checkout => c.args(["checkout", "--detach", &sha]),
        CommitAction::CherryPick => c.args(["cherry-pick", &sha]),
        CommitAction::Revert => c.args(["revert", "--no-edit", &sha]),
    };
    let out = c
        .output()
        .map_err(|e| history_failed(format!("git: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // Conflito deixa o cherry-pick/revert em andamento: o painel de
        // conflitos (que já detecta os dois) assume daqui.
        if super::conflicts::session_conflicts(repo)
            .ok()
            .flatten()
            .is_some()
        {
            return Err(AppError::new("history.conflict"));
        }
        return Err(history_failed(stderr));
    }
    Ok(())
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const C: &str = "cccccccccccccccccccccccccccccccccccccccc";

    #[test]
    fn decorations_classify_head_branches_remotes_tags_and_agent() {
        let refs = parse_decorations(
            "HEAD -> refs/heads/main, refs/remotes/origin/main, refs/remotes/origin/HEAD, \
             tag: refs/tags/v0.8.1, refs/heads/tyba/fix-health-x7k, refs/heads/origin/falsa, refs/stash",
        );
        let got: Vec<(&str, RefKind, bool)> = refs
            .iter()
            .map(|r| (r.name.as_str(), r.kind, r.is_head))
            .collect();
        assert_eq!(
            got,
            vec![
                ("main", RefKind::Branch, true),
                ("origin/main", RefKind::Remote, false),
                ("v0.8.1", RefKind::Tag, false),
                ("tyba/fix-health-x7k", RefKind::Agent, false),
                ("origin/falsa", RefKind::Branch, false),
            ]
        );
    }

    #[test]
    fn detached_head_is_its_own_chip() {
        let refs = parse_decorations("HEAD, refs/heads/x");
        assert_eq!(refs[0].kind, RefKind::Head);
        assert_eq!(refs[1].name, "x");
    }

    #[test]
    fn log_rows_parse_parents_refs_and_subject_with_separator() {
        let raw = format!(
            "{A}\x1f{B} {C}\x1fAna Ribeiro\x1f2026-10-12T14:08:00-03:00\x1fHEAD -> refs/heads/main\x1fMerge branch 'x'\0\
             \n{B}\x1f{C}\x1fMarcos\x1f2026-10-11T10:00:00-03:00\x1f\x1ffix: a \x1f b\0\
             {C}\x1f\x1fGT\x1f2026-10-10T10:00:00-03:00\x1f\x1finicial\0"
        );
        let rows = parse_log_rows(raw.as_bytes());
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].parents, vec![B.to_string(), C.to_string()]);
        assert_eq!(rows[0].refs[0].name, "main");
        assert!(rows[0].refs[0].is_head);
        assert_eq!(rows[1].sha, B, "newline entre registros não suja o sha");
        assert_eq!(rows[1].subject, "fix: a \x1f b");
        assert!(rows[1].refs.is_empty());
        assert!(rows[2].parents.is_empty(), "commit raiz");
    }

    #[test]
    fn log_rows_cap_huge_subjects_and_drop_garbage() {
        let long = "x".repeat(SUBJECT_MAX + 50);
        let raw = format!("{A}\x1f\x1fa\x1fd\x1f\x1f{long}\0nao-sha\x1f\x1fa\x1fd\x1f\x1fs\0");
        let rows = parse_log_rows(raw.as_bytes());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subject.chars().count(), SUBJECT_MAX + 1);
        assert!(rows[0].subject.ends_with('…'));
    }

    #[test]
    fn ref_tips_peel_annotated_tags_and_skip_non_commits() {
        let raw = format!("commit {A}  \ntag {B} commit {C}\ntag {B} tree {A}\nblob {A}  \n");
        assert_eq!(parse_ref_tips(&raw), vec![A.to_string(), C.to_string()]);
    }

    #[test]
    fn cursor_advances_frontier_and_consumes_tips() {
        let row = |sha: &str, parents: &[&str]| LogRow {
            sha: sha.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: String::new(),
            authored_at: String::new(),
            refs: Vec::new(),
            subject: String::new(),
        };
        let start = LogCursor {
            frontier: vec![],
            tips: vec![A.into(), C.into()],
            ..Default::default()
        };
        // A -> B; C ainda não saiu, mas é pai de B (vira fronteira, não ponta).
        let next = advance_cursor(&start, &[row(A, &[B]), row(B, &[C])]);
        assert_eq!(next.frontier, vec![C.to_string()]);
        assert!(next.tips.is_empty(), "C já está na fronteira");
    }

    #[test]
    fn cursor_rejects_anything_that_is_not_a_full_sha() {
        let bad = LogCursor {
            frontier: vec!["--output=/tmp/x".into()],
            tips: vec![],
            ..Default::default()
        };
        assert!(validate_cursor(&bad).is_err());
        assert!(validate_cursor(&LogCursor {
            frontier: vec![A.into()],
            ..Default::default()
        })
        .is_ok());
    }

    #[test]
    fn commit_meta_splits_subject_and_body() {
        let raw = format!(
            "{A}\x1f{B}\x1fAna\x1fana@x.com\x1f2026-10-12T14:08:00-03:00\x1fGT\x1f2026-10-12T14:09:00-03:00\x1frefs/heads/feat/x\x1ffeat: titulo\n\ncorpo com \x1f separador\nsegunda linha\n\0"
        );
        let d = parse_commit_meta(raw.as_bytes()).unwrap();
        assert_eq!(d.parents, vec![B.to_string()]);
        assert_eq!(d.author_email, "ana@x.com");
        assert_eq!(d.subject, "feat: titulo");
        assert_eq!(d.body, "corpo com \x1f separador\nsegunda linha");
        assert_eq!(d.refs[0].name, "feat/x");
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::PathBuf;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn commit(dir: &Path, file: &str, msg: &str) -> String {
        std::fs::write(dir.join(file), format!("{msg}\n")).unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-qm", msg]);
        git(dir, &["rev-parse", "HEAD"])
    }

    fn temp_repo() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-log-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "t@t.com"]);
        git(&dir, &["config", "user.name", "Teste"]);
        git(&dir, &["config", "commit.gpgsign", "false"]);
        dir
    }

    /// main: 0..4, feat/x sai de 1 com 2 commits e volta por merge, tyba/ag
    /// (agente) sai de 4 com 1 commit não mergeado.
    fn branchy_repo() -> PathBuf {
        let dir = temp_repo();
        commit(&dir, "a.txt", "c0");
        commit(&dir, "a.txt", "c1");
        git(&dir, &["checkout", "-qb", "feat/x"]);
        commit(&dir, "x.txt", "x1");
        commit(&dir, "x.txt", "x2");
        git(&dir, &["checkout", "-q", "main"]);
        commit(&dir, "a.txt", "c2");
        git(&dir, &["merge", "-q", "--no-ff", "-m", "merge x", "feat/x"]);
        commit(&dir, "a.txt", "c4");
        git(&dir, &["checkout", "-qb", "tyba/ag"]);
        commit(&dir, "ag.txt", "agente");
        git(&dir, &["checkout", "-q", "main"]);
        git(&dir, &["tag", "-a", "v1", "-m", "v1"]);
        dir
    }

    fn all_pages(repo: &Path, filter: &LogFilter, limit: usize) -> Vec<LogRow> {
        let mut out = Vec::new();
        let mut cursor = None;
        loop {
            let page = log_page(repo, filter, cursor.as_ref(), limit).unwrap();
            out.extend(page.rows);
            match page.cursor {
                Some(c) => cursor = Some(c),
                None => return out,
            }
        }
    }

    fn reference(repo: &Path, all: bool) -> Vec<String> {
        let mut args = vec!["rev-list", "--topo-order"];
        if all {
            args.extend(["HEAD", "--branches", "--remotes", "--tags"]);
        } else {
            args.push("HEAD");
        }
        git(repo, &args).lines().map(str::to_string).collect()
    }

    fn assert_topological(rows: &[LogRow]) {
        let pos: std::collections::HashMap<&str, usize> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.sha.as_str(), i))
            .collect();
        assert_eq!(pos.len(), rows.len(), "commit repetido entre páginas");
        for (i, r) in rows.iter().enumerate() {
            for p in &r.parents {
                if let Some(j) = pos.get(p.as_str()) {
                    assert!(*j > i, "pai antes do filho: {} -> {}", r.subject, p);
                }
            }
        }
    }

    #[test]
    fn paging_with_every_page_size_covers_the_whole_history_exactly_once() {
        let repo = branchy_repo();
        for all in [false, true] {
            let filter = LogFilter {
                all_branches: all,
                ..Default::default()
            };
            let mut expected = reference(&repo, all);
            expected.sort();
            for limit in 1..=10 {
                let rows = all_pages(&repo, &filter, limit);
                assert_topological(&rows);
                let mut got: Vec<String> = rows.iter().map(|r| r.sha.clone()).collect();
                got.sort();
                assert_eq!(got, expected, "all={all} limit={limit}");
            }
        }
        std::fs::remove_dir_all(&repo).ok();
    }

    /// A continuação de uma página é uma ordem topológica válida, mas não
    /// necessariamente a mesma de um `log` único (o git intercala as pontas
    /// de outro jeito). O que importa: as lanes calculadas página a página são
    /// as mesmas de calcular tudo de uma vez sobre as MESMAS linhas.
    #[test]
    fn graph_lanes_survive_every_page_boundary() {
        let repo = branchy_repo();
        let filter = LogFilter {
            all_branches: true,
            ..Default::default()
        };
        let whole = log_page(&repo, &filter, None, 200).unwrap();
        assert_eq!(whole.graph.len(), whole.rows.len());
        for limit in 1..=6 {
            let mut rows = Vec::new();
            let mut graph = Vec::new();
            let mut cursor = None;
            loop {
                let page = log_page(&repo, &filter, cursor.as_ref(), limit).unwrap();
                assert_eq!(page.graph.len(), page.rows.len());
                rows.extend(page.rows);
                graph.extend(page.graph);
                match page.cursor {
                    Some(c) => {
                        let pending: HashSet<&str> = c.graph.pending().collect();
                        let frontier: HashSet<&str> =
                            c.frontier.iter().map(String::as_str).collect();
                        assert_eq!(pending, frontier, "lanes e fronteira divergiram");
                        cursor = Some(c)
                    }
                    None => break,
                }
            }
            let mut fresh = GraphState::default();
            let expected: Vec<GraphRow> = rows
                .iter()
                .map(|r| {
                    let agent = r.refs.iter().any(|x| x.kind == RefKind::Agent);
                    fresh.row(&r.sha, &r.parents, agent)
                })
                .collect();
            assert_eq!(graph, expected, "limit={limit}");
            assert!(fresh.lanes.is_empty(), "nada pendente no fim do histórico");
        }
        let agent = whole
            .rows
            .iter()
            .position(|r| r.subject == "agente")
            .unwrap();
        assert_eq!(whole.graph[agent].color, super::super::graph::AGENT_COLOR);
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn filtered_page_has_no_graph() {
        let repo = branchy_repo();
        let page = log_page(
            &repo,
            &LogFilter {
                author: Some("Teste".into()),
                ..Default::default()
            },
            None,
            200,
        )
        .unwrap();
        assert!(page.filtered && !page.rows.is_empty() && page.graph.is_empty());
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn first_page_carries_refs_and_agent_branch_only_with_all_branches() {
        let repo = branchy_repo();
        let head_only = log_page(&repo, &LogFilter::default(), None, 200).unwrap();
        assert!(head_only.cursor.is_none());
        assert!(!head_only.rows.iter().any(|r| r.subject == "agente"));
        let top = &head_only.rows[0];
        assert!(top.refs.iter().any(|r| r.name == "main" && r.is_head));
        assert!(top
            .refs
            .iter()
            .any(|r| r.kind == RefKind::Tag && r.name == "v1"));

        let all = log_page(
            &repo,
            &LogFilter {
                all_branches: true,
                ..Default::default()
            },
            None,
            200,
        )
        .unwrap();
        let agent = all.rows.iter().find(|r| r.subject == "agente").unwrap();
        assert_eq!(agent.refs[0].kind, RefKind::Agent);
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn ref_filter_starts_from_that_branch() {
        let repo = branchy_repo();
        let page = log_page(
            &repo,
            &LogFilter {
                ref_name: Some("feat/x".into()),
                ..Default::default()
            },
            None,
            200,
        )
        .unwrap();
        let subjects: Vec<&str> = page.rows.iter().map(|r| r.subject.as_str()).collect();
        assert_eq!(subjects, vec!["x2", "x1", "c1", "c0"]);
        assert!(log_page(
            &repo,
            &LogFilter {
                ref_name: Some("--output=/tmp/pwn".into()),
                ..Default::default()
            },
            None,
            200
        )
        .is_err());
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn filters_page_with_skip_and_match_author_path_and_hash() {
        let repo = branchy_repo();
        let by_path = all_pages(
            &repo,
            &LogFilter {
                path: Some("x.txt".into()),
                ..Default::default()
            },
            1,
        );
        let subjects: Vec<&str> = by_path.iter().map(|r| r.subject.as_str()).collect();
        assert_eq!(subjects, vec!["x2", "x1"]);

        let by_msg = log_page(
            &repo,
            &LogFilter {
                query: Some("MERGE".into()),
                ..Default::default()
            },
            None,
            200,
        )
        .unwrap();
        assert!(by_msg.filtered);
        assert_eq!(by_msg.rows.len(), 1);

        let none = log_page(
            &repo,
            &LogFilter {
                author: Some("ninguém".into()),
                ..Default::default()
            },
            None,
            200,
        )
        .unwrap();
        assert!(none.rows.is_empty());

        let sha = git(&repo, &["rev-parse", "feat/x"]);
        let by_hash = log_page(
            &repo,
            &LogFilter {
                query: Some(sha[..8].to_string()),
                ..Default::default()
            },
            None,
            200,
        )
        .unwrap();
        assert_eq!(by_hash.rows.len(), 1);
        assert_eq!(by_hash.rows[0].sha, sha);
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn stats_count_history_and_commit_graph_write_is_detected() {
        let repo = branchy_repo();
        let stats = log_stats(&repo, &LogFilter::default()).unwrap();
        assert_eq!(stats.total, Some(7));
        let all = log_stats(
            &repo,
            &LogFilter {
                all_branches: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(all.total, Some(8), "+ o commit do agente");
        assert!(!stats.has_commit_graph);
        assert!(!stats.suggest_commit_graph, "repo pequeno não sugere nada");

        write_commit_graph(&repo).unwrap();
        assert!(has_commit_graph(&repo));
        let filtered = log_stats(
            &repo,
            &LogFilter {
                author: Some("x".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered.total, None);
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn unborn_repo_has_an_empty_history() {
        let repo = temp_repo();
        let page = log_page(&repo, &LogFilter::default(), None, 200).unwrap();
        assert!(page.rows.is_empty() && page.cursor.is_none());
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn detail_lists_files_against_first_parent_and_hunks_for_root() {
        let repo = branchy_repo();
        let merge = git(&repo, &["rev-parse", "main~1"]);
        let d = commit_detail(&repo, &merge).unwrap();
        assert_eq!(d.subject, "merge x");
        assert_eq!(d.parents.len(), 2);
        let paths: Vec<&str> = d.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["x.txt"], "merge mostra o que entrou na linha");

        let root = git(&repo, &["rev-list", "--max-parents=0", "HEAD"]);
        let d = commit_detail(&repo, &root).unwrap();
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].insertions, Some(1));
        let hunks = commit_hunks(&repo, &root, "a.txt", None).unwrap();
        assert_eq!(hunks.hunks.len(), 1);
        assert!(commit_detail(&repo, "--help").is_err());
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn cherry_pick_and_revert_create_commits_and_dirty_needs_consent() {
        let repo = branchy_repo();
        let agent = git(&repo, &["rev-parse", "tyba/ag"]);
        std::fs::write(repo.join("sujo.txt"), "x").unwrap();
        let err = apply_commit_action(&repo, &agent, CommitAction::CherryPick, false).unwrap_err();
        assert_eq!(err.code, "history.dirty");
        apply_commit_action(&repo, &agent, CommitAction::CherryPick, true).unwrap();
        assert_eq!(git(&repo, &["log", "-1", "--format=%s"]), "agente");
        assert!(repo.join("sujo.txt").exists(), "nada de stash/clean");

        let head = git(&repo, &["rev-parse", "HEAD"]);
        apply_commit_action(&repo, &head, CommitAction::Revert, true).unwrap();
        assert!(git(&repo, &["log", "-1", "--format=%s"]).starts_with("Revert"));

        let merge = git(&repo, &["rev-parse", "v1~1"]);
        let err = apply_commit_action(&repo, &merge, CommitAction::Revert, true).unwrap_err();
        assert_eq!(err.code, "history.merge_commit");

        std::fs::remove_file(repo.join("sujo.txt")).unwrap();
        let c1 = git(&repo, &["rev-parse", "v1~3"]);
        apply_commit_action(&repo, &c1, CommitAction::Checkout, false).unwrap();
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), c1);
        std::fs::remove_dir_all(&repo).ok();
    }
}

/// Medição, não teste: `cargo test --release --lib -- --ignored bench_log
/// --nocapture`. Gera um repo sintético (50k commits, uma branch de 3 commits
/// mergeada a cada 50) via `fast-import` e, se `TYBA_LOG_BENCH_REPO` apontar
/// um repo real, mede ele também — só leitura: o commit-graph é gravado
/// apenas no sintético.
#[cfg(test)]
mod bench {
    use super::*;
    use std::time::Instant;

    fn commit(s: &mut String, mark: usize, branch: &str, from: &[usize], i: usize) {
        let when = 1_600_000_000 + i * 60;
        s.push_str(&format!(
            "commit refs/heads/{branch}\nmark :{mark}\nauthor Bench <b@b> {when} -0300\n\
             committer Bench <b@b> {when} -0300\ndata 12\nfeat: c{:05}\n",
            i % 100_000
        ));
        for (k, f) in from.iter().enumerate() {
            s.push_str(if k == 0 { "from" } else { "merge" });
            s.push_str(&format!(" :{f}\n"));
        }
        s.push_str(&format!(
            "M 644 inline f{}.txt\ndata 6\nl{:04}\n\n",
            i % 7,
            i % 10_000
        ));
    }

    fn synthetic(n: usize) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-log-bench-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            assert!(Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .status()
                .unwrap()
                .success());
        };
        run(&["init", "-q", "-b", "main"]);
        let mut stream = String::new();
        let (mut mark, mut main_tip, mut i) = (0usize, 0usize, 0usize);
        while i < n {
            mark += 1;
            let from: Vec<usize> = if main_tip == 0 {
                vec![]
            } else {
                vec![main_tip]
            };
            commit(&mut stream, mark, "main", &from, i);
            main_tip = mark;
            i += 1;
            if i % 50 == 0 && i + 4 < n {
                let mut side = main_tip;
                for _ in 0..3 {
                    mark += 1;
                    commit(&mut stream, mark, "side", &[side], i);
                    side = mark;
                    i += 1;
                }
                mark += 1;
                commit(&mut stream, mark, "main", &[main_tip, side], i);
                main_tip = mark;
                i += 1;
            }
        }
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["fast-import", "--quiet"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stream.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        run(&["reset", "-q", "--hard", "main"]);
        dir
    }

    fn measure(label: &str, repo: &Path) {
        for all in [false, true] {
            let filter = LogFilter {
                all_branches: all,
                ..Default::default()
            };
            let t = Instant::now();
            let first = log_page(repo, &filter, None, PAGE_DEFAULT).unwrap();
            let first_ms = t.elapsed().as_secs_f64() * 1000.0;
            let mut cursor = first.cursor.clone();
            let mut next = Vec::new();
            for _ in 0..5 {
                let Some(c) = cursor.take() else { break };
                let t = Instant::now();
                let page = log_page(repo, &filter, Some(&c), PAGE_DEFAULT).unwrap();
                next.push(format!("{:.1}", t.elapsed().as_secs_f64() * 1000.0));
                cursor = page.cursor;
            }
            let json = serde_json::to_vec(&first).unwrap().len();
            let t = Instant::now();
            let stats = log_stats(repo, &filter).unwrap();
            let stats_ms = t.elapsed().as_secs_f64() * 1000.0;
            let lanes = first.graph.iter().map(|g| g.width).max().unwrap_or(0);
            eprintln!(
                "BENCH {label} all={all}: 1a pagina 200 = {first_ms:.1} ms ({json} B, {lanes} colunas); \
                 seguintes = {next:?} ms; stats = {stats_ms:.1} ms (total {:?}, commit-graph {})",
                stats.total, stats.has_commit_graph
            );
        }
    }

    #[test]
    #[ignore]
    fn bench_log() {
        let repo = synthetic(50_000);
        measure("sintetico 50k sem commit-graph", &repo);
        write_commit_graph(&repo).unwrap();
        measure("sintetico 50k com commit-graph", &repo);
        std::fs::remove_dir_all(&repo).ok();
        if let Ok(real) = std::env::var("TYBA_LOG_BENCH_REPO") {
            for path in real.split(':') {
                measure(path, Path::new(path));
            }
        }
    }
}
