use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde_json::Value;
use tauri::{AppHandle, Runtime};

use crate::agent::subagents::{
    is_plausible_subagent_id, sidecar_dir, Coordination, SharedSubagents,
};
use crate::session::SessionId;

use super::claude_project_dir_name;

pub type SharedDiskObserver = Arc<DiskObserver>;

pub const SIDECAR_POLL_INTERVAL: Duration = Duration::from_secs(1);

const HEURISTIC_SLACK_MS: u64 = 5_000;
const AMBIGUITY_WINDOW_MS: u64 = 2_000;
const FIRST_MESSAGE_SCAN_BYTES: u64 = 64 * 1024;
const FIRST_MESSAGE_SCAN_LINES: usize = 16;
/// Teto de leitura do `~/.claude/sessions/<pid>.json`. O registro real tem
/// meio KB; acima disto não é o arquivo que o Claude escreve, e não se lê.
const PID_FILE_MAX_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptCandidate {
    pub path: PathBuf,
    pub mtime_ms: u64,
    pub first_msg_ms: Option<u64>,
    /// Tamanho em bytes — é o que diz se o transcript está CRESCENDO entre
    /// dois ticks (ver [`DiskObserver::observe`]).
    pub len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSubagent {
    pub agent_id: String,
    pub agent_type: String,
    pub description: Option<String>,
}

/// Resolução do transcript ativo. `provisional` marca o melhor-esforço por
/// mtime sem nenhum candidato datado pós-agente: o bind vale, mas o poll da F1
/// continua re-resolvendo — se um candidato forte aparecer (o transcript do
/// agente ganhou a primeira mensagem datada), o observer rebinda pra ele.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTranscript {
    pub path: PathBuf,
    pub provisional: bool,
}

/// Escolhe o transcript ativo da sessão de shell entre os candidatos do slug.
/// Só concorre candidato plausível: mtime fresco relativo ao nascimento do
/// agente — um transcript frio de sessão anterior nunca é o ativo, mesmo sendo
/// o mais novo do slug (devolvê-lo travaria o observer no transcript errado
/// pra sempre, já que a detecção estável não re-resolve). Entre os plausíveis,
/// a heurística de janela de tempo (decisão 5) prefere o que tem a primeira
/// mensagem mais próxima do `agent_start_ms`; dois igualmente próximos são
/// indistinguíveis (dois claude quase simultâneos no mesmo cwd) e caem no
/// melhor-esforço por mtime, restrito aos pós-agente. Plausível sem timestamp
/// datável — recém-nascido só com headers, ou `--resume` que preserva
/// timestamps antigos mas volta a ser escrito — cai no melhor-esforço por
/// mtime. Sem candidato plausível (janela de startup: o transcript ainda não
/// nasceu) devolve `None` e o poll da F1 re-tenta até ele existir.
pub fn choose_active_transcript(
    candidates: &[TranscriptCandidate],
    agent_start_ms: u64,
) -> Option<ActiveTranscript> {
    let plausible: Vec<&TranscriptCandidate> = candidates
        .iter()
        .filter(|c| c.mtime_ms.saturating_add(HEURISTIC_SLACK_MS) >= agent_start_ms)
        .collect();
    let mut timed: Vec<(&TranscriptCandidate, u64)> = plausible
        .iter()
        .filter_map(|c| c.first_msg_ms.map(|ts| (*c, ts)))
        .filter(|(_, ts)| ts.saturating_add(HEURISTIC_SLACK_MS) >= agent_start_ms)
        .map(|(c, ts)| (c, ts.abs_diff(agent_start_ms)))
        .collect();
    timed.sort_by_key(|(_, delta)| *delta);
    match (timed.first(), timed.get(1)) {
        (Some((_, d0)), Some((_, d1))) if d1.saturating_sub(*d0) < AMBIGUITY_WINDOW_MS => timed
            .iter()
            .map(|(c, _)| *c)
            .max_by_key(|c| c.mtime_ms)
            .map(|c| ActiveTranscript {
                path: c.path.clone(),
                provisional: false,
            }),
        (Some((best, _)), _) => Some(ActiveTranscript {
            path: best.path.clone(),
            provisional: false,
        }),
        (None, _) => plausible
            .iter()
            .max_by_key(|c| c.mtime_ms)
            .map(|c| ActiveTranscript {
                path: c.path.clone(),
                provisional: true,
            }),
    }
}

/// Lista `~/.claude/projects/<slug>/<sessionId>.jsonl` do cwd do agente. Vazio
/// quando o slug não existe — tolerante, nunca panica.
fn slug_candidates(cwd: &Path, agent_start_ms: u64) -> Vec<TranscriptCandidate> {
    claude_projects_dir()
        .map(|dir| {
            list_transcript_candidates(&dir.join(claude_project_dir_name(cwd)), agent_start_ms)
        })
        .unwrap_or_default()
}

/// Escrito perto do nascimento do agente ou depois — o filtro de
/// plausibilidade de [`choose_active_transcript`].
fn is_fresh(candidate: &TranscriptCandidate, agent_start_ms: u64) -> bool {
    candidate.mtime_ms.saturating_add(HEURISTIC_SLACK_MS) >= agent_start_ms
}

fn claude_projects_dir() -> Option<PathBuf> {
    crate::ssh::home_dir().map(|home| home.join(".claude").join("projects"))
}

pub fn claude_sessions_dir() -> Option<PathBuf> {
    crate::ssh::home_dir().map(|home| home.join(".claude").join("sessions"))
}

