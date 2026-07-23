use std::collections::{HashSet, VecDeque};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Runtime};

use crate::agent::subagents::{finish_by_file, SharedCoordinator, SharedSessions};
use crate::approvals::now_ms;
use crate::session::redact::redact;
use crate::session::SessionId;
use crate::status::transcript::clean_summary;

pub const EVENT_SUBAGENT_TRANSCRIPT: &str = "subagents://transcript";

pub const TAIL_ENTRIES: usize = 500;

const TOOL_SUMMARY_MAX: usize = 200;
const TOOL_RESULT_MAX: usize = 500;
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Fim de subagente async: arquivo sem crescer por estes ciclos de poll E última
/// fala sendo a resposta final assistant. Pausa entre tool calls não conta —
/// exige a resposta final, não só estabilidade.
const STABLE_CYCLES: u32 = 2;
const FINISH_SUMMARY_MAX: usize = 280;
const FINAL_TAIL_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    AssistantText,
    ToolUse,
    ToolResult,
    UserText,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TranscriptEntry {
    pub seq: u64,
    pub kind: EntryKind,
    pub text: String,
    pub tool_name: Option<String>,
    pub ts: Option<String>,
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

fn clean_text(text: &str) -> String {
    redact(text.trim()).into_owned()
}

fn tool_use_summary(block: &Value, name: &str) -> String {
    let input = block.get("input");
    let raw = input
        .and_then(|i| i.get("command").and_then(Value::as_str))
        .or_else(|| input.and_then(|i| i.get("file_path").and_then(Value::as_str)))
        .or_else(|| input.and_then(|i| i.get("description").and_then(Value::as_str)))
        .unwrap_or(name);
    let redacted = redact(&collapse_whitespace(raw)).into_owned();
    truncate_chars(&redacted, TOOL_SUMMARY_MAX)
}

fn tool_result_text(block: &Value) -> String {
    let raw = match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => {
            let mut acc = String::new();
            for part in parts {
                if part.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        acc.push_str(text);
                    }
                }
            }
            acc
        }
        _ => String::new(),
    };
    let redacted = redact(&collapse_whitespace(&raw)).into_owned();
    truncate_chars(&redacted, TOOL_RESULT_MAX)
}

fn push_entry(
    out: &mut Vec<TranscriptEntry>,
    line_offset: u64,
    local: &mut u64,
    kind: EntryKind,
    text: String,
    tool_name: Option<String>,
    ts: &Option<String>,
) {
    out.push(TranscriptEntry {
        seq: line_offset + *local,
        kind,
        text,
        tool_name,
        ts: ts.clone(),
    });
    *local += 1;
}

fn parse_assistant(
    v: &Value,
    out: &mut Vec<TranscriptEntry>,
    line_offset: u64,
    local: &mut u64,
    ts: &Option<String>,
) {
    let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
        return;
    };
    if let Some(text) = content.as_str() {
        let text = clean_text(text);
        if !text.is_empty() {
            push_entry(
                out,
                line_offset,
                local,
                EntryKind::AssistantText,
                text,
                None,
                ts,
            );
        }
        return;
    }
    let Some(blocks) = content.as_array() else {
        return;
    };
    let mut combined = String::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                combined.push_str(text);
            }
        }
    }
    let combined = clean_text(&combined);
    if !combined.is_empty() {
        push_entry(
            out,
            line_offset,
            local,
            EntryKind::AssistantText,
            combined,
            None,
            ts,
        );
    }
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some("tool_use") {
            let name = block.get("name").and_then(Value::as_str).unwrap_or("");
            let summary = tool_use_summary(block, name);
            let tool_name = (!name.is_empty()).then(|| name.to_string());
            push_entry(
                out,
                line_offset,
                local,
                EntryKind::ToolUse,
                summary,
                tool_name,
                ts,
            );
        }
    }
}

