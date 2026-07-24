use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Runtime};

use crate::approvals::now_ms;
use crate::session::redact::redact;
use crate::session::SessionId;
use crate::status::subagent_transcript::{TranscriptWatchers, WatchSpec};
use crate::status::transcript::clean_summary;

pub type SharedSubagents = Arc<SubagentTracker>;

/// Fecha o turno da sessão em `Idle` quando o último subagente async termina fora
/// do fluxo de hooks. Recebe a sessão e o resumo do turno do orquestrador.
pub type IdleCoordinator = Arc<dyn Fn(SessionId, Option<String>) + Send + Sync>;

pub(crate) type SharedSessions = Arc<Mutex<HashMap<SessionId, SessionSubagents>>>;
pub(crate) type SharedCoordinator = Arc<Mutex<Option<IdleCoordinator>>>;

pub const EVENT_SUBAGENTS_CHANGED: &str = "subagents://changed";

const SUMMARY_MAX_CHARS: usize = 280;

const MIN_SUBAGENT_ID_LEN: usize = 8;

/// Um `agent_id` real de subagente do Claude Code é hexadecimal (17 chars nesta
/// máquina). Item de fila / prompt enfileirado não carrega um id assim — o gate
/// evita transformar stop espúrio em entrada fantasma no painel.
pub(crate) fn is_plausible_subagent_id(agent_id: &str) -> bool {
    agent_id.len() >= MIN_SUBAGENT_ID_LEN && agent_id.chars().all(|c| c.is_ascii_hexdigit())
}

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
    pub interrupted: bool,
    pub transcript_path: Option<PathBuf>,
    pub parent_transcript_path: Option<PathBuf>,
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
    pub interrupted: bool,
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
            agent_type: redact(&run.agent_type).into_owned(),
            description: redact(&run.description).into_owned(),
            status: run.status,
            started_at_ms: run.started_at_ms,
            ended_at_ms: run.ended_at_ms,
            summary: run.summary.as_deref().map(|s| redact(s).into_owned()),
            interrupted: run.interrupted,
        }
    }
}

pub struct Coordination {
    pub viewer_disarmed: bool,
    pub panel_disarmed: bool,
}

#[derive(Default)]
pub(crate) struct SessionSubagents {
    runs: Vec<SubagentRun>,
    focused: Option<String>,
    coordinated: bool,
    viewer_disarmed: bool,
    panel_disarmed: bool,
    orchestrator_idle: bool,
    orchestrator_idle_summary: Option<String>,
}

impl SessionSubagents {
    /// Rodada nova (primeiro subagente chegando com TODOS os anteriores Done)
    /// re-arma a coordenação — o painel/viewer voltam a auto-abrir — e limpa os
    /// nós da rodada anterior: o painel representa A RODADA, não o histórico
    /// (que vive no transcript). Sem o re-arm, o coordinate-once valia pra
    /// sessão inteira e as rodadas seguintes rodavam às cegas; sem a limpeza,
    /// as rodadas se empilhavam numa lista confusa (E2E de 2026-07-24).
    fn rearm_for_new_round(&mut self) {
        let round_settled = !self.runs.is_empty()
            && self
                .runs
                .iter()
                .all(|run| run.status == SubagentStatus::Done);
        if round_settled {
            self.runs.clear();
            self.focused = None;
            self.coordinated = false;
            self.viewer_disarmed = false;
            self.panel_disarmed = false;
            self.orchestrator_idle = false;
            self.orchestrator_idle_summary = None;
        }
    }

    fn push_pending(&mut self, agent_type: Option<String>, description: Option<String>, now: u64) {
        self.rearm_for_new_round();
        self.runs.push(SubagentRun {
            agent_id: None,
            agent_type: agent_type.unwrap_or_default(),
            description: description.unwrap_or_default(),
            status: SubagentStatus::Starting,
            started_at_ms: now,
            ended_at_ms: None,
            summary: None,
            interrupted: false,
            transcript_path: None,
            parent_transcript_path: None,
        });
    }

    fn pending_hint(&self, agent_type: &str) -> Option<String> {
        self.runs
            .iter()
            .find(|run| run.status == SubagentStatus::Starting && run.agent_type == agent_type)
            .map(|run| run.description.clone())
            .filter(|description| !description.is_empty())
    }

