use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Runtime};

pub mod client;
pub mod document;
pub mod jsonrpc;
pub mod managed;
pub mod registry;
pub mod restart;
pub mod sandbox;
pub mod uri;

use client::{base_env, DiagEmitter, LspServer, StateEmitter};
use document::ContentChange;
use registry::{entry_for_file, ServerEntry};
use sandbox::LspSpecCtx;

use crate::session::SessionId;

pub const EVENT_DIAGNOSTICS_PREFIX: &str = "lsp://diagnostics/";
pub const EVENT_STATUS_PREFIX: &str = "lsp://status/";
const IDLE_KILL: Duration = Duration::from_secs(300);
const REAP_INTERVAL: Duration = Duration::from_secs(45);

pub fn diagnostics_event(id: SessionId) -> String {
    format!("{EVENT_DIAGNOSTICS_PREFIX}{id}")
}

pub fn status_event(id: SessionId) -> String {
    format!("{EVENT_STATUS_PREFIX}{id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Starting,
    Ready,
    /// Caiu e o reinício com backoff já está agendado.
    Restarting,
    /// Desistiu (ou nem subiu): fica parado até o "tentar de novo".
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
    /// Código do servidor (`2551`, `"E0425"`). Volta no pedido de correção:
    /// sem ele o tsserver não sabe qual fix oferecer.
    pub code: Option<Value>,
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
        let code = value
            .get("code")
            .filter(|c| c.is_string() || c.is_number())
            .cloned();
        Some(DiagnosticIpc {
            range,
            severity,
            message,
            source,
            code,
        })
    }
}

