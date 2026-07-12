pub mod exec;
pub mod github;
pub mod gitlab;
pub mod remote;

use std::path::Path;

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use remote::Remote;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
    GitHub,
    GitLab,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForgeStatus {
    pub kind: ForgeKind,
    pub cli: String,
    pub installed: bool,
    pub authenticated: bool,
    pub web_create_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub checks: Vec<CheckRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckRun {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewComment {
    pub id: String,
    pub author: String,
    pub body: String,
    pub path: Option<String>,
    pub line: Option<u32>,
    pub url: Option<String>,
}

fn encode_branch(branch: &str) -> String {
    utf8_percent_encode(branch, NON_ALPHANUMERIC)
        .to_string()
        .replace("%2D", "-")
        .replace("%2E", ".")
        .replace("%5F", "_")
        .replace("%7E", "~")
}

fn remote_of(repo_root: &Path) -> Option<Remote> {
    let out = exec::run(
        "git",
        &["remote", "get-url", "origin"],
        repo_root,
        None,
        exec::LOCAL_TIMEOUT,
    )
    .ok()?;
    if !out.ok {
        return None;
    }
    remote::parse(String::from_utf8_lossy(&out.stdout).trim())
}

fn current_branch(repo_root: &Path) -> Result<String, AppError> {
    let out = exec::run(
        "git",
        &["rev-parse", "--abbrev-ref", "HEAD"],
        repo_root,
        None,
        exec::LOCAL_TIMEOUT,
    )
    .map_err(|detail| AppError::new("push.failed").with("detail", detail))?;
    if !out.ok {
        return Err(AppError::new("push.failed").with("detail", out.stderr_message()));
    }
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        return Err(AppError::new("push.detached_head"));
    }
    Ok(branch)
}

pub fn detect(repo_root: &Path) -> Option<ForgeKind> {
    remote_of(repo_root)?.kind()
}

pub fn status(repo_root: &Path, branch: Option<&str>) -> Option<ForgeStatus> {
    let remote = remote_of(repo_root)?;
    let kind = remote.kind()?;

    let (cli, installed, authenticated, web_create_url) = match kind {
        ForgeKind::GitHub => {
            let installed = github::installed();
            let authenticated = installed && github::authenticated(repo_root, &remote);
            (
                github::CLI,
                installed,
                authenticated,
                branch.map(|b| github::web_create_url(&remote, b)),
            )
        }
        ForgeKind::GitLab => {
            let installed = gitlab::installed();
            let authenticated = installed && gitlab::authenticated(repo_root, &remote);
            (
                gitlab::CLI,
                installed,
                authenticated,
                branch.map(|b| gitlab::web_create_url(&remote, b)),
            )
        }
    };

    Some(ForgeStatus {
        kind,
        cli: cli.into(),
        installed,
        authenticated,
        web_create_url,
    })
}

fn kind_of(worktree: &Path) -> Result<ForgeKind, AppError> {
    detect(worktree).ok_or_else(|| AppError::new("forge.no_forge"))
}

fn is_auth_error(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    [
        "not logged in",
        "authentication",
        "auth login",
        "unauthorized",
        "http 401",
        "401 unauthorized",
        "requires authentication",
    ]
    .iter()
    .any(|marker| detail.contains(marker))
}

pub fn create_pr(worktree: &Path, title: &str, body: &str) -> Result<PullRequest, AppError> {
    if title.trim().is_empty() {
        return Err(AppError::new("pr.title_empty"));
    }
    let branch = current_branch(worktree)?;
    if matches!(branch.as_str(), "main" | "master") {
        return Err(AppError::new("push.protected_branch").with("branch", branch));
    }
    crate::worktree::ops::push(worktree)?;
    match kind_of(worktree)? {
        ForgeKind::GitHub => github::create_pr(worktree, title, body),
        ForgeKind::GitLab => gitlab::create_pr(worktree, title, body),
    }
}

pub fn pr_for_branch(worktree: &Path, branch: &str) -> Result<Option<PullRequest>, AppError> {
    if branch.trim().is_empty() {
        return Err(AppError::new("pr.branch_empty"));
    }
    match kind_of(worktree)? {
        ForgeKind::GitHub => github::pr_for_branch(worktree, branch),
        ForgeKind::GitLab => gitlab::pr_for_branch(worktree, branch),
    }
}

pub fn pr_comments(worktree: &Path, number: u64) -> Result<Vec<ReviewComment>, AppError> {
    match kind_of(worktree)? {
        ForgeKind::GitHub => github::pr_comments(worktree, number),
        ForgeKind::GitLab => gitlab::pr_comments(worktree, number),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo(origin: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        for args in [vec!["init", "-q"], vec!["remote", "add", "origin", origin]] {
            let out = std::process::Command::new("git")
                .args(&args)
                .current_dir(root)
                .output()
                .expect("git");
            assert!(out.status.success(), "{args:?}");
        }
        dir
    }

    #[test]
    fn detects_github_from_the_origin_remote() {
        let repo = init_repo("git@github.com:tybadev/tyba-terminal.git");
        assert_eq!(detect(repo.path()), Some(ForgeKind::GitHub));
    }

    #[test]
    fn detects_gitlab_from_the_origin_remote() {
        let repo = init_repo("https://gitlab.com/group/sub/app.git");
        assert_eq!(detect(repo.path()), Some(ForgeKind::GitLab));
    }

    #[test]
    fn detects_nothing_on_an_unknown_forge() {
        let repo = init_repo("https://bitbucket.org/team/app.git");
        assert_eq!(detect(repo.path()), None);
        assert!(status(repo.path(), Some("main")).is_none());
        assert!(create_pr(repo.path(), "t", "b").is_err());
        assert!(pr_for_branch(repo.path(), "b").is_err());
        assert!(pr_comments(repo.path(), 1).is_err());
    }

    #[test]
    fn status_always_offers_the_web_fallback_url() {
        let repo = init_repo("git@github.com:tybadev/tyba-terminal.git");
        let status = status(repo.path(), Some("feat/forge")).expect("status");
        assert_eq!(status.kind, ForgeKind::GitHub);
        assert_eq!(status.cli, "gh");
        assert_eq!(
            status.web_create_url.as_deref(),
            Some("https://github.com/tybadev/tyba-terminal/compare/feat%2Fforge?expand=1")
        );
    }

    #[test]
    fn status_without_a_branch_has_no_fallback_url() {
        let repo = init_repo("https://gitlab.com/group/app.git");
        let status = status(repo.path(), None).expect("status");
        assert_eq!(status.cli, "glab");
        assert_eq!(status.web_create_url, None);
    }

    #[test]
    fn rejects_an_empty_title_or_branch_before_touching_the_network() {
        let repo = init_repo("git@github.com:tybadev/tyba-terminal.git");
        assert!(create_pr(repo.path(), "  ", "body").is_err());
        assert!(pr_for_branch(repo.path(), " ").is_err());
    }

    #[test]
    fn keeps_url_safe_characters_readable_in_the_branch() {
        assert_eq!(encode_branch("feat/forge-1.2_x~y"), "feat%2Fforge-1.2_x~y");
    }
}
