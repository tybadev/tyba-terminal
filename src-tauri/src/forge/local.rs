//! Trazer um PR para a máquina: "Abrir no TYBA" (worktree novo, isolado) e
//! "Checkout" (no checkout do próprio usuário). Só GitHub, só pela UI, sempre
//! depois de confirmação — são commands do Tauri que o agente não alcança.
//!
//! Nada aqui empurra nada: só `git fetch` do ref `refs/pull/<n>/head` e
//! operações locais. O head buscado é conferido contra o `head_sha` que o
//! usuário viu na tela; se o PR andou, recusa com `forge.pr_head_moved`.
//!
//! - **Abrir no TYBA** nunca toca no checkout do usuário: cria um worktree
//!   gerido (`~/.tyba/worktrees`) numa branch `tyba/pr/<n>-<sufixo>` no head
//!   do PR. O nome da branch é do TYBA, nunca o `headRefName` do PR — esse é
//!   escolhido pelo autor (num fork, por terceiros) e poderia coincidir com uma
//!   branch do usuário. O script de setup do repo **não** roda: o conteúdo é
//!   do PR, não do usuário.
//! - **Checkout** troca o checkout do usuário para o head do PR em **HEAD
//!   destacado** (`git switch --detach`): não cria nem move branch, e volta
//!   com `git switch -`. Recusa árvore suja, recusa sobrescrever arquivo
//!   ignorado (um `.env` que o PR versiona) e recusa rodar no worktree de uma
//!   sessão (ali o diff tem base congelada; o caminho é "Abrir no TYBA").

use std::path::{Path, PathBuf};

use super::prs::{action_remote, check_target, ensure_head};
use crate::error::AppError;
use crate::worktree::{self, Worktree};

/// Prefixo das branches criadas por "Abrir no TYBA" para o PR `number`. É ele
/// que acha a sessão que já abriu este PR (`tyba/pr/<n>-<sufixo>`).
pub fn branch_prefix(number: u64) -> String {
    worktree::pr_branch_prefix(number)
}

/// argv do fetch do head do PR: ref fixo montado de um `u64`, sem tags, sem
/// escrever ref nenhuma além do `FETCH_HEAD`.
pub fn fetch_args(number: u64) -> Vec<String> {
    vec![
        "fetch".into(),
        "--no-tags".into(),
        "--quiet".into(),
        "origin".into(),
        "--".into(),
        format!("refs/pull/{number}/head"),
    ]
}

fn failed(detail: impl Into<String>) -> AppError {
    AppError::new("forge.pr_local_failed").with("detail", detail.into())
}

/// Busca o head do PR e confere que é o `head_sha` que o usuário viu.
fn fetch_head(repo: &Path, number: u64, head_sha: &str) -> Result<(), AppError> {
    let out = {
        let mut c = worktree::git_in_net(repo);
        c.args(fetch_args(number));
        c.output().map_err(|e| failed(format!("git fetch: {e}")))?
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(failed(
            crate::session::redact::redact(stderr.trim()).into_owned(),
        ));
    }
    let fetched = {
        let mut c = worktree::git_in(repo);
        c.args(["rev-parse", "--verify", "--quiet", "FETCH_HEAD^{commit}"]);
        c.output()
            .map_err(|e| failed(format!("git rev-parse: {e}")))?
    };
    let fetched = String::from_utf8_lossy(&fetched.stdout).trim().to_string();
    ensure_head(head_sha, &fetched)
}

/// O repo principal de `path` (se `path` é um worktree, o repo dono dele).
fn main_repo(path: &Path) -> Result<PathBuf, AppError> {
    let main = worktree::main_repo_of(path).map_err(failed)?;
    Ok(crate::repo::canonicalize_or(&main))
}

/// "Abrir no TYBA": worktree novo no head do PR. Devolve o worktree e o repo
/// principal (para a sessão nascer com `repo_root`).
pub fn open_worktree(
    repo_root: &Path,
    number: u64,
    head_sha: &str,
) -> Result<(Worktree, PathBuf), AppError> {
    check_target(number, head_sha)?;
    action_remote(repo_root)?;
    let main = main_repo(repo_root)?;
    fetch_head(&main, number, head_sha)?;
    let wt = worktree::create_at(&main, number, head_sha).map_err(failed)?;
    Ok((wt, main))
}

