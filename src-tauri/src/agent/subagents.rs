use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Runtime};

use crate::approvals::now_ms;
use crate::session::redact::redact;
use crate::session::SessionId;
use crate::status::transcript::clean_summary;

pub type SharedSubagents = Arc<SubagentTracker>;

pub const EVENT_SUBAGENTS_CHANGED: &str = "subagents://changed";

const SUMMARY_MAX_CHARS: usize = 280;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Starting,
    Running,
    Done,
}

#[derive(Debug, Clone)]
pub struct SubagentRun {
    pub agent_id: Option<String>,
    pub agent_type: String,
    pub description: String,
    pub status: SubagentStatus,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub summary: Option<String>,
    pub transcript_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubagentRunDto {
    pub agent_id: Option<String>,
    pub agent_type: String,
    pub description: String,
    pub status: SubagentStatus,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubagentSnapshot {
    pub focused: Option<String>,
    pub subagents: Vec<SubagentRunDto>,
}

#[derive(Clone, Serialize)]
struct SubagentsChangedPayload {
    session_id: SessionId,
    focused: Option<String>,
    subagents: Vec<SubagentRunDto>,
}

impl SubagentRunDto {
    fn redacted(run: &SubagentRun) -> Self {
        Self {
            agent_id: run.agent_id.clone(),
            agent_type: run.agent_type.clone(),
            description: redact(&run.description).into_owned(),
            status: run.status,
            started_at_ms: run.started_at_ms,
            ended_at_ms: run.ended_at_ms,
            summary: run.summary.as_deref().map(|s| redact(s).into_owned()),
        }
    }
}

pub struct Coordination {
    pub viewer_disarmed: bool,
    pub panel_disarmed: bool,
}

#[derive(Default)]
struct SessionSubagents {
    runs: Vec<SubagentRun>,
    focused: Option<String>,
    coordinated: bool,
    viewer_disarmed: bool,
    panel_disarmed: bool,
}

impl SessionSubagents {
    fn push_pending(&mut self, agent_type: Option<String>, description: Option<String>, now: u64) {
        self.runs.push(SubagentRun {
            agent_id: None,
            agent_type: agent_type.unwrap_or_default(),
            description: description.unwrap_or_default(),
            status: SubagentStatus::Starting,
            started_at_ms: now,
            ended_at_ms: None,
            summary: None,
            transcript_path: None,
        });
    }

    fn promote(
        &mut self,
        agent_id: String,
        agent_type: String,
        parent_transcript_path: Option<&Path>,
        now: u64,
    ) {
        let slot = self
            .runs
            .iter()
            .position(|run| run.status == SubagentStatus::Starting && run.agent_type == agent_type);
        match slot {
            Some(index) => {
                let wanted_description = self.runs[index].description.clone();
                let hint = (!wanted_description.is_empty()).then_some(wanted_description.as_str());
                let transcript_path =
                    resolve_transcript(parent_transcript_path, &agent_id, &agent_type, hint);
                let run = &mut self.runs[index];
                run.agent_id = Some(agent_id.clone());
                run.status = SubagentStatus::Running;
                run.transcript_path = transcript_path;
            }
            None => {
                let transcript_path =
                    resolve_transcript(parent_transcript_path, &agent_id, &agent_type, None);
                self.runs.push(SubagentRun {
                    agent_id: Some(agent_id.clone()),
                    agent_type,
                    description: String::new(),
                    status: SubagentStatus::Running,
                    started_at_ms: now,
                    ended_at_ms: None,
                    summary: None,
                    transcript_path,
                });
            }
        }
        if self.focused.is_none() {
            self.focused = Some(agent_id);
        }
    }

    fn mark_stopped(&mut self, agent_id: String, last_assistant_message: Option<String>, now: u64) {
        let summary = last_assistant_message
            .as_deref()
            .and_then(|message| clean_summary(message, SUMMARY_MAX_CHARS));
        let slot = self.runs.iter().position(|run| {
            run.agent_id.as_deref() == Some(agent_id.as_str()) && run.status != SubagentStatus::Done
        });
        match slot {
            Some(index) => {
                let run = &mut self.runs[index];
                run.status = SubagentStatus::Done;
                run.ended_at_ms = Some(now);
                run.summary = summary;
            }
            None => {
                self.runs.push(SubagentRun {
                    agent_id: Some(agent_id),
                    agent_type: String::new(),
                    description: String::new(),
                    status: SubagentStatus::Done,
                    started_at_ms: now,
                    ended_at_ms: Some(now),
                    summary,
                    transcript_path: None,
                });
            }
        }
    }

    fn end_all(&mut self, now: u64) {
        for run in &mut self.runs {
            if run.status == SubagentStatus::Running {
                run.status = SubagentStatus::Done;
                run.ended_at_ms = Some(now);
            }
        }
    }

    fn set_focus(&mut self, agent_id: String) {
        if self
            .runs
            .iter()
            .any(|run| run.agent_id.as_deref() == Some(agent_id.as_str()))
        {
            self.focused = Some(agent_id);
        }
    }

    fn to_snapshot(&self) -> SubagentSnapshot {
        let mut ordered: Vec<&SubagentRun> = self.runs.iter().collect();
        ordered.sort_by_key(|run| run.started_at_ms);
        SubagentSnapshot {
            focused: self.focused.clone(),
            subagents: ordered.into_iter().map(SubagentRunDto::redacted).collect(),
        }
    }
}

#[derive(Default)]
pub struct SubagentTracker {
    sessions: Mutex<HashMap<SessionId, SessionSubagents>>,
}

impl SubagentTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_spawn_requested<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        agent_type: Option<String>,
        description: Option<String>,
    ) {
        let payload = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let entry = sessions.entry(session).or_default();
            entry.push_pending(agent_type, description, now_ms());
            changed_payload(session, entry)
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
    }

