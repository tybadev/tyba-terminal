use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};

use crate::worktree::git_in;

pub const EVENT_CHANGED: &str = "repo://changed";

const DEBOUNCE: Duration = Duration::from_millis(300);
const UNTRACKED_MAX_BYTES: u64 = 512 * 1024;
const UNTRACKED_MAX_FILES: usize = 500;

const WATCHED_NAMES: [&str; 5] = ["HEAD", "index", "ORIG_HEAD", "MERGE_HEAD", "packed-refs"];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RepoStatus {
    pub dirty: bool,
    pub changed: u32,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RepoSnapshot {
    pub root: String,
    pub branch: Option<String>,
    pub status: Option<RepoStatus>,
}

pub fn count_status_entries(stdout: &[u8]) -> u32 {
    let mut fields = stdout.split(|b| *b == 0).filter(|entry| !entry.is_empty());
    let mut changed = 0u32;
    while let Some(entry) = fields.next() {
        changed += 1;
        if matches!(entry.first(), Some(b'R') | Some(b'C')) {
            fields.next();
        }
    }
    changed
}

pub fn is_watched_event_path(path: &Path) -> bool {
    if path.extension().is_some_and(|ext| ext == "lock") {
        return false;
    }
    if path
        .components()
        .any(|c| c.as_os_str() == "refs" || c.as_os_str() == "logs")
    {
        return path.components().any(|c| c.as_os_str() == "heads");
    }
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| WATCHED_NAMES.contains(&n))
}

fn git_path(root: &Path, arg: &str) -> Option<PathBuf> {
    let out = git_in(root)
        .args(["rev-parse", "--path-format=absolute", arg])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if raw.is_empty() {
        None
    } else {
        Some(PathBuf::from(raw))
    }
}

pub fn watch_dirs(root: &Path) -> Vec<PathBuf> {
    let git_dir = git_path(root, "--git-dir");
    let common_dir = git_path(root, "--git-common-dir");

    let mut dirs = Vec::new();
    if let Some(dir) = git_dir {
        dirs.push(dir);
    }
    if let Some(common) = common_dir {
        dirs.push(common.join("refs").join("heads"));
        dirs.push(common);
    }
    dirs.retain(|d| d.is_dir());
    dirs.sort();
    dirs.dedup();
    dirs
}

pub fn branch(root: &Path) -> Option<String> {
    let out = git_in(root)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if branch.is_empty() {
        None
    } else {
        Some(branch)
    }
}

