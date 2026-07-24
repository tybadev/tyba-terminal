use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;

use crate::session::{AgentRunnerKind, SessionId};

use super::runner_binary;

pub const EVENT_CHANGED: &str = "agent-detected://changed";
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Um processo visto num único scan: identidade + o elo de parentesco que a
/// varredura da árvore segue. `start_ms` é o instante de nascimento em
/// milissegundos de época (wall-clock), melhor-esforço por plataforma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcRow {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub start_ms: u64,
}

/// O agente encontrado rodando sob uma sessão de shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectedAgent {
    pub pid: u32,
    pub start_ms: u64,
    pub kind: AgentRunnerKind,
}

#[derive(Clone, Serialize)]
pub struct AgentDetectedPayload {
    pub session_id: SessionId,
    pub detected: Option<DetectedAgent>,
}

/// Mapeia o nome do processo para o runner conhecido, reusando `runner_binary`
/// como fonte única dos nomes de binário de agente (`claude`, `codex`).
fn agent_kind_for_comm(comm: &str) -> Option<AgentRunnerKind> {
    [AgentRunnerKind::ClaudeCode, AgentRunnerKind::Codex]
        .into_iter()
        .find(|kind| runner_binary(kind) == Some(comm))
}

/// Nome de arquivo que parece versão (`2.1.218`): só dígitos e pontos, com
/// pelo menos um ponto. É a cara do binário real do instalador nativo do
/// Claude Code (`~/.local/share/claude/versions/<versão>`).
fn looks_like_version(name: &str) -> bool {
    name.contains('.') && !name.is_empty() && name.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Resolve o runner pelo CAMINHO do executável — o comm do kernel é o nome do
/// arquivo executado, e o launcher nativo do Claude Code executa
/// `.../claude/versions/2.1.218`, então o comm vira `2.1.218` e o casamento
/// por nome falha (foi exatamente o furo do E2E de 2026-07-24). Regras:
/// basename igual ao binário do runner, ou basename versão-like com um
/// diretório ancestral com o nome do runner.
fn kind_from_exec_path(path: &std::path::Path) -> Option<AgentRunnerKind> {
    let file = path.file_name()?.to_str()?;
    if let Some(kind) = agent_kind_for_comm(file) {
        return Some(kind);
    }
    if !looks_like_version(file) {
        return None;
    }
    let parent = path.parent()?;
    [AgentRunnerKind::ClaudeCode, AgentRunnerKind::Codex]
        .into_iter()
        .find(|kind| {
            runner_binary(kind).is_some_and(|bin| {
                parent
                    .components()
                    .filter_map(|c| c.as_os_str().to_str())
                    .any(|seg| seg == bin)
            })
        })
}

#[cfg(target_os = "macos")]
fn exec_path(pid: u32) -> Option<std::path::PathBuf> {
    let mut buf = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let len = unsafe {
        libc::proc_pidpath(
            pid as libc::c_int,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len() as u32,
        )
    };
    if len <= 0 {
        return None;
    }
    std::str::from_utf8(&buf[..len as usize])
        .ok()
        .map(std::path::PathBuf::from)
}

#[cfg(target_os = "linux")]
fn exec_path(pid: u32) -> Option<std::path::PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn exec_path(_pid: u32) -> Option<std::path::PathBuf> {
    None
}

/// Resolver de produção: comm primeiro (barato, cobre binário direto), caminho
/// do executável como reserva (uma syscall, e só pra processo da árvore do
/// shell que não casou por comm — nunca pro snapshot inteiro).
pub fn detect_kind(row: &ProcRow) -> Option<AgentRunnerKind> {
    agent_kind_for_comm(&row.comm)
        .or_else(|| exec_path(row.pid).and_then(|p| kind_from_exec_path(&p)))
}

/// Anda a árvore de processos a partir do `leader_pid` do shell (BFS pelos
/// filhos) e devolve o agente mais relevante: menor profundidade vence; empate
/// de profundidade é desempatado pelo mais recente (`start_ms`). O próprio líder
/// é o shell, nunca o agente. Pura sobre o resolver injetado — opera sobre um
/// snapshot já colhido, então um processo que sumiu no meio da varredura
/// simplesmente não aparece nas linhas; nunca causa panic. Em produção o
/// resolver é [`detect_kind`] (comm + caminho do executável).
pub fn find_agent(
    leader_pid: u32,
    rows: &[ProcRow],
    kind_of: &dyn Fn(&ProcRow) -> Option<AgentRunnerKind>,
) -> Option<DetectedAgent> {
    let mut children: HashMap<u32, Vec<&ProcRow>> = HashMap::new();
    for row in rows {
        children.entry(row.ppid).or_default().push(row);
    }

    let mut best: Option<DetectedAgent> = None;
    let mut best_depth = usize::MAX;
    let mut visited: HashSet<u32> = HashSet::from([leader_pid]);
    let mut queue: VecDeque<(u32, usize)> = VecDeque::from([(leader_pid, 0)]);

    while let Some((pid, depth)) = queue.pop_front() {
        let Some(kids) = children.get(&pid) else {
            continue;
        };
        for child in kids {
            if !visited.insert(child.pid) {
                continue;
            }
            let child_depth = depth + 1;
            if let Some(kind) = kind_of(child) {
                let better = match &best {
                    None => true,
                    Some(cur) => {
                        child_depth < best_depth
                            || (child_depth == best_depth && child.start_ms > cur.start_ms)
                    }
                };
                if better {
                    best = Some(DetectedAgent {
                        pid: child.pid,
                        start_ms: child.start_ms,
                        kind,
                    });
                    best_depth = child_depth;
                }
            }
            queue.push_back((child.pid, child_depth));
        }
    }

    best
}

/// Estado de detecção por sessão de shell, mantido no core e consultável por
/// command. O poll periódico chama `reconcile`; a UI escuta [`EVENT_CHANGED`].
#[derive(Default)]
pub struct AgentProber {
    detected: Mutex<HashMap<SessionId, DetectedAgent>>,
}

pub type SharedAgentProber = Arc<AgentProber>;

impl AgentProber {
    pub fn detected(&self, session: SessionId) -> Option<DetectedAgent> {
        self.detected.lock().get(&session).cloned()
    }

    /// Sonda cada shell vivo contra um único snapshot de processos, grava o novo
    /// estado por sessão e devolve só as sessões cuja detecção mudou (surgiu,
    /// sumiu ou trocou de agente) para o chamador emitir. Sessões ausentes de
    /// `shells` são descartadas em silêncio — o terminal delas já morreu.
    pub fn reconcile(
        &self,
        shells: &[(SessionId, u32)],
        rows: &[ProcRow],
    ) -> Vec<(SessionId, Option<DetectedAgent>)> {
        let mut state = self.detected.lock();
        let live: HashSet<SessionId> = shells.iter().map(|(id, _)| *id).collect();
        state.retain(|id, _| live.contains(id));

        let mut changes = Vec::new();
        for (session, leader_pid) in shells {
            let next = find_agent(*leader_pid, rows, &detect_kind);
            if state.get(session) == next.as_ref() {
                continue;
            }
            match &next {
                Some(agent) => {
                    state.insert(*session, agent.clone());
                }
                None => {
                    state.remove(session);
                }
            }
            changes.push((*session, next));
        }
        changes
    }
}

#[cfg(unix)]
const TERMINATE_GRACE: Duration = Duration::from_millis(1500);
#[cfg(unix)]
const TERMINATE_POLL: Duration = Duration::from_millis(100);

/// Encerra o agente detectado numa sessão de shell, cirurgicamente:
/// - re-verifica a identidade num snapshot fresco (pid + start_ms) — um pid
///   reciclado ou um agente que já saiu nunca é morto, e ambos contam como
///   sucesso (o objetivo é "não está mais rodando");
/// - mata o process group DO AGENTE (`getpgid`), nunca o do shell nem o
///   nosso: se o agente compartilha grupo com o líder do shell (shell sem job
///   control) ou com o próprio TYBA, mata só o pid;
/// - SIGTERM, grace de ~1,5s com poll de liveness, SIGKILL no que sobrar.
///
/// O princípio 9 (killpg da sessão inteira) vale pro kill de SESSÃO; aqui o
/// shell sobrevive de propósito — só o agente cru sai.
#[cfg(unix)]
pub fn terminate_detected(
    pid: u32,
    start_ms: u64,
    shell_leader_pid: Option<u32>,
) -> Result<(), crate::error::AppError> {
    let rows = snapshot();
    let Some(row) = rows.iter().find(|r| r.pid == pid) else {
        return Ok(());
    };
    if row.start_ms != start_ms {
        return Ok(());
    }
    let pid_i = pid as i32;
    let pgid = unsafe { libc::getpgid(pid_i) };
    let shell_pgid = shell_leader_pid.map(|lp| unsafe { libc::getpgid(lp as i32) });
    let own_pgid = unsafe { libc::getpgid(0) };
    let group_kill = pgid > 0 && Some(pgid) != shell_pgid && pgid != own_pgid;
    let send = |sig: i32| {
        if group_kill {
            unsafe { libc::killpg(pgid, sig) }
        } else {
            unsafe { libc::kill(pid_i, sig) }
        }
    };
    if send(libc::SIGTERM) != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(
            crate::error::AppError::new("agent.reopen_kill_failed").with("detail", err.to_string())
        );
    }
    let deadline = std::time::Instant::now() + TERMINATE_GRACE;
    while std::time::Instant::now() < deadline {
        if unsafe { libc::kill(pid_i, 0) } != 0 {
            return Ok(());
        }
        std::thread::sleep(TERMINATE_POLL);
    }
    let _ = send(libc::SIGKILL);
    Ok(())
}

