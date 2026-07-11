//! Ações de git do painel de diff: staging, discard, commit e push.
//!
//! Regras:
//! - paths chegam do webview: só relativos, sem `..` — e sempre viram
//!   pathspec `:(literal)` (nada de glob).
//! - discard é ação vermelha: o core só executa o que a UI confirmou,
//!   e untracked é removido via `git clean` (respeita o boundary do repo),
//!   nunca `fs::remove_file` cru.
//! - push recusa main/master SEMPRE, independente de quem pede
//!   (princípio #5); o resto é decisão explícita do humano no clique.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{git_in, git_text, run_git};

fn ensure_relative(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if p.is_absolute() {
        return Err(format!("path absoluto não é permitido: {path}"));
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("path com '..' não é permitido: {path}"));
    }
    if path.is_empty() {
        return Err("path vazio".into());
    }
    Ok(())
}

fn literal_specs(paths: &[String]) -> Result<Vec<String>, String> {
    paths.iter().try_fold(Vec::new(), |mut acc, p| {
        ensure_relative(p)?;
        acc.push(format!(":(literal){p}"));
        Ok(acc)
    })
}

/// `git add` dos paths (vazio = tudo, `-A`).
pub fn stage(worktree: &Path, paths: &[String]) -> Result<(), String> {
    let mut c = git_in(worktree);
    c.arg("add");
    if paths.is_empty() {
        c.arg("-A");
    } else {
        c.arg("--").args(literal_specs(paths)?);
    }
    run_git(c, "git add")?;
    Ok(())
}

/// Rename staged é um par (old, new) no index; tirar só o path novo do
/// stage deixaria a deleção do antigo staged — commit em seguida perderia
/// o arquivo do histórico. Expande cada path novo pro par completo.
fn expand_staged_renames(worktree: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    let raw = run_git(
        {
            let mut c = git_in(worktree);
            c.args([
                "diff",
                "--no-ext-diff",
                "--no-color",
                "--cached",
                "--name-status",
                "-M",
                "-z",
            ]);
            c
        },
        "git diff --cached --name-status",
    )?;
    let mut out = paths.to_vec();
    for (_, old_path, path) in super::diff::parse_name_status_z(&raw) {
        if let Some(old) = old_path {
            if paths.contains(&path) && !out.contains(&old) {
                out.push(old);
            }
        }
    }
    Ok(out)
}

/// `git restore --staged` dos paths (vazio = tudo).
pub fn unstage(worktree: &Path, paths: &[String]) -> Result<(), String> {
    let mut c = git_in(worktree);
    c.args(["restore", "--staged", "--"]);
    if paths.is_empty() {
        c.arg(".");
    } else {
        c.args(literal_specs(&expand_staged_renames(worktree, paths)?)?);
    }
    run_git(c, "git restore --staged")?;
    Ok(())
}

fn untracked_set(worktree: &Path) -> Result<std::collections::HashSet<String>, String> {
    let raw = run_git(
        {
            let mut c = git_in(worktree);
            c.args(["ls-files", "--others", "--exclude-standard", "-z"]);
            c
        },
        "git ls-files",
    )?;
    Ok(raw
        .split(|b| *b == 0)
        .filter(|f| !f.is_empty())
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect())
}

/// Descarta mudanças não-staged dos paths (vazio = tudo): tracked via
/// `git restore`, untracked via `git clean -fd` com pathspec literal.
pub fn discard(worktree: &Path, paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        run_git(
            {
                let mut c = git_in(worktree);
                c.args(["restore", "--worktree", "--", "."]);
                c
            },
            "git restore",
        )?;
        // -ff: repo aninhado criado por agente também sai — o usuário já
        // confirmou o discard em dois cliques; sumir em silêncio é pior.
        run_git(
            {
                let mut c = git_in(worktree);
                c.args(["clean", "-ffd"]);
                c
            },
            "git clean",
        )?;
        return Ok(());
    }

    let untracked = untracked_set(worktree)?;
    let (clean, restore): (Vec<String>, Vec<String>) =
        paths.iter().cloned().partition(|p| untracked.contains(p));
    if !restore.is_empty() {
        let mut c = git_in(worktree);
        c.args(["restore", "--worktree", "--"]);
        c.args(literal_specs(&restore)?);
        run_git(c, "git restore")?;
    }
    if !clean.is_empty() {
        let mut c = git_in(worktree);
        c.args(["clean", "-ffd", "--"]);
        c.args(literal_specs(&clean)?);
        run_git(c, "git clean")?;
    }
    Ok(())
}