fn parse_user(
    v: &Value,
    out: &mut Vec<TranscriptEntry>,
    line_offset: u64,
    local: &mut u64,
    ts: &Option<String>,
) {
    let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
        return;
    };
    if let Some(text) = content.as_str() {
        let text = clean_text(text);
        if !text.is_empty() {
            push_entry(out, line_offset, local, EntryKind::UserText, text, None, ts);
        }
        return;
    }
    let Some(blocks) = content.as_array() else {
        return;
    };
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some("tool_result") {
            let text = tool_result_text(block);
            if !text.is_empty() {
                push_entry(
                    out,
                    line_offset,
                    local,
                    EntryKind::ToolResult,
                    text,
                    None,
                    ts,
                );
            }
        }
    }
}

fn parse_into(out: &mut Vec<TranscriptEntry>, line: &str, line_offset: u64) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
        return;
    };
    let ts = v
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut local = 0u64;
    match v.get("type").and_then(Value::as_str) {
        Some("assistant") => parse_assistant(&v, out, line_offset, &mut local, &ts),
        Some("user") => parse_user(&v, out, line_offset, &mut local, &ts),
        _ => {}
    }
}

pub fn read_entries(
    path: &Path,
    cursor: Option<u64>,
    max_entries: usize,
) -> (Vec<TranscriptEntry>, u64, bool) {
    let Ok(mut file) = File::open(path) else {
        return (Vec::new(), cursor.unwrap_or(0), true);
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return (Vec::new(), cursor.unwrap_or(0), true);
    };
    match cursor {
        None => read_tail(&mut file, max_entries),
        Some(start) if start > len => read_forward(&mut file, 0, max_entries),
        Some(start) => read_forward(&mut file, start, max_entries),
    }
}

fn read_forward(
    file: &mut File,
    start: u64,
    max_entries: usize,
) -> (Vec<TranscriptEntry>, u64, bool) {
    if file.seek(SeekFrom::Start(start)).is_err() {
        return (Vec::new(), start, true);
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return (Vec::new(), start, true);
    }
    let mut entries = Vec::new();
    let mut rel = 0usize;
    let complete;
    loop {
        match buf[rel..].iter().position(|&b| b == b'\n') {
            Some(off) => {
                let line_end = rel + off;
                if let Ok(line) = std::str::from_utf8(&buf[rel..line_end]) {
                    parse_into(&mut entries, line, start + rel as u64);
                }
                rel = line_end + 1;
                if entries.len() >= max_entries {
                    complete = !buf[rel..].contains(&b'\n');
                    break;
                }
            }
            None => {
                complete = true;
                break;
            }
        }
    }
    (entries, start + rel as u64, complete)
}

fn read_tail(file: &mut File, max_entries: usize) -> (Vec<TranscriptEntry>, u64, bool) {
    if file.seek(SeekFrom::Start(0)).is_err() {
        return (Vec::new(), 0, true);
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return (Vec::new(), 0, true);
    }
    let mut window: VecDeque<TranscriptEntry> = VecDeque::new();
    let mut line = Vec::new();
    let mut rel = 0usize;
    let mut consumed = 0usize;
    while let Some(off) = buf[rel..].iter().position(|&b| b == b'\n') {
        let line_end = rel + off;
        if let Ok(text) = std::str::from_utf8(&buf[rel..line_end]) {
            line.clear();
            parse_into(&mut line, text, rel as u64);
            for entry in line.drain(..) {
                window.push_back(entry);
                while window.len() > max_entries {
                    window.pop_front();
                }
            }
        }
        rel = line_end + 1;
        consumed = rel;
    }
    (window.into_iter().collect(), consumed as u64, true)
}

#[derive(Clone, Serialize)]
struct TranscriptPayload {
    session_id: SessionId,
    agent_id: String,
    entries: Vec<TranscriptEntry>,
    cursor: u64,
}

pub struct WatchSpec {
    pub agent_id: String,
    pub agent_type: String,
    pub description: Option<String>,
    pub parent_path: Option<PathBuf>,
    pub path: Option<PathBuf>,
}

