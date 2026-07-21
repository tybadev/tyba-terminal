use std::collections::{HashMap, HashSet};
use std::io::BufReader;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use portable_pty::CommandBuilder;
use serde_json::{json, Value};

use super::document::{apply_changes, ContentChange};
use super::jsonrpc::{read_message, write_message};
use super::registry::ServerEntry;
use super::{DiagnosticIpc, DiagnosticsBatch, FileDiagnostics, RunState};
use crate::sandbox::SandboxSpec;

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_millis(2500);
const DIAG_DEBOUNCE: Duration = Duration::from_millis(120);
const MAX_RESTARTS: u32 = 4;
const MAX_RESTARTS_PER_HOUR: usize = 8;
const RESTART_WINDOW: Duration = Duration::from_secs(3600);
pub const MAX_DIAG_FILES: usize = 500;
pub const MAX_DIAG_PER_FILE: usize = 1000;

pub type DiagEmitter = dyn Fn(DiagnosticsBatch) + Send + Sync;

#[derive(Clone)]
struct OpenDoc {
    uri: String,
    language_id: String,
    version: i64,
    text: String,
}

struct BootParams {
    binary: std::path::PathBuf,
    spec: SandboxSpec,
    env: Vec<(String, String)>,
}

pub struct LspServer {
    pub entry: &'static ServerEntry,
    root: std::path::PathBuf,
    boot: BootParams,
    emit: Arc<DiagEmitter>,
    next_id: AtomicI64,
    pending: Mutex<HashMap<i64, Sender<Result<Value, String>>>>,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    leader_pid: Mutex<Option<u32>>,
    open_docs: Mutex<HashMap<String, OpenDoc>>,
    diagnostics: Mutex<HashMap<String, Vec<DiagnosticIpc>>>,
    dirty_diags: Mutex<HashSet<String>>,
    files_truncated: AtomicBool,
    state: Mutex<RunState>,
    last_error: Mutex<Option<String>>,
    last_activity: Mutex<Instant>,
    shutting_down: AtomicBool,
    restart_attempts: AtomicU32,
    restart_window: Mutex<Vec<Instant>>,
    diag_tx: Mutex<Option<Sender<()>>>,
}

impl Drop for LspServer {
    fn drop(&mut self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        *self.diag_tx.lock() = None;
        self.kill_process();
    }
}

impl LspServer {
    pub fn spawn(
        entry: &'static ServerEntry,
        root: std::path::PathBuf,
        binary: std::path::PathBuf,
        spec: SandboxSpec,
        env: Vec<(String, String)>,
        emit: Arc<DiagEmitter>,
    ) -> Result<Arc<Self>, String> {
        let server = Arc::new(LspServer {
            entry,
            root,
            boot: BootParams { binary, spec, env },
            emit,
            next_id: AtomicI64::new(1),
            pending: Mutex::new(HashMap::new()),
            stdin: Mutex::new(None),
            child: Mutex::new(None),
            leader_pid: Mutex::new(None),
            open_docs: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(HashMap::new()),
            dirty_diags: Mutex::new(HashSet::new()),
            files_truncated: AtomicBool::new(false),
            state: Mutex::new(RunState::Starting),
            last_error: Mutex::new(None),
            last_activity: Mutex::new(Instant::now()),
            shutting_down: AtomicBool::new(false),
            restart_attempts: AtomicU32::new(0),
            restart_window: Mutex::new(Vec::new()),
            diag_tx: Mutex::new(None),
        });
        boot(&server)?;
        Ok(server)
    }

    pub fn state(&self) -> RunState {
        *self.state.lock()
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }

    pub fn diagnostics_truncated(&self) -> bool {
        self.files_truncated.load(Ordering::Relaxed)
    }

