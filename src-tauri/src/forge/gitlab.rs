use std::path::Path;

use serde::Deserialize;

use super::exec::{self, LOCAL_TIMEOUT, NETWORK_TIMEOUT};
use super::remote::Remote;
use super::{CheckRun, PullRequest, ReviewComment};

pub const CLI: &str = "glab";

const TERMINAL_PIPELINE_STATUS: [&str; 6] = [
    "success",
    "failed",
    "canceled",
    "cancelled",
    "skipped",
    "manual",
];

#[derive(Debug, Deserialize)]
struct GlMergeRequest {
    iid: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    web_url: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    head_pipeline: Option<GlPipeline>,
    #[serde(default)]
    pipeline: Option<GlPipeline>,
}

#[derive(Debug, Deserialize)]
struct GlPipeline {
    #[serde(default)]
    status: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GlDiscussion {
    #[serde(default)]
    notes: Vec<GlNote>,
}

#[derive(Debug, Deserialize)]
struct GlNote {
    id: u64,
    #[serde(default)]
    body: String,
    #[serde(default)]
    author: Option<GlAuthor>,
    #[serde(default)]
    system: bool,
    #[serde(default)]
    position: Option<GlPosition>,
}

#[derive(Debug, Deserialize)]
struct GlAuthor {
    #[serde(default)]
    username: String,
}

#[derive(Debug, Deserialize)]
struct GlPosition {
    #[serde(default)]
    new_path: Option<String>,
    #[serde(default)]
    old_path: Option<String>,
    #[serde(default)]
    new_line: Option<u32>,
    #[serde(default)]
    old_line: Option<u32>,
}

fn map_state(state: &str) -> String {
    match state.trim().to_ascii_lowercase().as_str() {
        "opened" | "locked" => "open".into(),
        other => other.to_string(),
    }
}

fn map_pipeline(pipeline: GlPipeline) -> Option<CheckRun> {
    let status = pipeline.status.trim().to_ascii_lowercase();
    if status.is_empty() {
        return None;
    }
    let name = pipeline
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "pipeline".into());
    let terminal = TERMINAL_PIPELINE_STATUS.contains(&status.as_str());
    Some(CheckRun {
        name,
        status: if terminal {
            "completed".into()
        } else {
            status.clone()
        },
        conclusion: terminal.then_some(status),
    })
}

impl From<GlMergeRequest> for PullRequest {
    fn from(mr: GlMergeRequest) -> Self {
        let checks = mr
            .head_pipeline
            .or(mr.pipeline)
            .and_then(map_pipeline)
            .map(|c| vec![c])
            .unwrap_or_default();
        PullRequest {
            number: mr.iid,
            title: mr.title,
            url: mr.web_url,
            state: map_state(&mr.state),
            checks,
        }
    }
}

pub fn parse_mr(stdout: &[u8]) -> Result<PullRequest, String> {
    let mr: GlMergeRequest = serde_json::from_slice(stdout)
        .map_err(|e| format!("resposta inesperada de `glab mr view`: {e}"))?;
    Ok(mr.into())
}

pub fn parse_mr_list(stdout: &[u8]) -> Result<Vec<PullRequest>, String> {
    let mrs: Vec<GlMergeRequest> = serde_json::from_slice(stdout)
        .map_err(|e| format!("resposta inesperada de `glab mr list`: {e}"))?;
    Ok(mrs.into_iter().map(Into::into).collect())
}

pub fn parse_discussions(stdout: &[u8], mr_url: &str) -> Result<Vec<ReviewComment>, String> {
    let discussions: Vec<GlDiscussion> = serde_json::from_slice(stdout)
        .map_err(|e| format!("resposta inesperada da API do GitLab: {e}"))?;

    Ok(discussions
        .into_iter()
        .flat_map(|d| d.notes)
        .filter(|n| !n.system)
        .map(|n| {
            let (path, line) = match n.position {
                Some(p) => (p.new_path.or(p.old_path), p.new_line.or(p.old_line)),
                None => (None, None),
            };
            let url = (!mr_url.is_empty()).then(|| format!("{mr_url}#note_{}", n.id));
            ReviewComment {
                id: n.id.to_string(),
                author: n.author.map(|a| a.username).unwrap_or_default(),
                body: n.body,
                path,
                line,
                url,
            }
        })
        .collect())
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
        "{}/-/merge_requests/new?merge_request%5Bsource_branch%5D={}",
        remote.base_url(),
        super::encode_branch(branch)
    )
}

pub fn create_pr(worktree: &Path, title: &str, body: &str) -> Result<PullRequest, String> {
    let out = exec::run(
        CLI,
        &[
            "mr",
            "create",
            "--yes",
            "--no-editor",
            "--title",
            title,
            "--description",
            body,
        ],
        worktree,
        None,
        NETWORK_TIMEOUT,
    )?;
    if !out.ok {
        return Err(format!("`glab mr create` falhou: {}", out.stderr_message()));
    }
    let branch = super::current_branch(worktree)?;
    mr_by_ref(worktree, &branch)
}

pub fn mr_by_ref(worktree: &Path, reference: &str) -> Result<PullRequest, String> {
    let out = exec::run(
        CLI,
        &["mr", "view", reference, "--output", "json"],
        worktree,
        None,
        NETWORK_TIMEOUT,
    )?;
    if !out.ok {
        return Err(format!("`glab mr view` falhou: {}", out.stderr_message()));
    }
    parse_mr(&out.stdout)
}