/// `git commit -m` do que está staged. Mensagem vazia é erro antes de
/// encostar no git.
pub fn commit(worktree: &Path, message: &str) -> Result<(), String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("mensagem de commit vazia".into());
    }
    let mut c = git_in(worktree);
    c.args(["commit", "-m", message]);
    run_git(c, "git commit")?;
    Ok(())
}

const PROTECTED_BRANCHES: [&str; 2] = ["main", "master"];

/// `git push -u origin <branch>` — recusa main/master sempre.
pub fn push(worktree: &Path) -> Result<String, String> {
    let branch = git_text(
        {
            let mut c = git_in(worktree);
            c.args(["symbolic-ref", "--short", "-q", "HEAD"]);
            c
        },
        "git symbolic-ref",
    )?;
    if branch.is_empty() {
        return Err("HEAD destacado — sem branch pra push".into());
    }
    if PROTECTED_BRANCHES.contains(&branch.as_str()) {
        return Err(format!("push para {branch} é recusado pelo TYBA"));
    }
    let out = {
        let mut c = git_in(worktree);
        c.args(["push", "-u", "origin", &branch]);
        c.output().map_err(|e| format!("git push: {e}"))?
    };
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        return Err(format!("git push: {stderr}"));
    }
    Ok(stderr)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    Squash,
    MergeCommit,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergePreview {
    pub base_branch: String,
    pub source_branch: String,
    pub commits: usize,
    pub files_changed: usize,
    pub conflicts: Vec<String>,
    pub base_dirty: bool,
}

fn current_branch(repo: &Path, what: &str) -> Result<String, String> {
    let out = {
        let mut c = git_in(repo);
        c.args(["symbolic-ref", "--short", "-q", "HEAD"]);
        c.output().map_err(|e| format!("git symbolic-ref: {e}"))?
    };
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || branch.is_empty() {
        return Err(format!("{what} está com HEAD destacado — sem branch"));
    }
    Ok(branch)
}

fn nul_fields(raw: &[u8]) -> Vec<String> {
    raw.split(|b| *b == 0)
        .filter(|f| !f.is_empty())
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect()
}

