use std::path::Path;

use serde::Deserialize;

use super::exec::{self, LOCAL_TIMEOUT, NETWORK_TIMEOUT};
use super::remote::Remote;
use super::{CheckRun, PullRequest, ReviewComment};

pub const CLI: &str = "gh";

const PR_FIELDS: &str = "number,title,url,state,statusCheckRollup";

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

pub fn create_pr(worktree: &Path, title: &str, body: &str) -> Result<PullRequest, String> {
    let out = exec::run(
        CLI,
        &["pr", "create", "--title", title, "--body-file", "-"],
        worktree,
        Some(body.as_bytes()),
        NETWORK_TIMEOUT,
    )?;
    if !out.ok {
        return Err(format!("`gh pr create` falhou: {}", out.stderr_message()));
    }
    let number = pr_number_from_output(&out.stdout).ok_or_else(|| {
        format!(
            "`gh pr create` não devolveu a URL do PR: {}",
            out.stderr_message()
        )
    })?;
    pr_by_number(worktree, number)
}

pub fn pr_by_number(worktree: &Path, number: u64) -> Result<PullRequest, String> {
    let number = number.to_string();
    let out = exec::run(
        CLI,
        &["pr", "view", &number, "--json", PR_FIELDS],
        worktree,
        None,
        NETWORK_TIMEOUT,
    )?;
    if !out.ok {
        return Err(format!("`gh pr view` falhou: {}", out.stderr_message()));
    }
    parse_pr(&out.stdout)
}

pub fn pr_for_branch(worktree: &Path, branch: &str) -> Result<Option<PullRequest>, String> {
    let out = exec::run(
        CLI,
        &[
            "pr", "list", "--head", branch, "--state", "all", "--limit", "1", "--json", PR_FIELDS,
        ],
        worktree,
        None,
        NETWORK_TIMEOUT,
    )?;
    if !out.ok {
        return Err(format!("`gh pr list` falhou: {}", out.stderr_message()));
    }
    Ok(parse_pr_list(&out.stdout)?.into_iter().next())
}

pub fn pr_comments(worktree: &Path, number: u64) -> Result<Vec<ReviewComment>, String> {
    let inline = format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments?per_page=100");
    let issue = format!("repos/{{owner}}/{{repo}}/issues/{number}/comments?per_page=100");

    let mut comments = api_comments(worktree, &inline)?;
    comments.extend(api_comments(worktree, &issue)?);
    Ok(comments)
}

fn api_comments(worktree: &Path, endpoint: &str) -> Result<Vec<ReviewComment>, String> {
    let out = exec::run(CLI, &["api", endpoint], worktree, None, NETWORK_TIMEOUT)?;
    if !out.ok {
        return Err(format!("`gh api` falhou: {}", out.stderr_message()));
    }
    parse_comments(&out.stdout)
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
}
