use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde_json::Value;
use tauri::{AppHandle, Runtime};

use crate::agent::subagents::{is_plausible_subagent_id, sidecar_dir, SharedSubagents};
use crate::session::SessionId;

use super::claude_project_dir_name;

pub type SharedDiskObserver = Arc<DiskObserver>;

pub const SIDECAR_POLL_INTERVAL: Duration = Duration::from_secs(1);

const HEURISTIC_SLACK_MS: u64 = 5_000;
const AMBIGUITY_WINDOW_MS: u64 = 2_000;
const FIRST_MESSAGE_SCAN_BYTES: u64 = 64 * 1024;
const FIRST_MESSAGE_SCAN_LINES: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptCandidate {
    pub path: PathBuf,
    pub mtime_ms: u64,
    pub first_msg_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSubagent {
    pub agent_id: String,
    pub agent_type: String,
}

/// Escolhe o transcript ativo da sessão de shell entre os candidatos do slug.
/// Heurística de janela de tempo (decisão 5): o transcript certo tem a primeira
/// mensagem surgindo logo após o nascimento do agente, então filtra os que
/// começaram antes do `agent_start_ms` (dono anterior do cwd) e prefere o mais
/// próximo. Dois candidatos igualmente próximos do start são indistinguíveis
/// (dois claude quase simultâneos no mesmo cwd) e caem no melhor-esforço: o
/// mtime mais novo. Sem candidato datável, idem.
pub fn choose_active_transcript(
    candidates: &[TranscriptCandidate],
    agent_start_ms: u64,
) -> Option<PathBuf> {
    let mut timed: Vec<(&TranscriptCandidate, u64)> = candidates
        .iter()
        .filter_map(|c| c.first_msg_ms.map(|ts| (c, ts)))
        .filter(|(_, ts)| ts.saturating_add(HEURISTIC_SLACK_MS) >= agent_start_ms)
        .map(|(c, ts)| (c, ts.abs_diff(agent_start_ms)))
        .collect();
    timed.sort_by_key(|(_, delta)| *delta);
    match (timed.first(), timed.get(1)) {
        (Some((_, d0)), Some((_, d1))) if d1.saturating_sub(*d0) < AMBIGUITY_WINDOW_MS => timed
            .iter()
            .map(|(c, _)| *c)
            .max_by_key(|c| c.mtime_ms)
            .map(|c| c.path.clone()),
        (Some((best, _)), _) => Some(best.path.clone()),
        (None, _) => candidates
            .iter()
            .max_by_key(|c| c.mtime_ms)
            .map(|c| c.path.clone()),
    }
}

/// Lista `~/.claude/projects/<slug>/<sessionId>.jsonl` e escolhe o ativo pelo
/// `agent_start_ms` do agente detectado pela F1. `None` quando o slug não existe
/// ou está vazio — tolerante, nunca panica.
pub fn active_transcript_for_cwd(cwd: &Path, agent_start_ms: u64) -> Option<PathBuf> {
    let dir = claude_projects_dir()?.join(claude_project_dir_name(cwd));
    let candidates = list_transcript_candidates(&dir);
    choose_active_transcript(&candidates, agent_start_ms)
}

fn claude_projects_dir() -> Option<PathBuf> {
    crate::ssh::home_dir().map(|home| home.join(".claude").join("projects"))
}

fn list_transcript_candidates(dir: &Path) -> Vec<TranscriptCandidate> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let mtime_ms = meta.modified().ok().and_then(system_time_ms).unwrap_or(0);
        out.push(TranscriptCandidate {
            path: path.clone(),
            mtime_ms,
            first_msg_ms: first_message_ms(&path),
        });
    }
    out
}

fn system_time_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

fn first_message_ms(path: &Path) -> Option<u64> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file).take(FIRST_MESSAGE_SCAN_BYTES);
    let mut line = String::new();
    for _ in 0..FIRST_MESSAGE_SCAN_LINES {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(ms) = serde_json::from_str::<Value>(trimmed)
            .ok()
            .as_ref()
            .and_then(|v| v.get("timestamp"))
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_ms)
        {
            return Some(ms);
        }
    }
    None
}

fn parse_rfc3339_ms(raw: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.timestamp_millis().max(0) as u64)
}

