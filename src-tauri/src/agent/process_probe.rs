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
    /// Derivado de `/proc/<pid>/environ`, nunca de `/proc/<pid>/cmdline` —
    /// `_seccomp-exec` faz `.exec()` (bwrap.rs), que reescreve o cmdline mas
    /// preserva o env (tech-spec §7, FIX C9). `false`/`false` quando
    /// `detected` é `None`: sem agente, não há o que hospedar.
    pub hosting: bool,
    pub jailed: bool,
    /// Agente livre, fatia 4: claude no pane sem o plugin do TYBA (e com os
    /// hooks livres) — o pane mostra o chip de instalar. Decidido aqui, no
    /// core; o front só mostra.
    pub suggest_plugin: bool,
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
/// basename igual ao binário do runner, ou o layout exato do instalador
/// nativo — `.../<runner>/versions/<versão>` — pra não promover a agente
/// qualquer binário versionado sob uma pasta chamada `claude`.
fn kind_from_exec_path(path: &std::path::Path) -> Option<AgentRunnerKind> {
    let file = path.file_name()?.to_str()?;
    if let Some(kind) = agent_kind_for_comm(file) {
        return Some(kind);
    }
    if !looks_like_version(file) {
        return None;
    }
    let versions = path.parent()?;
    if versions.file_name()?.to_str()? != "versions" {
        return None;
    }
    let runner_dir = versions.parent()?.file_name()?.to_str()?;
    agent_kind_for_comm(runner_dir)
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

/// Resolver de produção: comm primeiro (barato, cobre binário direto); o
/// caminho do executável (uma syscall) só entra quando o comm parece VERSÃO —
/// a assinatura do launcher nativo. Filho comum de build (node, cc, rustc…)
/// nunca paga a syscall, então uma árvore com centenas de processos custa
/// ~zero por tick (performance-first).
pub fn detect_kind(row: &ProcRow) -> Option<AgentRunnerKind> {
    agent_kind_for_comm(&row.comm).or_else(|| {
        looks_like_version(&row.comm)
            .then(|| exec_path(row.pid).and_then(|p| kind_from_exec_path(&p)))
            .flatten()
    })
}

#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
const TYBA_HOSTED_MARKER: &[u8] = b"TYBA_HOSTED=1";
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
const TYBA_JAILED_MARKER: &[u8] = b"TYBA_JAILED=1";

/// A leitura pura: dado o conteúdo cru de `/proc/<pid>/environ` (pares
/// `KEY=VALUE` separados por NUL — nunca por `\n`, um valor pode conter
/// qualquer byte exceto NUL), devolve `(hosting, jailed)`. Separado de
/// [`hosting_markers`] pelo mesmo motivo de sempre: testável com bytes
/// sintéticos, sem precisar de um processo de verdade rodando. Fora do Linux
/// e do macOS `hosting_markers` nunca chama isto — `cfg_attr` silencia o
/// dead-code sem esconder a função do teste, que roda em qualquer
/// plataforma.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
fn parse_environ_markers(environ: &[u8]) -> (bool, bool) {
    let mut hosting = false;
    let mut jailed = false;
    for entry in environ.split(|b| *b == 0) {
        if entry == TYBA_HOSTED_MARKER {
            hosting = true;
        } else if entry == TYBA_JAILED_MARKER {
            jailed = true;
        }
    }
    (hosting, jailed)
}

/// Deriva `hosting`/`jailed` do env do processo — NUNCA do `cmdline` (tech-spec
/// §7, FIX C9): a cadeia `_seccomp-exec` faz `.exec()` (bwrap.rs), que
/// reescreve o cmdline visível em `/proc` mas preserva o env através do
/// `execve`, medido. `/proc/<pid>/environ` é `0400` same-uid — o mesmo uid que
/// já lê `/proc/<pid>/cwd` para a allowlist do canal. Pid que sumiu ou
/// environ ilegível (permissão, corrida) degrada para `(false, false)`: nunca
/// panic, nunca promessa de gate que não existe.
#[cfg(target_os = "linux")]
pub fn hosting_markers(pid: u32) -> (bool, bool) {
    match std::fs::read(format!("/proc/{pid}/environ")) {
        Ok(bytes) => parse_environ_markers(&bytes),
        Err(_) => (false, false),
    }
}

/// macOS (M3 do claude-monitor): sem `/proc`, o env de um processo do mesmo
/// usuário sai de `sysctl(KERN_PROCARGS2)` — o mesmo que o `ps -E` lê. O
/// `sandbox-exec` da cadeia hospedada faz `execve` no próprio pid, então o
/// env com os marcadores chega intacto ao `claude`. Falha de leitura degrada
/// para `(false, false)`, como no Linux.
#[cfg(target_os = "macos")]
pub fn hosting_markers(pid: u32) -> (bool, bool) {
    procargs2(pid)
        .as_deref()
        .and_then(procargs2_environ)
        .map(parse_environ_markers)
        .unwrap_or((false, false))
}

#[cfg(target_os = "macos")]
fn procargs2(pid: u32) -> Option<Vec<u8>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let mut size: libc::size_t = 0;
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(size);
    Some(buf)
}

