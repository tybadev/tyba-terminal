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

/// Anda a árvore de processos a partir do `leader_pid` do shell (BFS pelos
/// filhos) e devolve o agente mais relevante: menor profundidade vence; empate
/// de profundidade é desempatado pelo mais recente (`start_ms`). O próprio líder
/// é o shell, nunca o agente. Pura e sem I/O — opera sobre um snapshot já
/// colhido, então um processo que sumiu no meio da varredura simplesmente não
/// aparece nas linhas; nunca causa panic.
pub fn find_agent(leader_pid: u32, rows: &[ProcRow]) -> Option<DetectedAgent> {
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
            if let Some(kind) = agent_kind_for_comm(&child.comm) {
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
            let next = find_agent(*leader_pid, rows);
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
        let found = find_agent(100, &rows).expect("agente detectado");
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
        assert_eq!(find_agent(100, &rows), None);
    }

    #[test]
    fn a_tree_without_any_agent_is_none() {
        let rows = vec![row(100, 1, "bash", 10), row(200, 100, "git", 20)];
        assert_eq!(find_agent(100, &rows), None);
    }

    #[test]
    fn nearest_agent_wins_over_a_deeper_one() {
        let rows = vec![
            row(100, 1, "zsh", 10),
            row(200, 100, "claude", 20),
            row(300, 100, "npm", 25),
            row(400, 300, "codex", 30),
        ];
        let found = find_agent(100, &rows).expect("agente");
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
        let found = find_agent(100, &rows).expect("agente");
        assert_eq!(
            found.pid, 300,
            "no empate de profundidade vence o mais recente"
        );
        assert_eq!(found.start_ms, 40);
    }

    #[test]
    fn a_leader_absent_from_the_snapshot_yields_none_without_panicking() {
        let rows = vec![row(200, 100, "claude", 20)];
        assert_eq!(find_agent(999, &rows), None);
    }

    #[test]
    fn a_parent_cycle_in_the_snapshot_does_not_loop_forever() {
        // 100 é filho de 200 e 200 é filho de 100: pid reusado no meio do scan.
        let rows = vec![
            row(100, 200, "zsh", 10),
            row(200, 100, "sh", 15),
            row(300, 200, "codex", 20),
        ];
        let found = find_agent(100, &rows).expect("agente sob o líder");
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
}