/// "Checkout" no checkout do usuário, em HEAD destacado. Ver o topo do módulo.
pub fn checkout(repo: &Path, number: u64, head_sha: &str) -> Result<(), AppError> {
    check_target(number, head_sha)?;
    if worktree::is_dirty(repo).map_err(failed)? {
        return Err(AppError::new("forge.pr_checkout_dirty"));
    }
    action_remote(repo)?;
    checkout_fetched(repo, number, head_sha)
}

/// A parte local do checkout, depois das validações de alvo e forja.
fn checkout_fetched(repo: &Path, number: u64, head_sha: &str) -> Result<(), AppError> {
    fetch_head(repo, number, head_sha)?;
    // Conferido de novo logo antes de trocar: o fetch leva segundos.
    if worktree::is_dirty(repo).map_err(failed)? {
        return Err(AppError::new("forge.pr_checkout_dirty"));
    }
    // `--no-overwrite-ignore`: o `status` não vê arquivo ignorado, e sem a
    // flag o switch sobrescreveria em silêncio um `.env` do usuário que o PR
    // versiona (e o `git switch -` de volta apagaria o arquivo). `LC_ALL=C`
    // para a recusa do git ser reconhecível.
    let out = {
        let mut c = worktree::git_in_rw(repo);
        c.env("LC_ALL", "C").args([
            "switch",
            "--detach",
            "--no-guess",
            "--no-overwrite-ignore",
            head_sha,
        ]);
        c.output().map_err(|e| failed(format!("git switch: {e}")))?
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("would be overwritten") {
            return Err(AppError::new("forge.pr_checkout_ignored"));
        }
        return Err(failed(stderr.trim().to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn fetch_goes_to_a_fixed_pull_ref_after_double_dash() {
        assert_eq!(
            fetch_args(42),
            [
                "fetch",
                "--no-tags",
                "--quiet",
                "origin",
                "--",
                "refs/pull/42/head"
            ]
        );
    }

    #[test]
    fn branch_prefix_is_the_tyba_namespace() {
        assert_eq!(branch_prefix(7), "tyba/pr/7-");
        assert!(!"tyba/pr/70-abcd1234".starts_with(&branch_prefix(7)));
        // Uma task chamada "PR 7" vira `tyba/pr-7-…`: fora do namespace.
        let task = format!("tyba/{}-abcd1234", worktree::slugify("PR 7 conserta"));
        assert!(!task.starts_with(&branch_prefix(7)), "{task}");
    }

    fn repo_with_origin(origin: &str) -> tempfile::TempDir {
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
        dir
    }

    #[test]
    fn invalid_target_is_refused_before_touching_git_or_network() {
        let dir = repo_with_origin("git@github.com:exemplo/app.git");
        assert!(open_worktree(dir.path(), 0, SHA).is_err());
        assert_eq!(
            open_worktree(dir.path(), 1, "HEAD").unwrap_err().code,
            "forge.pr_invalid_head"
        );
        assert_eq!(
            checkout(dir.path(), 1, "--orphan").unwrap_err().code,
            "forge.pr_invalid_head"
        );
    }

    #[test]
    fn dirty_tree_refuses_checkout() {
        let dir = repo_with_origin("https://gitlab.com/group/app.git");
        std::fs::write(dir.path().join("rascunho.txt"), "x").unwrap();
        assert_eq!(
            checkout(dir.path(), 1, SHA).unwrap_err().code,
            "forge.pr_checkout_dirty"
        );
    }

    #[test]
    fn gitlab_has_no_pr_checkout() {
        let dir = repo_with_origin("https://gitlab.com/group/app.git");
        assert_eq!(
            checkout(dir.path(), 1, SHA).unwrap_err().code,
            "forge.pr_actions_unsupported"
        );
        assert_eq!(
            open_worktree(dir.path(), 1, SHA).unwrap_err().code,
            "forge.pr_actions_unsupported"
        );
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// `origin` local com `refs/pull/7/head` (o que o GitHub publica) e um
    /// clone na `main`: o fetch traz o head, confere o sha e o checkout fica
    /// destacado, sem branch nova.
    fn origin_with_pull_ref() -> (tempfile::TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q", "-b", "main"]);
        for (k, v) in [
            ("user.email", "t@t.invalid"),
            ("user.name", "t"),
            ("commit.gpgsign", "false"),
        ] {
            git(&origin, &["config", k, v]);
        }
        std::fs::write(origin.join("a.txt"), "a\n").unwrap();
        git(&origin, &["add", "-A"]);
        git(&origin, &["commit", "-qm", "init"]);
        git(&origin, &["switch", "-q", "-c", "contribuicao"]);
        std::fs::write(origin.join("pr.txt"), "pr\n").unwrap();
        git(&origin, &["add", "-A"]);
        git(&origin, &["commit", "-qm", "pr"]);
        let pr_sha = git(&origin, &["rev-parse", "HEAD"]);
        git(&origin, &["update-ref", "refs/pull/7/head", &pr_sha]);
        git(&origin, &["switch", "-q", "main"]);
        git(&origin, &["branch", "-qD", "contribuicao"]);
        let clone = dir.path().join("clone");
        git(
            dir.path(),
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        (dir, clone, pr_sha)
    }

    #[test]
    fn checkout_fetches_the_pull_ref_and_detaches_at_the_pr_head() {
        let (_dir, clone, pr_sha) = origin_with_pull_ref();
        let heads = |c: &Path| git(c, &["for-each-ref", "--format=%(refname)", "refs/heads"]);
        let branches_before = heads(&clone);
        checkout_fetched(&clone, 7, &pr_sha).expect("checkout");
        assert_eq!(git(&clone, &["rev-parse", "HEAD"]), pr_sha);
        assert_eq!(
            git(&clone, &["branch", "--show-current"]),
            "",
            "HEAD destacado"
        );
        assert_eq!(
            heads(&clone),
            branches_before,
            "nenhuma branch criada ou movida"
        );
        assert!(clone.join("pr.txt").exists());
    }

    #[test]
    fn checkout_refuses_to_overwrite_an_ignored_file_the_pr_tracks() {
        let (dir, clone, _) = origin_with_pull_ref();
        // PR que versiona `.env`, num repo onde o `.env` do usuário é ignorado.
        let origin = dir.path().join("origin");
        std::fs::write(origin.join(".gitignore"), ".env\n").unwrap();
        git(&origin, &["add", "-A"]);
        git(&origin, &["commit", "-qm", "ignora env"]);
        git(&origin, &["switch", "-q", "-c", "vaza-env"]);
        std::fs::write(origin.join(".env"), "TOKEN=do-pr\n").unwrap();
        git(&origin, &["add", "-f", ".env"]);
        git(&origin, &["commit", "-qm", "pr com env"]);
        let pr_sha = git(&origin, &["rev-parse", "HEAD"]);
        git(&origin, &["update-ref", "refs/pull/9/head", &pr_sha]);
        git(&origin, &["switch", "-q", "main"]);
        git(&clone, &["pull", "-q", "--ff-only"]);
        std::fs::write(clone.join(".env"), "TOKEN=do-usuario\n").unwrap();
        let before = git(&clone, &["rev-parse", "HEAD"]);

        let err = checkout_fetched(&clone, 9, &pr_sha).unwrap_err();

        assert_eq!(err.code, "forge.pr_checkout_ignored");
        assert_eq!(
            std::fs::read_to_string(clone.join(".env")).unwrap(),
            "TOKEN=do-usuario\n"
        );
        assert_eq!(git(&clone, &["rev-parse", "HEAD"]), before, "nada trocou");
        assert_eq!(git(&clone, &["branch", "--show-current"]), "main");
    }

    #[test]
    fn a_pr_that_moved_since_the_user_looked_is_refused() {
        let (_dir, clone, _pr_sha) = origin_with_pull_ref();
        let main_sha = git(&clone, &["rev-parse", "HEAD"]);
        let err = checkout_fetched(&clone, 7, SHA).unwrap_err();
        assert_eq!(err.code, "forge.pr_head_moved");
        assert_eq!(git(&clone, &["rev-parse", "HEAD"]), main_sha, "nada trocou");
    }

    #[test]
    fn missing_pull_ref_fails_without_touching_the_checkout() {
        let (_dir, clone, pr_sha) = origin_with_pull_ref();
        let err = checkout_fetched(&clone, 8, &pr_sha).unwrap_err();
        assert_eq!(err.code, "forge.pr_local_failed");
        assert_eq!(git(&clone, &["branch", "--show-current"]), "main");
    }
}