    pub fn retry(self: &Arc<Self>) {
        if matches!(self.state(), RunState::Starting | RunState::Ready) {
            return;
        }
        self.shutting_down.store(false, Ordering::SeqCst);
        self.restart_attempts.store(0, Ordering::SeqCst);
        self.restart_window.lock().clear();
        *self.last_error.lock() = None;
        *self.state.lock() = RunState::Starting;
        if boot(self).is_err() {
            *self.state.lock() = RunState::Crashed;
        }
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    pub fn diagnostic_count(&self, rel: &str) -> usize {
        self.diagnostics
            .lock()
            .get(rel)
            .map(|d| d.len())
            .unwrap_or(0)
    }

    fn touch(&self) {
        *self.last_activity.lock() = Instant::now();
    }

    pub fn idle_since(&self) -> Instant {
        *self.last_activity.lock()
    }

    pub fn has_open_docs(&self) -> bool {
        !self.open_docs.lock().is_empty()
    }

    fn file_uri(&self, rel: &str) -> String {
        let abs = self.root.join(rel);
        super::uri::from_path(&abs)
    }

    pub fn did_open(&self, rel: &str, language_id: &str, text: &str) {
        self.touch();
        let uri = self.file_uri(rel);
        {
            let mut docs = self.open_docs.lock();
            let entry = docs.entry(rel.to_string()).or_insert_with(|| OpenDoc {
                uri: uri.clone(),
                language_id: language_id.to_string(),
                version: 0,
                text: String::new(),
            });
            entry.version += 1;
            entry.text = text.to_string();
            entry.language_id = language_id.to_string();
        }
        if !matches!(self.state(), RunState::Ready) {
            return;
        }
        let version = self
            .open_docs
            .lock()
            .get(rel)
            .map(|d| d.version)
            .unwrap_or(1);
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": version,
                    "text": text,
                }
            }),
        );
    }

    pub fn did_change(&self, rel: &str, changes: Vec<ContentChange>) {
        self.touch();
        let (uri, version, lsp_changes) = {
            let mut docs = self.open_docs.lock();
            let Some(doc) = docs.get_mut(rel) else {
                return;
            };
            doc.version += 1;
            doc.text = apply_changes(&doc.text, &changes);
            let lsp_changes: Vec<Value> = changes
                .iter()
                .map(|c| match c.range {
                    Some(r) => json!({
                        "range": {
                            "start": {"line": r.start.line, "character": r.start.character},
                            "end": {"line": r.end.line, "character": r.end.character},
                        },
                        "text": c.text,
                    }),
                    None => json!({ "text": c.text }),
                })
                .collect();
            (doc.uri.clone(), doc.version, lsp_changes)
        };
        if !matches!(self.state(), RunState::Ready) {
            return;
        }
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": {"uri": uri, "version": version},
                "contentChanges": lsp_changes,
            }),
        );
    }

    pub fn did_close(&self, rel: &str) {
        let removed = self.open_docs.lock().remove(rel);
        let had_diags = self.diagnostics.lock().remove(rel).is_some();
        if had_diags {
            self.dirty_diags.lock().insert(rel.to_string());
            self.queue_diag_emit();
        }
        if let Some(doc) = removed {
            if matches!(self.state(), RunState::Ready) {
                self.notify(
                    "textDocument/didClose",
                    json!({ "textDocument": {"uri": doc.uri} }),
                );
            }
        }
    }

    pub fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.request_with_timeout(method, params, REQUEST_TIMEOUT)
    }

    fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.touch();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.pending.lock().insert(id, tx);
        let payload = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if let Err(e) = self.write_value(&payload) {
            self.pending.lock().remove(&id);
            return Err(e);
        }
        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(_) => {
                self.pending.lock().remove(&id);
                Err("timeout aguardando o language server".into())
            }
        }
    }

    fn notify(&self, method: &str, params: Value) {
        let payload = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let _ = self.write_value(&payload);
    }

    fn respond(&self, id: Value, result: Value) {
        let payload = json!({"jsonrpc": "2.0", "id": id, "result": result});
        let _ = self.write_value(&payload);
    }

    fn write_value(&self, value: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        let mut guard = self.stdin.lock();
        let stdin = guard
            .as_mut()
            .ok_or("language server sem stdin (não iniciado)")?;
        write_message(stdin, &body).map_err(|e| format!("falha ao escrever no server: {e}"))
    }

    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        *self.state.lock() = RunState::Stopped;
        let _ = self.request_with_timeout("shutdown", Value::Null, Duration::from_millis(800));
        self.notify("exit", Value::Null);
        *self.diag_tx.lock() = None;
        self.kill_process();
    }

    fn kill_process(&self) {
        if let Some(pid) = *self.leader_pid.lock() {
            kill_group(pid);
        }
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        *self.stdin.lock() = None;
    }

    fn queue_diag_emit(&self) {
        if let Some(tx) = self.diag_tx.lock().as_ref() {
            let _ = tx.send(());
        }
    }

    fn emit_diagnostics(&self) {
        let dirty: Vec<String> = self.dirty_diags.lock().drain().collect();
        if dirty.is_empty() {
            return;
        }
        let diags = self.diagnostics.lock();
        let files: Vec<FileDiagnostics> = dirty
            .into_iter()
            .map(|path| {
                let list = diags.get(&path).cloned().unwrap_or_default();
                FileDiagnostics {
                    path,
                    diagnostics: list,
                }
            })
            .collect();
        (self.emit)(DiagnosticsBatch {
            files,
            files_truncated: self.files_truncated.load(Ordering::Relaxed),
        });
    }
}

