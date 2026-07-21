use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use parking_lot::Mutex;

use crate::session::SessionId;

pub mod confine;
pub mod fs;
pub mod gitexec;
pub mod sftpwire;

use crate::files::backend::FileBackend;
use crate::files::gutter::GutterMarker;
use crate::files::search::SearchOutcome;
use crate::files::write::{hash_bytes, WriteResult};
use crate::files::{
    image_mime, page_end, Decoration, DirListing, Entry, FileContent, PanelInfo, EDIT_MAX_BYTES,
    IMAGE_MAX_BYTES, MAX_DIR_ENTRIES, READ_PAGE_BYTES,
};
use confine::{confine_existing, confine_parent, join, split_leaf, validate_rel};
use fs::{mode_dir, RemoteError, RemoteFs, RemoteResult, RemoteStat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteContext {
    Repo,
    OutsideRepo,
}

impl RemoteContext {
    pub fn kind_str(&self) -> &'static str {
        match self {
            RemoteContext::Repo => "repo",
            RemoteContext::OutsideRepo => "outside_repo",
        }
    }

    pub fn decorated(&self) -> bool {
        matches!(self, RemoteContext::Repo)
    }
}

/// Backend SFTP: implementa a mesma trait `FileBackend` do `LocalBackend`, sobre
/// a conexão SSH multiplexada. Sem estado próprio — o cache de listagem e o
/// índice de fuzzy vivem no `RemotePanel`.
pub struct SftpBackend<'a> {
    fs: &'a dyn RemoteFs,
    root: &'a str,
    context: RemoteContext,
    alias: &'a str,
}

impl<'a> SftpBackend<'a> {
    pub fn new(
        fs: &'a dyn RemoteFs,
        root: &'a str,
        context: RemoteContext,
        alias: &'a str,
    ) -> Self {
        Self {
            fs,
            root,
            context,
            alias,
        }
    }

    fn read_capped(&self, path: &str, cap: usize) -> RemoteResult<Vec<u8>> {
        self.fs.read_chunk(path, 0, cap)
    }
}

