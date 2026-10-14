//! Logs estruturados de container: o parser de linha (hora, nível, frame de
//! stack trace), o mapeamento de caminho do container para o host e o
//! seguidor (`docker logs -f`) que entrega as linhas ao webview em lotes.
//!
//! O webview só pinta: nível, frame e caminho no host já chegam resolvidos.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use parking_lot::Mutex;
use regex::Regex;
use serde::Serialize;

/// Linhas guardadas por container. O que passa disso sai pelo começo: é a
/// mesma janela que o `--tail` do docker daria, só que viva.
pub const BUFFER_CAP: usize = 5000;
/// Linhas pedidas ao docker na primeira abertura.
pub const INITIAL_TAIL: usize = 1000;
/// Cadência do lote para o webview (princípio 3: nunca um evento por linha).
const FLUSH_EVERY: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Frame {
    /// Caminho como aparece na linha (dentro do container).
    pub path: String,
    pub line: u32,
    pub col: Option<u32>,
    /// Mesmo arquivo no host, quando um bind mount o expõe e ele existe aqui.
    /// `None` = o link aparece desabilitado.
    pub host_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogLine {
    pub seq: u64,
    /// RFC3339 do docker (`--timestamps`).
    pub ts: Option<String>,
    pub ts_ms: Option<i64>,
    pub level: Option<LogLevel>,
    /// Linha de stack trace / continuação: herda o nível da anterior.
    pub continuation: bool,
    pub stderr: bool,
    pub text: String,
    pub frame: Option<Frame>,
}

/// Separa o timestamp que o `docker logs --timestamps` antepõe.
pub fn split_timestamp(raw: &str) -> (Option<(String, i64)>, &str) {
    let Some((head, rest)) = raw.split_once(' ') else {
        return (None, raw);
    };
    match chrono::DateTime::parse_from_rfc3339(head) {
        Ok(dt) => (Some((head.to_string(), dt.timestamp_millis())), rest),
        Err(_) => (None, raw),
    }
}

static LEVEL_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(fatal|panic|critical|crit|severe|error|err|exception|warning|warn|deprecated|info|notice|debug|trace|verbose)\b",
    )
    .unwrap()
});

/// `TypeError: …`, `ValueError: …`, `Uncaught ReferenceError …`, `java.lang.IllegalStateException`.
static EXCEPTION_HEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:Uncaught\s+)?(?:[\w$]+\.)*[A-Z][\w$]*(?:Error|Exception)\b|^Traceback \(most recent call last\)")
        .unwrap()
});

fn level_of_word(word: &str) -> LogLevel {
    match word.to_ascii_lowercase().as_str() {
        "fatal" | "panic" | "critical" | "crit" | "severe" | "error" | "err" | "exception" => {
            LogLevel::Error
        }
        "warning" | "warn" | "deprecated" => LogLevel::Warn,
        "info" | "notice" => LogLevel::Info,
        _ => LogLevel::Debug,
    }
}