/// O bloco de env dentro do buffer de `KERN_PROCARGS2`: `argc` (i32 nativo),
/// o caminho do executável, NULs de alinhamento, `argc` strings de argv e,
/// depois delas, o env — pares `KEY=VALUE` separados por NUL, a mesma forma
/// que [`parse_environ_markers`] já lê. Buffer truncado ou malformado é
/// `None`, nunca panic. Puro, para testar com bytes sintéticos.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn procargs2_environ(buf: &[u8]) -> Option<&[u8]> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    if argc < 0 {
        return None;
    }
    let mut at = 4;
    // Caminho do executável.
    at += buf.get(at..)?.iter().position(|b| *b == 0)?;
    // NULs de alinhamento até o argv[0].
    while buf.get(at) == Some(&0) {
        at += 1;
    }
    for _ in 0..argc {
        at += buf.get(at..)?.iter().position(|b| *b == 0)? + 1;
    }
    buf.get(at..)
}

/// Fora do Linux e do macOS não há canal shim↔core, não há marcador, e
/// `hosting` nunca é verdadeiro.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn hosting_markers(_pid: u32) -> (bool, bool) {
    (false, false)
}

/// O valor de `name` num bloco de env NUL-separado. Só a entrada que começa
/// com `name=` conta.
pub fn parse_env_var(environ: &[u8], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    environ
        .split(|b| *b == 0)
        .find_map(|entry| entry.strip_prefix(prefix.as_bytes()))
        .map(|v| String::from_utf8_lossy(v).into_owned())
}

/// Uma variável do env real do processo, lido como o de [`hosting_markers`].
/// Fatia 4 do agente livre: o `CLAUDE_CONFIG_DIR` do claude do pane, que o
/// TYBA aberto pelo Dock não herda.
#[cfg(target_os = "linux")]
pub fn env_var(pid: u32, name: &str) -> Option<String> {
    parse_env_var(&std::fs::read(format!("/proc/{pid}/environ")).ok()?, name)
}

#[cfg(target_os = "macos")]
pub fn env_var(pid: u32, name: &str) -> Option<String> {
    parse_env_var(procargs2_environ(&procargs2(pid)?)?, name)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn env_var(_pid: u32, _name: &str) -> Option<String> {
    None
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
            // Poda: um nó mais fundo que o melhor achado nunca vence (empate
            // só no MESMO nível), então nem se sonda nem se desce nele — sem
            // isso a BFS varreria a subárvore inteira do agente já detectado.
            let child_depth = depth + 1;
            if child_depth > best_depth {
                continue;
            }
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
            if child_depth < best_depth {
                queue.push_back((child.pid, child_depth));
            }
        }
    }

    best
}

