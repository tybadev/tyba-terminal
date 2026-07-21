use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::session::SessionId;
use crate::worktree::diff::{FileStatus, SessionDiff};

pub const EVENT_TREE_PREFIX: &str = "files://tree/";
pub const EVENT_DECORATIONS_PREFIX: &str = "files://decorations/";

pub const READ_PAGE_BYTES: usize = 256 * 1024;
const IMAGE_MAX_BYTES: u64 = 8 * 1024 * 1024;
const WATCH_DEBOUNCE: Duration = Duration::from_millis(200);

pub fn tree_event(id: SessionId) -> String {
    format!("{EVENT_TREE_PREFIX}{id}")
}

pub fn decorations_event(id: SessionId) -> String {
    format!("{EVENT_DECORATIONS_PREFIX}{id}")
}

#[derive(Debug, Clone)]
pub enum Context {
    AgentWorktree { base_ref: String },
    Repo,
    OutsideRepo,
}

impl Context {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Context::AgentWorktree { .. } => "agent_worktree",
            Context::Repo => "repo",
            Context::OutsideRepo => "outside_repo",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub gitignored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirListing {
    pub dir: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileContent {
    Text {
        text: String,
        offset: usize,
        next_offset: usize,
        total: u64,
        truncated: bool,
    },
    Image {
        data: String,
        mime: String,
        total: u64,
    },
    Binary {
        total: u64,
        mime: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecoStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Decoration {
    pub path: String,
    pub status: DecoStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct PanelInfo {
    pub root: String,
    pub kind: String,
    pub decorated: bool,
}

#[derive(Clone, Serialize)]
struct TreePayload {
    dirs: Vec<String>,
}

#[derive(Clone, Serialize)]
struct DecorationsPayload {
    decorations: Vec<Decoration>,
}

fn join_rel(dir_rel: &str, name: &str) -> String {
    if dir_rel.is_empty() {
        name.to_string()
    } else {
        format!("{dir_rel}/{name}")
    }
}

fn rel_of(root: &Path, path: &Path) -> Option<String> {
    let stripped = path.strip_prefix(root).ok()?;
    let s = stripped.to_string_lossy().replace('\\', "/");
    Some(s)
}

pub fn resolve_within(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim_start_matches('/');
    let candidate = if rel.is_empty() {
        root.to_path_buf()
    } else {
        let p = Path::new(rel);
        if p.is_absolute() {
            return Err("path absoluto rejeitado".into());
        }
        for comp in p.components() {
            match comp {
                std::path::Component::Normal(_) | std::path::Component::CurDir => {}
                _ => return Err("path traversal rejeitado".into()),
            }
        }
        root.join(p)
    };
    let canon = std::fs::canonicalize(&candidate).map_err(|e| format!("path inacessível: {e}"))?;
    let root_canon = std::fs::canonicalize(root).map_err(|e| format!("raiz inacessível: {e}"))?;
    if !canon.starts_with(&root_canon) {
        return Err("path fora da raiz do painel".into());
    }
    Ok(canon)
}

fn ignored_set(root: &Path, rels: &[String]) -> HashSet<String> {
    if rels.is_empty() {
        return HashSet::new();
    }
    let mut cmd = crate::worktree::git_in(root);
    cmd.env("GIT_LITERAL_PATHSPECS", "1");
    cmd.args(["check-ignore", "-z", "--"]);
    for rel in rels {
        cmd.arg(rel);
    }
    match crate::worktree::run_git(cmd, "git check-ignore") {
        Ok(raw) => raw
            .split(|b| *b == 0)
            .filter(|f| !f.is_empty())
            .map(|f| String::from_utf8_lossy(f).into_owned())
            .collect(),
        Err(_) => HashSet::new(),
    }
}

pub fn list_dir(root: &Path, dir_rel: &str, context: &Context) -> Result<DirListing, String> {
    let dir = resolve_within(root, dir_rel)?;
    if !dir.is_dir() {
        return Err("não é um diretório".into());
    }
    let mut entries = Vec::new();
    let mut child_rels = Vec::new();
    for ent in std::fs::read_dir(&dir).map_err(|e| format!("falha ao listar: {e}"))? {
        let Ok(ent) = ent else {
            continue;
        };
        let name = ent.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let is_symlink = ent.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        let is_dir = ent.metadata().map(|m| m.is_dir()).unwrap_or(false);
        let rel = join_rel(dir_rel, &name);
        child_rels.push(rel.clone());
        entries.push(Entry {
            name,
            rel_path: rel,
            is_dir,
            is_symlink,
            gitignored: false,
        });
    }
    if !matches!(context, Context::OutsideRepo) {
        let ignored = ignored_set(root, &child_rels);
        for entry in entries.iter_mut() {
            if ignored.contains(&entry.rel_path) {
                entry.gitignored = true;
            }
        }
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(DirListing {
        dir: dir_rel.to_string(),
        entries,
    })
}

fn image_mime(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        _ => return None,
    })
}

fn read_range(path: &Path, offset: usize, len: usize) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).map_err(|e| format!("falha ao abrir: {e}"))?;
    file.seek(SeekFrom::Start(offset as u64))
        .map_err(|e| format!("seek: {e}"))?;
    let mut buf = vec![0u8; len];
    let mut read = 0;
    while read < len {
        match file.read(&mut buf[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(e) => return Err(format!("read: {e}")),
        }
    }
    buf.truncate(read);
    Ok(buf)
}

fn page_end(data: &[u8], more_ahead: bool) -> usize {
    if !more_ahead {
        return data.len();
    }
    match std::str::from_utf8(data) {
        Ok(_) => data.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to().max(1),
        Err(_) => data.len(),
    }
}

pub fn read_file(root: &Path, rel: &str, offset: usize) -> Result<FileContent, String> {
    let full = resolve_within(root, rel)?;
    let meta = std::fs::metadata(&full).map_err(|e| format!("arquivo inacessível: {e}"))?;
    if !meta.is_file() {
        return Err("não é um arquivo".into());
    }
    let total = meta.len();
    let name = full
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();

    if let Some(mime) = image_mime(&name) {
        if total <= IMAGE_MAX_BYTES {
            let bytes = std::fs::read(&full).map_err(|e| format!("falha ao ler: {e}"))?;
            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Ok(FileContent::Image {
                data,
                mime: mime.to_string(),
                total,
            });
        }
        return Ok(FileContent::Binary {
            total,
            mime: Some(mime.to_string()),
        });
    }

    let buf = read_range(&full, offset, READ_PAGE_BYTES)?;
    if offset == 0 && buf.contains(&0) {
        return Ok(FileContent::Binary { total, mime: None });
    }
    let more_ahead = (offset as u64 + buf.len() as u64) < total;
    let end = page_end(&buf, more_ahead);
    let text = String::from_utf8_lossy(&buf[..end]).into_owned();
    let next_offset = offset + end;
    Ok(FileContent::Text {
        text,
        offset,
        next_offset,
        total,
        truncated: (next_offset as u64) < total,
    })
}

fn deco_of(status: FileStatus) -> DecoStatus {
    match status {
        FileStatus::Added => DecoStatus::Added,
        FileStatus::Deleted => DecoStatus::Deleted,
        FileStatus::Renamed | FileStatus::Copied => DecoStatus::Renamed,
        _ => DecoStatus::Modified,
    }
}

fn to_decorations(map: HashMap<String, DecoStatus>) -> Vec<Decoration> {
    let mut out: Vec<Decoration> = map
        .into_iter()
        .map(|(path, status)| Decoration { path, status })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

pub fn decorations_from_diff(diff: &SessionDiff) -> Vec<Decoration> {
    let mut map: HashMap<String, DecoStatus> = HashMap::new();
    for f in &diff.files {
        map.entry(f.path.clone())
            .or_insert_with(|| deco_of(f.status));
    }
    for f in &diff.staged_files {
        map.entry(f.path.clone())
            .or_insert_with(|| deco_of(f.status));
    }
    for f in &diff.unstaged_files {
        map.entry(f.path.clone())
            .or_insert_with(|| deco_of(f.status));
    }
    to_decorations(map)
}

pub fn parse_status_z(raw: &[u8]) -> Vec<Decoration> {
    let mut map: HashMap<String, DecoStatus> = HashMap::new();
    let mut fields = raw.split(|b| *b == 0).filter(|e| !e.is_empty());
    while let Some(entry) = fields.next() {
        if entry.len() < 3 {
            continue;
        }
        let x = entry[0];
        let y = entry[1];
        let path = String::from_utf8_lossy(&entry[3..]).into_owned();
        if matches!(x, b'R' | b'C') {
            fields.next();
        }
        let status = if x == b'?' && y == b'?' {
            DecoStatus::Untracked
        } else if x == b'A' || y == b'A' {
            DecoStatus::Added
        } else if x == b'D' || y == b'D' {
            DecoStatus::Deleted
        } else if x == b'R' || x == b'C' {
            DecoStatus::Renamed
        } else {
            DecoStatus::Modified
        };
        map.insert(path, status);
    }
    to_decorations(map)
}

fn status_decorations(root: &Path) -> Vec<Decoration> {
    let raw = match crate::worktree::run_git(
        {
            let mut c = crate::worktree::git_in(root);
            c.args(["status", "--porcelain", "-z"]);
            c
        },
        "git status",
    ) {
        Ok(raw) => raw,
        Err(_) => return Vec::new(),
    };
    parse_status_z(&raw)
}

pub fn compute_decorations(root: &Path, context: &Context) -> Vec<Decoration> {
    match context {
        Context::AgentWorktree { base_ref } => {
            match crate::worktree::diff::session_diff(root, base_ref) {
                Ok(diff) => decorations_from_diff(&diff),
                Err(_) => Vec::new(),
            }
        }
        Context::Repo => status_decorations(root),
        Context::OutsideRepo => Vec::new(),
    }
}

type FileDebouncer = Debouncer<notify::RecommendedWatcher, RecommendedCache>;

struct Panel {
    root: PathBuf,
    context: Context,
    watched: Arc<Mutex<HashSet<PathBuf>>>,
    debouncer: Option<FileDebouncer>,
}

#[derive(Default)]
pub struct FilesManager {
    panels: Mutex<HashMap<SessionId, Panel>>,
}

pub type SharedFiles = Arc<FilesManager>;

impl FilesManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn info(&self, id: SessionId) -> Option<(PathBuf, Context)> {
        self.panels
            .lock()
            .get(&id)
            .map(|p| (p.root.clone(), p.context.clone()))
    }

    pub fn seed<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        id: SessionId,
        root: PathBuf,
        context: Context,
    ) {
        let watched = Arc::new(Mutex::new(HashSet::new()));
        let debouncer = spawn_watcher(app.clone(), id, root.clone(), context.clone(), &watched);
        self.panels.lock().insert(
            id,
            Panel {
                root,
                context,
                watched,
                debouncer,
            },
        );
    }

    pub fn ensure<R, F>(
        &self,
        app: &AppHandle<R>,
        id: SessionId,
        resolve: F,
    ) -> Result<(PathBuf, Context), String>
    where
        R: Runtime,
        F: FnOnce() -> Result<(PathBuf, Context), String>,
    {
        if let Some(existing) = self.info(id) {
            return Ok(existing);
        }
        let (root, context) = resolve()?;
        self.seed(app, id, root.clone(), context.clone());
        Ok((root, context))
    }

    pub fn watch_dir(&self, id: SessionId, dir: PathBuf) {
        let mut panels = self.panels.lock();
        let Some(panel) = panels.get_mut(&id) else {
            return;
        };
        if panel.watched.lock().insert(dir.clone()) {
            if let Some(debouncer) = panel.debouncer.as_mut() {
                let _ = debouncer.watch(&dir, RecursiveMode::NonRecursive);
            }
        }
    }

    pub fn unwatch_dir(&self, id: SessionId, dir: PathBuf) {
        let mut panels = self.panels.lock();
        let Some(panel) = panels.get_mut(&id) else {
            return;
        };
        if panel.watched.lock().remove(&dir) {
            if let Some(debouncer) = panel.debouncer.as_mut() {
                let _ = debouncer.unwatch(&dir);
            }
        }
    }

    pub fn close(&self, id: SessionId) {
        self.panels.lock().remove(&id);
    }

    pub fn emit_decorations<R: Runtime>(&self, app: &AppHandle<R>, id: SessionId) {
        let Some((root, context)) = self.info(id) else {
            return;
        };
        let decorations = compute_decorations(&root, &context);
        let _ = app.emit(&decorations_event(id), DecorationsPayload { decorations });
    }

    pub fn emit_tree_reset<R: Runtime>(&self, app: &AppHandle<R>, id: SessionId) {
        let _ = app.emit(
            &tree_event(id),
            TreePayload {
                dirs: vec![String::new()],
            },
        );
    }
}

fn spawn_watcher<R: Runtime>(
    app: AppHandle<R>,
    id: SessionId,
    root: PathBuf,
    context: Context,
    watched: &Arc<Mutex<HashSet<PathBuf>>>,
) -> Option<FileDebouncer> {
    let tree_ev = tree_event(id);
    let deco_ev = decorations_event(id);
    let cb_root = root.clone();
    let cb_context = context.clone();
    let cb_watched = Arc::clone(watched);

    let mut debouncer = new_debouncer(WATCH_DEBOUNCE, None, move |result: DebounceEventResult| {
        let Ok(events) = result else {
            return;
        };
        let mut touched_git = false;
        let mut affected: HashSet<PathBuf> = HashSet::new();
        {
            let live = cb_watched.lock();
            for event in &events {
                for path in &event.paths {
                    if crate::repo::is_watched_event_path(path) {
                        touched_git = true;
                    }
                    if live.contains(path.as_path()) {
                        affected.insert(path.clone());
                    }
                    if let Some(parent) = path.parent() {
                        if live.contains(parent) {
                            affected.insert(parent.to_path_buf());
                        }
                    }
                }
            }
        }
        if !affected.is_empty() {
            let dirs: Vec<String> = affected
                .iter()
                .filter_map(|d| rel_of(&cb_root, d))
                .collect();
            let _ = app.emit(&tree_ev, TreePayload { dirs });
        }
        if touched_git || !affected.is_empty() {
            let decorations = compute_decorations(&cb_root, &cb_context);
            let _ = app.emit(&deco_ev, DecorationsPayload { decorations });
        }
    })
    .ok()?;

    for dir in crate::repo::watch_dirs(&root) {
        let mode = if dir.ends_with("refs/heads") {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        let _ = debouncer.watch(&dir, mode);
    }
    Some(debouncer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::diff::{CommitInfo, FileDiff};

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_rejects_parent_traversal() {
        let root = tmp();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let err = resolve_within(&root, "../etc").unwrap_err();
        assert!(err.contains("traversal") || err.contains("fora"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_rejects_absolute_path() {
        let root = tmp();
        assert!(resolve_within(&root, "/etc/passwd").is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_accepts_path_inside_root() {
        let root = tmp();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/f.txt"), "hi").unwrap();
        let resolved = resolve_within(&root, "a/b/f.txt").expect("dentro da raiz é aceito");
        let root_canon = std::fs::canonicalize(&root).unwrap();
        assert!(resolved.starts_with(&root_canon));
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn resolve_rejects_symlink_escaping_root() {
        let root = tmp();
        let outside = tmp();
        std::fs::write(outside.join("secret.txt"), "leak").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        let err = resolve_within(&root, "escape/secret.txt")
            .expect_err("symlink que escapa da raiz é recusado");
        assert!(err.contains("fora"));
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[cfg(unix)]
    #[test]
    fn resolve_accepts_symlink_that_stays_inside_root() {
        let root = tmp();
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::fs::write(root.join("real/f.txt"), "ok").unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        assert!(resolve_within(&root, "link/f.txt").is_ok());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn paginate_respects_offset_limit_and_truncation() {
        let data = b"0123456789";
        let first = page_end(&data[0..4], true);
        assert_eq!(first, 4);
        let last = page_end(&data[8..10], false);
        assert_eq!(last, 2);
    }

    #[test]
    fn page_end_keeps_full_slice_at_eof() {
        let data = "héllo".as_bytes();
        assert_eq!(page_end(data, false), data.len());
    }

    #[test]
    fn page_end_does_not_split_a_multibyte_char() {
        let s = "aé";
        let bytes = s.as_bytes();
        assert_eq!(bytes.len(), 3);
        let end = page_end(&bytes[..2], true);
        assert_eq!(end, 1);
        assert_eq!(&bytes[..end], b"a");
    }

    #[test]
    fn read_file_paginates_large_text_with_offset_and_truncated_flag() {
        let root = tmp();
        let big = "x".repeat(READ_PAGE_BYTES + 100);
        std::fs::write(root.join("big.txt"), &big).unwrap();

        let first = read_file(&root, "big.txt", 0).unwrap();
        match first {
            FileContent::Text {
                offset,
                next_offset,
                total,
                truncated,
                text,
            } => {
                assert_eq!(offset, 0);
                assert_eq!(next_offset, READ_PAGE_BYTES);
                assert_eq!(total, (READ_PAGE_BYTES + 100) as u64);
                assert!(truncated);
                assert_eq!(text.len(), READ_PAGE_BYTES);
            }
            other => panic!("esperava texto paginado, veio {other:?}"),
        }

        let second = read_file(&root, "big.txt", READ_PAGE_BYTES).unwrap();
        match second {
            FileContent::Text {
                next_offset,
                truncated,
                text,
                ..
            } => {
                assert!(!truncated);
                assert_eq!(next_offset, READ_PAGE_BYTES + 100);
                assert_eq!(text.len(), 100);
            }
            other => panic!("esperava a página final, veio {other:?}"),
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_file_flags_binary_by_nul_byte() {
        let root = tmp();
        std::fs::write(root.join("blob.bin"), [1u8, 2, 0, 3]).unwrap();
        match read_file(&root, "blob.bin", 0).unwrap() {
            FileContent::Binary { total, mime } => {
                assert_eq!(total, 4);
                assert!(mime.is_none());
            }
            other => panic!("esperava binário, veio {other:?}"),
        }
        std::fs::remove_dir_all(&root).ok();
    }

    fn fd(path: &str, status: FileStatus) -> FileDiff {
        FileDiff {
            path: path.to_string(),
            old_path: None,
            status,
            insertions: Some(1),
            deletions: Some(0),
        }
    }

    #[test]
    fn worktree_decoration_keeps_committed_status_over_dirty() {
        let diff = SessionDiff {
            root: "/x".into(),
            base_ref: "HEAD".into(),
            commits: vec![CommitInfo {
                sha: "abc".into(),
                subject: "s".into(),
                authored_at: "t".into(),
            }],
            files: vec![fd("src/a.rs", FileStatus::Modified)],
            staged_files: vec![fd("src/b.rs", FileStatus::Added)],
            unstaged_files: vec![
                fd("src/a.rs", FileStatus::Modified),
                fd("src/c.rs", FileStatus::Added),
            ],
        };
        let decos = decorations_from_diff(&diff);
        let by: HashMap<&str, DecoStatus> =
            decos.iter().map(|d| (d.path.as_str(), d.status)).collect();
        assert_eq!(by["src/a.rs"], DecoStatus::Modified);
        assert_eq!(by["src/b.rs"], DecoStatus::Added);
        assert_eq!(by["src/c.rs"], DecoStatus::Added);
        assert_eq!(decos.len(), 3);
    }

    #[test]
    fn repo_decoration_parses_nul_separated_status() {
        let raw = b" M src/keep.rs\0?? novo.txt\0A  staged.rs\0";
        let decos = parse_status_z(raw);
        let by: HashMap<&str, DecoStatus> =
            decos.iter().map(|d| (d.path.as_str(), d.status)).collect();
        assert_eq!(by["src/keep.rs"], DecoStatus::Modified);
        assert_eq!(by["novo.txt"], DecoStatus::Untracked);
        assert_eq!(by["staged.rs"], DecoStatus::Added);
        assert_eq!(decos.len(), 3);
    }

    #[test]
    fn repo_decoration_consumes_the_rename_origin_field() {
        let raw = b"R  novo.rs\0antigo.rs\0 M outro.rs\0";
        let decos = parse_status_z(raw);
        let by: HashMap<&str, DecoStatus> =
            decos.iter().map(|d| (d.path.as_str(), d.status)).collect();
        assert_eq!(by["novo.rs"], DecoStatus::Renamed);
        assert_eq!(by["outro.rs"], DecoStatus::Modified);
        assert!(
            !by.contains_key("antigo.rs"),
            "o path de origem do rename não é uma decoração própria"
        );
        assert_eq!(decos.len(), 2);
    }
}