/// A conversa que o próprio Claude Code diz estar rodando no processo `pid`.
///
/// O Claude (2.1.x) registra cada processo em `~/.claude/sessions/<pid>.json`
/// com `pid`, `sessionId` e `startedAt`, e reescreve o `sessionId` a cada troca
/// de conversa (`/clear`, `/resume`). É o vínculo processo→conversa exato, sem
/// heurística de tempo — e por isso vence a [`choose_active_transcript`]
/// quando existe.
///
/// Só lê o arquivo do pid que a F1 detectou, nunca lista a pasta: os
/// registros dos outros `claude` da máquina não são assunto deste pane. E
/// trata o conteúdo como dado não confiável — `None`, e a heurística de
/// sempre, para qualquer coisa fora do esperado:
/// - não é arquivo comum (symlink, pasta, fifo) ou passa de
///   [`PID_FILE_MAX_BYTES`];
/// - o `pid` de dentro não é o pedido;
/// - `startedAt` é anterior ao nascimento do processo: o arquivo é de um
///   processo morto que tinha o mesmo pid (o Claude não apaga o registro
///   quando morre por sinal);
/// - o `sessionId` não passa por [`crate::agent::conversation::is_plausible`].
pub fn pid_file_conversation(sessions_dir: &Path, pid: u32, agent_start_ms: u64) -> Option<String> {
    let path = sessions_dir.join(format!("{pid}.json"));
    let file = open_regular_nofollow(&path)?;
    let mut raw = Vec::new();
    file.take(PID_FILE_MAX_BYTES + 1)
        .read_to_end(&mut raw)
        .ok()?;
    if raw.len() as u64 > PID_FILE_MAX_BYTES {
        return None;
    }
    let entry: Value = serde_json::from_slice(&raw).ok()?;
    if entry.get("pid").and_then(Value::as_u64) != Some(u64::from(pid)) {
        return None;
    }
    let started = entry.get("startedAt").and_then(Value::as_u64)?;
    if started.saturating_add(HEURISTIC_SLACK_MS) < agent_start_ms {
        return None;
    }
    let id = entry.get("sessionId").and_then(Value::as_str)?.trim();
    crate::agent::conversation::is_plausible(id).then(|| id.to_string())
}

/// Abre `path` só se for arquivo comum, decidido **pelo descritor aberto**:
/// checar o caminho e depois abrir deixaria a troca por uma FIFO entre os dois
/// passos travar o `open` — e com ele o poll de detecção de todos os panes.
/// `O_NOFOLLOW` recusa symlink, `O_NONBLOCK` faz FIFO abrir sem esperar
/// escritor (e ser recusada pelo `fstat` logo em seguida).
#[cfg(unix)]
fn open_regular_nofollow(path: &Path) -> Option<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    (meta.is_file() && meta.len() <= PID_FILE_MAX_BYTES).then_some(file)
}

#[cfg(not(unix))]
fn open_regular_nofollow(path: &Path) -> Option<File> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > PID_FILE_MAX_BYTES {
        return None;
    }
    File::open(path).ok()
}

/// Só lê a primeira mensagem dos candidatos frescos (ver [`is_fresh`]): os
/// frios nunca concorrem na heurística, e o poll re-resolve a cada tick — ler
/// o cabeçalho de todo transcript antigo do slug seria I/O jogado fora.
fn list_transcript_candidates(dir: &Path, agent_start_ms: u64) -> Vec<TranscriptCandidate> {
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
        let fresh = mtime_ms.saturating_add(HEURISTIC_SLACK_MS) >= agent_start_ms;
        out.push(TranscriptCandidate {
            first_msg_ms: fresh.then(|| first_message_ms(&path)).flatten(),
            path,
            mtime_ms,
            len: meta.len(),
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
        let Some((agent_type, description)) = read_agent_meta(sidecar, agent_id) else {
            continue;
        };
        out.push(DiscoveredSubagent {
            agent_id: agent_id.to_string(),
            agent_type,
            description,
        });
    }
    out
}

fn read_agent_meta(sidecar: &Path, agent_id: &str) -> Option<(String, Option<String>)> {
    let meta = sidecar.join(format!("agent-{agent_id}.meta.json"));
    let body = std::fs::read_to_string(meta).ok()?;
    let value = serde_json::from_str::<Value>(&body).ok()?;
    let agent_type = value
        .get("agentType")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)?;
    let description = value
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(str::to_string);
    Some((agent_type, description))
}

/// Bind provisório re-resolve pela heurística por ~90s (45 ticks × 2s da F1)
/// e então assenta: o candidato forte, quando existe, nasce nos primeiros
/// segundos (primeiro prompt do claude). Assentado, a heurística não mexe mais
/// no bind — só o crescimento do transcript o migra (ver
/// [`DiskObserver::observe`]). Melhor-esforço documentado da decisão 5.
const PROVISIONAL_SETTLE_TICKS: u32 = 45;

struct SessionObserver {
    parent: PathBuf,
    poll_stop: Arc<AtomicBool>,
    provisional: bool,
    stable_ticks: u32,
    /// Tamanho de cada transcript do slug no tick anterior: é a régua do
    /// "está crescendo".
    sizes: HashMap<PathBuf, u64>,
    /// Transcripts que já estavam sendo escritos quando o bind nasceu, fora o
    /// ligado — de outro `claude` vivo no mesmo cwd. Nunca viram alvo de
    /// migração, senão o painel pularia para a conversa do vizinho.
    foreign: HashSet<PathBuf>,
}

impl SessionObserver {
    fn tick_provisional(&mut self) {
        self.stable_ticks = self.stable_ticks.saturating_add(1);
        if self.stable_ticks >= PROVISIONAL_SETTLE_TICKS {
            self.provisional = false;
        }
    }
}

/// Callback que leva a [`Coordination`] devolvida pelo tracker de volta ao
/// layout — o mesmo auto-abrir de painel do caminho hook-driven.
pub type CoordinateFn = Arc<dyn Fn(SessionId, Coordination) + Send + Sync>;

/// Ponte disco→tracker por sessão de shell. Enquanto a F1 reporta um agente
/// numa sessão de shell, mantém uma thread que descobre subagentes por arquivo
/// e os sintetiza no [`SubagentTracker`] com os MESMOS métodos dos hooks — a
/// `Coordination` devolvida aciona o mesmo auto-abrir de painel via
/// [`CoordinateFn`]. Sem sessão observada não há thread nem varredura — custo
/// zero.
pub struct DiskObserver {
    sessions: Mutex<HashMap<SessionId, SessionObserver>>,
    coordinate: CoordinateFn,
}

impl Default for DiskObserver {
    fn default() -> Self {
        Self::with_coordinator(Arc::new(|_, _| {}))
    }
}