    pub fn on_subagent_started<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        agent_id: String,
        agent_type: String,
        parent_transcript_path: Option<&Path>,
    ) -> Option<Coordination> {
        let (payload, coordination) = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let entry = sessions.entry(session).or_default();
            entry.promote(agent_id, agent_type, parent_transcript_path, now_ms());
            let coordination = (!entry.coordinated).then(|| {
                entry.coordinated = true;
                Coordination {
                    viewer_disarmed: entry.viewer_disarmed,
                    panel_disarmed: entry.panel_disarmed,
                }
            });
            (changed_payload(session, entry), coordination)
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
        coordination
    }

    pub fn disarm_viewer(&self, session: SessionId) {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        sessions.entry(session).or_default().viewer_disarmed = true;
    }

    pub fn disarm_panel(&self, session: SessionId) {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        sessions.entry(session).or_default().panel_disarmed = true;
    }

    pub fn on_subagent_stopped<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        agent_id: String,
        last_assistant_message: Option<String>,
    ) {
        let payload = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let entry = sessions.entry(session).or_default();
            entry.mark_stopped(agent_id, last_assistant_message, now_ms());
            changed_payload(session, entry)
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
    }

    pub fn on_session_ended<R: Runtime>(&self, app: &AppHandle<R>, session: SessionId) {
        let payload = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let Some(entry) = sessions.get_mut(&session) else {
                return;
            };
            entry.end_all(now_ms());
            changed_payload(session, entry)
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
    }

    pub fn focus<R: Runtime>(&self, app: &AppHandle<R>, session: SessionId, agent_id: String) {
        let payload = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let Some(entry) = sessions.get_mut(&session) else {
                return;
            };
            entry.set_focus(agent_id);
            changed_payload(session, entry)
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
    }

    pub fn remove_session(&self, session: SessionId) {
        self.sessions
            .lock()
            .expect("subagents lock")
            .remove(&session);
    }

    pub fn snapshot(&self, session: SessionId) -> SubagentSnapshot {
        self.sessions
            .lock()
            .expect("subagents lock")
            .get(&session)
            .map(SessionSubagents::to_snapshot)
            .unwrap_or_else(|| SubagentSnapshot {
                focused: None,
                subagents: Vec::new(),
            })
    }
}

