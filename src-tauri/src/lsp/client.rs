use std::collections::{HashMap, HashSet};
use std::io::BufReader;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use portable_pty::CommandBuilder;
use serde_json::{json, Value};

use super::document::{apply_changes, ContentChange};
use super::jsonrpc::{read_message, write_message};
use super::registry::ServerEntry;
use super::restart::{Decision, RestartPolicy};
use super::{DiagnosticIpc, DiagnosticsBatch, FileDiagnostics, RunState};
use crate::sandbox::SandboxSpec;

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_millis(2500);
const DIAG_DEBOUNCE: Duration = Duration::from_millis(120);
pub const MAX_DIAG_FILES: usize = 500;
pub const MAX_DIAG_PER_FILE: usize = 1000;

pub type DiagEmitter = dyn Fn(DiagnosticsBatch) + Send + Sync;
/// Avisa a UI que o estado do server mudou (subindo, pronto, reiniciando,
/// indisponível) — sem isso a queda só aparecia no próximo arquivo aberto.
pub type StateEmitter = dyn Fn() + Send + Sync;

#[derive(Debug)]
enum RequestError {
    /// O server respondeu, com erro.
    Server(String),
    Timeout,
    /// Sem server do outro lado: não subiu, morreu ou o pipe quebrou.
    Gone(String),
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestError::Server(e) | RequestError::Gone(e) => f.write_str(e),
            RequestError::Timeout => f.write_str("timeout aguardando o language server"),
        }
    }
}

#[derive(Clone)]
struct OpenDoc {
    uri: String,
    language_id: String,
    version: i64,
    text: String,
}

struct BootParams {
    program: std::path::PathBuf,
    pre_args: Vec<std::ffi::OsString>,
    spec: SandboxSpec,
    env: Vec<(String, String)>,
}

