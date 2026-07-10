//! Worktrees: boundary de escrita de cada sessão de agente (Fase 3).
//!
//! Regras dos docs:
//! - shell-out para o binário `git` (não git2/gitoxide no MVP)
//! - sempre `-z`, `--no-color`, `-c core.quotePath=false`
//! - three-dot semantics: diff contra `base_ref` salvo na criação
//! - `git stash` NUNCA na automação (compartilhado entre worktrees)

pub mod diff;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use sha2::Digest;

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
    /// tyba/<slug>-<sufixo curto>
    pub branch: String,
    /// sha da base no momento da criação (base do three-dot diff)
    pub base_ref: String,
    pub dirty: bool,
    pub ahead: u32,
}

fn run_git(mut cmd: Command, what: &str) -> Result<Vec<u8>, String> {
    let out = cmd.output().map_err(|e| format!("{what}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{what}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

fn git_text(cmd: Command, what: &str) -> Result<String, String> {
    Ok(String::from_utf8_lossy(&run_git(cmd, what)?)
        .trim()
        .to_string())
}

pub fn managed_root() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or("HOME indisponível")?;
    Ok(PathBuf::from(home).join(".tyba").join("worktrees"))
}

pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = true;
    for c in title.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "task".into()
    } else {
        slug
    }
}

fn short_suffix() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

pub fn head_sha(repo: &Path) -> Result<String, String> {
    git_text(
        {
            let mut c = git_in(repo);
            c.args(["rev-parse", "HEAD"]);
            c
        },
        "git rev-parse HEAD",
    )
}

pub fn is_dirty(worktree: &Path) -> Result<bool, String> {
    let out = run_git(
        {
            let mut c = git_in(worktree);
            c.args(["status", "--porcelain", "-z"]);
            c
        },
        "git status",
    )?;
    Ok(!out.is_empty())
}

/// Commits no worktree que não existem em `upstream` (sha ou ref).
pub fn ahead_count(worktree: &Path, upstream: &str) -> Result<u32, String> {
    git_text(
        {
            let mut c = git_in(worktree);
            c.args(["rev-list", "--count", &format!("{upstream}..HEAD")]);
            c
        },
        "git rev-list --count",
    )?
    .parse()
    .map_err(|e| format!("rev-list count: {e}"))
}

pub fn create(repo_root: &Path, title: &str) -> Result<Worktree, String> {
    create_in(&managed_root()?, repo_root, title)
}

fn create_in(managed: &Path, repo_root: &Path, title: &str) -> Result<Worktree, String> {
    let repo_root = crate::repo::canonicalize_or(repo_root);
    let base_ref = head_sha(&repo_root)?;
    let slug = slugify(title);
    let suffix = short_suffix();
    let branch = format!("tyba/{slug}-{suffix}");
    let repo_name = repo_root
        .file_name()
        .map(|n| slugify(&n.to_string_lossy()))
        .unwrap_or_else(|| "repo".into());
    let path = managed.join(repo_name).join(format!("{slug}-{suffix}"));
    std::fs::create_dir_all(path.parent().unwrap_or(managed))
        .map_err(|e| format!("mkdir worktrees: {e}"))?;

    let mut cmd = git_in(&repo_root);
    cmd.arg("worktree")
        .arg("add")
        .arg(&path)
        .arg("-b")
        .arg(&branch)
        .arg(&base_ref);
    run_git(cmd, "git worktree add")?;

    Ok(Worktree {
        path,
        branch,
        base_ref,
        dirty: false,
        ahead: 0,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>,
}

pub fn parse_worktree_list(porcelain: &str) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    for record in porcelain.split("\n\n").filter(|r| !r.trim().is_empty()) {
        let mut path = None;
        let mut head = String::new();
        let mut branch = None;
        for line in record.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if let Some(h) = line.strip_prefix("HEAD ") {
                head = h.to_string();
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
            }
        }
        if let Some(path) = path {
            entries.push(WorktreeEntry { path, head, branch });
        }
    }
    entries
}

pub fn list(repo: &Path) -> Result<Vec<WorktreeEntry>, String> {
    let out = git_text(
        {
            let mut c = git_in(repo);
            c.args(["worktree", "list", "--porcelain"]);
            c
        },
        "git worktree list",
    )?;
    Ok(parse_worktree_list(&out))
}