fn merge_tree(main: &Path, base: &str, source: &str) -> Result<(String, Vec<String>), String> {
    let out = {
        let mut c = git_in(main);
        c.args([
            "merge-tree",
            "--write-tree",
            "--name-only",
            "-z",
            base,
            source,
        ]);
        c.output().map_err(|e| format!("git merge-tree: {e}"))?
    };
    if !matches!(out.status.code(), Some(0) | Some(1)) {
        return Err(format!(
            "git merge-tree: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut fields = out.stdout.split(|b| *b == 0);
    let tree = fields
        .next()
        .map(|f| String::from_utf8_lossy(f).trim().to_string())
        .unwrap_or_default();
    if tree.is_empty() {
        return Err("git merge-tree não produziu árvore".into());
    }
    let mut conflicts: Vec<String> = Vec::new();
    for field in fields {
        if field.is_empty() {
            break;
        }
        let path = String::from_utf8_lossy(field).into_owned();
        if !conflicts.contains(&path) {
            conflicts.push(path);
        }
    }
    Ok((tree, conflicts))
}

struct MergePlan {
    main: PathBuf,
    preview: MergePreview,
}

fn plan(worktree: &Path) -> Result<MergePlan, String> {
    let main = super::main_repo_of(worktree)?;
    let base_branch = current_branch(&main, "o repo principal")?;
    let source_branch = current_branch(worktree, "a sessão")?;
    if base_branch == source_branch {
        return Err(format!(
            "a sessão está na própria branch base ({base_branch}) — nada a mergear"
        ));
    }
    let raw = run_git(
        {
            let mut c = git_in(worktree);
            c.args([
                "diff",
                "--no-ext-diff",
                "--no-color",
                "--name-only",
                "-z",
                &format!("{base_branch}...HEAD"),
            ]);
            c
        },
        "git diff --name-only",
    )?;
    let (_, conflicts) = merge_tree(&main, &base_branch, &source_branch)?;
    Ok(MergePlan {
        preview: MergePreview {
            commits: super::ahead_count(worktree, &base_branch)? as usize,
            files_changed: nul_fields(&raw).len(),
            conflicts,
            base_dirty: super::is_dirty(&main)?,
            base_branch,
            source_branch,
        },
        main,
    })
}

pub fn merge_preview(worktree: &Path) -> Result<MergePreview, String> {
    Ok(plan(worktree)?.preview)
}

pub fn merge_into_base(
    worktree: &Path,
    strategy: MergeStrategy,
    message: Option<&str>,
) -> Result<String, String> {
    let MergePlan { main, preview } = plan(worktree)?;

    if preview.base_dirty {
        return Err(format!(
            "a branch base ({}) tem trabalho não-commitado — commite ou descarte antes de mergear",
            preview.base_branch
        ));
    }
    if !preview.conflicts.is_empty() {
        return Err(format!(
            "merge recusado: conflito com {} em {} — resolva na sessão antes",
            preview.base_branch,
            preview.conflicts.join(", ")
        ));
    }
    if preview.commits == 0 {
        return Err(format!(
            "nada para mergear: {} não tem commits além de {}",
            preview.source_branch, preview.base_branch
        ));
    }

    let base_head = super::head_sha(&main)?;
    let source_head = super::head_sha(worktree)?;

    let (tree, conflicts) = merge_tree(&main, &base_head, &source_head)?;
    if !conflicts.is_empty() {
        return Err(format!(
            "merge recusado: conflito em {}",
            conflicts.join(", ")
        ));
    }

    let message = message
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match strategy {
            MergeStrategy::Squash => format!("squash: {}", preview.source_branch),
            MergeStrategy::MergeCommit => {
                format!(
                    "merge: {} em {}",
                    preview.source_branch, preview.base_branch
                )
            }
        });

    let mut c = git_in(&main);
    c.args(["commit-tree", &tree, "-p", &base_head]);
    if matches!(strategy, MergeStrategy::MergeCommit) {
        c.args(["-p", &source_head]);
    }
    c.args(["-m", &message]);
    let merged = git_text(c, "git commit-tree")?;
    if merged.is_empty() {
        return Err("git commit-tree não produziu commit".into());
    }

    let now_head = super::head_sha(&main)?;
    if now_head != base_head {
        return Err(format!(
            "a branch base ({}) avançou durante o merge — revise e tente de novo",
            preview.base_branch
        ));
    }
    run_git(
        {
            let mut c = git_in(&main);
            c.args(["merge", "--ff-only", &merged]);
            c
        },
        "git merge --ff-only",
    )
    .map_err(|_| {
        format!(
            "a branch base ({}) mudou durante o merge — nada foi alterado, tente de novo",
            preview.base_branch
        )
    })?;
    Ok(merged)
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
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

    struct Fixture {
        base: PathBuf,
        repo: PathBuf,
        worktree: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.base).ok();
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let base = std::env::temp_dir().join(format!("tyba-merge-{tag}-{}", uuid::Uuid::new_v4()));
        let repo = base.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@t"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        fs::write(repo.join("f.txt"), "a\nb\nc\n").unwrap();
        fs::write(repo.join("outro.txt"), "intacto\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "init"]);
        let wt = super::super::create_in(&base.join("managed"), &repo, "tarefa").unwrap();
        Fixture {
            base,
            repo,
            worktree: wt.path,
        }
    }

    fn work(fx: &Fixture, content: &str, message: &str) {
        fs::write(fx.worktree.join("f.txt"), content).unwrap();
        git(&fx.worktree, &["add", "-A"]);
        git(&fx.worktree, &["commit", "-qm", message]);
    }

    fn head(dir: &Path) -> String {
        super::super::head_sha(dir).unwrap()
    }

    fn parents(repo: &Path, sha: &str) -> Vec<String> {
        let raw = git_text(
            {
                let mut c = git_in(repo);
                c.args(["rev-list", "--parents", "-n", "1", sha]);
                c
            },
            "rev-list --parents",
        )
        .unwrap();
        raw.split_whitespace().skip(1).map(String::from).collect()
    }

    #[test]
    fn preview_counts_commits_and_files_of_the_session() {
        let fx = fixture("preview");
        work(&fx, "a\nX\nc\n", "um");
        fs::write(fx.worktree.join("novo.txt"), "n\n").unwrap();
        git(&fx.worktree, &["add", "-A"]);
        git(&fx.worktree, &["commit", "-qm", "dois"]);

        let p = merge_preview(&fx.worktree).unwrap();

        assert_eq!(p.base_branch, "main");
        assert!(p.source_branch.starts_with("tyba/tarefa-"), "{p:?}");
        assert_eq!(p.commits, 2);
        assert_eq!(p.files_changed, 2);
        assert!(p.conflicts.is_empty(), "{p:?}");
        assert!(!p.base_dirty);
    }

    #[test]
    fn clean_squash_merge_fast_forwards_the_base() {
        let fx = fixture("squash");
        work(&fx, "a\nX\nc\n", "um");
        work(&fx, "a\nX\nZ\n", "dois");
        let before = head(&fx.repo);

        let sha = merge_into_base(&fx.worktree, MergeStrategy::Squash, Some("feat: tudo")).unwrap();

        assert_eq!(head(&fx.repo), sha);
        assert_eq!(parents(&fx.repo, &sha), vec![before.clone()]);
        assert_eq!(
            fs::read_to_string(fx.repo.join("f.txt")).unwrap(),
            "a\nX\nZ\n"
        );
        assert!(!super::super::is_dirty(&fx.repo).unwrap());
        assert_eq!(
            super::super::ahead_count(&fx.repo, &before).unwrap(),
            1,
            "squash colapsa os 2 commits em 1"
        );
    }

    #[test]
    fn clean_merge_commit_keeps_both_parents() {
        let fx = fixture("mergecommit");
        work(&fx, "a\nX\nc\n", "um");
        let base_before = head(&fx.repo);
        let source = head(&fx.worktree);

        let sha = merge_into_base(&fx.worktree, MergeStrategy::MergeCommit, None).unwrap();

        assert_eq!(head(&fx.repo), sha);
        assert_eq!(parents(&fx.repo, &sha), vec![base_before, source]);
        assert_eq!(
            fs::read_to_string(fx.repo.join("f.txt")).unwrap(),
            "a\nX\nc\n"
        );
    }

    #[test]
    fn conflict_is_detected_in_preview_and_merge_is_refused_without_touching_the_repo() {
        let fx = fixture("conflito");
        work(&fx, "a\nSESSAO\nc\n", "sessao");
        fs::write(fx.repo.join("f.txt"), "a\nBASE\nc\n").unwrap();
        git(&fx.repo, &["commit", "-qam", "base andou"]);
        let before = head(&fx.repo);

        let p = merge_preview(&fx.worktree).unwrap();
        assert_eq!(p.conflicts, vec!["f.txt".to_string()]);

        let err = merge_into_base(&fx.worktree, MergeStrategy::Squash, None).unwrap_err();
        assert!(err.contains("conflito"), "{err}");
        assert!(err.contains("f.txt"), "{err}");

        assert_eq!(head(&fx.repo), before);
        assert_eq!(
            fs::read_to_string(fx.repo.join("f.txt")).unwrap(),
            "a\nBASE\nc\n"
        );
        assert!(!super::super::is_dirty(&fx.repo).unwrap());
        assert!(!fx.repo.join(".git/MERGE_HEAD").exists());
    }

    #[test]
    fn merge_preserves_base_commits_made_after_the_worktree_forked() {
        let fx = fixture("toctou");
        work(&fx, "a\nb\nc\nd\n", "adiciona d");

        fs::write(fx.repo.join("novo.txt"), "commit da base\n").unwrap();
        git(&fx.repo, &["add", "-A"]);
        git(&fx.repo, &["commit", "-qm", "base avancou"]);

        merge_into_base(&fx.worktree, MergeStrategy::Squash, None)
            .expect("merge de source que não conflita com o commit novo da base");

        assert!(
            fx.repo.join("novo.txt").exists(),
            "o commit da base foi perdido no merge"
        );
        let head = super::super::head_sha(&fx.repo).unwrap();
        let parents = {
            let out = Command::new("git")
                .arg("-C")
                .arg(&fx.repo)
                .args(["rev-list", "--parents", "-n", "1", &head])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        assert!(!parents.is_empty());
    }

    #[test]
    fn dirty_base_refuses_merge_and_keeps_the_uncommitted_work() {
        let fx = fixture("suja");
        work(&fx, "a\nX\nc\n", "um");
        fs::write(fx.repo.join("outro.txt"), "trabalho do usuario\n").unwrap();
        let before = head(&fx.repo);

        let p = merge_preview(&fx.worktree).unwrap();
        assert!(p.base_dirty);

        let err = merge_into_base(&fx.worktree, MergeStrategy::MergeCommit, None).unwrap_err();
        assert!(err.contains("não-commitado"), "{err}");

        assert_eq!(head(&fx.repo), before);
        assert_eq!(
            fs::read_to_string(fx.repo.join("outro.txt")).unwrap(),
            "trabalho do usuario\n"
        );
        assert_eq!(
            fs::read_to_string(fx.repo.join("f.txt")).unwrap(),
            "a\nb\nc\n"
        );
    }

    #[test]
    fn nothing_to_merge_is_refused() {
        let fx = fixture("vazio");
        let before = head(&fx.repo);

        let p = merge_preview(&fx.worktree).unwrap();
        assert_eq!(p.commits, 0);
        assert_eq!(p.files_changed, 0);

        let err = merge_into_base(&fx.worktree, MergeStrategy::Squash, None).unwrap_err();
        assert!(err.contains("nada para mergear"), "{err}");
        assert_eq!(head(&fx.repo), before);
    }

    #[test]
    fn merging_the_main_repo_into_itself_is_refused() {
        let fx = fixture("selfmerge");
        let err = merge_preview(&fx.repo).unwrap_err();
        assert!(err.contains("própria branch base"), "{err}");
    }

    #[test]
    fn detached_session_head_is_refused() {
        let fx = fixture("detached");
        work(&fx, "a\nX\nc\n", "um");
        git(&fx.worktree, &["checkout", "-q", "--detach"]);

        let err = merge_preview(&fx.worktree).unwrap_err();
        assert!(err.contains("destacado"), "{err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_repo() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-ops-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .unwrap();
            assert!(ok.status.success(), "{args:?}: {:?}", ok);
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "commit.gpgsign", "false"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        fs::write(dir.join("a.txt"), "a\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "init"]);
        dir
    }

    fn status_porcelain(dir: &Path) -> String {
        let raw = run_git(
            {
                let mut c = git_in(dir);
                c.args(["status", "--porcelain"]);
                c
            },
            "status",
        )
        .unwrap();
        String::from_utf8_lossy(&raw).into_owned()
    }

    #[test]
    fn stage_unstage_roundtrip() {
        let repo = temp_repo();
        fs::write(repo.join("a.txt"), "b\n").unwrap();
        stage(&repo, &["a.txt".into()]).unwrap();
        assert!(status_porcelain(&repo).starts_with("M "));
        unstage(&repo, &["a.txt".into()]).unwrap();
        assert!(status_porcelain(&repo).starts_with(" M"));
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn discard_restores_tracked_and_removes_untracked() {
        let repo = temp_repo();
        fs::write(repo.join("a.txt"), "changed\n").unwrap();
        fs::write(repo.join("novo.txt"), "x\n").unwrap();
        discard(&repo, &["a.txt".into(), "novo.txt".into()]).unwrap();
        assert_eq!(status_porcelain(&repo), "");
        assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "a\n");
        assert!(!repo.join("novo.txt").exists());
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn discard_all_cleans_everything_unstaged() {
        let repo = temp_repo();
        fs::write(repo.join("a.txt"), "changed\n").unwrap();
        fs::create_dir_all(repo.join("dir")).unwrap();
        fs::write(repo.join("dir/n.txt"), "x\n").unwrap();
        discard(&repo, &[]).unwrap();
        assert_eq!(status_porcelain(&repo), "");
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn commit_requires_message_and_commits_staged_only() {
        let repo = temp_repo();
        fs::write(repo.join("a.txt"), "b\n").unwrap();
        assert!(commit(&repo, "  ").is_err());
        stage(&repo, &[]).unwrap();
        fs::write(repo.join("a.txt"), "c\n").unwrap();
        commit(&repo, "feat: staged").unwrap();
        // O que não estava staged continua no worktree.
        assert!(status_porcelain(&repo).starts_with(" M"));
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn unstage_of_staged_rename_releases_the_whole_pair() {
        let repo = temp_repo();
        let run = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(ok.status.success(), "{args:?}");
        };
        run(&["mv", "a.txt", "b.txt"]);
        unstage(&repo, &["b.txt".into()]).unwrap();
        let st = status_porcelain(&repo);
        // Nada staged: o par virou deleção não-staged + untracked.
        assert!(
            st.lines()
                .all(|l| l.starts_with(" ") || l.starts_with("??")),
            "{st}"
        );
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn discard_removes_agent_created_nested_repo() {
        let repo = temp_repo();
        let nested = repo.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&nested)
            .args(["init", "-q"])
            .output()
            .unwrap();
        assert!(ok.status.success());
        fs::write(nested.join("x.txt"), "x\n").unwrap();

        discard(&repo, &["nested/".into()]).unwrap();
        assert!(!nested.exists(), "repo aninhado devia ter sido removido");
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn push_refuses_protected_branch_before_touching_network() {
        let repo = temp_repo();
        let err = push(&repo).unwrap_err();
        assert!(err.contains("recusado"), "{err}");
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn paths_with_parent_dir_or_absolute_are_rejected() {
        let repo = temp_repo();
        assert!(stage(&repo, &["../fora.txt".into()]).is_err());
        assert!(discard(&repo, &["/etc/passwd".into()]).is_err());
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn literal_pathspec_does_not_glob() {
        let repo = temp_repo();
        fs::write(repo.join("a[0].txt"), "x\n").unwrap();
        fs::write(repo.join("a0.txt"), "y\n").unwrap();
        stage(&repo, &["a[0].txt".into()]).unwrap();
        let st = status_porcelain(&repo);
        assert!(st.contains("A  a[0].txt"), "{st}");
        assert!(st.contains("?? a0.txt"), "{st}");
        fs::remove_dir_all(&repo).ok();
    }
}