fn absolute_only(path: &str) -> String {
    std::env::split_paths(path)
        .filter(|p| p.is_absolute())
        .filter_map(|p| p.to_str().map(str::to_string))
        .collect::<Vec<_>>()
        .join(":")
}

pub fn base_env(user_env: &HashMap<String, String>, entry: &ServerEntry) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = Vec::new();
    for key in ["HOME", "USER", "LANG", "TMPDIR", "SHELL"] {
        if let Some(v) = user_env.get(key) {
            env.push((key.to_string(), v.clone()));
        }
    }
    for var in entry.forwarded_env() {
        if let Some(v) = user_env.get(var) {
            env.push((var.to_string(), v.clone()));
        }
    }
    env.push((
        "PATH".to_string(),
        absolute_only(&crate::shell_path::agent_path()),
    ));
    env.push(("NO_COLOR".to_string(), "1".to_string()));
    env.push(("TYBA_LSP".to_string(), "1".to_string()));
    env
}

fn to_std_command(wrapped: &CommandBuilder, env: &[(String, String)]) -> Result<Command, String> {
    let argv = wrapped.get_argv();
    let mut iter = argv.iter();
    let program = iter.next().ok_or("comando de LSP vazio")?;
    let mut cmd = Command::new(program);
    for arg in iter {
        cmd.arg(arg);
    }
    if let Some(cwd) = wrapped.get_cwd() {
        cmd.current_dir(cwd);
    }
    cmd.env_clear();
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(marker) = wrapped.get_env("TYBA_SANDBOX") {
        cmd.env("TYBA_SANDBOX", marker);
    }
    Ok(cmd)
}

fn boot(server: &Arc<LspServer>) -> Result<(), String> {
    let mut argv: Vec<std::ffi::OsString> = vec![server.boot.binary.clone().into_os_string()];
    for arg in server.entry.args {
        argv.push(arg.into());
    }
    let mut builder = CommandBuilder::from_argv(argv);
    builder.cwd(&server.root);

    let sandbox = crate::sandbox::platform_sandbox()?;
    if sandbox.jailed_spawner(&server.boot.spec)?.is_some() {
        return Err(
            "a jaula do LSP por reescrita de argv não existe nesta plataforma — recusado (fail-closed)"
                .into(),
        );
    }
    let wrapped = sandbox.wrap(builder, &server.boot.spec)?;
    let mut cmd = to_std_command(&wrapped, &server.boot.env)?;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        unsafe {
            cmd.pre_exec(|| {
                libc::setpriority(libc::PRIO_PROCESS, 0, 10);
                Ok(())
            });
        }
    }
    crate::repo::no_console_window(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("falha ao subir o language server: {e}"))?;
    let pid = child.id();
    let stdin = child.stdin.take().ok_or("server sem stdin")?;
    let stdout = child.stdout.take().ok_or("server sem stdout")?;
    let stderr = child.stderr.take();

    *server.leader_pid.lock() = Some(pid);
    *server.stdin.lock() = Some(stdin);
    *server.child.lock() = Some(child);
    *server.state.lock() = RunState::Starting;

    let (diag_tx, diag_rx) = channel();
    *server.diag_tx.lock() = Some(diag_tx);
    spawn_diag_thread(Arc::clone(server), diag_rx);

    if let Some(stderr) = stderr {
        let debug = std::env::var_os("TYBA_LSP_STDERR").is_some();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut sink = [0u8; 4096];
            let mut stderr = stderr;
            while let Ok(n) = stderr.read(&mut sink) {
                if n == 0 {
                    break;
                }
                if debug {
                    eprint!("{}", String::from_utf8_lossy(&sink[..n]));
                }
            }
        });
    }

    let reader_server = Arc::clone(server);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(body)) = read_message(&mut reader) {
            dispatch(&reader_server, &body);
        }
        on_reader_exit(&reader_server);
    });

    let init_server = Arc::clone(server);
    std::thread::spawn(move || handshake(&init_server));

    Ok(())
}

