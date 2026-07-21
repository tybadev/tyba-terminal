use std::path::PathBuf;

use super::gutter::GutterMarker;
use super::search::SearchOutcome;
use super::write::WriteResult;
use super::{Context, Decoration, DirListing, FileContent, PanelInfo};

pub trait FileBackend {
    fn panel_info(&self) -> PanelInfo;
    fn list_dir(&self, dir_rel: &str) -> Result<DirListing, String>;
    fn read_file(&self, rel: &str, offset: usize) -> Result<FileContent, String>;
    fn edit_begin(&self, rel: &str) -> Result<(String, String), String>;
    fn write(&self, rel: &str, content: &str, expected_hash: &str) -> Result<WriteResult, String>;
    fn create(&self, rel: &str, is_dir: bool) -> Result<(), String>;
    fn rename(&self, from: &str, to: &str) -> Result<(), String>;
    fn delete(&self, rel: &str) -> Result<(), String>;
    fn decorations(&self) -> Vec<Decoration>;
    fn search(&self, query: &str, limit: usize) -> SearchOutcome;
    fn gutter(&self, rel: &str) -> Vec<GutterMarker>;
}

pub struct LocalBackend {
    pub root: PathBuf,
    pub context: Context,
}

impl LocalBackend {
    pub fn new(root: PathBuf, context: Context) -> Self {
        Self { root, context }
    }
}

impl FileBackend for LocalBackend {
    fn panel_info(&self) -> PanelInfo {
        PanelInfo {
            root: self.root.to_string_lossy().into_owned(),
            kind: self.context.kind_str().to_string(),
            decorated: !matches!(self.context, Context::OutsideRepo),
            remote: false,
            host: None,
        }
    }

    fn list_dir(&self, dir_rel: &str) -> Result<DirListing, String> {
        super::list_dir(&self.root, dir_rel, &self.context)
    }

    fn read_file(&self, rel: &str, offset: usize) -> Result<FileContent, String> {
        super::read_file(&self.root, rel, offset)
    }

    fn edit_begin(&self, rel: &str) -> Result<(String, String), String> {
        let full = super::resolve_within(&self.root, rel)?;
        let (mut file, _real) = super::open_verified(&self.root, &full)?;
        let meta = file
            .metadata()
            .map_err(|e| format!("arquivo inacessível: {e}"))?;
        if !meta.is_file() {
            return Err("não é um arquivo".into());
        }
        if meta.len() > super::EDIT_MAX_BYTES {
            return Err("arquivo grande demais para editar no painel".into());
        }
        use std::io::Read;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| format!("falha ao ler: {e}"))?;
        if buf.contains(&0) {
            return Err("arquivo binário não é editável".into());
        }
        let hash = super::write::hash_bytes(&buf);
        let text = String::from_utf8(buf).map_err(|_| "arquivo não é UTF-8 válido")?;
        Ok((text, hash))
    }

    fn write(&self, rel: &str, content: &str, expected_hash: &str) -> Result<WriteResult, String> {
        super::write::write_file(&self.root, rel, content, expected_hash)
    }

    fn create(&self, rel: &str, is_dir: bool) -> Result<(), String> {
        super::write::create(&self.root, rel, is_dir)
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        super::write::rename(&self.root, from, to)
    }

    fn delete(&self, rel: &str) -> Result<(), String> {
        super::write::delete(&self.root, rel)
    }

    fn decorations(&self) -> Vec<Decoration> {
        super::compute_decorations(&self.root, &self.context)
    }

    fn search(&self, query: &str, limit: usize) -> SearchOutcome {
        super::search::FuzzyIndex::new().search(&self.root, &self.context, query, limit)
    }

    fn gutter(&self, rel: &str) -> Vec<GutterMarker> {
        super::gutter::compute(&self.root, &self.context, rel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tyba-backend-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    #[test]
    fn local_backend_lists_reads_and_reports_like_the_free_functions() {
        let root = tmp();
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let backend = LocalBackend::new(root.clone(), Context::OutsideRepo);

        let info = backend.panel_info();
        assert!(!info.remote);
        assert_eq!(info.kind, "outside_repo");

        let listing = backend.list_dir("").unwrap();
        assert!(listing.entries.iter().any(|e| e.name == "a.txt"));

        match backend.read_file("a.txt", 0).unwrap() {
            FileContent::Text { text, .. } => assert_eq!(text, "hello"),
            other => panic!("esperava texto, veio {other:?}"),
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn local_backend_edit_write_round_trips_with_hash_guard() {
        let root = tmp();
        std::fs::write(root.join("f.txt"), "v1").unwrap();
        let backend = LocalBackend::new(root.clone(), Context::OutsideRepo);

        let (text, hash) = backend.edit_begin("f.txt").unwrap();
        assert_eq!(text, "v1");
        let result = backend.write("f.txt", "v2", &hash).unwrap();
        assert!(matches!(result, WriteResult::Written { .. }));
        assert_eq!(std::fs::read_to_string(root.join("f.txt")).unwrap(), "v2");

        let stale = backend.write("f.txt", "v3", "deadbeef").unwrap();
        assert!(matches!(stale, WriteResult::Conflict { .. }));
        std::fs::remove_dir_all(&root).ok();
    }
}