/// Limite de saltos que [`find_owning_session`] sobe pela árvore de
/// processos. Existe para que um ciclo de `ppid` no snapshot (pid reciclado
/// no meio da varredura) nunca gire para sempre — pior caso é devolver `None`
/// (a requisição é recusada, nunca escala).
const OWNING_SESSION_MAX_HOPS: u32 = 16;

/// Resolve a sessão de shell DONA de um pid, subindo `ppid` a partir dele até
/// achar o líder de uma sessão conhecida (canal shim↔core, §4.2). O pid de
/// entrada é sempre o do PEER autenticado por `SO_PEERCRED` — nunca algo que o
/// pedido afirma —, e é por isso que este é o núcleo da autenticação por
/// sessão do canal: quem conectou só pode hospedar agente na sessão cujo
/// líder está na ancestralidade dele.
///
/// Pura sobre um snapshot já colhido e a lista de líderes já resolvida — a
/// mesma forma de `find_agent`, para o mesmo motivo: testável sem processos
/// de verdade, e um pid que sumiu no meio da varredura simplesmente encerra a
/// subida (`None`), nunca panic.
pub fn find_owning_session(
    peer_pid: u32,
    shells: &[(SessionId, u32)],
    rows: &[ProcRow],
) -> Option<SessionId> {
    let ppid_of: HashMap<u32, u32> = rows.iter().map(|r| (r.pid, r.ppid)).collect();
    let leader_session: HashMap<u32, SessionId> =
        shells.iter().map(|&(id, pid)| (pid, id)).collect();

    let mut pid = peer_pid;
    for _ in 0..OWNING_SESSION_MAX_HOPS {
        if let Some(&id) = leader_session.get(&pid) {
            return Some(id);
        }
        if pid == 1 {
            return None;
        }
        pid = *ppid_of.get(&pid)?;
    }
    None
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
        // Binário versionado sob uma pasta chamada `claude` que NÃO é o layout
        // do instalador (`<runner>/versions/<versão>`) não vira agente.
        assert_eq!(
            kind_from_exec_path(std::path::Path::new("/Users/x/claude/1.2.3")),
            None
        );
        assert_eq!(
            kind_from_exec_path(std::path::Path::new("/home/claude/apps/versions/1.2.3")),
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

    // --- find_owning_session (canal shim↔core, §4.2) ---
    //
    // Sobe a árvore de processos a partir do pid do PEER (o cliente `tyba
    // _jail`, nunca o que o pedido afirma) até achar o líder de uma sessão de
    // shell conhecida. Ver o `[!danger]` da spec: é a peça que autentica QUAL
    // sessão pediu o agente hospedado.

    #[test]
    fn owning_session_resolves_a_direct_child_of_the_shell_leader() {
        let s = session();
        let rows = vec![row(100, 1, "zsh", 10), row(200, 100, "tyba", 20)];
        assert_eq!(
            find_owning_session(200, &[(s, 100)], &rows),
            Some(s),
            "o peer é filho direto do líder do shell"
        );
    }

    #[test]
    fn owning_session_walks_up_multiple_hops() {
        let s = session();
        // shell(100) -> node(150) -> tyba(200): dois saltos até o líder.
        let rows = vec![
            row(100, 1, "zsh", 10),
            row(150, 100, "node", 15),
            row(200, 150, "tyba", 20),
        ];
        assert_eq!(find_owning_session(200, &[(s, 100)], &rows), Some(s));
    }

    #[test]
    fn owning_session_is_none_for_a_pid_unrelated_to_any_shell() {
        let s = session();
        let rows = vec![row(100, 1, "zsh", 10), row(999, 1, "tyba", 5)];
        assert_eq!(find_owning_session(999, &[(s, 100)], &rows), None);
    }

    #[test]
    fn owning_session_stops_at_pid_1_without_matching() {
        let s = session();
        // 50 é filho direto do init (pid 1) — nunca escala além dele.
        let rows = vec![row(100, 1, "zsh", 10), row(50, 1, "orphan", 5)];
        assert_eq!(find_owning_session(50, &[(s, 100)], &rows), None);
    }

    #[test]
    fn owning_session_is_bounded_and_survives_a_ppid_cycle() {
        let s = session();
        // 10 <-> 20 se apontam mutuamente (pid reciclado no meio do scan): sem
        // limite de saltos isto giraria para sempre.
        let rows = vec![row(10, 20, "a", 1), row(20, 10, "b", 2)];
        assert_eq!(find_owning_session(10, &[(s, 100)], &rows), None);
    }

    /// M3: com dois panes abertos, o peer sob o shell B é do B — nunca do
    /// primeiro líder da lista.
    #[test]
    fn owning_session_picks_the_shell_that_is_actually_an_ancestor() {
        let a = session();
        let b = session();
        let rows = vec![
            row(100, 1, "zsh", 10),
            row(300, 1, "zsh", 11),
            row(310, 300, "tyba", 20),
        ];
        assert_eq!(
            find_owning_session(310, &[(a, 100), (b, 300)], &rows),
            Some(b)
        );
    }

    // --- hosting/jailed via marcador de env (tech-spec §7) ------------------

    /// Buffer no formato de `KERN_PROCARGS2`, montado à mão.
    fn procargs2_of(exec: &str, argv: &[&str], env: &[&str]) -> Vec<u8> {
        let mut buf = (argv.len() as i32).to_ne_bytes().to_vec();
        buf.extend_from_slice(exec.as_bytes());
        buf.extend_from_slice(&[0, 0, 0, 0]);
        for arg in argv {
            buf.extend_from_slice(arg.as_bytes());
            buf.push(0);
        }
        buf.extend_from_slice(&environ_of(env));
        buf
    }

    #[test]
    fn env_var_reads_only_the_exact_entry() {
        let env = environ_of(&[
            "XCLAUDE_CONFIG_DIR=/no",
            "CLAUDE_CONFIG_DIR=/Users/x/.claude-work",
            "HOME=/Users/x",
        ]);
        assert_eq!(
            parse_env_var(&env, "CLAUDE_CONFIG_DIR").as_deref(),
            Some("/Users/x/.claude-work")
        );
        assert_eq!(
            parse_env_var(&environ_of(&["HOME=/x"]), "CLAUDE_CONFIG_DIR"),
            None
        );
    }

    #[test]
    fn procargs2_environ_skips_exec_path_padding_and_argv() {
        // Um argv que CONTÉM o marcador não pode contar: só o env vale.
        let buf = procargs2_of(
            "/usr/local/bin/claude",
            &["claude", "TYBA_HOSTED=1"],
            &["HOME=/Users/x", "TYBA_HOSTED=1", "TYBA_JAILED=1"],
        );
        let env = procargs2_environ(&buf).expect("env");
        assert_eq!(parse_environ_markers(env), (true, true));

        let only_argv = procargs2_of("/bin/x", &["x", "TYBA_HOSTED=1"], &["HOME=/Users/x"]);
        assert_eq!(
            parse_environ_markers(procargs2_environ(&only_argv).unwrap()),
            (false, false)
        );
    }

    #[test]
    fn procargs2_environ_rejects_truncated_or_malformed_buffers() {
        assert_eq!(procargs2_environ(&[]), None);
        assert_eq!(procargs2_environ(&[1, 0]), None);
        assert_eq!(procargs2_environ(&(-1i32).to_ne_bytes()), None);
        // argc diz 3, mas só há um argumento antes do fim do buffer.
        let mut short = 3i32.to_ne_bytes().to_vec();
        short.extend_from_slice(b"/bin/x\0\0a\0");
        assert_eq!(procargs2_environ(&short), None);
    }

    fn environ_of(pairs: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for pair in pairs {
            bytes.extend_from_slice(pair.as_bytes());
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn parse_environ_detects_both_markers_together() {
        let environ = environ_of(&["HOME=/root", "TYBA_HOSTED=1", "TYBA_JAILED=1", "PATH=/bin"]);
        assert_eq!(parse_environ_markers(&environ), (true, true));
    }

    #[test]
    fn parse_environ_detects_hosted_without_jailed() {
        // Gate-sem-jaula (§1): hospedado, mas fora da allowlist ou sem userns.
        let environ = environ_of(&["TYBA_HOSTED=1", "TYBA_JAILED=0"]);
        assert_eq!(parse_environ_markers(&environ), (true, false));
    }

    #[test]
    fn parse_environ_without_any_marker_is_neither() {
        // `claude` cru, sem passar pelo shim v2 nenhum — o caso do v1.
        let environ = environ_of(&["HOME=/root", "PATH=/bin", "SHELL=/bin/zsh"]);
        assert_eq!(parse_environ_markers(&environ), (false, false));
    }

    #[test]
    fn parse_environ_ignores_a_value_that_merely_contains_the_marker_as_substring() {
        // Um valor de env de OUTRA variável não pode ser confundido com o
        // marcador — a comparação é da entrada NUL-delimitada inteira, nunca
        // `contains`.
        let environ = environ_of(&["SOME_VAR=xTYBA_HOSTED=1x", "OTHER=TYBA_HOSTED=10"]);
        assert_eq!(parse_environ_markers(&environ), (false, false));
    }

    /// Um processo parado por alguns segundos, com env limpo, para ter o env
    /// lido de fora. No Linux, `sleep`. No macOS o `/bin/sleep` é binário de
    /// plataforma e o kernel esconde o env dele de qualquer outro processo
    /// (`ps -E` também não vê) — o `claude` de verdade não é, então o teste
    /// usa o próprio binário de teste rodando [`idle_helper`].
    fn idle_child() -> std::process::Command {
        #[cfg(target_os = "macos")]
        {
            let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
            cmd.args([
                "--exact",
                "agent::process_probe::tests::idle_helper",
                "--ignored",
                "--test-threads=1",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .env_clear()
            .env("TYBA_IDLE_HELPER", "1");
            cmd
        }
        #[cfg(not(target_os = "macos"))]
        {
            let mut cmd = std::process::Command::new("sleep");
            cmd.arg("5").env_clear();
            cmd
        }
    }

    /// Só serve de processo-alvo para [`idle_child`] no macOS.
    #[test]
    #[ignore]
    fn idle_helper() {
        if std::env::var_os("TYBA_IDLE_HELPER").is_some() {
            std::thread::sleep(Duration::from_secs(5));
        }
    }

    /// Prova viva (não só o parser): um processo real, com os dois marcadores
    /// no env, lido pelo `hosting_markers` de produção via `/proc/<pid>/environ`
    /// de verdade — a mesma dupla que `prepare_hosted_agent` (Track B) grava.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn hosting_markers_reads_a_real_process_environ() {
        let mut child = idle_child()
            .env("TYBA_HOSTED", "1")
            .env("TYBA_JAILED", "1")
            .spawn()
            .expect("spawn sleep marcado");
        let pid = child.id();

        let mut seen = (false, false);
        for _ in 0..50 {
            seen = hosting_markers(pid);
            if seen == (true, true) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();

        assert_eq!(
            seen,
            (true, true),
            "hosting_markers deveria ler os dois marcadores do /proc/<pid>/environ real"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn hosting_markers_of_a_process_without_any_marker_is_neither() {
        let mut child = idle_child()
            .env("HOME", "/root")
            .spawn()
            .expect("spawn sleep sem marcador");
        let pid = child.id();
        std::thread::sleep(Duration::from_millis(200));

        let seen = hosting_markers(pid);
        let _ = child.kill();
        let _ = child.wait();

        assert_eq!(seen, (false, false));
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
}
