use std::path::Path;

use serde::Deserialize;

use super::exec::{self, LOCAL_TIMEOUT, NETWORK_TIMEOUT};
use super::remote::Remote;
use super::{
    is_auth_error, CheckRun, PullRequest, ReviewComment, RunLog, WorkflowJob, WorkflowRun,
    WorkflowStep,
};
use crate::error::AppError;

pub const CLI: &str = "gh";

pub(super) fn cli_failed(detail: impl Into<String>) -> AppError {
    let detail = detail.into();
    if is_auth_error(&detail) {
        AppError::new("forge.not_authenticated").with("cli", CLI)
    } else {
        AppError::new("forge.cli_failed")
            .with("cli", CLI)
            .with("detail", detail)
    }
}

fn create_failed(detail: impl Into<String>) -> AppError {
    let detail = detail.into();
    if is_auth_error(&detail) {
        AppError::new("forge.not_authenticated").with("cli", CLI)
    } else {
        AppError::new("forge.create_failed").with("detail", detail)
    }
}

const PR_FIELDS: &str = "number,title,url,state,statusCheckRollup";

const RUN_FIELDS: &str = "databaseId,name,status,conclusion,url,headBranch,event,createdAt";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRun {
    database_id: u64,
    name: String,
    status: String,
    conclusion: Option<String>,
    url: String,
    head_branch: String,
    event: String,
    created_at: String,
}

