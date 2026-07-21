use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer_opt, DebounceEventResult, Debouncer, NoCache};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::session::SessionId;
use crate::worktree::diff::{FileStatus, SessionDiff};

pub mod gutter;
pub mod search;
pub mod write;

use gutter::GutterMarker;
use search::FuzzyIndex;

pub const EVENT_TREE_PREFIX: &str = "files://tree/";
pub const EVENT_DECORATIONS_PREFIX: &str = "files://decorations/";
pub const EVENT_CONFLICT_PREFIX: &str = "files://conflict/";
pub const EVENT_GUTTER_PREFIX: &str = "files://gutter/";

pub const EDIT_MAX_BYTES: u64 = 8 * 1024 * 1024;

pub const READ_PAGE_BYTES: usize = 256 * 1024;
pub const MAX_DIR_ENTRIES: usize = 2000;
const IMAGE_MAX_BYTES: u64 = 8 * 1024 * 1024;
const WATCH_DEBOUNCE: Duration = Duration::from_millis(200);

pub fn tree_event(id: SessionId) -> String {
    format!("{EVENT_TREE_PREFIX}{id}")
}

pub fn decorations_event(id: SessionId) -> String {
    format!("{EVENT_DECORATIONS_PREFIX}{id}")
}

pub fn conflict_event(id: SessionId) -> String {
    format!("{EVENT_CONFLICT_PREFIX}{id}")
}

pub fn gutter_event(id: SessionId) -> String {
    format!("{EVENT_GUTTER_PREFIX}{id}")
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
    pub total: usize,
    pub truncated: bool,
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

#[derive(Clone, Serialize)]
struct GutterPayload {
    path: String,
    markers: Vec<GutterMarker>,
}

#[derive(Clone, Serialize)]
struct ConflictPayload {
    path: String,
    disk_hash: Option<String>,
}

fn file_hash(root: &Path, abs: &Path) -> Option<String> {
    let (mut file, _real) = open_verified(root, abs).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    Some(write::hash_bytes(&buf))
}

fn join_rel(dir_rel: &str, name: &str) -> String {
    if dir_rel.is_empty() {
        name.to_string()
    } else {
        format!("{dir_rel}/{name}")
    }
}

pub(crate) fn rel_of(root: &Path, path: &Path) -> Option<String> {
    let stripped = path.strip_prefix(root).ok()?;
    let s = stripped.to_string_lossy().replace('\\', "/");
    Some(s)
}

pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let mut cur = Some(start);
    while let Some(dir) = cur {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
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
    use std::io::Write;
    use std::process::Stdio;
    if rels.is_empty() {
        return HashSet::new();
    }
    let mut cmd = crate::worktree::git_in(root);
    cmd.env("GIT_LITERAL_PATHSPECS", "1");
    cmd.args(["check-ignore", "-z", "--stdin"]);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    let Ok(mut child) = cmd.spawn() else {
        return HashSet::new();
    };
    let stdin = child.stdin.take();
    let mut payload = Vec::new();
    for rel in rels {
        payload.extend_from_slice(rel.as_bytes());
        payload.push(0);
    }
    let writer = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(&payload);
        }
    });
    let output = child.wait_with_output();
    let _ = writer.join();
    match output {
        Ok(out) => out
            .stdout
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
        entries.push(Entry {
            name,
            rel_path: rel,
            is_dir,
            is_symlink,
            gitignored: false,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let total = entries.len();
    let truncated = total > MAX_DIR_ENTRIES;
    if truncated {
        entries.truncate(MAX_DIR_ENTRIES);
    }
    if !matches!(context, Context::OutsideRepo) {
        let kept: Vec<String> = entries.iter().map(|e| e.rel_path.clone()).collect();
        let ignored = ignored_set(root, &kept);
        for entry in entries.iter_mut() {
            if ignored.contains(&entry.rel_path) {
                entry.gitignored = true;
            }
        }
    }
    Ok(DirListing {
        dir: dir_rel.to_string(),
        entries,
        total,
        truncated,
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

#[cfg(target_os = "macos")]
pub(crate) fn fd_real_path_raw(fd: std::os::unix::io::RawFd) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut buf = [0u8; libc::PATH_MAX as usize];
    let rc = unsafe { libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr()) };
    if rc < 0 {
        return None;
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..end])))
}

