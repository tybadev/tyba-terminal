//! Branch explorer: listar branches e analisar o que uma branch tem sobre a
//! default (spec: git-panel/branch-explorer). Fetch é a única op de rede e
//! só roda por clique explícito (princípio #4). Checkout (fase 2) escreve no
//! working tree — recusa árvore suja e exige confirmação na UI.

use std::path::Path;

use serde::Serialize;

use crate::error::AppError;

use super::{git_in, git_in_net, git_in_rw, git_text, run_git};

pub const MAX_BRANCHES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
    pub subject: String,
    pub committed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchList {
    pub branches: Vec<BranchInfo>,
    pub truncated: usize,
}

pub fn list(repo: &Path) -> Result<BranchList, String> {
    let raw = run_git(
        {
            let mut c = git_in(repo);
            c.args([
                "for-each-ref",
                "--sort=-committerdate",
                "--format=%(refname)%00%(HEAD)%00%(subject)%00%(committerdate:iso-strict)",
                "refs/heads",
                "refs/remotes",
            ]);
            c
        },
        "git for-each-ref",
    )?;
    Ok(parse_branches(&String::from_utf8_lossy(&raw)))
}

/// Uma entrada por linha, campos separados por NUL. Remota some quando existe
/// local de mesmo nome; `origin/HEAD` (symref) é ruído e cai fora.
pub fn parse_branches(raw: &str) -> BranchList {
    let mut locals: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut entries: Vec<(String, bool, bool, String, String)> = Vec::new();

    for line in raw.lines() {
        let fields: Vec<&str> = line.split('\u{0}').collect();
        if fields.len() < 4 {
            continue;
        }
        let refname = fields[0];
        let is_current = fields[1] == "*";
        let (name, is_remote, dedup_key) = if let Some(rest) = refname.strip_prefix("refs/heads/") {
            (rest.to_string(), false, rest.to_string())
        } else if let Some(rest) = refname.strip_prefix("refs/remotes/") {
            let Some((_, short)) = rest.split_once('/') else {
                continue;
            };
            if short == "HEAD" {
                continue;
            }
            (rest.to_string(), true, short.to_string())
        } else {
            continue;
        };
        if !is_remote {
            locals.insert(dedup_key.clone());
        }
        entries.push((
            name,
            is_current,
            is_remote,
            fields[2].to_string(),
            fields[3].to_string(),
        ));
        let _ = dedup_key;
    }

    let mut branches: Vec<BranchInfo> = entries
        .into_iter()
        .filter(|(name, _, is_remote, _, _)| {
            if !*is_remote {
                return true;
            }
            let short = name.split_once('/').map(|(_, s)| s).unwrap_or(name);
            !locals.contains(short)
        })
        .map(
            |(name, is_current, is_remote, subject, committed_at)| BranchInfo {
                name,
                is_current,
                is_remote,
                subject,
                committed_at,
            },
        )
        .collect();

    let truncated = branches.len().saturating_sub(MAX_BRANCHES);
    branches.truncate(MAX_BRANCHES);
    BranchList {
        branches,
        truncated,
    }
}

/// Base pra comparar branches: a default do remoto; sem remoto, main/master
/// local; em último caso, HEAD.
pub fn default_base(repo: &Path) -> String {
    if let Ok(head) = git_text(
        {
            let mut c = git_in(repo);
            c.args(["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"]);
            c
        },
        "git symbolic-ref",
    ) {
        if !head.is_empty() {
            return head;
        }
    }
    for candidate in ["main", "master"] {
        let ok = git_text(
            {
                let mut c = git_in(repo);
                c.args([
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{candidate}"),
                ]);
                c
            },
            "git rev-parse",
        )
        .map(|out| !out.is_empty())
        .unwrap_or(false);
        if ok {
            return candidate.to_string();
        }
    }
    "HEAD".to_string()
}

/// Nomes vêm do for-each-ref, mas o IPC é fronteira: nada que o git possa
/// confundir com flag ou range passa daqui.
pub fn validate_ref_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && !name.starts_with('-')
        && !name.contains("..")
        && !name.chars().any(|c| c.is_whitespace() || c == '\u{0}');
    if ok {
        Ok(())
    } else {
        Err(format!("nome de branch inválido: {name}"))
    }
}

pub fn merge_base(repo: &Path, base: &str, branch: &str) -> Result<String, String> {
    git_text(
        {
            let mut c = git_in(repo);
            c.args(["merge-base", base, branch]);
            c
        },
        "git merge-base",
    )
}

pub fn fetch(repo: &Path) -> Result<(), String> {
    run_git(
        {
            let mut c = git_in_net(repo);
            c.args(["fetch", "--all", "--prune"]);
            c
        },
        "git fetch",
    )?;
    Ok(())
}