fn changed_payload(session: SessionId, entry: &SessionSubagents) -> SubagentsChangedPayload {
    let snapshot = entry.to_snapshot();
    SubagentsChangedPayload {
        session_id: session,
        focused: snapshot.focused,
        subagents: snapshot.subagents,
    }
}

fn sidecar_dir(parent_transcript_path: &Path) -> PathBuf {
    parent_transcript_path.with_extension("").join("subagents")
}

fn transcript_by_agent_id(sidecar: &Path, agent_id: &str) -> Option<PathBuf> {
    let bare = agent_id.strip_prefix("agent-").unwrap_or(agent_id);
    let candidate = sidecar.join(format!("agent-{bare}.jsonl"));
    candidate.is_file().then_some(candidate)
}

fn meta_sibling_jsonl(meta_path: &Path) -> PathBuf {
    let name = meta_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let base = name.strip_suffix(".meta.json").unwrap_or(name);
    meta_path.with_file_name(format!("{base}.jsonl"))
}

struct MetaMatch {
    modified: std::time::SystemTime,
    jsonl: PathBuf,
    description_match: bool,
}

fn transcript_by_meta_scan(
    sidecar: &Path,
    agent_type: &str,
    description: Option<&str>,
) -> Option<PathBuf> {
    let mut candidates: Vec<MetaMatch> = Vec::new();
    for entry in std::fs::read_dir(sidecar).ok()?.flatten() {
        let meta_path = entry.path();
        let is_meta = meta_path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".meta.json"));
        if !is_meta {
            continue;
        }
        let Some(meta) = std::fs::read_to_string(&meta_path)
            .ok()
            .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        else {
            continue;
        };
        if meta.get("agentType").and_then(Value::as_str) != Some(agent_type) {
            continue;
        }
        let jsonl = meta_sibling_jsonl(&meta_path);
        if !jsonl.is_file() {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let description_match =
            description.is_some() && meta.get("description").and_then(Value::as_str) == description;
        candidates.push(MetaMatch {
            modified,
            jsonl,
            description_match,
        });
    }
    let refined: Vec<&MetaMatch> = candidates.iter().filter(|m| m.description_match).collect();
    let pool: Vec<&MetaMatch> = if refined.is_empty() {
        candidates.iter().collect()
    } else {
        refined
    };
    pool.into_iter()
        .max_by_key(|m| m.modified)
        .map(|m| m.jsonl.clone())
}