#[cfg(not(unix))]
pub fn terminate_detected(
    _pid: u32,
    _start_ms: u64,
    _shell_leader_pid: Option<u32>,
) -> Result<(), crate::error::AppError> {
    Err(crate::error::AppError::new("agent.reopen_unsupported"))
}

/// Snapshot de todos os processos vivos (pid, ppid, comm, start_ms). macOS via
/// libproc, Linux via `/proc`, demais plataformas vazio (ver stubs abaixo).
#[cfg(target_os = "macos")]
pub fn snapshot() -> Vec<ProcRow> {
    imp_macos::snapshot()
}

#[cfg(target_os = "linux")]
pub fn snapshot() -> Vec<ProcRow> {
    imp_linux::snapshot()
}

// Windows: andar a árvore de processos exige um snapshot da Toolhelp
// (CreateToolhelp32Snapshot + Process32Next) — fica para uma fatia futura. Sem
// isso a detecção só devolve None, sem travar o build nem o poll.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn snapshot() -> Vec<ProcRow> {
    Vec::new()
}

#[cfg(target_os = "macos")]
mod imp_macos {
    use super::ProcRow;

    // <sys/proc_info.h>: PROC_ALL_PIDS = 1. A libc não exporta a constante.
    const PROC_ALL_PIDS: u32 = 1;

