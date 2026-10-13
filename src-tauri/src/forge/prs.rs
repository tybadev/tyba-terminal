//! Lista rica e detalhe de PR (git completo, fatias 3–4: só leitura).
//!
//! Só GitHub por enquanto: para GitLab, sem forja ou sem `gh`, as consultas
//! devolvem `None` — a UI lê isso como "esta forja não tem lista rica", não
//! como "não há PR". Todo texto que vem daqui (título, corpo, comentário) é
//! entrada não confiável; quem renderiza trata como texto.
//!
//! O vínculo com sessão do TYBA (`session`) sai sempre `None` daqui: quem
//! conhece as sessões é o `lib.rs`, que preenche na saída — e por isso o cache
//! guarda a página sem ele (sessões nascem e morrem dentro dos 30s).

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::exec::{self, NETWORK_TIMEOUT};
use super::github::{self, cli_failed};
use super::remote::Remote;
use super::{cli_missing, remote_of, resolve_kind, ForgeKind, CACHE_TTL};
use crate::error::AppError;
use crate::session::SessionId;

const LIST_LIMIT: &str = "100";
const RECENT_MERGED_LIMIT: &str = "5";

const LIST_FIELDS: &str = "number,title,url,state,isDraft,author,headRefName,baseRefName,\
updatedAt,mergedAt,mergedBy,statusCheckRollup,reviewDecision,latestReviews,reviewRequests,\
comments,additions,deletions";

const DETAIL_FIELDS: &str = "number,title,url,state,isDraft,author,headRefName,baseRefName,\
headRefOid,createdAt,body,additions,deletions,changedFiles,commits,files,statusCheckRollup,reviews,\
reviewRequests,comments,reviewDecision,mergeable,mergeStateStatus";