/// Troca a branch do working tree. Recusa árvore suja (nada de stash na
/// automação); branch remota (`origin/x`) vira local com tracking.
pub fn checkout(repo: &Path, branch: &str, is_remote: bool) -> Result<(), AppError> {
    let branch = branch.trim();
    validate_ref_name(branch)
        .map_err(|detail| AppError::new("checkout.failed").with("detail", detail))?;
    if super::is_dirty(repo)
        .map_err(|detail| AppError::new("checkout.failed").with("detail", detail))?
    {
        return Err(AppError::new("checkout.dirty"));
    }
    let out = {
        let mut c = git_in_rw(repo);
        if is_remote {
            c.args(["checkout", "--track", branch]);
        } else {
            c.args(["checkout", branch]);
        }
        c.output()
            .map_err(|e| AppError::new("checkout.failed").with("detail", format!("git checkout: {e}")))?
    };
    if !out.status.success() {
        return Err(AppError::new("checkout.failed").with(
            "detail",
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_locals_and_remotes_with_dedup_and_skips_origin_head() {
        let raw = concat!(
            "refs/heads/develop\u{0}*\u{0}feat: x\u{0}2026-07-13T01:00:00-03:00\n",
            "refs/remotes/origin/HEAD\u{0} \u{0}\u{0}2026-07-13T01:00:00-03:00\n",
            "refs/remotes/origin/develop\u{0} \u{0}feat: x\u{0}2026-07-13T01:00:00-03:00\n",
            "refs/remotes/origin/feat/novo\u{0} \u{0}feat: novo\u{0}2026-07-12T10:00:00-03:00\n",
            "refs/heads/fix/y\u{0} \u{0}fix: y\u{0}2026-07-11T09:00:00-03:00\n",
        );
        let list = parse_branches(raw);
        let names: Vec<&str> = list.branches.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, vec!["develop", "origin/feat/novo", "fix/y"]);
        assert!(list.branches[0].is_current);
        assert!(!list.branches[0].is_remote);
        assert!(list.branches[1].is_remote);
        assert_eq!(list.truncated, 0);
    }

    #[test]
    fn subject_with_nul_never_happens_but_short_lines_are_skipped() {
        let list = parse_branches("refs/heads/x\u{0}*\n");
        assert!(list.branches.is_empty());
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn temp_repo() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-branches-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "t@t.com"]);
        git(&dir, &["config", "user.name", "t"]);
        git(&dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.txt"), "base\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-qm", "base"]);
        dir
    }

    #[test]
    fn lists_branches_and_branch_diff_shows_only_the_branch_work() {
        let repo = temp_repo();
        git(&repo, &["checkout", "-qb", "feat/x"]);
        std::fs::write(repo.join("novo.txt"), "da branch\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "feat: na branch"]);
        git(&repo, &["checkout", "-q", "main"]);

        let list = list(&repo).expect("list");
        let names: Vec<&str> = list.branches.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"main") && names.contains(&"feat/x"));
        assert!(
            list.branches
                .iter()
                .find(|b| b.name == "main")
                .unwrap()
                .is_current
        );

        let base = default_base(&repo);
        assert_eq!(base, "main");
        let mb = merge_base(&repo, &base, "feat/x").unwrap();
        let diff = crate::worktree::diff::branch_diff(&repo, &mb, "feat/x").unwrap();
        assert_eq!(diff.commits.len(), 1);
        assert_eq!(diff.commits[0].subject, "feat: na branch");
        assert_eq!(diff.files.len(), 1);
        assert_eq!(diff.files[0].path, "novo.txt");
        assert!(diff.staged_files.is_empty() && diff.unstaged_files.is_empty());

        let hunks = crate::worktree::diff::range_file_hunks(
            &repo,
            &format!("{mb}..feat/x"),
            "novo.txt",
            None,
        )
        .unwrap();
        assert_eq!(hunks.hunks.len(), 1);

        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn ref_name_validation_blocks_flags_and_ranges() {
        assert!(validate_ref_name("feat/ok-1.2").is_ok());
        assert!(validate_ref_name("origin/feat/ok").is_ok());
        assert!(validate_ref_name("-rf").is_err());
        assert!(validate_ref_name("a..b").is_err());
        assert!(validate_ref_name("a b").is_err());
        assert!(validate_ref_name("").is_err());
    }

    fn checkout_repo() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-checkout-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "t@t.com"]);
        git(&dir, &["config", "user.name", "t"]);
        git(&dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.txt"), "base\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-qm", "base"]);
        git(&dir, &["branch", "feat/x"]);
        dir
    }

    fn current(dir: &Path) -> String {
        git_text(
            {
                let mut c = git_in(dir);
                c.args(["symbolic-ref", "--short", "HEAD"]);
                c
            },
            "symbolic-ref",
        )
        .unwrap()
    }

    #[test]
    fn checkout_switches_to_a_local_branch() {
        let repo = checkout_repo();
        checkout(&repo, "feat/x", false).unwrap();
        assert_eq!(current(&repo), "feat/x");
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn checkout_refuses_a_dirty_tree() {
        let repo = checkout_repo();
        std::fs::write(repo.join("a.txt"), "sujo\n").unwrap();
        let err = checkout(&repo, "feat/x", false).unwrap_err();
        assert_eq!(err.code, "checkout.dirty", "{err}");
        assert_eq!(current(&repo), "main");
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn checkout_of_a_remote_branch_creates_the_tracking_local() {
        let repo = checkout_repo();
        let bare = std::env::temp_dir().join(format!("tyba-checkout-{}.git", uuid::Uuid::new_v4()));
        git(
            &repo,
            &["clone", "--bare", "-q", ".", bare.to_str().unwrap()],
        );
        git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
        git(&repo, &["fetch", "-q", "origin"]);
        git(&repo, &["branch", "-D", "feat/x"]);

        checkout(&repo, "origin/feat/x", true).unwrap();

        assert_eq!(current(&repo), "feat/x");
        std::fs::remove_dir_all(&repo).ok();
        std::fs::remove_dir_all(&bare).ok();
    }

    #[test]
    fn checkout_rejects_flag_looking_names() {
        let repo = checkout_repo();
        let err = checkout(&repo, "-rf", false).unwrap_err();
        assert_eq!(err.code, "checkout.failed", "{err}");
        std::fs::remove_dir_all(&repo).ok();
    }
}