#[cfg(target_os = "linux")]
pub(crate) fn fd_real_path_raw(fd: std::os::unix::io::RawFd) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/self/fd/{fd}")).ok()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn fd_real_path(file: &std::fs::File) -> Option<PathBuf> {
    use std::os::unix::io::AsRawFd;
    fd_real_path_raw(file.as_raw_fd())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn fd_real_path(_file: &std::fs::File) -> Option<PathBuf> {
    None
}

pub(crate) fn open_verified(root: &Path, full: &Path) -> Result<(std::fs::File, PathBuf), String> {
    let file = std::fs::File::open(full).map_err(|e| format!("falha ao abrir: {e}"))?;
    let root_canon = std::fs::canonicalize(root).map_err(|e| format!("raiz inacessível: {e}"))?;
    let real = match fd_real_path(&file) {
        Some(path) => std::fs::canonicalize(&path).unwrap_or(path),
        None => {
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            {
                return Err("não foi possível revalidar o arquivo aberto".into());
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                std::fs::canonicalize(full).map_err(|e| format!("arquivo inacessível: {e}"))?
            }
        }
    };
    if !real.starts_with(&root_canon) {
        return Err("arquivo fora da raiz do painel".into());
    }
    Ok((file, real))
}

fn read_page(file: &mut std::fs::File, offset: usize, len: usize) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
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