/// Repo principal de um worktree: parent do `--git-common-dir`.
pub fn main_repo_of(worktree: &Path) -> Result<PathBuf, String> {
    let common =
        crate::repo::git_path(worktree, "--git-common-dir").ok_or("git-common-dir indisponível")?;
    common
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("git-common-dir sem parent: {}", common.display()))
}

pub fn is_managed(path: &Path) -> bool {
    managed_root()
        .map(|root| {
            crate::repo::canonicalize_or(path).starts_with(crate::repo::canonicalize_or(&root))
        })
        .unwrap_or(false)
}

pub fn remove(worktree: &Path, delete_branch: bool, force: bool) -> Result<(), String> {
    remove_managed_by(worktree, delete_branch, force, is_managed(worktree))
}

fn remove_managed_by(
    worktree: &Path,
    delete_branch: bool,
    force: bool,
    managed: bool,
) -> Result<(), String> {
    if !managed {
        return Err("worktree fora do diretório gerenciado do TYBA".into());
    }
    if !force && is_dirty(worktree)? {
        return Err("worktree tem mudanças não commitadas (use force para descartar)".into());
    }
    let branch = git_text(
        {
            let mut c = git_in(worktree);
            c.args(["rev-parse", "--abbrev-ref", "HEAD"]);
            c
        },
        "git rev-parse --abbrev-ref",
    )?;
    let main = main_repo_of(worktree)?;
    if !force {
        let unmerged = ahead_count(worktree, &head_sha(&main)?)?;
        if unmerged > 0 {
            return Err(format!(
                "worktree tem {unmerged} commit(s) não mergeado(s) (use force para descartar)"
            ));
        }
    }

    let mut cmd = git_in(&main);
    cmd.arg("worktree").arg("remove");
    if force {
        cmd.arg("--force");
    }
    cmd.arg(worktree);
    run_git(cmd, "git worktree remove")?;

    if delete_branch && branch != "HEAD" {
        let mut cmd = git_in(&main);
        cmd.arg("branch")
            .arg(if force { "-D" } else { "-d" })
            .arg(&branch);
        run_git(cmd, "git branch -d")?;
    }
    Ok(())
}

#[derive(Debug, Default, Serialize)]
pub struct GcReport {
    pub removed: Vec<PathBuf>,
    pub kept: Vec<OrphanWorktree>,
}

#[derive(Debug, Serialize)]
pub struct OrphanWorktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub dirty: bool,
    pub unmerged_commits: u32,
    pub reason: String,
}

pub fn gc_orphans(known: &HashSet<PathBuf>) -> GcReport {
    match managed_root() {
        Ok(root) => gc_orphans_in(&root, known),
        Err(_) => GcReport::default(),
    }
}

const GC_RECENT_GRACE: std::time::Duration = std::time::Duration::from_secs(120);

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

fn recently_touched(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|age| age < GC_RECENT_GRACE)
        .unwrap_or(true)
}

fn gc_orphans_in(managed: &Path, known: &HashSet<PathBuf>) -> GcReport {
    let mut report = GcReport::default();
    let managed_canon = crate::repo::canonicalize_or(managed);
    let known: HashSet<PathBuf> = known
        .iter()
        .map(|p| crate::repo::canonicalize_or(p))
        .collect();

    let repos = match std::fs::read_dir(managed) {
        Ok(dir) => dir,
        Err(_) => return report,
    };
    for repo_dir in repos.flatten().filter(|e| e.path().is_dir()) {
        let worktrees = match std::fs::read_dir(repo_dir.path()) {
            Ok(dir) => dir,
            Err(_) => continue,
        };
        for wt in worktrees.flatten().filter(|e| e.path().is_dir()) {
            if is_symlink(&wt.path()) || is_symlink(&repo_dir.path()) {
                continue;
            }
            let path = crate::repo::canonicalize_or(&wt.path());
            if !path.starts_with(&managed_canon) || known.contains(&path) {
                continue;
            }
            gc_one(&path, &mut report);
        }
    }
    report
}

