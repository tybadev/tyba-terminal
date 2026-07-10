use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Runtime};

use crate::worktree::git_in;

pub const EVENT_CHANGED: &str = "repo://changed";
pub const EVENT_RECONCILED: &str = "repo://reconciled";

const DEBOUNCE: Duration = Duration::from_millis(300);
const MIN_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(2);
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
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
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
    let components: Vec<_> = path.components().map(|c| c.as_os_str()).collect();
    if components
        .windows(2)
        .any(|pair| pair[0] == "refs" && pair[1] == "heads")
    {
        return true;
    }
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| WATCHED_NAMES.contains(&n))
}

#[cfg(target_os = "linux")]
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(target_os = "macos")]
fn proc_pidinfo_struct<T>(pid: u32, flavor: libc::c_int) -> Option<T> {
    let mut info: T = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<T>() as libc::c_int;
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            flavor,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        )
    };
    (written == size).then_some(info)
}

#[cfg(target_os = "macos")]
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let info: libc::proc_vnodepathinfo = proc_pidinfo_struct(pid, libc::PROC_PIDVNODEPATHINFO)?;
    let raw = &info.pvi_cdir.vip_path;
    let bytes = unsafe {
        std::slice::from_raw_parts(raw.as_ptr() as *const u8, std::mem::size_of_val(raw))
    };
    let end = bytes.iter().position(|b| *b == 0)?;
    if end == 0 {
        return None;
    }
    Some(PathBuf::from(OsStr::from_bytes(&bytes[..end])))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn process_cwd(_pid: u32) -> Option<PathBuf> {
    None
}

/// Instante de nascimento do processo, em unidade opaca da plataforma.
/// Igualdade entre dois valores do mesmo pid é a única operação suportada;
/// pid morto, zumbi ou reusado nunca devolve o valor original.
#[cfg(target_os = "linux")]
pub fn process_start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_proc_stat_start_time(&stat)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_proc_stat_start_time(stat: &str) -> Option<u64> {
    let after_comm = stat.rsplit_once(')')?.1;
    let mut fields = after_comm.split_whitespace();
    if fields.next()? == "Z" {
        return None;
    }
    fields.nth(18)?.parse().ok()
}

#[cfg(target_os = "macos")]
pub fn process_start_time(pid: u32) -> Option<u64> {
    let info: libc::proc_bsdinfo = proc_pidinfo_struct(pid, libc::PROC_PIDTBSDINFO)?;
    Some(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn process_start_time(_pid: u32) -> Option<u64> {
    None
}

pub fn canonicalize_or(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub fn toplevel(cwd: &Path) -> Option<PathBuf> {
    git_path(cwd, "--show-toplevel")
}

pub(crate) fn git_path(root: &Path, arg: &str) -> Option<PathBuf> {
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

#[derive(PartialEq, Eq)]
struct UntrackedKey {
    mtime: SystemTime,
    len: u64,
    ctime: Option<(i64, i64)>,
}

impl UntrackedKey {
    fn of(meta: &std::fs::Metadata) -> Option<Self> {
        Some(Self {
            mtime: meta.modified().ok()?,
            len: meta.len(),
            ctime: inode_ctime(meta),
        })
    }
}

#[cfg(unix)]
fn inode_ctime(meta: &std::fs::Metadata) -> Option<(i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.ctime(), meta.ctime_nsec()))
}

#[cfg(not(unix))]
fn inode_ctime(_meta: &std::fs::Metadata) -> Option<(i64, i64)> {
    None
}

struct UntrackedEntry {
    key: UntrackedKey,
    lines: u32,
}

#[derive(Default)]
pub struct UntrackedCache(HashMap<PathBuf, UntrackedEntry>);

fn count_text_lines(path: &Path) -> Option<u32> {
    let content = std::fs::read(path).ok()?;
    if content.contains(&0) {
        return Some(0);
    }
    let mut count = content.iter().filter(|b| **b == b'\n').count();
    if !content.is_empty() && content.last() != Some(&b'\n') {
        count += 1;
    }
    Some(count as u32)
}

fn untracked_insertions(root: &Path, cache: &mut UntrackedCache) -> u32 {
    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    let Ok(list) = git_in(root)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .output()
    else {
        return 0;
    };
    let mut fresh = HashMap::new();
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
        let Some(key) = UntrackedKey::of(&meta) else {
            continue;
        };
        let count = match cache.0.get(&full) {
            Some(entry) if entry.key == key => Some(entry.lines),
            _ => count_text_lines(&full),
        };
        let Some(count) = count else {
            continue;
        };
        lines += count;
        fresh.insert(full, UntrackedEntry { key, lines: count });
    }
    cache.0 = fresh;
    lines
}

pub fn status(root: &Path, cache: &mut UntrackedCache) -> Option<RepoStatus> {
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
        insertions += untracked_insertions(root, cache);
    } else {
        cache.0.clear();
    }
    Some(RepoStatus {
        dirty: changed > 0,
        changed,
        insertions,
        deletions,
    })
}