    fn promote(
        &mut self,
        agent_id: String,
        agent_type: String,
        description: Option<String>,
        transcript_path: Option<PathBuf>,
        parent_transcript_path: Option<&Path>,
        now: u64,
    ) {
        self.rearm_for_new_round();
        let slot = self
            .runs
            .iter()
            .position(|run| run.status == SubagentStatus::Starting && run.agent_type == agent_type);
        match slot {
            Some(index) => {
                let run = &mut self.runs[index];
                run.agent_id = Some(agent_id.clone());
                run.status = SubagentStatus::Running;
                run.transcript_path = transcript_path;
                run.parent_transcript_path = parent_transcript_path.map(Path::to_path_buf);
            }
            None => {
                self.runs.push(SubagentRun {
                    agent_id: Some(agent_id.clone()),
                    agent_type,
                    description: description.unwrap_or_default(),
                    status: SubagentStatus::Running,
                    started_at_ms: now,
                    ended_at_ms: None,
                    summary: None,
                    interrupted: false,
                    transcript_path,
                    parent_transcript_path: parent_transcript_path.map(Path::to_path_buf),
                });
            }
        }
        if self.focused.is_none() {
            self.focused = Some(agent_id);
        }
    }

    fn remove_pending(&mut self, agent_type: Option<String>, description: Option<String>) {
        let wanted_type = agent_type.unwrap_or_default();
        let wanted_description = description.unwrap_or_default();
        let exact = (!wanted_description.is_empty())
            .then(|| {
                self.runs.iter().rposition(|run| {
                    run.status == SubagentStatus::Starting
                        && run.agent_type == wanted_type
                        && run.description == wanted_description
                })
            })
            .flatten();
        let slot = exact.or_else(|| {
            self.runs.iter().rposition(|run| {
                run.status == SubagentStatus::Starting && run.agent_type == wanted_type
            })
        });
        if let Some(index) = slot {
            self.runs.remove(index);
        }
    }

    fn mark_stopped(
        &mut self,
        agent_id: String,
        agent_type: Option<String>,
        last_assistant_message: Option<String>,
        now: u64,
    ) -> bool {
        let summary = last_assistant_message
            .as_deref()
            .and_then(|message| clean_summary(message, SUMMARY_MAX_CHARS));
        let active = self.runs.iter().position(|run| {
            run.agent_id.as_deref() == Some(agent_id.as_str()) && run.status != SubagentStatus::Done
        });
        if let Some(index) = active {
            let run = &mut self.runs[index];
            run.status = SubagentStatus::Done;
            run.ended_at_ms = Some(now);
            run.summary = summary;
            return true;
        }
        let known = self
            .runs
            .iter()
            .any(|run| run.agent_id.as_deref() == Some(agent_id.as_str()));
        if known {
            return false;
        }
        let plausible_type = agent_type
            .as_deref()
            .map(|t| !t.trim().is_empty())
            .unwrap_or(false);
        if !is_plausible_subagent_id(&agent_id) || !plausible_type {
            return false;
        }
        self.runs.push(SubagentRun {
            agent_id: Some(agent_id),
            agent_type: agent_type.unwrap_or_default(),
            description: String::new(),
            status: SubagentStatus::Done,
            started_at_ms: now,
            ended_at_ms: Some(now),
            summary,
            interrupted: false,
            transcript_path: None,
            parent_transcript_path: None,
        });
        true
    }

    fn active_count(&self) -> usize {
        self.runs
            .iter()
            .filter(|run| {
                matches!(
                    run.status,
                    SubagentStatus::Starting | SubagentStatus::Running
                )
            })
            .count()
    }

    /// Marca `Done` um subagente `Running` cuja conclusão foi detectada pelo
    /// arquivo (EOF estável + última fala assistant). Não toca em `Starting`
    /// (ainda sem `agent_id`) nem em runs já `Done`.
    fn finish_running_by_file(
        &mut self,
        agent_id: &str,
        summary: Option<String>,
        now: u64,
    ) -> bool {
        let slot = self.runs.iter().position(|run| {
            run.agent_id.as_deref() == Some(agent_id) && run.status == SubagentStatus::Running
        });
        let Some(index) = slot else {
            return false;
        };
        let run = &mut self.runs[index];
        run.status = SubagentStatus::Done;
        run.ended_at_ms = Some(now);
        run.summary = summary.and_then(|s| clean_summary(&s, SUMMARY_MAX_CHARS));
        true
    }

    fn finish_running_interrupted(&mut self, agent_id: &str, now: u64) -> bool {
        let slot = self.runs.iter().position(|run| {
            run.agent_id.as_deref() == Some(agent_id) && run.status == SubagentStatus::Running
        });
        let Some(index) = slot else {
            return false;
        };
        let run = &mut self.runs[index];
        run.status = SubagentStatus::Done;
        run.ended_at_ms = Some(now);
        run.interrupted = true;
        true
    }

    /// `Some(summary)` quando o orquestrador já fechou o turno e não há mais
    /// subagente ativo — a sessão pode descer para `Idle`. `None` mantém como
    /// está.
    fn idle_transition(&self) -> Option<Option<String>> {
        (self.orchestrator_idle && self.active_count() == 0)
            .then(|| self.orchestrator_idle_summary.clone())
    }