fn read_capped(file: &mut std::fs::File, cap: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut buf = Vec::new();
    file.take(cap as u64)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read: {e}"))?;
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
    let (mut file, real) = open_verified(root, &full)?;
    let meta = file
        .metadata()
        .map_err(|e| format!("arquivo inacessível: {e}"))?;
    if !meta.is_file() {
        return Err("não é um arquivo".into());
    }
    let total = meta.len();
    let name = real
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();

    if let Some(mime) = image_mime(&name) {
        if total <= IMAGE_MAX_BYTES {
            let bytes = read_capped(&mut file, IMAGE_MAX_BYTES as usize)?;
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

    let buf = read_page(&mut file, offset, READ_PAGE_BYTES)?;
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

type FileDebouncer = Debouncer<notify::RecommendedWatcher, NoCache>;

#[derive(Debug, Clone)]
struct OpenFile {
    rel: String,
    abs: PathBuf,
    editing: bool,
    hash: Option<String>,
}

struct Panel {
    root: PathBuf,
    context: Context,
    watched: Arc<Mutex<HashSet<PathBuf>>>,
    debouncer: Option<FileDebouncer>,
    index: Arc<FuzzyIndex>,
    open: Arc<Mutex<Option<OpenFile>>>,
}

impl Panel {
    fn watch_parent(&mut self, abs: &Path) {
        let Some(parent) = abs.parent() else {
            return;
        };
        let parent = parent.to_path_buf();
        if self.watched.lock().insert(parent.clone()) {
            if let Some(debouncer) = self.debouncer.as_mut() {
                let _ = debouncer.watch(&parent, RecursiveMode::NonRecursive);
            }
        }
    }
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
        let index = Arc::new(FuzzyIndex::new());
        let open = Arc::new(Mutex::new(None));
        let debouncer = spawn_watcher(
            app.clone(),
            id,
            root.clone(),
            context.clone(),
            &watched,
            &index,
            &open,
        );
        self.panels.lock().insert(
            id,
            Panel {
                root,
                context,
                watched,
                debouncer,
                index,
                open,
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

    pub fn search(&self, id: SessionId, query: &str, limit: usize) -> search::SearchOutcome {
        let target = {
            let panels = self.panels.lock();
            panels
                .get(&id)
                .map(|p| (Arc::clone(&p.index), p.root.clone(), p.context.clone()))
        };
        let Some((index, root, context)) = target else {
            return search::SearchOutcome {
                paths: Vec::new(),
                truncated: false,
            };
        };
        index.search(&root, &context, query, limit)
    }

    pub fn invalidate_index(&self, id: SessionId) {
        if let Some(panel) = self.panels.lock().get(&id) {
            panel.index.invalidate();
        }
    }

    pub fn gutter(&self, id: SessionId, rel: &str) -> Vec<GutterMarker> {
        let Some((root, context)) = self.info(id) else {
            return Vec::new();
        };
        gutter::compute(&root, &context, rel)
    }

    pub fn set_open(&self, id: SessionId, rel: Option<String>) {
        let mut panels = self.panels.lock();
        let Some(panel) = panels.get_mut(&id) else {
            return;
        };
        match rel {
            None => {
                *panel.open.lock() = None;
            }
            Some(rel) => {
                let Ok(abs) = resolve_within(&panel.root, &rel) else {
                    return;
                };
                panel.watch_parent(&abs);
                *panel.open.lock() = Some(OpenFile {
                    rel,
                    abs,
                    editing: false,
                    hash: None,
                });
            }
        }
    }

    pub fn edit_begin(&self, id: SessionId, rel: &str) -> Result<(String, String), String> {
        let (root, _) = self.info(id).ok_or("painel não encontrado")?;
        let full = resolve_within(&root, rel)?;
        let (mut file, real) = open_verified(&root, &full)?;
        let meta = file
            .metadata()
            .map_err(|e| format!("arquivo inacessível: {e}"))?;
        if !meta.is_file() {
            return Err("não é um arquivo".into());
        }
        if meta.len() > EDIT_MAX_BYTES {
            return Err("arquivo grande demais para editar no painel".into());
        }
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| format!("falha ao ler: {e}"))?;
        if buf.contains(&0) {
            return Err("arquivo binário não é editável".into());
        }
        let hash = write::hash_bytes(&buf);
        let text = String::from_utf8(buf).map_err(|_| "arquivo não é UTF-8 válido")?;

        let mut panels = self.panels.lock();
        if let Some(panel) = panels.get_mut(&id) {
            panel.watch_parent(&real);
            *panel.open.lock() = Some(OpenFile {
                rel: rel.to_string(),
                abs: real,
                editing: true,
                hash: Some(hash.clone()),
            });
        }
        Ok((text, hash))
    }

    pub fn edit_end(&self, id: SessionId) {
        if let Some(panel) = self.panels.lock().get(&id) {
            if let Some(open) = panel.open.lock().as_mut() {
                open.editing = false;
                open.hash = None;
            }
        }
    }

    /// Após um save bem-sucedido: re-captura o hash do conteúdo escrito para que
    /// o evento do watcher gerado pelo próprio write não vire falso conflito.
    pub fn note_written(&self, id: SessionId, rel: &str, hash: &str) {
        if let Some(panel) = self.panels.lock().get(&id) {
            if let Some(open) = panel.open.lock().as_mut() {
                if open.rel == rel && open.editing {
                    open.hash = Some(hash.to_string());
                }
            }
        }
    }

    pub fn emit_gutter<R: Runtime>(&self, app: &AppHandle<R>, id: SessionId, rel: &str) {
        let markers = self.gutter(id, rel);
        let _ = app.emit(
            &gutter_event(id),
            GutterPayload {
                path: rel.to_string(),
                markers,
            },
        );
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

#[allow(clippy::too_many_arguments)]
fn spawn_watcher<R: Runtime>(
    app: AppHandle<R>,
    id: SessionId,
    root: PathBuf,
    context: Context,
    watched: &Arc<Mutex<HashSet<PathBuf>>>,
    index: &Arc<FuzzyIndex>,
    open: &Arc<Mutex<Option<OpenFile>>>,
) -> Option<FileDebouncer> {
    let tree_ev = tree_event(id);
    let deco_ev = decorations_event(id);
    let gutter_ev = gutter_event(id);
    let conflict_ev = conflict_event(id);
    let cb_root = root.clone();
    let cb_context = context.clone();
    let cb_watched = Arc::clone(watched);
    let cb_index = Arc::clone(index);
    let cb_open = Arc::clone(open);

    let mut debouncer = new_debouncer_opt::<_, notify::RecommendedWatcher, NoCache>(
        WATCH_DEBOUNCE,
        None,
        move |result: DebounceEventResult| {
            let Ok(events) = result else {
                return;
            };
            let open_snapshot = cb_open.lock().clone();
            let mut touched_git = false;
            let mut open_touched = false;
            let mut affected: HashSet<PathBuf> = HashSet::new();
            {
                let live = cb_watched.lock();
                for event in &events {
                    for path in &event.paths {
                        if crate::repo::is_watched_event_path(path) {
                            touched_git = true;
                        }
                        if let Some(of) = &open_snapshot {
                            if path == &of.abs {
                                open_touched = true;
                            }
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
            if touched_git || !affected.is_empty() || open_touched {
                cb_index.invalidate();
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
            if let Some(of) = &open_snapshot {
                if open_touched || touched_git {
                    let markers = gutter::compute(&cb_root, &cb_context, &of.rel);
                    let _ = app.emit(
                        &gutter_ev,
                        GutterPayload {
                            path: of.rel.clone(),
                            markers,
                        },
                    );
                }
                if open_touched && of.editing {
                    if let Some(expected) = &of.hash {
                        let disk = file_hash(&cb_root, &of.abs);
                        if disk.as_deref() != Some(expected.as_str()) {
                            let _ = app.emit(
                                &conflict_ev,
                                ConflictPayload {
                                    path: of.rel.clone(),
                                    disk_hash: disk,
                                },
                            );
                        }
                    }
                }
            }
        },
        NoCache,
        notify::Config::default(),
    )
    .ok()?;

    if !matches!(context, Context::OutsideRepo) {
        for dir in crate::repo::watch_dirs(&root) {
            let mode = if dir.ends_with("refs/heads") {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            let _ = debouncer.watch(&dir, mode);
        }
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

    #[test]
    fn open_verified_accepts_a_file_inside_root() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "ok").unwrap();
        let full = resolve_within(&root, "a.txt").unwrap();
        let (_file, real) = open_verified(&root, &full).expect("arquivo dentro da raiz abre");
        let root_canon = std::fs::canonicalize(&root).unwrap();
        assert!(real.starts_with(&root_canon));
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn open_verified_rejects_an_fd_resolving_outside_root() {
        let root = tmp();
        let outside = tmp();
        std::fs::write(outside.join("secret.txt"), "leak").unwrap();
        let link = root.join("escape");
        std::os::unix::fs::symlink(outside.join("secret.txt"), &link).unwrap();
        let err = open_verified(&root, &link)
            .expect_err("fd que resolve pra fora da raiz é recusado na revalidação");
        assert!(err.contains("fora"));
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn root_is_fixed_while_open_but_reopen_after_close_re_resolves() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let mgr = FilesManager::new();
        let id = uuid::Uuid::new_v4();
        let first = tmp();
        let second = tmp();

        mgr.seed(&handle, id, first.clone(), Context::OutsideRepo);
        assert_eq!(mgr.info(id).map(|(root, _)| root), Some(first.clone()));

        let (fixed, _) = mgr
            .ensure(&handle, id, || Ok((second.clone(), Context::OutsideRepo)))
            .unwrap();
        assert_eq!(fixed, first, "a raiz é fixa enquanto o painel está aberto");

        mgr.close(id);
        assert!(mgr.info(id).is_none(), "fechar derruba o painel no core");

        let (reopened, _) = mgr
            .ensure(&handle, id, || Ok((second.clone(), Context::OutsideRepo)))
            .unwrap();
        assert_eq!(reopened, second, "reabrir re-resolve a raiz");

        std::fs::remove_dir_all(&first).ok();
        std::fs::remove_dir_all(&second).ok();
    }

    #[test]
    fn find_repo_root_walks_up_to_the_dot_git() {
        let base = tmp();
        let repo = base.join("proj");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let deep = repo.join("src/inner");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(find_repo_root(&deep).as_deref(), Some(repo.as_path()));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn find_repo_root_does_not_flag_a_dir_without_dot_git() {
        let base = tmp();
        let plain = base.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert_ne!(find_repo_root(&plain).as_deref(), Some(plain.as_path()));
        std::fs::remove_dir_all(&base).ok();
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