fn leaf_of(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

impl FileBackend for SftpBackend<'_> {
    fn panel_info(&self) -> PanelInfo {
        PanelInfo {
            root: self.root.to_string(),
            kind: self.context.kind_str().to_string(),
            decorated: self.context.decorated(),
            remote: true,
            host: Some(self.alias.to_string()),
        }
    }

    fn list_dir(&self, dir_rel: &str) -> Result<DirListing, String> {
        let dir_norm = validate_rel(dir_rel)?;
        let dir_real = if dir_norm.is_empty() {
            self.root.to_string()
        } else {
            confine_existing(self.fs, self.root, &dir_norm)?
        };
        let raw = self.fs.readdir(&dir_real).map_err(String::from)?;
        let mut entries: Vec<Entry> = Vec::new();
        for e in raw {
            if e.name == ".git" {
                continue;
            }
            let rel_path = if dir_norm.is_empty() {
                e.name.clone()
            } else {
                format!("{dir_norm}/{}", e.name)
            };
            entries.push(Entry {
                name: e.name,
                rel_path,
                is_dir: e.stat.is_dir(),
                is_symlink: e.stat.is_symlink(),
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
        if matches!(self.context, RemoteContext::Repo) {
            let rels: Vec<String> = entries.iter().map(|e| e.rel_path.clone()).collect();
            let ignored = gitexec::remote_check_ignore(self.fs, self.root, &rels);
            for entry in entries.iter_mut() {
                if ignored.contains(&entry.rel_path) {
                    entry.gitignored = true;
                }
            }
        }
        Ok(DirListing {
            dir: dir_norm,
            entries,
            total,
            truncated,
        })
    }

    fn read_file(&self, rel: &str, offset: usize) -> Result<FileContent, String> {
        let target = confine_existing(self.fs, self.root, rel)?;
        let st = self.fs.stat(&target).map_err(String::from)?;
        if st.is_dir() {
            return Err("não é um arquivo".into());
        }
        let total = st.size;
        let name = leaf_of(rel);
        if let Some(mime) = image_mime(name) {
            if total <= IMAGE_MAX_BYTES {
                let bytes = self
                    .read_capped(&target, IMAGE_MAX_BYTES as usize)
                    .map_err(String::from)?;
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
        let buf = self
            .fs
            .read_chunk(&target, offset as u64, READ_PAGE_BYTES)
            .map_err(String::from)?;
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

    fn edit_begin(&self, rel: &str) -> Result<(String, String), String> {
        let target = confine_existing(self.fs, self.root, rel)?;
        let st = self.fs.stat(&target).map_err(String::from)?;
        if !st.is_file() {
            return Err("não é um arquivo".into());
        }
        let buf = self
            .fs
            .read_chunk(&target, 0, EDIT_MAX_BYTES as usize + 1)
            .map_err(String::from)?;
        if buf.len() as u64 > EDIT_MAX_BYTES {
            return Err("arquivo grande demais para editar no painel".into());
        }
        if buf.contains(&0) {
            return Err("arquivo binário não é editável".into());
        }
        let hash = hash_bytes(&buf);
        let text = String::from_utf8(buf).map_err(|_| "arquivo não é UTF-8 válido")?;
        Ok((text, hash))
    }

    fn write(&self, rel: &str, content: &str, expected_hash: &str) -> Result<WriteResult, String> {
        if content.len() as u64 > EDIT_MAX_BYTES {
            return Err("arquivo grande demais para salvar pelo painel".into());
        }
        let (parent_rel, leaf) = split_leaf(rel)?;
        let parent_real = confine_parent(self.fs, self.root, &parent_rel)?;
        let target = join(&parent_real, &leaf);

        let existing = match self.fs.lstat(&target) {
            Ok(st) => Some(st),
            Err(RemoteError::NotFound) => None,
            Err(e) => return Err(e.into()),
        };
        if let Some(st) = &existing {
            if st.is_symlink() {
                return Err("destino é um symlink; não é editável pelo painel".into());
            }
            if st.is_dir() {
                return Err("destino não é um arquivo comum".into());
            }
        }
        let (disk_hash, perm) = match &existing {
            Some(st) => {
                let bytes = self
                    .fs
                    .read_chunk(&target, 0, EDIT_MAX_BYTES as usize + 1)
                    .map_err(String::from)?;
                (Some(hash_bytes(&bytes)), st.perm())
            }
            None => (None, 0o644),
        };
        let matches = match &disk_hash {
            Some(h) => h == expected_hash,
            None => expected_hash.is_empty(),
        };
        if !matches {
            return Ok(WriteResult::Conflict { disk_hash });
        }
        let tmp = join(
            &parent_real,
            &format!(".tyba-write-{}", uuid::Uuid::new_v4()),
        );
        self.fs
            .write_new(&tmp, content.as_bytes(), perm)
            .map_err(String::from)?;
        if let Err(e) = self.fs.posix_rename(&tmp, &target) {
            let _ = self.fs.remove(&tmp);
            return Err(e.into());
        }
        Ok(WriteResult::Written {
            hash: hash_bytes(content.as_bytes()),
        })
    }

    fn create(&self, rel: &str, is_dir: bool) -> Result<(), String> {
        let (parent_rel, leaf) = split_leaf(rel)?;
        let parent_real = confine_parent(self.fs, self.root, &parent_rel)?;
        let target = join(&parent_real, &leaf);
        match self.fs.lstat(&target) {
            Ok(_) => return Err("já existe um item com esse nome".into()),
            Err(RemoteError::NotFound) => {}
            Err(e) => return Err(e.into()),
        }
        if is_dir {
            self.fs.mkdir(&target, mode_dir()).map_err(String::from)
        } else {
            self.fs.write_new(&target, &[], 0o644).map_err(String::from)
        }
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        let (from_parent, from_leaf) = split_leaf(from)?;
        let (to_parent, to_leaf) = split_leaf(to)?;
        let from_real = confine_parent(self.fs, self.root, &from_parent)?;
        let to_real = confine_parent(self.fs, self.root, &to_parent)?;
        let src = join(&from_real, &from_leaf);
        let dst = join(&to_real, &to_leaf);
        if src == dst {
            return Ok(());
        }
        match self.fs.lstat(&dst) {
            Ok(_) => return Err("já existe um item com esse nome".into()),
            Err(RemoteError::NotFound) => {}
            Err(e) => return Err(e.into()),
        }
        self.fs.rename(&src, &dst).map_err(String::from)
    }

    fn delete(&self, rel: &str) -> Result<(), String> {
        let (parent_rel, leaf) = split_leaf(rel)?;
        let parent_real = confine_parent(self.fs, self.root, &parent_rel)?;
        let target = join(&parent_real, &leaf);
        let st = self.fs.lstat(&target).map_err(String::from)?;
        remove_tree(self.fs, &target, &st).map_err(String::from)
    }

    fn decorations(&self) -> Vec<Decoration> {
        match self.context {
            RemoteContext::Repo => gitexec::remote_decorations(self.fs, self.root),
            RemoteContext::OutsideRepo => Vec::new(),
        }
    }

    fn search(&self, query: &str, limit: usize) -> SearchOutcome {
        match self.context {
            RemoteContext::Repo => {
                let (names, truncated) = gitexec::remote_ls_files(self.fs, self.root);
                SearchOutcome {
                    paths: crate::files::search::rank(&names, query, limit),
                    truncated,
                }
            }
            RemoteContext::OutsideRepo => SearchOutcome {
                paths: Vec::new(),
                truncated: false,
            },
        }
    }

    fn gutter(&self, rel: &str) -> Vec<GutterMarker> {
        match self.context {
            RemoteContext::Repo => gitexec::remote_gutter(self.fs, self.root, rel),
            RemoteContext::OutsideRepo => Vec::new(),
        }
    }
}

/// Remoção recursiva sem seguir symlink no topo: symlink → remove o link; dir →
/// esvazia e remove; arquivo → remove.
fn remove_tree(fs: &dyn RemoteFs, path: &str, st: &RemoteStat) -> RemoteResult<()> {
    if st.is_symlink() {
        return fs.remove(path);
    }
    if st.is_dir() {
        for entry in fs.readdir(path)? {
            let child = join(path, &entry.name);
            remove_tree(fs, &child, &entry.stat)?;
        }
        return fs.rmdir(path);
    }
    fs.remove(path)
}

/// Cwd remoto da sessão: `pane_current_path` do tmux invisível (1 exec) →
/// senão `.` (que o realpath do SFTP resolve para o home).
fn remote_cwd(fs: &dyn RemoteFs, tmux_name: Option<&str>) -> String {
    if let Some(name) = tmux_name {
        if let Ok(out) = fs.exec(&[
            "tmux",
            "display-message",
            "-p",
            "-t",
            name,
            "-F",
            "#{pane_current_path}",
        ]) {
            if out.ok() {
                let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !p.is_empty() {
                    return p;
                }
            }
        }
    }
    ".".to_string()
}

/// Resolve a raiz remota: repo root via `git rev-parse` no cwd → senão o cwd.
/// Função pura sobre o transporte — testada com mock.
pub fn resolve_remote_root(
    fs: &dyn RemoteFs,
    tmux_name: Option<&str>,
) -> RemoteResult<(String, RemoteContext)> {
    let cwd = remote_cwd(fs, tmux_name);
    let cwd_real = fs.realpath(&cwd)?;
    match gitexec::rev_parse_toplevel(fs, &cwd_real) {
        Some(top) => {
            let root = fs.realpath(&top)?;
            Ok((root, RemoteContext::Repo))
        }
        None => Ok((cwd_real, RemoteContext::OutsideRepo)),
    }
}

pub struct RemotePanel {
    fs: Arc<dyn RemoteFs>,
    alias: String,
    root: String,
    context: RemoteContext,
    listing: Mutex<HashMap<String, DirListing>>,
    fuzzy: Mutex<Option<(Vec<String>, bool)>>,
    open: Mutex<Option<String>>,
}

impl RemotePanel {
    pub fn new(fs: Arc<dyn RemoteFs>, alias: String, root: String, context: RemoteContext) -> Self {
        Self {
            fs,
            alias,
            root,
            context,
            listing: Mutex::new(HashMap::new()),
            fuzzy: Mutex::new(None),
            open: Mutex::new(None),
        }
    }

    fn backend(&self) -> SftpBackend<'_> {
        SftpBackend::new(&*self.fs, &self.root, self.context, &self.alias)
    }

    pub fn host(&self) -> &str {
        &self.alias
    }

    pub fn decorated(&self) -> bool {
        self.context.decorated()
    }

    pub fn info(&self) -> PanelInfo {
        self.backend().panel_info()
    }

    fn invalidate(&self) {
        self.listing.lock().clear();
        *self.fuzzy.lock() = None;
    }

    /// Refresh sob demanda: substitui o watcher. Limpa os caches para a próxima
    /// leitura reconsultar o host.
    pub fn refresh(&self) {
        self.invalidate();
    }

    pub fn list_dir(&self, dir: &str) -> Result<DirListing, String> {
        let key = validate_rel(dir)?;
        if let Some(cached) = self.listing.lock().get(&key) {
            return Ok(cached.clone());
        }
        let listing = self.backend().list_dir(&key)?;
        self.listing.lock().insert(key, listing.clone());
        Ok(listing)
    }

    pub fn read_file(&self, rel: &str, offset: usize) -> Result<FileContent, String> {
        self.backend().read_file(rel, offset)
    }

    pub fn edit_begin(&self, rel: &str) -> Result<(String, String), String> {
        self.backend().edit_begin(rel)
    }

    pub fn write(&self, rel: &str, content: &str, expected: &str) -> Result<WriteResult, String> {
        let result = self.backend().write(rel, content, expected)?;
        if matches!(result, WriteResult::Written { .. }) {
            self.invalidate();
        }
        Ok(result)
    }

    pub fn create(&self, rel: &str, is_dir: bool) -> Result<(), String> {
        self.backend().create(rel, is_dir)?;
        self.invalidate();
        Ok(())
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        self.backend().rename(from, to)?;
        self.invalidate();
        Ok(())
    }

    pub fn delete(&self, rel: &str) -> Result<(), String> {
        self.backend().delete(rel)?;
        self.invalidate();
        Ok(())
    }

    pub fn decorations(&self) -> Vec<Decoration> {
        self.backend().decorations()
    }

    pub fn search(&self, query: &str, limit: usize) -> SearchOutcome {
        if matches!(self.context, RemoteContext::OutsideRepo) {
            return SearchOutcome {
                paths: Vec::new(),
                truncated: false,
            };
        }
        let cached = {
            let mut guard = self.fuzzy.lock();
            if guard.is_none() {
                *guard = Some(gitexec::remote_ls_files(&*self.fs, &self.root));
            }
            guard.clone()
        };
        let (names, truncated) = cached.unwrap_or_default();
        SearchOutcome {
            paths: crate::files::search::rank(&names, query, limit),
            truncated,
        }
    }

    pub fn gutter(&self, rel: &str) -> Vec<GutterMarker> {
        self.backend().gutter(rel)
    }

    pub fn set_open(&self, rel: Option<String>) {
        *self.open.lock() = rel;
    }
}

#[derive(Default)]
pub struct RemoteFilesManager {
    panels: Mutex<HashMap<SessionId, Arc<RemotePanel>>>,
}

pub type SharedRemoteFiles = Arc<RemoteFilesManager>;

impl RemoteFilesManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: SessionId) -> Option<Arc<RemotePanel>> {
        self.panels.lock().get(&id).cloned()
    }

    /// Fechar o painel derruba o `RemotePanel` — o Drop do `SshRemote` mata o
    /// filho `ssh -s sftp`. Reabrir re-resolve a raiz (mesmo ciclo do local).
    pub fn close(&self, id: SessionId) {
        self.panels.lock().remove(&id);
    }

    pub fn ensure<F>(&self, id: SessionId, build: F) -> Result<Arc<RemotePanel>, String>
    where
        F: FnOnce() -> Result<Arc<RemotePanel>, String>,
    {
        if let Some(existing) = self.get(id) {
            return Ok(existing);
        }
        let panel = build()?;
        self.panels.lock().insert(id, Arc::clone(&panel));
        Ok(panel)
    }

    pub fn reseed(&self, id: SessionId, panel: Arc<RemotePanel>) {
        self.panels.lock().insert(id, panel);
    }
}

/// Abre a conexão SFTP na conexão multiplexada e monta o painel remoto.
pub fn build_panel(alias: &str, tmux_name: Option<&str>) -> Result<Arc<RemotePanel>, String> {
    let fs: Arc<dyn RemoteFs> =
        Arc::new(sftpwire::SshRemote::connect(alias).map_err(|e| e.message())?);
    let (root, context) = resolve_remote_root(&*fs, tmux_name).map_err(|e| e.message())?;
    Ok(Arc::new(RemotePanel::new(
        fs,
        alias.to_string(),
        root,
        context,
    )))
}

#[cfg(test)]
mod tests {
    use super::fs::{ExecOutput, RemoteDirEntry};
    use super::*;

    #[derive(Clone)]
    enum Node {
        File { bytes: Vec<u8>, perm: u32 },
        Dir,
        Symlink { target: String },
    }

    #[derive(Default)]
    struct MockRemote {
        nodes: Mutex<HashMap<String, Node>>,
        toplevel: Option<String>,
    }

    impl MockRemote {
        fn with(entries: &[(&str, Node)]) -> Self {
            let mut nodes = HashMap::new();
            for (path, node) in entries {
                nodes.insert((*path).to_string(), node.clone());
            }
            MockRemote {
                nodes: Mutex::new(nodes),
                toplevel: None,
            }
        }

        fn resolve(&self, path: &str) -> String {
            let mut acc = String::new();
            for seg in path.split('/') {
                match seg {
                    "" | "." => {}
                    ".." => {
                        if let Some(i) = acc.rfind('/') {
                            acc.truncate(i);
                        }
                    }
                    _ => {
                        let cand = format!("{acc}/{seg}");
                        if let Some(Node::Symlink { target }) = self.nodes.lock().get(&cand) {
                            acc = self.resolve(&target.clone());
                        } else {
                            acc = cand;
                        }
                    }
                }
            }
            if acc.is_empty() {
                "/".to_string()
            } else {
                acc
            }
        }

        fn stat_of(&self, key: &str) -> RemoteResult<RemoteStat> {
            match self.nodes.lock().get(key) {
                Some(Node::File { bytes, perm }) => Ok(RemoteStat {
                    size: bytes.len() as u64,
                    mode: fs::mode_file(*perm),
                }),
                Some(Node::Dir) => Ok(RemoteStat {
                    size: 0,
                    mode: mode_dir(),
                }),
                Some(Node::Symlink { .. }) => Ok(RemoteStat {
                    size: 0,
                    mode: fs::mode_symlink(),
                }),
                None => Err(RemoteError::NotFound),
            }
        }
    }

    impl RemoteFs for MockRemote {
        fn host(&self) -> &str {
            "mock"
        }
        fn realpath(&self, path: &str) -> RemoteResult<String> {
            Ok(self.resolve(path))
        }
        fn readdir(&self, path: &str) -> RemoteResult<Vec<RemoteDirEntry>> {
            let real = self.resolve(path);
            let prefix = format!("{real}/");
            let nodes = self.nodes.lock();
            let mut out = Vec::new();
            for (k, _) in nodes.iter() {
                if let Some(rest) = k.strip_prefix(&prefix) {
                    if !rest.is_empty() && !rest.contains('/') {
                        out.push(k.clone());
                    }
                }
            }
            drop(nodes);
            let mut entries = Vec::new();
            for k in out {
                let name = k.rsplit('/').next().unwrap().to_string();
                entries.push(RemoteDirEntry {
                    name,
                    stat: self.stat_of(&k)?,
                });
            }
            Ok(entries)
        }
        fn lstat(&self, path: &str) -> RemoteResult<RemoteStat> {
            // no-follow: resolve só o diretório-pai, mantém o leaf literal.
            let (parent, leaf) = match path.rsplit_once('/') {
                Some((p, l)) => (p, l),
                None => ("", path),
            };
            let key = format!("{}/{}", self.resolve(parent), leaf);
            self.stat_of(&key)
        }
        fn stat(&self, path: &str) -> RemoteResult<RemoteStat> {
            self.stat_of(&self.resolve(path))
        }
        fn read_chunk(&self, path: &str, offset: u64, len: usize) -> RemoteResult<Vec<u8>> {
            let key = self.resolve(path);
            match self.nodes.lock().get(&key) {
                Some(Node::File { bytes, .. }) => {
                    let start = (offset as usize).min(bytes.len());
                    let end = (start + len).min(bytes.len());
                    Ok(bytes[start..end].to_vec())
                }
                Some(_) => Err(RemoteError::Denied),
                None => Err(RemoteError::NotFound),
            }
        }
        fn write_new(&self, path: &str, data: &[u8], mode: u32) -> RemoteResult<()> {
            let (parent, leaf) = path.rsplit_once('/').unwrap();
            let key = format!("{}/{}", self.resolve(parent), leaf);
            let mut nodes = self.nodes.lock();
            if nodes.contains_key(&key) {
                return Err(RemoteError::Sftp {
                    code: 4,
                    message: "exists".into(),
                });
            }
            nodes.insert(
                key,
                Node::File {
                    bytes: data.to_vec(),
                    perm: mode & 0o7777,
                },
            );
            Ok(())
        }
        fn posix_rename(&self, from: &str, to: &str) -> RemoteResult<()> {
            let mut nodes = self.nodes.lock();
            let node = nodes.remove(from).ok_or(RemoteError::NotFound)?;
            nodes.insert(to.to_string(), node);
            Ok(())
        }
        fn rename(&self, from: &str, to: &str) -> RemoteResult<()> {
            self.posix_rename(from, to)
        }
        fn remove(&self, path: &str) -> RemoteResult<()> {
            let (parent, leaf) = path.rsplit_once('/').unwrap();
            let key = format!("{}/{}", self.resolve(parent), leaf);
            self.nodes
                .lock()
                .remove(&key)
                .ok_or(RemoteError::NotFound)?;
            Ok(())
        }
        fn rmdir(&self, path: &str) -> RemoteResult<()> {
            self.nodes
                .lock()
                .remove(path)
                .ok_or(RemoteError::NotFound)?;
            Ok(())
        }
        fn mkdir(&self, path: &str, _mode: u32) -> RemoteResult<()> {
            self.nodes.lock().insert(path.to_string(), Node::Dir);
            Ok(())
        }
        fn exec(&self, argv: &[&str]) -> RemoteResult<ExecOutput> {
            if argv.contains(&"rev-parse") {
                return match &self.toplevel {
                    Some(top) => Ok(ExecOutput {
                        code: 0,
                        stdout: format!("{top}\n").into_bytes(),
                        stderr: Vec::new(),
                    }),
                    None => Ok(ExecOutput {
                        code: 128,
                        stdout: Vec::new(),
                        stderr: b"not a git repo".to_vec(),
                    }),
                };
            }
            Ok(ExecOutput {
                code: 1,
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        }
    }

    fn panel(mock: MockRemote, root: &str, ctx: RemoteContext) -> RemotePanel {
        RemotePanel::new(Arc::new(mock), "mock".into(), root.to_string(), ctx)
    }

    #[test]
    fn confinement_refuses_read_through_a_symlink_escaping_root() {
        let mock = MockRemote::with(&[
            ("/root", Node::Dir),
            (
                "/root/escape",
                Node::Symlink {
                    target: "/outside".into(),
                },
            ),
            ("/outside", Node::Dir),
            (
                "/outside/secret.txt",
                Node::File {
                    bytes: b"leak".to_vec(),
                    perm: 0o644,
                },
            ),
        ]);
        let p = panel(mock, "/root", RemoteContext::OutsideRepo);
        let err = p.read_file("escape/secret.txt", 0).unwrap_err();
        assert!(
            err.contains("negada") || err.contains("raiz") || err.contains("fora"),
            "symlink que escapa da raiz é recusado por realpath; veio: {err}"
        );
    }

    #[test]
    fn confinement_refuses_write_into_an_escaping_symlink_dir() {
        let mock = MockRemote::with(&[
            ("/root", Node::Dir),
            (
                "/root/sub",
                Node::Symlink {
                    target: "/outside".into(),
                },
            ),
            ("/outside", Node::Dir),
        ]);
        let p = panel(mock, "/root", RemoteContext::OutsideRepo);
        let err = p.write("sub/f.txt", "x", "").unwrap_err();
        assert!(
            err.contains("negada") || err.contains("raiz"),
            "veio: {err}"
        );
    }

    #[test]
    fn hash_guard_refuses_a_divergent_write_and_keeps_disk() {
        let mock = MockRemote::with(&[
            ("/root", Node::Dir),
            (
                "/root/f.txt",
                Node::File {
                    bytes: b"on disk".to_vec(),
                    perm: 0o644,
                },
            ),
        ]);
        let p = panel(mock, "/root", RemoteContext::OutsideRepo);
        let stale = hash_bytes(b"what the ui loaded");
        let result = p.write("f.txt", "clobber", &stale).unwrap();
        match result {
            WriteResult::Conflict { disk_hash } => {
                assert_eq!(disk_hash, Some(hash_bytes(b"on disk")));
            }
            other => panic!("esperava conflito, veio {other:?}"),
        }
        match p.read_file("f.txt", 0).unwrap() {
            FileContent::Text { text, .. } => {
                assert_eq!(text, "on disk", "o disco remoto não pode ter sido tocado")
            }
            other => panic!("veio {other:?}"),
        }
    }

    #[test]
    fn hash_guard_applies_a_matching_write() {
        let mock = MockRemote::with(&[
            ("/root", Node::Dir),
            (
                "/root/f.txt",
                Node::File {
                    bytes: b"v1".to_vec(),
                    perm: 0o600,
                },
            ),
        ]);
        let p = panel(mock, "/root", RemoteContext::OutsideRepo);
        let result = p.write("f.txt", "v2", &hash_bytes(b"v1")).unwrap();
        assert!(matches!(result, WriteResult::Written { .. }));
        match p.read_file("f.txt", 0).unwrap() {
            FileContent::Text { text, .. } => assert_eq!(text, "v2"),
            other => panic!("veio {other:?}"),
        }
    }

    #[test]
    fn write_through_a_symlink_leaf_is_refused() {
        let mock = MockRemote::with(&[
            ("/root", Node::Dir),
            (
                "/root/link",
                Node::Symlink {
                    target: "/outside/x".into(),
                },
            ),
            ("/outside", Node::Dir),
            (
                "/outside/x",
                Node::File {
                    bytes: b"outside".to_vec(),
                    perm: 0o644,
                },
            ),
        ]);
        let p = panel(mock, "/root", RemoteContext::OutsideRepo);
        let err = p.write("link", "leak", "").unwrap_err();
        assert!(err.contains("symlink"), "veio: {err}");
    }

    #[test]
    fn delete_of_a_symlink_removes_the_link_not_the_target() {
        let fs = Arc::new(MockRemote::with(&[
            ("/root", Node::Dir),
            (
                "/root/link",
                Node::Symlink {
                    target: "/outside/keep.txt".into(),
                },
            ),
            ("/outside", Node::Dir),
            (
                "/outside/keep.txt",
                Node::File {
                    bytes: b"outside".to_vec(),
                    perm: 0o644,
                },
            ),
        ]));
        let p = RemotePanel::new(
            fs.clone(),
            "mock".into(),
            "/root".into(),
            RemoteContext::OutsideRepo,
        );
        p.delete("link").unwrap();
        assert!(
            fs.nodes.lock().get("/outside/keep.txt").is_some(),
            "o alvo do symlink nunca é removido"
        );
        assert!(
            fs.nodes.lock().get("/root/link").is_none(),
            "o link em si foi removido"
        );
    }

    #[test]
    fn listing_cache_is_invalidated_by_a_write_not_by_time() {
        let mock = MockRemote::with(&[
            ("/root", Node::Dir),
            (
                "/root/a.txt",
                Node::File {
                    bytes: b"a".to_vec(),
                    perm: 0o644,
                },
            ),
        ]);
        let p = panel(mock, "/root", RemoteContext::OutsideRepo);
        let first = p.list_dir("").unwrap();
        assert_eq!(first.entries.len(), 1);
        assert_eq!(p.listing.lock().len(), 1, "listagem entrou no cache");

        // Sem ação de escrita, o cache permanece (nunca expira por tempo).
        let _ = p.list_dir("").unwrap();
        assert_eq!(p.listing.lock().len(), 1);

        p.create("b.txt", false).unwrap();
        assert!(
            p.listing.lock().is_empty(),
            "uma ação de escrita invalida o cache de listagem"
        );
        let after = p.list_dir("").unwrap();
        assert_eq!(after.entries.len(), 2, "a releitura vê o arquivo novo");
    }

    #[test]
    fn resolve_root_uses_repo_toplevel_when_in_a_repo() {
        let mut mock = MockRemote::with(&[("/home/u/proj", Node::Dir)]);
        mock.toplevel = Some("/home/u/proj".into());
        let (root, ctx) = resolve_remote_root(&mock, None).unwrap();
        // cwd fallback "." resolve para "/" no mock; rev-parse devolve o toplevel.
        assert_eq!(root, "/home/u/proj");
        assert_eq!(ctx, RemoteContext::Repo);
    }

    #[test]
    fn resolve_root_falls_back_to_cwd_outside_a_repo() {
        let mock = MockRemote::with(&[("/home/u", Node::Dir)]);
        let (_root, ctx) = resolve_remote_root(&mock, None).unwrap();
        assert_eq!(ctx, RemoteContext::OutsideRepo);
    }

    #[test]
    fn delete_of_a_regular_file_removes_it() {
        let mock = MockRemote::with(&[
            ("/root", Node::Dir),
            (
                "/root/f.txt",
                Node::File {
                    bytes: b"x".to_vec(),
                    perm: 0o644,
                },
            ),
        ]);
        let fs: Arc<MockRemote> = Arc::new(mock);
        let p = RemotePanel::new(
            fs.clone(),
            "mock".into(),
            "/root".into(),
            RemoteContext::OutsideRepo,
        );
        p.delete("f.txt").unwrap();
        assert!(fs.nodes.lock().get("/root/f.txt").is_none());
    }
}