    fn end_all(&mut self, now: u64) {
        self.runs
            .retain(|run| run.status != SubagentStatus::Starting);
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

    fn running_targets(&self) -> Vec<WatchSpec> {
        self.runs
            .iter()
            .filter(|run| run.status == SubagentStatus::Running)
            .filter_map(|run| {
                Some(WatchSpec {
                    agent_id: run.agent_id.clone()?,
                    agent_type: run.agent_type.clone(),
                    description: (!run.description.is_empty()).then(|| run.description.clone()),
                    parent_path: run.parent_transcript_path.clone(),
                    path: run.transcript_path.clone(),
                })
            })
            .collect()
    }
}

#[derive(Default)]
pub struct SubagentTracker {
    sessions: SharedSessions,
    watchers: TranscriptWatchers,
    coordinator: SharedCoordinator,
}

impl SubagentTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_idle_coordinator(&self, coordinator: IdleCoordinator) {
        *self.coordinator.lock().expect("subagents coordinator lock") = Some(coordinator);
    }

    pub fn register_session(&self, session: SessionId) {
        self.sessions
            .lock()
            .expect("subagents lock")
            .entry(session)
            .or_default();
    }

    pub fn note_orchestrator_working(&self, session: SessionId) {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        if let Some(entry) = sessions.get_mut(&session) {
            entry.orchestrator_idle = false;
            entry.orchestrator_idle_summary = None;
        }
    }

    pub fn note_orchestrator_idle(&self, session: SessionId, summary: Option<String>) {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        if let Some(entry) = sessions.get_mut(&session) {
            entry.orchestrator_idle = true;
            entry.orchestrator_idle_summary = summary;
        }
    }

    pub fn has_active(&self, session: SessionId) -> bool {
        self.sessions
            .lock()
            .expect("subagents lock")
            .get(&session)
            .map(|entry| entry.active_count() > 0)
            .unwrap_or(false)
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
            let Some(entry) = sessions.get_mut(&session) else {
                return;
            };
            entry.push_pending(agent_type, description, now_ms());
            changed_payload(session, entry)
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
    }