fn resolve_transcript(
    parent_transcript_path: Option<&Path>,
    agent_id: &str,
    agent_type: &str,
    description: Option<&str>,
) -> Option<PathBuf> {
    let sidecar = sidecar_dir(parent_transcript_path?);
    transcript_by_agent_id(&sidecar, agent_id)
        .or_else(|| transcript_by_meta_scan(&sidecar, agent_type, description))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn status_of(session: &SessionSubagents, agent_id: &str) -> Option<SubagentStatus> {
        session
            .runs
            .iter()
            .find(|run| run.agent_id.as_deref() == Some(agent_id))
            .map(|run| run.status)
    }

    #[test]
    fn spawn_request_creates_a_starting_run() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), Some("revisar".into()), 10);
        assert_eq!(session.runs.len(), 1);
        assert_eq!(session.runs[0].status, SubagentStatus::Starting);
        assert!(session.runs[0].agent_id.is_none());
        assert_eq!(session.runs[0].agent_type, "reviewer");
        assert!(session.focused.is_none());
    }

    #[test]
    fn start_promotes_oldest_compatible_pending_among_many() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), Some("primeiro".into()), 10);
        session.push_pending(Some("reviewer".into()), Some("segundo".into()), 20);
        session.push_pending(Some("explorer".into()), Some("outro tipo".into()), 30);

        session.promote("a1".into(), "reviewer".into(), None, 40);

        let promoted = session
            .runs
            .iter()
            .find(|run| run.agent_id.as_deref() == Some("a1"))
            .unwrap();
        assert_eq!(promoted.description, "primeiro");
        assert_eq!(promoted.status, SubagentStatus::Running);
        let still_pending = session
            .runs
            .iter()
            .filter(|run| run.status == SubagentStatus::Starting)
            .count();
        assert_eq!(still_pending, 2);
    }

    #[test]
    fn start_without_matching_pending_creates_running_run() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), None, 10);
        session.promote("a9".into(), "explorer".into(), None, 20);
        assert_eq!(status_of(&session, "a9"), Some(SubagentStatus::Running));
        assert_eq!(
            session
                .runs
                .iter()
                .filter(|run| run.status == SubagentStatus::Starting)
                .count(),
            1
        );
    }

    #[test]
    fn first_running_becomes_default_focus() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, 10);
        assert_eq!(session.focused.as_deref(), Some("a1"));
        session.promote("a2".into(), "explorer".into(), None, 20);
        assert_eq!(session.focused.as_deref(), Some("a1"));
    }

    #[test]
    fn focus_stays_on_agent_after_it_finishes() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, 10);
        session.promote("a2".into(), "explorer".into(), None, 20);
        session.set_focus("a2".into());
        session.mark_stopped("a2".into(), Some("pronto".into()), 30);
        assert_eq!(session.focused.as_deref(), Some("a2"));
        assert_eq!(status_of(&session, "a2"), Some(SubagentStatus::Done));
    }

    #[test]
    fn focus_ignores_unknown_agent() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, 10);
        session.set_focus("ghost".into());
        assert_eq!(session.focused.as_deref(), Some("a1"));
    }

    #[test]
    fn stop_sets_summary_from_last_assistant_message() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, 10);
        session.mark_stopped("a1".into(), Some("  achei   dois bugs ".into()), 20);
        let run = &session.runs[0];
        assert_eq!(run.status, SubagentStatus::Done);
        assert_eq!(run.ended_at_ms, Some(20));
        assert_eq!(run.summary.as_deref(), Some("achei dois bugs"));
    }

    #[test]
    fn stop_summary_truncates_at_280_chars() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, 10);
        session.mark_stopped("a1".into(), Some("x".repeat(400)), 20);
        assert!(session.runs[0].summary.as_ref().unwrap().chars().count() <= 280);
    }

    #[test]
    fn stop_of_unknown_agent_creates_orphan_done_run() {
        let mut session = SessionSubagents::default();
        session.mark_stopped("aX".into(), Some("resumo".into()), 50);
        assert_eq!(session.runs.len(), 1);
        let run = &session.runs[0];
        assert_eq!(run.agent_id.as_deref(), Some("aX"));
        assert_eq!(run.status, SubagentStatus::Done);
        assert_eq!(run.summary.as_deref(), Some("resumo"));
        assert!(session.focused.is_none());
    }

    #[test]
    fn session_end_marks_running_as_done_without_summary() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, 10);
        session.push_pending(Some("explorer".into()), None, 20);
        session.end_all(99);
        let running_done = session
            .runs
            .iter()
            .find(|run| run.agent_id.as_deref() == Some("a1"))
            .unwrap();
        assert_eq!(running_done.status, SubagentStatus::Done);
        assert_eq!(running_done.ended_at_ms, Some(99));
        assert!(running_done.summary.is_none());
        assert!(session
            .runs
            .iter()
            .any(|run| run.status == SubagentStatus::Starting));
    }

    #[test]
    fn snapshot_orders_by_started_at_and_redacts() {
        let mut session = SessionSubagents::default();
        session.push_pending(
            Some("reviewer".into()),
            Some("token sk-abcdef1234567890ABCDEFghijkl aqui".into()),
            30,
        );
        session.push_pending(Some("explorer".into()), Some("primeiro".into()), 10);
        let snapshot = session.to_snapshot();
        assert_eq!(snapshot.subagents.len(), 2);
        assert_eq!(snapshot.subagents[0].started_at_ms, 10);
        assert_eq!(snapshot.subagents[1].started_at_ms, 30);
        assert!(!snapshot.subagents[1]
            .description
            .contains("sk-abcdef1234567890ABCDEFghijkl"));
        assert!(snapshot.subagents[1].description.contains("[REDACTED]"));
    }

    #[test]
    fn sidecar_dir_strips_jsonl_and_joins_subagents() {
        let parent = Path::new("/home/x/.claude/projects/slug/session-42.jsonl");
        assert_eq!(
            sidecar_dir(parent),
            Path::new("/home/x/.claude/projects/slug/session-42/subagents")
        );
    }

    fn write(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
    }

    #[test]
    fn resolve_finds_file_by_agent_id_with_and_without_prefix() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");
        let sidecar = dir.path().join("session").join("subagents");
        fs::create_dir_all(&sidecar).unwrap();
        let jsonl = sidecar.join("agent-a17hex.jsonl");
        write(&jsonl, "{}");

        assert_eq!(
            resolve_transcript(Some(&parent), "a17hex", "reviewer", None),
            Some(jsonl.clone())
        );
        assert_eq!(
            resolve_transcript(Some(&parent), "agent-a17hex", "reviewer", None),
            Some(jsonl)
        );
    }

    #[test]
    fn resolve_falls_back_to_meta_scan_matching_agent_type() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");
        let sidecar = dir.path().join("session").join("subagents");
        fs::create_dir_all(&sidecar).unwrap();

        write(
            &sidecar.join("agent-other.meta.json"),
            r#"{"agentType":"explorer","description":"outro"}"#,
        );
        write(&sidecar.join("agent-other.jsonl"), "{}");
        write(
            &sidecar.join("agent-match.meta.json"),
            r#"{"agentType":"reviewer","description":"revisar diff"}"#,
        );
        let wanted = sidecar.join("agent-match.jsonl");
        write(&wanted, "{}");

        let resolved = resolve_transcript(
            Some(&parent),
            "hook-id-not-a-file",
            "reviewer",
            Some("revisar diff"),
        );
        assert_eq!(resolved, Some(wanted));
    }

    #[test]
    fn resolve_returns_none_when_sidecar_missing() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");
        assert_eq!(
            resolve_transcript(Some(&parent), "aX", "reviewer", None),
            None
        );
    }

    #[test]
    fn resolve_returns_none_without_parent_path() {
        assert_eq!(resolve_transcript(None, "aX", "reviewer", None), None);
    }

    #[test]
    fn promote_resolves_transcript_path_for_running_run() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");
        let sidecar = dir.path().join("session").join("subagents");
        fs::create_dir_all(&sidecar).unwrap();
        let jsonl = sidecar.join("agent-a1.jsonl");
        write(&jsonl, "{}");

        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), Some("revisar".into()), 10);
        session.promote("a1".into(), "reviewer".into(), Some(&parent), 20);
        assert_eq!(session.runs[0].transcript_path, Some(jsonl));
    }

    #[test]
    fn tracker_emits_and_snapshots_across_transitions() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.on_spawn_requested(
            &handle,
            session,
            Some("reviewer".into()),
            Some("revisar".into()),
        );
        tracker.on_subagent_started(&handle, session, "a1".into(), "reviewer".into(), None);
        let snapshot = tracker.snapshot(session);
        assert_eq!(snapshot.focused.as_deref(), Some("a1"));
        assert_eq!(snapshot.subagents.len(), 1);
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Running);

        tracker.on_session_ended(&handle, session);
        assert_eq!(
            tracker.snapshot(session).subagents[0].status,
            SubagentStatus::Done
        );

        tracker.remove_session(session);
        assert!(tracker.snapshot(session).subagents.is_empty());
    }
}