    pub fn snapshot() -> Vec<ProcRow> {
        let pids = list_pids();
        let mut rows = Vec::with_capacity(pids.len());
        for pid in pids {
            if pid == 0 {
                continue;
            }
            let Some(info) =
                crate::repo::proc_pidinfo_struct::<libc::proc_bsdinfo>(pid, libc::PROC_PIDTBSDINFO)
            else {
                continue;
            };
            rows.push(ProcRow {
                pid,
                ppid: info.pbi_ppid,
                comm: comm_of(&info.pbi_comm),
                start_ms: info.pbi_start_tvsec.saturating_mul(1000) + info.pbi_start_tvusec / 1000,
            });
        }
        rows
    }

    fn list_pids() -> Vec<u32> {
        let needed = unsafe { libc::proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };
        if needed <= 0 {
            return Vec::new();
        }
        let slots = needed as usize / std::mem::size_of::<libc::c_int>() + 16;
        let mut buf = vec![0 as libc::c_int; slots];
        let cap = (buf.len() * std::mem::size_of::<libc::c_int>()) as libc::c_int;
        let got = unsafe {
            libc::proc_listpids(PROC_ALL_PIDS, 0, buf.as_mut_ptr() as *mut libc::c_void, cap)
        };
        if got <= 0 {
            return Vec::new();
        }
        let count = got as usize / std::mem::size_of::<libc::c_int>();
        buf.into_iter()
            .take(count)
            .filter(|p| *p > 0)
            .map(|p| p as u32)
            .collect()
    }

    fn comm_of(buf: &[libc::c_char]) -> String {
        let bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len()) };
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }
}

#[cfg(target_os = "linux")]
mod imp_linux {
    use super::ProcRow;