/// Varre o sidecar dir do transcript pai e devolve os subagentes ainda não
/// vistos: um `agent-<id>.jsonl` novo é um subagente que iniciou. Só entra id
/// hexadecimal plausível (anti-fantasma, reusa [`is_plausible_subagent_id`]) e
/// com `agentType` legível no `agent-<id>.meta.json` nativo do Claude — meta
/// ausente adia até o próximo poll, para não registrar tipo vazio.
pub fn scan_new_subagents(sidecar: &Path, known: &HashSet<String>) -> Vec<DiscoveredSubagent> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(sidecar) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(agent_id) = name
            .strip_prefix("agent-")
            .and_then(|rest| rest.strip_suffix(".jsonl"))
        else {
            continue;
        };
        if !is_plausible_subagent_id(agent_id) || known.contains(agent_id) {
            continue;
        }
        let Some(agent_type) = read_agent_type(sidecar, agent_id) else {
            continue;
        };
        out.push(DiscoveredSubagent {
            agent_id: agent_id.to_string(),
            agent_type,
        });
    }
    out
}

fn read_agent_type(sidecar: &Path, agent_id: &str) -> Option<String> {
    let meta = sidecar.join(format!("agent-{agent_id}.meta.json"));
    let body = std::fs::read_to_string(meta).ok()?;
    serde_json::from_str::<Value>(&body)
        .ok()?
        .get("agentType")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

struct SessionObserver {
    parent: PathBuf,
    poll_stop: Arc<AtomicBool>,
}

/// Ponte disco→tracker por sessão de shell. Enquanto a F1 reporta um agente
/// numa sessão de shell, mantém uma thread que descobre subagentes por arquivo
/// e os sintetiza no [`SubagentTracker`] com os MESMOS métodos dos hooks. Sem
/// sessão observada não há thread nem varredura — custo zero.
#[derive(Default)]
pub struct DiskObserver {
    sessions: Mutex<HashMap<SessionId, SessionObserver>>,
}

impl DiskObserver {
    pub fn new() -> Self {
        Self::default()
    }

    /// A F1 reporta um agente Claude na sessão de shell: resolve o transcript
    /// ativo e passa a observar seu sidecar. Idempotente enquanto o transcript
    /// for o mesmo; um transcript novo (ou reinício após o agente sumir)
    /// recomeça do zero com o painel limpo.
    pub fn observe<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        tracker: &SharedSubagents,
        session: SessionId,
        cwd: &Path,
        agent_start_ms: u64,
    ) {
        self.observe_with(app, tracker, session, || {
            active_transcript_for_cwd(cwd, agent_start_ms)
        });
    }

    /// `true` enquanto uma thread de varredura está viva para a sessão. `false`
    /// tanto para sessão nunca observada quanto para uma congelada — é o gancho
    /// que o poll da F1 usa para re-tentar `observe` até o transcript resolver.
    pub fn is_observing(&self, session: SessionId) -> bool {
        self.sessions
            .lock()
            .expect("disk observer lock")
            .get(&session)
            .map(|observer| !observer.poll_stop.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    fn observe_with<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        tracker: &SharedSubagents,
        session: SessionId,
        resolve: impl FnOnce() -> Option<PathBuf>,
    ) {
        let Some(parent) = resolve() else {
            return;
        };
        {
            let map = self.sessions.lock().expect("disk observer lock");
            match map.get(&session) {
                Some(observer)
                    if !observer.poll_stop.load(Ordering::SeqCst) && observer.parent == parent =>
                {
                    return;
                }
                Some(observer) => observer.poll_stop.store(true, Ordering::SeqCst),
                None => {}
            }
        }
        tracker.remove_session(app, session);
        tracker.register_session(session);
        let poll_stop = Arc::new(AtomicBool::new(false));
        spawn_sidecar_poll(
            app.clone(),
            Arc::clone(tracker),
            session,
            parent.clone(),
            Arc::clone(&poll_stop),
        );
        self.sessions
            .lock()
            .expect("disk observer lock")
            .insert(session, SessionObserver { parent, poll_stop });
    }

    /// A F1 reporta que o agente sumiu (a sessão de shell continua viva): para a
    /// varredura e congela os subagentes em `Done` (paridade com o hook `Ended`).
    /// A entrada fica como marcador para a limpeza por liveness.
    pub fn stop<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        tracker: &SharedSubagents,
        session: SessionId,
    ) {
        let freeze = {
            let map = self.sessions.lock().expect("disk observer lock");
            match map.get(&session) {
                Some(observer) if !observer.poll_stop.load(Ordering::SeqCst) => {
                    observer.poll_stop.store(true, Ordering::SeqCst);
                    true
                }
                _ => false,
            }
        };
        if freeze {
            tracker.on_session_ended(app, session);
        }
    }

    /// Encerra as observações de sessões de shell que sumiram do conjunto vivo —
    /// a F1 descarta terminais mortos em silêncio (sem evento), então a limpeza
    /// vem por liveness. Terminal fechado ⇒ para a thread e limpa o painel.
    pub fn retain_live<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        tracker: &SharedSubagents,
        live: &HashSet<SessionId>,
    ) {
        let dead: Vec<SessionId> = {
            let mut map = self.sessions.lock().expect("disk observer lock");
            let mut dead = Vec::new();
            map.retain(|session, observer| {
                if live.contains(session) {
                    return true;
                }
                observer.poll_stop.store(true, Ordering::SeqCst);
                dead.push(*session);
                false
            });
            dead
        };
        for session in dead {
            tracker.remove_session(app, session);
        }
    }
}

