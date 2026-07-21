use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};

pub mod client;
pub mod document;
pub mod jsonrpc;
pub mod registry;
pub mod sandbox;
pub mod uri;

use client::{base_env, DiagEmitter, LspServer};
use document::ContentChange;
use registry::{entry_for_file, ServerEntry};
use sandbox::LspSpecCtx;

use crate::session::SessionId;

pub const EVENT_DIAGNOSTICS_PREFIX: &str = "lsp://diagnostics/";
const IDLE_KILL: Duration = Duration::from_secs(300);
const REAP_INTERVAL: Duration = Duration::from_secs(45);

pub fn diagnostics_event(id: SessionId) -> String {
    format!("{EVENT_DIAGNOSTICS_PREFIX}{id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Starting,
    Ready,
    Crashed,
    Stopped,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct PosIpc {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct RangeIpc {
    pub start: PosIpc,
    pub end: PosIpc,
}

impl RangeIpc {
    fn from_lsp(value: &Value) -> Option<Self> {
        let start = value.get("start")?;
        let end = value.get("end")?;
        Some(RangeIpc {
            start: PosIpc {
                line: start.get("line")?.as_u64()? as u32,
                character: start.get("character")?.as_u64()? as u32,
            },
            end: PosIpc {
                line: end.get("line")?.as_u64()? as u32,
                character: end.get("character")?.as_u64()? as u32,
            },
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticIpc {
    pub range: RangeIpc,
    pub severity: u8,
    pub message: String,
    pub source: Option<String>,
}

impl DiagnosticIpc {
    pub fn from_lsp(value: &Value) -> Option<Self> {
        let range = RangeIpc::from_lsp(value.get("range")?)?;
        let severity = value.get("severity").and_then(Value::as_u64).unwrap_or(1) as u8;
        let message = value.get("message").and_then(Value::as_str)?.to_string();
        let source = value
            .get("source")
            .and_then(Value::as_str)
            .map(str::to_string);
        Some(DiagnosticIpc {
            range,
            severity,
            message,
            source,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FileDiagnostics {
    pub path: String,
    pub diagnostics: Vec<DiagnosticIpc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsBatch {
    pub files: Vec<FileDiagnostics>,
    pub files_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
    pub kind: Option<u8>,
    pub insert_text: Option<String>,
    pub sort_text: Option<String>,
    pub filter_text: Option<String>,
}

impl CompletionItem {
    fn from_lsp(value: &Value) -> Option<Self> {
        let label = value.get("label").and_then(Value::as_str)?.to_string();
        let insert_text = value
            .get("textEdit")
            .and_then(|te| te.get("newText"))
            .and_then(Value::as_str)
            .or_else(|| value.get("insertText").and_then(Value::as_str))
            .map(str::to_string);
        Some(CompletionItem {
            label,
            detail: value
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_string),
            kind: value.get("kind").and_then(Value::as_u64).map(|k| k as u8),
            insert_text,
            sort_text: value
                .get("sortText")
                .and_then(Value::as_str)
                .map(str::to_string),
            filter_text: value
                .get("filterText")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    fn list_from(value: &Value) -> Vec<Self> {
        let items = if let Some(items) = value.get("items").and_then(Value::as_array) {
            items
        } else if let Some(items) = value.as_array() {
            items
        } else {
            return Vec::new();
        };
        items.iter().filter_map(CompletionItem::from_lsp).collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Hover {
    pub contents: String,
}

impl Hover {
    fn from_lsp(value: &Value) -> Option<Self> {
        let contents = value.get("contents")?;
        let text = flatten_markup(contents);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Hover {
            contents: trimmed.to_string(),
        })
    }
}

fn flatten_markup(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Object(map) => map
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default(),
        Value::Array(items) => items
            .iter()
            .map(flatten_markup)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LocationIpc {
    pub path: String,
    pub in_root: bool,
    pub line: u32,
    pub character: u32,
}

impl LocationIpc {
    fn from_lsp(root: &Path, value: &Value) -> Option<Self> {
        let uri = value
            .get("uri")
            .or_else(|| value.get("targetUri"))
            .and_then(Value::as_str)?;
        let range = value
            .get("range")
            .or_else(|| value.get("targetSelectionRange"))
            .or_else(|| value.get("targetRange"))?;
        let start = range.get("start")?;
        let line = start.get("line")?.as_u64()? as u32;
        let character = start.get("character")?.as_u64()? as u32;
        match uri::to_rel(root, uri) {
            Some(rel) => Some(LocationIpc {
                path: rel,
                in_root: true,
                line,
                character,
            }),
            None => {
                let abs = uri::to_path(uri)?;
                Some(LocationIpc {
                    path: abs.to_string_lossy().into_owned(),
                    in_root: false,
                    line,
                    character,
                })
            }
        }
    }

    fn list_from(root: &Path, value: &Value) -> Vec<Self> {
        match value {
            Value::Array(items) => items
                .iter()
                .filter_map(|v| LocationIpc::from_lsp(root, v))
                .collect(),
            Value::Object(_) => LocationIpc::from_lsp(root, value).into_iter().collect(),
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SignatureInfoIpc {
    pub label: String,
    pub documentation: Option<String>,
    pub parameters: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignatureHelp {
    pub signatures: Vec<SignatureInfoIpc>,
    pub active_signature: u32,
    pub active_parameter: u32,
}

impl SignatureHelp {
    fn from_lsp(value: &Value) -> Option<Self> {
        let signatures = value.get("signatures").and_then(Value::as_array)?;
        if signatures.is_empty() {
            return None;
        }
        let sigs = signatures
            .iter()
            .map(|sig| {
                let label = sig
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let documentation = sig
                    .get("documentation")
                    .map(flatten_markup)
                    .filter(|s| !s.is_empty());
                let parameters = sig
                    .get("parameters")
                    .and_then(Value::as_array)
                    .map(|params| {
                        params
                            .iter()
                            .filter_map(|p| p.get("label"))
                            .map(|l| match l {
                                Value::String(s) => s.clone(),
                                _ => String::new(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                SignatureInfoIpc {
                    label,
                    documentation,
                    parameters,
                }
            })
            .collect();
        Some(SignatureHelp {
            signatures: sigs,
            active_signature: value
                .get("activeSignature")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32,
            active_parameter: value
                .get("activeParameter")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallHintIpc {
    pub command: String,
    pub manager: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LspStatus {
    Unsupported,
    Experimental {
        server: String,
    },
    Absent {
        server: String,
        install: Option<InstallHintIpc>,
        alternatives: Vec<InstallHintIpc>,
    },
    Available {
        server: String,
    },
    Starting {
        server: String,
    },
    Ready {
        server: String,
        diagnostics: usize,
        truncated: bool,
    },
    Crashed {
        server: String,
        reason: Option<String>,
    },
}

type ServerKey = (SessionId, &'static str);

pub const MAX_COMPLETIONS: usize = 500;
pub const MAX_LOCATIONS: usize = 100;
pub const MAX_SIGNATURES: usize = 20;

enum ServerSlot {
    Reserving,
    Live(Arc<LspServer>),
}

#[derive(Default)]
pub struct LspManager {
    servers: Mutex<HashMap<ServerKey, ServerSlot>>,
}

pub type SharedLsp = Arc<LspManager>;

impl LspManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn get(&self, id: SessionId, entry: &ServerEntry) -> Option<Arc<LspServer>> {
        match self.servers.lock().get(&(id, entry.id)) {
            Some(ServerSlot::Live(server)) => Some(Arc::clone(server)),
            _ => None,
        }
    }

    fn is_reserving(&self, id: SessionId, entry: &ServerEntry) -> bool {
        matches!(
            self.servers.lock().get(&(id, entry.id)),
            Some(ServerSlot::Reserving)
        )
    }

    pub fn status(&self, id: SessionId, rel: &str, root: &Path) -> LspStatus {
        let Some((entry, _lang)) = entry_for_file(rel) else {
            return LspStatus::Unsupported;
        };
        let label = entry.label.to_string();
        if !entry.default_enabled {
            return LspStatus::Experimental { server: label };
        }
        if let Some(server) = self.get(id, entry) {
            return match server.state() {
                RunState::Starting => LspStatus::Starting { server: label },
                RunState::Ready => LspStatus::Ready {
                    server: label,
                    diagnostics: server.diagnostic_count(rel),
                    truncated: server.diagnostics_truncated(),
                },
                RunState::Crashed => LspStatus::Crashed {
                    server: label,
                    reason: server.last_error(),
                },
                RunState::Stopped => LspStatus::Available { server: label },
            };
        }
        if self.is_reserving(id, entry) {
            return LspStatus::Starting { server: label };
        }
        match self.discover(entry, root) {
            Some(_) => LspStatus::Available { server: label },
            None => {
                let chosen = registry::choose_install(entry, &self.path_dirs());
                LspStatus::Absent {
                    server: label,
                    install: chosen.chosen.map(install_ipc),
                    alternatives: chosen.alternatives.into_iter().map(install_ipc).collect(),
                }
            }
        }
    }

    fn path_dirs(&self) -> Vec<PathBuf> {
        std::env::split_paths(&crate::shell_path::agent_path()).collect()
    }

    fn discover(&self, entry: &ServerEntry, root: &Path) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        registry::discover(entry, &self.path_dirs(), &home, root)
    }

    pub fn open<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        id: SessionId,
        root: &Path,
        rel: &str,
        text: &str,
        spawn: bool,
    ) -> LspStatus {
        let Some((entry, lang)) = entry_for_file(rel) else {
            return LspStatus::Unsupported;
        };
        if let Some(server) = self.get(id, entry) {
            server.did_open(rel, lang, text);
            return self.status(id, rel, root);
        }
        if !spawn || !entry.default_enabled {
            return self.status(id, rel, root);
        }
        if let Ok(server) = self.spawn_server(app, id, root, entry) {
            server.did_open(rel, lang, text);
        }
        self.status(id, rel, root)
    }

    pub fn retry<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        id: SessionId,
        root: &Path,
        rel: &str,
    ) -> LspStatus {
        if let Some((entry, _)) = entry_for_file(rel) {
            if entry.default_enabled {
                if let Some(server) = self.get(id, entry) {
                    server.retry();
                } else {
                    let _ = self.spawn_server(app, id, root, entry);
                }
            }
        }
        self.status(id, rel, root)
    }

    fn spawn_server<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        id: SessionId,
        root: &Path,
        entry: &'static ServerEntry,
    ) -> Result<Arc<LspServer>, String> {
        let key = (id, entry.id);
        {
            let mut servers = self.servers.lock();
            match servers.get(&key) {
                Some(ServerSlot::Live(server)) => return Ok(Arc::clone(server)),
                Some(ServerSlot::Reserving) => return Err("server já subindo".into()),
                None => {
                    servers.insert(key, ServerSlot::Reserving);
                }
            }
        }
        match self.build_and_spawn(app, id, root, entry) {
            Ok(server) => {
                let mut servers = self.servers.lock();
                if matches!(servers.get(&key), Some(ServerSlot::Reserving)) {
                    servers.insert(key, ServerSlot::Live(Arc::clone(&server)));
                    Ok(server)
                } else {
                    drop(servers);
                    server.shutdown();
                    Err("o painel fechou durante a subida do server".into())
                }
            }
            Err(e) => {
                let mut servers = self.servers.lock();
                if matches!(servers.get(&key), Some(ServerSlot::Reserving)) {
                    servers.remove(&key);
                }
                Err(e)
            }
        }
    }

    fn build_and_spawn<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        id: SessionId,
        root: &Path,
        entry: &'static ServerEntry,
    ) -> Result<Arc<LspServer>, String> {
        let user_env: HashMap<String, String> = std::env::vars().collect();
        let home = user_env
            .get("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or("HOME indisponível")?;
        let path_dirs = self.path_dirs();
        let binary = registry::discover(entry, &path_dirs, &home, root)
            .ok_or("binário do language server não encontrado")?;
        let cache = cache_dir(id, entry);
        std::fs::create_dir_all(cache.join(".rt")).map_err(|e| format!("cache do LSP: {e}"))?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut exec_path_dirs = path_dirs.clone();
        if let Some(parent) = binary.parent() {
            exec_path_dirs.push(parent.to_path_buf());
        }
        let read_allow_extra = crate::user_config::load(&home)
            .map(|c| c.sandbox_read_allow)
            .unwrap_or_default();
        let extra_reads = registry::derived_read_grants(entry, &binary, &path_dirs);
        let ctx = LspSpecCtx {
            root,
            cache_dir: &cache,
            exe: &exe,
            env: &user_env,
            exec_path_dirs,
            read_allow_extra,
            extra_reads,
            data_dir: data_dir(&home),
        };
        let spec = sandbox::lsp_sandbox_spec(entry, &ctx)?;
        let env = base_env(&user_env, entry);
        let (program, pre) = registry::spawn_command(entry, &binary, &path_dirs);
        let pre_args: Vec<std::ffi::OsString> =
            pre.into_iter().map(|p| p.into_os_string()).collect();

        let app = app.clone();
        let event = diagnostics_event(id);
        let emit: Arc<DiagEmitter> = Arc::new(move |batch: DiagnosticsBatch| {
            let _ = app.emit(&event, batch);
        });

        LspServer::spawn(
            entry,
            root.to_path_buf(),
            program,
            pre_args,
            spec,
            env,
            emit,
        )
    }

    pub fn change(&self, id: SessionId, rel: &str, changes: Vec<ContentChange>) {
        if let Some((entry, _)) = entry_for_file(rel) {
            if let Some(server) = self.get(id, entry) {
                server.did_change(rel, changes);
            }
        }
    }

    pub fn close_doc(&self, id: SessionId, rel: &str) {
        if let Some((entry, _)) = entry_for_file(rel) {
            if let Some(server) = self.get(id, entry) {
                server.did_close(rel);
            }
        }
    }

    fn ready_server(&self, id: SessionId, rel: &str) -> Option<Arc<LspServer>> {
        let (entry, _) = entry_for_file(rel)?;
        let server = self.get(id, entry)?;
        matches!(server.state(), RunState::Ready).then_some(server)
    }

    pub fn completion(
        &self,
        id: SessionId,
        rel: &str,
        line: u32,
        character: u32,
    ) -> Vec<CompletionItem> {
        let Some(server) = self.ready_server(id, rel) else {
            return Vec::new();
        };
        let params = position_params(&server_uri(&server, rel), line, character);
        match server.request("textDocument/completion", params) {
            Ok(value) => {
                let mut items = CompletionItem::list_from(&value);
                items.truncate(MAX_COMPLETIONS);
                items
            }
            Err(_) => Vec::new(),
        }
    }

    pub fn hover(&self, id: SessionId, rel: &str, line: u32, character: u32) -> Option<Hover> {
        let server = self.ready_server(id, rel)?;
        let params = position_params(&server_uri(&server, rel), line, character);
        let value = server.request("textDocument/hover", params).ok()?;
        Hover::from_lsp(&value)
    }

    pub fn definition(
        &self,
        id: SessionId,
        rel: &str,
        line: u32,
        character: u32,
    ) -> Vec<LocationIpc> {
        let Some(server) = self.ready_server(id, rel) else {
            return Vec::new();
        };
        let params = position_params(&server_uri(&server, rel), line, character);
        match server.request("textDocument/definition", params) {
            Ok(value) => {
                let mut locs = LocationIpc::list_from(server.root(), &value);
                locs.truncate(MAX_LOCATIONS);
                locs
            }
            Err(_) => Vec::new(),
        }
    }

    pub fn signature(
        &self,
        id: SessionId,
        rel: &str,
        line: u32,
        character: u32,
    ) -> Option<SignatureHelp> {
        let server = self.ready_server(id, rel)?;
        let params = position_params(&server_uri(&server, rel), line, character);
        let value = server.request("textDocument/signatureHelp", params).ok()?;
        let mut help = SignatureHelp::from_lsp(&value)?;
        help.signatures.truncate(MAX_SIGNATURES);
        Some(help)
    }

    pub fn close_session(&self, id: SessionId) {
        let removed: Vec<(ServerKey, Arc<LspServer>)> = {
            let mut servers = self.servers.lock();
            let keys: Vec<ServerKey> = servers
                .keys()
                .filter(|(sid, _)| *sid == id)
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|k| match servers.remove(&k) {
                    Some(ServerSlot::Live(server)) => Some((k, server)),
                    _ => None,
                })
                .collect()
        };
        for ((_, server_id), server) in removed {
            server.shutdown();
            remove_cache(id, server_id);
        }
    }

    pub fn reap_idle(&self) {
        let stale: Vec<(ServerKey, Arc<LspServer>)> = {
            let mut servers = self.servers.lock();
            let keys: Vec<ServerKey> = servers
                .iter()
                .filter(|(_, slot)| match slot {
                    ServerSlot::Live(server) => {
                        !server.has_open_docs() && server.idle_since().elapsed() >= IDLE_KILL
                    }
                    ServerSlot::Reserving => false,
                })
                .map(|(k, _)| *k)
                .collect();
            keys.into_iter()
                .filter_map(|k| match servers.remove(&k) {
                    Some(ServerSlot::Live(server)) => Some((k, server)),
                    _ => None,
                })
                .collect()
        };
        for ((session_id, server_id), server) in stale {
            server.shutdown();
            remove_cache(session_id, server_id);
        }
    }
}

fn install_ipc(install: registry::Install) -> InstallHintIpc {
    InstallHintIpc {
        command: install.command.to_string(),
        manager: install.manager.label().to_string(),
    }
}

fn server_uri(server: &LspServer, rel: &str) -> String {
    uri::from_path(&server.root().join(rel))
}

fn position_params(uri: &str, line: u32, character: u32) -> Value {
    json!({
        "textDocument": {"uri": uri},
        "position": {"line": line, "character": character},
    })
}

fn cache_dir(id: SessionId, entry: &ServerEntry) -> PathBuf {
    std::env::temp_dir().join(format!("tyba-lsp-{id}-{}", entry.id))
}

fn remove_cache(id: SessionId, server_id: &str) {
    if let Some(entry) = registry::entry_by_id(server_id) {
        let _ = std::fs::remove_dir_all(cache_dir(id, entry));
    }
}

fn data_dir(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support/dev.tyba.app")
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"))
            .join("dev.tyba.app")
    }
}

pub fn spawn_reaper(manager: SharedLsp) {
    std::thread::spawn(move || loop {
        std::thread::sleep(REAP_INTERVAL);
        manager.reap_idle();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_parses_a_completion_list() {
        let value = json!({
            "isIncomplete": false,
            "items": [
                {"label": "println!", "kind": 3, "detail": "macro", "insertText": "println!"},
                {"label": "print!", "kind": 3},
            ],
        });
        let items = CompletionItem::list_from(&value);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "println!");
        assert_eq!(items[0].detail.as_deref(), Some("macro"));
        assert_eq!(items[0].kind, Some(3));
    }

    #[test]
    fn completion_parses_a_bare_item_array() {
        let value = json!([{"label": "foo"}, {"label": "bar"}]);
        assert_eq!(CompletionItem::list_from(&value).len(), 2);
    }

    #[test]
    fn completion_prefers_text_edit_new_text() {
        let value = json!({"items": [
            {"label": "x", "insertText": "ins", "textEdit": {"newText": "edited", "range": {}}}
        ]});
        let items = CompletionItem::list_from(&value);
        assert_eq!(items[0].insert_text.as_deref(), Some("edited"));
    }

    #[test]
    fn hover_flattens_markup_content() {
        let value = json!({"contents": {"kind": "markdown", "value": "`fn main()`"}});
        assert_eq!(Hover::from_lsp(&value).unwrap().contents, "`fn main()`");
    }

    #[test]
    fn hover_joins_a_marked_string_array() {
        let value = json!({"contents": ["one", {"language": "rust", "value": "two"}]});
        assert_eq!(Hover::from_lsp(&value).unwrap().contents, "one\n\ntwo");
    }

    #[test]
    fn empty_hover_is_none() {
        assert!(Hover::from_lsp(&json!({"contents": ""})).is_none());
    }

    #[test]
    fn definition_outside_root_is_flagged() {
        let root = Path::new("/private/wt/proj");
        let value = json!({
            "uri": "file:///usr/lib/rustlib/src/core.rs",
            "range": {"start": {"line": 10, "character": 4}, "end": {"line": 10, "character": 8}},
        });
        let locs = LocationIpc::list_from(root, &value);
        assert_eq!(locs.len(), 1);
        assert!(!locs[0].in_root);
        assert_eq!(locs[0].path, "/usr/lib/rustlib/src/core.rs");
        assert_eq!(locs[0].line, 10);
    }

    #[test]
    fn signature_help_parses_labels_and_active_indices() {
        let value = json!({
            "signatures": [{
                "label": "fn push(&mut self, value: T)",
                "parameters": [{"label": "value: T"}],
            }],
            "activeSignature": 0,
            "activeParameter": 0,
        });
        let help = SignatureHelp::from_lsp(&value).unwrap();
        assert_eq!(help.signatures.len(), 1);
        assert_eq!(help.signatures[0].parameters, vec!["value: T".to_string()]);
    }

    #[test]
    fn status_for_unknown_extension_is_unsupported() {
        let mgr = LspManager::new();
        let status = mgr.status(SessionId::new_v4(), "notes.xyz", Path::new("/tmp"));
        assert!(matches!(status, LspStatus::Unsupported));
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requer rust-analyzer instalado; sobe um server real dentro da jaula"]
    fn live_rust_analyzer_boots_inside_the_seatbelt_jail() {
        use crate::lsp::client::{base_env, LspServer};
        use crate::lsp::registry::{self, entry_by_id};
        use crate::lsp::sandbox::{lsp_sandbox_spec, LspSpecCtx};
        use std::time::{Duration, Instant};

        let user_env: HashMap<String, String> = std::env::vars().collect();
        let home = PathBuf::from(user_env.get("HOME").cloned().unwrap());
        let entry = entry_by_id("rust-analyzer").unwrap();
        let path_dirs: Vec<PathBuf> =
            std::env::split_paths(&crate::shell_path::agent_path()).collect();
        let root = std::env::temp_dir().join(format!("tyba-lsp-live-{}", SessionId::new_v4()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"live\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() { let x = 1; }\n").unwrap();
        let Some(binary) = registry::discover(entry, &path_dirs, &home, &root) else {
            std::fs::remove_dir_all(&root).ok();
            eprintln!("rust-analyzer ausente — validação viva pulada");
            return;
        };

        let cache =
            std::env::temp_dir().join(format!("tyba-lsp-live-cache-{}", SessionId::new_v4()));
        std::fs::create_dir_all(cache.join(".rt")).unwrap();
        let exe = std::env::current_exe().unwrap();
        let mut exec_path_dirs = path_dirs.clone();
        exec_path_dirs.push(binary.parent().unwrap().to_path_buf());
        let extra_reads = registry::derived_read_grants(entry, &binary, &path_dirs);
        let ctx = LspSpecCtx {
            root: &root,
            cache_dir: &cache,
            exe: &exe,
            env: &user_env,
            exec_path_dirs,
            read_allow_extra: vec![],
            extra_reads,
            data_dir: data_dir(&home),
        };
        let spec = lsp_sandbox_spec(entry, &ctx).unwrap();
        assert!(!spec.allow_network);
        let (program, pre) = registry::spawn_command(entry, &binary, &path_dirs);
        let pre_args: Vec<std::ffi::OsString> =
            pre.into_iter().map(|p| p.into_os_string()).collect();

        let server = LspServer::spawn(
            entry,
            root.clone(),
            program,
            pre_args,
            spec,
            base_env(&user_env, entry),
            Arc::new(|_batch: DiagnosticsBatch| {}),
        )
        .expect("server enjaulado subiu");

        let text = std::fs::read_to_string(root.join("src/main.rs")).unwrap();
        server.did_open("src/main.rs", "rust", &text);

        let deadline = Instant::now() + Duration::from_secs(40);
        while Instant::now() < deadline && !matches!(server.state(), RunState::Ready) {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert_eq!(
            server.state(),
            RunState::Ready,
            "o handshake initialize/initialized rodou dentro do sandbox-exec"
        );

        let hover = server.request(
            "textDocument/hover",
            json!({
                "textDocument": {"uri": uri::from_path(&root.join("src/main.rs"))},
                "position": {"line": 0, "character": 16},
            }),
        );
        assert!(hover.is_ok(), "o server enjaulado respondeu a um request");

        server.shutdown();
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&cache).ok();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requer typescript-language-server + typescript clássico num projeto real"]
    fn live_typescript_language_server_boots_inside_the_jail() {
        use crate::lsp::client::{base_env, LspServer};
        use crate::lsp::registry::{self, entry_by_id};
        use crate::lsp::sandbox::{lsp_sandbox_spec, LspSpecCtx};
        use std::time::{Duration, Instant};

        let user_env: HashMap<String, String> = std::env::vars().collect();
        let home = PathBuf::from(user_env.get("HOME").cloned().unwrap());
        let entry = entry_by_id("typescript-language-server").unwrap();
        let path_dirs: Vec<PathBuf> =
            std::env::split_paths(&crate::shell_path::agent_path()).collect();

        let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let root = std::fs::canonicalize(&project).unwrap();
        let rel = "src/lib/ipc.ts";
        let tsserver = root.join("node_modules/typescript/lib/tsserver.js");
        let file = root.join(rel);
        if !tsserver.is_file() || !file.is_file() {
            eprintln!("projeto sem typescript clássico ou sem {rel} — validação viva pulada");
            return;
        }
        let Some(binary) = registry::discover(entry, &path_dirs, &home, &root) else {
            eprintln!("typescript-language-server ausente — validação viva pulada");
            return;
        };

        let cache =
            std::env::temp_dir().join(format!("tyba-lsp-live-ts-cache-{}", SessionId::new_v4()));
        std::fs::create_dir_all(cache.join(".rt")).unwrap();
        let exe = std::env::current_exe().unwrap();
        let mut exec_path_dirs = path_dirs.clone();
        exec_path_dirs.push(binary.parent().unwrap().to_path_buf());
        let extra_reads = registry::derived_read_grants(entry, &binary, &path_dirs);
        let ctx = LspSpecCtx {
            root: &root,
            cache_dir: &cache,
            exe: &exe,
            env: &user_env,
            exec_path_dirs,
            read_allow_extra: vec![],
            extra_reads,
            data_dir: data_dir(&home),
        };
        let spec = lsp_sandbox_spec(entry, &ctx).unwrap();
        let (program, pre) = registry::spawn_command(entry, &binary, &path_dirs);
        let pre_args: Vec<std::ffi::OsString> =
            pre.into_iter().map(|p| p.into_os_string()).collect();

        let server = LspServer::spawn(
            entry,
            root.clone(),
            program,
            pre_args,
            spec,
            base_env(&user_env, entry),
            Arc::new(|_batch: DiagnosticsBatch| {}),
        )
        .expect("server node enjaulado subiu");

        let text = std::fs::read_to_string(&file).unwrap();
        server.did_open(rel, "typescript", &text);

        let deadline = Instant::now() + Duration::from_secs(40);
        while Instant::now() < deadline && !matches!(server.state(), RunState::Ready) {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert_eq!(
            server.state(),
            RunState::Ready,
            "node achou runtime, deps e o typescript do projeto dentro da jaula"
        );

        let completion = server.request(
            "textDocument/completion",
            json!({
                "textDocument": {"uri": uri::from_path(&file)},
                "position": {"line": 8, "character": 0},
            }),
        );
        assert!(completion.is_ok(), "o server node respondeu a completion");

        server.shutdown();
        std::fs::remove_dir_all(&cache).ok();
    }
}