fn level_of_json(msg: &str) -> Option<LogLevel> {
    if !msg.starts_with('{') {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(msg).ok()?;
    let field = ["level", "lvl", "severity", "log.level"]
        .iter()
        .find_map(|k| value.get(*k))?;
    if let Some(n) = field.as_u64() {
        // Níveis numéricos do pino/bunyan.
        return Some(match n {
            50.. => LogLevel::Error,
            40..=49 => LogLevel::Warn,
            30..=39 => LogLevel::Info,
            _ => LogLevel::Debug,
        });
    }
    let word = field.as_str()?;
    LEVEL_WORD.find(word).map(|m| level_of_word(m.as_str()))
}

/// Nível da mensagem (já sem timestamp). Heurística: JSON estruturado primeiro;
/// depois cabeçalho de exceção; depois a PRIMEIRA palavra de nível da linha —
/// `info: tentando de novo após error` é info, não erro.
pub fn detect_level(msg: &str) -> Option<LogLevel> {
    if let Some(level) = level_of_json(msg.trim_start()) {
        return Some(level);
    }
    if EXCEPTION_HEAD.is_match(msg) {
        return Some(LogLevel::Error);
    }
    LEVEL_WORD.find(msg).map(|m| level_of_word(m.as_str()))
}

static FRAME_JS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bat (?:[^()]*\()?(?P<path>(?:/|\./|[A-Za-z]:\\)?[^\s():]+\.[A-Za-z0-9]+):(?P<line>\d+)(?::(?P<col>\d+))?\)?")
        .unwrap()
});
static FRAME_PY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"File "(?P<path>[^"]+)", line (?P<line>\d+)"#).unwrap());
/// Go, Rust, compiladores: `/app/main.go:42`, `src/main.rs:10:5`. Exige uma
/// barra no caminho para não confundir `host:porta` com arquivo.
static FRAME_GENERIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:^|[\s(\["'=])(?P<path>(?:/|\./)?[\w.\-]+(?:/[\w.\-]+)+\.[A-Za-z0-9]+):(?P<line>\d+)(?::(?P<col>\d+))?"#)
        .unwrap()
});

/// Primeiro `arquivo:linha` reconhecível na mensagem.
pub fn detect_frame(msg: &str) -> Option<Frame> {
    for re in [&*FRAME_JS, &*FRAME_PY, &*FRAME_GENERIC] {
        if let Some(c) = re.captures(msg) {
            let path = c.name("path")?.as_str().to_string();
            if path.contains("://") {
                continue;
            }
            let line = c.name("line")?.as_str().parse().ok()?;
            let col = c.name("col").and_then(|m| m.as_str().parse().ok());
            return Some(Frame {
                path,
                line,
                col,
                host_path: None,
            });
        }
    }
    None
}

fn is_continuation(msg: &str) -> bool {
    msg.starts_with(' ') || msg.starts_with('\t') || msg.trim_start().starts_with("at ")
}

/// Uma linha crua do `docker logs --timestamps`. `prev` é o nível da linha
/// anterior do mesmo stream: linha de stack trace o herda.
pub fn parse_line(raw: &str, stderr: bool, prev: Option<LogLevel>, seq: u64) -> LogLine {
    let raw = raw.strip_suffix('\r').unwrap_or(raw);
    let (ts, msg) = split_timestamp(raw);
    let continuation =
        matches!(prev, Some(LogLevel::Error | LogLevel::Warn)) && is_continuation(msg);
    let level = if continuation {
        prev
    } else {
        detect_level(msg)
    };
    LogLine {
        seq,
        ts_ms: ts.as_ref().map(|(_, ms)| *ms),
        ts: ts.map(|(s, _)| s),
        level,
        continuation,
        stderr,
        text: msg.to_string(),
        frame: detect_frame(msg),
    }
}

/// Bind mount do container: `source` (host) montado em `destination`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mount {
    pub source: String,
    pub destination: String,
}

fn normalized(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::RootDir => out.push("/"),
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            // `..` num caminho de log é raro e sair de um mount por ele é
            // exatamente o que não se quer resolver.
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// Caminho de dentro do container → caminho no host, pelo bind mount mais
/// específico que o cobre. Relativo resolve contra o `WorkingDir` do container.
pub fn map_container_path(path: &str, workdir: Option<&str>, mounts: &[Mount]) -> Option<PathBuf> {
    let raw = Path::new(path);
    let absolute = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        Path::new(workdir.filter(|w| w.starts_with('/'))?).join(raw)
    };
    let inside = normalized(&absolute)?;
    mounts
        .iter()
        .filter_map(|m| {
            let dest = normalized(Path::new(&m.destination))?;
            let rest = inside.strip_prefix(&dest).ok()?;
            Some((dest.components().count(), Path::new(&m.source).join(rest)))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, host)| host)
}

/// O que sai por Copiar/Baixar: as linhas juntas, com segredos mascarados
/// (princípio 10) — a tela mostra o que o container escreveu, a área de
/// transferência e o disco não levam o token junto.
pub fn export_text(lines: &[String]) -> String {
    let joined = lines.join("\n");
    crate::session::redact::redact(&joined).into_owned()
}

/// Onde o "Baixar" grava: na pasta dada, com nome saneado (o webview não
/// escolhe caminho) e sem sobrescrever o que já existe.
pub fn download_target(dir: &Path, name: &str) -> PathBuf {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let clean = clean.trim_start_matches('.');
    let clean = if clean.is_empty() { "tyba.txt" } else { clean };
    let first = dir.join(clean);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match clean.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (clean.to_string(), String::new()),
    };
    (1..)
        .map(|n| dir.join(format!("{stem}-{n}{ext}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

#[derive(Debug, Clone, Serialize)]
pub struct LogsBatch {
    pub lines: Vec<LogLine>,
    /// O `docker logs -f` terminou (container parou, foi removido).
    pub ended: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogsSnapshot {
    pub lines: Vec<LogLine>,
    pub following: bool,
}

pub type Emit = Arc<dyn Fn(LogsBatch) + Send + Sync>;

#[derive(Debug, Clone, Default)]
pub struct PathMapping {
    pub workdir: Option<String>,
    pub mounts: Vec<Mount>,
}

impl PathMapping {
    fn annotate(&self, line: &mut LogLine) {
        if let Some(frame) = line.frame.as_mut() {
            frame.host_path =
                map_container_path(&frame.path, self.workdir.as_deref(), &self.mounts)
                    .filter(|p| p.is_file())
                    .map(|p| p.to_string_lossy().into_owned());
        }
    }
}

struct Follower {
    buffer: Mutex<VecDeque<LogLine>>,
    pending: Mutex<Vec<LogLine>>,
    seq: AtomicU64,
    running: AtomicBool,
    child: Mutex<Option<Child>>,
    mapping: PathMapping,
}

impl Follower {
    fn push(&self, raw: &str, stderr: bool, prev: &mut Option<LogLevel>) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let mut line = parse_line(raw, stderr, *prev, seq);
        *prev = line.level;
        self.mapping.annotate(&mut line);
        self.pending.lock().push(line);
    }

    fn last_ts(&self) -> Option<String> {
        self.buffer.lock().iter().rev().find_map(|l| l.ts.clone())
    }
}

/// Um `docker logs -f` por container, compartilhado pelas abas que o mostram.
#[derive(Default)]
pub struct LogsManager {
    followers: Mutex<HashMap<String, Arc<Follower>>>,
}

impl LogsManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Garante o seguidor vivo e devolve o que já se tem. `spawn` recebe o
    /// `--since` (retomada) ou `None` (primeira vez, `--tail`) e devolve o
    /// processo com stdout/stderr em pipe.
    pub fn start<S>(
        &self,
        id: &str,
        mapping: PathMapping,
        emit: Emit,
        spawn: S,
    ) -> Result<LogsSnapshot, String>
    where
        S: FnOnce(Option<String>) -> Result<Child, String>,
    {
        let existing = self.followers.lock().get(id).cloned();
        if let Some(f) = &existing {
            if f.running.load(Ordering::Acquire) {
                return Ok(LogsSnapshot {
                    lines: f.buffer.lock().iter().cloned().collect(),
                    following: true,
                });
            }
        }
        let follower = match existing {
            Some(old) => Arc::new(Follower {
                buffer: Mutex::new(std::mem::take(&mut *old.buffer.lock())),
                pending: Mutex::new(Vec::new()),
                seq: AtomicU64::new(old.seq.load(Ordering::Relaxed)),
                running: AtomicBool::new(true),
                child: Mutex::new(None),
                mapping,
            }),
            None => Arc::new(Follower {
                buffer: Mutex::new(VecDeque::new()),
                pending: Mutex::new(Vec::new()),
                seq: AtomicU64::new(0),
                running: AtomicBool::new(true),
                child: Mutex::new(None),
                mapping,
            }),
        };
        let mut child = spawn(follower.last_ts())?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        *follower.child.lock() = Some(child);

        let mut readers = Vec::new();
        for (pipe, is_err) in [
            (
                stdout.map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
                false,
            ),
            (
                stderr.map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
                true,
            ),
        ] {
            let Some(pipe) = pipe else { continue };
            let f = Arc::clone(&follower);
            readers.push(std::thread::spawn(move || {
                let mut prev = None;
                let reader = BufReader::new(pipe);
                for line in reader.split(b'\n') {
                    let Ok(bytes) = line else { break };
                    f.push(&String::from_utf8_lossy(&bytes), is_err, &mut prev);
                }
            }));
        }

        let f = Arc::clone(&follower);
        std::thread::spawn(move || loop {
            std::thread::sleep(FLUSH_EVERY);
            let done = readers.iter().all(|r| r.is_finished());
            let batch: Vec<LogLine> = std::mem::take(&mut *f.pending.lock());
            if !batch.is_empty() {
                let mut buf = f.buffer.lock();
                buf.extend(batch.iter().cloned());
                while buf.len() > BUFFER_CAP {
                    buf.pop_front();
                }
            }
            if done {
                f.running.store(false, Ordering::Release);
                if let Some(mut child) = f.child.lock().take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                emit(LogsBatch {
                    lines: batch,
                    ended: true,
                });
                break;
            }
            if !batch.is_empty() {
                emit(LogsBatch {
                    lines: batch,
                    ended: false,
                });
            }
        });

        let snapshot = follower.buffer.lock().iter().cloned().collect();
        self.followers.lock().insert(id.to_string(), follower);
        Ok(LogsSnapshot {
            lines: snapshot,
            following: true,
        })
    }

    /// Mata o seguidor e esquece o buffer.
    pub fn stop(&self, id: &str) {
        if let Some(f) = self.followers.lock().remove(id) {
            kill(&f);
        }
    }

    /// Só sobrevive o seguidor de container que ainda tem aba de logs aberta.
    /// Chamado a cada mudança de layout: fechar a aba, o workspace, o pane ou
    /// a sessão dona — qualquer caminho que tire a aba — mata o `docker logs -f`.
    pub fn retain(&self, open: &std::collections::HashSet<String>) {
        let dead: Vec<Arc<Follower>> = {
            let mut followers = self.followers.lock();
            let gone: Vec<String> = followers
                .keys()
                .filter(|id| !open.contains(*id))
                .cloned()
                .collect();
            gone.iter().filter_map(|id| followers.remove(id)).collect()
        };
        for f in dead {
            kill(&f);
        }
    }

    /// Saída do app: nenhum `docker logs -f` fica órfão.
    pub fn stop_all(&self) {
        self.retain(&std::collections::HashSet::new());
    }

    pub fn is_following(&self, id: &str) -> bool {
        self.followers.lock().contains_key(id)
    }
}

fn kill(f: &Follower) {
    if let Some(mut child) = f.child.lock().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_docker_timestamp_and_keeps_message() {
        let (ts, msg) = split_timestamp("2026-10-12T15:24:02.123456789Z Servidor em :3000");
        let (s, ms) = ts.unwrap();
        assert_eq!(s, "2026-10-12T15:24:02.123456789Z");
        assert_eq!(ms, 1_791_818_642_123);
        assert_eq!(msg, "Servidor em :3000");
        let (none, msg) = split_timestamp("sem hora nenhuma");
        assert!(none.is_none());
        assert_eq!(msg, "sem hora nenhuma");
    }

    #[test]
    fn detects_levels_from_text_json_and_exceptions() {
        assert_eq!(detect_level("GET /health 200 1ms"), None);
        assert_eq!(
            detect_level("[WARN] Pool do Postgres a 80%"),
            Some(LogLevel::Warn)
        );
        assert_eq!(detect_level("level=error msg=boom"), Some(LogLevel::Error));
        assert_eq!(
            detect_level("TypeError: process.uptim is not a function"),
            Some(LogLevel::Error)
        );
        assert_eq!(
            detect_level("Traceback (most recent call last):"),
            Some(LogLevel::Error)
        );
        assert_eq!(
            detect_level("info: tentando de novo após error"),
            Some(LogLevel::Info),
            "a primeira palavra de nível manda"
        );
        assert_eq!(
            detect_level(r#"{"level":50,"msg":"x"}"#),
            Some(LogLevel::Error)
        );
        assert_eq!(
            detect_level(r#"{"level":"warn","msg":"x"}"#),
            Some(LogLevel::Warn)
        );
        assert_eq!(
            detect_level(r#"{"severity":"DEBUG"}"#),
            Some(LogLevel::Debug)
        );
        assert_eq!(detect_level("GET /errors 200"), None, "palavra inteira só");
    }

    #[test]
    fn extracts_stack_frames_of_several_runtimes() {
        let js = detect_frame("    at /app/src/server.js:8:43").unwrap();
        assert_eq!(
            (js.path.as_str(), js.line, js.col),
            ("/app/src/server.js", 8, Some(43))
        );
        let js2 = detect_frame(
            "    at Layer.handle (/app/node_modules/express/lib/router/layer.js:95:5)",
        )
        .unwrap();
        assert_eq!(js2.path, "/app/node_modules/express/lib/router/layer.js");
        assert_eq!(js2.line, 95);
        let py = detect_frame(r#"  File "/srv/app/main.py", line 12, in <module>"#).unwrap();
        assert_eq!(
            (py.path.as_str(), py.line, py.col),
            ("/srv/app/main.py", 12, None)
        );
        let go = detect_frame("\t/app/cmd/api/main.go:42 +0x1d").unwrap();
        assert_eq!((go.path.as_str(), go.line), ("/app/cmd/api/main.go", 42));
        let rs = detect_frame("panicked at src/main.rs:10:5:").unwrap();
        assert_eq!(
            (rs.path.as_str(), rs.line, rs.col),
            ("src/main.rs", 10, Some(5))
        );
    }

    #[test]
    fn frames_ignore_urls_hosts_and_node_internals() {
        assert!(detect_frame("Servidor em http://localhost:3000").is_none());
        assert!(detect_frame("conectando em db.example.com:5432").is_none());
        assert!(
            detect_frame("    at Module._compile (node:internal/modules/cjs/loader:1256:14)")
                .is_none()
        );
        assert!(detect_frame("GET https://api.example.com/v1/users.json:200").is_none());
    }

    #[test]
    fn stack_lines_inherit_error_level_as_continuation() {
        let head = parse_line(
            "2026-10-12T15:26:58Z TypeError: process.uptim is not a function",
            true,
            None,
            1,
        );
        assert_eq!(head.level, Some(LogLevel::Error));
        assert!(!head.continuation);
        let frame = parse_line(
            "2026-10-12T15:26:58Z     at /app/src/server.js:8:43",
            true,
            head.level,
            2,
        );
        assert!(frame.continuation);
        assert_eq!(frame.level, Some(LogLevel::Error));
        assert_eq!(frame.frame.unwrap().line, 8);
        // Linha recuada depois de uma info não é continuação.
        let indented = parse_line("  listening", false, Some(LogLevel::Info), 3);
        assert!(!indented.continuation);
        assert_eq!(indented.level, None);
    }

    #[test]
    fn maps_container_path_through_most_specific_bind_mount() {
        let mounts = vec![
            Mount {
                source: "/Users/dev/poc-git".into(),
                destination: "/app".into(),
            },
            Mount {
                source: "/Users/dev/shared".into(),
                destination: "/app/vendor".into(),
            },
        ];
        assert_eq!(
            map_container_path("/app/src/server.js", None, &mounts),
            Some(PathBuf::from("/Users/dev/poc-git/src/server.js"))
        );
        assert_eq!(
            map_container_path("/app/vendor/lib.js", None, &mounts),
            Some(PathBuf::from("/Users/dev/shared/lib.js"))
        );
        assert_eq!(
            map_container_path("src/server.js", Some("/app"), &mounts),
            Some(PathBuf::from("/Users/dev/poc-git/src/server.js")),
            "relativo resolve contra o WorkingDir"
        );
        assert_eq!(map_container_path("src/server.js", None, &mounts), None);
        assert_eq!(
            map_container_path("/usr/lib/node/x.js", None, &mounts),
            None
        );
        assert_eq!(
            map_container_path("/app/../etc/passwd", None, &mounts),
            None,
            "`..` não escapa do mount"
        );
        assert_eq!(
            map_container_path("/application/x.js", None, &mounts),
            None,
            "prefixo é por componente, não por texto"
        );
    }

    #[test]
    fn export_masks_secrets_before_copy_or_download() {
        let lines = vec![
            "GET /health 200".to_string(),
            "token=ghp_abcdefghijklmnopqrstuvwxyz0123456789 aceito".to_string(),
            "AWS AKIAIOSFODNN7EXAMPLE".to_string(),
        ];
        let out = export_text(&lines);
        assert!(out.contains("GET /health 200"));
        assert!(!out.contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
        assert_eq!(
            out.matches(crate::session::redact::REDACTION_MARK).count(),
            2
        );
        assert_eq!(out.lines().count(), 3);
    }

    #[test]
    fn download_target_sanitizes_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("tyba-dl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = download_target(&dir, "../api logs.txt");
        assert_eq!(first, dir.join("-api-logs.txt"));
        std::fs::write(&first, "x").unwrap();
        assert_eq!(
            download_target(&dir, "../api logs.txt"),
            dir.join("-api-logs-1.txt")
        );
        assert_eq!(download_target(&dir, ""), dir.join("tyba.txt"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // /bin/sh (e libc::kill) só no Unix.
    #[cfg(unix)]
    #[test]
    fn retain_kills_followers_without_an_open_tab() {
        use std::process::{Command, Stdio};
        let mgr = LogsManager::new();
        let pid = Arc::new(Mutex::new(0u32));
        for id in ["keep", "drop"] {
            let pid = Arc::clone(&pid);
            mgr.start(id, PathMapping::default(), Arc::new(|_| {}), move |_| {
                let child = Command::new("/bin/sh")
                    .args(["-c", "sleep 30"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                if id == "drop" {
                    *pid.lock() = child.id();
                }
                Ok(child)
            })
            .unwrap();
        }
        mgr.retain(&std::collections::HashSet::from(["keep".to_string()]));
        assert!(mgr.is_following("keep"));
        assert!(!mgr.is_following("drop"));
        // O processo do seguidor descartado morreu (kill + wait).
        let alive = unsafe { libc::kill(*pid.lock() as libc::pid_t, 0) } == 0;
        assert!(!alive, "docker logs -f ficou órfão");
        mgr.stop_all();
        assert!(!mgr.is_following("keep"));
    }

    // /bin/sh (e libc::kill) só no Unix.
    #[cfg(unix)]
    #[test]
    fn follower_batches_lines_and_reports_end() {
        use std::process::{Command, Stdio};
        let mgr = LogsManager::new();
        let got: Arc<Mutex<Vec<LogsBatch>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&got);
        let snapshot = mgr
            .start(
                "abc",
                PathMapping::default(),
                Arc::new(move |b| sink.lock().push(b)),
                |since| {
                    assert!(since.is_none());
                    Command::new("/bin/sh")
                        .args(["-c", "printf '2026-10-12T15:24:02Z info up\\n2026-10-12T15:24:03Z Error: boom\\n'"])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .map_err(|e| e.to_string())
                },
            )
            .unwrap();
        assert!(snapshot.following);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !got.lock().iter().any(|b| b.ended) {
            assert!(
                std::time::Instant::now() < deadline,
                "seguidor não terminou"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let lines: Vec<LogLine> = got.lock().iter().flat_map(|b| b.lines.clone()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].level, Some(LogLevel::Error));
        assert_eq!(lines[0].seq + 1, lines[1].seq);
        mgr.stop("abc");
    }
}
