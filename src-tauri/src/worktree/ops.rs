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

use super::diff::DiffScope;
use super::{git_in, git_in_net, git_in_rw, git_text, run_git};
use crate::error::AppError;

fn merge_failed(detail: impl Into<String>) -> AppError {
    AppError::new("merge.failed").with("detail", detail.into())
}

fn push_failed(detail: impl Into<String>) -> AppError {
    AppError::new("push.failed").with("detail", detail.into())
}

pub(crate) fn ensure_relative(path: &str) -> Result<(), String> {
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
    let mut c = git_in_rw(worktree);
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
    let mut c = git_in_rw(worktree);
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
                let mut c = git_in_rw(worktree);
                c.args(["restore", "--worktree", "--", "."]);
                c
            },
            "git restore",
        )?;
        // -ff: repo aninhado criado por agente também sai — o usuário já
        // confirmou o discard em dois cliques; sumir em silêncio é pior.
        run_git(
            {
                let mut c = git_in_rw(worktree);
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
        let mut c = git_in_rw(worktree);
        c.args(["restore", "--worktree", "--"]);
        c.args(literal_specs(&restore)?);
        run_git(c, "git restore")?;
    }
    if !clean.is_empty() {
        let mut c = git_in_rw(worktree);
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
    let mut c = git_in_rw(worktree);
    c.args(["commit", "-m", message]);
    run_git(c, "git commit")?;
    Ok(())
}

fn git_failed(code: &str, detail: impl Into<String>) -> AppError {
    AppError::new(code).with("detail", detail.into())
}

/// `true`/`false` do `merge-base --is-ancestor`; outro código (ref que não
/// existe, sem upstream) é erro.
fn is_ancestor(worktree: &Path, ancestor: &str, of: &str) -> Result<bool, String> {
    let out = {
        let mut c = git_in(worktree);
        c.args(["merge-base", "--is-ancestor", ancestor, of]);
        c.output().map_err(|e| format!("git merge-base: {e}"))?
    };
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "git merge-base: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// `git commit --amend -m`, com as regras do `commit`. Recusa se o HEAD já
/// está no upstream: reescrever o que foi publicado obriga a um force-push.
/// Sem upstream (ou HEAD solto), não há o que proteger e permite.
pub fn commit_amend(worktree: &Path, message: &str) -> Result<(), AppError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(git_failed("git.amend_failed", "mensagem de commit vazia"));
    }
    if is_ancestor(worktree, "HEAD", "@{upstream}").unwrap_or(false) {
        return Err(AppError::new("git.amend_published"));
    }
    let mut c = git_in_rw(worktree);
    c.args(["commit", "--amend", "-m", message]);
    run_git(c, "git commit --amend").map_err(|e| git_failed("git.amend_failed", e))?;
    Ok(())
}