#[derive(Clone)]
struct WatchTarget {
    agent_id: String,
    agent_type: String,
    description: Option<String>,
    parent_path: Option<PathBuf>,
    path: Option<PathBuf>,
    cursor: Option<u64>,
    last_len: Option<u64>,
    stable_cycles: u32,
    finished: bool,
}

impl WatchTarget {
    fn from_spec(spec: &WatchSpec) -> Self {
        Self {
            agent_id: spec.agent_id.clone(),
            agent_type: spec.agent_type.clone(),
            description: spec.description.clone(),
            parent_path: spec.parent_path.clone(),
            path: spec.path.clone(),
            cursor: None,
            last_len: None,
            stable_cycles: 0,
            finished: false,
        }
    }

    fn resolve(&self) -> Option<PathBuf> {
        crate::agent::subagents::resolve_transcript(
            self.parent_path.as_deref(),
            &self.agent_id,
            &self.agent_type,
            self.description.as_deref(),
        )
    }
}

/// Lê a cauda do `agent-<id>.jsonl` e devolve `Some(resumo)` só quando a última
/// entrada relevante (assistant/user) é a **resposta final** do subagente: um
/// `assistant` com texto e sem `tool_use` pendente. `user` no fim (tool_result
/// ou interrupção) ou `assistant` de tool_use ⇒ `None` — ainda não terminou.
/// Parser tolerante: formato inesperado degrada para `None`, nunca panica.
pub(crate) fn final_assistant_summary(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(FINAL_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return None;
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return None;
    }
    let text = String::from_utf8_lossy(&buf);
    let tail = if start > 0 {
        text.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
    } else {
        text.as_ref()
    };
    for line in tail.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("assistant") => return final_assistant_text(&v),
            Some("user") => return None,
            _ => continue,
        }
    }
    None
}

fn final_assistant_text(v: &Value) -> Option<String> {
    if v.get("message")
        .and_then(|m| m.get("stop_reason"))
        .and_then(Value::as_str)
        == Some("tool_use")
    {
        return None;
    }
    let content = v.get("message").and_then(|m| m.get("content"))?;
    if let Some(text) = content.as_str() {
        return clean_summary(text, FINISH_SUMMARY_MAX);
    }
    let blocks = content.as_array()?;
    if blocks
        .iter()
        .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
    {
        return None;
    }
    let mut combined = String::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                combined.push_str(text);
            }
        }
    }
    clean_summary(&combined, FINISH_SUMMARY_MAX)
}

struct WatchState {
    targets: Vec<WatchTarget>,
}

struct WatcherSlot {
    state: Arc<Mutex<WatchState>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

struct PollResult {
    agent_id: String,
    path: PathBuf,
    cursor: u64,
    entries: Vec<TranscriptEntry>,
    file_len: Option<u64>,
}

#[derive(Default)]
pub(crate) struct TranscriptWatchers {
    sessions: Mutex<std::collections::HashMap<SessionId, WatcherSlot>>,
}

impl TranscriptWatchers {
    pub(crate) fn sync<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        active: Vec<WatchSpec>,
        sessions: SharedSessions,
        coordinator: SharedCoordinator,
    ) {
        if active.is_empty() {
            self.stop_session(app, session);
            return;
        }
        let state = {
            let mut map = self.sessions.lock().expect("watchers lock");
            if map
                .get(&session)
                .map(|slot| !slot.running.load(Ordering::SeqCst))
                .unwrap_or(false)
            {
                map.remove(&session);
            }
            if let Some(slot) = map.get(&session) {
                Arc::clone(&slot.state)
            } else {
                let slot = spawn_slot(app, session, &active, sessions, coordinator);
                map.insert(session, slot);
                return;
            }
        };
        reconcile(app, session, &state, &active);
    }

    pub(crate) fn stop_session<R: Runtime>(&self, app: &AppHandle<R>, session: SessionId) {
        let Some(slot) = self
            .sessions
            .lock()
            .expect("watchers lock")
            .remove(&session)
        else {
            return;
        };
        slot.stop.store(true, Ordering::SeqCst);
        let targets = std::mem::take(&mut slot.state.lock().expect("watch state").targets);
        for target in targets {
            emit_final(app, session, target);
        }
    }