impl DiskObserver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_coordinator(coordinate: CoordinateFn) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            coordinate,
        }
    }

    /// A F1 reporta um agente Claude na sessão de shell: resolve o transcript
    /// ativo e passa a observar seu sidecar. Chamado a CADA tick enquanto o
    /// agente vive — o vínculo é com a conversa, não com o processo (M2 do
    /// claude-monitor). Devolve `true` quando o transcript ligado mudou.
    ///
    /// Duas réguas, nesta ordem:
    /// 1. **Crescimento.** O ligado parou e outro transcript do slug cresceu
    ///    neste tick (`/clear` abre um arquivo novo; `/resume` volta a escrever
    ///    num antigo): o bind migra para ele e o painel é ARQUIVADO — os
    ///    subagentes da conversa anterior ficam, nada é zerado. Transcript que
    ///    já estava fresco quando o bind nasceu é de outro `claude`
    ///    (`foreign`) e nunca é alvo.
    /// 2. **Heurística** ([`choose_active_transcript`]) para o primeiro bind e
    ///    para o provisório. Bind provisório é pegajoso: outra resolução
    ///    provisória não o desaloja (senão dois candidatos vivos fariam o
    ///    painel zerar em flapping) — só uma resolução forte rebinda, e depois
    ///    de [`PROVISIONAL_SETTLE_TICKS`] re-resoluções estáveis o bind assenta.
    ///    Bind forte a heurística não mexe mais.
    ///
    /// Antes das duas, o registro do próprio Claude ([`pid_file_conversation`]):
    /// com ele, o bind é exato e as réguas não entram — ver
    /// [`DiskObserver::observe_exact`].
    pub fn observe<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        tracker: &SharedSubagents,
        session: SessionId,
        cwd: &Path,
        exact: Option<&str>,
        agent_start_ms: u64,
    ) -> bool {
        let candidates = slug_candidates(cwd, agent_start_ms);
        self.observe_candidates(app, tracker, session, &candidates, exact, agent_start_ms)
    }

    fn observe_candidates<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        tracker: &SharedSubagents,
        session: SessionId,
        candidates: &[TranscriptCandidate],
        exact: Option<&str>,
        agent_start_ms: u64,
    ) -> bool {
        if let Some(conversation) = exact {
            return self.observe_exact(
                app,
                tracker,
                session,
                candidates,
                conversation,
                agent_start_ms,
            );
        }
        if let Some(grown) = self.growth_target(session, candidates) {
            self.bind(app, tracker, session, grown, false, true);
            self.seed(session, candidates, agent_start_ms, true);
            return true;
        }
        let changed = self.observe_with(app, tracker, session, || {
            choose_active_transcript(candidates, agent_start_ms)
        });
        if changed {
            self.seed(session, candidates, agent_start_ms, false);
        }
        changed
    }

    /// Bind pela conversa que o Claude registrou para o processo. Mesma
    /// semântica de painel das réguas: trocar de um bind assentado é migração
    /// (arquiva), corrigir um palpite provisório ou ligar um agente novo zera.
    ///
    /// Conversa conhecida cujo transcript ainda não existe (conversa nova, antes
    /// da primeira mensagem) **espera**: o bind atual fica, e nenhum palpite da
    /// heurística entra no lugar — ela erraria justamente o que já se sabe.
    /// A régua de tamanhos segue atualizada, para o caso de o registro sumir e
    /// a régua de crescimento voltar a valer no tick seguinte.
    fn observe_exact<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        tracker: &SharedSubagents,
        session: SessionId,
        candidates: &[TranscriptCandidate],
        conversation: &str,
        agent_start_ms: u64,
    ) -> bool {
        let target = candidates
            .iter()
            .find(|c| c.path.file_stem().and_then(|s| s.to_str()) == Some(conversation))
            .map(|c| c.path.clone());
        let current = {
            let mut map = self.sessions.lock().expect("disk observer lock");
            let live = map
                .get_mut(&session)
                .filter(|o| !o.poll_stop.load(Ordering::SeqCst));
            match live {
                Some(observer) => {
                    observer.sizes = candidates.iter().map(|c| (c.path.clone(), c.len)).collect();
                    if target.as_ref() == Some(&observer.parent) {
                        observer.provisional = false;
                        return false;
                    }
                    Some(observer.provisional)
                }
                None => None,
            }
        };
        let Some(target) = target else {
            return false;
        };
        let archive = current == Some(false);
        self.bind(app, tracker, session, target, false, archive);
        self.seed(session, candidates, agent_start_ms, archive);
        true
    }

    /// O transcript para onde o bind deve migrar, ou `None`. Atualiza a régua
    /// de tamanhos em todo tick. Só migra quando o ligado NÃO cresceu: com os
    /// dois crescendo juntos (o `claude` ainda fecha a conversa antiga), espera
    /// o próximo tick.
    fn growth_target(
        &self,
        session: SessionId,
        candidates: &[TranscriptCandidate],
    ) -> Option<PathBuf> {
        let mut map = self.sessions.lock().expect("disk observer lock");
        let observer = map.get_mut(&session)?;
        if observer.poll_stop.load(Ordering::SeqCst) {
            return None;
        }
        let sizes = &observer.sizes;
        let grew = |c: &TranscriptCandidate| c.len > sizes.get(&c.path).copied().unwrap_or(0);
        let bound_grew = candidates
            .iter()
            .any(|c| c.path == observer.parent && grew(c));
        let target = if bound_grew {
            None
        } else {
            candidates
                .iter()
                .filter(|c| c.path != observer.parent && !observer.foreign.contains(&c.path))
                .filter(|c| grew(c))
                .max_by_key(|c| c.mtime_ms)
                .map(|c| c.path.clone())
        };
        observer.sizes = candidates.iter().map(|c| (c.path.clone(), c.len)).collect();
        target
    }

    /// Fotografa o slug logo depois de um bind: os tamanhos viram a régua do
    /// próximo tick e, num bind novo (não migração), o que já estava fresco
    /// vira `foreign`. Migração mantém o `foreign` do bind original — a
    /// conversa anterior do PRÓPRIO agente continua elegível (`/resume` nela
    /// de volta).
    fn seed(
        &self,
        session: SessionId,
        candidates: &[TranscriptCandidate],
        agent_start_ms: u64,
        keep_foreign: bool,
    ) {
        let mut map = self.sessions.lock().expect("disk observer lock");
        let Some(observer) = map.get_mut(&session) else {
            return;
        };
        observer.sizes = candidates.iter().map(|c| (c.path.clone(), c.len)).collect();
        if !keep_foreign {
            let parent = observer.parent.clone();
            observer.foreign = candidates
                .iter()
                .filter(|c| c.path != parent && is_fresh(c, agent_start_ms))
                .map(|c| c.path.clone())
                .collect();
        }
    }

    /// `true` enquanto uma thread de varredura está viva para a sessão. `false`
    /// tanto para sessão nunca observada quanto para uma congelada.
    pub fn is_observing(&self, session: SessionId) -> bool {
        self.sessions
            .lock()
            .expect("disk observer lock")
            .get(&session)
            .map(|observer| !observer.poll_stop.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// Id da conversa do transcript ligado enquanto a observação está viva — é
    /// o que o "Reabrir com gate" retoma (M1 do claude-monitor). Observação
    /// congelada devolve `None`: o bind é de um agente que já sumiu, e retomar
    /// a conversa dele no lugar da do agente atual seria trocar de conversa às
    /// cegas. O id já sai validado por [`crate::agent::conversation`].
    pub fn bound_conversation(&self, session: SessionId) -> Option<String> {
        let parent = self
            .sessions
            .lock()
            .expect("disk observer lock")
            .get(&session)
            .filter(|observer| !observer.poll_stop.load(Ordering::SeqCst))
            .map(|observer| observer.parent.clone())?;
        crate::agent::conversation::from_transcript(&parent)
    }

    /// `true` só com uma thread viva num bind forte. Só os testes perguntam:
    /// desde o M2 o poll da F1 chama `observe` em todo tick, assentado ou não.
    #[cfg(test)]
    pub fn is_settled(&self, session: SessionId) -> bool {
        self.sessions
            .lock()
            .expect("disk observer lock")
            .get(&session)
            .map(|observer| !observer.poll_stop.load(Ordering::SeqCst) && !observer.provisional)
            .unwrap_or(false)
    }

    fn observe_with<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        tracker: &SharedSubagents,
        session: SessionId,
        resolve: impl FnOnce() -> Option<ActiveTranscript>,
    ) -> bool {
        let Some(active) = resolve() else {
            return false;
        };
        {
            let mut map = self.sessions.lock().expect("disk observer lock");
            if let Some(observer) = map
                .get_mut(&session)
                .filter(|o| !o.poll_stop.load(Ordering::SeqCst))
            {
                if observer.parent == active.path {
                    if !active.provisional {
                        observer.provisional = false;
                    } else if observer.provisional {
                        observer.tick_provisional();
                    }
                    return false;
                }
                // Bind forte só migra por crescimento (`observe`): a heurística
                // mudar de ideia — o empate por mtime oscila a cada escrita —
                // não é troca de conversa.
                if !observer.provisional {
                    return false;
                }
                if active.provisional {
                    observer.tick_provisional();
                    return false;
                }
            }
        }
        // Provisório corrigido por um forte, ou agente novo depois de outro ter
        // sumido: o palpite anterior não era desta conversa, painel limpo.
        self.bind(
            app,
            tracker,
            session,
            active.path,
            active.provisional,
            false,
        );
        true
    }

    /// Liga a sessão a `parent` e sobe a varredura do sidecar dele, parando a
    /// anterior. `archive` decide o painel: `true` (migração de conversa) o
    /// mantém — os subagentes da conversa anterior continuam lá, com seus
    /// watchers terminando sozinhos —; `false` (palpite corrigido, agente
    /// novo) o zera como antes.
    fn bind<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        tracker: &SharedSubagents,
        session: SessionId,
        parent: PathBuf,
        provisional: bool,
        archive: bool,
    ) {
        let foreign = {
            let mut map = self.sessions.lock().expect("disk observer lock");
            map.get_mut(&session).map(|observer| {
                observer.poll_stop.store(true, Ordering::SeqCst);
                std::mem::take(&mut observer.foreign)
            })
        };
        if !archive {
            tracker.remove_session(app, session);
        }
        tracker.register_session(session);
        let poll_stop = Arc::new(AtomicBool::new(false));
        spawn_sidecar_poll(
            app.clone(),
            Arc::clone(tracker),
            session,
            parent.clone(),
            Arc::clone(&poll_stop),
            Arc::clone(&self.coordinate),
        );
        let foreign = if archive {
            foreign.unwrap_or_default()
        } else {
            HashSet::new()
        };
        self.sessions.lock().expect("disk observer lock").insert(
            session,
            SessionObserver {
                parent,
                poll_stop,
                provisional,
                stable_ticks: 0,
                sizes: HashMap::new(),
                foreign,
            },
        );
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
    coordinate: CoordinateFn,
) {
    let sidecar = sidecar_dir(&parent);
    let spawned = std::thread::Builder::new()
        .name("disk-observer-sidecar".into())
        .spawn(move || {
            // Painel arquivado (migração de volta para uma conversa já vista)
            // já tem estes subagentes: re-anunciá-los duplicaria a entrada,
            // porque `promote` não deduplica por `agent_id`.
            let mut known: HashSet<String> = tracker
                .snapshot(session)
                .subagents
                .into_iter()
                .filter_map(|run| run.agent_id)
                .collect();
            while !poll_stop.load(Ordering::SeqCst) {
                for coordination in
                    poll_once(&app, &tracker, session, &sidecar, &parent, &mut known)
                {
                    coordinate(session, coordination);
                }
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
) -> Vec<Coordination> {
    let mut coordinations = Vec::new();
    for found in scan_new_subagents(sidecar, known) {
        known.insert(found.agent_id.clone());
        if let Some(coordination) = tracker.on_subagent_started(
            app,
            session,
            found.agent_id,
            found.agent_type,
            found.description,
            Some(parent),
        ) {
            coordinations.push(coordination);
        }
    }
    coordinations
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
            len: 0,
        }
    }

    fn strong(path: &str) -> Option<ActiveTranscript> {
        Some(ActiveTranscript {
            path: PathBuf::from(path),
            provisional: false,
        })
    }

    fn provisional(path: &str) -> Option<ActiveTranscript> {
        Some(ActiveTranscript {
            path: PathBuf::from(path),
            provisional: true,
        })
    }

    #[test]
    fn choose_without_usable_timestamps_falls_back_to_newest_mtime_as_provisional() {
        let candidates = vec![
            candidate("/a.jsonl", 100, None),
            candidate("/b.jsonl", 300, None),
            candidate("/c.jsonl", 200, None),
        ];
        assert_eq!(
            choose_active_transcript(&candidates, 1_000),
            provisional("/b.jsonl")
        );
    }

    #[test]
    fn time_heuristic_picks_the_transcript_born_just_after_the_agent() {
        // O dono anterior do cwd (old.jsonl) tem o mtime mais novo, mas a
        // primeira mensagem é bem antes do agente nascer; o transcript certo
        // (new.jsonl) surge logo após o start e vence, mesmo com mtime mais
        // velho.
        let candidates = vec![
            candidate("/old.jsonl", 9_999, Some(1_000)),
            candidate("/new.jsonl", 5_000, Some(10_500)),
        ];
        assert_eq!(
            choose_active_transcript(&candidates, 10_000),
            strong("/new.jsonl")
        );
    }

    #[test]
    fn two_near_simultaneous_transcripts_are_ambiguous_and_fall_back_to_newest() {
        // Dois claude quase simultâneos no mesmo cwd: os dois começam logo após o
        // start e são indistinguíveis pela heurística ⇒ melhor-esforço (mtime).
        let candidates = vec![
            candidate("/one.jsonl", 14_000, Some(10_300)),
            candidate("/two.jsonl", 17_000, Some(10_600)),
        ];
        assert_eq!(
            choose_active_transcript(&candidates, 10_000),
            strong("/two.jsonl")
        );
    }

    #[test]
    fn ambiguous_fallback_stays_within_post_agent_candidates() {
        // Um claude antigo ainda ativo no mesmo cwd (stale) tem o mtime mais
        // novo, mas começou antes do start: o desempate por mtime do caso
        // ambíguo não pode escolhê-lo — fica entre os que passaram o filtro.
        let candidates = vec![
            candidate("/stale.jsonl", 99_999, Some(1_000)),
            candidate("/one.jsonl", 14_000, Some(10_300)),
            candidate("/two.jsonl", 17_000, Some(10_600)),
        ];
        assert_eq!(
            choose_active_transcript(&candidates, 10_000),
            strong("/two.jsonl")
        );
    }

    #[test]
    fn choose_on_empty_is_none() {
        assert_eq!(choose_active_transcript(&[], 10_000), None);
    }

    #[test]
    fn transcripts_started_before_the_agent_are_ignored_by_the_heuristic() {
        // O dono anterior do cwd segue vivo (mtime fresco), mas a primeira
        // mensagem dele é pré-agente: a heurística fica com o transcript datado
        // pós-agente, mesmo com mtime mais velho.
        let candidates = vec![
            candidate("/stale.jsonl", 50_900, Some(2_000)),
            candidate("/live.jsonl", 50_200, Some(50_100)),
        ];
        assert_eq!(
            choose_active_transcript(&candidates, 50_000),
            strong("/live.jsonl")
        );
    }

    #[test]
    fn startup_window_with_only_stale_candidates_resolves_none() {
        // Claude nasceu e ainda não gravou o transcript: o slug só tem sessões
        // frias de outros claude. Devolver a mais nova travaria o observer no
        // transcript errado — a resposta certa é None, e o poll da F1 re-tenta
        // até o transcript novo existir.
        let candidates = vec![
            candidate("/old-timed.jsonl", 20_000, Some(19_000)),
            candidate("/old-untimed.jsonl", 30_000, None),
        ];
        assert_eq!(choose_active_transcript(&candidates, 100_000), None);
    }

    #[test]
    fn resumed_transcript_with_old_timestamps_but_fresh_mtime_wins_as_provisional() {
        // `claude --resume`: as linhas preservam timestamps antigos, mas o
        // arquivo volta a ser escrito agora — o mtime fresco o mantém elegível
        // no melhor-esforço, marcado provisório para re-resolução.
        let candidates = vec![
            candidate("/resumed.jsonl", 100_800, Some(1_000)),
            candidate("/cold.jsonl", 20_000, Some(19_000)),
        ];
        assert_eq!(
            choose_active_transcript(&candidates, 100_000),
            provisional("/resumed.jsonl")
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
        let candidates = list_transcript_candidates(dir.path(), 0);
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
        assert!(list_transcript_candidates(Path::new("/nope/does/not/exist"), 0).is_empty());
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
                description: Some("revisar diff".into()),
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

        let coordinations = poll_once(
            &handle,
            &tracker,
            session,
            sidecar.path(),
            &parent,
            &mut known,
        );
        assert_eq!(coordinations.len(), 1);
        assert!(!coordinations[0].panel_disarmed);

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
            None,
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

        let resolved = strong_transcript(&parent);
        observer.observe_with(&handle, &tracker, session, move || Some(resolved));
        assert!(observer.is_observing(session));
        assert!(observer.is_settled(session));
        assert!(wait_for_subagent(&tracker, session, "a1b2c3d4e5"));

        observer.retain_live(&handle, &tracker, &HashSet::new());
        assert!(!observer.is_observing(session));
    }

    #[test]
    fn resumed_session_end_to_end_binds_provisionally_and_captures_new_sidecars() {
        // O cenário do E2E de 2026-07-24: `claude --continue` numa sessão de
        // ONTEM. O slug só tem transcripts frios; o retomado volta a ser
        // escrito (mtime fresco, timestamps antigos) e os sidecars novos caem
        // no diretório ANTIGO. O fluxo real: resolve → None (tudo frio) →
        // retry → bind provisório no retomado → thread captura os sidecars.
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();
        let slug = TempDir::new().unwrap();
        let resumed = slug.path().join("resumed.jsonl");
        let cold = slug.path().join("cold.jsonl");
        write(
            &resumed,
            "{\"type\":\"user\",\"timestamp\":\"2026-07-23T18:28:00.000Z\",\"message\":{}}\n",
        );
        write(
            &cold,
            "{\"type\":\"user\",\"timestamp\":\"2026-07-23T01:18:00.000Z\",\"message\":{}}\n",
        );
        let old = "202607231528";
        for path in [&resumed, &cold] {
            let status = std::process::Command::new("touch")
                .args(["-t", old])
                .arg(path)
                .status()
                .expect("touch");
            assert!(status.success());
        }
        let sidecar = resumed.with_extension("").join("subagents");
        fs::create_dir_all(&sidecar).unwrap();
        write(&sidecar.join("agent-a13c17fb51.jsonl"), "{}");
        write(
            &sidecar.join("agent-a13c17fb51.meta.json"),
            r#"{"agentType":"explorer"}"#,
        );

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Tick 1: agente acabou de nascer, nada foi escrito ainda — todos os
        // candidatos são frios ⇒ nenhum bind, o retry fica armado.
        let candidates = list_transcript_candidates(slug.path(), now_ms);
        observer.observe_with(&handle, &tracker, session, || {
            choose_active_transcript(&candidates, now_ms)
        });
        assert!(!observer.is_observing(session));

        // O claude retomado escreve: mtime do transcript fica fresco.
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&resumed)
            .unwrap();
        writeln!(file, r#"{{"type":"assistant","message":{{}}}}"#).unwrap();
        drop(file);

        // Tick 2: o retomado vira o único plausível ⇒ bind provisório, e a
        // thread captura o sidecar novo do diretório antigo.
        let candidates = list_transcript_candidates(slug.path(), now_ms);
        let chosen = choose_active_transcript(&candidates, now_ms);
        assert_eq!(
            chosen,
            Some(ActiveTranscript {
                path: resumed.clone(),
                provisional: true
            })
        );
        observer.observe_with(&handle, &tracker, session, || chosen.clone());
        assert!(observer.is_observing(session));
        assert!(!observer.is_settled(session));
        assert!(wait_for_subagent(&tracker, session, "a13c17fb51"));

        observer.retain_live(&handle, &tracker, &HashSet::new());
    }

    #[test]
    fn provisional_bind_is_sticky_until_a_strong_resolution() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();
        let dir = TempDir::new().unwrap();
        let first = dir.path().join("first.jsonl");
        let second = dir.path().join("second.jsonl");
        write(&first, "{}");
        write(&second, "{}");

        let bind = provisional_transcript(&first);
        observer.observe_with(&handle, &tracker, session, move || Some(bind));
        assert!(observer.is_observing(session));
        assert!(!observer.is_settled(session));

        // Outra resolução provisória (candidato vivo concorrente) não desaloja
        // o bind — senão o painel zeraria em flapping a cada poll.
        let rival = provisional_transcript(&second);
        observer.observe_with(&handle, &tracker, session, move || Some(rival));
        assert_eq!(observer.bound_parent(session), Some(first.clone()));

        // Uma resolução forte rebinda.
        let settled = strong_transcript(&second);
        observer.observe_with(&handle, &tracker, session, move || Some(settled));
        assert_eq!(observer.bound_parent(session), Some(second));
        assert!(observer.is_settled(session));

        observer.retain_live(&handle, &tracker, &HashSet::new());
    }

    #[test]
    fn provisional_bind_settles_after_stable_reresolutions() {
        // `--resume` nunca ganha candidato forte: depois da janela de
        // assentamento o bind vira definitivo e o re-resolve (I/O no slug)
        // para — performance-first.
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");

        let bind = provisional_transcript(&parent);
        observer.observe_with(&handle, &tracker, session, move || Some(bind));
        for _ in 0..PROVISIONAL_SETTLE_TICKS {
            assert!(!observer.is_settled(session));
            let again = provisional_transcript(&parent);
            observer.observe_with(&handle, &tracker, session, move || Some(again));
        }
        assert!(observer.is_settled(session));
        assert_eq!(observer.bound_parent(session), Some(parent));

        observer.retain_live(&handle, &tracker, &HashSet::new());
    }

    #[test]
    fn provisional_bind_upgrades_in_place_when_the_same_path_resolves_strong() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("session.jsonl");
        write(&parent, "{}");

        let bind = provisional_transcript(&parent);
        observer.observe_with(&handle, &tracker, session, move || Some(bind));
        assert!(!observer.is_settled(session));

        let upgraded = strong_transcript(&parent);
        observer.observe_with(&handle, &tracker, session, move || Some(upgraded));
        assert!(observer.is_settled(session));
        assert_eq!(observer.bound_parent(session), Some(parent));

        observer.retain_live(&handle, &tracker, &HashSet::new());
    }

    #[test]
    fn observe_coordinates_the_panel_when_a_subagent_appears() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let fired: Arc<Mutex<Vec<SessionId>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&fired);
        let observer = DiskObserver::with_coordinator(Arc::new(move |session, _| {
            sink.lock().unwrap().push(session);
        }));
        let session = SessionId::new_v4();
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

        observer.begin(&handle, &tracker, session, parent);
        assert!(wait_for_subagent(&tracker, session, "a1b2c3d4e5"));
        let coordinated = (0..200).any(|_| {
            if fired.lock().unwrap().contains(&session) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
            false
        });
        assert!(coordinated);

        observer.retain_live(&handle, &tracker, &HashSet::new());
    }

    #[test]
    fn bound_conversation_is_the_live_transcript_id_and_none_once_frozen() {
        // M1: o "Reabrir com gate" retoma a conversa ligada. Fixture
        // sintética: id escrito à mão, nada copiado de ~/.claude/projects.
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();
        assert_eq!(observer.bound_conversation(session), None);

        let dir = TempDir::new().unwrap();
        let id = "11111111-2222-3333-4444-555555555555";
        let parent = dir.path().join(format!("{id}.jsonl"));
        write(
            &parent,
            &format!("{{\"type\":\"user\",\"sessionId\":\"{id}\",\"message\":{{}}}}\n"),
        );
        observer.begin(&handle, &tracker, session, parent);
        assert_eq!(observer.bound_conversation(session).as_deref(), Some(id));

        // Agente sumiu: o bind congelado é de outro processo, nunca se retoma.
        observer.stop(&handle, &tracker, session);
        assert_eq!(observer.bound_conversation(session), None);

        observer.retain_live(&handle, &tracker, &HashSet::new());
    }

    #[test]
    fn bound_conversation_rejects_an_implausible_id() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();
        let dir = TempDir::new().unwrap();
        // Nem o `sessionId` interno nem o nome do arquivo são id plausível: o
        // `-` inicial viraria opção no argv do claude.
        let parent = dir.path().join("--dangerously-x.jsonl");
        write(
            &parent,
            "{\"type\":\"user\",\"sessionId\":\"--dangerously-skip-permissions\"}\n",
        );
        observer.begin(&handle, &tracker, session, parent);
        assert_eq!(observer.bound_conversation(session), None);

        observer.retain_live(&handle, &tracker, &HashSet::new());
    }

    // --- M2 do claude-monitor: o vínculo segue a conversa que cresce ---
    //
    // Fixtures sintéticas, escritas à mão: ids e conteúdo inventados, nada
    // copiado de ~/.claude/projects.

    const CONV_A: &str = "aaaaaaaa-0000-4000-8000-000000000001";
    const CONV_B: &str = "bbbbbbbb-0000-4000-8000-000000000002";
    const CONV_C: &str = "cccccccc-0000-4000-8000-000000000003";

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    fn append_turn(slug: &Path, conversation: &str) -> PathBuf {
        use std::io::Write as _;
        let path = slug.join(format!("{conversation}.jsonl"));
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(
            file,
            r#"{{"type":"user","sessionId":"{conversation}","timestamp":"{}","message":{{"content":"oi"}}}}"#,
            chrono::Utc::now().to_rfc3339()
        )
        .unwrap();
        path
    }

    struct Tick<'a> {
        observer: &'a DiskObserver,
        handle: &'a AppHandle<tauri::test::MockRuntime>,
        tracker: &'a SharedSubagents,
        session: SessionId,
        slug: &'a Path,
        start: u64,
    }

    impl Tick<'_> {
        fn run(&self) -> bool {
            let candidates = list_transcript_candidates(self.slug, self.start);
            self.observer.observe_candidates(
                self.handle,
                self.tracker,
                self.session,
                &candidates,
                None,
                self.start,
            )
        }

        fn run_exact(&self, conversation: &str) -> bool {
            let candidates = list_transcript_candidates(self.slug, self.start);
            self.observer.observe_candidates(
                self.handle,
                self.tracker,
                self.session,
                &candidates,
                Some(conversation),
                self.start,
            )
        }
    }

    #[test]
    fn clear_migrates_the_bind_to_the_growing_transcript_and_archives_the_panel() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();
        let slug = TempDir::new().unwrap();
        let tick = Tick {
            observer: &observer,
            handle: &handle,
            tracker: &tracker,
            session,
            slug: slug.path(),
            start: now_ms().saturating_sub(1_000),
        };

        // O claude nasceu e escreveu a conversa A: bind forte nela.
        let a = append_turn(slug.path(), CONV_A);
        assert!(tick.run());
        assert_eq!(observer.bound_parent(session), Some(a.clone()));
        tracker.on_subagent_started(
            &handle,
            session,
            "a1b2c3d4e5".into(),
            "explorer".into(),
            None,
            Some(&a),
        );

        // `/clear`: a conversa B nasce e cresce, a A para.
        let b = append_turn(slug.path(), CONV_B);
        assert!(tick.run());
        assert_eq!(observer.bound_parent(session), Some(b.clone()));
        assert_eq!(
            observer.bound_conversation(session).as_deref(),
            Some(CONV_B)
        );
        // Arquivado, nunca zerado: o subagente da conversa A segue no painel.
        assert!(tracker
            .snapshot(session)
            .subagents
            .iter()
            .any(|run| run.agent_id.as_deref() == Some("a1b2c3d4e5")));

        // Nada cresceu: a heurística (que prefere A, mais perto do start) não
        // desfaz a migração.
        assert!(!tick.run());
        assert_eq!(observer.bound_parent(session), Some(b));

        // `/resume` de volta para A: A cresce, B parou — o bind volta, sem
        // duplicar o subagente que o painel arquivado já tinha.
        append_turn(slug.path(), CONV_A);
        assert!(tick.run());
        assert_eq!(observer.bound_parent(session), Some(a));
        std::thread::sleep(SIDECAR_POLL_INTERVAL + Duration::from_millis(200));
        let runs = tracker.snapshot(session).subagents;
        assert_eq!(
            runs.iter()
                .filter(|run| run.agent_id.as_deref() == Some("a1b2c3d4e5"))
                .count(),
            1
        );

        observer.retain_live(&handle, &tracker, &HashSet::new());
    }

    #[test]
    fn bind_stays_while_the_bound_transcript_is_still_growing() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();
        let slug = TempDir::new().unwrap();
        let tick = Tick {
            observer: &observer,
            handle: &handle,
            tracker: &tracker,
            session,
            slug: slug.path(),
            start: now_ms().saturating_sub(1_000),
        };

        let a = append_turn(slug.path(), CONV_A);
        assert!(tick.run());
        // Os dois crescem no mesmo tick (o claude ainda fecha a conversa
        // antiga): espera, não migra.
        append_turn(slug.path(), CONV_A);
        append_turn(slug.path(), CONV_B);
        assert!(!tick.run());
        assert_eq!(observer.bound_parent(session), Some(a));

        observer.retain_live(&handle, &tracker, &HashSet::new());
    }

    #[test]
    fn transcript_already_live_at_bind_never_steals_it() {
        // Outro claude no mesmo cwd já escrevia C quando o bind em A nasceu:
        // C crescer com A parado (o dono pensando) não é troca de conversa.
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();
        let slug = TempDir::new().unwrap();
        let start = now_ms().saturating_sub(1_000);
        let tick = Tick {
            observer: &observer,
            handle: &handle,
            tracker: &tracker,
            session,
            slug: slug.path(),
            start,
        };

        append_turn(slug.path(), CONV_C);
        let a = append_turn(slug.path(), CONV_A);
        let candidates = list_transcript_candidates(slug.path(), start);
        let bind = strong_transcript(&a);
        assert!(observer.observe_with(&handle, &tracker, session, move || Some(bind)));
        observer.seed(session, &candidates, start, false);

        append_turn(slug.path(), CONV_C);
        assert!(!tick.run());
        assert_eq!(observer.bound_parent(session), Some(a));

        observer.retain_live(&handle, &tracker, &HashSet::new());
    }

    // --- `~/.claude/sessions/<pid>.json`: vínculo exato processo→conversa ---
    //
    // Registros sintéticos, escritos à mão com os campos que o Claude grava;
    // nenhum copiado de ~/.claude/sessions.

    const PID: u32 = 4242;

    fn write_pid_file(dir: &Path, pid: u32, body: &str) {
        fs::write(dir.join(format!("{pid}.json")), body).unwrap();
    }

    fn pid_record(pid: u32, conversation: &str, started_at: u64) -> String {
        format!(
            r#"{{"pid":{pid},"sessionId":"{conversation}","cwd":"/tmp/projeto","startedAt":{started_at},"kind":"interactive","entrypoint":"cli","status":"idle"}}"#
        )
    }

    #[test]
    fn pid_file_gives_the_conversation_of_the_detected_process() {
        let dir = TempDir::new().unwrap();
        write_pid_file(dir.path(), PID, &pid_record(PID, CONV_A, 10_400));
        assert_eq!(
            pid_file_conversation(dir.path(), PID, 10_000).as_deref(),
            Some(CONV_A)
        );
    }

    #[test]
    fn pid_file_absent_or_of_another_pid_is_none() {
        let dir = TempDir::new().unwrap();
        assert_eq!(pid_file_conversation(dir.path(), PID, 10_000), None);
        // O nome diz 4242 e o conteúdo diz outro processo: não confia.
        write_pid_file(dir.path(), PID, &pid_record(7, CONV_A, 10_400));
        assert_eq!(pid_file_conversation(dir.path(), PID, 10_000), None);
    }

    #[test]
    fn pid_file_older_than_the_process_is_a_recycled_pid() {
        let dir = TempDir::new().unwrap();
        // Registro de um claude morto há horas que tinha o mesmo pid.
        write_pid_file(dir.path(), PID, &pid_record(PID, CONV_A, 1_000));
        assert_eq!(pid_file_conversation(dir.path(), PID, 3_600_000), None);
    }

    #[test]
    fn pid_file_with_an_implausible_id_or_garbage_is_none() {
        let dir = TempDir::new().unwrap();
        write_pid_file(
            dir.path(),
            PID,
            &pid_record(PID, "--dangerously-skip-permissions", 10_400),
        );
        assert_eq!(pid_file_conversation(dir.path(), PID, 10_000), None);
        write_pid_file(dir.path(), PID, "{\"pid\": 4242, \"sessionId\": ");
        assert_eq!(pid_file_conversation(dir.path(), PID, 10_000), None);
        write_pid_file(dir.path(), PID, r#"{"pid":4242,"startedAt":10400}"#);
        assert_eq!(pid_file_conversation(dir.path(), PID, 10_000), None);
    }

    #[test]
    fn pid_file_over_the_size_cap_is_not_read() {
        let dir = TempDir::new().unwrap();
        let padding = "x".repeat(PID_FILE_MAX_BYTES as usize);
        let body = format!(
            r#"{{"pid":{PID},"sessionId":"{CONV_A}","startedAt":10400,"pad":"{padding}"}}"#
        );
        write_pid_file(dir.path(), PID, &body);
        assert_eq!(pid_file_conversation(dir.path(), PID, 10_000), None);
    }

    #[cfg(unix)]
    #[test]
    fn pid_file_that_is_a_symlink_is_not_followed() {
        let dir = TempDir::new().unwrap();
        let elsewhere = dir.path().join("outro.json");
        fs::write(&elsewhere, pid_record(PID, CONV_A, 10_400)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.path().join(format!("{PID}.json"))).unwrap();
        assert_eq!(pid_file_conversation(dir.path(), PID, 10_000), None);
    }

    #[cfg(unix)]
    #[test]
    fn pid_file_that_is_a_fifo_is_refused_without_blocking() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(format!("{PID}.json"));
        let c = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        // Sem escritor: um `open` bloqueante pendurava aqui para sempre.
        assert_eq!(pid_file_conversation(dir.path(), PID, 10_000), None);
    }

    #[test]
    fn exact_conversation_beats_the_heuristic_and_follows_clear() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();
        let slug = TempDir::new().unwrap();
        let tick = Tick {
            observer: &observer,
            handle: &handle,
            tracker: &tracker,
            session,
            slug: slug.path(),
            start: now_ms().saturating_sub(1_000),
        };

        // Dois transcripts frescos no slug — a heurística teria de adivinhar.
        // O registro do processo diz B: o bind é B, forte.
        append_turn(slug.path(), CONV_A);
        let b = append_turn(slug.path(), CONV_B);
        assert!(tick.run_exact(CONV_B));
        assert_eq!(observer.bound_parent(session), Some(b.clone()));
        assert!(observer.is_settled(session));
        tracker.on_subagent_started(
            &handle,
            session,
            "b1b2c3d4e5".into(),
            "explorer".into(),
            None,
            Some(&b),
        );

        // A cresce (outro claude no mesmo cwd), o registro segue em B: fica.
        append_turn(slug.path(), CONV_A);
        assert!(!tick.run_exact(CONV_B));
        assert_eq!(observer.bound_parent(session), Some(b.clone()));

        // `/clear`: o registro já diz C, mas o transcript de C ainda não
        // existe. Espera em B — nenhum palpite entra no lugar.
        assert!(!tick.run_exact(CONV_C));
        assert_eq!(observer.bound_parent(session), Some(b));

        // C ganhou a primeira mensagem: migra e ARQUIVA o painel.
        let c = append_turn(slug.path(), CONV_C);
        assert!(tick.run_exact(CONV_C));
        assert_eq!(observer.bound_parent(session), Some(c));
        assert_eq!(
            observer.bound_conversation(session).as_deref(),
            Some(CONV_C)
        );
        assert!(tracker
            .snapshot(session)
            .subagents
            .iter()
            .any(|run| run.agent_id.as_deref() == Some("b1b2c3d4e5")));

        observer.retain_live(&handle, &tracker, &HashSet::new());
    }

    #[test]
    fn exact_conversation_corrects_a_provisional_guess() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = Arc::new(SubagentTracker::new());
        let observer = DiskObserver::new();
        let session = SessionId::new_v4();
        let slug = TempDir::new().unwrap();
        let tick = Tick {
            observer: &observer,
            handle: &handle,
            tracker: &tracker,
            session,
            slug: slug.path(),
            start: now_ms().saturating_sub(1_000),
        };
        let a = append_turn(slug.path(), CONV_A);
        let b = append_turn(slug.path(), CONV_B);
        let guess = provisional_transcript(&a);
        assert!(observer.observe_with(&handle, &tracker, session, move || Some(guess)));

        assert!(tick.run_exact(CONV_B));
        assert_eq!(observer.bound_parent(session), Some(b));
        assert!(observer.is_settled(session));

        observer.retain_live(&handle, &tracker, &HashSet::new());
    }

    fn strong_transcript(path: &Path) -> ActiveTranscript {
        ActiveTranscript {
            path: path.to_path_buf(),
            provisional: false,
        }
    }

    fn provisional_transcript(path: &Path) -> ActiveTranscript {
        ActiveTranscript {
            path: path.to_path_buf(),
            provisional: true,
        }
    }

    impl DiskObserver {
        fn begin<R: Runtime>(
            &self,
            app: &AppHandle<R>,
            tracker: &SharedSubagents,
            session: SessionId,
            parent: PathBuf,
        ) {
            let bind = strong_transcript(&parent);
            self.observe_with(app, tracker, session, move || Some(bind));
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

        fn bound_parent(&self, session: SessionId) -> Option<PathBuf> {
            self.sessions
                .lock()
                .unwrap()
                .get(&session)
                .map(|o| o.parent.clone())
        }
    }
}