fn ahead_behind(root: &Path) -> Option<(u32, u32)> {
    let out = git_in(root)
        .args(["rev-list", "--left-right", "--count", "@{upstream}...HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text.split_whitespace();
    let behind = parts.next()?.parse().ok()?;
    let ahead = parts.next()?.parse().ok()?;
    Some((ahead, behind))
}

pub fn snapshot(root: &Path, cache: &mut UntrackedCache) -> RepoSnapshot {
    let (ahead, behind) = match ahead_behind(root) {
        Some((ahead, behind)) => (Some(ahead), Some(behind)),
        None => (None, None),
    };
    RepoSnapshot {
        root: root.to_string_lossy().into_owned(),
        branch: branch(root),
        status: status(root, cache),
        ahead,
        behind,
    }
}

struct Watched {
    _debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
    last: Arc<Mutex<Option<RepoSnapshot>>>,
    alive: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Watched {
    fn drop(&mut self) {
        self.alive.store(false, std::sync::atomic::Ordering::SeqCst);
    }
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

    pub fn snapshots(&self) -> Vec<RepoSnapshot> {
        self.watched
            .lock()
            .values()
            .filter_map(|w| w.last.lock().clone())
            .collect()
    }

    pub fn set_roots<R: Runtime>(&self, app: &AppHandle<R>, roots: HashSet<PathBuf>) {
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
            watched.insert(root, entry);
        }
    }
}

fn spawn_watcher<R: Runtime>(
    app: AppHandle<R>,
    root: PathBuf,
    dirs: &[PathBuf],
) -> Option<Watched> {
    let last: Arc<Mutex<Option<RepoSnapshot>>> = Arc::new(Mutex::new(None));
    let seed = Arc::clone(&last);
    let callback_root = root.clone();
    let emit_app = app.clone();
    let cache = Arc::new(Mutex::new(UntrackedCache::default()));
    let callback_cache = Arc::clone(&cache);
    let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let callback_alive = Arc::clone(&alive);

    let mut last_run: Option<Instant> = None;
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
        if let Some(wait) = last_run.and_then(|at| MIN_SNAPSHOT_INTERVAL.checked_sub(at.elapsed()))
        {
            std::thread::sleep(wait);
        }
        if !callback_alive.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let next = snapshot(&callback_root, &mut callback_cache.lock());
        last_run = Some(Instant::now());
        let mut guard = last.lock();
        if guard.as_ref() == Some(&next) {
            return;
        }
        *guard = Some(next.clone());
        drop(guard);
        if callback_alive.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = app.emit(EVENT_CHANGED, next);
        }
    })
    .ok()?;

    for dir in dirs {
        let mode = if dir.ends_with("refs/heads") {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        if debouncer.watch(dir, mode).is_err() {
            return None;
        }
    }

    let initial = snapshot(&root, &mut cache.lock());
    let _ = emit_app.emit(EVENT_CHANGED, initial.clone());
    *seed.lock() = Some(initial);

    Some(Watched {
        _debouncer: debouncer,
        last: seed,
        alive,
    })
}

#[cfg(all(test, unix))]
mod canonicalize_tests {
    use super::canonicalize_or;
    use std::path::Path;