fn gc_one(path: &Path, report: &mut GcReport) {
    let keep = |report: &mut GcReport, branch: Option<String>, dirty, unmerged, reason: &str| {
        report.kept.push(OrphanWorktree {
            path: path.to_path_buf(),
            branch,
            dirty,
            unmerged_commits: unmerged,
            reason: reason.into(),
        });
    };

    if recently_touched(path) {
        keep(report, None, false, 0, "recém-criado (aguardando dono)");
        return;
    }
    let main = match main_repo_of(path) {
        Ok(main) => main,
        Err(_) => {
            keep(report, None, false, 0, "repo principal inacessível");
            return;
        }
    };
    let dirty = match is_dirty(path) {
        Ok(d) => d,
        Err(_) => {
            keep(report, None, false, 0, "estado ilegível");
            return;
        }
    };
    let branch = git_text(
        {
            let mut c = git_in(path);
            c.args(["rev-parse", "--abbrev-ref", "HEAD"]);
            c
        },
        "rev-parse",
    )
    .ok();
    if dirty {
        keep(report, branch, true, 0, "mudanças não commitadas");
        return;
    }
    let main_head = match head_sha(&main) {
        Ok(h) => h,
        Err(_) => {
            keep(report, branch, false, 0, "HEAD do repo principal ilegível");
            return;
        }
    };
    let unmerged = match ahead_count(path, &main_head) {
        Ok(n) => n,
        Err(_) => {
            keep(report, branch, false, 0, "histórico ilegível");
            return;
        }
    };
    if unmerged > 0 {
        keep(report, branch, false, unmerged, "commits não mergeados");
        return;
    }
    match remove_managed_by(path, true, false, true) {
        Ok(()) => report.removed.push(path.to_path_buf()),
        Err(e) => keep(report, branch, false, 0, &format!("remoção falhou: {e}")),
    }
}

pub const SETUP_SCRIPT_REL: &str = ".tyba/setup.sh";

#[derive(Debug, Clone, Serialize)]
pub struct SetupScript {
    pub path: PathBuf,
    pub content: String,
    pub hash: String,
}

pub fn setup_script(root: &Path) -> Option<SetupScript> {
    let path = root.join(SETUP_SCRIPT_REL);
    let content = std::fs::read_to_string(&path).ok()?;
    let hash = format!("{:x}", sha2::Sha256::digest(content.as_bytes()));
    Some(SetupScript {
        path,
        content,
        hash,
    })
}

const SETUP_ENV_ALLOWLIST: [&str; 6] = ["PATH", "HOME", "USER", "LANG", "TMPDIR", "SHELL"];

/// Env mínimo do setup: o script é código do repo, não do usuário —
/// nunca recebe o env completo do shell (princípio #6). A allowlist
/// configurável por repo (`.tyba/config`) chega na Fase 4.
fn filter_setup_env(
    vars: impl Iterator<Item = (String, String)>,
    worktree: &Path,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = vars
        .filter(|(k, _)| SETUP_ENV_ALLOWLIST.contains(&k.as_str()))
        .collect();
    env.push((
        "TYBA_WORKTREE".into(),
        worktree.to_string_lossy().into_owned(),
    ));
    env
}

pub fn setup_env(worktree: &Path) -> Vec<(String, String)> {
    filter_setup_env(std::env::vars(), worktree)
}