fn handshake(server: &Arc<LspServer>) {
    let root_uri = super::uri::from_path(&server.root);
    let init_options: Value = server
        .entry
        .init_options
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null);
    let params = json!({
        "processId": std::process::id(),
        "clientInfo": {"name": "tyba", "version": env!("CARGO_PKG_VERSION")},
        "rootUri": root_uri,
        "capabilities": client_capabilities(),
        "initializationOptions": init_options,
        "workspaceFolders": [{"uri": root_uri, "name": server.entry.id}],
    });
    match server.request_with_timeout("initialize", params, INITIALIZE_TIMEOUT) {
        Ok(_) => {
            server.notify("initialized", json!({}));
            *server.state.lock() = RunState::Ready;
            *server.last_error.lock() = None;
            server.restart_attempts.store(0, Ordering::SeqCst);
            reopen_documents(server);
        }
        Err(e) => {
            *server.last_error.lock() = Some(format!("initialize falhou: {e}"));
            *server.state.lock() = RunState::Crashed;
            server.kill_process();
        }
    }
}

fn reopen_documents(server: &Arc<LspServer>) {
    let docs: Vec<OpenDoc> = server.open_docs.lock().values().cloned().collect();
    for doc in docs {
        server.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": doc.uri,
                    "languageId": doc.language_id,
                    "version": doc.version,
                    "text": doc.text,
                }
            }),
        );
    }
}

fn on_reader_exit(server: &Arc<LspServer>) {
    for (_, tx) in server.pending.lock().drain() {
        let _ = tx.send(Err("language server encerrou".into()));
    }
    if server.shutting_down.load(Ordering::SeqCst) {
        return;
    }
    if matches!(server.state(), RunState::Crashed) {
        return;
    }
    *server.state.lock() = RunState::Crashed;
    *server.last_error.lock() = Some("o processo do language server encerrou".into());
    server.kill_process();
    schedule_restart(server);
}

fn restart_window_exhausted(server: &Arc<LspServer>) -> bool {
    let mut window = server.restart_window.lock();
    let now = Instant::now();
    window.retain(|t| now.duration_since(*t) < RESTART_WINDOW);
    if window.len() >= MAX_RESTARTS_PER_HOUR {
        return true;
    }
    window.push(now);
    false
}

fn schedule_restart(server: &Arc<LspServer>) {
    if server.shutting_down.load(Ordering::SeqCst) {
        return;
    }
    if restart_window_exhausted(server) {
        *server.state.lock() = RunState::Crashed;
        *server.last_error.lock() =
            Some("reinícios demais em pouco tempo — pare e tente de novo".into());
        return;
    }
    let attempt = server.restart_attempts.fetch_add(1, Ordering::SeqCst) + 1;
    if attempt > MAX_RESTARTS {
        *server.state.lock() = RunState::Crashed;
        *server.last_error.lock() = Some("o language server segue caindo".into());
        return;
    }
    let backoff = Duration::from_millis(500 * (1u64 << (attempt.min(5) - 1)));
    let restart_server = Arc::clone(server);
    std::thread::spawn(move || {
        std::thread::sleep(backoff);
        if restart_server.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        *restart_server.state.lock() = RunState::Starting;
        if boot(&restart_server).is_err() {
            *restart_server.state.lock() = RunState::Crashed;
        }
    });
}

fn spawn_diag_thread(server: Arc<LspServer>, rx: Receiver<()>) {
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            std::thread::sleep(DIAG_DEBOUNCE);
            while rx.try_recv().is_ok() {}
            server.emit_diagnostics();
        }
    });
}

fn dispatch(server: &Arc<LspServer>, body: &[u8]) {
    let Ok(msg) = serde_json::from_slice::<Value>(body) else {
        return;
    };
    let has_method = msg.get("method").and_then(Value::as_str).is_some();
    if !has_method {
        if let Some(id) = msg.get("id").and_then(Value::as_i64) {
            let sender = server.pending.lock().remove(&id);
            if let Some(tx) = sender {
                if let Some(err) = msg.get("error") {
                    let message = err
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("erro do language server")
                        .to_string();
                    let _ = tx.send(Err(message));
                } else {
                    let _ = tx.send(Ok(msg.get("result").cloned().unwrap_or(Value::Null)));
                }
            }
        }
        return;
    }
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(id) = msg.get("id").cloned() {
        let result = match method {
            "workspace/configuration" => {
                let count = msg
                    .get("params")
                    .and_then(|p| p.get("items"))
                    .and_then(Value::as_array)
                    .map(|a| a.len())
                    .unwrap_or(0);
                Value::Array(vec![Value::Null; count])
            }
            "workspace/applyEdit" => json!({"applied": false}),
            _ => Value::Null,
        };
        server.respond(id, result);
        return;
    }
    if method == "textDocument/publishDiagnostics" {
        handle_diagnostics(server, msg.get("params"));
    }
}