    pub fn snapshot() -> Vec<ProcRow> {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        let clk_tck = clock_ticks_per_sec();
        let boot_ms = boot_time_ms();
        let mut rows = Vec::new();
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            let Some((ppid, start_ticks)) = super::parse_stat_ppid_starttime(&stat) else {
                continue;
            };
            let comm = std::fs::read_to_string(format!("/proc/{pid}/comm"))
                .map(|c| c.trim_end().to_string())
                .unwrap_or_default();
            let start_ms = boot_ms + start_ticks.saturating_mul(1000) / clk_tck;
            rows.push(ProcRow {
                pid,
                ppid,
                comm,
                start_ms,
            });
        }
        rows
    }

    fn clock_ticks_per_sec() -> u64 {
        let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if v > 0 {
            v as u64
        } else {
            100
        }
    }

    fn boot_time_ms() -> u64 {
        let Ok(stat) = std::fs::read_to_string("/proc/stat") else {
            return 0;
        };
        for line in stat.lines() {
            if let Some(rest) = line.strip_prefix("btime ") {
                if let Ok(secs) = rest.trim().parse::<u64>() {
                    return secs.saturating_mul(1000);
                }
            }
        }
        0
    }
}

/// Extrai (ppid, starttime em ticks) de uma linha `/proc/<pid>/stat`. O `comm`
/// pode conter espaços e parênteses, então corta no último `)`: depois dele os
/// campos são posicionais — [0]=state, [1]=ppid, [19]=starttime (campos 3, 4 e
/// 22 do proc(5)). Frágil por natureza, por isso tem teste.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_stat_ppid_starttime(stat: &str) -> Option<(u32, u64)> {
    let after_comm = stat.rsplit_once(')')?.1;
    let mut fields = after_comm.split_whitespace();
    let _state = fields.next()?;
    let ppid = fields.next()?.parse().ok()?;
    let start_ticks = fields.nth(17)?.parse().ok()?;
    Some((ppid, start_ticks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, ppid: u32, comm: &str, start_ms: u64) -> ProcRow {
        ProcRow {
            pid,
            ppid,
            comm: comm.to_string(),
            start_ms,
        }
    }

    fn comm_only(row: &ProcRow) -> Option<AgentRunnerKind> {
        agent_kind_for_comm(&row.comm)
    }

    #[test]
    fn version_like_names() {
        assert!(looks_like_version("2.1.218"));
        assert!(looks_like_version("0.4"));
        assert!(!looks_like_version("claude"));
        assert!(!looks_like_version("2-1-218"));
        assert!(!looks_like_version("v2.1.218"));
        assert!(!looks_like_version(""));
    }

    #[test]
    fn exec_path_resolves_the_native_installer_layout() {
        // O launcher nativo executa `.../claude/versions/2.1.218`: o comm vira
        // "2.1.218" e só o caminho denuncia o runner.
        let kind = kind_from_exec_path(std::path::Path::new(
            "/Users/x/.local/share/claude/versions/2.1.218",
        ));
        assert_eq!(kind, Some(AgentRunnerKind::ClaudeCode));
        let codex = kind_from_exec_path(std::path::Path::new(
            "/Users/x/.local/share/codex/versions/1.2.3",
        ));
        assert_eq!(codex, Some(AgentRunnerKind::Codex));
    }

    #[test]
    fn exec_path_matches_plain_binary_name_too() {
        assert_eq!(
            kind_from_exec_path(std::path::Path::new("/usr/local/bin/claude")),
            Some(AgentRunnerKind::ClaudeCode)
        );
    }

    #[test]
    fn exec_path_rejects_version_without_runner_ancestor_and_non_versions() {
        assert_eq!(
            kind_from_exec_path(std::path::Path::new("/opt/tools/2.1.218")),
            None
        );
        assert_eq!(
            kind_from_exec_path(std::path::Path::new(
                "/Users/x/.local/share/claude/versions/beta"
            )),
            None
        );
    }

    #[test]
    fn find_agent_uses_the_injected_resolver_for_versioned_comm() {
        // O caso real do E2E: comm "2.1.218" não casa por nome; o resolver de
        // produção resolve pelo caminho do executável — aqui simulado.
        let rows = vec![row(100, 1, "zsh", 10), row(200, 100, "2.1.218", 20)];
        let resolver = |r: &ProcRow| (r.comm == "2.1.218").then_some(AgentRunnerKind::ClaudeCode);
        let found = find_agent(100, &rows, &resolver).expect("agente versionado");
        assert_eq!(found.pid, 200);
        assert_eq!(found.kind, AgentRunnerKind::ClaudeCode);
        assert_eq!(find_agent(100, &rows, &comm_only), None);
    }

    #[test]
    fn agent_kind_for_comm_maps_known_binaries_only() {
        assert_eq!(
            agent_kind_for_comm("claude"),
            Some(AgentRunnerKind::ClaudeCode)
        );
        assert_eq!(agent_kind_for_comm("codex"), Some(AgentRunnerKind::Codex));
        assert_eq!(agent_kind_for_comm("node"), None);
        assert_eq!(agent_kind_for_comm("claude-helper"), None);
        assert_eq!(agent_kind_for_comm(""), None);
    }

    #[test]
    fn finds_claude_that_is_a_direct_child_of_the_shell() {
        let rows = vec![
            row(100, 1, "zsh", 10),
            row(200, 100, "claude", 20),
            row(300, 200, "node", 30),
        ];
        let found = find_agent(100, &rows, &comm_only).expect("agente detectado");
        assert_eq!(found.pid, 200);
        assert_eq!(found.kind, AgentRunnerKind::ClaudeCode);
        assert_eq!(found.start_ms, 20);
    }

    #[test]
    fn ignores_processes_that_are_not_agents() {
        let rows = vec![
            row(100, 1, "zsh", 10),
            row(200, 100, "vim", 20),
            row(300, 100, "node", 30),
            row(400, 300, "esbuild", 40),
        ];
        assert_eq!(find_agent(100, &rows, &comm_only), None);
    }

    #[test]
    fn a_tree_without_any_agent_is_none() {
        let rows = vec![row(100, 1, "bash", 10), row(200, 100, "git", 20)];
        assert_eq!(find_agent(100, &rows, &comm_only), None);
    }

    #[test]
    fn nearest_agent_wins_over_a_deeper_one() {
        let rows = vec![
            row(100, 1, "zsh", 10),
            row(200, 100, "claude", 20),
            row(300, 100, "npm", 25),
            row(400, 300, "codex", 30),
        ];
        let found = find_agent(100, &rows, &comm_only).expect("agente");
        assert_eq!(found.pid, 200, "claude a 1 salto vence o codex a 2 saltos");
        assert_eq!(found.kind, AgentRunnerKind::ClaudeCode);
    }

    #[test]
    fn same_depth_tie_breaks_to_the_freshest_start() {
        let rows = vec![
            row(100, 1, "zsh", 10),
            row(200, 100, "claude", 20),
            row(300, 100, "codex", 40),
        ];
        let found = find_agent(100, &rows, &comm_only).expect("agente");
        assert_eq!(
            found.pid, 300,
            "no empate de profundidade vence o mais recente"
        );
        assert_eq!(found.start_ms, 40);
    }

    #[test]
    fn a_leader_absent_from_the_snapshot_yields_none_without_panicking() {
        let rows = vec![row(200, 100, "claude", 20)];
        assert_eq!(find_agent(999, &rows, &comm_only), None);
    }

    #[test]
    fn a_parent_cycle_in_the_snapshot_does_not_loop_forever() {
        // 100 é filho de 200 e 200 é filho de 100: pid reusado no meio do scan.
        let rows = vec![
            row(100, 200, "zsh", 10),
            row(200, 100, "sh", 15),
            row(300, 200, "codex", 20),
        ];
        let found = find_agent(100, &rows, &comm_only).expect("agente sob o líder");
        assert_eq!(found.pid, 300);
        assert_eq!(found.kind, AgentRunnerKind::Codex);
    }

    #[test]
    fn parse_stat_extracts_ppid_and_starttime_across_parens_in_comm() {
        let stat = "200 (claude (helper)) S 100 200 200 0 -1 4194304 1 0 0 0 0 0 0 0 20 0 1 0 987654 100 0 0";
        assert_eq!(parse_stat_ppid_starttime(stat), Some((100, 987654)));
    }

    #[test]
    fn parse_stat_rejects_truncated_lines() {
        assert_eq!(parse_stat_ppid_starttime("200 (claude) S 100"), None);
        assert_eq!(parse_stat_ppid_starttime(""), None);
    }

    fn session() -> SessionId {
        SessionId::new_v4()
    }

    #[test]
    fn reconcile_reports_a_change_once_then_stays_quiet() {
        let prober = AgentProber::default();
        let s = session();
        let rows = vec![row(100, 1, "zsh", 10), row(200, 100, "claude", 20)];

        let first = prober.reconcile(&[(s, 100)], &rows);
        assert_eq!(first.len(), 1, "a primeira detecção deve emitir");
        assert_eq!(first[0].0, s);
        assert_eq!(first[0].1.as_ref().map(|d| d.pid), Some(200));
        assert_eq!(prober.detected(s).map(|d| d.pid), Some(200));

        let second = prober.reconcile(&[(s, 100)], &rows);
        assert!(second.is_empty(), "detecção estável não deve reemitir");
    }

    #[test]
    fn reconcile_reports_the_loss_of_a_previously_detected_agent() {
        let prober = AgentProber::default();
        let s = session();
        let with_agent = vec![row(100, 1, "zsh", 10), row(200, 100, "claude", 20)];
        let without = vec![row(100, 1, "zsh", 10)];

        prober.reconcile(&[(s, 100)], &with_agent);
        let lost = prober.reconcile(&[(s, 100)], &without);
        assert_eq!(lost.len(), 1);
        assert_eq!(lost[0], (s, None));
        assert_eq!(prober.detected(s), None);
    }

    #[test]
    fn reconcile_drops_a_vanished_session_silently() {
        let prober = AgentProber::default();
        let s = session();
        let rows = vec![row(100, 1, "zsh", 10), row(200, 100, "codex", 20)];

        prober.reconcile(&[(s, 100)], &rows);
        assert!(prober.detected(s).is_some());

        let changes = prober.reconcile(&[], &[]);
        assert!(changes.is_empty(), "sessão morta não gera evento");
        assert_eq!(prober.detected(s), None, "estado da sessão morta é limpo");
    }

    #[cfg(unix)]
    fn spawn_sleeper_in_own_group() -> std::process::Child {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new("sleep");
        cmd.arg("30");
        unsafe {
            cmd.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        cmd.spawn().expect("spawn sleep")
    }

    #[cfg(unix)]
    fn snapshot_start_ms(pid: u32) -> Option<u64> {
        for _ in 0..50 {
            if let Some(row) = snapshot().into_iter().find(|r| r.pid == pid) {
                return Some(row.start_ms);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[cfg(unix)]
    fn wait_gone(child: &mut std::process::Child) -> bool {
        for _ in 0..100 {
            if let Ok(Some(_)) = child.try_wait() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[cfg(unix)]
    #[test]
    fn terminate_kills_the_verified_agent() {
        let mut child = spawn_sleeper_in_own_group();
        let pid = child.id();
        let start_ms = snapshot_start_ms(pid).expect("sleeper no snapshot");

        terminate_detected(pid, start_ms, None).expect("terminate");
        assert!(wait_gone(&mut child), "sleeper deveria morrer no TERM");
    }

    #[cfg(unix)]
    #[test]
    fn terminate_refuses_a_recycled_pid() {
        // start_ms divergente = o pid não é mais o agente detectado; nada morre
        // e a chamada é sucesso (o agente detectado já não existe).
        let mut child = spawn_sleeper_in_own_group();
        let pid = child.id();
        let start_ms = snapshot_start_ms(pid).expect("sleeper no snapshot");

        terminate_detected(pid, start_ms + 999_999, None).expect("mismatch é ok");
        assert!(
            matches!(child.try_wait(), Ok(None)),
            "sleeper deveria seguir vivo"
        );

        let _ = child.kill();
        let _ = child.wait();
    }

    #[cfg(unix)]
    #[test]
    fn terminate_spares_the_group_when_the_agent_shares_it_with_the_shell() {
        // Shell sem job control: o agente herda o pgid do líder. killpg aqui
        // levaria o shell (e o grupo inteiro) junto — o kill degrada pra pid
        // único. O espectador no mesmo grupo prova que não houve killpg: se
        // houvesse, ele (e o próprio runner de teste) morreria no TERM.
        let mut agent = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn agente");
        let mut bystander = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn espectador");
        let pid = agent.id();
        let start_ms = snapshot_start_ms(pid).expect("agente no snapshot");

        terminate_detected(pid, start_ms, Some(std::process::id())).expect("terminate");
        assert!(wait_gone(&mut agent), "agente deveria morrer no TERM");
        assert!(
            matches!(bystander.try_wait(), Ok(None)),
            "irmão de grupo deveria sobreviver — killpg teria levado o grupo"
        );

        let _ = bystander.kill();
        let _ = bystander.wait();
    }

    #[cfg(unix)]
    #[test]
    fn terminate_of_a_dead_pid_is_success() {
        let mut child = spawn_sleeper_in_own_group();
        let pid = child.id();
        let start_ms = snapshot_start_ms(pid).expect("sleeper no snapshot");
        child.kill().expect("kill");
        child.wait().expect("wait");

        terminate_detected(pid, start_ms, None).expect("morto é sucesso");
    }
}
