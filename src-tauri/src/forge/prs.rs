//! Lista rica e detalhe de PR (git completo, fatias 3–4: só leitura).
//!
//! Só GitHub por enquanto: para GitLab, sem forja ou sem `gh`, as consultas
//! devolvem `None` — a UI lê isso como "esta forja não tem lista rica", não
//! como "não há PR". Todo texto que vem daqui (título, corpo, comentário) é
//! entrada não confiável; quem renderiza trata como texto.
//!
//! O vínculo com sessão do TYBA (`session`) sai sempre `None` daqui: quem
//! conhece as sessões é o `lib.rs`, que preenche na saída — e por isso o cache
//! guarda a página sem ele (sessões nascem e morrem entre duas revalidações).
//! O cache é stale-while-revalidate (ver a seção "cache").

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::exec::{self, NETWORK_TIMEOUT};
use super::github::{self, cli_failed};
use super::rate::{self, ForgeRateLimit, RateLimit};
use super::remote::Remote;
use super::{cli_missing, remote_of, resolve_kind, ForgeKind, CACHE_TTL};
use crate::error::AppError;
use crate::session::SessionId;

/// Padrão e teto do `limit` da lista ("carregar mais" pede 200, 300…).
pub const LIST_LIMIT: u32 = 100;
pub const LIST_LIMIT_MAX: u32 = 1000;
const RECENT_MERGED_LIMIT: u32 = 5;

const LIST_FIELDS: &str = "number,title,url,state,isDraft,author,headRefName,baseRefName,\
updatedAt,mergedAt,mergedBy,statusCheckRollup,reviewDecision,latestReviews,reviewRequests,\
comments,additions,deletions,isCrossRepository,headRepositoryOwner";

/// Detalhe do PR numa chamada só (antes: `gh pr view --json` mais a REST de
/// comentários em linha, paginada). Conexões com teto de 100 itens: commits e
/// revisões trazem os ÚLTIMOS 100, arquivos e threads os primeiros 100 (o total
/// de arquivos segue em `changedFiles`). `rateLimit` vem junto, de graça.
pub const DETAIL_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) {
  rateLimit { limit remaining resetAt }
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      number title url state isDraft
      author { login }
      headRefName baseRefName headRefOid createdAt body
      additions deletions changedFiles
      isCrossRepository
      headRepositoryOwner { login }
      reviewDecision mergeable mergeStateStatus
      commits(last: 100) { nodes { commit {
        oid messageHeadline authoredDate
        authors(first: 1) { nodes { name user { login } } }
      } } }
      files(first: 100) { nodes { path additions deletions } }
      headCommit: commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes {
        __typename
        ... on CheckRun { name status conclusion startedAt completedAt detailsUrl checkSuite { workflowRun { workflow { name } } } }
        ... on StatusContext { context state createdAt targetUrl }
      } } } } } }
      reviews(last: 100) { nodes { author { login } state body submittedAt commit { oid } } }
      reviewRequests(first: 100) { nodes { requestedReviewer {
        __typename
        ... on User { login }
        ... on Bot { login }
        ... on Mannequin { login }
        ... on Team { name slug }
      } } }
      comments(last: 100) { nodes { author { login } body createdAt } }
      reviewThreads(first: 100) { nodes { path line originalLine
        comments(first: 100) { nodes { author { login } body createdAt diffHunk } } } }
    }
  }
}";

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
    /// PR vindo de fork (`isCrossRepository`).
    pub cross_repo: bool,
    /// O head está no próprio repo (não é fork e o dono do head é o owner do
    /// repo): só assim o PR pode se ligar a uma sessão. Não vai para a UI.
    #[serde(skip)]
    pub own_head: bool,
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
    /// A lista (ou a de abertos, de onde saem as contagens) bateu no `limit`.
    pub capped: bool,
    pub repo: String,
    pub forge: ForgeKind,
    pub fetched_at: String,
    /// Limite da API baixo: a revalidação automática parou até `reset_at`.
    /// Preenchido na saída, nunca guardado no cache.
    pub rate_limit: Option<ForgeRateLimit>,
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
    pub cross_repo: bool,
    /// Ver [`PrListItem::own_head`].
    #[serde(skip)]
    pub own_head: bool,
    pub session: Option<PrSessionLink>,
    /// Ver [`PrPage::rate_limit`].
    pub rate_limit: Option<ForgeRateLimit>,
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
    #[serde(default)]
    is_cross_repository: bool,
    #[serde(default)]
    head_repository_owner: Option<GhActor>,
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

// --- GraphQL do detalhe ([`DETAIL_QUERY`]) -------------------------------------

/// Conexão GraphQL: `nodes` pode trazer `null` (item sem acesso, conta apagada).
#[derive(Debug, Deserialize)]
struct Nodes<T> {
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<T>>,
}