    pub fn on_spawn_denied<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        agent_type: Option<String>,
        description: Option<String>,
    ) {
        let payload = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let Some(entry) = sessions.get_mut(&session) else {
                return;
            };
            entry.remove_pending(agent_type, description);
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
        description: Option<String>,
        parent_transcript_path: Option<&Path>,
    ) -> Option<Coordination> {
        let agent_id = strip_agent_prefix(&agent_id);
        let hint = {
            let sessions = self.sessions.lock().expect("subagents lock");
            sessions
                .get(&session)
                .and_then(|entry| entry.pending_hint(&agent_type))
                .or_else(|| description.clone())
        };
        let transcript_path = resolve_transcript(
            parent_transcript_path,
            &agent_id,
            &agent_type,
            hint.as_deref(),
        );
        let (payload, targets, coordination) = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let entry = sessions.get_mut(&session)?;
            entry.promote(
                agent_id,
                agent_type,
                description,
                transcript_path,
                parent_transcript_path,
                now_ms(),
            );
            let coordination = (!entry.coordinated).then_some(Coordination {
                viewer_disarmed: entry.viewer_disarmed,
                panel_disarmed: entry.panel_disarmed,
            });
            (
                changed_payload(session, entry),
                entry.running_targets(),
                coordination,
            )
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
        self.sync_watchers(app, session, targets);
        coordination
    }

    pub fn mark_coordinated(&self, session: SessionId) {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        sessions.entry(session).or_default().coordinated = true;
    }

    pub fn disarm_viewer(&self, session: SessionId) {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        if let Some(entry) = sessions.get_mut(&session) {
            entry.viewer_disarmed = true;
        }
    }

    pub fn disarm_panel(&self, session: SessionId) {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        if let Some(entry) = sessions.get_mut(&session) {
            entry.panel_disarmed = true;
        }
    }

    /// Devolve `Some(summary)` quando o stop deste subagente deixou a sessão
    /// pronta para descer a `Idle` (orquestrador já ocioso, zero subagente
    /// ativo). O chamador (que possui o `SessionManager`) aplica o status.
    pub fn on_subagent_stopped<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        agent_id: String,
        agent_type: Option<String>,
        last_assistant_message: Option<String>,
    ) -> Option<Option<String>> {
        let agent_id = strip_agent_prefix(&agent_id);
        let (payload, targets, idle) = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let entry = sessions.get_mut(&session)?;
            if !entry.mark_stopped(agent_id, agent_type, last_assistant_message, now_ms()) {
                return None;
            }
            let idle = entry.idle_transition();
            (
                changed_payload(session, entry),
                entry.running_targets(),
                idle,
            )
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
        self.sync_watchers(app, session, targets);
        idle
    }

    pub fn on_session_ended<R: Runtime>(&self, app: &AppHandle<R>, session: SessionId) {
        let (payload, targets) = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let Some(entry) = sessions.get_mut(&session) else {
                return;
            };
            entry.end_all(now_ms());
            (changed_payload(session, entry), entry.running_targets())
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
        self.sync_watchers(app, session, targets);
    }

    fn sync_watchers<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: SessionId,
        targets: Vec<WatchSpec>,
    ) {
        self.watchers.sync(
            app,
            session,
            targets,
            Arc::clone(&self.sessions),
            Arc::clone(&self.coordinator),
        );
    }

    pub fn focus<R: Runtime>(&self, app: &AppHandle<R>, session: SessionId, agent_id: String) {
        let agent_id = strip_agent_prefix(&agent_id);
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

    pub fn remove_session<R: Runtime>(&self, app: &AppHandle<R>, session: SessionId) {
        self.watchers.stop_session(app, session);
        let payload = {
            let mut sessions = self.sessions.lock().expect("subagents lock");
            let Some(mut entry) = sessions.remove(&session) else {
                return;
            };
            entry.end_all(now_ms());
            changed_payload(session, &entry)
        };
        let _ = app.emit(EVENT_SUBAGENTS_CHANGED, payload);
    }

    pub fn resolve_transcript_path(&self, session: SessionId, agent_id: &str) -> Option<PathBuf> {
        let mut sessions = self.sessions.lock().expect("subagents lock");
        let run = sessions
            .get_mut(&session)?
            .runs
            .iter_mut()
            .find(|run| run.agent_id.as_deref() == Some(agent_id))?;
        if run.transcript_path.is_none() {
            let hint = (!run.description.is_empty()).then_some(run.description.as_str());
            run.transcript_path = resolve_transcript(
                run.parent_transcript_path.as_deref(),
                agent_id,
                &run.agent_type,
                hint,
            );
        }
        run.transcript_path.clone()
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

/// Chamado pelo watcher quando um subagente async termina por arquivo. Marca o
/// run `Done` e devolve o snapshot para reemitir e a transição de status
/// pendente (se houver). Roda sob o lock de `sessions`, que é solto antes de
/// tocar em status/app — sem lock aninhado.
pub(crate) fn finish_by_file(
    sessions: &SharedSessions,
    session: SessionId,
    agent_id: &str,
    summary: Option<String>,
    now: u64,
) -> Option<(SubagentSnapshot, Option<Option<String>>)> {
    let mut map = sessions.lock().ok()?;
    let entry = map.get_mut(&session)?;
    if !entry.finish_running_by_file(agent_id, summary, now) {
        return None;
    }
    let idle = entry.idle_transition();
    Some((entry.to_snapshot(), idle))
}

pub(crate) fn session_orchestrator_idle(sessions: &SharedSessions, session: SessionId) -> bool {
    sessions
        .lock()
        .ok()
        .and_then(|map| map.get(&session).map(|entry| entry.orchestrator_idle))
        .unwrap_or(false)
}

pub(crate) fn finish_by_file_interrupted(
    sessions: &SharedSessions,
    session: SessionId,
    agent_id: &str,
    now: u64,
) -> Option<(SubagentSnapshot, Option<Option<String>>)> {
    let mut map = sessions.lock().ok()?;
    let entry = map.get_mut(&session)?;
    if !entry.finish_running_interrupted(agent_id, now) {
        return None;
    }
    let idle = entry.idle_transition();
    Some((entry.to_snapshot(), idle))
}

pub(crate) fn emit_snapshot<R: Runtime>(
    app: &AppHandle<R>,
    session: SessionId,
    snapshot: SubagentSnapshot,
) {
    let _ = app.emit(
        EVENT_SUBAGENTS_CHANGED,
        SubagentsChangedPayload {
            session_id: session,
            focused: snapshot.focused,
            subagents: snapshot.subagents,
        },
    );
}

fn strip_agent_prefix(agent_id: &str) -> String {
    agent_id
        .strip_prefix("agent-")
        .unwrap_or(agent_id)
        .to_string()
}

pub(crate) fn sidecar_dir(parent_transcript_path: &Path) -> PathBuf {
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

pub(crate) fn resolve_transcript(
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

        session.promote("a1".into(), "reviewer".into(), None, None, None, 40);

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
        session.promote("a9".into(), "explorer".into(), None, None, None, 20);
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
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        assert_eq!(session.focused.as_deref(), Some("a1"));
        session.promote("a2".into(), "explorer".into(), None, None, None, 20);
        assert_eq!(session.focused.as_deref(), Some("a1"));
    }

    #[test]
    fn focus_stays_on_agent_after_it_finishes() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        session.promote("a2".into(), "explorer".into(), None, None, None, 20);
        session.set_focus("a2".into());
        session.mark_stopped("a2".into(), None, Some("pronto".into()), 30);
        assert_eq!(session.focused.as_deref(), Some("a2"));
        assert_eq!(status_of(&session, "a2"), Some(SubagentStatus::Done));
    }

    #[test]
    fn focus_ignores_unknown_agent() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        session.set_focus("ghost".into());
        assert_eq!(session.focused.as_deref(), Some("a1"));
    }

    #[test]
    fn stop_sets_summary_from_last_assistant_message() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        session.mark_stopped("a1".into(), None, Some("  achei   dois bugs ".into()), 20);
        let run = &session.runs[0];
        assert_eq!(run.status, SubagentStatus::Done);
        assert_eq!(run.ended_at_ms, Some(20));
        assert_eq!(run.summary.as_deref(), Some("achei dois bugs"));
    }

    #[test]
    fn stop_summary_truncates_at_280_chars() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        session.mark_stopped("a1".into(), None, Some("x".repeat(400)), 20);
        assert!(session.runs[0].summary.as_ref().unwrap().chars().count() <= 280);
    }

    #[test]
    fn stop_of_unknown_plausible_agent_creates_orphan_done_run() {
        let mut session = SessionSubagents::default();
        let created = session.mark_stopped(
            "a51505d91f9ef581f".into(),
            Some("general-purpose".into()),
            Some("resumo".into()),
            50,
        );
        assert!(created);
        assert_eq!(session.runs.len(), 1);
        let run = &session.runs[0];
        assert_eq!(run.agent_id.as_deref(), Some("a51505d91f9ef581f"));
        assert_eq!(run.agent_type, "general-purpose");
        assert_eq!(run.status, SubagentStatus::Done);
        assert_eq!(run.summary.as_deref(), Some("resumo"));
        assert!(session.focused.is_none());
    }

    #[test]
    fn stop_without_plausible_id_creates_no_phantom() {
        let mut session = SessionSubagents::default();
        let created = session.mark_stopped("aX".into(), Some("general-purpose".into()), None, 50);
        assert!(!created);
        assert!(session.runs.is_empty());
    }

    #[test]
    fn stop_without_agent_type_creates_no_phantom() {
        let mut session = SessionSubagents::default();
        let created = session.mark_stopped("a51505d91f9ef581f".into(), None, None, 50);
        assert!(!created);
        assert!(session.runs.is_empty());
        let blank = session.mark_stopped(
            "a51505d91f9ef581f".into(),
            Some("   ".into()),
            Some("resumo".into()),
            50,
        );
        assert!(!blank);
        assert!(session.runs.is_empty());
    }

    #[test]
    fn session_end_marks_running_as_done_and_discards_starting() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
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
        assert!(!session
            .runs
            .iter()
            .any(|run| run.status == SubagentStatus::Starting));
    }

    #[test]
    fn deny_removes_newest_compatible_pending_among_many() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), Some("primeiro".into()), 10);
        session.push_pending(Some("reviewer".into()), Some("segundo".into()), 20);
        session.push_pending(Some("explorer".into()), Some("outro".into()), 30);

        session.remove_pending(Some("reviewer".into()), Some("segundo".into()));

        assert_eq!(session.runs.len(), 2);
        assert!(session.runs.iter().any(|run| run.description == "primeiro"));
        assert!(session.runs.iter().any(|run| run.description == "outro"));
        assert!(!session.runs.iter().any(|run| run.description == "segundo"));
    }

    #[test]
    fn deny_without_description_removes_newest_of_type() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), Some("primeiro".into()), 10);
        session.push_pending(Some("reviewer".into()), Some("segundo".into()), 20);

        session.remove_pending(Some("reviewer".into()), None);

        assert_eq!(session.runs.len(), 1);
        assert_eq!(session.runs[0].description, "primeiro");
    }

    #[test]
    fn deny_without_matching_pending_is_noop() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), Some("primeiro".into()), 10);

        session.remove_pending(Some("explorer".into()), None);

        assert_eq!(session.runs.len(), 1);
        assert_eq!(session.runs[0].status, SubagentStatus::Starting);
    }

    #[test]
    fn duplicate_stop_does_not_change_runs() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "reviewer".into(), None, None, None, 10);
        assert!(session.mark_stopped("a1".into(), None, Some("pronto".into()), 20));
        let len_after_first = session.runs.len();
        assert!(!session.mark_stopped("a1".into(), None, Some("de novo".into()), 30));
        assert_eq!(session.runs.len(), len_after_first);
        assert_eq!(session.runs[0].summary.as_deref(), Some("pronto"));
        assert_eq!(session.runs[0].ended_at_ms, Some(20));
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
        let hint = session.pending_hint("reviewer");
        let path = resolve_transcript(Some(&parent), "a1", "reviewer", hint.as_deref());
        session.promote("a1".into(), "reviewer".into(), None, path, None, 20);
        assert_eq!(session.runs[0].transcript_path, Some(jsonl));
    }

    #[test]
    fn tracker_emits_and_snapshots_across_transitions() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.on_spawn_requested(
            &handle,
            session,
            Some("reviewer".into()),
            Some("revisar".into()),
        );
        tracker.on_subagent_started(&handle, session, "a1".into(), "reviewer".into(), None, None);
        let snapshot = tracker.snapshot(session);
        assert_eq!(snapshot.focused.as_deref(), Some("a1"));
        assert_eq!(snapshot.subagents.len(), 1);
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Running);

        tracker.on_session_ended(&handle, session);
        assert_eq!(
            tracker.snapshot(session).subagents[0].status,
            SubagentStatus::Done
        );

        tracker.remove_session(&handle, session);
        assert!(tracker.snapshot(session).subagents.is_empty());
    }

    #[test]
    fn events_after_remove_session_are_noop() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.on_subagent_started(&handle, session, "a1".into(), "reviewer".into(), None, None);
        tracker.remove_session(&handle, session);

        let restarted = tracker.on_subagent_started(
            &handle,
            session,
            "a2".into(),
            "reviewer".into(),
            None,
            None,
        );
        assert!(restarted.is_none());
        tracker.on_subagent_stopped(&handle, session, "a2".into(), None, Some("pronto".into()));
        tracker.on_spawn_requested(&handle, session, Some("explorer".into()), None);

        assert!(tracker.snapshot(session).subagents.is_empty());
        assert!(!tracker
            .sessions
            .lock()
            .expect("subagents lock")
            .contains_key(&session));
    }

    #[test]
    fn start_and_stop_correlate_across_agent_prefix() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.on_subagent_started(
            &handle,
            session,
            "agent-abc".into(),
            "reviewer".into(),
            None,
            None,
        );
        tracker.on_subagent_stopped(&handle, session, "abc".into(), None, Some("pronto".into()));

        let snapshot = tracker.snapshot(session);
        assert_eq!(snapshot.subagents.len(), 1);
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Done);
        assert_eq!(snapshot.subagents[0].agent_id.as_deref(), Some("abc"));
        assert_eq!(snapshot.subagents[0].summary.as_deref(), Some("pronto"));
    }

    #[test]
    fn coordination_retries_until_marked() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        let first = tracker.on_subagent_started(
            &handle,
            session,
            "a1".into(),
            "reviewer".into(),
            None,
            None,
        );
        assert!(first.is_some());

        let retry = tracker.on_subagent_started(
            &handle,
            session,
            "a2".into(),
            "reviewer".into(),
            None,
            None,
        );
        assert!(retry.is_some());

        tracker.mark_coordinated(session);

        let after = tracker.on_subagent_started(
            &handle,
            session,
            "a3".into(),
            "reviewer".into(),
            None,
            None,
        );
        assert!(after.is_none());
    }

    #[test]
    fn new_round_rearms_coordination_and_disarms() {
        // Rodada 1 abre o painel (coordination), conclui, e o usuário pode até
        // ter fechado o painel (disarm). Rodada 2 é contexto novo: coordena de
        // novo — sem isso a segunda rodada roda às cegas.
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        let first = tracker
            .on_subagent_started(&handle, session, "a1".into(), "explorer".into(), None, None)
            .expect("rodada 1 coordena");
        assert!(!first.panel_disarmed);
        tracker.mark_coordinated(session);
        tracker.disarm_panel(session);
        tracker.on_subagent_stopped(&handle, session, "a1".into(), None, None);

        let second = tracker
            .on_subagent_started(&handle, session, "b2".into(), "reviewer".into(), None, None)
            .expect("rodada 2 re-coordena");
        assert!(!second.panel_disarmed);
        assert!(!second.viewer_disarmed);
        tracker.mark_coordinated(session);

        let same_round = tracker.on_subagent_started(
            &handle,
            session,
            "c3".into(),
            "explorer".into(),
            None,
            None,
        );
        assert!(same_round.is_none(), "coordenação continua once POR RODADA");
    }

    #[test]
    fn coordination_carries_disarm_flags() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.disarm_viewer(session);
        let coordination = tracker
            .on_subagent_started(&handle, session, "a1".into(), "reviewer".into(), None, None)
            .unwrap();
        assert!(coordination.viewer_disarmed);
        assert!(!coordination.panel_disarmed);
    }

    #[test]
    fn spawn_denied_removes_pending_and_reemits() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.on_spawn_requested(
            &handle,
            session,
            Some("reviewer".into()),
            Some("revisar".into()),
        );
        assert_eq!(tracker.snapshot(session).subagents.len(), 1);

        tracker.on_spawn_denied(
            &handle,
            session,
            Some("reviewer".into()),
            Some("revisar".into()),
        );
        assert!(tracker.snapshot(session).subagents.is_empty());
    }

    #[test]
    fn session_end_leaves_no_running_or_starting() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.on_subagent_started(&handle, session, "a1".into(), "reviewer".into(), None, None);
        tracker.on_spawn_requested(
            &handle,
            session,
            Some("explorer".into()),
            Some("mapear".into()),
        );
        tracker.on_session_ended(&handle, session);

        let snapshot = tracker.snapshot(session);
        assert!(snapshot
            .subagents
            .iter()
            .all(|run| run.status == SubagentStatus::Done));
    }

    #[test]
    fn transcript_path_is_re_resolved_when_sidecar_appears_late() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();

        tracker.register_session(session);
        tracker.on_subagent_started(
            &handle,
            session,
            "a1".into(),
            "reviewer".into(),
            None,
            Some(&parent),
        );
        assert!(tracker.resolve_transcript_path(session, "a1").is_none());

        let sidecar = dir.path().join("session").join("subagents");
        fs::create_dir_all(&sidecar).unwrap();
        let jsonl = sidecar.join("agent-a1.jsonl");
        write(&jsonl, "{}");

        assert_eq!(tracker.resolve_transcript_path(session, "a1"), Some(jsonl));
        tracker.remove_session(&handle, session);
    }

    #[test]
    fn active_count_reflects_starting_and_running() {
        let mut session = SessionSubagents::default();
        assert_eq!(session.active_count(), 0);
        session.push_pending(Some("reviewer".into()), None, 10);
        assert_eq!(session.active_count(), 1);
        session.promote("a1".into(), "explorer".into(), None, None, None, 20);
        assert_eq!(session.active_count(), 2);
        session.mark_stopped("a1".into(), None, Some("pronto".into()), 30);
        assert_eq!(session.active_count(), 1);
    }

    #[test]
    fn finish_by_file_marks_running_done_with_summary() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "explorer".into(), None, None, None, 10);
        assert!(session.finish_running_by_file("a1", Some("  fim  do  turno ".into()), 40));
        let run = &session.runs[0];
        assert_eq!(run.status, SubagentStatus::Done);
        assert_eq!(run.ended_at_ms, Some(40));
        assert_eq!(run.summary.as_deref(), Some("fim do turno"));
    }

    #[test]
    fn finish_by_file_ignores_starting_and_done() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), None, 10);
        assert!(!session.finish_running_by_file("a1", Some("x".into()), 40));
        session.promote("a1".into(), "reviewer".into(), None, None, None, 20);
        assert!(session.finish_running_by_file("a1", None, 40));
        assert!(!session.finish_running_by_file("a1", Some("de novo".into()), 50));
        assert_eq!(session.runs[0].status, SubagentStatus::Done);
        assert_eq!(session.runs[0].ended_at_ms, Some(40));
    }

    #[test]
    fn finish_running_interrupted_marks_running_done_without_summary() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "explorer".into(), None, None, None, 10);
        assert!(session.finish_running_interrupted("a1", 40));
        let run = &session.runs[0];
        assert_eq!(run.status, SubagentStatus::Done);
        assert_eq!(run.ended_at_ms, Some(40));
        assert!(run.interrupted);
        assert!(run.summary.is_none());
    }

    #[test]
    fn finish_running_interrupted_ignores_starting_and_done() {
        let mut session = SessionSubagents::default();
        session.push_pending(Some("reviewer".into()), None, 10);
        assert!(!session.finish_running_interrupted("a1", 40));
        session.promote("a1".into(), "reviewer".into(), None, None, None, 20);
        assert!(session.finish_running_interrupted("a1", 40));
        assert!(!session.finish_running_interrupted("a1", 50));
        assert_eq!(session.runs[0].ended_at_ms, Some(40));
    }

    #[test]
    fn interrupted_run_is_serialized_in_snapshot() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "explorer".into(), None, None, None, 10);
        session.finish_running_interrupted("a1", 40);
        let snapshot = session.to_snapshot();
        assert!(snapshot.subagents[0].interrupted);
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Done);
    }

    #[test]
    fn finish_by_file_interrupted_flips_last_active_to_idle() {
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        {
            let mut sessions = tracker.sessions.lock().unwrap();
            let entry = sessions.get_mut(&session).unwrap();
            entry.promote("a1".into(), "explorer".into(), None, None, None, 10);
            entry.orchestrator_idle = true;
            entry.orchestrator_idle_summary = Some("turno".into());
        }
        assert!(session_orchestrator_idle(&tracker.sessions, session));
        let (snapshot, idle) =
            finish_by_file_interrupted(&tracker.sessions, session, "a1", 99).unwrap();
        assert!(snapshot.subagents[0].interrupted);
        assert_eq!(idle, Some(Some("turno".into())));
    }

    #[test]
    fn session_orchestrator_idle_defaults_false_without_signal() {
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        assert!(!session_orchestrator_idle(&tracker.sessions, session));
    }

    #[test]
    fn idle_transition_only_when_orchestrator_idle_and_no_active() {
        let mut session = SessionSubagents::default();
        session.promote("a1".into(), "explorer".into(), None, None, None, 10);
        assert!(session.idle_transition().is_none());
        session.orchestrator_idle = true;
        session.orchestrator_idle_summary = Some("resumo do turno".into());
        assert!(session.idle_transition().is_none());
        session.finish_running_by_file("a1", None, 20);
        assert_eq!(
            session.idle_transition(),
            Some(Some("resumo do turno".into()))
        );
    }

    #[test]
    fn has_active_true_with_running_subagent() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        assert!(!tracker.has_active(session));
        tracker.on_subagent_started(&handle, session, "a1".into(), "explorer".into(), None, None);
        assert!(tracker.has_active(session));
    }

    #[test]
    fn file_finish_flips_session_to_idle_when_orchestrator_idle() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        tracker.on_subagent_started(&handle, session, "a1".into(), "explorer".into(), None, None);

        type Flips = Arc<Mutex<Vec<(SessionId, Option<String>)>>>;
        let flips: Flips = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&flips);
        tracker.set_idle_coordinator(Arc::new(move |s, summary| {
            sink.lock().unwrap().push((s, summary));
        }));

        // Orquestrador ocioso, mas subagente ainda ativo: nada acontece ainda.
        tracker.note_orchestrator_idle(session, Some("turno pronto".into()));

        let (snapshot, idle) =
            finish_by_file(&tracker.sessions, session, "a1", Some("fim".into()), 99).unwrap();
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Done);
        // simula o watcher aplicando a transição via coordinator
        if let Some(summary) = idle {
            if let Some(coordinator) = tracker.coordinator.lock().unwrap().clone() {
                coordinator(session, summary);
            }
        }
        let recorded = flips.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, session);
        assert_eq!(recorded[0].1.as_deref(), Some("turno pronto"));
    }

    #[test]
    fn subagent_stop_returns_idle_transition_when_last_one_ends() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        tracker.on_subagent_started(&handle, session, "a1".into(), "explorer".into(), None, None);
        tracker.note_orchestrator_idle(session, Some("turno".into()));

        let idle = tracker.on_subagent_stopped(
            &handle,
            session,
            "a1".into(),
            Some("explorer".into()),
            Some("achei".into()),
        );
        assert_eq!(idle, Some(Some("turno".into())));
    }

    #[test]
    fn subagent_stop_holds_running_while_another_active() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        tracker.on_subagent_started(&handle, session, "a1".into(), "explorer".into(), None, None);
        tracker.on_subagent_started(&handle, session, "a2".into(), "reviewer".into(), None, None);
        tracker.note_orchestrator_idle(session, Some("turno".into()));

        let idle = tracker.on_subagent_stopped(
            &handle,
            session,
            "a1".into(),
            Some("explorer".into()),
            Some("achei".into()),
        );
        assert!(idle.is_none());
        assert!(tracker.has_active(session));
    }

    #[test]
    fn working_signal_clears_orchestrator_idle() {
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        tracker.note_orchestrator_idle(session, Some("turno".into()));
        tracker.note_orchestrator_working(session);
        let sessions = tracker.sessions.lock().unwrap();
        let entry = sessions.get(&session).unwrap();
        assert!(!entry.orchestrator_idle);
        assert!(entry.orchestrator_idle_summary.is_none());
    }

    #[test]
    fn session_end_freezes_active_subagent_as_done_without_phantom() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker = SubagentTracker::new();
        let session = SessionId::new_v4();
        tracker.register_session(session);
        // subagente async ainda aberto no tracker
        tracker.on_subagent_started(
            &handle,
            session,
            "a51505d91f9ef581f".into(),
            "general-purpose".into(),
            None,
            None,
        );
        // item de fila / stop espúrio não vira entrada
        tracker.on_subagent_stopped(
            &handle,
            session,
            "queued".into(),
            None,
            Some("prompt".into()),
        );

        tracker.on_session_ended(&handle, session);

        let snapshot = tracker.snapshot(session);
        assert_eq!(snapshot.subagents.len(), 1);
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Done);
        assert!(snapshot.subagents[0].ended_at_ms.is_some());
        assert!(!snapshot.subagents.iter().any(|run| matches!(
            run.status,
            SubagentStatus::Running | SubagentStatus::Starting
        )));
    }
}