impl From<GhRun> for WorkflowRun {
    fn from(r: GhRun) -> Self {
        WorkflowRun {
            id: r.database_id,
            name: r.name,
            // O `gh` já entrega minúsculo, mas normalizar aqui é o que garante
            // que a UI possa comparar sem adivinhar — mesmo contrato do CheckRun.
            status: r.status.to_lowercase(),
            conclusion: r
                .conclusion
                .filter(|c| !c.is_empty())
                .map(|c| c.to_lowercase()),
            url: r.url,
            head_branch: r.head_branch,
            event: r.event,
            created_at: r.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GhJobs {
    jobs: Vec<GhJob>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhJob {
    database_id: u64,
    name: String,
    status: String,
    conclusion: Option<String>,
    url: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
    #[serde(default)]
    steps: Vec<GhStep>,
}

#[derive(Debug, Deserialize)]
struct GhStep {
    name: String,
    status: String,
    #[serde(default)]
    conclusion: Option<String>,
}

fn job_conclusion(conclusion: Option<String>) -> Option<String> {
    conclusion
        .filter(|c| !c.is_empty())
        .map(|c| c.to_lowercase())
}

/// O `gh` serializa horário que ainda não aconteceu como o zero do Go
/// (`0001-01-01T00:00:00Z`), não como `null`.
fn job_time(value: Option<String>) -> Option<String> {
    value.filter(|t| !t.is_empty() && !t.starts_with("0001-"))
}

impl From<GhJob> for WorkflowJob {
    fn from(j: GhJob) -> Self {
        WorkflowJob {
            id: j.database_id,
            name: j.name,
            status: j.status.to_lowercase(),
            conclusion: job_conclusion(j.conclusion),
            url: j.url,
            started_at: job_time(j.started_at),
            completed_at: job_time(j.completed_at),
            steps: j
                .steps
                .into_iter()
                .map(|s| WorkflowStep {
                    name: s.name,
                    status: s.status.to_lowercase(),
                    conclusion: job_conclusion(s.conclusion),
                })
                .collect(),
        }
    }
}

pub fn parse_runs(stdout: &[u8]) -> Result<Vec<WorkflowRun>, String> {
    let runs: Vec<GhRun> = serde_json::from_slice(stdout)
        .map_err(|e| format!("resposta inesperada de `gh run list`: {e}"))?;
    Ok(runs.into_iter().map(Into::into).collect())
}

/// Run da REST (`GET repos/{o}/{r}/actions/runs`), o que o GET condicional traz.
#[derive(Debug, Deserialize)]
struct ApiRun {
    id: u64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    head_branch: Option<String>,
    #[serde(default)]
    event: String,
    #[serde(default)]
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct ApiRuns {
    #[serde(default)]
    workflow_runs: Vec<ApiRun>,
}

/// Mesmo contrato de [`parse_runs`], a partir da REST.
pub fn parse_api_runs(body: &[u8]) -> Result<Vec<WorkflowRun>, String> {
    let runs: ApiRuns = serde_json::from_slice(body)
        .map_err(|e| format!("resposta inesperada da API de runs do GitHub: {e}"))?;
    Ok(runs
        .workflow_runs
        .into_iter()
        .map(|r| {
            GhRun {
                database_id: r.id,
                name: r.name.unwrap_or_default(),
                status: r.status.unwrap_or_default(),
                conclusion: r.conclusion,
                url: r.html_url,
                head_branch: r.head_branch.unwrap_or_default(),
                event: r.event,
                created_at: r.created_at,
            }
            .into()
        })
        .collect())
}

pub fn parse_jobs(stdout: &[u8]) -> Result<Vec<WorkflowJob>, String> {
    let out: GhJobs = serde_json::from_slice(stdout)
        .map_err(|e| format!("resposta inesperada de `gh run view`: {e}"))?;
    Ok(out.jobs.into_iter().map(Into::into).collect())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPullRequest {
    number: u64,
    title: String,
    url: String,
    state: String,
    #[serde(default)]
    status_check_rollup: Option<Vec<GhCheck>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum GhCheck {
    CheckRun {
        name: String,
        #[serde(default)]
        status: String,
        #[serde(default)]
        conclusion: Option<String>,
    },
    StatusContext {
        context: String,
        #[serde(default)]
        state: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct GhUser {
    #[serde(default)]
    login: String,
}

#[derive(Debug, Deserialize)]
struct GhComment {
    id: u64,
    #[serde(default)]
    body: String,
    #[serde(default)]
    user: Option<GhUser>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    original_line: Option<u32>,
    #[serde(default)]
    html_url: Option<String>,
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn non_empty(value: &str) -> Option<String> {
    let value = normalize(value);
    (!value.is_empty()).then_some(value)
}

impl From<GhCheck> for Option<CheckRun> {
    fn from(check: GhCheck) -> Self {
        match check {
            GhCheck::CheckRun {
                name,
                status,
                conclusion,
            } => Some(CheckRun {
                name,
                status: normalize(&status),
                conclusion: conclusion.as_deref().and_then(non_empty),
            }),
            GhCheck::StatusContext { context, state } => Some(CheckRun {
                name: context,
                status: "completed".into(),
                conclusion: non_empty(&state),
            }),
            GhCheck::Unknown => None,
        }
    }
}

impl From<GhPullRequest> for PullRequest {
    fn from(pr: GhPullRequest) -> Self {
        PullRequest {
            number: pr.number,
            title: pr.title,
            url: pr.url,
            state: normalize(&pr.state),
            checks: pr
                .status_check_rollup
                .unwrap_or_default()
                .into_iter()
                .filter_map(Into::into)
                .collect(),
        }
    }
}

pub fn parse_pr_list(stdout: &[u8]) -> Result<Vec<PullRequest>, String> {
    let prs: Vec<GhPullRequest> = serde_json::from_slice(stdout)
        .map_err(|e| format!("resposta inesperada de `gh pr list`: {e}"))?;
    Ok(prs.into_iter().map(Into::into).collect())
}

pub fn parse_pr(stdout: &[u8]) -> Result<PullRequest, String> {
    let pr: GhPullRequest = serde_json::from_slice(stdout)
        .map_err(|e| format!("resposta inesperada de `gh pr view`: {e}"))?;
    Ok(pr.into())
}

pub fn parse_comments(stdout: &[u8]) -> Result<Vec<ReviewComment>, String> {
    let comments: Vec<GhComment> = serde_json::from_slice(stdout)
        .map_err(|e| format!("resposta inesperada da API do GitHub: {e}"))?;
    Ok(comments
        .into_iter()
        .map(|c| ReviewComment {
            id: c.id.to_string(),
            author: c.user.map(|u| u.login).unwrap_or_default(),
            body: c.body,
            path: c.path,
            line: c.line.or(c.original_line),
            url: c.html_url,
        })
        .collect())
}

pub fn pr_number_from_output(stdout: &[u8]) -> Option<u64> {
    String::from_utf8_lossy(stdout)
        .lines()
        .rev()
        .find_map(|line| {
            let line = line.trim();
            let (_, tail) = line.rsplit_once("/pull/")?;
            let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().ok()
        })
}

pub fn installed() -> bool {
    exec::run(CLI, &["--version"], Path::new("/"), None, LOCAL_TIMEOUT)
        .map(|out| out.ok)
        .unwrap_or(false)
}

pub fn authenticated(repo_root: &Path, remote: &Remote) -> bool {
    exec::run(
        CLI,
        &["auth", "status", "--hostname", &remote.host],
        repo_root,
        None,
        NETWORK_TIMEOUT,
    )
    .map(|out| out.ok)
    .unwrap_or(false)
}

pub fn web_create_url(remote: &Remote, branch: &str) -> String {
    format!(
        "{}/compare/{}?expand=1",
        remote.base_url(),
        super::encode_branch(branch)
    )
}

pub fn create_pr(worktree: &Path, title: &str, body: &str) -> Result<PullRequest, AppError> {
    let out = exec::run(
        CLI,
        &["pr", "create", "--title", title, "--body-file", "-"],
        worktree,
        Some(body.as_bytes()),
        NETWORK_TIMEOUT,
    )
    .map_err(create_failed)?;
    if !out.ok {
        return Err(create_failed(out.stderr_message()));
    }
    let number =
        pr_number_from_output(&out.stdout).ok_or_else(|| create_failed(out.stderr_message()))?;
    pr_by_number(worktree, number)
}

pub fn pr_by_number(worktree: &Path, number: u64) -> Result<PullRequest, AppError> {
    let number = number.to_string();
    let out = exec::run(
        CLI,
        &["pr", "view", &number, "--json", PR_FIELDS],
        worktree,
        None,
        NETWORK_TIMEOUT,
    )
    .map_err(cli_failed)?;
    if !out.ok {
        return Err(cli_failed(out.stderr_message()));
    }
    parse_pr(&out.stdout).map_err(cli_failed)
}

pub fn pr_for_branch(worktree: &Path, branch: &str) -> Result<Option<PullRequest>, AppError> {
    let out = exec::run(
        CLI,
        &[
            "pr", "list", "--head", branch, "--state", "all", "--limit", "1", "--json", PR_FIELDS,
        ],
        worktree,
        None,
        NETWORK_TIMEOUT,
    )
    .map_err(cli_failed)?;
    if !out.ok {
        return Err(cli_failed(out.stderr_message()));
    }
    Ok(parse_pr_list(&out.stdout)
        .map_err(cli_failed)?
        .into_iter()
        .next())
}

/// Runs pela REST com GET condicional: o poll da CI só gasta limite quando
/// algum run mudou. Sem remote GitHub válido, cai no `gh run list`. Com o
/// balde `core` zerado, recusa sem chamar o `gh` (cada 403 a mais alonga o
/// bloqueio secundário).
fn core_available(host: &str) -> Result<(), AppError> {
    match super::rate::exhausted(host, "core") {
        Some(_) => Err(AppError::new("forge.rate_limited")),
        None => Ok(()),
    }
}

pub fn run_list(repo: &Path, limit: u32) -> Result<Vec<WorkflowRun>, AppError> {
    if let Some(remote) = super::prs::github_remote(repo) {
        core_available(&remote.host)?;
        let endpoint = format!(
            "repos/{}/{}/actions/runs?per_page={}",
            remote.owner,
            remote.repo,
            limit.clamp(1, 50)
        );
        let body = super::conditional::get(repo, &remote.host, &endpoint)?;
        return parse_api_runs(&body).map_err(cli_failed);
    }
    let limit = limit.clamp(1, 50).to_string();
    let out = exec::run(
        CLI,
        &["run", "list", "--limit", &limit, "--json", RUN_FIELDS],
        repo,
        None,
        NETWORK_TIMEOUT,
    )
    .map_err(cli_failed)?;
    if !out.ok {
        return Err(cli_failed(out.stderr_message()));
    }
    parse_runs(&out.stdout).map_err(cli_failed)
}

/// argv do `gh run view`: com `--repo` do `origin` quando há remote GitHub
/// válido — o mesmo repo do `run_list` (num clone de fork, o `gh` sozinho
/// pode escolher o upstream).
fn run_jobs_args(run_id: u64, repo_flag: Option<String>) -> Vec<String> {
    let mut args: Vec<String> = vec!["run".into(), "view".into(), run_id.to_string()];
    if let Some(repo) = repo_flag {
        args.extend(["--repo".into(), repo]);
    }
    args.extend(["--json".into(), "jobs".into()]);
    args
}

pub fn run_jobs(repo: &Path, run_id: u64) -> Result<Vec<WorkflowJob>, AppError> {
    let args = run_jobs_args(
        run_id,
        super::prs::github_remote(repo).map(|r| super::prs::repo_flag(&r)),
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = exec::run(CLI, &args, repo, None, NETWORK_TIMEOUT).map_err(cli_failed)?;
    if !out.ok {
        return Err(cli_failed(out.stderr_message()));
    }
    parse_jobs(&out.stdout).map_err(cli_failed)
}

/// Teto do log devolvido à UI. O log de um job falho passa fácil de MB; o
/// erro está no fim, então é o fim que fica.
pub const RUN_LOG_MAX_BYTES: usize = 400 * 1024;

fn looks_like_timestamp(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() >= 20 && b[..4].iter().all(u8::is_ascii_digit) && b[4] == b'-' && value.ends_with('Z')
}

/// `gh run view --log` prefixa cada linha com `<job>\t<step>\t<timestamp> `.
/// Devolve o passo (se veio) e só o texto.
fn split_log_line(line: &str) -> (Option<&str>, &str) {
    let mut parts = line.splitn(3, '\t');
    let (Some(_job), Some(step), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
        return (None, line);
    };
    let rest = rest.trim_start_matches('\u{feff}');
    let body = match rest.split_once(' ') {
        Some((ts, body)) if looks_like_timestamp(ts) => body,
        _ if looks_like_timestamp(rest) => "",
        _ => rest,
    };
    (Some(step), body)
}

/// Tira os prefixos, acha o primeiro passo com `##[error]` (e a linha dele),
/// guarda o FIM do log até `max` bytes (começando numa linha inteira) e
/// mascara segredos. Se o corte levou a linha do erro, `error_line` é `None`.
pub fn parse_run_log(stdout: &[u8], max: usize) -> RunLog {
    let raw = String::from_utf8_lossy(stdout);
    let mut failed_step: Option<&str> = None;
    let mut error_line: Option<usize> = None;
    let mut text = String::with_capacity(raw.len());
    for (idx, line) in raw.lines().enumerate() {
        let (step, body) = split_log_line(line);
        if body.contains("##[error]") {
            error_line = error_line.or(Some(idx));
            if failed_step.is_none() {
                failed_step = step.filter(|s| !s.is_empty() && *s != "UNKNOWN STEP");
            }
        }
        text.push_str(body);
        text.push('\n');
    }
    let truncated = text.len() > max;
    if truncated {
        let mut cut = text.len() - max;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        if let Some(nl) = text[cut..].find('\n') {
            cut += nl + 1;
        }
        let dropped = text[..cut].matches('\n').count();
        error_line = error_line.and_then(|i| i.checked_sub(dropped));
        text.drain(..cut);
    }
    let redact = crate::session::redact::redact;
    RunLog {
        text: redact(&text).into_owned(),
        truncated,
        failed_step: failed_step.map(|s| redact(s).into_owned()),
        error_line,
    }
}

pub fn run_log(repo_root: &Path, job_id: u64) -> Result<RunLog, AppError> {
    if job_id == 0 {
        return Err(cli_failed("id de job inválido"));
    }
    let remote = super::prs::action_remote(repo_root)?;
    let repo = super::prs::repo_flag(&remote);
    let job = job_id.to_string();
    let out = super::prs::gh(
        repo_root,
        &["run", "view", "--repo", &repo, "--job", &job, "--log"],
    )?;
    Ok(parse_run_log(&out, RUN_LOG_MAX_BYTES))
}

/// argv do `gh run rerun`; `--failed` reroda só os jobs que falharam.
pub fn rerun_args(repo: &str, run_id: u64, failed_only: bool) -> Result<Vec<String>, AppError> {
    if run_id == 0 {
        return Err(cli_failed("id de run inválido"));
    }
    let mut args: Vec<String> = ["run", "rerun", "--repo", repo].map(String::from).to_vec();
    if failed_only {
        args.push("--failed".into());
    }
    args.extend(["--".into(), run_id.to_string()]);
    Ok(args)
}

/// VERMELHO: só a UI chama, depois da confirmação (como o merge de PR). O
/// mesmo `gh run rerun` digitado por agente é vermelho no classificador.
pub fn run_rerun(repo_root: &Path, run_id: u64, failed_only: bool) -> Result<(), AppError> {
    if run_id == 0 {
        return Err(cli_failed("id de run inválido"));
    }
    let remote = super::prs::action_remote(repo_root)?;
    let args = rerun_args(&super::prs::repo_flag(&remote), run_id, failed_only)?;
    super::prs::gh_write(repo_root, &args, None)
}

pub fn pr_list(repo: &Path) -> Result<Vec<PullRequest>, AppError> {
    let out = exec::run(
        CLI,
        &[
            "pr", "list", "--state", "open", "--limit", "30", "--json", PR_FIELDS,
        ],
        repo,
        None,
        NETWORK_TIMEOUT,
    )
    .map_err(cli_failed)?;
    if !out.ok {
        return Err(cli_failed(out.stderr_message()));
    }
    parse_pr_list(&out.stdout).map_err(cli_failed)
}

pub fn pr_comments(worktree: &Path, number: u64) -> Result<Vec<ReviewComment>, AppError> {
    let inline = format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments?per_page=100");
    let issue = format!("repos/{{owner}}/{{repo}}/issues/{number}/comments?per_page=100");

    let mut comments = api_comments(worktree, &inline)?;
    comments.extend(api_comments(worktree, &issue)?);
    Ok(comments)
}

fn api_comments(worktree: &Path, endpoint: &str) -> Result<Vec<ReviewComment>, AppError> {
    let out =
        exec::run(CLI, &["api", endpoint], worktree, None, NETWORK_TIMEOUT).map_err(cli_failed)?;
    if !out.ok {
        return Err(cli_failed(out.stderr_message()));
    }
    parse_comments(&out.stdout).map_err(cli_failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PR_LIST: &str = include_str!("fixtures/gh_pr_list.json");
    const PR_VIEW: &str = include_str!("fixtures/gh_pr_view.json");
    const REVIEW_COMMENTS: &str = include_str!("fixtures/gh_review_comments.json");

    #[test]
    fn parses_a_real_gh_pr_list_payload() {
        let prs = parse_pr_list(PR_LIST.as_bytes()).unwrap();
        assert_eq!(prs.len(), 2);

        let closed = &prs[0];
        assert_eq!(closed.number, 13845);
        assert_eq!(closed.state, "closed");
        assert_eq!(
            closed.title,
            "Fix --reviewer regression with \"@org/team\" team slugs"
        );
        assert_eq!(closed.url, "https://github.com/cli/cli/pull/13845");
        assert_eq!(closed.checks.len(), 3);
        assert_eq!(closed.checks[0].name, "label-external / label_issues");
        assert_eq!(closed.checks[0].status, "completed");
        assert_eq!(closed.checks[0].conclusion.as_deref(), Some("success"));

        let open = &prs[1];
        assert_eq!(open.state, "open");
    }

    #[test]
    fn treats_an_empty_conclusion_as_pending() {
        let prs = parse_pr_list(PR_LIST.as_bytes()).unwrap();
        let pending = &prs[1].checks[1];
        assert_eq!(pending.name, "Community PR Approvals");
        assert_eq!(pending.status, "in_progress");
        assert_eq!(pending.conclusion, None);
    }

    #[test]
    fn maps_status_context_entries_to_check_runs() {
        let prs = parse_pr_list(PR_LIST.as_bytes()).unwrap();
        let context = prs[1]
            .checks
            .iter()
            .find(|c| c.name == "tide")
            .expect("StatusContext");
        assert_eq!(context.status, "completed");
        assert_eq!(context.conclusion.as_deref(), Some("pending"));
    }

    #[test]
    fn parses_a_pr_without_checks() {
        let pr = parse_pr(PR_VIEW.as_bytes()).unwrap();
        assert_eq!(pr.number, 13832);
        assert_eq!(pr.state, "merged");
        assert!(pr.checks.is_empty());
    }

    #[test]
    fn parses_real_review_comments_with_path_and_line() {
        let comments = parse_comments(REVIEW_COMMENTS.as_bytes()).unwrap();
        assert_eq!(comments.len(), 3);

        assert_eq!(comments[0].id, "332430752");
        assert_eq!(comments[0].author, "mislav");
        assert_eq!(comments[0].path.as_deref(), Some("command/pr.go"));
        assert_eq!(comments[0].line, Some(297));
        assert_eq!(
            comments[0].url.as_deref(),
            Some("https://github.com/cli/cli/pull/3#discussion_r332430752")
        );
    }

    #[test]
    fn falls_back_to_the_original_line_when_the_comment_is_outdated() {
        let comments = parse_comments(REVIEW_COMMENTS.as_bytes()).unwrap();
        assert_eq!(comments[1].id, "333030758");
        assert_eq!(comments[1].line, Some(347));
    }

    #[test]
    fn keeps_non_inline_comments_without_path_or_line() {
        let comments = parse_comments(REVIEW_COMMENTS.as_bytes()).unwrap();
        assert_eq!(comments[2].id, "539473254");
        assert_eq!(comments[2].path, None);
        assert_eq!(comments[2].line, None);
        assert!(!comments[2].body.is_empty());
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(parse_pr_list(b"not json").is_err());
        assert!(parse_comments(b"{}").is_err());
    }

    #[test]
    fn extracts_the_pr_number_from_the_create_output() {
        let stdout = b"Warning: 3 uncommitted changes\nhttps://github.com/cli/cli/pull/13850\n";
        assert_eq!(pr_number_from_output(stdout), Some(13850));
        assert_eq!(pr_number_from_output(b"nothing here"), None);
    }

    #[test]
    fn builds_the_web_fallback_url_escaping_the_branch() {
        let remote = super::super::remote::parse("git@github.com:tybadev/tyba-terminal.git")
            .expect("remote");
        assert_eq!(
            web_create_url(&remote, "feat/forge#1"),
            "https://github.com/tybadev/tyba-terminal/compare/feat%2Fforge%231?expand=1"
        );
    }

    #[test]
    fn parse_runs_le_a_fixture_do_gh() {
        let runs = parse_runs(include_str!("fixtures/gh_run_list.json").as_bytes()).unwrap();
        assert_eq!(runs.len(), 3);

        let release = &runs[0];
        assert_eq!(release.id, 111_111_111);
        assert_eq!(release.name, "Release");
        assert_eq!(release.status, "in_progress");
        assert_eq!(release.conclusion, None);
        // A tag no head_branch é o ponto da feature: um run de tag não tem PR,
        // e é ele que responde "cortei a tag, em que pé está o release?".
        assert_eq!(release.head_branch, "v9.9.9");
        assert_eq!(release.event, "push");
    }

    #[test]
    fn parse_runs_normaliza_status_e_conclusao() {
        let runs = parse_runs(include_str!("fixtures/gh_run_list.json").as_bytes()).unwrap();
        assert_eq!(runs[1].conclusion.as_deref(), Some("success"));
        assert_eq!(runs[2].conclusion.as_deref(), Some("failure"));
        for r in &runs {
            assert_eq!(r.status, r.status.to_lowercase());
        }
    }

    #[test]
    fn run_list_nao_chama_o_gh_com_core_zerado() {
        let host = "runs-precheck.example";
        crate::forge::rate::reset_for_tests(host);
        assert!(core_available(host).is_ok());
        crate::forge::rate::record_exhausted(host, "core", None);
        assert_eq!(core_available(host).unwrap_err().code, "forge.rate_limited");
        // Só o `core` (REST) barra: o balde GraphQL é outro.
        crate::forge::rate::reset_for_tests(host);
        crate::forge::rate::record_exhausted(host, "graphql", None);
        assert!(core_available(host).is_ok());
        crate::forge::rate::reset_for_tests(host);
    }

    #[test]
    fn run_jobs_usa_o_mesmo_repo_do_run_list() {
        assert_eq!(
            run_jobs_args(7, Some("github.com/exemplo/app".into())),
            [
                "run",
                "view",
                "7",
                "--repo",
                "github.com/exemplo/app",
                "--json",
                "jobs"
            ]
        );
        assert_eq!(
            run_jobs_args(7, None),
            ["run", "view", "7", "--json", "jobs"]
        );
    }

    #[test]
    fn parse_api_runs_tem_o_mesmo_contrato_do_gh_run_list() {
        let runs = parse_api_runs(include_bytes!("fixtures/gh_api_runs.json")).expect("parse");
        let cli = parse_runs(include_bytes!("fixtures/gh_run_list.json")).expect("parse");
        assert_eq!(runs.len(), 3);
        assert_eq!(runs, cli[..3].to_vec());
        assert!(parse_api_runs(b"nada").is_err());
        assert_eq!(parse_api_runs(b"{}").expect("vazio"), Vec::new());
    }

    #[test]
    fn parse_jobs_le_a_fixture_do_gh() {
        let jobs = parse_jobs(include_str!("fixtures/gh_run_jobs.json").as_bytes()).unwrap();
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0].name, "gates / frontend");
        assert_eq!(jobs[0].conclusion.as_deref(), Some("success"));
        assert_eq!(jobs[1].status, "in_progress");
        assert_eq!(jobs[1].conclusion, None);
        // Job na fila ainda não tem url — e a UI não pode explodir por isso.
        assert_eq!(jobs[2].status, "queued");
        assert_eq!(jobs[2].url, None);
    }

    #[test]
    fn parse_jobs_traz_id_horarios_e_passos() {
        let jobs = parse_jobs(include_str!("fixtures/gh_run_jobs.json").as_bytes()).unwrap();
        assert_eq!(jobs[0].id, 9_000_000_001);
        assert_eq!(jobs[0].started_at.as_deref(), Some("2026-01-02T10:00:00Z"));
        assert_eq!(
            jobs[0].completed_at.as_deref(),
            Some("2026-01-02T10:03:10Z")
        );
        assert_eq!(jobs[0].steps.len(), 3);
        assert_eq!(jobs[0].steps[1].name, "bun run test");
        assert_eq!(jobs[0].steps[2].conclusion.as_deref(), Some("skipped"));
        // Zero do Go = ainda não terminou; conclusão vazia = passo rodando.
        assert_eq!(jobs[1].completed_at, None);
        assert_eq!(jobs[1].steps[0].conclusion, None);
        // Job na fila não traz horário nem passos.
        assert_eq!(jobs[2].started_at, None);
        assert!(jobs[2].steps.is_empty());
    }

    // Log sintético no formato do `gh run view --job <id> --log`.
    const RUN_LOG: &str = "build\tSet up job\t\u{feff}2026-01-02T10:00:00.1234567Z Current runner version: '2.300.0'\n\
build\tRun tests\t2026-01-02T10:00:01.0000000Z running 3 tests\n\
build\tRun tests\t2026-01-02T10:00:02.0000000Z token=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789\n\
build\tRun tests\t2026-01-02T10:00:03.0000000Z ##[error]Process completed with exit code 1.\n\
build\tPost checkout\t2026-01-02T10:00:04.0000000Z ##[error]outro erro depois\n";

    #[test]
    fn run_log_tira_o_prefixo_e_acha_o_passo_que_falhou() {
        let log = parse_run_log(RUN_LOG.as_bytes(), RUN_LOG_MAX_BYTES);
        assert!(!log.truncated);
        assert_eq!(log.failed_step.as_deref(), Some("Run tests"));
        let lines: Vec<&str> = log.text.lines().collect();
        assert_eq!(lines[0], "Current runner version: '2.300.0'");
        assert_eq!(lines[1], "running 3 tests");
        assert_eq!(lines[3], "##[error]Process completed with exit code 1.");
        assert_eq!(log.error_line, Some(3));
    }

    #[test]
    fn run_log_error_line_conta_no_texto_cortado() {
        let mut raw = String::new();
        for i in 0..50 {
            raw.push_str(&format!(
                "j\tpasso\t2026-01-02T10:00:00.0000000Z linha {i}\n"
            ));
        }
        raw.push_str("j\tpasso\t2026-01-02T10:00:01.0000000Z ##[error]quebrou\n");
        raw.push_str("j\tpasso\t2026-01-02T10:00:02.0000000Z fim\n");
        let log = parse_run_log(raw.as_bytes(), 60);
        assert!(log.truncated);
        let i = log.error_line.expect("a linha do erro ficou no corte");
        assert_eq!(log.text.lines().nth(i), Some("##[error]quebrou"));
        // Sem corte, conta desde o começo.
        assert_eq!(
            parse_run_log(raw.as_bytes(), RUN_LOG_MAX_BYTES).error_line,
            Some(50)
        );
        // O corte levou a linha do erro: não aponta para outra.
        let mut raw2 = String::from("j\tp\t2026-01-02T10:00:00.0000000Z ##[error]cedo\n");
        for i in 0..50 {
            raw2.push_str(&format!("linha {i}\n"));
        }
        let log = parse_run_log(raw2.as_bytes(), 60);
        assert_eq!(log.error_line, None);
        assert_eq!(log.failed_step.as_deref(), Some("p"));
    }

    #[test]
    fn run_log_mascara_segredos() {
        let log = parse_run_log(RUN_LOG.as_bytes(), RUN_LOG_MAX_BYTES);
        assert!(!log.text.contains("ghp_"), "{}", log.text);
        assert!(log.text.contains("token=[REDACTED]"));
    }

    #[test]
    fn run_log_grande_guarda_o_fim_a_partir_de_linha_inteira() {
        let mut raw = String::new();
        for i in 0..1000 {
            raw.push_str(&format!("linha {i}\n"));
        }
        let log = parse_run_log(raw.as_bytes(), 100);
        assert!(log.truncated);
        assert!(log.text.len() <= 100);
        assert!(log.text.starts_with("linha "), "{}", log.text);
        assert!(log.text.ends_with("linha 999\n"));
        // Sem prefixo do gh, a linha passa inteira e não há passo.
        assert_eq!(log.failed_step, None);
        assert_eq!(log.error_line, None);
    }

    #[test]
    fn rerun_monta_argv_com_failed_so_quando_pedido() {
        assert_eq!(
            rerun_args("github.com/acme/widget", 42, false).unwrap(),
            [
                "run",
                "rerun",
                "--repo",
                "github.com/acme/widget",
                "--",
                "42"
            ]
        );
        assert_eq!(
            rerun_args("github.com/acme/widget", 42, true).unwrap(),
            [
                "run",
                "rerun",
                "--repo",
                "github.com/acme/widget",
                "--failed",
                "--",
                "42"
            ]
        );
        assert!(rerun_args("github.com/acme/widget", 0, true).is_err());
    }

    #[test]
    fn run_log_recusa_job_zero_antes_da_rede() {
        assert!(run_log(Path::new("/"), 0).is_err());
        assert!(run_rerun(Path::new("/"), 0, false).is_err());
    }

    #[test]
    fn parse_runs_recusa_lixo_em_vez_de_entregar_run_vazio() {
        assert!(parse_runs(b"").is_err());
        assert!(parse_runs(b"nao sou json").is_err());
        assert!(parse_jobs(b"{}").is_err());
        // Lista vazia é resposta legítima: repo sem run nenhum.
        assert_eq!(parse_runs(b"[]").unwrap().len(), 0);
    }
}
