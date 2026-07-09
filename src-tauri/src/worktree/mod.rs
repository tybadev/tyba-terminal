//! Worktrees: boundary de escrita de cada sessão de agente (Fase 3).
//!
//! Regras dos docs:
//! - shell-out para o binário `git` (não git2/gitoxide no MVP)
//! - sempre `-z`, `--no-color`, `-c core.quotePath=false`
//! - three-dot semantics: diff contra `base_ref` salvo na criação
//! - `git stash` NUNCA na automação (compartilhado entre worktrees)

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(windows)]
const HOOKS_PATH: &str = "core.hooksPath=NUL";
#[cfg(not(windows))]
const HOOKS_PATH: &str = "core.hooksPath=/dev/null";

pub fn git_in(path: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(path);
    cmd.args([
        "-c",
        "core.fsmonitor=false",
        "-c",
        "diff.external=",
        "-c",
        "core.quotePath=false",
        "-c",
        "core.pager=cat",
        "-c",
        HOOKS_PATH,
        "--no-optional-locks",
    ]);
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_NAMESPACE")
        .env_remove("GIT_CEILING_DIRECTORIES")
        .env_remove("GIT_EXTERNAL_DIFF")
        .env_remove("GIT_CONFIG")
        .env_remove("GIT_CONFIG_COUNT")
        .stdin(Stdio::null());
    cmd
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worktree {
    /// ~/.tyba/worktrees/<repo>/<branch>
    pub path: PathBuf,
    /// agent/<slug>-<sufixo curto>
    pub branch: String,
    /// sha da base no momento da criação (base do three-dot diff)
    pub base_ref: String,
    pub dirty: bool,
    pub ahead: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    enum AttrSource {
        WorktreeRoot,
        WorktreeSubdir,
        GitDirInfo,
    }

    fn hostile_repo(tag: &str, source: AttrSource) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("tyba-attr-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let marker = dir.join("FILTER_RAN");

        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .stdin(Stdio::null())
                .output()
                .expect("git")
                .status
                .success();
            assert!(ok, "git {args:?} falhou");
        };

        git(&["init", "-q", "."]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.join("sub/f.txt"), "hello\n").unwrap();
        let attrs = "* filter=pwn\n";
        match source {
            AttrSource::WorktreeRoot => std::fs::write(dir.join(".gitattributes"), attrs).unwrap(),
            AttrSource::WorktreeSubdir => {
                std::fs::write(dir.join("sub/.gitattributes"), attrs).unwrap()
            }
            AttrSource::GitDirInfo => {}
        }
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "base"]);
        if matches!(source, AttrSource::GitDirInfo) {
            std::fs::create_dir_all(dir.join(".git/info")).unwrap();
            std::fs::write(dir.join(".git/info/attributes"), attrs).unwrap();
        }
        git(&[
            "config",
            "filter.pwn.clean",
            &format!("sh -c 'touch \"{}\"; cat'", marker.display()),
        ]);
        std::fs::write(dir.join("sub/f.txt"), "hello\nchanged\n").unwrap();
        (dir, marker)
    }

    fn assert_filter_never_ran(tag: &str, source: AttrSource) {
        let (dir, marker) = hostile_repo(tag, source);

        let status = git_in(&dir)
            .args(["status", "--porcelain", "-z"])
            .output()
            .unwrap();
        assert!(status.status.success(), "status falhou em {tag}");

        let diff = git_in(&dir)
            .args(["diff", "--no-ext-diff", "--numstat", "--no-color", "HEAD"])
            .output()
            .unwrap();
        assert!(diff.status.success(), "diff falhou em {tag}");
        assert!(
            String::from_utf8_lossy(&diff.stdout).contains("f.txt"),
            "numstat deve continuar correto em {tag}"
        );

        let ran = marker.exists();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!ran, "filtro clean executou sob git_in via {tag}");
    }

    #[test]
    #[cfg(unix)]
    #[ignore = "git_in nao impede filtro definido na config do repo; exige sandbox do processo git (ver SECURITY.md)"]
    fn git_in_neutralizes_worktree_root_attributes() {
        assert_filter_never_ran("root", AttrSource::WorktreeRoot);
    }

    #[test]
    #[cfg(unix)]
    #[ignore = "git_in nao impede filtro definido na config do repo; exige sandbox do processo git (ver SECURITY.md)"]
    fn git_in_neutralizes_worktree_subdir_attributes() {
        assert_filter_never_ran("subdir", AttrSource::WorktreeSubdir);
    }

    #[test]
    #[cfg(unix)]
    #[ignore = "git_in nao impede filtro definido na config do repo; exige sandbox do processo git (ver SECURITY.md)"]
    fn git_in_neutralizes_git_dir_info_attributes() {
        assert_filter_never_ran("info", AttrSource::GitDirInfo);
    }
}

// TODO(fase 3):
// - create(repo_root, task_title) -> Worktree
//   `git worktree add <path> -b <branch> <base_sha>` + hooks .tyba/setup.sh
// - remove(worktree, delete_branch: bool)
// - gc_orphans() no startup via `git worktree list --porcelain`
// - diff module: SessionDiff { commits, files, uncommitted } com hunks lazy
