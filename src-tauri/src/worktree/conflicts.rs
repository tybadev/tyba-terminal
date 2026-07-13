//! Detecção de operação conflitada em andamento (merge/rebase/cherry-pick).
//!
//! Fonte de verdade da feature "resolver conflitos com agente": marcadores
//! no git-dir do worktree (`MERGE_HEAD`, `rebase-merge/`, `rebase-apply/`,
//! `CHERRY_PICK_HEAD`) + entradas `u` do `status --porcelain=v2 -z`
//! (princípio #8: NUL-separated).

use std::path::Path;

use serde::Serialize;

use super::{git_in, resolved_git_dirs, run_git};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictOperation {
    Merge,
    Rebase,
    CherryPick,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictFile {
    pub path: String,
    /// XY do porcelain v2 (`UU`, `AA`, `DD`, `AU`…).
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictState {
    pub root: String,
    pub operation: ConflictOperation,
    pub ours: Option<String>,
    pub theirs: Option<String>,
    /// Vazia = operação em andamento com tudo resolvido (falta concluir).
    pub files: Vec<ConflictFile>,
}

pub fn session_conflicts(worktree: &Path) -> Result<Option<ConflictState>, String> {
    let Ok(dirs) = resolved_git_dirs(worktree) else {
        return Ok(None);
    };
    let git_dir = dirs.git_dir;

    let operation = if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists()
    {
        ConflictOperation::Rebase
    } else if git_dir.join("MERGE_HEAD").exists() {
        ConflictOperation::Merge
    } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
        ConflictOperation::CherryPick
    } else {
        return Ok(None);
    };

    let status = run_git(
        {
            let mut c = git_in(worktree);
            c.args(["status", "--porcelain=v2", "-z"]);
            c
        },
        "git status",
    )?;

    Ok(Some(ConflictState {
        root: worktree.to_string_lossy().into_owned(),
        operation,
        ours: ours_label(worktree, &git_dir),
        theirs: theirs_label(worktree, operation),
        files: parse_unmerged_z(&status),
    }))
}

fn git_text(worktree: &Path, args: &[&str]) -> Option<String> {
    let out = {
        let mut c = git_in(worktree);
        c.args(args);
        c
    }
    .output()
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn ours_label(worktree: &Path, git_dir: &Path) -> Option<String> {
    let head = git_text(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if head != "HEAD" {
        return Some(head);
    }
    let head_name = std::fs::read_to_string(git_dir.join("rebase-merge/head-name"))
        .or_else(|_| std::fs::read_to_string(git_dir.join("rebase-apply/head-name")))
        .ok()?;
    Some(
        head_name
            .trim()
            .strip_prefix("refs/heads/")
            .unwrap_or(head_name.trim())
            .to_string(),
    )
}

fn theirs_label(worktree: &Path, operation: ConflictOperation) -> Option<String> {
    match operation {
        ConflictOperation::Merge => {
            git_text(worktree, &["name-rev", "--name-only", "--always", "MERGE_HEAD"])
                .or_else(|| git_text(worktree, &["rev-parse", "--short", "MERGE_HEAD"]))
        }
        ConflictOperation::Rebase => git_text(worktree, &["rev-parse", "--short", "REBASE_HEAD"]),
        ConflictOperation::CherryPick => {
            git_text(worktree, &["rev-parse", "--short", "CHERRY_PICK_HEAD"])
        }
    }
}

fn parse_unmerged_z(bytes: &[u8]) -> Vec<ConflictFile> {
    let text = String::from_utf8_lossy(bytes);
    let mut fields = text.split('\0');
    let mut files = Vec::new();
    while let Some(field) = fields.next() {
        if field.starts_with("2 ") {
            fields.next();
            continue;
        }
        let Some(rest) = field.strip_prefix("u ") else {
            continue;
        };
        let mut parts = rest.splitn(10, ' ');
        let kind = parts.next().unwrap_or_default().to_string();
        let Some(path) = parts.nth(8) else {
            continue;
        };
        if !path.is_empty() {
            files.push(ConflictFile {
                path: path.to_string(),
                kind,
            });
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    fn git(dir: &Path, args: &[&str]) {
        let out = git_raw(dir, args);
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn git_raw(dir: &Path, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("git")
    }

    fn temp_repo() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-conflicts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "t@t.com"]);
        git(&dir, &["config", "user.name", "t"]);
        git(&dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.txt"), "um\ndois\ntres\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-qm", "base"]);
        dir
    }

    fn conflicted_merge_repo() -> PathBuf {
        let dir = temp_repo();
        git(&dir, &["checkout", "-qb", "feature"]);
        std::fs::write(dir.join("a.txt"), "um\nfeature\ntres\n").unwrap();
        git(&dir, &["commit", "-aqm", "feature"]);
        git(&dir, &["checkout", "-q", "main"]);
        std::fs::write(dir.join("a.txt"), "um\nmain\ntres\n").unwrap();
        git(&dir, &["commit", "-aqm", "main"]);
        let merge = git_raw(&dir, &["merge", "feature"]);
        assert!(!merge.status.success(), "merge deveria conflitar");
        dir
    }

    #[test]
    fn clean_repo_has_no_conflict_state() {
        let dir = temp_repo();
        assert_eq!(session_conflicts(&dir).unwrap(), None);
    }

    #[test]
    fn non_git_dir_has_no_conflict_state() {
        let dir = std::env::temp_dir().join(format!("tyba-conflicts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(session_conflicts(&dir).unwrap(), None);
    }

    #[test]
    fn detects_a_conflicted_merge_with_files_and_labels() {
        let dir = conflicted_merge_repo();
        let state = session_conflicts(&dir).unwrap().expect("estado de conflito");
        assert_eq!(state.operation, ConflictOperation::Merge);
        assert_eq!(state.ours.as_deref(), Some("main"));
        assert_eq!(state.theirs.as_deref(), Some("feature"));
        assert_eq!(state.files.len(), 1);
        assert_eq!(state.files[0].path, "a.txt");
        assert_eq!(state.files[0].kind, "UU");
    }

    #[test]
    fn merge_fully_resolved_but_unconcluded_keeps_the_operation() {
        let dir = conflicted_merge_repo();
        std::fs::write(dir.join("a.txt"), "um\nresolvido\ntres\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        let state = session_conflicts(&dir).unwrap().expect("estado de conflito");
        assert_eq!(state.operation, ConflictOperation::Merge);
        assert!(state.files.is_empty());
    }

    #[test]
    fn detects_a_conflicted_rebase_with_the_branch_label() {
        let dir = temp_repo();
        git(&dir, &["checkout", "-qb", "feature"]);
        std::fs::write(dir.join("a.txt"), "um\nfeature\ntres\n").unwrap();
        git(&dir, &["commit", "-aqm", "feature"]);
        git(&dir, &["checkout", "-q", "main"]);
        std::fs::write(dir.join("a.txt"), "um\nmain\ntres\n").unwrap();
        git(&dir, &["commit", "-aqm", "main"]);
        git(&dir, &["checkout", "-q", "feature"]);
        let rebase = git_raw(&dir, &["rebase", "main"]);
        assert!(!rebase.status.success(), "rebase deveria conflitar");

        let state = session_conflicts(&dir).unwrap().expect("estado de conflito");
        assert_eq!(state.operation, ConflictOperation::Rebase);
        assert_eq!(state.ours.as_deref(), Some("feature"));
        assert_eq!(state.files.len(), 1);
        assert_eq!(state.files[0].path, "a.txt");
    }

    #[test]
    fn parses_only_unmerged_entries_from_porcelain_v2() {
        let raw = [
            "1 M. N... 100644 100644 100644 aaa bbb mudado.txt",
            "u UU N... 100644 100644 100644 100644 aaa bbb ccc conflito.txt",
            "u AA N... 000000 100644 100644 100644 000 ddd eee com espaco.txt",
            "? solto.txt",
        ]
        .join("\0");
        let files = parse_unmerged_z(raw.as_bytes());
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "conflito.txt");
        assert_eq!(files[0].kind, "UU");
        assert_eq!(files[1].path, "com espaco.txt");
        assert_eq!(files[1].kind, "AA");
    }

    #[test]
    fn rename_entries_consume_the_orig_path_field() {
        let orig_path_disguised_as_unmerged =
            "u UU N... 100644 100644 100644 100644 aaa bbb ccc falso.txt";
        let raw = format!(
            "2 R. N... 100644 100644 100644 aaa bbb R100 novo.txt\0{orig_path_disguised_as_unmerged}\0u UU N... 100644 100644 100644 100644 aaa bbb ccc conflito.txt\0"
        );
        let files = parse_unmerged_z(raw.as_bytes());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "conflito.txt");
    }
}