    #[cfg(test)]
    fn active_sessions(&self) -> usize {
        self.sessions.lock().expect("watchers lock").len()
    }
}

fn reconcile<R: Runtime>(
    app: &AppHandle<R>,
    session: SessionId,
    state: &Arc<Mutex<WatchState>>,
    active: &[WatchSpec],
) {
    let removed = {
        let mut guard = state.lock().expect("watch state");
        let active_ids: HashSet<&str> = active.iter().map(|spec| spec.agent_id.as_str()).collect();
        let (kept, removed): (Vec<WatchTarget>, Vec<WatchTarget>) =
            std::mem::take(&mut guard.targets)
                .into_iter()
                .partition(|target| active_ids.contains(target.agent_id.as_str()));
        guard.targets = kept;
        for spec in active {
            if !guard.targets.iter().any(|t| t.agent_id == spec.agent_id) {
                guard.targets.push(WatchTarget::from_spec(spec));
            }
        }
        removed
    };
    for target in removed {
        emit_final(app, session, target);
    }
}

fn spawn_slot<R: Runtime>(
    app: &AppHandle<R>,
    session: SessionId,
    active: &[WatchSpec],
    sessions: SharedSessions,
    coordinator: SharedCoordinator,
) -> WatcherSlot {
    let state = Arc::new(Mutex::new(WatchState {
        targets: active.iter().map(WatchTarget::from_spec).collect(),
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let running = Arc::new(AtomicBool::new(true));
    let spawned = {
        let state = Arc::clone(&state);
        let stop = Arc::clone(&stop);
        let running = Arc::clone(&running);
        let app = app.clone();
        std::thread::Builder::new()
            .name("subagent-transcript".into())
            .spawn(move || {
                watch_loop(&app, session, &state, &stop, &sessions, &coordinator);
                running.store(false, Ordering::SeqCst);
            })
    };
    if spawned.is_err() {
        running.store(false, Ordering::SeqCst);
    }
    WatcherSlot {
        state,
        stop,
        running,
    }
}

struct Completion {
    agent_id: String,
    summary: Option<String>,
}

fn watch_loop<R: Runtime>(
    app: &AppHandle<R>,
    session: SessionId,
    state: &Arc<Mutex<WatchState>>,
    stop: &Arc<AtomicBool>,
    sessions: &SharedSessions,
    coordinator: &SharedCoordinator,
) {
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let polls: Vec<WatchTarget> = {
            let Ok(guard) = state.lock() else {
                break;
            };
            if guard.targets.is_empty() {
                break;
            }
            guard
                .targets
                .iter()
                .filter(|t| !t.finished)
                .cloned()
                .collect()
        };
        let mut results: Vec<PollResult> = Vec::new();
        for poll in &polls {
            let Some(path) = poll.path.clone().or_else(|| poll.resolve()) else {
                continue;
            };
            let (entries, next, _complete) = read_entries(&path, poll.cursor, TAIL_ENTRIES);
            let file_len = std::fs::metadata(&path).map(|m| m.len()).ok();
            results.push(PollResult {
                agent_id: poll.agent_id.clone(),
                path,
                cursor: next,
                entries,
                file_len,
            });
        }
        let (to_emit, completions) = {
            let Ok(mut guard) = state.lock() else {
                break;
            };
            let mut emit = Vec::new();
            let mut done = Vec::new();
            for result in results {
                let Some(target) = guard
                    .targets
                    .iter_mut()
                    .find(|t| t.agent_id == result.agent_id)
                else {
                    continue;
                };
                if target.path.is_none() {
                    target.path = Some(result.path.clone());
                }
                target.cursor = Some(result.cursor);
                if result.file_len.is_some() && result.file_len == target.last_len {
                    target.stable_cycles = target.stable_cycles.saturating_add(1);
                } else {
                    target.last_len = result.file_len;
                    target.stable_cycles = 0;
                }
                if !target.finished && target.stable_cycles >= STABLE_CYCLES {
                    if let Some(summary) = final_assistant_summary(&result.path) {
                        target.finished = true;
                        done.push(Completion {
                            agent_id: target.agent_id.clone(),
                            summary: Some(summary),
                        });
                    }
                }
                emit.push(result);
            }
            (emit, done)
        };
        for result in to_emit {
            if !result.entries.is_empty() {
                emit_transcript(
                    app,
                    session,
                    &result.agent_id,
                    result.entries,
                    result.cursor,
                );
            }
        }
        for completion in completions {
            finish_target(app, session, sessions, coordinator, completion);
        }
        let all_finished = {
            match state.lock() {
                Ok(guard) => !guard.targets.is_empty() && guard.targets.iter().all(|t| t.finished),
                Err(_) => break,
            }
        };
        if all_finished {
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn finish_target<R: Runtime>(
    app: &AppHandle<R>,
    session: SessionId,
    sessions: &SharedSessions,
    coordinator: &SharedCoordinator,
    completion: Completion,
) {
    let Some((snapshot, idle)) = finish_by_file(
        sessions,
        session,
        &completion.agent_id,
        completion.summary,
        now_ms(),
    ) else {
        return;
    };
    crate::agent::subagents::emit_snapshot(app, session, snapshot);
    if let Some(summary) = idle {
        let coordinator = coordinator
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned());
        if let Some(coordinator) = coordinator {
            coordinator(session, summary);
        }
    }
}

fn emit_final<R: Runtime>(app: &AppHandle<R>, session: SessionId, target: WatchTarget) {
    let Some(path) = target.path.clone().or_else(|| target.resolve()) else {
        return;
    };
    let mut cursor = target.cursor;
    loop {
        let (entries, next, complete) = read_entries(&path, cursor, TAIL_ENTRIES);
        let progressed = Some(next) != cursor || !entries.is_empty();
        if !entries.is_empty() {
            emit_transcript(app, session, &target.agent_id, entries, next);
        }
        cursor = Some(next);
        if complete || !progressed {
            break;
        }
    }
}

fn emit_transcript<R: Runtime>(
    app: &AppHandle<R>,
    session: SessionId,
    agent_id: &str,
    entries: Vec<TranscriptEntry>,
    cursor: u64,
) {
    let _ = app.emit(
        EVENT_SUBAGENT_TRANSCRIPT,
        TranscriptPayload {
            session_id: session,
            agent_id: agent_id.to_string(),
            entries,
            cursor,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn parse_line(line: &str, line_offset: u64) -> Vec<TranscriptEntry> {
        let mut out = Vec::new();
        parse_into(&mut out, line, line_offset);
        out
    }

    #[test]
    fn parses_simple_assistant_text() {
        let line =
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"olá mundo"}]}}"#;
        let entries = parse_line(line, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, EntryKind::AssistantText);
        assert_eq!(entries[0].text, "olá mundo");
        assert!(entries[0].tool_name.is_none());
    }

    #[test]
    fn concatenates_multi_block_assistant_text() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Parte um "},{"type":"text","text":"parte dois"}]}}"#;
        let entries = parse_line(line, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, EntryKind::AssistantText);
        assert_eq!(entries[0].text, "Parte um parte dois");
    }

    #[test]
    fn assistant_text_and_tool_use_in_one_message() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"vou rodar"},{"type":"tool_use","name":"Bash","input":{"command":"ls -la"}}]}}"#;
        let entries = parse_line(line, 100);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, EntryKind::AssistantText);
        assert_eq!(entries[0].seq, 100);
        assert_eq!(entries[1].kind, EntryKind::ToolUse);
        assert_eq!(entries[1].seq, 101);
    }

    #[test]
    fn tool_use_bash_uses_command_summary() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test --all"}}]}}"#;
        let entries = parse_line(line, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, EntryKind::ToolUse);
        assert_eq!(entries[0].tool_name.as_deref(), Some("Bash"));
        assert_eq!(entries[0].text, "cargo test --all");
    }

    #[test]
    fn tool_use_write_falls_back_to_file_path() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/repo/src/main.rs","content":"x"}}]}}"#;
        let entries = parse_line(line, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tool_name.as_deref(), Some("Write"));
        assert_eq!(entries[0].text, "/repo/src/main.rs");
    }

    #[test]
    fn tool_use_without_input_uses_bare_name() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"TodoWrite"}]}}"#;
        let entries = parse_line(line, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].tool_name.as_deref(), Some("TodoWrite"));
        assert_eq!(entries[0].text, "TodoWrite");
    }

    #[test]
    fn tool_result_text_is_capped() {
        let content = "z".repeat(900);
        let line = format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"toolu_1","content":[{{"type":"text","text":"{content}"}}]}}]}}}}"#
        );
        let entries = parse_line(&line, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, EntryKind::ToolResult);
        assert!(entries[0].text.chars().count() <= TOOL_RESULT_MAX);
        assert!(entries[0].text.ends_with('…'));
    }

    #[test]
    fn tool_result_string_content_is_extracted() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"saída direta"}]}}"#;
        let entries = parse_line(line, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, EntryKind::ToolResult);
        assert_eq!(entries[0].text, "saída direta");
    }

    #[test]
    fn first_user_line_is_the_prompt() {
        let line = r#"{"type":"user","parentUuid":null,"message":{"content":"revise o diff e aponte bugs"}}"#;
        let entries = parse_line(line, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, EntryKind::UserText);
        assert_eq!(entries[0].text, "revise o diff e aponte bugs");
    }

    #[test]
    fn attachment_lines_are_ignored() {
        let line = r#"{"type":"attachment","message":{"content":"ruído"}}"#;
        assert!(parse_line(line, 0).is_empty());
    }

    #[test]
    fn malformed_lines_are_ignored_without_panic() {
        assert!(parse_line("{corrompido", 0).is_empty());
        assert!(parse_line("", 0).is_empty());
        assert!(parse_line("   ", 0).is_empty());
        assert!(parse_line("não é json", 0).is_empty());
    }

    #[test]
    fn cap_truncation_lands_on_utf8_char_boundary() {
        let content = "á".repeat(700);
        let line = format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t","content":"{content}"}}]}}}}"#
        );
        let entries = parse_line(&line, 0);
        assert_eq!(entries.len(), 1);
        let text = &entries[0].text;
        assert!(text.chars().count() <= TOOL_RESULT_MAX);
        assert!(text.chars().all(|c| c == 'á' || c == '…'));
    }

    #[test]
    fn redaction_is_applied_to_text() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"a chave é sk-abcdef1234567890ABCDEFghijkl pronto"}]}}"#;
        let entries = parse_line(line, 0);
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].text.contains("sk-abcdef1234567890ABCDEFghijkl"));
        assert!(entries[0].text.contains("[REDACTED]"));
    }

    #[test]
    fn redaction_is_applied_to_tool_summary() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"export TOKEN=sk-abcdef1234567890ABCDEFghijkl"}}]}}"#;
        let entries = parse_line(line, 0);
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].text.contains("sk-abcdef1234567890ABCDEFghijkl"));
        assert!(entries[0].text.contains("[REDACTED]"));
    }

    fn write(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
    }

    #[test]
    fn incremental_cursor_advances_across_reads() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("agent.jsonl");
        write(
            &path,
            "{\"type\":\"user\",\"message\":{\"content\":\"prompt\"}}\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"primeira\"}]}}\n",
        );
        let (entries, cursor, complete) = read_entries(&path, None, TAIL_ENTRIES);
        assert_eq!(entries.len(), 2);
        assert!(complete);

        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut f| {
                use std::io::Write;
                f.write_all(
                    b"{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"segunda\"}]}}\n",
                )
            })
            .unwrap();

        let (more, next, complete) = read_entries(&path, Some(cursor), TAIL_ENTRIES);
        assert_eq!(more.len(), 1);
        assert_eq!(more[0].text, "segunda");
        assert!(next > cursor);
        assert!(complete);
    }

    #[test]
    fn partial_trailing_line_is_not_consumed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("agent.jsonl");
        write(
            &path,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}\n{\"type\":\"assistant\",\"message\"",
        );
        let (entries, cursor, complete) = read_entries(&path, None, TAIL_ENTRIES);
        assert_eq!(entries.len(), 1);
        assert!(complete);

        let (empty, next, _) = read_entries(&path, Some(cursor), TAIL_ENTRIES);
        assert!(empty.is_empty());
        assert_eq!(next, cursor);

        fs::write(
            &path,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"segunda\"}]}}\n",
        )
        .unwrap();
        let (more, _, _) = read_entries(&path, Some(cursor), TAIL_ENTRIES);
        assert_eq!(more.len(), 1);
        assert_eq!(more[0].text, "segunda");
    }

    #[test]
    fn truncated_or_recreated_file_restarts_from_zero() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("agent.jsonl");
        let big = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"linha\"}]}}\n".repeat(10);
        write(&path, &big);
        let (_entries, cursor, _) = read_entries(&path, None, TAIL_ENTRIES);
        assert!(cursor > 0);

        write(
            &path,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"nova\"}]}}\n",
        );
        let (entries, next, complete) = read_entries(&path, Some(cursor), TAIL_ENTRIES);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "nova");
        assert!(next < cursor);
        assert!(complete);
    }

    #[test]
    fn initial_load_caps_at_max_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("agent.jsonl");
        let mut body = String::new();
        for i in 0..(TAIL_ENTRIES + 100) {
            body.push_str(&format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"linha {i}\"}}]}}}}\n"
            ));
        }
        write(&path, &body);
        let (entries, cursor, complete) = read_entries(&path, None, TAIL_ENTRIES);
        assert_eq!(entries.len(), TAIL_ENTRIES);
        assert!(complete);
        assert_eq!(cursor, body.len() as u64);
        assert_eq!(entries.last().unwrap().text, "linha 599");
        let (empty, next, _) = read_entries(&path, Some(cursor), TAIL_ENTRIES);
        assert!(empty.is_empty());
        assert_eq!(next, cursor);
    }

    #[test]
    fn incremental_batch_respects_max_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("agent.jsonl");
        let mut body = String::new();
        for i in 0..5 {
            body.push_str(&format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"n{i}\"}}]}}}}\n"
            ));
        }
        write(&path, &body);
        let (entries, cursor, complete) = read_entries(&path, Some(0), 2);
        assert_eq!(entries.len(), 2);
        assert!(!complete);
        let (rest, _, complete) = read_entries(&path, Some(cursor), 10);
        assert_eq!(rest.len(), 3);
        assert!(complete);
    }

    #[test]
    fn missing_file_never_panics() {
        let (entries, cursor, complete) =
            read_entries(Path::new("/nope/agent.jsonl"), Some(42), TAIL_ENTRIES);
        assert!(entries.is_empty());
        assert_eq!(cursor, 42);
        assert!(complete);
    }

    #[test]
    fn watcher_slot_lifecycle_is_deterministic() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let watchers = TranscriptWatchers::default();
        let session = SessionId::new_v4();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("agent.jsonl");
        write(
            &path,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"oi\"}]}}\n",
        );

        watchers.sync(
            &handle,
            session,
            vec![WatchSpec {
                agent_id: "a1".into(),
                agent_type: "reviewer".into(),
                description: None,
                parent_path: None,
                path: Some(path),
            }],
            Arc::new(Mutex::new(std::collections::HashMap::new())),
            Arc::new(Mutex::new(None)),
        );
        assert_eq!(watchers.active_sessions(), 1);
        watchers.stop_session(&handle, session);
        assert_eq!(watchers.active_sessions(), 0);
    }
}