fn nodes<T>(conn: Option<Nodes<T>>) -> Vec<T> {
    conn.map(|c| c.nodes.into_iter().flatten().collect())
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
struct GqlResponse {
    #[serde(default)]
    data: Option<GqlData>,
    #[serde(default)]
    errors: Vec<GqlError>,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    #[serde(default)]
    message: String,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlData {
    #[serde(default)]
    rate_limit: Option<GqlRate>,
    #[serde(default)]
    repository: Option<GqlRepo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlRate {
    limit: u64,
    remaining: u64,
    reset_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlRepo {
    #[serde(default)]
    pull_request: Option<GqlPr>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlPr {
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
    is_cross_repository: bool,
    #[serde(default)]
    head_repository_owner: Option<GhActor>,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    mergeable: Option<String>,
    #[serde(default)]
    merge_state_status: Option<String>,
    #[serde(default)]
    commits: Option<Nodes<GqlCommitNode>>,
    #[serde(default)]
    files: Option<Nodes<GhFile>>,
    #[serde(default)]
    head_commit: Option<Nodes<GqlHeadCommit>>,
    #[serde(default)]
    reviews: Option<Nodes<GhReview>>,
    #[serde(default)]
    review_requests: Option<Nodes<GqlReviewRequest>>,
    #[serde(default)]
    comments: Option<Nodes<GhIssueComment>>,
    #[serde(default)]
    review_threads: Option<Nodes<GqlThread>>,
}

#[derive(Debug, Deserialize)]
struct GqlCommitNode {
    commit: GqlCommit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlCommit {
    #[serde(default)]
    oid: String,
    #[serde(default)]
    message_headline: String,
    #[serde(default)]
    authored_date: String,
    #[serde(default)]
    authors: Option<Nodes<GqlGitActor>>,
}

#[derive(Debug, Deserialize)]
struct GqlGitActor {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    user: Option<GhActor>,
}

#[derive(Debug, Deserialize)]
struct GqlHeadCommit {
    commit: GqlRollupCommit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlRollupCommit {
    #[serde(default)]
    status_check_rollup: Option<GqlRollup>,
}

#[derive(Debug, Deserialize)]
struct GqlRollup {
    #[serde(default)]
    contexts: Option<Nodes<GqlContext>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum GqlContext {
    #[serde(rename_all = "camelCase")]
    CheckRun {
        #[serde(default)]
        name: String,
        #[serde(default)]
        status: String,
        #[serde(default)]
        conclusion: Option<String>,
        #[serde(default)]
        started_at: Option<String>,
        #[serde(default)]
        completed_at: Option<String>,
        #[serde(default)]
        details_url: Option<String>,
        #[serde(default)]
        check_suite: Option<GqlCheckSuite>,
    },
    #[serde(rename_all = "camelCase")]
    StatusContext {
        #[serde(default)]
        context: String,
        #[serde(default)]
        state: String,
        #[serde(default)]
        created_at: Option<String>,
        #[serde(default)]
        target_url: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlCheckSuite {
    #[serde(default)]
    workflow_run: Option<GqlWorkflowRun>,
}

#[derive(Debug, Deserialize)]
struct GqlWorkflowRun {
    #[serde(default)]
    workflow: Option<GqlNamed>,
}

#[derive(Debug, Deserialize)]
struct GqlNamed {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlReviewRequest {
    #[serde(default)]
    requested_reviewer: Option<GhReviewRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlThread {
    #[serde(default)]
    path: String,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    original_line: Option<u32>,
    #[serde(default)]
    comments: Option<Nodes<GqlThreadComment>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GqlThreadComment {
    #[serde(default)]
    author: Option<GhActor>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    diff_hunk: String,
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

/// Head no próprio repo: não é fork E o dono do head é o owner do repo. Um
/// fork pode ter uma branch com o mesmo nome da de uma sessão; sem isto, o PR
/// dele apareceria como aberto pela sessão. Dono desconhecido não liga.
fn own_head(cross_repo: bool, head_owner: Option<&GhActor>, repo_owner: &str) -> bool {
    !cross_repo
        && !repo_owner.is_empty()
        && head_owner.is_some_and(|o| o.login.eq_ignore_ascii_case(repo_owner))
}

fn list_item(pr: GhListPr, me: &str, repo_owner: &str) -> PrListItem {
    let own_head = own_head(
        pr.is_cross_repository,
        pr.head_repository_owner.as_ref(),
        repo_owner,
    );
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
        cross_repo: pr.is_cross_repository,
        own_head,
        session: None,
    }
}

/// `me` é o login do usuário no `gh`: decide `review_requested`. `repo_owner`
/// é o owner do repo consultado: decide `own_head`.
pub fn parse_list(stdout: &[u8], me: &str, repo_owner: &str) -> Result<Vec<PrListItem>, String> {
    let prs: Vec<GhListPr> = serde_json::from_slice(stdout)
        .map_err(|e| format!("resposta inesperada de `gh pr list`: {e}"))?;
    Ok(prs
        .into_iter()
        .map(|pr| list_item(pr, me, repo_owner))
        .collect())
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

fn gql_check(context: GqlContext) -> Option<PrCheck> {
    let rollup = match context {
        GqlContext::CheckRun {
            name,
            status,
            conclusion,
            started_at,
            completed_at,
            details_url,
            check_suite,
        } => GhRollup::CheckRun {
            name,
            status,
            conclusion,
            workflow_name: check_suite
                .and_then(|s| s.workflow_run)
                .and_then(|r| r.workflow)
                .map(|w| w.name),
            started_at,
            completed_at,
            details_url,
        },
        GqlContext::StatusContext {
            context,
            state,
            created_at,
            target_url,
        } => GhRollup::StatusContext {
            context,
            state,
            started_at: created_at,
            target_url,
        },
        GqlContext::Unknown => GhRollup::Unknown,
    };
    to_check(rollup)
}

/// Autor do commit: o login da conta, senão o nome do git.
fn commit_author(actor: Option<GqlGitActor>) -> String {
    let Some(actor) = actor else {
        return String::new();
    };
    actor
        .user
        .map(|u| u.login)
        .filter(|l| !l.is_empty())
        .or_else(|| non_empty(actor.name))
        .unwrap_or_default()
}

/// Uma thread de revisão: o trecho e a linha vêm da thread (a linha atual, ou a
/// original quando o comentário ficou desatualizado). Thread sem comentário
/// visível some.
fn gql_thread(thread: GqlThread) -> Option<PrThread> {
    let comments = nodes(thread.comments);
    let diff_hunk = comments.first()?.diff_hunk.clone();
    Some(PrThread {
        path: thread.path,
        line: thread.line.or(thread.original_line),
        diff_hunk,
        comments: comments
            .into_iter()
            .map(|c| PrThreadComment {
                author: login(c.author),
                body: c.body,
                created_at: c.created_at,
            })
            .collect(),
    })
}

/// Erro da resposta GraphQL, já em texto para o usuário.
fn gql_errors(errors: &[GqlError]) -> String {
    let joined = errors
        .iter()
        .map(|e| match &e.kind {
            Some(kind) if !kind.is_empty() => format!("{kind}: {}", e.message),
            _ => e.message.clone(),
        })
        .collect::<Vec<_>>()
        .join(" / ");
    if joined.trim().is_empty() {
        "resposta GraphQL sem o PR".into()
    } else {
        joined
    }
}

/// Resposta de [`DETAIL_QUERY`]: o detalhe completo (com threads) e a leitura
/// de limite da GraphQL, quando veio.
pub fn parse_detail_graphql(
    stdout: &[u8],
    repo_owner: &str,
) -> Result<(PrDetail, Option<RateLimit>), String> {
    let response: GqlResponse = serde_json::from_slice(stdout)
        .map_err(|e| format!("resposta inesperada da GraphQL do GitHub: {e}"))?;
    let data = response.data;
    let rate = data
        .as_ref()
        .and_then(|d| d.rate_limit.as_ref())
        .and_then(|r| rate::from_graphql(r.limit, r.remaining, &r.reset_at));
    let Some(pr) = data.and_then(|d| d.repository).and_then(|r| r.pull_request) else {
        return Err(gql_errors(&response.errors));
    };
    let checks = pr
        .head_commit
        .map(|c| c.nodes.into_iter().flatten().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .next_back()
        .and_then(|c| c.commit.status_check_rollup)
        .map(|r| nodes(r.contexts))
        .unwrap_or_default()
        .into_iter()
        .filter_map(gql_check)
        .collect();
    let detail = PrDetail {
        cross_repo: pr.is_cross_repository,
        own_head: own_head(
            pr.is_cross_repository,
            pr.head_repository_owner.as_ref(),
            repo_owner,
        ),
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
        commits: nodes(pr.commits)
            .into_iter()
            .map(|n| PrCommit {
                sha: n.commit.oid,
                subject: n.commit.message_headline,
                author: commit_author(nodes(n.commit.authors).into_iter().next()),
                authored_at: n.commit.authored_date,
            })
            .collect(),
        files: nodes(pr.files)
            .into_iter()
            .map(|f| PrFile {
                path: f.path,
                additions: f.additions,
                deletions: f.deletions,
            })
            .collect(),
        checks,
        reviews: nodes(pr.reviews)
            .into_iter()
            .map(|r| PrReview {
                author: login(r.author),
                state: review_state(&r.state),
                submitted_at: timestamp(r.submitted_at),
                body: r.body,
                commit: r.commit.map(|c| c.oid).filter(|o| !o.is_empty()),
            })
            .collect(),
        pending_reviewers: nodes(pr.review_requests)
            .into_iter()
            .filter_map(|r| r.requested_reviewer)
            .filter_map(reviewer)
            .collect(),
        threads: nodes(pr.review_threads)
            .into_iter()
            .filter_map(gql_thread)
            .collect(),
        comments: nodes(pr.comments).into_iter().map(thread_comment).collect(),
        review: review_decision(pr.review_decision),
        mergeable: mergeable(pr.mergeable.as_deref().unwrap_or_default()),
        merge_state: lower(pr.merge_state_status.as_deref().unwrap_or_default()),
        session: None,
        rate_limit: None,
    };
    Ok((detail, rate))
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

pub(super) fn github_remote(repo_root: &Path) -> Option<Remote> {
    let remote = remote_of(repo_root)?;
    if resolve_kind(&remote)? != ForgeKind::GitHub || cli_missing(ForgeKind::GitHub) {
        return None;
    }
    let valid = safe_segment(&remote.host, &['.'])
        && safe_segment(&remote.owner, &[])
        && safe_segment(&remote.repo, &['.']);
    valid.then_some(remote)
}

pub(super) fn repo_flag(remote: &Remote) -> String {
    format!("{}/{}/{}", remote.host, remote.owner, remote.repo)
}

pub(super) fn gh(repo_root: &Path, args: &[&str]) -> Result<Vec<u8>, AppError> {
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

/// `limit` do pedido, no intervalo aceito (padrão [`LIST_LIMIT`]).
pub fn clamp_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(LIST_LIMIT).clamp(1, LIST_LIMIT_MAX)
}

fn list_args(repo: &str, filter: &[&str], limit: u32) -> Vec<String> {
    let mut args: Vec<String> = ["pr", "list", "--repo", repo, "--limit"]
        .map(String::from)
        .to_vec();
    args.push(limit.to_string());
    args.extend(filter.iter().map(|f| f.to_string()));
    args.extend(["--json".into(), LIST_FIELDS.into()]);
    args
}

/// Há mais PRs do que vieram: a lista pedida ou a de abertos (contagens)
/// bateu no `limit`.
pub fn is_capped(items: usize, open: usize, limit: u32) -> bool {
    let limit = limit as usize;
    items >= limit || open >= limit
}

fn list(
    repo_root: &Path,
    remote: &Remote,
    me: &str,
    filter: &[&str],
    limit: u32,
) -> Result<Vec<PrListItem>, AppError> {
    let args = list_args(&repo_flag(remote), filter, limit);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    parse_list(&gh(repo_root, &args)?, me, &remote.owner).map_err(cli_failed)
}

fn fetch_page(
    repo_root: &Path,
    remote: &Remote,
    filter: PrFilter,
    limit: u32,
) -> Result<PrPage, AppError> {
    let me = viewer_login(repo_root, remote)?;
    let open = list(repo_root, remote, &me, &["--state", "open"], limit)?;
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
            list(repo_root, remote, &me, &["--author", "@me"], limit)?,
            Vec::new(),
        ),
        PrFilter::Review => (
            list(
                repo_root,
                remote,
                &me,
                &["--search", "review-requested:@me"],
                limit,
            )?,
            Vec::new(),
        ),
        PrFilter::Closed => (
            list(repo_root, remote, &me, &["--state", "closed"], limit)?,
            Vec::new(),
        ),
    };
    Ok(PrPage {
        counts: counts(&open, &me),
        capped: is_capped(items.len(), open.len(), limit),
        items,
        recent_merged,
        repo: remote.project(),
        forge: ForgeKind::GitHub,
        fetched_at: now_iso(),
        rate_limit: None,
    })
}

/// Erro do `gh` que diz "limite estourado" vira `forge.rate_limited` e
/// marca o balde como zerado até descobrir o reset.
fn rate_aware(host: &str, resource: &str, err: AppError) -> AppError {
    let limited = err.code == "forge.rate_limited"
        || err
            .params
            .get("detail")
            .is_some_and(|d| rate::is_rate_limit_error(d));
    if !limited {
        return err;
    }
    if rate::exhausted(host, resource).is_none() {
        rate::record_exhausted(host, resource);
    }
    AppError::new("forge.rate_limited")
}

pub fn detail_args(remote: &Remote, number: u64) -> Vec<String> {
    vec![
        "api".into(),
        "graphql".into(),
        "--hostname".into(),
        remote.host.clone(),
        "-f".into(),
        format!("query={DETAIL_QUERY}"),
        "-f".into(),
        format!("owner={}", remote.owner),
        "-f".into(),
        format!("name={}", remote.repo),
        "-F".into(),
        format!("number={number}"),
    ]
}

/// Uma chamada GraphQL. O `gh api graphql` sai com 1 quando a resposta tem
/// `errors` (PR inexistente), mas o JSON vem no stdout: decide-se por ele.
fn fetch_detail(repo_root: &Path, remote: &Remote, number: u64) -> Result<PrDetail, AppError> {
    let args = detail_args(remote, number);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = exec::run(github::CLI, &args, repo_root, None, NETWORK_TIMEOUT)
        .map_err(|e| rate_aware(&remote.host, "graphql", cli_failed(e)))?;
    if out.stdout.trim_ascii().is_empty() {
        return Err(rate_aware(
            &remote.host,
            "graphql",
            cli_failed(out.stderr_message()),
        ));
    }
    let (detail, reading) = parse_detail_graphql(&out.stdout, &remote.owner)
        .map_err(|e| rate_aware(&remote.host, "graphql", cli_failed(e)))?;
    if let Some(reading) = reading {
        rate::record(&remote.host, reading);
    }
    Ok(detail)
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// --- cache (stale-while-revalidate) --------------------------------------------
//
// Até 30s o cache responde sozinho. Depois disso, ele ainda responde na hora
// (a tela não espera a rede), e uma revalidação vai para segundo plano — no
// máximo uma por chave. Se o dado mudou, `notify` avisa a UI, que relê o cache.
// Com o limite da API baixo, a revalidação não sai até o reset. Nada disso
// roda sozinho: só um pedido da UI (tela de Git aberta) dispara revalidação.

type Cache<K, V> = Mutex<HashMap<K, (Instant, V)>>;

/// O que mudou depois de uma revalidação em segundo plano. `number` vazio é a
/// lista; com número, o detalhe daquele PR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ForgeChange {
    /// `owner/repo`, como em [`PrPage::repo`].
    pub repo: String,
    pub number: Option<u64>,
}

/// Quem avisa a UI (no app, um `emit` do Tauri).
pub type Notify = std::sync::Arc<dyn Fn(ForgeChange) + Send + Sync>;

fn page_cache() -> &'static Cache<(String, PrFilter, u32), PrPage> {
    static CACHE: std::sync::OnceLock<Cache<(String, PrFilter, u32), PrPage>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn detail_cache() -> &'static Cache<(String, u64), PrDetail> {
    static CACHE: std::sync::OnceLock<Cache<(String, u64), PrDetail>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, PartialEq, Eq)]
enum Lookup<V> {
    /// Dentro do TTL.
    Fresh(V),
    /// Venceu: serve e revalida.
    Stale(V),
    Miss,
}

fn lookup<K: std::hash::Hash + Eq, V: Clone>(cache: &Cache<K, V>, key: &K) -> Lookup<V> {
    match cache.lock().get(key) {
        Some((at, v)) if at.elapsed() <= CACHE_TTL => Lookup::Fresh(v.clone()),
        Some((_, v)) => Lookup::Stale(v.clone()),
        None => Lookup::Miss,
    }
}

/// Guarda `value`; devolve se mudou em relação ao que havia (`same` decide).
fn store<K: std::hash::Hash + Eq, V>(
    cache: &Cache<K, V>,
    key: K,
    value: V,
    same: impl Fn(&V, &V) -> bool,
) -> bool {
    let mut cache = cache.lock();
    let changed = cache.get(&key).is_none_or(|(_, old)| !same(old, &value));
    cache.insert(key, (Instant::now(), value));
    changed
}

/// Revalidação que não saiu (limite baixo, erro): adia a próxima por um TTL
/// em vez de tentar de novo a cada pedido.
fn touch<K: std::hash::Hash + Eq, V>(cache: &Cache<K, V>, key: &K) {
    if let Some(entry) = cache.lock().get_mut(key) {
        entry.0 = Instant::now();
    }
}

/// A página nova é igual à velha? `fetched_at` não conta.
pub fn same_page(old: &PrPage, new: &PrPage) -> bool {
    old.items == new.items
        && old.recent_merged == new.recent_merged
        && old.counts == new.counts
        && old.capped == new.capped
        && old.repo == new.repo
}

fn inflight() -> &'static Mutex<std::collections::HashSet<String>> {
    static SET: std::sync::OnceLock<Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// Uma revalidação por `tag` de cada vez, numa thread própria.
fn revalidate_in_background(tag: String, job: impl FnOnce() + Send + 'static) {
    if !inflight().lock().insert(tag.clone()) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("forge-swr".into())
        .spawn(move || {
            job();
            inflight().lock().remove(&tag);
        });
    if spawned.is_err() {
        // Sem thread, sem revalidação: o próximo pedido tenta de novo.
        inflight().lock().clear();
    }
}

/// Revalidar agora? Sem leitura recente do balde GraphQL, consulta
/// `/rate_limit` (de graça) antes de decidir.
fn may_revalidate(repo_root: &Path, host: &str) -> bool {
    if rate::unknown(host, "graphql") {
        super::conditional::refresh_rate_limit(repo_root, host);
    }
    !rate::back_off(host, "graphql")
}

fn exhausted_error(host: &str) -> Option<AppError> {
    rate::exhausted(host, "graphql").map(|r| {
        AppError::new("forge.rate_limited").with(
            "reset_at",
            chrono::DateTime::from_timestamp(r.reset, 0)
                .unwrap_or_default()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        )
    })
}

/// `None`: a forja não tem lista rica aqui (GitLab, sem forja, sem `gh`).
/// `fresh=false` aceita o cache (velho ou não, ver o topo desta seção);
/// `fresh=true` busca agora, a não ser que o limite esteja zerado — aí serve
/// o cache, se houver. `limit` passa por [`clamp_limit`].
pub fn page(
    repo_root: &Path,
    filter: PrFilter,
    limit: Option<u32>,
    fresh: bool,
    notify: Notify,
) -> Result<Option<PrPage>, AppError> {
    let Some(remote) = github_remote(repo_root) else {
        return Ok(None);
    };
    let limit = clamp_limit(limit);
    let key = (repo_flag(&remote), filter, limit);
    let served = match lookup(page_cache(), &key) {
        Lookup::Fresh(v) if !fresh => v,
        Lookup::Stale(v) if !fresh => {
            let root = repo_root.to_path_buf();
            let remote = remote.clone();
            let key = key.clone();
            revalidate_in_background(format!("page:{key:?}"), move || {
                if !may_revalidate(&root, &remote.host) {
                    touch(page_cache(), &key);
                    return;
                }
                match fetch_page(&root, &remote, filter, limit) {
                    Ok(new) => {
                        let repo = new.repo.clone();
                        if store(page_cache(), key, new, same_page) {
                            notify(ForgeChange { repo, number: None });
                        }
                    }
                    Err(_) => touch(page_cache(), &key),
                }
            });
            v
        }
        cached => {
            if let Some(err) = exhausted_error(&remote.host) {
                match cached {
                    Lookup::Fresh(v) | Lookup::Stale(v) => v,
                    Lookup::Miss => return Err(err),
                }
            } else {
                let v = fetch_page(repo_root, &remote, filter, limit)
                    .map_err(|e| rate_aware(&remote.host, "graphql", e))?;
                store(page_cache(), key, v.clone(), same_page);
                v
            }
        }
    };
    Ok(Some(PrPage {
        rate_limit: rate::low_for_ui(&remote.host),
        ..served
    }))
}

pub fn detail(
    repo_root: &Path,
    number: u64,
    fresh: bool,
    notify: Notify,
) -> Result<Option<PrDetail>, AppError> {
    if number == 0 {
        return Err(cli_failed("número de PR inválido"));
    }
    let Some(remote) = github_remote(repo_root) else {
        return Ok(None);
    };
    let key = (repo_flag(&remote), number);
    let served = match lookup(detail_cache(), &key) {
        Lookup::Fresh(v) if !fresh => v,
        Lookup::Stale(v) if !fresh => {
            let root = repo_root.to_path_buf();
            let remote = remote.clone();
            let key = key.clone();
            revalidate_in_background(format!("detail:{key:?}"), move || {
                if !may_revalidate(&root, &remote.host) {
                    touch(detail_cache(), &key);
                    return;
                }
                match fetch_detail(&root, &remote, number) {
                    Ok(new) => {
                        if store(detail_cache(), key, new, PartialEq::eq) {
                            notify(ForgeChange {
                                repo: remote.project(),
                                number: Some(number),
                            });
                        }
                    }
                    Err(_) => touch(detail_cache(), &key),
                }
            });
            v
        }
        cached => {
            if let Some(err) = exhausted_error(&remote.host) {
                match cached {
                    Lookup::Fresh(v) | Lookup::Stale(v) => v,
                    Lookup::Miss => return Err(err),
                }
            } else {
                let v = fetch_detail(repo_root, &remote, number)?;
                store(detail_cache(), key, v.clone(), PartialEq::eq);
                v
            }
        }
    };
    Ok(Some(PrDetail {
        rate_limit: rate::low_for_ui(&remote.host),
        ..served
    }))
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
pub(super) fn action_remote(repo_root: &Path) -> Result<Remote, AppError> {
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

pub(super) fn gh_write(
    repo_root: &Path,
    args: &[String],
    stdin: Option<&[u8]>,
) -> Result<(), AppError> {
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
    page_cache().lock().retain(|(r, _, _), _| r != repo);
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
    const DETAIL: &str = include_str!("fixtures/gh_graphql_pr_detail.json");

    fn parse_detail(json: &[u8], owner: &str) -> Result<PrDetail, String> {
        parse_detail_graphql(json, owner).map(|(d, _)| d)
    }

    /// PR mínimo na resposta GraphQL, para os testes de fork.
    fn gql_pr(fields: &str) -> Vec<u8> {
        format!(r#"{{"data":{{"repository":{{"pullRequest":{{"number":9,{fields}}}}}}}}}"#)
            .into_bytes()
    }

    #[test]
    fn list_maps_state_draft_and_merged() {
        let items = parse_list(LIST.as_bytes(), "eu-mesmo", "exemplo").unwrap();
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
        let items = parse_list(LIST.as_bytes(), "eu-mesmo", "exemplo").unwrap();
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
        let items = parse_list(LIST.as_bytes(), "eu-mesmo", "exemplo").unwrap();
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
        let items = parse_list(LIST.as_bytes(), "eu-mesmo", "exemplo").unwrap();
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
        let items = parse_list(LIST.as_bytes(), "eu-mesmo", "exemplo").unwrap();
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
        let d = parse_detail(DETAIL.as_bytes(), "exemplo").unwrap();
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
        let d = parse_detail(DETAIL.as_bytes(), "exemplo").unwrap();
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
    fn threads_come_from_the_same_graphql_call() {
        let d = parse_detail(DETAIL.as_bytes(), "exemplo").unwrap();
        assert_eq!(d.threads.len(), 2, "thread sem comentário visível some");
        assert_eq!(d.threads[0].path, "src/tela.ts");
        assert_eq!(d.threads[0].line, Some(10));
        assert!(d.threads[0].diff_hunk.starts_with("@@"));
        let bodies: Vec<&str> = d.threads[0]
            .comments
            .iter()
            .map(|c| c.body.as_str())
            .collect();
        assert_eq!(
            bodies,
            vec!["por que isso?", "explico abaixo", "conta apagada"]
        );
        assert_eq!(
            d.threads[0].comments[2].author, "",
            "usuário apagado não derruba"
        );
        // Comentário desatualizado: cai para `originalLine`.
        assert_eq!(d.threads[1].line, Some(7));
        assert_eq!(d.threads[1].comments.len(), 1);
    }

    #[test]
    fn graphql_brings_the_rate_limit_reading() {
        let (_, rate) = parse_detail_graphql(DETAIL.as_bytes(), "exemplo").unwrap();
        let rate = rate.expect("rateLimit");
        assert_eq!(rate.resource, "graphql");
        assert_eq!(rate.remaining, 4321);
        assert_eq!(rate.reset, 1_900_000_000);
    }

    #[test]
    fn graphql_errors_become_the_message() {
        let json = br#"{"data":{"repository":{"pullRequest":null}},
          "errors":[{"type":"NOT_FOUND","message":"Could not resolve to a PullRequest with the number of 7."}]}"#;
        let err = parse_detail(json, "exemplo").unwrap_err();
        assert!(err.contains("NOT_FOUND"), "{err}");
        let limited = br#"{"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded for user ID 1."}]}"#;
        let err = parse_detail(limited, "exemplo").unwrap_err();
        assert!(rate::is_rate_limit_error(&err), "{err}");
        assert!(parse_detail(b"{}", "").is_err());
    }

    #[test]
    fn detail_tolerates_null_connections_and_nodes() {
        let json = gql_pr(
            r#""files":null,"headCommit":{"nodes":[{"commit":{"statusCheckRollup":null}}]},
            "reviews":{"nodes":[null]},"reviewRequests":null,"mergeable":null"#,
        );
        let d = parse_detail(&json, "").unwrap();
        assert!(d.files.is_empty() && d.checks.is_empty() && d.reviews.is_empty());
        assert_eq!(d.mergeable, PrMergeable::Unknown);
    }

    #[test]
    fn detail_args_send_the_query_and_typed_number() {
        let remote = Remote {
            host: "github.com".into(),
            owner: "exemplo".into(),
            repo: "app".into(),
        };
        let args = detail_args(&remote, 42);
        assert_eq!(&args[..4], ["api", "graphql", "--hostname", "github.com"]);
        assert!(args.contains(&"owner=exemplo".to_string()));
        assert!(args.contains(&"name=app".to_string()));
        let n = args.iter().position(|a| a == "number=42").unwrap();
        assert_eq!(args[n - 1], "-F", "número vai tipado (Int), não string");
    }

    #[test]
    fn parsers_reject_garbage() {
        assert!(parse_list(b"nao sou json", "", "").is_err());
        assert!(parse_detail(b"[]", "").is_err());
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
            let noop: Notify = std::sync::Arc::new(|_| {});
            assert_eq!(
                page(dir.path(), PrFilter::Open, None, false, noop.clone()).unwrap(),
                None
            );
            assert_eq!(detail(dir.path(), 1, false, noop).unwrap(), None);
        }
    }

    #[test]
    fn fork_pr_with_a_session_branch_name_is_not_own_head() {
        // Mesmo nome de branch de uma sessão; só o primeiro é do próprio repo.
        let json = br#"[
          {"number": 1, "headRefName": "tyba/x-ab12", "isCrossRepository": false,
           "headRepositoryOwner": {"id": "U_1", "login": "Exemplo"}},
          {"number": 2, "headRefName": "tyba/x-ab12", "isCrossRepository": true,
           "headRepositoryOwner": {"id": "U_2", "login": "atacante"}},
          {"number": 3, "headRefName": "tyba/x-ab12", "isCrossRepository": false,
           "headRepositoryOwner": {"id": "U_2", "login": "atacante"}},
          {"number": 4, "headRefName": "tyba/x-ab12", "isCrossRepository": true,
           "headRepositoryOwner": {"id": "U_1", "login": "exemplo"}},
          {"number": 5, "headRefName": "tyba/x-ab12"}
        ]"#;
        let items = parse_list(json, "", "exemplo").unwrap();
        let own: Vec<bool> = items.iter().map(|i| i.own_head).collect();
        assert_eq!(own, [true, false, false, false, false]);
        let cross: Vec<bool> = items.iter().map(|i| i.cross_repo).collect();
        assert_eq!(cross, [false, true, false, true, false]);
        let v = serde_json::to_value(&items[1]).unwrap();
        assert_eq!(v["cross_repo"], true);
        assert!(v.get("own_head").is_none(), "own_head não vai para a UI");

        let fork =
            gql_pr(r#""isCrossRepository": true, "headRepositoryOwner": {"login": "atacante"}"#);
        let d = parse_detail(&fork, "exemplo").unwrap();
        assert!(d.cross_repo && !d.own_head);
        let own =
            gql_pr(r#""isCrossRepository": false, "headRepositoryOwner": {"login": "exemplo"}"#);
        assert!(parse_detail(&own, "exemplo").unwrap().own_head);
        // Owner do repo desconhecido: não liga.
        assert!(!parse_detail(&own, "").unwrap().own_head);
    }

    #[test]
    fn list_and_detail_ask_gh_for_the_fork_fields() {
        let fields: Vec<&str> = LIST_FIELDS.split(',').collect();
        assert!(fields.contains(&"isCrossRepository"));
        assert!(fields.contains(&"headRepositoryOwner"));
        assert!(DETAIL_QUERY.contains("isCrossRepository"));
        assert!(DETAIL_QUERY.contains("headRepositoryOwner { login }"));
    }

    #[test]
    fn limit_goes_to_gh_pr_list_and_is_clamped() {
        assert_eq!(clamp_limit(None), 100);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(300)), 300);
        assert_eq!(clamp_limit(Some(5000)), 1000);
        let args = list_args(REPO, &["--author", "@me"], 300);
        assert_eq!(
            args,
            [
                "pr",
                "list",
                "--repo",
                REPO,
                "--limit",
                "300",
                "--author",
                "@me",
                "--json",
                LIST_FIELDS
            ]
        );
    }

    #[test]
    fn capped_when_the_list_or_the_open_list_hits_the_limit() {
        assert!(!is_capped(99, 99, 100));
        assert!(is_capped(100, 100, 100), "a lista pedida bateu no limite");
        assert!(is_capped(3, 100, 100), "a de abertos (contagens) bateu");
        assert!(is_capped(200, 40, 200));
        assert!(!is_capped(0, 0, 1));
    }

    #[test]
    fn detail_rejects_number_zero() {
        assert!(detail(Path::new("/"), 0, false, std::sync::Arc::new(|_| {})).is_err());
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
            capped: false,
            repo: "invalida/teste".into(),
            forge: ForgeKind::GitHub,
            fetched_at: String::new(),
            rate_limit: None,
        };
        let now = Instant::now();
        let detail = parse_detail(DETAIL.as_bytes(), "exemplo").unwrap();
        {
            let mut pages = page_cache().lock();
            pages.insert((repo.into(), PrFilter::Open, 100), (now, page.clone()));
            pages.insert((repo.into(), PrFilter::Open, 200), (now, page.clone()));
            pages.insert((repo.into(), PrFilter::Mine, 100), (now, page.clone()));
            pages.insert(
                ("github.com/outro/repo".into(), PrFilter::Open, 100),
                (now, page),
            );
            let mut details = detail_cache().lock();
            details.insert((repo.into(), 42), (now, detail.clone()));
            details.insert((repo.into(), 43), (now, detail));
        }
        invalidate(repo, 42);
        let pages = page_cache().lock();
        assert!(!pages.keys().any(|(r, _, _)| r == repo));
        assert!(pages.contains_key(&("github.com/outro/repo".to_string(), PrFilter::Open, 100)));
        let details = detail_cache().lock();
        assert!(!details.contains_key(&(repo.to_string(), 42)));
        assert!(details.contains_key(&(repo.to_string(), 43)));
    }

    fn empty_page(fetched_at: &str) -> PrPage {
        PrPage {
            items: Vec::new(),
            recent_merged: Vec::new(),
            counts: PrCounts::default(),
            capped: false,
            repo: "exemplo/app".into(),
            forge: ForgeKind::GitHub,
            fetched_at: fetched_at.into(),
            rate_limit: None,
        }
    }

    #[test]
    fn cache_is_fresh_then_stale_then_touched_back() {
        let cache: Cache<&str, u32> = Mutex::new(HashMap::new());
        assert_eq!(lookup(&cache, &"k"), Lookup::Miss);
        assert!(store(&cache, "k", 1, PartialEq::eq), "primeira vez muda");
        assert_eq!(lookup(&cache, &"k"), Lookup::Fresh(1));
        let old = Instant::now()
            .checked_sub(CACHE_TTL + std::time::Duration::from_secs(1))
            .unwrap();
        cache.lock().insert("k", (old, 1));
        assert_eq!(lookup(&cache, &"k"), Lookup::Stale(1), "velho ainda serve");
        touch(&cache, &"k");
        assert_eq!(lookup(&cache, &"k"), Lookup::Fresh(1), "adiado por um TTL");
        assert!(!store(&cache, "k", 1, PartialEq::eq), "igual não avisa");
        assert!(store(&cache, "k", 2, PartialEq::eq));
    }

    #[test]
    fn page_change_ignores_fetched_at() {
        let a = empty_page("2026-01-01T00:00:00Z");
        let b = empty_page("2026-01-01T00:05:00Z");
        assert!(same_page(&a, &b));
        let mut c = b.clone();
        c.counts.open = 1;
        assert!(!same_page(&a, &c));
    }

    #[test]
    fn one_background_revalidation_per_key() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let runs = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let r = runs.clone();
        revalidate_in_background("teste:swr".into(), move || {
            r.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            started_tx.send(()).unwrap();
            rx.recv().unwrap();
        });
        started_rx.recv().unwrap();
        let r = runs.clone();
        revalidate_in_background("teste:swr".into(), move || {
            r.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        tx.send(()).unwrap();
        for _ in 0..200 {
            if !inflight().lock().contains("teste:swr") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!inflight().lock().contains("teste:swr"));
    }

    #[test]
    fn rate_limit_errors_from_gh_become_typed() {
        let host = "prs-rate.example";
        rate::reset_for_tests(host);
        let e = rate_aware(
            host,
            "graphql",
            cli_failed("HTTP 403: API rate limit exceeded"),
        );
        assert_eq!(e.code, "forge.rate_limited");
        assert!(exhausted_error(host).is_some());
        let other = rate_aware(host, "graphql", cli_failed("HTTP 404"));
        assert_eq!(other.code, "forge.cli_failed");
        rate::reset_for_tests(host);
    }

    #[test]
    fn change_event_uses_the_ts_contract_names() {
        let v = serde_json::to_value(ForgeChange {
            repo: "exemplo/app".into(),
            number: Some(7),
        })
        .unwrap();
        assert_eq!(v, serde_json::json!({"repo": "exemplo/app", "number": 7}));
        let v = serde_json::to_value(empty_page("")).unwrap();
        assert_eq!(v["rate_limit"], serde_json::Value::Null);
    }
}