    #[test]
    fn resolves_a_symlinked_path_to_the_physical_one() {
        let real = std::env::temp_dir().join(format!("tyba-real-{}", uuid::Uuid::new_v4()));
        let link = std::env::temp_dir().join(format!("tyba-link-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert_eq!(canonicalize_or(&link), canonicalize_or(&real));
        assert_ne!(canonicalize_or(&link), link);

        std::fs::remove_file(&link).ok();
        std::fs::remove_dir_all(&real).ok();
    }

    #[test]
    fn a_path_that_does_not_exist_falls_back_to_itself() {
        let ghost = Path::new("/definitivamente/nao/existe/aqui");
        assert_eq!(canonicalize_or(ghost), ghost.to_path_buf());
    }

    #[test]
    fn the_kernel_cwd_and_the_logical_path_converge_after_canonicalize() {
        let real = std::env::temp_dir().join(format!("tyba-conv-{}", uuid::Uuid::new_v4()));
        let link = std::env::temp_dir().join(format!("tyba-conv-link-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let logical = link.join(".");
        let physical = canonicalize_or(&real);

        assert_eq!(canonicalize_or(&logical), physical);

        std::fs::remove_file(&link).ok();
        std::fs::remove_dir_all(&real).ok();
    }
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
        git(&repo, &["config", "commit.gpgsign", "false"]);
        git(&repo, &["config", "tag.gpgsign", "false"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "init"]);
        repo
    }

    #[test]
    fn process_cwd_reads_the_real_working_directory_of_a_live_process() {
        use std::process::{Command, Stdio};

        let repo = temp_repo();
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("read _ || sleep 5")
            .current_dir(&repo)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .expect("spawn");

        std::thread::sleep(std::time::Duration::from_millis(150));
        let seen = super::process_cwd(child.id());

        let _ = child.kill();
        let _ = child.wait();

        let seen = seen.expect("process_cwd devolveu None");
        let want = std::fs::canonicalize(&repo).unwrap();
        let got = std::fs::canonicalize(&seen).unwrap();
        std::fs::remove_dir_all(repo.parent().unwrap()).ok();
        assert_eq!(got, want, "cwd real divergiu");
    }

    #[test]
    fn toplevel_of_a_subdirectory_is_the_repo_root() {
        let repo = temp_repo();
        let sub = repo.join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        let top = super::toplevel(&sub).expect("toplevel");
        let want = std::fs::canonicalize(&repo).unwrap();
        let got = std::fs::canonicalize(&top).unwrap();
        std::fs::remove_dir_all(repo.parent().unwrap()).ok();
        assert_eq!(got, want);
    }

    #[test]
    fn toplevel_outside_a_repo_is_none() {
        let dir = std::env::temp_dir().join(format!("tyba-norepo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let top = super::toplevel(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(top.is_none());
    }

    #[test]
    fn watcher_emits_when_the_repo_changes_under_it() {
        use std::collections::HashSet;
        use std::sync::mpsc;
        use std::time::Duration;
        use tauri::Listener;

        let repo = temp_repo();
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();

        let (tx, rx) = mpsc::channel();
        handle.listen_any(super::EVENT_CHANGED, move |_| {
            let _ = tx.send(());
        });

        let watcher = super::RepoWatcher::new();
        watcher.set_roots(&handle, HashSet::from([repo.clone()]));

        rx.recv_timeout(Duration::from_secs(5))
            .expect("snapshot inicial nao foi emitido no registro");

        git(&repo, &["switch", "-c", "probe", "-q"]);

        let fired = rx.recv_timeout(Duration::from_secs(10)).is_ok();
        std::fs::remove_dir_all(repo.parent().unwrap()).ok();
        assert!(fired, "watcher nao emitiu apos troca de branch externa");
    }

    #[test]
    fn watcher_stays_quiet_when_nothing_changes() {
        use std::collections::HashSet;
        use std::sync::mpsc;
        use std::time::Duration;
        use tauri::Listener;

        let repo = temp_repo();
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();

        let (tx, rx) = mpsc::channel();
        handle.listen_any(super::EVENT_CHANGED, move |_| {
            let _ = tx.send(());
        });

        let watcher = super::RepoWatcher::new();
        watcher.set_roots(&handle, HashSet::from([repo.clone()]));
        rx.recv_timeout(Duration::from_secs(5))
            .expect("snapshot inicial");

        git(&repo, &["status", "--porcelain"]);
        std::thread::sleep(Duration::from_millis(900));

        let spurious = rx.try_recv().is_ok();
        std::fs::remove_dir_all(repo.parent().unwrap()).ok();
        assert!(!spurious, "watcher emitiu sem o snapshot ter mudado");
    }

    #[test]
    fn process_start_time_distinguishes_a_live_process_from_a_reaped_pid() {
        use std::process::{Command, Stdio};

        let mut child = Command::new("sleep")
            .arg("30")
            .stdout(Stdio::null())
            .spawn()
            .expect("spawn");
        let pid = child.id();

        let live = super::process_start_time(pid);
        assert!(live.is_some(), "processo vivo devia ter start time");
        assert_eq!(
            live,
            super::process_start_time(pid),
            "start time devia ser estável enquanto o processo vive"
        );

        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(
            super::process_start_time(pid),
            None,
            "pid colhido não devia ter start time"
        );
    }

    #[test]
    fn untracked_cache_is_consulted_when_the_key_matches() {
        let repo = temp_repo();
        let file = repo.join("novo.txt");
        std::fs::write(&file, "a\nb").unwrap();

        let mut cache = super::UntrackedCache::default();
        let real = super::status(&repo, &mut cache).expect("status");
        assert_eq!(real.insertions, 2);

        let key = super::UntrackedKey::of(&std::fs::metadata(&file).unwrap()).unwrap();
        cache
            .0
            .insert(file.clone(), super::UntrackedEntry { key, lines: 40 });
        let forged = super::status(&repo, &mut cache).expect("status");
        assert_eq!(
            forged.insertions, 40,
            "chave igual devia responder do cache sem reler o arquivo"
        );

        std::fs::remove_dir_all(repo.parent().unwrap()).ok();
    }

    #[test]
    fn content_change_with_restored_mtime_is_still_recounted() {
        let repo = temp_repo();
        let file = repo.join("novo.txt");
        std::fs::write(&file, "a\nb").unwrap();

        let mut cache = super::UntrackedCache::default();
        let first = super::status(&repo, &mut cache).expect("status");
        assert_eq!(first.insertions, 2);

        let mtime = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::fs::write(&file, "abc").unwrap();
        let handle = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
        handle
            .set_times(std::fs::FileTimes::new().set_modified(mtime))
            .unwrap();

        let second = super::status(&repo, &mut cache).expect("status");
        assert_eq!(
            second.insertions, 1,
            "conteúdo novo com mtime restaurado devia furar o cache (ctime muda)"
        );

        std::fs::remove_dir_all(repo.parent().unwrap()).ok();
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_file_is_recounted_once_it_becomes_readable() {
        use std::os::unix::fs::PermissionsExt;

        let repo = temp_repo();
        let file = repo.join("segredo.txt");
        std::fs::write(&file, "a\nb\nc").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();

        let mut cache = super::UntrackedCache::default();
        let blind = super::status(&repo, &mut cache).expect("status");
        assert_eq!(blind.insertions, 0, "arquivo ilegível não conta linhas");

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        let healed = super::status(&repo, &mut cache).expect("status");
        assert_eq!(
            healed.insertions, 3,
            "falha de leitura não pode ficar congelada no cache"
        );

        std::fs::remove_dir_all(repo.parent().unwrap()).ok();
    }

    #[test]
    fn proc_stat_parser_survives_parentheses_and_spaces_in_comm() {
        let stat = "1234 (meu) proc (x)) S 1 1234 1234 0 -1 4194560 100 0 0 0 5 3 0 0 20 0 1 0 987654 1000000 200 18446744073709551615";
        assert_eq!(super::parse_proc_stat_start_time(stat), Some(987654));
    }

    #[test]
    fn proc_stat_parser_refuses_zombies_and_garbage() {
        let zombie =
            "1234 (proc) Z 1 1234 1234 0 -1 4194560 100 0 0 0 5 3 0 0 20 0 1 0 987654 0 0 1";
        assert_eq!(super::parse_proc_stat_start_time(zombie), None);
        assert_eq!(super::parse_proc_stat_start_time("1234 (proc) S 1 2"), None);
        assert_eq!(super::parse_proc_stat_start_time(""), None);
    }

    #[test]
    fn ahead_behind_tracks_the_upstream() {
        let repo = temp_repo();
        let no_upstream = super::snapshot(&repo, &mut super::UntrackedCache::default());
        assert_eq!((no_upstream.ahead, no_upstream.behind), (None, None));

        let base = repo.parent().unwrap();
        let clone = base.join("clone");
        git(
            base,
            &[
                "clone",
                "-q",
                repo.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        git(&clone, &["config", "user.email", "t@t.com"]);
        git(&clone, &["config", "user.name", "t"]);

        let synced = super::snapshot(&clone, &mut super::UntrackedCache::default());
        assert_eq!((synced.ahead, synced.behind), (Some(0), Some(0)));

        std::fs::write(clone.join("c.txt"), "c\n").unwrap();
        git(&clone, &["add", "-A"]);
        git(&clone, &["commit", "-qm", "ahead"]);
        let ahead = super::snapshot(&clone, &mut super::UntrackedCache::default());
        assert_eq!((ahead.ahead, ahead.behind), (Some(1), Some(0)));

        std::fs::write(repo.join("d.txt"), "d\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "upstream-moves"]);
        git(&clone, &["fetch", "-q"]);
        let diverged = super::snapshot(&clone, &mut super::UntrackedCache::default());
        assert_eq!((diverged.ahead, diverged.behind), (Some(1), Some(1)));

        std::fs::remove_dir_all(base).ok();
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
    fn repo_whose_path_contains_logs_or_refs_still_matches() {
        assert!(is_watched_event_path(Path::new(
            "/home/u/dev/logs/svc/.git/index"
        )));
        assert!(is_watched_event_path(Path::new(
            "/home/u/refs/proj/.git/HEAD"
        )));
    }

    #[test]
    fn slash_named_branch_ref_matches() {
        assert!(is_watched_event_path(Path::new(
            "/r/.git/refs/heads/feat/git-watcher"
        )));
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