fn handle_diagnostics(server: &Arc<LspServer>, params: Option<&Value>) {
    let Some(params) = params else {
        return;
    };
    let Some(uri) = params.get("uri").and_then(Value::as_str) else {
        return;
    };
    let Some(rel) = super::uri::to_rel(&server.root, uri) else {
        return;
    };
    let mut diags: Vec<DiagnosticIpc> = params
        .get("diagnostics")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(DiagnosticIpc::from_lsp).collect())
        .unwrap_or_default();
    if cap_per_file(&mut diags) {
        server.files_truncated.store(true, Ordering::Relaxed);
    }
    {
        let mut map = server.diagnostics.lock();
        match admit_file(map.len(), map.contains_key(&rel), diags.is_empty()) {
            Admit::Skip => return,
            Admit::SkipTruncated => {
                server.files_truncated.store(true, Ordering::Relaxed);
                return;
            }
            Admit::Track => {}
        }
        if diags.is_empty() {
            map.remove(&rel);
        } else {
            map.insert(rel.clone(), diags);
        }
    }
    server.dirty_diags.lock().insert(rel);
    server.queue_diag_emit();
}

fn cap_per_file(diags: &mut Vec<DiagnosticIpc>) -> bool {
    if diags.len() > MAX_DIAG_PER_FILE {
        diags.truncate(MAX_DIAG_PER_FILE);
        true
    } else {
        false
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Admit {
    Track,
    Skip,
    SkipTruncated,
}

fn admit_file(map_len: usize, known: bool, empty: bool) -> Admit {
    if !known && empty {
        Admit::Skip
    } else if !known && map_len >= MAX_DIAG_FILES {
        Admit::SkipTruncated
    } else {
        Admit::Track
    }
}

fn client_capabilities() -> Value {
    json!({
        "textDocument": {
            "synchronization": {"didSave": false, "dynamicRegistration": false},
            "completion": {
                "completionItem": {
                    "snippetSupport": false,
                    "documentationFormat": ["markdown", "plaintext"],
                },
                "contextSupport": true,
            },
            "hover": {"contentFormat": ["markdown", "plaintext"]},
            "signatureHelp": {
                "signatureInformation": {
                    "documentationFormat": ["markdown", "plaintext"],
                    "parameterInformation": {"labelOffsetSupport": true},
                }
            },
            "definition": {"linkSupport": false},
            "publishDiagnostics": {"relatedInformation": false},
        },
        "workspace": {"configuration": true, "workspaceFolders": true},
        "window": {"workDoneProgress": false},
        "general": {"positionEncodings": ["utf-16"]},
    })
}

fn kill_group(pid: u32) {
    #[cfg(unix)]
    unsafe {
        let pgid = libc::getpgid(pid as libc::pid_t);
        let target = if pgid > 0 { pgid } else { pid as libc::pid_t };
        libc::killpg(target, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::{PosIpc, RangeIpc};

    fn diag(msg: &str) -> DiagnosticIpc {
        DiagnosticIpc {
            range: RangeIpc {
                start: PosIpc {
                    line: 0,
                    character: 0,
                },
                end: PosIpc {
                    line: 0,
                    character: 1,
                },
            },
            severity: 1,
            message: msg.to_string(),
            source: None,
        }
    }

    #[test]
    fn per_file_cap_truncates_and_signals() {
        let mut small = vec![diag("a"); 10];
        assert!(!cap_per_file(&mut small));
        assert_eq!(small.len(), 10);

        let mut huge = vec![diag("x"); MAX_DIAG_PER_FILE + 50];
        assert!(
            cap_per_file(&mut huge),
            "estouro precisa sinalizar truncamento"
        );
        assert_eq!(huge.len(), MAX_DIAG_PER_FILE);
    }

    #[test]
    fn new_file_beyond_the_cap_is_dropped_with_a_signal() {
        assert_eq!(
            admit_file(MAX_DIAG_FILES, false, false),
            Admit::SkipTruncated
        );
        assert_eq!(admit_file(MAX_DIAG_FILES, true, false), Admit::Track);
        assert_eq!(admit_file(3, false, true), Admit::Skip);
        assert_eq!(admit_file(3, false, false), Admit::Track);
    }
}