/// Executa o conteúdo CONSENTIDO via stdin do `sh` — o que roda é
/// byte a byte o que foi hasheado no consent; trocar o arquivo em
/// disco entre o check e a execução não muda o que executa.
pub fn run_setup(
    worktree: &Path,
    script: &SetupScript,
    env: &[(String, String)],
) -> Result<String, String> {
    use std::io::Write;

    let mut cmd = Command::new("sh");
    cmd.current_dir(worktree).stdin(Stdio::piped()).env_clear();
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| format!("setup.sh: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("stdin do setup indisponível")?
        .write_all(script.content.as_bytes())
        .map_err(|e| format!("setup.sh stdin: {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("setup.sh: {e}"))?;
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        let trimmed = log.trim();
        let skip = trimmed.chars().count().saturating_sub(400);
        let tail: String = trimmed.chars().skip(skip).collect();
        return Err(format!("setup.sh saiu com {}: {tail}", out.status));
    }
    Ok(log)
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
        git(&["config", "commit.gpgsign", "false"]);
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

// TODO(leva B): diff module: SessionDiff { commits, files, uncommitted } com hunks lazy
// TODO(fase 5): rotear `sh setup.sh` e o processo git pela trait Sandbox (#42)

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn temp_base(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("tyba-wt-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn backdate(path: &Path) {
        let ok = Command::new("touch")
            .args(["-m", "-t", "202601010000"])
            .arg(path)
            .status()
            .unwrap()
            .success();
        assert!(ok, "touch -m falhou");
    }

    fn temp_repo(base: &Path) -> PathBuf {
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@t.com"]);
        git(&repo, &["config", "user.name", "t"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        git(&repo, &["config", "tag.gpgsign", "false"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "init"]);
        repo
    }

    #[test]
    fn slugify_normalizes_titles() {
        assert_eq!(slugify("Fix do Watcher!"), "fix-do-watcher");
        assert_eq!(slugify("  éàç  "), "task");
        assert_eq!(slugify("a--b"), "a-b");
        assert!(slugify(&"x".repeat(100)).len() <= 40);
    }

    #[test]
    fn parses_worktree_list_porcelain() {
        let raw = "worktree /repo\nHEAD abc123\nbranch refs/heads/main\n\nworktree /wt/x\nHEAD def456\ndetached\n";
        let entries = parse_worktree_list(raw);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(entries[1].path, PathBuf::from("/wt/x"));
        assert_eq!(entries[1].branch, None);
    }

    #[test]
    fn create_makes_branch_at_base_sha_under_managed_dir() {
        let base = temp_base("create");
        let repo = temp_repo(&base);
        let managed = base.join("managed");

        let wt = create_in(&managed, &repo, "Minha Task").expect("create");

        assert!(wt.path.starts_with(&managed));
        assert!(wt.branch.starts_with("tyba/minha-task-"));
        assert_eq!(wt.base_ref, head_sha(&repo).unwrap());
        assert_eq!(head_sha(&wt.path).unwrap(), wt.base_ref);
        assert!(!is_dirty(&wt.path).unwrap());
        assert_eq!(
            main_repo_of(&wt.path).unwrap(),
            crate::repo::canonicalize_or(&repo)
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn dirty_worktree_refuses_removal_without_force() {
        let base = temp_base("dirty");
        let repo = temp_repo(&base);
        let wt = create_in(&base.join("managed"), &repo, "t").unwrap();
        std::fs::write(wt.path.join("novo.txt"), "x").unwrap();

        let err = remove_managed_by(&wt.path, false, false, true).unwrap_err();
        assert!(err.contains("não commitadas"), "{err}");
        assert!(wt.path.exists());

        remove_managed_by(&wt.path, true, true, true).expect("force remove");
        assert!(!wt.path.exists());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn unmanaged_path_is_never_removed() {
        let base = temp_base("unmanaged");
        let repo = temp_repo(&base);
        let err = remove_managed_by(&repo, false, false, false).unwrap_err();
        assert!(err.contains("fora do diretório gerenciado"), "{err}");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn gc_removes_clean_merged_orphans_and_keeps_the_rest() {
        let base = temp_base("gc");
        let repo = temp_repo(&base);
        let managed = base.join("managed");

        let disposable = create_in(&managed, &repo, "descartavel").unwrap();
        let dirty = create_in(&managed, &repo, "sujo").unwrap();
        std::fs::write(dirty.path.join("wip.txt"), "x").unwrap();
        let ahead = create_in(&managed, &repo, "com-commit").unwrap();
        std::fs::write(ahead.path.join("b.txt"), "b\n").unwrap();
        git(&ahead.path, &["add", "-A"]);
        git(&ahead.path, &["commit", "-qm", "trabalho"]);
        let known = create_in(&managed, &repo, "conhecido").unwrap();
        for wt in [&disposable.path, &dirty.path, &ahead.path] {
            backdate(wt);
        }
        let disposable_canon = crate::repo::canonicalize_or(&disposable.path);

        let report = gc_orphans_in(&managed, &HashSet::from([known.path.clone()]));

        assert_eq!(
            report.removed,
            vec![disposable_canon],
            "kept: {:?}",
            report.kept
        );
        assert!(!disposable.path.exists());
        assert!(dirty.path.exists());
        assert!(ahead.path.exists());
        assert!(known.path.exists());

        let reasons: Vec<&str> = report.kept.iter().map(|k| k.reason.as_str()).collect();
        assert!(reasons.contains(&"mudanças não commitadas"));
        assert!(reasons.contains(&"commits não mergeados"));
        assert_eq!(report.kept.len(), 2, "{reasons:?}");

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn setup_script_hash_tracks_content() {
        let base = temp_base("setup");
        let repo = temp_repo(&base);
        assert!(setup_script(&repo).is_none());

        std::fs::create_dir_all(repo.join(".tyba")).unwrap();
        std::fs::write(repo.join(SETUP_SCRIPT_REL), "echo oi\n").unwrap();
        let first = setup_script(&repo).unwrap();
        std::fs::write(repo.join(SETUP_SCRIPT_REL), "echo tchau\n").unwrap();
        let second = setup_script(&repo).unwrap();

        assert_ne!(first.hash, second.hash);
        assert_eq!(first.hash.len(), 64);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn setup_env_filter_drops_everything_outside_the_allowlist() {
        let vars = vec![
            ("HOME".to_string(), "/home/t".to_string()),
            ("AWS_SECRET_ACCESS_KEY".to_string(), "vazou".to_string()),
            ("PATH".to_string(), "/bin".to_string()),
        ];
        let env = filter_setup_env(vars.into_iter(), Path::new("/wt"));
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"HOME"));
        assert!(keys.contains(&"PATH"));
        assert!(keys.contains(&"TYBA_WORKTREE"));
        assert!(!keys.contains(&"AWS_SECRET_ACCESS_KEY"));
    }

    #[test]
    #[cfg(unix)]
    fn run_setup_executes_the_consented_bytes_not_the_file_on_disk() {
        let base = temp_base("setupenv");
        let repo = temp_repo(&base);
        std::fs::create_dir_all(repo.join(".tyba")).unwrap();
        std::fs::write(
            repo.join(SETUP_SCRIPT_REL),
            "printf '%s' \"$TYBA_WORKTREE\"\n",
        )
        .unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "setup"]);
        let wt = create_in(&base.join("managed"), &repo, "env").unwrap();

        let script = setup_script(&wt.path).expect("script");
        std::fs::write(
            wt.path.join(SETUP_SCRIPT_REL),
            "echo TROCADO-DEPOIS-DO-CONSENT\n",
        )
        .unwrap();
        let env = vec![(
            "TYBA_WORKTREE".to_string(),
            wt.path.to_string_lossy().into_owned(),
        )];

        let log = run_setup(&wt.path, &script, &env).expect("setup");

        assert!(
            !log.contains("TROCADO"),
            "executou o arquivo trocado em vez do conteúdo consentido: {log}"
        );
        assert_eq!(log.trim(), wt.path.to_string_lossy());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn gc_skips_symlinked_entries_and_fresh_worktrees() {
        let base = temp_base("gcsym");
        let repo = temp_repo(&base);
        let managed = base.join("managed");

        let fresh = create_in(&managed, &repo, "fresquinho").unwrap();
        let outside = create_in(&base.join("fora"), &repo, "de-fora").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside.path, managed.join("repo").join("link")).unwrap();

        let report = gc_orphans_in(&managed, &HashSet::new());

        assert!(report.removed.is_empty(), "{:?}", report.removed);
        assert!(
            outside.path.exists(),
            "symlink não podia alcançar fora do managed"
        );
        assert!(fresh.path.exists());
        assert!(report
            .kept
            .iter()
            .any(|k| k.reason.contains("recém-criado")));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn clean_worktree_with_unmerged_commits_refuses_removal_without_force() {
        let base = temp_base("unmerged");
        let repo = temp_repo(&base);
        let wt = create_in(&base.join("managed"), &repo, "t").unwrap();
        std::fs::write(wt.path.join("b.txt"), "b\n").unwrap();
        git(&wt.path, &["add", "-A"]);
        git(&wt.path, &["commit", "-qm", "trabalho"]);

        let err = remove_managed_by(&wt.path, true, false, true).unwrap_err();
        assert!(err.contains("não mergeado"), "{err}");
        assert!(wt.path.exists());

        remove_managed_by(&wt.path, true, true, true).expect("force");
        assert!(!wt.path.exists());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn failing_setup_surfaces_the_tail_of_the_log() {
        let base = temp_base("setupfail");
        let repo = temp_repo(&base);
        std::fs::create_dir_all(repo.join(".tyba")).unwrap();
        std::fs::write(repo.join(SETUP_SCRIPT_REL), "echo quebrou >&2; exit 3\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "setup"]);
        let wt = create_in(&base.join("managed"), &repo, "fail").unwrap();
        let script = setup_script(&wt.path).expect("script");

        let err = run_setup(&wt.path, &script, &[]).unwrap_err();
        assert!(err.contains("quebrou"), "{err}");
        std::fs::remove_dir_all(&base).ok();
    }
}
