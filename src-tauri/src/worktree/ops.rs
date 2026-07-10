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

use std::path::Path;

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