pub struct LspServer {
    pub entry: &'static ServerEntry,
    root: std::path::PathBuf,
    init_options: Option<String>,
    config: Value,
    boot: BootParams,
    emit: Arc<DiagEmitter>,
    on_state: Arc<StateEmitter>,
    next_id: AtomicI64,
    pending: Mutex<HashMap<i64, Sender<Result<Value, RequestError>>>>,
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
    restart: Mutex<RestartPolicy>,
    /// Cada `boot` sobe um processo novo e incrementa a geração. Threads de
    /// leitura/handshake carregam a geração em que nasceram: a de um processo
    /// que já morreu não mexe no estado do processo que o substituiu.
    generation: AtomicU64,
    /// Última geração cuja falha já foi tratada — leitor e handshake podem
    /// perceber a mesma morte; só o primeiro decide.
    failed_generation: AtomicU64,
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
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        entry: &'static ServerEntry,
        root: std::path::PathBuf,
        program: std::path::PathBuf,
        pre_args: Vec<std::ffi::OsString>,
        spec: SandboxSpec,
        env: Vec<(String, String)>,
        init_options: Option<String>,
        emit: Arc<DiagEmitter>,
        on_state: Arc<StateEmitter>,
    ) -> Result<Arc<Self>, String> {
        let config = init_options
            .as_deref()
            .or(entry.init_options)
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
        let server = Arc::new(LspServer {
            entry,
            root,
            init_options,
            config,
            boot: BootParams {
                program,
                pre_args,
                spec,
                env,
            },
            emit,
            on_state,
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
            restart: Mutex::new(RestartPolicy::default()),
            generation: AtomicU64::new(0),
            failed_generation: AtomicU64::new(0),
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

    fn set_state(&self, state: RunState) {
        let changed = {
            let mut cur = self.state.lock();
            let changed = *cur != state;
            *cur = state;
            changed
        };
        if changed {
            (self.on_state)();
        }
    }

    /// "Tentar de novo" do dono: zera o histórico de quedas e sobe já.
    pub fn retry(self: &Arc<Self>) {
        if matches!(
            self.state(),
            RunState::Starting | RunState::Ready | RunState::Restarting
        ) {
            return;
        }
        self.shutting_down.store(false, Ordering::SeqCst);
        self.restart.lock().reset();
        *self.last_error.lock() = None;
        boot_or_give_up(self);
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

    fn config_for(&self, section: Option<&str>) -> Value {
        let Some(section) = section.filter(|s| !s.is_empty()) else {
            return self.config.clone();
        };
        let mut cur = &self.config;
        for key in section.split('.') {
            match cur.get(key) {
                Some(next) => cur = next,
                None => return Value::Null,
            }
        }
        cur.clone()
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

    /// (caminho relativo, languageId, texto) dos documentos abertos — para um
    /// server novo, que substitui este, reabrir o que o editor tem.
    pub fn open_docs_snapshot(&self) -> Vec<(String, String, String)> {
        self.open_docs
            .lock()
            .iter()
            .map(|(rel, d)| (rel.clone(), d.language_id.clone(), d.text.clone()))
            .collect()
    }

    fn file_uri(&self, rel: &str) -> String {
        let abs = self.root.join(rel);
        super::uri::from_path(&abs)
    }

    // did_open/did_change/did_close seguram `open_docs` até a notificação sair,
    // e o handshake segura o mesmo lock enquanto marca Ready e reabre os
    // documentos. Sem isso uma edição que chega no meio do reinício entra no
    // texto do didOpen E sai de novo como didChange — o server aplica duas vezes.
    pub fn did_open(&self, rel: &str, language_id: &str, text: &str) {
        self.touch();
        let uri = self.file_uri(rel);
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
        let version = entry.version;
        if !matches!(self.state(), RunState::Ready) {
            return;
        }
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
        let mut docs = self.open_docs.lock();
        let (uri, version, lsp_changes) = {
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

    pub fn did_save(&self, rel: &str) {
        self.touch();
        let uri = self.open_docs.lock().get(rel).map(|d| d.uri.clone());
        if let Some(uri) = uri {
            if matches!(self.state(), RunState::Ready) {
                self.notify(
                    "textDocument/didSave",
                    json!({ "textDocument": {"uri": uri} }),
                );
            }
        }
    }

    pub fn did_close(&self, rel: &str) {
        {
            let mut docs = self.open_docs.lock();
            if let Some(doc) = docs.remove(rel) {
                if matches!(self.state(), RunState::Ready) {
                    self.notify(
                        "textDocument/didClose",
                        json!({ "textDocument": {"uri": doc.uri} }),
                    );
                }
            }
        }
        let had_diags = self.diagnostics.lock().remove(rel).is_some();
        if had_diags {
            self.dirty_diags.lock().insert(rel.to_string());
            self.queue_diag_emit();
        }
    }

    /// Request para o server. Só sai com o server `Ready`: um server caído ou
    /// reiniciando responde erro na hora, sem escrever num pipe morto nem
    /// segurar o editor até o timeout.
    pub fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        if self.state() != RunState::Ready {
            return Err("language server indisponível".into());
        }
        self.request_with_timeout(method, params, REQUEST_TIMEOUT)
            .map_err(|e| e.to_string())
    }

    fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, RequestError> {
        self.touch();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.pending.lock().insert(id, tx);
        let payload = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if let Err(e) = self.write_value(&payload) {
            self.pending.lock().remove(&id);
            return Err(RequestError::Gone(e));
        }
        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(_) => {
                self.pending.lock().remove(&id);
                Err(RequestError::Timeout)
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
        self.set_state(RunState::Stopped);
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
    let mut argv: Vec<std::ffi::OsString> = vec![server.boot.program.clone().into_os_string()];
    for arg in &server.boot.pre_args {
        argv.push(arg.clone());
    }
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

    let gen = server.generation.fetch_add(1, Ordering::SeqCst) + 1;
    *server.leader_pid.lock() = Some(pid);
    *server.stdin.lock() = Some(stdin);
    *server.child.lock() = Some(child);
    server.set_state(RunState::Starting);

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
        let reason = loop {
            match read_message(&mut reader) {
                Ok(Some(body)) => dispatch(&reader_server, &body),
                Ok(None) => break "o processo do language server encerrou".to_string(),
                Err(e) => break format!("resposta inválida do language server: {e}"),
            }
        };
        fail(&reader_server, gen, reason, true);
    });

    let init_server = Arc::clone(server);
    std::thread::spawn(move || handshake(&init_server, gen));

    Ok(())
}

/// Parâmetros do `initialize`. `processId` vai SEMPRE nulo: o server roda
/// enjaulado (Seatbelt só deixa sinalizar processos da mesma sandbox; bwrap
/// isola o namespace de PID), então o `kill(pid, 0)` com que os servers em
/// Node (vscode-languageserver: typescript, pyright, yaml, json, bash…) vigiam
/// o pai dá EPERM/ESRCH a cada 3s e eles se matam achando que o TYBA morreu.
/// Era o "LSP cai sempre". O ciclo de vida já é do core: o server morre com o
/// stdin fechado e o grupo de processo é morto no shutdown.
fn initialize_params(root_uri: &str, init_options: Value, name: &str) -> Value {
    json!({
        "processId": Value::Null,
        "clientInfo": {"name": "tyba", "version": env!("CARGO_PKG_VERSION")},
        "rootUri": root_uri,
        "capabilities": client_capabilities(),
        "initializationOptions": init_options,
        "workspaceFolders": [{"uri": root_uri, "name": name}],
    })
}

/// Motivo legível para um `initialize` recusado. O caso conhecido é o do
/// typescript-language-server sem TypeScript clássico à vista: ele procura
/// `node_modules/typescript` na raiz e o pacote ao lado dele — e o TypeScript 7
/// (nativo) instalado global não traz `tsserver.js`.
fn initialize_error(server_id: &str, message: &str) -> String {
    if server_id == "typescript-language-server"
        && message.contains("Could not find a valid TypeScript installation")
    {
        return "nenhum TypeScript com tsserver neste projeto nem junto do \
                typescript-language-server (o TypeScript 7 não traz tsserver) — \
                instale typescript@5 no projeto ou global"
            .into();
    }
    format!("initialize falhou: {message}")
}

fn handshake(server: &Arc<LspServer>, gen: u64) {
    let root_uri = super::uri::from_path(&server.root);
    let init_options: Value = server
        .init_options
        .as_deref()
        .or(server.entry.init_options)
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null);
    let params = initialize_params(&root_uri, init_options, server.entry.id);
    match server.request_with_timeout("initialize", params, INITIALIZE_TIMEOUT) {
        Ok(_) => {
            if server.generation.load(Ordering::SeqCst) != gen {
                return;
            }
            server.notify("initialized", json!({}));
            if mark_ready_and_reopen(server) {
                *server.last_error.lock() = None;
                server.restart.lock().on_ready(Instant::now());
            }
        }
        // O server respondeu com erro: é determinístico (ex.: "Could not find a
        // valid TypeScript installation"), reiniciar só repetiria o erro.
        Err(RequestError::Server(e)) => {
            fail(server, gen, initialize_error(server.entry.id, &e), false)
        }
        Err(RequestError::Timeout) => fail(
            server,
            gen,
            "o language server não respondeu ao initialize".into(),
            true,
        ),
        // Morreu ou o pipe quebrou: o leitor também percebe; quem chegar
        // primeiro decide (fail é idempotente por geração).
        Err(RequestError::Gone(e)) => fail(server, gen, e, true),
    }
}

/// Marca Ready e reabre os documentos sob o mesmo lock de `open_docs`. Só
/// vale saindo de Starting: se no meio do initialize o painel fechou (Stopped)
/// ou a morte já foi tratada (Restarting/Crashed), não ressuscita o estado.
fn mark_ready_and_reopen(server: &Arc<LspServer>) -> bool {
    {
        let docs = server.open_docs.lock();
        {
            let mut state = server.state.lock();
            if *state != RunState::Starting {
                return false;
            }
            *state = RunState::Ready;
        }
        for doc in docs.values() {
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
    (server.on_state)();
    true
}

fn exit_detail(server: &LspServer) -> Option<String> {
    let status = server.child.lock().as_mut()?.try_wait().ok()??;
    if let Some(code) = status.code() {
        return Some(format!("código {code}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return Some(format!("sinal {sig}"));
        }
    }
    None
}

/// Uma morte do processo da geração `gen` — percebida pelo leitor (EOF, frame
/// inválido) ou pelo handshake (erro, timeout). Mata o que sobrou, solta quem
/// esperava resposta e decide: reiniciar com backoff ou desistir com motivo.
fn fail(server: &Arc<LspServer>, gen: u64, reason: String, restartable: bool) {
    if server.generation.load(Ordering::SeqCst) != gen {
        return;
    }
    if server.failed_generation.swap(gen, Ordering::SeqCst) == gen {
        return;
    }
    if server.shutting_down.load(Ordering::SeqCst) {
        return;
    }
    let reason = match exit_detail(server) {
        Some(detail) => format!("{reason} ({detail})"),
        None => reason,
    };
    server.kill_process();
    for (_, tx) in server.pending.lock().drain() {
        let _ = tx.send(Err(RequestError::Gone("language server encerrou".into())));
    }
    let decision = if restartable {
        server.restart.lock().on_crash(Instant::now())
    } else {
        Decision::GiveUp(reason.clone())
    };
    match decision {
        Decision::GiveUp(why) => {
            *server.last_error.lock() = Some(if why == reason {
                reason
            } else {
                format!("{why} — último erro: {reason}")
            });
            server.set_state(RunState::Crashed);
        }
        Decision::Retry { after, .. } => {
            *server.last_error.lock() = Some(reason);
            server.set_state(RunState::Restarting);
            let restart_server = Arc::clone(server);
            std::thread::spawn(move || {
                std::thread::sleep(after);
                if restart_server.shutting_down.load(Ordering::SeqCst)
                    || restart_server.generation.load(Ordering::SeqCst) != gen
                    || restart_server.state() != RunState::Restarting
                {
                    return;
                }
                boot_or_give_up(&restart_server);
            });
        }
    }
}

/// Subir pode falhar antes de existir processo (binário sumiu, jaula
/// recusou): aí não há o que reiniciar — fica indisponível com o motivo.
fn boot_or_give_up(server: &Arc<LspServer>) {
    if let Err(e) = boot(server) {
        *server.last_error.lock() = Some(e);
        server.set_state(RunState::Crashed);
    }
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
                    let _ = tx.send(Err(RequestError::Server(message)));
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
                let items = msg
                    .get("params")
                    .and_then(|p| p.get("items"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let values = items
                    .iter()
                    .map(|item| server.config_for(item.get("section").and_then(Value::as_str)))
                    .collect();
                Value::Array(values)
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
            "synchronization": {"didSave": true, "willSave": false, "dynamicRegistration": false},
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
            // Só correções que vêm com a edição pronta: o TYBA não executa
            // comando de servidor (workspace/executeCommand).
            "codeAction": {
                "codeActionLiteralSupport": {"codeActionKind": {"valueSet": ["quickfix"]}},
                "isPreferredSupport": true,
            },
            "documentSymbol": {"hierarchicalDocumentSymbolSupport": true},
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

    #[test]
    fn initialize_never_sends_the_host_process_id() {
        // Enjaulado, o server não enxerga o PID do TYBA: com processId preenchido
        // os servers em Node dão kill(pid, 0) → EPERM e se matam a cada 3s.
        let params = initialize_params("file:///private/wt/proj", Value::Null, "x");
        assert_eq!(params.get("processId"), Some(&Value::Null));
        assert_eq!(params["rootUri"], "file:///private/wt/proj");
        assert_eq!(
            params["workspaceFolders"][0]["uri"],
            "file:///private/wt/proj"
        );
    }

    #[test]
    fn missing_typescript_becomes_an_actionable_reason() {
        let msg = "Request initialize failed with message: Could not find a valid \
                   TypeScript installation. Please ensure...";
        let reason = initialize_error("typescript-language-server", msg);
        assert!(reason.contains("typescript@5"), "{reason}");
        let other = initialize_error("pyright", "boom");
        assert_eq!(other, "initialize falhou: boom");
    }

    /// Server sintético que morre na hora (`sh -c 'exit 3'`), subido pela
    /// jaula real da plataforma. Sem sandbox disponível (CI sem bwrap), pula.
    #[cfg(unix)]
    #[test]
    fn a_server_that_dies_goes_to_restarting_and_requests_fail_fast() {
        use crate::lsp::registry::entry_by_id;
        use crate::lsp::sandbox::{lsp_sandbox_spec, LspSpecCtx};

        let tmp = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let base = tmp.join(format!("lsp-crash-{}", uuid::Uuid::new_v4()));
        let root = base.join("proj");
        let cache = base.join("cache");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(cache.join(".rt")).unwrap();
        let mut env: HashMap<String, String> = HashMap::new();
        env.insert("HOME".into(), base.join("home").to_string_lossy().into());
        let entry = entry_by_id("taplo").unwrap();
        let exe = std::env::current_exe().unwrap();
        let ctx = LspSpecCtx {
            root: &root,
            cache_dir: &cache,
            exe: &exe,
            env: &env,
            exec_path_dirs: vec![std::path::PathBuf::from("/bin")],
            read_allow_extra: vec![],
            extra_reads: vec![],
            data_dir: base.join("data"),
            data_dir_reads: vec![],
        };
        let spec = lsp_sandbox_spec(entry, &ctx).unwrap();
        let state_changes = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&state_changes);
        let spawned = LspServer::spawn(
            entry,
            root.clone(),
            "/bin/sh".into(),
            vec!["-c".into(), "exit 3".into()],
            spec,
            vec![("PATH".into(), "/bin:/usr/bin".into())],
            None,
            Arc::new(|_b: DiagnosticsBatch| {}),
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        );
        let server = match spawned {
            Ok(s) => s,
            Err(e) => {
                eprintln!("sandbox indisponível aqui ({e}) — teste pulado");
                std::fs::remove_dir_all(&base).ok();
                return;
            }
        };
        server.did_open("a.toml", "toml", "a = 1\n");

        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && server.state() != RunState::Restarting {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(server.state(), RunState::Restarting);
        let reason = server.last_error().unwrap_or_default();
        assert!(reason.contains("encerrou"), "{reason}");
        assert!(
            state_changes.load(Ordering::SeqCst) >= 1,
            "a UI é avisada da queda"
        );

        let t = Instant::now();
        assert!(server.request("textDocument/hover", Value::Null).is_err());
        assert!(
            t.elapsed() < Duration::from_millis(200),
            "request para server caído falha na hora, sem esperar timeout"
        );
        assert!(
            server.has_open_docs(),
            "o documento fica guardado para o reopen"
        );

        server.shutdown();
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(
            server.state(),
            RunState::Stopped,
            "shutdown cancela o reinício agendado"
        );
        std::fs::remove_dir_all(&base).ok();
    }

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
            code: None,
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