// --- contrato (espelha `src/lib/ipc.ts`, seção "PRs ricos") -----------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrFilter {
    Open,
    Mine,
    Review,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PrCheckStatus {
    Success,
    Failure,
    Pending,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PrMergeable {
    Mergeable,
    Conflicting,
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PrCiSummary {
    pub passed: u32,
    pub failed: u32,
    pub pending: u32,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrSessionLink {
    pub session: SessionId,
    pub title: String,
    pub agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrListItem {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: PrState,
    pub author: String,
    pub author_name: Option<String>,
    pub head: String,
    pub base: String,
    pub updated_at: String,
    pub merged_at: Option<String>,
    pub merged_by: Option<String>,
    pub ci: PrCiSummary,
    pub review: Option<PrReviewDecision>,
    pub approvals: u32,
    pub review_requested: bool,
    pub comments: u32,
    pub additions: u64,
    pub deletions: u64,
    pub session: Option<PrSessionLink>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PrCounts {
    pub open: u32,
    pub mine: u32,
    pub review: u32,
    pub draft: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrPage {
    pub items: Vec<PrListItem>,
    pub recent_merged: Vec<PrListItem>,
    pub counts: PrCounts,
    pub repo: String,
    pub forge: ForgeKind,
    pub fetched_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrCommit {
    pub sha: String,
    pub subject: String,
    pub author: String,
    pub authored_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrCheck {
    pub name: String,
    pub workflow: Option<String>,
    pub status: PrCheckStatus,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrReview {
    pub author: String,
    pub state: PrReviewState,
    pub submitted_at: Option<String>,
    pub body: String,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrThreadComment {
    pub author: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrThread {
    pub path: String,
    pub line: Option<u32>,
    pub diff_hunk: String,
    pub comments: Vec<PrThreadComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrDetail {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: PrState,
    pub author: String,
    pub head: String,
    pub head_sha: String,
    pub base: String,
    pub created_at: String,
    pub body: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub commits: Vec<PrCommit>,
    pub files: Vec<PrFile>,
    pub checks: Vec<PrCheck>,
    pub reviews: Vec<PrReview>,
    pub pending_reviewers: Vec<String>,
    pub threads: Vec<PrThread>,
    pub comments: Vec<PrThreadComment>,
    pub review: Option<PrReviewDecision>,
    pub mergeable: PrMergeable,
    pub merge_state: String,
    pub session: Option<PrSessionLink>,
}

// --- JSON do `gh` ------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct GhActor {
    #[serde(default)]
    login: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum GhRollup {
    #[serde(rename_all = "camelCase")]
    CheckRun {
        #[serde(default)]
        name: String,
        #[serde(default)]
        status: String,
        #[serde(default)]
        conclusion: Option<String>,
        #[serde(default)]
        workflow_name: Option<String>,
        #[serde(default)]
        started_at: Option<String>,
        #[serde(default)]
        completed_at: Option<String>,
        #[serde(default)]
        details_url: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    StatusContext {
        #[serde(default)]
        context: String,
        #[serde(default)]
        state: String,
        #[serde(default)]
        started_at: Option<String>,
        #[serde(default)]
        target_url: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhReview {
    #[serde(default)]
    author: Option<GhActor>,
    #[serde(default)]
    state: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    submitted_at: Option<String>,
    #[serde(default)]
    commit: Option<GhCommitRef>,
}

#[derive(Debug, Deserialize)]
struct GhCommitRef {
    #[serde(default)]
    oid: String,
}

/// Usuário (`login`) ou time (`name`; o `gh` não traz `login` para time).
#[derive(Debug, Deserialize)]
struct GhReviewRequest {
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhIssueComment {
    #[serde(default)]
    author: Option<GhActor>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhListPr {
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    author: Option<GhActor>,
    #[serde(default)]
    head_ref_name: String,
    #[serde(default)]
    base_ref_name: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    merged_at: Option<String>,
    #[serde(default)]
    merged_by: Option<GhActor>,
    #[serde(default)]
    status_check_rollup: Option<Vec<GhRollup>>,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    latest_reviews: Option<Vec<GhReview>>,
    #[serde(default)]
    review_requests: Option<Vec<GhReviewRequest>>,
    #[serde(default)]
    comments: Option<Vec<GhIssueComment>>,
    #[serde(default)]
    additions: u64,
    #[serde(default)]
    deletions: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhCommit {
    #[serde(default)]
    oid: String,
    #[serde(default)]
    message_headline: String,
    #[serde(default)]
    authored_date: String,
    #[serde(default)]
    authors: Vec<GhActor>,
}

#[derive(Debug, Deserialize)]
struct GhFile {
    #[serde(default)]
    path: String,
    #[serde(default)]
    additions: u64,
    #[serde(default)]
    deletions: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhDetailPr {
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    author: Option<GhActor>,
    #[serde(default)]
    head_ref_name: String,
    #[serde(default)]
    base_ref_name: String,
    #[serde(default)]
    head_ref_oid: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    additions: u64,
    #[serde(default)]
    deletions: u64,
    #[serde(default)]
    changed_files: u64,
    #[serde(default)]
    commits: Option<Vec<GhCommit>>,
    #[serde(default)]
    files: Option<Vec<GhFile>>,
    #[serde(default)]
    status_check_rollup: Option<Vec<GhRollup>>,
    #[serde(default)]
    reviews: Option<Vec<GhReview>>,
    #[serde(default)]
    review_requests: Option<Vec<GhReviewRequest>>,
    #[serde(default)]
    comments: Option<Vec<GhIssueComment>>,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    mergeable: String,
    #[serde(default)]
    merge_state_status: String,
}

/// Comentário em linha da REST API (`pulls/{n}/comments`).
#[derive(Debug, Deserialize)]
struct GhReviewComment {
    id: u64,
    #[serde(default)]
    in_reply_to_id: Option<u64>,
    #[serde(default)]
    path: String,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    original_line: Option<u32>,
    #[serde(default)]
    diff_hunk: String,
    #[serde(default)]
    user: Option<GhActor>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    created_at: String,
}

// --- normalização ------------------------------------------------------------

fn lower(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn login(actor: Option<GhActor>) -> String {
    actor.map(|a| a.login).unwrap_or_default()
}

/// O GraphQL do GitHub manda `0001-01-01T00:00:00Z` para "ainda não aconteceu".
fn timestamp(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty() && !v.starts_with("0001-01-01"))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

fn pr_state(state: &str, is_draft: bool) -> PrState {
    match lower(state).as_str() {
        "merged" => PrState::Merged,
        "closed" => PrState::Closed,
        // Rascunho só enquanto aberto: o GitHub mantém `isDraft` num PR fechado,
        // e mostrá-lo como rascunho esconderia que já foi fechado.
        _ if is_draft => PrState::Draft,
        _ => PrState::Open,
    }
}

fn review_decision(value: Option<String>) -> Option<PrReviewDecision> {
    match lower(value.as_deref().unwrap_or_default()).as_str() {
        "approved" => Some(PrReviewDecision::Approved),
        "changes_requested" => Some(PrReviewDecision::ChangesRequested),
        "review_required" => Some(PrReviewDecision::ReviewRequired),
        _ => None,
    }
}

fn review_state(value: &str) -> PrReviewState {
    match lower(value).as_str() {
        "approved" => PrReviewState::Approved,
        "changes_requested" => PrReviewState::ChangesRequested,
        "dismissed" => PrReviewState::Dismissed,
        "pending" => PrReviewState::Pending,
        _ => PrReviewState::Commented,
    }
}

fn mergeable(value: &str) -> PrMergeable {
    match lower(value).as_str() {
        "mergeable" => PrMergeable::Mergeable,
        "conflicting" => PrMergeable::Conflicting,
        _ => PrMergeable::Unknown,
    }
}

/// CheckRun: só `completed` tem conclusão que conta; StatusContext: `state`.
/// O que não terminou (queued, in_progress, pending, expected) é pendente.
fn check_status(status: &str, conclusion: Option<&str>) -> PrCheckStatus {
    if lower(status) != "completed" {
        return PrCheckStatus::Pending;
    }
    conclusion_status(conclusion.unwrap_or_default())
}

fn conclusion_status(value: &str) -> PrCheckStatus {
    match lower(value).as_str() {
        "success" | "neutral" => PrCheckStatus::Success,
        "skipped" => PrCheckStatus::Skipped,
        "failure" | "cancelled" | "timed_out" | "action_required" | "error" | "startup_failure" => {
            PrCheckStatus::Failure
        }
        _ => PrCheckStatus::Pending,
    }
}

fn to_check(item: GhRollup) -> Option<PrCheck> {
    match item {
        GhRollup::CheckRun {
            name,
            status,
            conclusion,
            workflow_name,
            started_at,
            completed_at,
            details_url,
        } => Some(PrCheck {
            status: check_status(&status, conclusion.as_deref()),
            name,
            workflow: non_empty(workflow_name),
            started_at: timestamp(started_at),
            completed_at: timestamp(completed_at),
            url: non_empty(details_url),
        }),
        GhRollup::StatusContext {
            context,
            state,
            started_at,
            target_url,
        } => Some(PrCheck {
            name: context,
            workflow: None,
            status: conclusion_status(&state),
            started_at: timestamp(started_at),
            completed_at: None,
            url: non_empty(target_url),
        }),
        GhRollup::Unknown => None,
    }
}

fn checks(rollup: Option<Vec<GhRollup>>) -> Vec<PrCheck> {
    rollup
        .unwrap_or_default()
        .into_iter()
        .filter_map(to_check)
        .collect()
}

/// `skipped` conta como passou: check pulado não bloqueia o merge, e somá-lo a
/// "pendente" deixaria o PR amarelo para sempre.
pub fn ci_summary(checks: &[PrCheck]) -> PrCiSummary {
    let mut ci = PrCiSummary::default();
    for check in checks {
        match check.status {
            PrCheckStatus::Success | PrCheckStatus::Skipped => ci.passed += 1,
            PrCheckStatus::Failure => ci.failed += 1,
            PrCheckStatus::Pending => ci.pending += 1,
        }
        ci.total += 1;
    }
    ci
}

fn reviewer(request: GhReviewRequest) -> Option<String> {
    non_empty(request.login).or_else(|| non_empty(request.name))
}

fn thread_comment(comment: GhIssueComment) -> PrThreadComment {
    PrThreadComment {
        author: login(comment.author),
        body: comment.body,
        created_at: comment.created_at,
    }
}

fn list_item(pr: GhListPr, me: &str) -> PrListItem {
    let checks = checks(pr.status_check_rollup);
    let approvals = pr
        .latest_reviews
        .unwrap_or_default()
        .iter()
        .filter(|r| review_state(&r.state) == PrReviewState::Approved)
        .count() as u32;
    let review_requested = !me.is_empty()
        && pr
            .review_requests
            .unwrap_or_default()
            .iter()
            .any(|r| r.login.as_deref() == Some(me));
    let (author, author_name) = match pr.author {
        Some(a) => (a.login, non_empty(a.name)),
        None => (String::new(), None),
    };
    PrListItem {
        number: pr.number,
        title: pr.title,
        url: pr.url,
        state: pr_state(&pr.state, pr.is_draft),
        author,
        author_name,
        head: pr.head_ref_name,
        base: pr.base_ref_name,
        updated_at: pr.updated_at,
        merged_at: timestamp(pr.merged_at),
        merged_by: pr.merged_by.map(|a| a.login).filter(|l| !l.is_empty()),
        ci: ci_summary(&checks),
        review: review_decision(pr.review_decision),
        approvals,
        review_requested,
        comments: pr.comments.map(|c| c.len()).unwrap_or(0) as u32,
        additions: pr.additions,
        deletions: pr.deletions,
        session: None,
    }
}

/// `me` é o login do usuário no `gh`: decide `review_requested`.
pub fn parse_list(stdout: &[u8], me: &str) -> Result<Vec<PrListItem>, String> {
    let prs: Vec<GhListPr> = serde_json::from_slice(stdout)
        .map_err(|e| format!("resposta inesperada de `gh pr list`: {e}"))?;
    Ok(prs.into_iter().map(|pr| list_item(pr, me)).collect())
}

/// Contagens das abas: sempre sobre a lista de abertos.
pub fn counts(open: &[PrListItem], me: &str) -> PrCounts {
    let n = |f: &dyn Fn(&PrListItem) -> bool| open.iter().filter(|p| f(p)).count() as u32;
    PrCounts {
        open: open.len() as u32,
        mine: n(&|p| !me.is_empty() && p.author == me),
        review: n(&|p| p.review_requested),
        draft: n(&|p| p.state == PrState::Draft),
    }
}

/// O detalhe sem as threads, que vêm de outra chamada ([`parse_threads`]).
pub fn parse_detail(stdout: &[u8]) -> Result<PrDetail, String> {
    let pr: GhDetailPr = serde_json::from_slice(stdout)
        .map_err(|e| format!("resposta inesperada de `gh pr view`: {e}"))?;
    Ok(PrDetail {
        number: pr.number,
        title: pr.title,
        url: pr.url,
        state: pr_state(&pr.state, pr.is_draft),
        author: login(pr.author),
        head: pr.head_ref_name,
        head_sha: pr.head_ref_oid,
        base: pr.base_ref_name,
        created_at: pr.created_at,
        body: pr.body,
        additions: pr.additions,
        deletions: pr.deletions,
        changed_files: pr.changed_files,
        commits: pr
            .commits
            .unwrap_or_default()
            .into_iter()
            .map(|c| PrCommit {
                sha: c.oid,
                subject: c.message_headline,
                author: c
                    .authors
                    .into_iter()
                    .next()
                    .map(|a| {
                        if a.login.is_empty() {
                            a.name.unwrap_or_default()
                        } else {
                            a.login
                        }
                    })
                    .unwrap_or_default(),
                authored_at: c.authored_date,
            })
            .collect(),
        files: pr
            .files
            .unwrap_or_default()
            .into_iter()
            .map(|f| PrFile {
                path: f.path,
                additions: f.additions,
                deletions: f.deletions,
            })
            .collect(),
        checks: checks(pr.status_check_rollup),
        reviews: pr
            .reviews
            .unwrap_or_default()
            .into_iter()
            .map(|r| PrReview {
                author: login(r.author),
                state: review_state(&r.state),
                submitted_at: timestamp(r.submitted_at),
                body: r.body,
                commit: r.commit.map(|c| c.oid).filter(|o| !o.is_empty()),
            })
            .collect(),
        pending_reviewers: pr
            .review_requests
            .unwrap_or_default()
            .into_iter()
            .filter_map(reviewer)
            .collect(),
        threads: Vec::new(),
        comments: pr
            .comments
            .unwrap_or_default()
            .into_iter()
            .map(thread_comment)
            .collect(),
        review: review_decision(pr.review_decision),
        mergeable: mergeable(&pr.mergeable),
        merge_state: lower(&pr.merge_state_status),
        session: None,
    })
}

/// Saída de `gh api --paginate`: um array JSON por página, concatenados
/// (`[...][...]`). A raiz (sem `in_reply_to_id`) abre a thread; resposta vai
/// para a thread da raiz — mesmo se apontar para outra resposta. Resposta cuja
/// raiz não veio abre a própria thread em vez de sumir.
pub fn parse_threads(stdout: &[u8]) -> Result<Vec<PrThread>, String> {
    let mut comments = Vec::new();
    for page in serde_json::Deserializer::from_slice(stdout).into_iter::<Vec<GhReviewComment>>() {
        comments.extend(page.map_err(|e| format!("resposta inesperada da API do GitHub: {e}"))?);
    }
    let mut threads: Vec<PrThread> = Vec::new();
    let mut thread_of: HashMap<u64, usize> = HashMap::new();
    for c in comments {
        let parent = c
            .in_reply_to_id
            .and_then(|parent| thread_of.get(&parent).copied());
        let idx = match parent {
            Some(idx) => idx,
            None => {
                threads.push(PrThread {
                    path: c.path.clone(),
                    line: c.line.or(c.original_line),
                    diff_hunk: c.diff_hunk.clone(),
                    comments: Vec::new(),
                });
                threads.len() - 1
            }
        };
        thread_of.insert(c.id, idx);
        threads[idx].comments.push(PrThreadComment {
            author: login(c.user),
            body: c.body,
            created_at: c.created_at,
        });
    }
    Ok(threads)
}

// --- execução ------------------------------------------------------------------

/// Nome de owner/repo/host do GitHub: o que vai para argv e para o caminho da
/// API. Qualquer coisa fora disso (inclusive `-` no começo) é recusada.
fn safe_segment(value: &str, extra: &[char]) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || extra.contains(&c))
}

fn github_remote(repo_root: &Path) -> Option<Remote> {
    let remote = remote_of(repo_root)?;
    if resolve_kind(&remote)? != ForgeKind::GitHub || cli_missing(ForgeKind::GitHub) {
        return None;
    }
    let valid = safe_segment(&remote.host, &['.'])
        && safe_segment(&remote.owner, &[])
        && safe_segment(&remote.repo, &['.']);
    valid.then_some(remote)
}

fn repo_flag(remote: &Remote) -> String {
    format!("{}/{}/{}", remote.host, remote.owner, remote.repo)
}

fn gh(repo_root: &Path, args: &[&str]) -> Result<Vec<u8>, AppError> {
    let out = exec::run(github::CLI, args, repo_root, None, NETWORK_TIMEOUT).map_err(cli_failed)?;
    if !out.ok {
        return Err(cli_failed(out.stderr_message()));
    }
    Ok(out.stdout)
}

/// Login do usuário no `gh`, por host, pela vida do app: não muda sem um
/// `gh auth` — e perguntá-lo a cada página seria um processo a mais por refresh.
fn viewer_login(repo_root: &Path, remote: &Remote) -> Result<String, AppError> {
    static LOGINS: std::sync::OnceLock<Mutex<HashMap<String, String>>> = std::sync::OnceLock::new();
    let logins = LOGINS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(login) = logins.lock().get(&remote.host) {
        return Ok(login.clone());
    }
    let out = gh(
        repo_root,
        &["api", "--hostname", &remote.host, "user", "--jq", ".login"],
    )?;
    let login = String::from_utf8_lossy(&out).trim().to_string();
    if !login.is_empty() {
        logins.lock().insert(remote.host.clone(), login.clone());
    }
    Ok(login)
}

fn list(
    repo_root: &Path,
    remote: &Remote,
    me: &str,
    filter: &[&str],
    limit: &str,
) -> Result<Vec<PrListItem>, AppError> {
    let repo = repo_flag(remote);
    let mut args = vec!["pr", "list", "--repo", &repo, "--limit", limit];
    args.extend_from_slice(filter);
    args.extend_from_slice(&["--json", LIST_FIELDS]);
    parse_list(&gh(repo_root, &args)?, me).map_err(cli_failed)
}

fn fetch_page(repo_root: &Path, remote: &Remote, filter: PrFilter) -> Result<PrPage, AppError> {
    let me = viewer_login(repo_root, remote)?;
    let open = list(repo_root, remote, &me, &["--state", "open"], LIST_LIMIT)?;
    let (items, recent_merged) = match filter {
        PrFilter::Open => {
            let merged = list(
                repo_root,
                remote,
                &me,
                &["--state", "merged"],
                RECENT_MERGED_LIMIT,
            )?;
            (open.clone(), merged)
        }
        PrFilter::Mine => (
            list(repo_root, remote, &me, &["--author", "@me"], LIST_LIMIT)?,
            Vec::new(),
        ),
        PrFilter::Review => (
            list(
                repo_root,
                remote,
                &me,
                &["--search", "review-requested:@me"],
                LIST_LIMIT,
            )?,
            Vec::new(),
        ),
        PrFilter::Closed => (
            list(repo_root, remote, &me, &["--state", "closed"], LIST_LIMIT)?,
            Vec::new(),
        ),
    };
    Ok(PrPage {
        counts: counts(&open, &me),
        items,
        recent_merged,
        repo: remote.project(),
        forge: ForgeKind::GitHub,
        fetched_at: now_iso(),
    })
}

fn fetch_detail(repo_root: &Path, remote: &Remote, number: u64) -> Result<PrDetail, AppError> {
    let repo = repo_flag(remote);
    let n = number.to_string();
    let view = gh(
        repo_root,
        &[
            "pr",
            "view",
            "--repo",
            &repo,
            "--json",
            DETAIL_FIELDS,
            "--",
            &n,
        ],
    )?;
    let mut detail = parse_detail(&view).map_err(cli_failed)?;
    let endpoint = format!(
        "repos/{}/{}/pulls/{number}/comments?per_page=100",
        remote.owner, remote.repo
    );
    let threads = gh(
        repo_root,
        &["api", "--hostname", &remote.host, "--paginate", &endpoint],
    )?;
    detail.threads = parse_threads(&threads).map_err(cli_failed)?;
    Ok(detail)
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// --- cache -----------------------------------------------------------------------

type Cache<K, V> = Mutex<HashMap<K, (Instant, V)>>;

fn page_cache() -> &'static Cache<(String, PrFilter), PrPage> {
    static CACHE: std::sync::OnceLock<Cache<(String, PrFilter), PrPage>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn detail_cache() -> &'static Cache<(String, u64), PrDetail> {
    static CACHE: std::sync::OnceLock<Cache<(String, u64), PrDetail>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached<K, V, F>(cache: &Cache<K, V>, key: K, fresh: bool, fetch: F) -> Result<V, AppError>
where
    K: std::hash::Hash + Eq,
    V: Clone,
    F: FnOnce() -> Result<V, AppError>,
{
    if !fresh {
        if let Some((at, value)) = cache.lock().get(&key) {
            if at.elapsed() <= CACHE_TTL {
                return Ok(value.clone());
            }
        }
    }
    let value = fetch()?;
    cache.lock().insert(key, (Instant::now(), value.clone()));
    Ok(value)
}

/// `None`: a forja não tem lista rica aqui (GitLab, sem forja, sem `gh`).
/// `fresh=false` aceita o cache de 30s.
pub fn page(repo_root: &Path, filter: PrFilter, fresh: bool) -> Result<Option<PrPage>, AppError> {
    let Some(remote) = github_remote(repo_root) else {
        return Ok(None);
    };
    let key = (repo_flag(&remote), filter);
    cached(page_cache(), key, fresh, || {
        fetch_page(repo_root, &remote, filter)
    })
    .map(Some)
}

pub fn detail(repo_root: &Path, number: u64, fresh: bool) -> Result<Option<PrDetail>, AppError> {
    if number == 0 {
        return Err(cli_failed("número de PR inválido"));
    }
    let Some(remote) = github_remote(repo_root) else {
        return Ok(None);
    };
    let key = (repo_flag(&remote), number);
    cached(detail_cache(), key, fresh, || {
        fetch_detail(repo_root, &remote, number)
    })
    .map(Some)
}

// --- revisar e mergear (fatias 5–6): VERMELHO ------------------------------------
//
// Só a UI dispara, depois de uma confirmação que mostra o PR e o texto — do
// mesmo jeito que `forge_create_pr` e o merge local: são commands do Tauri, que
// o agente não alcança. Agente que digita `gh pr review`/`gh pr merge` no
// terminal cai no classificador (`approvals`, `RedReason::ForgeWrite`), que
// sempre pede aprovação.
//
// `head_sha` amarra a ação ao que o usuário viu. Se o PR andou, recusa com
// `forge.pr_head_moved` para a UI pedir uma nova revisão.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrReviewEvent {
    Comment,
    Approve,
    RequestChanges,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrMergeStrategy {
    Squash,
    Merge,
    Rebase,
}

fn invalid_number() -> AppError {
    cli_failed("número de PR inválido")
}

fn is_full_sha(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn check_target(number: u64, head_sha: &str) -> Result<(), AppError> {
    if number == 0 {
        return Err(invalid_number());
    }
    if !is_full_sha(head_sha) {
        return Err(AppError::new("forge.pr_invalid_head"));
    }
    Ok(())
}

/// A decisão do "o PR andou?", sem rede: `current` é o `headRefOid` de agora.
pub fn ensure_head(expected: &str, current: &str) -> Result<(), AppError> {
    if expected.eq_ignore_ascii_case(current.trim()) {
        Ok(())
    } else {
        Err(AppError::new("forge.pr_head_moved")
            .with("expected", expected)
            .with("current", current.trim()))
    }
}

/// argv do `gh pr review` e o stdin. O corpo vai SEMPRE pelo stdin
/// (`--body-file -`), nunca em argv; `approve` sem corpo não manda nenhum.
pub fn review_args(
    repo: &str,
    number: u64,
    event: PrReviewEvent,
    body: &str,
    head_sha: &str,
) -> Result<(Vec<String>, Option<Vec<u8>>), AppError> {
    check_target(number, head_sha)?;
    let empty = body.trim().is_empty();
    if empty && event != PrReviewEvent::Approve {
        return Err(AppError::new("forge.pr_review_body_empty"));
    }
    let flag = match event {
        PrReviewEvent::Comment => "--comment",
        PrReviewEvent::Approve => "--approve",
        PrReviewEvent::RequestChanges => "--request-changes",
    };
    let mut args: Vec<String> = ["pr", "review", "--repo", repo, flag]
        .map(String::from)
        .to_vec();
    let stdin = if empty {
        None
    } else {
        args.extend(["--body-file".into(), "-".into()]);
        Some(body.as_bytes().to_vec())
    };
    args.extend(["--".into(), number.to_string()]);
    Ok((args, stdin))
}

/// argv do `gh pr merge`. Nunca `--admin` (pula proteção de branch), nunca
/// `--auto` (merge depois, sem ninguém olhando), nunca `--delete-branch`.
/// `--match-head-commit` faz o GitHub recusar se o head mudou desde a checagem.
///
/// Mergear para main/master é permitido aqui: a regra 5 do CLAUDE.md recusa
/// PUSH de sessão de agente para main; isto é o dono mergeando pela UI.
pub fn merge_args(
    repo: &str,
    number: u64,
    strategy: PrMergeStrategy,
    head_sha: &str,
) -> Result<Vec<String>, AppError> {
    check_target(number, head_sha)?;
    let flag = match strategy {
        PrMergeStrategy::Squash => "--squash",
        PrMergeStrategy::Merge => "--merge",
        PrMergeStrategy::Rebase => "--rebase",
    };
    Ok([
        "pr",
        "merge",
        "--repo",
        repo,
        flag,
        "--match-head-commit",
        head_sha,
        "--",
        &number.to_string(),
    ]
    .map(String::from)
    .to_vec())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhHead {
    head_ref_oid: String,
}

pub fn parse_head(stdout: &[u8]) -> Result<String, String> {
    serde_json::from_slice::<GhHead>(stdout)
        .map(|h| h.head_ref_oid)
        .map_err(|e| format!("resposta inesperada de `gh pr view`: {e}"))
}

/// Ação só existe no GitHub com `gh`: sem isso, não há o que degradar.
fn action_remote(repo_root: &Path) -> Result<Remote, AppError> {
    github_remote(repo_root).ok_or_else(|| AppError::new("forge.pr_actions_unsupported"))
}

/// Head de agora, sem cache: o do cache pode ter 30s.
fn current_head(repo_root: &Path, repo: &str, number: u64) -> Result<String, AppError> {
    let n = number.to_string();
    let out = gh(
        repo_root,
        &[
            "pr",
            "view",
            "--repo",
            repo,
            "--json",
            "headRefOid",
            "--",
            &n,
        ],
    )?;
    parse_head(&out).map_err(cli_failed)
}

fn gh_write(repo_root: &Path, args: &[String], stdin: Option<&[u8]>) -> Result<(), AppError> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out =
        exec::run(github::CLI, &args, repo_root, stdin, NETWORK_TIMEOUT).map_err(cli_failed)?;
    if !out.ok {
        return Err(cli_failed(out.stderr_message()));
    }
    Ok(())
}

fn invalidate(repo: &str, number: u64) {
    detail_cache()
        .lock()
        .retain(|(r, n), _| !(r == repo && *n == number));
    page_cache().lock().retain(|(r, _), _| r != repo);
}

/// Sobra uma janela entre a checagem do head e o review: o `gh pr review` não
/// aceita commit alvo. No merge, `--match-head-commit` fecha essa janela.
pub fn review(
    repo_root: &Path,
    number: u64,
    event: PrReviewEvent,
    body: &str,
    head_sha: &str,
) -> Result<(), AppError> {
    check_target(number, head_sha)?;
    let remote = action_remote(repo_root)?;
    let repo = repo_flag(&remote);
    let (args, stdin) = review_args(&repo, number, event, body, head_sha)?;
    ensure_head(head_sha, &current_head(repo_root, &repo, number)?)?;
    gh_write(repo_root, &args, stdin.as_deref())?;
    invalidate(&repo, number);
    Ok(())
}

pub fn merge(
    repo_root: &Path,
    number: u64,
    strategy: PrMergeStrategy,
    head_sha: &str,
) -> Result<(), AppError> {
    check_target(number, head_sha)?;
    let remote = action_remote(repo_root)?;
    let repo = repo_flag(&remote);
    let args = merge_args(&repo, number, strategy, head_sha)?;
    ensure_head(head_sha, &current_head(repo_root, &repo, number)?)?;
    gh_write(repo_root, &args, None)?;
    invalidate(&repo, number);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = include_str!("fixtures/gh_pr_list_rich.json");
    const VIEW: &str = include_str!("fixtures/gh_pr_view_rich.json");
    const THREADS: &str = include_str!("fixtures/gh_pr_review_threads.json");

    #[test]
    fn list_maps_state_draft_and_merged() {
        let items = parse_list(LIST.as_bytes(), "eu-mesmo").unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].state, PrState::Open);
        assert_eq!(items[1].state, PrState::Draft);
        assert_eq!(items[2].state, PrState::Merged);
        assert_eq!(items[2].merged_by.as_deref(), Some("ana-dev"));
        assert_eq!(items[2].merged_at.as_deref(), Some("2025-12-30T10:00:00Z"));
        assert_eq!(items[0].merged_by, None);
    }

    #[test]
    fn closed_draft_is_closed_not_draft() {
        assert_eq!(pr_state("CLOSED", true), PrState::Closed);
        assert_eq!(pr_state("OPEN", true), PrState::Draft);
    }

    #[test]
    fn list_aggregates_ci_from_the_rollup() {
        let items = parse_list(LIST.as_bytes(), "eu-mesmo").unwrap();
        // build ✓, skipped conta como passou; lint ✗; e2e e o StatusContext pendentes.
        assert_eq!(
            items[0].ci,
            PrCiSummary {
                passed: 2,
                failed: 1,
                pending: 2,
                total: 5
            }
        );
        assert_eq!(items[1].ci, PrCiSummary::default());
        // `statusCheckRollup: null` não pode derrubar o parse.
        assert_eq!(items[2].ci.total, 0);
    }

    #[test]
    fn list_counts_approvals_review_and_comments() {
        let items = parse_list(LIST.as_bytes(), "eu-mesmo").unwrap();
        assert_eq!(items[0].approvals, 1);
        assert_eq!(items[0].review, Some(PrReviewDecision::Approved));
        assert_eq!(items[1].review, None, "reviewDecision vazio vira null");
        assert_eq!(items[2].review, Some(PrReviewDecision::ChangesRequested));
        assert_eq!(items[2].approvals, 2);
        assert_eq!(items[0].comments, 2);
        assert_eq!(items[0].author_name.as_deref(), Some("Ana Exemplo"));
        assert_eq!(items[1].author_name, None, "nome vazio vira null");
        assert!(items[0].review_requested);
        // Pedido a um time não é pedido ao usuário.
        assert!(!items[1].review_requested);
    }

    #[test]
    fn counts_come_from_the_open_list() {
        let items = parse_list(LIST.as_bytes(), "eu-mesmo").unwrap();
        let open: Vec<_> = items.into_iter().take(2).collect();
        assert_eq!(
            counts(&open, "eu-mesmo"),
            PrCounts {
                open: 2,
                mine: 1,
                review: 1,
                draft: 1
            }
        );
        // Sem login conhecido, nada é "meu".
        assert_eq!(counts(&open, "").mine, 0);
    }

    #[test]
    fn list_serializes_with_the_ts_contract_names() {
        let items = parse_list(LIST.as_bytes(), "eu-mesmo").unwrap();
        let v = serde_json::to_value(&items[1]).unwrap();
        assert_eq!(v["state"], "draft");
        assert_eq!(v["review"], serde_json::Value::Null);
        assert_eq!(v["session"], serde_json::Value::Null);
        assert_eq!(v["head"], "rascunho");
        let v = serde_json::to_value(&items[2]).unwrap();
        assert_eq!(v["review"], "changes_requested");
    }

    #[test]
    fn detail_maps_checks_with_workflow_and_mergeable() {
        let d = parse_detail(VIEW.as_bytes()).unwrap();
        assert_eq!(d.number, 42);
        assert_eq!(d.head_sha, "2222222222222222222222222222222222222222");
        assert_eq!(d.state, PrState::Open);
        assert_eq!(d.changed_files, 2);
        assert_eq!(d.mergeable, PrMergeable::Conflicting);
        assert_eq!(d.merge_state, "dirty");
        assert_eq!(d.review, Some(PrReviewDecision::ChangesRequested));

        assert_eq!(d.checks.len(), 4);
        assert_eq!(d.checks[0].workflow.as_deref(), Some("CI"));
        assert_eq!(d.checks[0].status, PrCheckStatus::Success);
        assert_eq!(
            d.checks[0].completed_at.as_deref(),
            Some("2026-01-02T09:05:00Z")
        );
        // Na fila: sem workflow, sem horário (o zero do GraphQL vira null), sem url.
        assert_eq!(d.checks[1].status, PrCheckStatus::Pending);
        assert_eq!(d.checks[1].workflow, None);
        assert_eq!(d.checks[1].started_at, None);
        assert_eq!(d.checks[1].url, None);
        assert_eq!(d.checks[2].status, PrCheckStatus::Skipped);
        assert_eq!(d.checks[3].name, "ci/externo");
        assert_eq!(d.checks[3].status, PrCheckStatus::Failure);

        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["mergeable"], "conflicting");
        assert_eq!(v["checks"][2]["status"], "skipped");
        assert_eq!(v["reviews"][0]["state"], "changes_requested");
    }

    #[test]
    fn detail_maps_commits_reviews_reviewers_and_comments() {
        let d = parse_detail(VIEW.as_bytes()).unwrap();
        assert_eq!(d.commits.len(), 2);
        assert_eq!(d.commits[0].subject, "feat: primeiro passo");
        assert_eq!(d.commits[0].author, "ana-dev");
        assert_eq!(d.commits[1].author, "Sem Login", "sem login, usa o nome");
        assert_eq!(d.files[1].path, "src/tela.test.ts");
        assert_eq!(d.reviews[1].state, PrReviewState::Approved);
        assert_eq!(
            d.reviews[1].commit.as_deref(),
            Some("2222222222222222222222222222222222222222")
        );
        assert_eq!(d.pending_reviewers, vec!["eu-mesmo", "time-core"]);
        assert_eq!(d.comments.len(), 1);
        assert_eq!(d.comments[0].author, "beto-dev");
    }

    #[test]
    fn threads_group_replies_under_their_root_across_pages() {
        let threads = parse_threads(THREADS.as_bytes()).unwrap();
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].path, "src/tela.ts");
        assert_eq!(threads[0].line, Some(10));
        assert!(threads[0].diff_hunk.starts_with("@@"));
        let bodies: Vec<&str> = threads[0]
            .comments
            .iter()
            .map(|c| c.body.as_str())
            .collect();
        assert_eq!(
            bodies,
            vec!["por que isso?", "explico abaixo", "segunda página"]
        );
        assert_eq!(
            threads[0].comments[2].author, "",
            "usuário apagado não derruba"
        );
        // Comentário desatualizado: cai para `original_line`.
        assert_eq!(threads[1].line, Some(7));
        assert_eq!(threads[1].comments.len(), 1);
    }

    #[test]
    fn orphan_reply_opens_its_own_thread() {
        let json = br#"[{"id": 9, "in_reply_to_id": 1, "path": "a", "diff_hunk": "", "body": "x", "created_at": ""}]"#;
        assert_eq!(parse_threads(json).unwrap().len(), 1);
    }

    #[test]
    fn parsers_reject_garbage() {
        assert!(parse_list(b"nao sou json", "").is_err());
        assert!(parse_detail(b"[]").is_err());
        assert!(parse_threads(b"{}").is_err());
        assert!(parse_threads(b"").unwrap().is_empty());
    }

    #[test]
    fn remote_segments_that_could_be_flags_are_refused() {
        assert!(safe_segment("tybadev", &[]));
        assert!(safe_segment("tyba-terminal.v2", &['.']));
        assert!(!safe_segment("-R", &[]));
        assert!(!safe_segment("a/b", &[]));
        assert!(!safe_segment("..", &['.']));
        assert!(safe_segment(".github", &['.']));
        assert!(!safe_segment("", &[]));
    }

    #[test]
    fn gitlab_or_unknown_forge_has_no_rich_list() {
        for origin in [
            "https://gitlab.com/group/app.git",
            "https://bitbucket.org/t/a.git",
        ] {
            let dir = tempfile::tempdir().unwrap();
            for args in [vec!["init", "-q"], vec!["remote", "add", "origin", origin]] {
                let ok = std::process::Command::new("git")
                    .args(&args)
                    .current_dir(dir.path())
                    .status()
                    .unwrap()
                    .success();
                assert!(ok);
            }
            assert_eq!(page(dir.path(), PrFilter::Open, false).unwrap(), None);
            assert_eq!(detail(dir.path(), 1, false).unwrap(), None);
        }
    }

    #[test]
    fn detail_rejects_number_zero() {
        assert!(detail(Path::new("/"), 0, false).is_err());
    }

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    const REPO: &str = "github.com/exemplo/app";

    #[test]
    fn review_body_goes_to_stdin_never_argv() {
        let corpo = "texto do review --admin";
        for (event, flag) in [
            (PrReviewEvent::Comment, "--comment"),
            (PrReviewEvent::Approve, "--approve"),
            (PrReviewEvent::RequestChanges, "--request-changes"),
        ] {
            let (args, stdin) = review_args(REPO, 7, event, corpo, SHA).unwrap();
            assert_eq!(
                args,
                [
                    "pr",
                    "review",
                    "--repo",
                    REPO,
                    flag,
                    "--body-file",
                    "-",
                    "--",
                    "7"
                ]
            );
            assert_eq!(stdin.as_deref(), Some(corpo.as_bytes()));
            assert!(!args.iter().any(|a| a.contains("texto")));
        }
    }

    #[test]
    fn empty_body_only_for_approve() {
        let (args, stdin) = review_args(REPO, 7, PrReviewEvent::Approve, "  ", SHA).unwrap();
        assert_eq!(
            args,
            ["pr", "review", "--repo", REPO, "--approve", "--", "7"]
        );
        assert_eq!(stdin, None);
        for event in [PrReviewEvent::Comment, PrReviewEvent::RequestChanges] {
            let err = review_args(REPO, 7, event, " \n", SHA).unwrap_err();
            assert_eq!(err.code, "forge.pr_review_body_empty");
        }
    }

    #[test]
    fn merge_args_by_strategy_without_dangerous_flags() {
        for (strategy, flag) in [
            (PrMergeStrategy::Squash, "--squash"),
            (PrMergeStrategy::Merge, "--merge"),
            (PrMergeStrategy::Rebase, "--rebase"),
        ] {
            let args = merge_args(REPO, 9, strategy, SHA).unwrap();
            assert_eq!(
                args,
                [
                    "pr",
                    "merge",
                    "--repo",
                    REPO,
                    flag,
                    "--match-head-commit",
                    SHA,
                    "--",
                    "9"
                ]
            );
            for proibida in ["--admin", "--auto", "--delete-branch", "-d"] {
                assert!(!args.iter().any(|a| a == proibida), "{proibida}");
            }
        }
    }

    #[test]
    fn actions_validate_number_and_head_sha() {
        let curto = &SHA[..39];
        let nao_hex = "g123456789abcdef0123456789abcdef01234567";
        for sha in [curto, nao_hex, "", "--admin"] {
            assert_eq!(
                merge_args(REPO, 1, PrMergeStrategy::Squash, sha)
                    .unwrap_err()
                    .code,
                "forge.pr_invalid_head",
                "{sha}"
            );
            assert_eq!(
                review_args(REPO, 1, PrReviewEvent::Approve, "", sha)
                    .unwrap_err()
                    .code,
                "forge.pr_invalid_head"
            );
        }
        assert_eq!(
            merge_args(REPO, 0, PrMergeStrategy::Squash, SHA)
                .unwrap_err()
                .code,
            "forge.cli_failed"
        );
        assert!(review_args(REPO, 0, PrReviewEvent::Approve, "", SHA).is_err());
        // A validação vem antes de qualquer rede.
        assert!(merge(Path::new("/"), 0, PrMergeStrategy::Merge, SHA).is_err());
        assert_eq!(
            review(Path::new("/"), 1, PrReviewEvent::Approve, "", "x")
                .unwrap_err()
                .code,
            "forge.pr_invalid_head"
        );
    }

    #[test]
    fn head_moved_is_refused() {
        assert!(ensure_head(SHA, SHA).is_ok());
        assert!(ensure_head(SHA, &format!("{}\n", SHA.to_uppercase())).is_ok());
        let outro = "fedcba9876543210fedcba9876543210fedcba98";
        let err = ensure_head(SHA, outro).unwrap_err();
        assert_eq!(err.code, "forge.pr_head_moved");
        assert_eq!(err.params["current"], outro);
        assert_eq!(err.params["expected"], SHA);
    }

    #[test]
    fn head_parses_from_gh_pr_view() {
        let json = format!(r#"{{"headRefOid":"{SHA}"}}"#);
        assert_eq!(parse_head(json.as_bytes()).unwrap(), SHA);
        assert!(parse_head(b"{}").is_err());
    }

    #[test]
    fn events_and_strategies_use_the_ts_contract_names() {
        let e: PrReviewEvent = serde_json::from_str(r#""request_changes""#).unwrap();
        assert_eq!(e, PrReviewEvent::RequestChanges);
        let s: PrMergeStrategy = serde_json::from_str(r#""squash""#).unwrap();
        assert_eq!(s, PrMergeStrategy::Squash);
        assert!(serde_json::from_str::<PrMergeStrategy>(r#""admin""#).is_err());
    }

    #[test]
    fn gitlab_has_no_pr_actions() {
        let dir = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["remote", "add", "origin", "https://gitlab.com/g/a.git"],
        ] {
            assert!(std::process::Command::new("git")
                .args(&args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success());
        }
        assert_eq!(
            merge(dir.path(), 1, PrMergeStrategy::Squash, SHA)
                .unwrap_err()
                .code,
            "forge.pr_actions_unsupported"
        );
    }

    #[test]
    fn success_invalidates_detail_and_lists_of_the_repo() {
        let repo = "github.com/invalida/teste";
        let page = PrPage {
            items: Vec::new(),
            recent_merged: Vec::new(),
            counts: PrCounts::default(),
            repo: "invalida/teste".into(),
            forge: ForgeKind::GitHub,
            fetched_at: String::new(),
        };
        let now = Instant::now();
        let detail = parse_detail(VIEW.as_bytes()).unwrap();
        {
            let mut pages = page_cache().lock();
            pages.insert((repo.into(), PrFilter::Open), (now, page.clone()));
            pages.insert((repo.into(), PrFilter::Mine), (now, page.clone()));
            pages.insert(
                ("github.com/outro/repo".into(), PrFilter::Open),
                (now, page),
            );
            let mut details = detail_cache().lock();
            details.insert((repo.into(), 42), (now, detail.clone()));
            details.insert((repo.into(), 43), (now, detail));
        }
        invalidate(repo, 42);
        let pages = page_cache().lock();
        assert!(!pages.keys().any(|(r, _)| r == repo));
        assert!(pages.contains_key(&("github.com/outro/repo".to_string(), PrFilter::Open)));
        let details = detail_cache().lock();
        assert!(!details.contains_key(&(repo.to_string(), 42)));
        assert!(details.contains_key(&(repo.to_string(), 43)));
    }
}