fn diff_numstat(root: &Path) -> (u32, u32) {
    let Ok(out) = git_in(root)
        .args(["diff", "--no-ext-diff", "--numstat", "--no-color", "HEAD"])
        .output()
    else {
        return (0, 0);
    };
    if !out.status.success() {
        return (0, 0);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut insertions = 0u32;
    let mut deletions = 0u32;
    for line in text.lines() {
        let mut parts = line.split('\t');
        let added = parts.next().and_then(|v| v.parse::<u32>().ok());
        let removed = parts.next().and_then(|v| v.parse::<u32>().ok());
        insertions += added.unwrap_or(0);
        deletions += removed.unwrap_or(0);
    }
    (insertions, deletions)
}

fn untracked_insertions(root: &Path) -> u32 {
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    let Ok(list) = git_in(root)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .output()
    else {
        return 0;
    };
    let mut lines = 0u32;
    for file in list
        .stdout
        .split(|b| *b == 0)
        .filter(|f| !f.is_empty())
        .take(UNTRACKED_MAX_FILES)
    {
        #[cfg(unix)]
        let full = root.join(std::ffi::OsStr::from_bytes(file));
        #[cfg(not(unix))]
        let full = root.join(String::from_utf8_lossy(file).as_ref());

        let Ok(meta) = std::fs::metadata(&full) else {
            continue;
        };
        if !meta.is_file() || meta.len() > UNTRACKED_MAX_BYTES {
            continue;
        }
        if let Ok(content) = std::fs::read(&full) {
            if content.contains(&0) {
                continue;
            }
            let mut count = content.iter().filter(|b| **b == b'\n').count();
            if !content.is_empty() && content.last() != Some(&b'\n') {
                count += 1;
            }
            lines += count as u32;
        }
    }
    lines
}

pub fn status(root: &Path) -> Option<RepoStatus> {
    let out = git_in(root)
        .args(["status", "--porcelain", "-z"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let changed = count_status_entries(&out.stdout);
    let (mut insertions, deletions) = if changed > 0 {
        diff_numstat(root)
    } else {
        (0, 0)
    };
    if changed > 0 {
        insertions += untracked_insertions(root);
    }
    Some(RepoStatus {
        dirty: changed > 0,
        changed,
        insertions,
        deletions,
    })
}

pub fn snapshot(root: &Path) -> RepoSnapshot {
    RepoSnapshot {
        root: root.to_string_lossy().into_owned(),
        branch: branch(root),
        status: status(root),
    }
}

struct Watched {
    _debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
}

#[derive(Default)]
pub struct RepoWatcher {
    watched: Mutex<HashMap<PathBuf, Watched>>,
}

pub type SharedRepoWatcher = Arc<RepoWatcher>;

impl RepoWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_roots(&self, app: &AppHandle, roots: HashSet<PathBuf>) {
        let mut watched = self.watched.lock();
        watched.retain(|root, _| roots.contains(root));

        for root in roots {
            if watched.contains_key(&root) {
                continue;
            }
            let dirs = watch_dirs(&root);
            if dirs.is_empty() {
                continue;
            }
            let Some(entry) = spawn_watcher(app.clone(), root.clone(), &dirs) else {
                continue;
            };
            let _ = app.emit(EVENT_CHANGED, snapshot(&root));
            watched.insert(root, entry);
        }
    }
}

fn spawn_watcher(app: AppHandle, root: PathBuf, dirs: &[PathBuf]) -> Option<Watched> {
    let last: Arc<Mutex<Option<RepoSnapshot>>> = Arc::new(Mutex::new(None));
    let callback_root = root.clone();

    let mut debouncer = new_debouncer(DEBOUNCE, None, move |result: DebounceEventResult| {
        let Ok(events) = result else {
            return;
        };
        if !events
            .iter()
            .any(|e| e.paths.iter().any(|p| is_watched_event_path(p)))
        {
            return;
        }
        let next = snapshot(&callback_root);
        let mut guard = last.lock();
        if guard.as_ref() == Some(&next) {
            return;
        }
        *guard = Some(next.clone());
        drop(guard);
        let _ = app.emit(EVENT_CHANGED, next);
    })
    .ok()?;

    for dir in dirs {
        if debouncer.watch(dir, RecursiveMode::NonRecursive).is_err() {
            return None;
        }
    }
    Some(Watched {
        _debouncer: debouncer,
    })
}

#[cfg(test)]
mod tests {
    use super::{count_status_entries, is_watched_event_path, watch_dirs};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git")
            .status
            .success();
        assert!(ok, "git {args:?} falhou");
    }

    fn temp_repo() -> PathBuf {
        let base = std::env::temp_dir().join(format!("tyba-repo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@t.com"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "init"]);
        repo
    }

    #[test]
    fn counts_rename_once() {
        assert_eq!(count_status_entries(b"R  new.txt\0old.txt\0"), 1);
    }

    #[test]
    fn counts_plain_entries() {
        assert_eq!(count_status_entries(b" M keep.txt\0?? new.txt\0"), 2);
    }

    #[test]
    fn watches_head_index_and_refs() {
        assert!(is_watched_event_path(Path::new("/r/.git/HEAD")));
        assert!(is_watched_event_path(Path::new("/r/.git/index")));
        assert!(is_watched_event_path(Path::new("/r/.git/packed-refs")));
        assert!(is_watched_event_path(Path::new("/r/.git/refs/heads/main")));
    }

    #[test]
    fn ignores_lock_files_and_objects() {
        assert!(!is_watched_event_path(Path::new("/r/.git/index.lock")));
        assert!(!is_watched_event_path(Path::new("/r/.git/HEAD.lock")));
        assert!(!is_watched_event_path(Path::new("/r/.git/objects/ab/cdef")));
        assert!(!is_watched_event_path(Path::new("/r/.git/refs/tags/v1")));
    }

    #[test]
    fn watch_dirs_of_a_plain_repo_point_at_its_own_git_dir() {
        let repo = temp_repo();
        let dirs = watch_dirs(&repo);
        assert!(!dirs.is_empty(), "nenhum dir resolvido");
        assert!(dirs.iter().any(|d| d.ends_with(".git")));
        assert!(dirs.iter().any(|d| d.ends_with("refs/heads")));
        std::fs::remove_dir_all(repo.parent().unwrap()).ok();
    }

    #[test]
    fn notify_actually_fires_when_git_touches_the_index() {
        use notify::RecursiveMode;
        use notify_debouncer_full::{new_debouncer, DebounceEventResult};
        use std::sync::mpsc;
        use std::time::Duration;

        let repo = temp_repo();
        let dirs = watch_dirs(&repo);
        assert!(!dirs.is_empty());

        let (tx, rx) = mpsc::channel();
        let mut debouncer = new_debouncer(
            Duration::from_millis(200),
            None,
            move |result: DebounceEventResult| {
                if let Ok(events) = result {
                    let hit = events
                        .iter()
                        .any(|e| e.paths.iter().any(|p| is_watched_event_path(p)));
                    if hit {
                        let _ = tx.send(());
                    }
                }
            },
        )
        .expect("debouncer");
        for dir in &dirs {
            debouncer
                .watch(dir, RecursiveMode::NonRecursive)
                .expect("watch");
        }

        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(repo.join("b.txt"), "b\n").unwrap();
        git(&repo, &["add", "b.txt"]);

        let fired = rx.recv_timeout(Duration::from_secs(10)).is_ok();
        drop(debouncer);
        std::fs::remove_dir_all(repo.parent().unwrap()).ok();
        assert!(fired, "watcher nao disparou ao git tocar o index");
    }

    #[test]
    fn watch_dirs_of_a_worktree_cover_both_git_dir_and_common_dir() {
        let repo = temp_repo();
        let wt = repo.parent().unwrap().join("wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                wt.to_str().unwrap(),
                "-b",
                "feat/x",
            ],
        );

        let dirs = watch_dirs(&wt);
        assert!(
            dirs.iter().any(|d| d.ends_with("worktrees/wt")),
            "git-dir do worktree nao observado: {dirs:?}"
        );
        assert!(
            dirs.iter().any(|d| d.ends_with("refs/heads")),
            "refs/heads do common-dir nao observado: {dirs:?}"
        );
        std::fs::remove_dir_all(repo.parent().unwrap()).ok();
    }
}