fn spawn_sidecar_poll<R: Runtime>(
    app: AppHandle<R>,
    tracker: SharedSubagents,
    session: SessionId,
    parent: PathBuf,
    poll_stop: Arc<AtomicBool>,
) {
    let sidecar = sidecar_dir(&parent);
    let spawned = std::thread::Builder::new()
        .name("disk-observer-sidecar".into())
        .spawn(move || {
            let mut known: HashSet<String> = HashSet::new();
            while !poll_stop.load(Ordering::SeqCst) {
                poll_once(&app, &tracker, session, &sidecar, &parent, &mut known);
                std::thread::sleep(SIDECAR_POLL_INTERVAL);
            }
        });
    if let Err(err) = spawned {
        eprintln!("[tyba] disk-observer: thread de sidecar não iniciou: {err}");
    }
}

fn poll_once<R: Runtime>(
    app: &AppHandle<R>,
    tracker: &SharedSubagents,
    session: SessionId,
    sidecar: &Path,
    parent: &Path,
    known: &mut HashSet<String>,
) {
    for found in scan_new_subagents(sidecar, known) {
        known.insert(found.agent_id.clone());
        tracker.on_subagent_started(app, session, found.agent_id, found.agent_type, Some(parent));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::subagents::{SubagentStatus, SubagentTracker};
    use std::fs;
    use tempfile::TempDir;

    fn candidate(path: &str, mtime_ms: u64, first_msg_ms: Option<u64>) -> TranscriptCandidate {
        TranscriptCandidate {
            path: PathBuf::from(path),
            mtime_ms,
            first_msg_ms,
        }
    }

    #[test]
    fn choose_without_usable_timestamps_falls_back_to_newest_mtime() {
        let candidates = vec![
            candidate("/a.jsonl", 100, None),
            candidate("/b.jsonl", 300, None),
            candidate("/c.jsonl", 200, None),
        ];
        assert_eq!(
            choose_active_transcript(&candidates, 1_000),
            Some(PathBuf::from("/b.jsonl"))
        );
    }

    #[test]
    fn time_heuristic_picks_the_transcript_born_just_after_the_agent() {
        // O dono anterior do cwd (a.jsonl) tem o mtime mais novo, mas a primeira
        // mensagem é bem antes do agente nascer; o transcript certo (b.jsonl)
        // surge logo após o start e vence, mesmo com mtime mais velho.
        let candidates = vec![
            candidate("/old.jsonl", 9_999, Some(1_000)),
            candidate("/new.jsonl", 5_000, Some(10_500)),
        ];
        assert_eq!(
            choose_active_transcript(&candidates, 10_000),
            Some(PathBuf::from("/new.jsonl"))
        );
    }

    #[test]
    fn two_near_simultaneous_transcripts_are_ambiguous_and_fall_back_to_newest() {
        // Dois claude quase simultâneos no mesmo cwd: os dois começam logo após o
        // start e são indistinguíveis pela heurística ⇒ melhor-esforço (mtime).
        let candidates = vec![
            candidate("/one.jsonl", 4_000, Some(10_300)),
            candidate("/two.jsonl", 7_000, Some(10_600)),
        ];
        assert_eq!(
            choose_active_transcript(&candidates, 10_000),
            Some(PathBuf::from("/two.jsonl"))
        );
    }

    #[test]
    fn ambiguous_fallback_stays_within_post_agent_candidates() {
        // Um claude antigo ainda ativo no mesmo cwd (stale) tem o mtime mais
        // novo, mas começou antes do start: o desempate por mtime do caso
        // ambíguo não pode escolhê-lo — fica entre os que passaram o filtro.
        let candidates = vec![
            candidate("/stale.jsonl", 99_999, Some(1_000)),
            candidate("/one.jsonl", 4_000, Some(10_300)),
            candidate("/two.jsonl", 7_000, Some(10_600)),
        ];
        assert_eq!(
            choose_active_transcript(&candidates, 10_000),
            Some(PathBuf::from("/two.jsonl"))
        );
    }

    #[test]
    fn choose_on_empty_is_none() {
        assert_eq!(choose_active_transcript(&[], 10_000), None);
    }

    #[test]
    fn transcripts_started_before_the_agent_are_ignored_by_the_heuristic() {
        let candidates = vec![
            candidate("/stale.jsonl", 1, Some(2_000)),
            candidate("/live.jsonl", 2, Some(50_100)),
        ];
        assert_eq!(
            choose_active_transcript(&candidates, 50_000),
            Some(PathBuf::from("/live.jsonl"))
        );
    }

    fn write(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
    }

    #[test]
    fn list_candidates_reads_first_message_timestamp() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session-1.jsonl");
        write(
            &path,
            "{\"type\":\"user\",\"timestamp\":\"2026-07-23T20:00:00.000Z\",\"message\":{\"content\":\"oi\"}}\n{\"type\":\"assistant\",\"timestamp\":\"2026-07-23T20:00:05.000Z\",\"message\":{\"content\":[]}}\n",
        );
        write(&dir.path().join("ignore.txt"), "x");
        let candidates = list_transcript_candidates(dir.path());
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, path);
        assert_eq!(
            candidates[0].first_msg_ms,
            parse_rfc3339_ms("2026-07-23T20:00:00.000Z")
        );
        assert!(candidates[0].mtime_ms > 0);
    }

    #[test]
    fn missing_slug_dir_yields_no_candidates() {
        assert!(list_transcript_candidates(Path::new("/nope/does/not/exist")).is_empty());
    }

    fn sidecar_with(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (name, body) in files {
            write(&dir.path().join(name), body);
        }
        dir
    }

    #[test]
    fn scan_finds_new_agent_with_type_from_meta() {
        let dir = sidecar_with(&[
            ("agent-a1b2c3d4e5.jsonl", "{}"),
            (
                "agent-a1b2c3d4e5.meta.json",
                r#"{"agentType":"reviewer","description":"revisar diff"}"#,
            ),
        ]);
        let found = scan_new_subagents(dir.path(), &HashSet::new());
        assert_eq!(
            found,
            vec![DiscoveredSubagent {
                agent_id: "a1b2c3d4e5".into(),
                agent_type: "reviewer".into(),
            }]
        );
    }

    #[test]
    fn scan_ignores_non_agent_files_and_the_meta_itself() {
        let dir = sidecar_with(&[
            ("notes.txt", "x"),
            ("other.jsonl", "{}"),
            ("agent-a1b2c3d4e5.meta.json", r#"{"agentType":"reviewer"}"#),
        ]);
        assert!(scan_new_subagents(dir.path(), &HashSet::new()).is_empty());
    }

    #[test]
    fn scan_ignores_non_plausible_ids() {
        let dir = sidecar_with(&[
            ("agent-xyz.jsonl", "{}"),
            ("agent-xyz.meta.json", r#"{"agentType":"reviewer"}"#),
            ("agent-a1.jsonl", "{}"),
            ("agent-a1.meta.json", r#"{"agentType":"reviewer"}"#),
        ]);
        assert!(scan_new_subagents(dir.path(), &HashSet::new()).is_empty());
    }

    #[test]
    fn scan_skips_already_known_ids() {
        let dir = sidecar_with(&[
            ("agent-a1b2c3d4e5.jsonl", "{}"),
            ("agent-a1b2c3d4e5.meta.json", r#"{"agentType":"reviewer"}"#),
        ]);
        let known: HashSet<String> = ["a1b2c3d4e5".to_string()].into_iter().collect();
        assert!(scan_new_subagents(dir.path(), &known).is_empty());
    }

    #[test]
    fn scan_waits_for_meta_before_registering() {
        let dir = sidecar_with(&[("agent-a1b2c3d4e5.jsonl", "{}")]);
        assert!(scan_new_subagents(dir.path(), &HashSet::new()).is_empty());
        write(
            &dir.path().join("agent-a1b2c3d4e5.meta.json"),
            r#"{"agentType":"explorer"}"#,
        );
        let found = scan_new_subagents(dir.path(), &HashSet::new());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].agent_type, "explorer");
    }

    #[test]
    fn scan_skips_meta_without_agent_type() {
        let dir = sidecar_with(&[
            ("agent-a1b2c3d4e5.jsonl", "{}"),
            (
                "agent-a1b2c3d4e5.meta.json",
                r#"{"description":"sem tipo"}"#,
            ),
        ]);
        assert!(scan_new_subagents(dir.path(), &HashSet::new()).is_empty());
    }

    #[test]
    fn poll_once_registers_discovered_subagent_into_tracker() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let session = SessionId::new_v4();
        tracker.register_session(session);

        let sidecar = sidecar_with(&[
            ("agent-a1b2c3d4e5.jsonl", "{}"),
            (
                "agent-a1b2c3d4e5.meta.json",
                r#"{"agentType":"general-purpose"}"#,
            ),
        ]);
        let parent = sidecar.path().join("parent.jsonl");
        let mut known = HashSet::new();

        poll_once(
            &handle,
            &tracker,
            session,
            sidecar.path(),
            &parent,
            &mut known,
        );

        let snapshot = tracker.snapshot(session);
        assert_eq!(snapshot.subagents.len(), 1);
        assert_eq!(
            snapshot.subagents[0].agent_id.as_deref(),
            Some("a1b2c3d4e5")
        );
        assert_eq!(snapshot.subagents[0].agent_type, "general-purpose");
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Running);
        assert!(known.contains("a1b2c3d4e5"));

        poll_once(
            &handle,
            &tracker,
            session,
            sidecar.path(),
            &parent,
            &mut known,
        );
        assert_eq!(tracker.snapshot(session).subagents.len(), 1);
    }

    #[test]
    fn observe_registers_the_session_and_stop_freezes_it() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");

        observer.begin(&handle, &tracker, session, parent.clone());
        assert!(observer.is_observing(session));

        tracker.on_subagent_started(
            &handle,
            session,
            "a1b2c3d4e5".into(),
            "explorer".into(),
            Some(&parent),
        );
        assert!(tracker.has_active(session));

        observer.stop(&handle, &tracker, session);
        let snapshot = tracker.snapshot(session);
        assert_eq!(snapshot.subagents.len(), 1);
        assert_eq!(snapshot.subagents[0].status, SubagentStatus::Done);
        assert!(!tracker.has_active(session));
        assert!(observer.is_frozen(session));
    }

    #[test]
    fn retain_live_ends_observers_for_vanished_shells() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let alive = SessionId::new_v4();
        let gone = SessionId::new_v4();
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");

        observer.begin(&handle, &tracker, alive, parent.clone());
        observer.begin(&handle, &tracker, gone, parent.clone());
        assert!(observer.is_observing(alive));
        assert!(observer.is_observing(gone));

        let live: HashSet<SessionId> = [alive].into_iter().collect();
        observer.retain_live(&handle, &tracker, &live);

        assert!(observer.is_observing(alive));
        assert!(!observer.is_observing(gone));
        assert!(!observer.contains(gone));
    }

    fn wait_for_subagent(tracker: &SharedSubagents, session: SessionId, agent_id: &str) -> bool {
        for _ in 0..200 {
            let seen = tracker
                .snapshot(session)
                .subagents
                .iter()
                .any(|run| run.agent_id.as_deref() == Some(agent_id));
            if seen {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn observe_retries_until_the_transcript_resolves() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();

        // 1º poll: o slug do claude ainda não tem transcript — nada é observado.
        observer.observe_with(&handle, &tracker, session, || None);
        assert!(!observer.is_observing(session));
        assert!(!observer.contains(session));

        // 2º poll: o transcript apareceu; a re-tentativa inicia a varredura, que
        // captura o subagente do sidecar.
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");
        let sidecar = parent.with_extension("").join("subagents");
        fs::create_dir_all(&sidecar).unwrap();
        write(&sidecar.join("agent-a1b2c3d4e5.jsonl"), "{}");
        write(
            &sidecar.join("agent-a1b2c3d4e5.meta.json"),
            r#"{"agentType":"explorer"}"#,
        );

        let resolved = parent.clone();
        observer.observe_with(&handle, &tracker, session, move || Some(resolved));
        assert!(observer.is_observing(session));
        assert!(wait_for_subagent(&tracker, session, "a1b2c3d4e5"));

        observer.retain_live(&handle, &tracker, &HashSet::new());
        assert!(!observer.is_observing(session));
    }

    impl DiskObserver {
        fn begin<R: Runtime>(
            &self,
            app: &AppHandle<R>,
            tracker: &SharedSubagents,
            session: SessionId,
            parent: PathBuf,
        ) {
            self.observe_with(app, tracker, session, || Some(parent));
        }

        fn is_frozen(&self, session: SessionId) -> bool {
            self.sessions
                .lock()
                .unwrap()
                .get(&session)
                .map(|o| o.poll_stop.load(Ordering::SeqCst))
                .unwrap_or(false)
        }

        fn contains(&self, session: SessionId) -> bool {
            self.sessions.lock().unwrap().contains_key(&session)
        }
    }
}