pub fn pr_for_branch(worktree: &Path, branch: &str) -> Result<Option<PullRequest>, String> {
    let out = exec::run(
        CLI,
        &[
            "mr",
            "list",
            "--all",
            "--source-branch",
            branch,
            "--per-page",
            "1",
            "--output",
            "json",
        ],
        worktree,
        None,
        NETWORK_TIMEOUT,
    )?;
    if !out.ok {
        return Err(format!("`glab mr list` falhou: {}", out.stderr_message()));
    }
    Ok(parse_mr_list(&out.stdout)?.into_iter().next())
}

pub fn pr_comments(worktree: &Path, number: u64) -> Result<Vec<ReviewComment>, String> {
    let endpoint = format!("projects/:fullpath/merge_requests/{number}/discussions?per_page=100");
    let out = exec::run(CLI, &["api", &endpoint], worktree, None, NETWORK_TIMEOUT)?;
    if !out.ok {
        return Err(format!("`glab api` falhou: {}", out.stderr_message()));
    }
    let mr_url = super::remote_of(worktree)
        .map(|r| format!("{}/-/merge_requests/{number}", r.base_url()))
        .unwrap_or_default();
    parse_discussions(&out.stdout, &mr_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MR_VIEW: &str = include_str!("fixtures/gl_mr_view.json");
    const MR_LIST: &str = include_str!("fixtures/gl_mr_list.json");
    const DISCUSSIONS: &str = include_str!("fixtures/gl_discussions.json");

    #[test]
    fn parses_a_real_glab_mr_view_payload() {
        let mr = parse_mr(MR_VIEW.as_bytes()).unwrap();
        assert_eq!(mr.number, 2260);
        assert_eq!(mr.state, "merged");
        assert_eq!(
            mr.url,
            "https://gitlab.com/gitlab-org/cli/-/merge_requests/2260"
        );
        assert!(mr.title.starts_with("fix: glab ci status fails"));
    }

    #[test]
    fn maps_the_head_pipeline_to_a_single_check() {
        let mr = parse_mr(MR_VIEW.as_bytes()).unwrap();
        assert_eq!(mr.checks.len(), 1);
        assert_eq!(mr.checks[0].name, "pipeline");
        assert_eq!(mr.checks[0].status, "completed");
        assert_eq!(mr.checks[0].conclusion.as_deref(), Some("success"));
    }

    #[test]
    fn keeps_a_running_pipeline_without_a_conclusion() {
        let json = br#"{"iid":9,"title":"t","web_url":"u","state":"opened","head_pipeline":{"status":"running","name":"build"}}"#;
        let mr = parse_mr(json).unwrap();
        assert_eq!(mr.state, "open");
        assert_eq!(mr.checks[0].name, "build");
        assert_eq!(mr.checks[0].status, "running");
        assert_eq!(mr.checks[0].conclusion, None);
    }

    #[test]
    fn parses_a_real_glab_mr_list_payload_without_pipelines() {
        let mrs = parse_mr_list(MR_LIST.as_bytes()).unwrap();
        assert_eq!(mrs.len(), 3);
        assert_eq!(mrs[0].number, 3500);
        assert_eq!(mrs[0].state, "open");
        assert!(mrs[0].checks.is_empty());
    }

    #[test]
    fn parses_discussions_into_inline_and_plain_comments() {
        let comments = parse_discussions(
            DISCUSSIONS.as_bytes(),
            "https://gitlab.com/g/p/-/merge_requests/3",
        )
        .unwrap();
        assert_eq!(comments.len(), 3);

        assert_eq!(comments[0].id, "1126");
        assert_eq!(comments[0].author, "venkatesh.thalluri");
        assert_eq!(comments[0].path, None);
        assert_eq!(comments[0].line, None);
        assert_eq!(
            comments[0].url.as_deref(),
            Some("https://gitlab.com/g/p/-/merge_requests/3#note_1126")
        );

        assert_eq!(comments[1].id, "1129");
        assert_eq!(
            comments[1].path.as_deref(),
            Some("internal/glrepo/remote.go")
        );
        assert_eq!(comments[1].line, Some(118));

        assert_eq!(comments[2].id, "1140");
        assert_eq!(comments[2].author, "ashu07das");
    }

    #[test]
    fn drops_system_notes() {
        let comments = parse_discussions(DISCUSSIONS.as_bytes(), "").unwrap();
        assert!(!comments.iter().any(|c| c.id == "1131"));
        assert!(comments.iter().all(|c| c.url.is_none()));
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(parse_mr(b"[]").is_err());
        assert!(parse_mr_list(b"{}").is_err());
        assert!(parse_discussions(b"nope", "").is_err());
    }

    #[test]
    fn builds_the_web_fallback_url_for_subgroups() {
        let remote =
            super::super::remote::parse("git@gitlab.com:group/sub/app.git").expect("remote");
        assert_eq!(
            web_create_url(&remote, "feat/forge"),
            "https://gitlab.com/group/sub/app/-/merge_requests/new?merge_request%5Bsource_branch%5D=feat%2Fforge"
        );
    }
}