/// Diagnóstico como o webview o devolve ao pedir correção.
#[derive(Debug, Clone, Deserialize)]
pub struct DiagnosticIn {
    pub range: RangeIn,
    pub severity: u8,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<Value>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PosIn {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct RangeIn {
    pub start: PosIn,
    pub end: PosIn,
}

impl DiagnosticIn {
    fn to_lsp(&self) -> Value {
        let mut v = json!({
            "range": {
                "start": {"line": self.range.start.line, "character": self.range.start.character},
                "end": {"line": self.range.end.line, "character": self.range.end.character},
            },
            "severity": self.severity,
            "message": self.message,
        });
        if let Some(source) = &self.source {
            v["source"] = json!(source);
        }
        if let Some(code) = &self.code {
            v["code"] = code.clone();
        }
        v
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextEditIpc {
    pub range: RangeIpc,
    pub new_text: String,
}

/// Correção rápida aplicável no editor: só edições no próprio arquivo.
#[derive(Debug, Clone, Serialize)]
pub struct QuickFix {
    pub title: String,
    pub preferred: bool,
    pub edits: Vec<TextEditIpc>,
}

fn text_edits(value: &Value) -> Option<Vec<TextEditIpc>> {
    value
        .as_array()?
        .iter()
        .map(|e| {
            Some(TextEditIpc {
                range: RangeIpc::from_lsp(e.get("range")?)?,
                new_text: e.get("newText")?.as_str()?.to_string(),
            })
        })
        .collect()
}

fn same_file(a: &str, b: &str) -> bool {
    a == b || matches!((uri::to_path(a), uri::to_path(b)), (Some(x), Some(y)) if x == y)
}

impl QuickFix {
    /// Resposta de `textDocument/codeAction` → correções aplicáveis em `uri`.
    /// Descarta comandos (o TYBA não os executa), ações que mexem noutro
    /// arquivo ou criam/renomeiam arquivo — aplicar metade seria pior que nada.
    pub fn list_from(value: &Value, uri: &str) -> Vec<QuickFix> {
        let Some(items) = value.as_array() else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|item| {
                if item.get("command").is_some() {
                    return None;
                }
                if let Some(kind) = item.get("kind").and_then(Value::as_str) {
                    if !kind.starts_with("quickfix") {
                        return None;
                    }
                }
                let edit = item.get("edit")?;
                let mut edits = Vec::new();
                if let Some(changes) = edit.get("changes").and_then(Value::as_object) {
                    for (target, list) in changes {
                        if !same_file(target, uri) {
                            return None;
                        }
                        edits.extend(text_edits(list)?);
                    }
                }
                if let Some(doc_changes) = edit.get("documentChanges").and_then(Value::as_array) {
                    for change in doc_changes {
                        let target = change.get("textDocument")?.get("uri")?.as_str()?;
                        if change.get("kind").is_some() || !same_file(target, uri) {
                            return None;
                        }
                        edits.extend(text_edits(change.get("edits")?)?);
                    }
                }
                if edits.is_empty() {
                    return None;
                }
                Some(QuickFix {
                    title: item.get("title")?.as_str()?.to_string(),
                    preferred: item
                        .get("isPreferred")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    edits,
                })
            })
            .collect()
    }
}

/// Cadeia de símbolos (função, classe…) que contém a posição, de fora para
/// dentro — o fim do breadcrumb do editor.
pub fn symbol_path_at(value: &Value, line: u32, character: u32) -> Vec<String> {
    fn contains(range: &Value, line: u32, character: u32) -> bool {
        let Some(r) = RangeIpc::from_lsp(range) else {
            return false;
        };
        let after_start = (line, character) >= (r.start.line, r.start.character);
        let before_end = (line, character) <= (r.end.line, r.end.character);
        after_start && before_end
    }
    fn walk(nodes: &[Value], line: u32, character: u32, out: &mut Vec<String>) {
        for node in nodes {
            // DocumentSymbol tem `range`; SymbolInformation, `location.range`.
            let range = node
                .get("range")
                .or_else(|| node.get("location").and_then(|l| l.get("range")));
            if range.is_some_and(|r| contains(r, line, character)) {
                if let Some(name) = node.get("name").and_then(Value::as_str) {
                    out.push(name.to_string());
                }
                if let Some(children) = node.get("children").and_then(Value::as_array) {
                    walk(children, line, character, out);
                }
                return;
            }
        }
    }
    let mut out = Vec::new();
    if let Some(nodes) = value.as_array() {
        walk(nodes, line, character, &mut out);
    }
    out
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
pub struct InstallHints {
    pub install: Option<InstallHintIpc>,
    pub alternatives: Vec<InstallHintIpc>,
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
        /// O que falta, em pt-BR (ex.: TypeScript sem tsserver).
        missing: Option<String>,
        install: Option<InstallHintIpc>,
        alternatives: Vec<InstallHintIpc>,
    },
    Available {
        server: String,
    },
    Starting {
        server: String,
    },
    Restarting {
        server: String,
        reason: Option<String>,
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
    ManagedOffer {
        server: String,
        /// O que falta e por que o download resolve, em pt-BR.
        missing: Option<String>,
        card: managed::Card,
    },
    Installing {
        server: String,
        server_id: String,
        progress: managed::Progress,
    },
}

type ServerKey = (SessionId, &'static str);

pub const MAX_COMPLETIONS: usize = 500;
pub const MAX_LOCATIONS: usize = 100;
pub const MAX_QUICK_FIXES: usize = 20;
pub const MAX_SIGNATURES: usize = 20;

enum ServerSlot {
    Reserving,
    Live(Arc<LspServer>),
    /// A subida falhou antes de existir processo (jaula recusou, binário
    /// sumiu). Guardado para a UI mostrar o motivo em vez de "disponível"
    /// para sempre; o próximo open/retry tenta de novo.
    Failed(String),
}

#[derive(Default)]
pub struct LspManager {
    servers: Mutex<HashMap<ServerKey, ServerSlot>>,
    managed: Mutex<Option<managed::SharedManaged>>,
}

pub type SharedLsp = Arc<LspManager>;

impl LspManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attach_managed(&self, managed: managed::SharedManaged) {
        *self.managed.lock() = Some(managed);
    }

    fn managed(&self) -> Option<managed::SharedManaged> {
        self.managed.lock().clone()
    }

    fn get(&self, id: SessionId, entry: &ServerEntry) -> Option<Arc<LspServer>> {
        match self.servers.lock().get(&(id, entry.id)) {
            Some(ServerSlot::Live(server)) => Some(Arc::clone(server)),
            _ => None,
        }
    }

    fn failure(&self, id: SessionId, entry: &ServerEntry) -> Option<String> {
        match self.servers.lock().get(&(id, entry.id)) {
            Some(ServerSlot::Failed(reason)) => Some(reason.clone()),
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
                RunState::Restarting => LspStatus::Restarting {
                    server: label,
                    reason: server.last_error(),
                },
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
        let missing = match self.launch(entry, root) {
            registry::Launch::Missing(missing) => missing,
            _ => {
                if let Some(reason) = self.failure(id, entry) {
                    return LspStatus::Crashed {
                        server: label,
                        reason: Some(reason),
                    };
                }
                return LspStatus::Available { server: label };
            }
        };
        // Nada utilizável na máquina: a oferta do download verificado vem antes
        // de qualquer falha antiga — é a ação que resolve.
        if let Some(managed) = self.managed() {
            match managed.outcome(entry) {
                managed::Outcome::Installed => return LspStatus::Available { server: label },
                managed::Outcome::Installing(progress) => {
                    return LspStatus::Installing {
                        server: label,
                        server_id: entry.id.to_string(),
                        progress,
                    }
                }
                managed::Outcome::Offer(card) => {
                    return LspStatus::ManagedOffer {
                        server: label,
                        missing: Some(missing),
                        card,
                    }
                }
                managed::Outcome::NotManaged => {}
            }
        }
        let chosen = registry::choose_install(entry, &self.path_dirs());
        LspStatus::Absent {
            server: label,
            missing: Some(missing),
            install: chosen.chosen.map(install_ipc),
            alternatives: chosen.alternatives.into_iter().map(install_ipc).collect(),
        }
    }

    /// Com que binário este server subiria nesta raiz — a mesma decisão para o
    /// status e para o spawn, para a UI nunca prometer o que o spawn não cumpre.
    fn launch(&self, entry: &ServerEntry, root: &Path) -> registry::Launch {
        let managed_installed = self
            .managed()
            .map(|m| m.is_installed(entry))
            .unwrap_or(false);
        registry::choose_launch(entry, self.discover(entry, root), managed_installed, root)
    }

    fn path_dirs(&self) -> Vec<PathBuf> {
        std::env::split_paths(&crate::shell_path::agent_path()).collect()
    }

    pub fn install_hints(&self, server_id: &str) -> InstallHints {
        match registry::entry_by_id(server_id) {
            Some(entry) => {
                let chosen = registry::choose_install(entry, &self.path_dirs());
                InstallHints {
                    install: chosen.chosen.map(install_ipc),
                    alternatives: chosen.alternatives.into_iter().map(install_ipc).collect(),
                }
            }
            None => InstallHints {
                install: None,
                alternatives: Vec::new(),
            },
        }
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
        if !spawn
            || !entry.default_enabled
            || matches!(self.launch(entry, root), registry::Launch::Missing(_))
        {
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
                match self.get(id, entry) {
                    // Desistiu: sobe um server novo, com a escolha de binário
                    // refeita — o gerenciado pode ter acabado de chegar, e o
                    // processo velho seguiria com o binário que não servia.
                    Some(server) if server.state() == RunState::Crashed => {
                        let docs = server.open_docs_snapshot();
                        self.drop_server(id, entry);
                        if let Ok(fresh) = self.spawn_server(app, id, root, entry) {
                            for (path, lang, text) in docs {
                                fresh.did_open(&path, &lang, &text);
                            }
                        }
                    }
                    Some(server) => server.retry(),
                    None => {
                        let _ = self.spawn_server(app, id, root, entry);
                    }
                }
            }
        }
        self.status(id, rel, root)
    }

    fn drop_server(&self, id: SessionId, entry: &ServerEntry) {
        let removed = self.servers.lock().remove(&(id, entry.id));
        if let Some(ServerSlot::Live(server)) = removed {
            server.shutdown();
        }
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
                Some(ServerSlot::Failed(_)) | None => {
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
                    servers.insert(key, ServerSlot::Failed(e.clone()));
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
        let (program, pre, exec_extra, extra_reads, data_dir_reads, is_managed) =
            match self.launch(entry, root) {
                registry::Launch::Missing(missing) => return Err(missing),
                registry::Launch::Discovered(binary) => {
                    let exec_extra = binary
                        .parent()
                        .map(|p| vec![p.to_path_buf()])
                        .unwrap_or_default();
                    let extra_reads = registry::derived_read_grants(entry, &binary, &path_dirs);
                    let (program, pre) = registry::spawn_command(entry, &binary, &path_dirs);
                    (program, pre, exec_extra, extra_reads, Vec::new(), false)
                }
                registry::Launch::Managed => {
                    let incomplete = "a instalação gerenciada está incompleta — baixe de novo";
                    let plan = self
                        .managed()
                        .and_then(|m| m.plan(entry))
                        .ok_or(incomplete)?;
                    (
                        plan.program,
                        plan.pre_args,
                        plan.exec_dirs,
                        Vec::new(),
                        plan.reads,
                        true,
                    )
                }
            };
        let cache = cache_dir(id, entry);
        std::fs::create_dir_all(cache.join(".rt")).map_err(|e| format!("cache do LSP: {e}"))?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut exec_path_dirs = path_dirs.clone();
        exec_path_dirs.extend(exec_extra);
        let read_allow_extra = crate::user_config::load(&home)
            .map(|c| c.sandbox_read_allow)
            .unwrap_or_default();
        let ctx = LspSpecCtx {
            root,
            cache_dir: &cache,
            exe: &exe,
            env: &user_env,
            exec_path_dirs,
            read_allow_extra,
            extra_reads,
            data_dir: data_dir(&home),
            data_dir_reads,
        };
        let spec = sandbox::lsp_sandbox_spec(entry, &ctx)?;
        let env = base_env(&user_env, entry);
        let pre_args: Vec<std::ffi::OsString> =
            pre.into_iter().map(|p| p.into_os_string()).collect();
        let init_options = embedded_init_options(entry, &cache.join(".rt"));

        let diag_app = app.clone();
        let event = diagnostics_event(id);
        let emit: Arc<DiagEmitter> = Arc::new(move |batch: DiagnosticsBatch| {
            let _ = diag_app.emit(&event, batch);
        });
        let state_app = app.clone();
        let state_event = status_event(id);
        let on_state: Arc<StateEmitter> = Arc::new(move || {
            let _ = state_app.emit(&state_event, ());
        });

        let server = LspServer::spawn(
            entry,
            root.to_path_buf(),
            program,
            pre_args,
            spec,
            env,
            init_options,
            emit,
            on_state,
        )?;

        if is_managed {
            if let Some(managed) = self.managed() {
                let watch = Arc::clone(&server);
                let sid = entry.id;
                std::thread::spawn(move || {
                    let deadline = std::time::Instant::now() + Duration::from_secs(60);
                    loop {
                        match watch.state() {
                            RunState::Ready => {
                                managed.gc_after_green(sid);
                                break;
                            }
                            RunState::Crashed | RunState::Stopped => break,
                            RunState::Starting | RunState::Restarting => {}
                        }
                        if std::time::Instant::now() > deadline {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                });
            }
        }
        Ok(server)
    }

    pub fn change(&self, id: SessionId, rel: &str, changes: Vec<ContentChange>) {
        if let Some((entry, _)) = entry_for_file(rel) {
            if let Some(server) = self.get(id, entry) {
                server.did_change(rel, changes);
            }
        }
    }

    pub fn did_save(&self, id: SessionId, rel: &str) {
        if let Some((entry, _)) = entry_for_file(rel) {
            if let Some(server) = self.get(id, entry) {
                server.did_save(rel);
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

    pub fn code_actions(
        &self,
        id: SessionId,
        rel: &str,
        diagnostics: &[DiagnosticIn],
    ) -> Vec<QuickFix> {
        let Some(server) = self.ready_server(id, rel) else {
            return Vec::new();
        };
        let Some(first) = diagnostics.first() else {
            return Vec::new();
        };
        let uri = server_uri(&server, rel);
        let params = json!({
            "textDocument": {"uri": uri},
            "range": {
                "start": {"line": first.range.start.line, "character": first.range.start.character},
                "end": {"line": first.range.end.line, "character": first.range.end.character},
            },
            "context": {
                "diagnostics": diagnostics.iter().map(DiagnosticIn::to_lsp).collect::<Vec<_>>(),
                "only": ["quickfix"],
            },
        });
        match server.request("textDocument/codeAction", params) {
            Ok(value) => {
                let mut fixes = QuickFix::list_from(&value, &uri);
                fixes.truncate(MAX_QUICK_FIXES);
                fixes
            }
            Err(_) => Vec::new(),
        }
    }

    pub fn symbol_path(&self, id: SessionId, rel: &str, line: u32, character: u32) -> Vec<String> {
        let Some(server) = self.ready_server(id, rel) else {
            return Vec::new();
        };
        let params = json!({"textDocument": {"uri": server_uri(&server, rel)}});
        match server.request("textDocument/documentSymbol", params) {
            Ok(value) => symbol_path_at(&value, line, character),
            Err(_) => Vec::new(),
        }
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
                    ServerSlot::Reserving | ServerSlot::Failed(_) => false,
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

fn embedded_init_options(entry: &ServerEntry, schema_dir: &Path) -> Option<String> {
    let schemas = registry::embedded_schemas(entry);
    if schemas.is_empty() {
        return None;
    }
    let mut associations = serde_json::Map::new();
    for schema in schemas {
        let path = schema_dir.join(schema.filename);
        if std::fs::write(&path, schema.content).is_err() {
            continue;
        }
        associations.insert(uri::from_path(&path), json!(schema.globs));
    }
    if associations.is_empty() {
        return None;
    }
    let init = json!({
        "yaml": {
            "validate": true,
            "hover": true,
            "completion": true,
            "schemaStore": {"enable": false},
            "schemas": associations,
        }
    });
    Some(init.to_string())
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

pub fn data_dir(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support")
            .join(crate::profile::IDENTIFIER)
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"))
            .join(crate::profile::IDENTIFIER)
    }
}

pub fn resolve_data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())?;
    Some(data_dir(&home))
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

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    #[ignore = "requer rust-analyzer instalado; sobe um server real dentro da jaula"]
    fn live_rust_analyzer_boots_inside_the_jail() {
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
            data_dir_reads: vec![],
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
            None,
            Arc::new(|_batch: DiagnosticsBatch| {}),
            Arc::new(|| {}),
        )
        .expect("server enjaulado subiu");

        let text = std::fs::read_to_string(root.join("src/main.rs")).unwrap();
        server.did_open("src/main.rs", "rust", &text);

        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline && !matches!(server.state(), RunState::Ready) {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert_eq!(
            server.state(),
            RunState::Ready,
            "o handshake initialize/initialized rodou dentro da jaula"
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

    #[cfg(any(target_os = "macos", target_os = "linux"))]
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
            data_dir_reads: vec![],
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
            None,
            Arc::new(|_batch: DiagnosticsBatch| {}),
            Arc::new(|| {}),
        )
        .expect("server node enjaulado subiu");

        let text = std::fs::read_to_string(&file).unwrap();
        server.did_open(rel, "typescript", &text);

        let deadline = Instant::now() + Duration::from_secs(120);
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

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    #[ignore = "reproduz completion depois do save com ts-ls real"]
    fn live_ts_completion_survives_save() {
        use crate::lsp::client::{base_env, LspServer};
        use crate::lsp::document::ContentChange;
        use crate::lsp::registry::{self, entry_by_id};
        use crate::lsp::sandbox::{lsp_sandbox_spec, LspSpecCtx};
        use std::time::{Duration, Instant};

        let user_env: HashMap<String, String> = std::env::vars().collect();
        let home = PathBuf::from(user_env.get("HOME").cloned().unwrap());
        let entry = entry_by_id("typescript-language-server").unwrap();
        let path_dirs: Vec<PathBuf> =
            std::env::split_paths(&crate::shell_path::agent_path()).collect();
        let root =
            std::fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap())
                .unwrap();
        if !root
            .join("node_modules/typescript/lib/tsserver.js")
            .is_file()
        {
            eprintln!("projeto sem typescript clássico — pulado");
            return;
        }
        let Some(binary) = registry::discover(entry, &path_dirs, &home, &root) else {
            eprintln!("ts-ls ausente — pulado");
            return;
        };
        let rel = "tyba_lsp_probe.ts";
        let probe = root.join(rel);
        let uri = uri::from_path(&probe);
        std::fs::write(&probe, "const s = \"hello\";\n").unwrap();

        let cache =
            std::env::temp_dir().join(format!("tyba-lsp-save-cache-{}", SessionId::new_v4()));
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
            data_dir_reads: vec![],
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
            None,
            Arc::new(|_b: DiagnosticsBatch| {}),
            Arc::new(|| {}),
        )
        .unwrap();

        use crate::lsp::document::{Position, Range};
        let type_char = |line: u32, col: u32, c: &str| {
            server.did_change(
                rel,
                vec![ContentChange {
                    range: Some(Range {
                        start: Position {
                            line,
                            character: col,
                        },
                        end: Position {
                            line,
                            character: col,
                        },
                    }),
                    text: c.to_string(),
                }],
            );
        };
        let complete = |line: u32, ch: u32| -> usize {
            server
                .request(
                    "textDocument/completion",
                    json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": ch}}),
                )
                .map(|v| CompletionItem::list_from(&v).len())
                .unwrap_or(0)
        };

        let seed = "const soma = (a: number) => a;\n\n";
        server.did_open(rel, "typescript", seed);
        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline && !matches!(server.state(), RunState::Ready) {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert_eq!(server.state(), RunState::Ready);
        // Mais que os 3s do vigia de processo-pai do vscode-languageserver: com
        // o processId do host no initialize o server morria aqui dentro.
        std::thread::sleep(Duration::from_secs(7));
        assert_eq!(
            server.state(),
            RunState::Ready,
            "o server enjaulado segue de pé depois do vigia de processo-pai"
        );

        for (i, c) in "som".chars().enumerate() {
            type_char(1, i as u32, &c.to_string());
        }
        std::thread::sleep(Duration::from_millis(700));
        let before = complete(1, 3);

        std::fs::write(root.join(".probe.tmp"), format!("{seed}som")).unwrap();
        std::fs::rename(root.join(".probe.tmp"), &probe).unwrap();
        server.did_save(rel);
        std::thread::sleep(Duration::from_millis(1500));

        type_char(1, 3, "a");
        type_char(1, 4, "(");
        std::thread::sleep(Duration::from_millis(700));
        let after = complete(1, 4);

        server.shutdown();
        std::fs::remove_file(&probe).ok();
        std::fs::remove_dir_all(&cache).ok();
        assert!(before > 0, "completion funcionava antes do save");
        assert!(after > 0, "completion continua viva depois do save+edição");
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    #[ignore = "requer yaml-language-server; valida descoberta + schemas embarcados offline"]
    fn live_yaml_docker_compose_completion_offline() {
        use crate::lsp::client::{base_env, LspServer};
        use crate::lsp::registry::{self, entry_by_id};
        use crate::lsp::sandbox::{lsp_sandbox_spec, LspSpecCtx};
        use std::time::{Duration, Instant};

        let user_env: HashMap<String, String> = std::env::vars().collect();
        let home = PathBuf::from(user_env.get("HOME").cloned().unwrap());
        let entry = entry_by_id("yaml-language-server").unwrap();
        let path_dirs: Vec<PathBuf> =
            std::env::split_paths(&crate::shell_path::agent_path()).collect();
        let root = std::env::temp_dir().join(format!("tyba-lsp-yaml-{}", SessionId::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let rel = "docker-compose.yml";
        let file = root.join(rel);
        std::fs::write(&file, "\n").unwrap();

        let Some(binary) = registry::discover(entry, &path_dirs, &home, &root) else {
            std::fs::remove_dir_all(&root).ok();
            eprintln!("yaml-language-server ausente (nem sob version-manager) — pulado");
            return;
        };

        let cache =
            std::env::temp_dir().join(format!("tyba-lsp-yaml-cache-{}", SessionId::new_v4()));
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
            data_dir_reads: vec![],
        };
        let spec = lsp_sandbox_spec(entry, &ctx).unwrap();
        let (program, pre) = registry::spawn_command(entry, &binary, &path_dirs);
        let pre_args: Vec<std::ffi::OsString> =
            pre.into_iter().map(|p| p.into_os_string()).collect();
        let init_options = embedded_init_options(entry, &cache.join(".rt"));
        assert!(init_options.is_some(), "yaml-ls recebe schemas embarcados");

        let server = LspServer::spawn(
            entry,
            root.clone(),
            program,
            pre_args,
            spec,
            base_env(&user_env, entry),
            init_options,
            Arc::new(|_b: DiagnosticsBatch| {}),
            Arc::new(|| {}),
        )
        .unwrap();

        server.did_open(rel, "yaml", "\n");
        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline && !matches!(server.state(), RunState::Ready) {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert_eq!(server.state(), RunState::Ready, "yaml-ls subiu enjaulado");
        std::thread::sleep(Duration::from_millis(800));

        let value = server
            .request(
                "textDocument/completion",
                json!({"textDocument": {"uri": uri::from_path(&file)}, "position": {"line": 0, "character": 0}}),
            )
            .unwrap();
        let labels: Vec<String> = CompletionItem::list_from(&value)
            .into_iter()
            .map(|c| c.label)
            .collect();

        server.shutdown();
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&cache).ok();
        assert!(
            labels.iter().any(|l| l == "services"),
            "schema compose embarcado (sem rede) oferece chaves como `services`: {labels:?}"
        );
    }

    #[test]
    fn quick_fix_keeps_only_same_file_edits_and_drops_commands() {
        let uri = "file:///repo/src/server.js";
        let value = json!([
            {
                "title": "Change spelling to 'uptime'",
                "kind": "quickfix",
                "isPreferred": true,
                "edit": {"changes": {uri: [
                    {"range": {"start": {"line": 7, "character": 37}, "end": {"line": 7, "character": 42}}, "newText": "uptime"}
                ]}}
            },
            {"title": "Run linter", "command": "eslint.fix"},
            {
                "title": "Fix all in project",
                "kind": "quickfix",
                "edit": {"changes": {"file:///repo/src/other.js": [
                    {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "x"}
                ]}}
            },
            {
                "title": "Create file",
                "kind": "quickfix",
                "edit": {"documentChanges": [{"kind": "create", "uri": "file:///repo/new.js"}]}
            },
            {
                "title": "Organize imports",
                "kind": "source.organizeImports",
                "edit": {"changes": {uri: [
                    {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": ""}
                ]}}
            },
            {
                "title": "Add missing semicolon",
                "edit": {"documentChanges": [{"textDocument": {"uri": uri, "version": 3}, "edits": [
                    {"range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 4}}, "newText": ";"}
                ]}]}
            }
        ]);
        let fixes = QuickFix::list_from(&value, uri);
        let titles: Vec<&str> = fixes.iter().map(|f| f.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["Change spelling to 'uptime'", "Add missing semicolon"]
        );
        assert!(fixes[0].preferred);
        assert_eq!(fixes[0].edits[0].new_text, "uptime");
        assert_eq!(fixes[0].edits[0].range.start.character, 37);
    }

    #[test]
    fn diagnostic_code_round_trips_to_code_action_context() {
        let d = DiagnosticIpc::from_lsp(&json!({
            "range": {"start": {"line": 7, "character": 37}, "end": {"line": 7, "character": 42}},
            "severity": 1,
            "message": "Property 'uptim' does not exist",
            "source": "ts",
            "code": 2551
        }))
        .unwrap();
        assert_eq!(d.code, Some(json!(2551)));
        let back: DiagnosticIn = serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
        let lsp = back.to_lsp();
        assert_eq!(lsp["code"], json!(2551));
        assert_eq!(lsp["range"]["end"]["character"], json!(42));
        assert_eq!(lsp["source"], json!("ts"));
    }

    #[test]
    fn symbol_path_walks_hierarchy_to_innermost() {
        let range = |a: u32, b: u32| json!({"start": {"line": a, "character": 0}, "end": {"line": b, "character": 0}});
        let value = json!([
            {"name": "imports", "range": range(0, 2)},
            {"name": "App", "range": range(4, 20), "children": [
                {"name": "health", "range": range(6, 9)},
                {"name": "listen", "range": range(11, 12)}
            ]}
        ]);
        assert_eq!(symbol_path_at(&value, 7, 3), vec!["App", "health"]);
        assert_eq!(symbol_path_at(&value, 10, 0), vec!["App"]);
        assert!(symbol_path_at(&value, 3, 0).is_empty());
        let flat =
            json!([{"name": "main", "location": {"uri": "file:///x", "range": range(1, 5)}}]);
        assert_eq!(symbol_path_at(&flat, 2, 0), vec!["main"]);
    }
}