/// Equivale a `git pull --ff-only`, em dois passos: o `fetch` roda no perfil
/// de rede e o avanço (`merge --ff-only`, que escreve arquivos e pode rodar
/// filtro de conteúdo) roda na jaula de escrita — um `pull` inteiro no perfil
/// de rede escaparia dela. Só por clique; nunca rebase nem merge commit.
pub fn pull(worktree: &Path) -> Result<String, AppError> {
    let upstream = git_text(
        {
            let mut c = git_in(worktree);
            c.args([
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ]);
            c
        },
        "git rev-parse @{upstream}",
    )
    .map_err(|_| AppError::new("git.no_upstream"))?;
    run_git(
        {
            let mut c = git_in_net(worktree);
            c.arg("fetch");
            c
        },
        "git fetch",
    )
    .map_err(|e| git_failed("git.pull_failed", e))?;
    let failed = |e: String| git_failed("git.pull_failed", e);
    if is_ancestor(worktree, "@{upstream}", "HEAD").map_err(failed)? {
        return Ok("já em dia".into());
    }
    if !is_ancestor(worktree, "HEAD", "@{upstream}").map_err(failed)? {
        return Err(AppError::new("git.pull_not_ff").with("upstream", upstream));
    }
    let before = super::head_sha(worktree).map_err(failed)?;
    run_git(
        {
            let mut c = git_in_rw(worktree);
            c.args(["merge", "--ff-only", "@{upstream}"]);
            c
        },
        "git merge --ff-only",
    )
    .map_err(failed)?;
    let count = git_text(
        {
            let mut c = git_in(worktree);
            c.args(["rev-list", "--count", &format!("{before}..HEAD")]);
            c
        },
        "git rev-list --count",
    )
    .map_err(failed)?;
    Ok(match count.as_str() {
        "0" => "já em dia".into(),
        "1" => "1 commit".into(),
        n => format!("{n} commits"),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HunkAction {
    Stage,
    Unstage,
    Discard,
}

fn hunk_invalid(detail: impl Into<String>) -> AppError {
    git_failed("git.hunk_invalid", detail)
}

fn hunk_unsupported(kind: &str) -> AppError {
    AppError::new("git.hunk_unsupported").with("kind", kind)
}

/// Cabeçalho do arquivo (`diff --git`, `index`, `---`, `+++`) mais o trecho
/// `index`, em bytes crus: CRLF e "\ No newline at end of file" sobrevivem.
/// Recorta como `parse_unified_hunks`: um trecho começa em `@@`, e o diff
/// acaba no segundo `diff --git`.
fn single_hunk_patch(raw: &[u8], index: usize) -> Option<Vec<u8>> {
    let mut patch = Vec::new();
    let mut hunk = Vec::new();
    let mut current: Option<usize> = None;
    let mut past_first_file = false;
    for line in raw.split_inclusive(|b| *b == b'\n') {
        if line.starts_with(b"diff --git ") {
            if past_first_file {
                break;
            }
            past_first_file = true;
        }
        if line.starts_with(b"@@") {
            current = Some(current.map_or(0, |n| n + 1));
        }
        match current {
            None => patch.extend_from_slice(line),
            Some(n) if n == index => hunk.extend_from_slice(line),
            Some(n) if n > index => break,
            Some(_) => {}
        }
    }
    if hunk.is_empty() {
        return None;
    }
    patch.extend_from_slice(&hunk);
    Some(patch)
}

fn is_staged_rename(worktree: &Path, path: &str) -> Result<bool, String> {
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
    Ok(super::diff::parse_name_status_z(&raw)
        .into_iter()
        .any(|(_, old, new)| old.is_some_and(|old| old == path || new == path)))
}

/// Age num trecho só: stage/discard no escopo unstaged, unstage no staged.
/// Recalcula o diff com a mesma fonte do `session_diff_hunks` e recusa se o
/// trecho `hunk_index` não tem mais o `header` que a UI mostrou.
pub fn hunk_action(
    worktree: &Path,
    base_ref: &str,
    path: &str,
    scope: DiffScope,
    hunk_index: usize,
    header: &str,
    action: HunkAction,
) -> Result<(), AppError> {
    let valid = matches!(
        (action, scope),
        (HunkAction::Stage | HunkAction::Discard, DiffScope::Unstaged)
            | (HunkAction::Unstage, DiffScope::Staged)
    );
    if !valid {
        return Err(hunk_invalid(format!(
            "{action:?} não vale no escopo {scope:?}"
        )));
    }
    ensure_relative(path).map_err(hunk_invalid)?;
    let raw = super::diff::file_diff_raw(worktree, base_ref, scope, path, None)
        .map_err(|e| git_failed("git.hunk_apply_failed", e))?;
    if raw.is_empty() {
        let untracked = scope == DiffScope::Unstaged
            && super::diff::is_untracked(worktree, path).map_err(hunk_invalid)?;
        return Err(if untracked {
            hunk_unsupported("untracked")
        } else {
            AppError::new("git.hunk_changed")
        });
    }
    let parsed = super::diff::parse_unified_hunks(&String::from_utf8_lossy(&raw));
    if parsed.binary {
        return Err(hunk_unsupported("binary"));
    }
    let renamed = raw
        .split(|b| *b == b'\n')
        .take_while(|l| !l.starts_with(b"@@"))
        .any(|l| l.starts_with(b"rename from ") || l.starts_with(b"copy from "));
    if renamed
        || (scope == DiffScope::Staged && is_staged_rename(worktree, path).map_err(hunk_invalid)?)
    {
        return Err(hunk_unsupported("rename"));
    }
    if parsed.hunks.get(hunk_index).map(|h| h.header.as_str()) != Some(header) {
        return Err(AppError::new("git.hunk_changed"));
    }
    let patch =
        single_hunk_patch(&raw, hunk_index).ok_or_else(|| AppError::new("git.hunk_changed"))?;
    let mut c = git_in_rw(worktree);
    c.arg("apply");
    match action {
        HunkAction::Stage => c.arg("--cached"),
        HunkAction::Unstage => c.args(["--cached", "--reverse"]),
        HunkAction::Discard => c.arg("--reverse"),
    };
    c.arg("-");
    super::log::run_git_stdin(c, &patch, "git apply")
        .map_err(|e| git_failed("git.hunk_apply_failed", e))?;
    Ok(())
}

const PROTECTED_BRANCHES: [&str; 2] = ["main", "master"];

/// `git push -u origin <branch>` — recusa main/master sempre.
pub fn push(worktree: &Path) -> Result<String, AppError> {
    let out = {
        let mut c = git_in(worktree);
        c.args(["symbolic-ref", "--short", "-q", "HEAD"]);
        c.output()
            .map_err(|e| push_failed(format!("git symbolic-ref: {e}")))?
    };
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || branch.is_empty() {
        return Err(AppError::new("push.detached_head"));
    }
    if PROTECTED_BRANCHES.contains(&branch.as_str()) {
        return Err(AppError::new("push.protected_branch").with("branch", branch));
    }
    let out = {
        let mut c = git_in_net(worktree);
        c.args(["push", "-u", "origin", &branch]);
        c.output()
            .map_err(|e| push_failed(format!("git push: {e}")))?
    };
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        return Err(push_failed(format!("git push: {stderr}")));
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

fn current_branch(repo: &Path, what: &str) -> Result<String, AppError> {
    let out = {
        let mut c = git_in(repo);
        c.args(["symbolic-ref", "--short", "-q", "HEAD"]);
        c.output()
            .map_err(|e| merge_failed(format!("git symbolic-ref: {e}")))?
    };
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || branch.is_empty() {
        return Err(AppError::new("merge.detached_head").with("what", what));
    }
    Ok(branch)
}

fn nul_fields(raw: &[u8]) -> Vec<String> {
    raw.split(|b| *b == 0)
        .filter(|f| !f.is_empty())
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect()
}

fn merge_tree(main: &Path, base: &str, source: &str) -> Result<(String, Vec<String>), AppError> {
    let out = {
        let mut c = git_in_rw(main);
        c.args([
            "merge-tree",
            "--write-tree",
            "--name-only",
            "-z",
            base,
            source,
        ]);
        c.output()
            .map_err(|e| merge_failed(format!("git merge-tree: {e}")))?
    };
    if !matches!(out.status.code(), Some(0) | Some(1)) {
        return Err(merge_failed(format!(
            "git merge-tree: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let mut fields = out.stdout.split(|b| *b == 0);
    let tree = fields
        .next()
        .map(|f| String::from_utf8_lossy(f).trim().to_string())
        .unwrap_or_default();
    if tree.is_empty() {
        return Err(merge_failed("git merge-tree não produziu árvore"));
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

fn plan(worktree: &Path) -> Result<MergePlan, AppError> {
    let main = super::main_repo_of(worktree).map_err(merge_failed)?;
    let base_branch = current_branch(&main, "o repo principal")?;
    let source_branch = current_branch(worktree, "a sessão")?;
    if base_branch == source_branch {
        return Err(AppError::new("merge.same_branch").with("branch", base_branch));
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
    )
    .map_err(merge_failed)?;
    let (_, conflicts) = merge_tree(&main, &base_branch, &source_branch)?;
    Ok(MergePlan {
        preview: MergePreview {
            commits: super::ahead_count(worktree, &base_branch).map_err(merge_failed)? as usize,
            files_changed: nul_fields(&raw).len(),
            conflicts,
            base_dirty: super::is_dirty(&main).map_err(merge_failed)?,
            base_branch,
            source_branch,
        },
        main,
    })
}

pub fn merge_preview(worktree: &Path) -> Result<MergePreview, AppError> {
    Ok(plan(worktree)?.preview)
}

/// Traz a base pra branch da sessão DENTRO do worktree, parando no conflito:
/// o agente resolve na própria jaula e o merge local vira limpo depois. A
/// direção inverte de propósito — o checkout do dono nunca é tocado.
pub fn materialize_conflict(worktree: &Path) -> Result<(), AppError> {
    let plan = plan(worktree)?;
    if super::is_dirty(worktree).map_err(merge_failed)? {
        return Err(AppError::new("merge.worktree_dirty"));
    }
    let out = {
        let mut c = git_in_rw(worktree);
        c.args(["merge", "--no-edit", &plan.preview.base_branch]);
        c.output()
            .map_err(|e| merge_failed(format!("git merge: {e}")))?
    };
    if out.status.success() {
        return Ok(());
    }
    match super::conflicts::session_conflicts(worktree) {
        Ok(Some(state)) if !state.files.is_empty() => Ok(()),
        _ => Err(merge_failed(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        )),
    }
}

pub fn merge_into_base(
    worktree: &Path,
    strategy: MergeStrategy,
    message: Option<&str>,
) -> Result<String, AppError> {
    let MergePlan { main, preview } = plan(worktree)?;

    if preview.base_dirty {
        return Err(AppError::new("merge.base_dirty").with("branch", preview.base_branch));
    }
    if !preview.conflicts.is_empty() {
        return Err(AppError::new("merge.conflict").with("files", preview.conflicts.join(", ")));
    }
    if preview.commits == 0 {
        return Err(AppError::new("merge.nothing")
            .with("source", preview.source_branch)
            .with("base", preview.base_branch));
    }

    let base_head = super::head_sha(&main).map_err(merge_failed)?;
    let source_head = super::head_sha(worktree).map_err(merge_failed)?;

    let (tree, conflicts) = merge_tree(&main, &base_head, &source_head)?;
    if !conflicts.is_empty() {
        return Err(AppError::new("merge.conflict").with("files", conflicts.join(", ")));
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

    let mut c = git_in_rw(&main);
    c.args(["commit-tree", &tree, "-p", &base_head]);
    if matches!(strategy, MergeStrategy::MergeCommit) {
        c.args(["-p", &source_head]);
    }
    c.args(["-m", &message]);
    let merged = git_text(c, "git commit-tree").map_err(merge_failed)?;
    if merged.is_empty() {
        return Err(merge_failed("git commit-tree não produziu commit"));
    }

    let now_head = super::head_sha(&main).map_err(merge_failed)?;
    if now_head != base_head {
        return Err(AppError::new("merge.base_moved").with("branch", preview.base_branch));
    }
    run_git(
        {
            let mut c = git_in_rw(&main);
            c.args(["merge", "--ff-only", &merged]);
            c
        },
        "git merge --ff-only",
    )
    .map_err(|_| AppError::new("merge.base_moved").with("branch", preview.base_branch.clone()))?;
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
    fn materialize_brings_the_conflict_into_the_worktree_only() {
        let fx = fixture("materialize");
        work(&fx, "a\nsessao\nc\n", "sessao");
        fs::write(fx.repo.join("f.txt"), "a\nbase\nc\n").unwrap();
        git(&fx.repo, &["commit", "-aqm", "base avança"]);

        materialize_conflict(&fx.worktree).unwrap();

        let state = super::super::conflicts::session_conflicts(&fx.worktree)
            .unwrap()
            .expect("conflito materializado");
        assert_eq!(state.files.len(), 1);
        assert_eq!(state.files[0].path, "f.txt");
        assert_eq!(
            super::super::conflicts::session_conflicts(&fx.repo).unwrap(),
            None,
            "o checkout do dono não pode ser tocado"
        );
    }

    #[test]
    fn materialize_refuses_a_dirty_worktree() {
        let fx = fixture("materialize-dirty");
        work(&fx, "a\nsessao\nc\n", "sessao");
        fs::write(fx.repo.join("f.txt"), "a\nbase\nc\n").unwrap();
        git(&fx.repo, &["commit", "-aqm", "base avança"]);
        fs::write(fx.worktree.join("outro.txt"), "sujo\n").unwrap();

        let err = materialize_conflict(&fx.worktree).unwrap_err();
        assert_eq!(err.code, "merge.worktree_dirty", "{err}");
    }

    #[test]
    fn materialize_without_conflict_completes_the_merge() {
        let fx = fixture("materialize-clean");
        work(&fx, "a\nsessao\nc\n", "sessao");
        fs::write(fx.repo.join("outro.txt"), "mudou sem conflito\n").unwrap();
        git(&fx.repo, &["commit", "-aqm", "base avança limpa"]);

        materialize_conflict(&fx.worktree).unwrap();

        assert_eq!(
            super::super::conflicts::session_conflicts(&fx.worktree).unwrap(),
            None
        );
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
        assert_eq!(err.code, "merge.conflict", "{err}");
        assert!(err.params["files"].contains("f.txt"), "{err}");

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
        assert_eq!(err.code, "merge.base_dirty", "{err}");

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
        assert_eq!(err.code, "merge.nothing", "{err}");
        assert_eq!(head(&fx.repo), before);
    }

    #[test]
    fn merging_the_main_repo_into_itself_is_refused() {
        let fx = fixture("selfmerge");
        let err = merge_preview(&fx.repo).unwrap_err();
        assert_eq!(err.code, "merge.same_branch", "{err}");
    }

    #[test]
    fn detached_session_head_is_refused() {
        let fx = fixture("detached");
        work(&fx, "a\nX\nc\n", "um");
        git(&fx.worktree, &["checkout", "-q", "--detach"]);

        let err = merge_preview(&fx.worktree).unwrap_err();
        assert_eq!(err.code, "merge.detached_head", "{err}");
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
        assert_eq!(err.code, "push.protected_branch", "{err}");
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn paths_with_parent_dir_or_absolute_are_rejected() {
        let repo = temp_repo();
        assert!(stage(&repo, &["../fora.txt".into()]).is_err());
        assert!(discard(&repo, &["/etc/passwd".into()]).is_err());
        fs::remove_dir_all(&repo).ok();
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn sha(dir: &Path, what: &str) -> String {
        git_text(
            {
                let mut c = git_in(dir);
                c.args(["rev-parse", what]);
                c
            },
            "rev-parse",
        )
        .unwrap()
    }

    /// Clone de `temp_repo` com upstream em origin/main.
    fn cloned(origin: &Path) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-ops-clone-{}", uuid::Uuid::new_v4()));
        git_ok(
            Path::new("."),
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                dir.to_str().unwrap(),
            ],
        );
        git_ok(&dir, &["config", "commit.gpgsign", "false"]);
        git_ok(&dir, &["config", "user.email", "t@t"]);
        git_ok(&dir, &["config", "user.name", "t"]);
        dir
    }

    #[test]
    fn amend_rewrites_a_local_commit_and_refuses_a_published_one() {
        let origin = temp_repo();
        let repo = cloned(&origin);
        // HEAD == origin/main: já publicado.
        let err = commit_amend(&repo, "outra mensagem").unwrap_err();
        assert_eq!(err.code, "git.amend_published", "{err}");

        fs::write(repo.join("a.txt"), "local\n").unwrap();
        git_ok(&repo, &["commit", "-qam", "local"]);
        assert_eq!(
            commit_amend(&repo, "  ").unwrap_err().code,
            "git.amend_failed"
        );
        let before = sha(&repo, "HEAD");
        commit_amend(&repo, "local reescrito").unwrap();
        assert_ne!(sha(&repo, "HEAD"), before);
        assert_eq!(sha(&repo, "HEAD~1"), sha(&repo, "origin/main"));

        // Sem upstream, permite.
        let solo = temp_repo();
        commit_amend(&solo, "init reescrito").unwrap();
        for d in [origin, repo, solo] {
            fs::remove_dir_all(&d).ok();
        }
    }

    #[test]
    fn pull_fast_forwards_and_refuses_divergence_or_missing_upstream() {
        let origin = temp_repo();
        let repo = cloned(&origin);
        assert_eq!(pull(&repo).unwrap(), "já em dia");

        for (i, msg) in ["um", "dois"].iter().enumerate() {
            fs::write(origin.join("a.txt"), format!("{i}\n")).unwrap();
            git_ok(&origin, &["commit", "-qam", msg]);
        }
        assert_eq!(pull(&repo).unwrap(), "2 commits");
        assert_eq!(sha(&repo, "HEAD"), sha(&origin, "HEAD"));

        fs::write(origin.join("a.txt"), "remoto\n").unwrap();
        git_ok(&origin, &["commit", "-qam", "remoto"]);
        fs::write(repo.join("b.txt"), "local\n").unwrap();
        git_ok(&repo, &["add", "-A"]);
        git_ok(&repo, &["commit", "-qm", "local"]);
        let local = sha(&repo, "HEAD");
        assert_eq!(pull(&repo).unwrap_err().code, "git.pull_not_ff");
        assert_eq!(sha(&repo, "HEAD"), local, "pull recusado não mexe no HEAD");

        let solo = temp_repo();
        assert_eq!(pull(&solo).unwrap_err().code, "git.no_upstream");
        for d in [origin, repo, solo] {
            fs::remove_dir_all(&d).ok();
        }
    }

    const TWENTY: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12\nl13\nl14\nl15\nl16\nl17\nl18\nl19\nl20\n";

    /// `f.txt` commitado com 20 linhas e duas mudanças longe uma da outra:
    /// dois trechos no escopo unstaged.
    fn two_hunks_repo() -> PathBuf {
        let repo = temp_repo();
        fs::write(repo.join("f.txt"), TWENTY).unwrap();
        git_ok(&repo, &["add", "-A"]);
        git_ok(&repo, &["commit", "-qm", "f"]);
        let edited = TWENTY
            .replace("l2\n", "DOIS\n")
            .replace("l19\n", "DEZENOVE\n");
        fs::write(repo.join("f.txt"), edited).unwrap();
        repo
    }

    fn hunks(repo: &Path, scope: DiffScope) -> Vec<super::super::diff::Hunk> {
        super::super::diff::file_hunks(repo, "HEAD", scope, "f.txt", None)
            .unwrap()
            .hunks
    }

    fn act(
        repo: &Path,
        scope: DiffScope,
        index: usize,
        action: HunkAction,
    ) -> Result<(), AppError> {
        let header = hunks(repo, scope)[index].header.clone();
        hunk_action(repo, "HEAD", "f.txt", scope, index, &header, action)
    }

    #[test]
    fn stage_one_hunk_leaves_the_other_unstaged_and_unstage_brings_it_back() {
        let repo = two_hunks_repo();
        assert_eq!(hunks(&repo, DiffScope::Unstaged).len(), 2);

        act(&repo, DiffScope::Unstaged, 0, HunkAction::Stage).unwrap();
        let staged = hunks(&repo, DiffScope::Staged);
        let unstaged = hunks(&repo, DiffScope::Unstaged);
        assert_eq!(staged.len(), 1);
        assert_eq!(unstaged.len(), 1);
        assert!(staged[0].lines.iter().any(|l| l.text == "DOIS"));
        assert!(unstaged[0].lines.iter().any(|l| l.text == "DEZENOVE"));

        act(&repo, DiffScope::Staged, 0, HunkAction::Unstage).unwrap();
        assert!(hunks(&repo, DiffScope::Staged).is_empty());
        assert_eq!(hunks(&repo, DiffScope::Unstaged).len(), 2);
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn discard_one_hunk_removes_only_that_change() {
        let repo = two_hunks_repo();
        act(&repo, DiffScope::Unstaged, 1, HunkAction::Discard).unwrap();
        assert_eq!(
            fs::read_to_string(repo.join("f.txt")).unwrap(),
            TWENTY.replace("l2\n", "DOIS\n")
        );
        fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn hunk_action_refuses_changed_header_bad_scope_and_unsupported_files() {
        let repo = two_hunks_repo();
        let header = hunks(&repo, DiffScope::Unstaged)[0].header.clone();
        let err = hunk_action(
            &repo,
            "HEAD",
            "f.txt",
            DiffScope::Unstaged,
            1,
            &header,
            HunkAction::Stage,
        )
        .unwrap_err();
        assert_eq!(err.code, "git.hunk_changed");
        let err = hunk_action(
            &repo,
            "HEAD",
            "f.txt",
            DiffScope::Unstaged,
            9,
            &header,
            HunkAction::Stage,
        )
        .unwrap_err();
        assert_eq!(err.code, "git.hunk_changed");
        assert!(
            hunks(&repo, DiffScope::Staged).is_empty(),
            "recusa não mexe no index"
        );

        for (scope, action) in [
            (DiffScope::Staged, HunkAction::Stage),
            (DiffScope::Staged, HunkAction::Discard),
            (DiffScope::Unstaged, HunkAction::Unstage),
            (DiffScope::Committed, HunkAction::Stage),
        ] {
            let err = hunk_action(&repo, "HEAD", "f.txt", scope, 0, &header, action).unwrap_err();
            assert_eq!(err.code, "git.hunk_invalid", "{scope:?} {action:?}");
        }
        for bad in ["../f.txt", "/etc/passwd"] {
            let err = hunk_action(
                &repo,
                "HEAD",
                bad,
                DiffScope::Unstaged,
                0,
                &header,
                HunkAction::Stage,
            )
            .unwrap_err();
            assert_eq!(err.code, "git.hunk_invalid", "{bad}");
        }

        fs::write(repo.join("novo.txt"), "x\n").unwrap();
        let err = hunk_action(
            &repo,
            "HEAD",
            "novo.txt",
            DiffScope::Unstaged,
            0,
            "@@ -0,0 +1 @@",
            HunkAction::Stage,
        )
        .unwrap_err();
        assert_eq!(err.code, "git.hunk_unsupported");
        assert_eq!(
            err.params.get("kind").map(String::as_str),
            Some("untracked")
        );

        fs::write(repo.join("bin.dat"), [0u8, 1, 2, 3]).unwrap();
        git_ok(&repo, &["add", "bin.dat"]);
        git_ok(&repo, &["commit", "-qm", "bin"]);
        fs::write(repo.join("bin.dat"), [0u8, 9, 9, 9]).unwrap();
        let err = hunk_action(
            &repo,
            "HEAD",
            "bin.dat",
            DiffScope::Unstaged,
            0,
            "x",
            HunkAction::Stage,
        )
        .unwrap_err();
        assert_eq!(err.params.get("kind").map(String::as_str), Some("binary"));

        git_ok(&repo, &["mv", "a.txt", "renomeado.txt"]);
        let err = hunk_action(
            &repo,
            "HEAD",
            "renomeado.txt",
            DiffScope::Staged,
            0,
            "x",
            HunkAction::Unstage,
        )
        .unwrap_err();
        assert_eq!(err.params.get("kind").map(String::as_str), Some("rename"));
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
