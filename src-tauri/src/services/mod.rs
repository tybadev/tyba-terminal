//! Serviços e portas: o que está escutando na máquina da sessão, de qual
//! processo e de qual sessão — e encerrar com as travas do core.
//!
//! O retrato junta três fontes: quem escuta (`listen`), a árvore e a
//! identidade dos processos (`agent::process_probe::snapshot`, com o
//! `start_ms` que identifica o processo) e grupo/dono/uso (`ps`). A origem e a
//! recusa saem de `classify`, pura. Comando exibido sai redigido
//! (`cmdline`); o env de processo nunca é lido.
//!
//! Encerrar: o core confere de novo, imediatamente antes do sinal, que o PID
//! ainda é o mesmo processo (`start_ms`) e que nenhuma regra recusa; manda
//! SIGTERM ao grupo (ou só ao pid, se o grupo leva junto algo que não pode
//! morrer) e diz se saiu. SIGKILL só num segundo comando, 5 s depois do TERM.

pub mod classify;
pub mod cmdline;
pub mod listen;
pub mod ps;
pub mod remote;

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;

use crate::agent::process_probe::ProcRow;
use crate::error::AppError;
use crate::session::SessionId;

use classify::{Facts, Member, Origin, Refusal, SignalTarget, World};

pub const EVENT_SESSION_PORTS: &str = "services://session-ports";
/// Cadência do chip da sidebar: bem mais lenta que a da tela (~3 s, na UI).
pub const CHIP_POLL: Duration = Duration::from_secs(10);
/// SIGKILL só depois disso desde o SIGTERM.
pub const FORCE_AFTER: Duration = Duration::from_secs(5);
/// Teto de qualquer `lsof`/`ps`.
const RUN_TIMEOUT: Duration = Duration::from_secs(5);
/// Quanto o TERM espera para dizer "saiu".
const TERM_WAIT: Duration = Duration::from_millis(2000);
const KILL_WAIT: Duration = Duration::from_millis(1000);
const EXIT_POLL: Duration = Duration::from_millis(100);
/// Teto de nós na árvore do detalhe e por sessão em Processos.
const TREE_CAP: usize = 60;

/// Roda um binário por caminho absoluto, sem shell, com env vazio (só
/// `LC_ALL=C`), stdin nulo, saída lida até `max_bytes` e teto de tempo.
pub(crate) fn run_bounded(
    bin: &Path,
    args: &[&str],
    max_bytes: usize,
) -> Result<Vec<u8>, AppError> {
    let fail = |e: &dyn std::fmt::Display| {
        AppError::new("services.scan_failed").with("detail", e.to_string())
    };
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::repo::no_console_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| fail(&e))?;
    let stdout = child.stdout.take().ok_or_else(|| fail(&"sem stdout"))?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.take(max_bytes as u64).read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        match child.try_wait().map_err(|e| fail(&e))? {
            Some(_) => break,
            None if start.elapsed() >= RUN_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AppError::new("services.scan_timeout"));
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    // Saída maior que o teto: o leitor parou e o processo pode ter morrido de
    // SIGPIPE; o que foi lido serve.
    reader.join().map_err(|_| fail(&"leitor"))
}

/// Quem é o TYBA e quais sessões existem, no instante da varredura.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub shells: Vec<(SessionId, u32)>,
    pub tyba_pid: u32,
    pub tyba_pgid: u32,
    pub my_uid: u32,
}

impl Ctx {
    #[cfg(unix)]
    pub fn current(shells: Vec<(SessionId, u32)>) -> Self {
        Self {
            shells,
            tyba_pid: std::process::id(),
            tyba_pgid: unsafe { libc::getpgid(0) }.max(0) as u32,
            my_uid: unsafe { libc::getuid() },
        }
    }

    #[cfg(not(unix))]
    pub fn current(shells: Vec<(SessionId, u32)>) -> Self {
        Self {
            shells,
            tyba_pid: std::process::id(),
            tyba_pgid: 0,
            my_uid: 0,
        }
    }
}

/// Um retrato: quem escuta + processos.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub listeners: Vec<listen::Listener>,
    pub rows: Vec<ProcRow>,
    pub ps: Vec<ps::PsRow>,
}

fn world_of(scan: &Scan, ctx: &Ctx) -> World {
    let mut parent: HashMap<u32, u32> = scan.rows.iter().map(|r| (r.pid, r.ppid)).collect();
    for p in &scan.ps {
        parent.entry(p.pid).or_insert(p.ppid);
    }
    World {
        parent,
        leaders: ctx.shells.iter().map(|&(id, pid)| (pid, id)).collect(),
        tyba_pid: ctx.tyba_pid,
        tyba_pgid: ctx.tyba_pgid,
        my_uid: ctx.my_uid,
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PortEntry {
    pub port: u16,
    pub addresses: Vec<String>,
    pub pid: u32,
    /// Identidade do processo (com o pid): vai de volta no encerrar.
    pub start_ms: u64,
    pub comm: String,
    /// Redigido.
    pub command: String,
    pub origin: Origin,
    pub cpu_percent: Option<f32>,
    pub rss_bytes: Option<u64>,
    pub pgid: Option<u32>,
    /// `Some` = o core não encerra este; a UI mostra o motivo.
    pub refusal: Option<Refusal>,
}

struct Index<'a> {
    rows: HashMap<u32, &'a ProcRow>,
    ps: HashMap<u32, &'a ps::PsRow>,
}

impl<'a> Index<'a> {
    fn new(scan: &'a Scan) -> Self {
        Self {
            rows: scan.rows.iter().map(|r| (r.pid, r)).collect(),
            ps: scan.ps.iter().map(|p| (p.pid, p)).collect(),
        }
    }

    fn comm(&self, pid: u32) -> String {
        self.rows
            .get(&pid)
            .map(|r| r.comm.clone())
            .filter(|c| !c.is_empty())
            .or_else(|| {
                self.ps.get(&pid).map(|p| {
                    let exe = p.args.split_whitespace().next().unwrap_or("");
                    exe.rsplit('/').next().unwrap_or(exe).to_string()
                })
            })
            .unwrap_or_default()
    }

    fn facts(&self, pid: u32, comm: &'a str) -> Facts<'a> {
        let ps = self.ps.get(&pid);
        Facts {
            pid,
            uid: ps.map(|p| p.uid),
            comm,
            args: ps.map(|p| p.args.as_str()).unwrap_or(""),
        }
    }
}

/// As portas em escuta, uma linha por (porta, pid), ordenadas por porta.
pub fn build_ports(scan: &Scan, ctx: &Ctx) -> Vec<PortEntry> {
    let world = world_of(scan, ctx);
    let index = Index::new(scan);
    let mut grouped: Vec<(u16, u32, Vec<String>)> = Vec::new();
    for l in &scan.listeners {
        match grouped
            .iter_mut()
            .find(|(p, pid, _)| *p == l.port && *pid == l.pid)
        {
            Some((_, _, addrs)) => {
                if !addrs.contains(&l.address) {
                    addrs.push(l.address.clone());
                }
            }
            None => grouped.push((l.port, l.pid, vec![l.address.clone()])),
        }
    }
    grouped.sort_by_key(|(port, pid, _)| (*port, *pid));
    grouped
        .into_iter()
        .map(|(port, pid, addresses)| {
            let comm = index.comm(pid);
            let facts = index.facts(pid, &comm);
            let origin = classify::classify(&facts, &world);
            let refusal = classify::refusal(&facts, &origin, &world);
            let ps = index.ps.get(&pid);
            PortEntry {
                port,
                addresses,
                pid,
                start_ms: index.rows.get(&pid).map(|r| r.start_ms).unwrap_or(0),
                command: cmdline::redact_command(facts.args),
                comm: comm.clone(),
                origin,
                cpu_percent: ps.map(|p| p.cpu),
                rss_bytes: ps.map(|p| p.rss_kb.saturating_mul(1024)),
                pgid: ps.map(|p| p.pgid),
                refusal,
            }
        })
        .collect()
}

/// Um nó da árvore do processo (detalhe e seção Processos).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TreeNode {
    pub pid: u32,
    pub start_ms: u64,
    pub depth: u32,
    pub comm: String,
    pub command: String,
    pub ports: Vec<u16>,
    pub cpu_percent: Option<f32>,
    pub rss_bytes: Option<u64>,
    pub refusal: Option<Refusal>,
}

fn children_map(world: &World) -> HashMap<u32, Vec<u32>> {
    let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&pid, &ppid) in &world.parent {
        if pid != ppid {
            kids.entry(ppid).or_default().push(pid);
        }
    }
    for list in kids.values_mut() {
        list.sort_unstable();
    }
    kids
}

fn subtree(root: u32, scan: &Scan, world: &World, kids: &HashMap<u32, Vec<u32>>) -> Vec<TreeNode> {
    let index = Index::new(scan);
    let mut ports: HashMap<u32, Vec<u16>> = HashMap::new();
    for l in &scan.listeners {
        let list = ports.entry(l.pid).or_default();
        if !list.contains(&l.port) {
            list.push(l.port);
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![(root, 0u32)];
    while let Some((pid, depth)) = stack.pop() {
        if out.len() >= TREE_CAP || !seen.insert(pid) {
            continue;
        }
        let comm = index.comm(pid);
        let facts = index.facts(pid, &comm);
        let origin = classify::classify(&facts, world);
        let ps = index.ps.get(&pid);
        let mut own_ports = ports.get(&pid).cloned().unwrap_or_default();
        own_ports.sort_unstable();
        out.push(TreeNode {
            pid,
            start_ms: index.rows.get(&pid).map(|r| r.start_ms).unwrap_or(0),
            depth,
            command: cmdline::redact_command(facts.args),
            refusal: classify::refusal(&facts, &origin, world),
            comm,
            ports: own_ports,
            cpu_percent: ps.map(|p| p.cpu),
            rss_bytes: ps.map(|p| p.rss_kb.saturating_mul(1024)),
        });
        if let Some(children) = kids.get(&pid) {
            for &child in children.iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
    out
}

/// O detalhe de uma porta: a árvore (do shell da sessão, ou do líder do grupo)
/// e quem vai junto no encerrar.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Detail {
    pub tree: Vec<TreeNode>,
    /// Os processos que o SIGTERM alcança (o próprio, se não for o grupo).
    pub group: Vec<GroupMember>,
    pub target: Option<SignalTarget>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GroupMember {
    pub pid: u32,
    pub comm: String,
}

fn group_of(scan: &Scan, world: &World, pid: u32) -> (Option<SignalTarget>, Vec<GroupMember>) {
    let index = Index::new(scan);
    let Some(row) = index.ps.get(&pid) else {
        return (None, Vec::new());
    };
    let members: Vec<&ps::PsRow> = scan.ps.iter().filter(|p| p.pgid == row.pgid).collect();
    let as_members: Vec<Member> = members
        .iter()
        .map(|p| Member {
            pid: p.pid,
            uid: p.uid,
        })
        .collect();
    let target = classify::signal_target(pid, row.pgid, &as_members, world);
    let group = match target {
        SignalTarget::Group { .. } => members
            .iter()
            .map(|p| GroupMember {
                pid: p.pid,
                comm: index.comm(p.pid),
            })
            .collect(),
        SignalTarget::Process { .. } => vec![GroupMember {
            pid,
            comm: index.comm(pid),
        }],
    };
    (Some(target), group)
}

pub fn build_detail(scan: &Scan, ctx: &Ctx, pid: u32) -> Detail {
    let world = world_of(scan, ctx);
    let kids = children_map(&world);
    let index = Index::new(scan);
    // Raiz: o shell da sessão; fora de sessão, o líder do grupo se ele é
    // ancestral; senão o próprio processo.
    let session_root = world.session_of(pid).and_then(|session| {
        ctx.shells
            .iter()
            .find(|(id, _)| *id == session)
            .map(|&(_, leader)| leader)
    });
    let group_root = || {
        let pgid = index.ps.get(&pid)?.pgid;
        let mut cur = pid;
        for _ in 0..64 {
            if cur == pgid {
                return Some(pgid);
            }
            cur = *world.parent.get(&cur)?;
            if cur <= 1 {
                return None;
            }
        }
        None
    };
    let root = session_root.or_else(group_root).unwrap_or(pid);
    let (target, group) = group_of(scan, &world, pid);
    Detail {
        tree: subtree(root, scan, &world, &kids),
        group,
        target,
    }
}

/// Seção Processos: a árvore de cada sessão viva, a partir do shell.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SessionProcs {
    pub session_id: SessionId,
    pub nodes: Vec<TreeNode>,
}

pub fn build_processes(scan: &Scan, ctx: &Ctx) -> Vec<SessionProcs> {
    let world = world_of(scan, ctx);
    let kids = children_map(&world);
    ctx.shells
        .iter()
        .map(|&(session_id, leader)| SessionProcs {
            session_id,
            nodes: subtree(leader, scan, &world, &kids),
        })
        .collect()
}

/// As portas de cada sessão (chip da sidebar).
pub fn session_ports(scan: &Scan, ctx: &Ctx) -> HashMap<SessionId, Vec<u16>> {
    classify::ports_by_session(&scan.listeners, &world_of(scan, ctx))
}

/// Varredura local. `with_ps = false` é a do chip (só quem escuta e a árvore).
pub fn scan_local(with_ps: bool) -> Result<Scan, AppError> {
    let listeners = listen::listeners()?;
    let rows = crate::agent::process_probe::snapshot();
    let ps = if with_ps { ps::all()? } else { Vec::new() };
    Ok(Scan {
        listeners,
        rows,
        ps,
    })
}

/// Só a árvore e o `ps` (seção Processos não precisa do lsof).
pub fn scan_processes() -> Result<Scan, AppError> {
    Ok(Scan {
        listeners: listen::listeners().unwrap_or_default(),
        rows: crate::agent::process_probe::snapshot(),
        ps: ps::all()?,
    })
}

/// O que o encerrar devolve.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct KillOutcome {
    /// O processo-alvo não existe mais.
    pub exited: bool,
    pub target: Option<SignalTarget>,
    /// Quando `false` com `exited = false`, a UI oferece o SIGKILL depois de
    /// `force_after_ms`.
    pub force_after_ms: u64,
}

/// TERMs mandados e ainda não resolvidos: o SIGKILL só passa por aqui.
#[derive(Debug, Default)]
pub struct TermLog {
    sent: HashMap<(u32, u64), Instant>,
}

impl TermLog {
    pub fn record(&mut self, key: (u32, u64), at: Instant) {
        self.sent.insert(key, at);
        // Não cresce sem fim: o que passou de um minuto não serve mais.
        self.sent
            .retain(|_, t| at.saturating_duration_since(*t) < Duration::from_secs(60));
    }

    /// `Ok` se houve TERM há pelo menos [`FORCE_AFTER`]; `Err(ms)` = falta
    /// tanto (ou `u64::MAX` = nunca houve TERM).
    pub fn may_force(&self, key: (u32, u64), now: Instant) -> Result<(), u64> {
        let Some(at) = self.sent.get(&key) else {
            return Err(u64::MAX);
        };
        let elapsed = now.saturating_duration_since(*at);
        if elapsed >= FORCE_AFTER {
            Ok(())
        } else {
            Err((FORCE_AFTER - elapsed).as_millis() as u64)
        }
    }

    pub fn forget(&mut self, key: (u32, u64)) {
        self.sent.remove(&key);
    }
}

/// Estado do core da feature.
#[derive(Default)]
pub struct ServicesState {
    term_log: Mutex<TermLog>,
    chip: Mutex<HashMap<SessionId, Vec<u16>>>,
}

pub type SharedServices = std::sync::Arc<ServicesState>;

impl ServicesState {
    pub fn chip(&self) -> HashMap<SessionId, Vec<u16>> {
        self.chip.lock().clone()
    }

    /// Grava as portas por sessão; `true` se mudou (o chamador emite).
    pub fn set_chip(&self, next: HashMap<SessionId, Vec<u16>>) -> bool {
        let mut cur = self.chip.lock();
        if *cur == next {
            return false;
        }
        *cur = next;
        true
    }

    /// SIGTERM, com a conferência de identidade e as recusas na hora.
    /// `confirmed` é o grupo que o usuário viu na confirmação (`None` = só o
    /// processo): alvo diferente na hora do sinal é recusado.
    pub fn terminate(
        &self,
        ctx: &Ctx,
        pid: u32,
        start_ms: u64,
        confirmed: Option<u32>,
    ) -> Result<KillOutcome, AppError> {
        let Some(target) = verify(ctx, pid, start_ms, confirmed)? else {
            return Ok(KillOutcome {
                exited: true,
                target: None,
                force_after_ms: 0,
            });
        };
        send(target, Signal::Term)?;
        let exited = wait_exit(pid, start_ms, TERM_WAIT);
        if exited {
            self.term_log.lock().forget((pid, start_ms));
        } else {
            self.term_log.lock().record((pid, start_ms), Instant::now());
        }
        Ok(KillOutcome {
            exited,
            target: Some(target),
            force_after_ms: if exited {
                0
            } else {
                FORCE_AFTER.as_millis() as u64
            },
        })
    }

    /// SIGKILL: só depois de um TERM a este mesmo processo há ≥ 5 s.
    pub fn force(
        &self,
        ctx: &Ctx,
        pid: u32,
        start_ms: u64,
        confirmed: Option<u32>,
    ) -> Result<KillOutcome, AppError> {
        if let Err(wait_ms) = self
            .term_log
            .lock()
            .may_force((pid, start_ms), Instant::now())
        {
            return Err(if wait_ms == u64::MAX {
                AppError::new("services.force_without_term")
            } else {
                AppError::new("services.force_too_soon")
                    .with("seconds", wait_ms.div_ceil(1000).to_string())
            });
        }
        let Some(target) = verify(ctx, pid, start_ms, confirmed)? else {
            self.term_log.lock().forget((pid, start_ms));
            return Ok(KillOutcome {
                exited: true,
                target: None,
                force_after_ms: 0,
            });
        };
        send(target, Signal::Kill)?;
        let exited = wait_exit(pid, start_ms, KILL_WAIT);
        self.term_log.lock().forget((pid, start_ms));
        Ok(KillOutcome {
            exited,
            target: Some(target),
            force_after_ms: 0,
        })
    }
}

/// O alvo de agora é o que o usuário confirmou? Puro.
pub fn check_confirmed(target: SignalTarget, confirmed: Option<u32>) -> Result<(), AppError> {
    let same = match (target, confirmed) {
        (SignalTarget::Group { pgid }, Some(seen)) => pgid == seen,
        (SignalTarget::Process { .. }, None) => true,
        _ => false,
    };
    if same {
        Ok(())
    } else {
        Err(AppError::new("services.target_changed"))
    }
}

/// O mesmo processo de quando a lista foi feita? `Ok(None)` = já saiu.
fn verify(
    ctx: &Ctx,
    pid: u32,
    start_ms: u64,
    confirmed: Option<u32>,
) -> Result<Option<SignalTarget>, AppError> {
    let rows = crate::agent::process_probe::snapshot();
    let Some(row) = rows.iter().find(|r| r.pid == pid) else {
        return Ok(None);
    };
    if row.start_ms != start_ms {
        return Err(AppError::new("services.process_changed").with("pid", pid.to_string()));
    }
    let scan = Scan {
        listeners: Vec::new(),
        rows,
        ps: ps::all()?,
    };
    if !scan.ps.iter().any(|p| p.pid == pid) {
        return Ok(None);
    }
    let target = decide(&scan, ctx, pid)?;
    check_confirmed(target, confirmed)?;
    // O `ps` pode levar segundos: a identidade é conferida de novo logo antes
    // do sinal, não só no começo.
    match crate::agent::process_probe::snapshot()
        .into_iter()
        .find(|r| r.pid == pid)
    {
        None => Ok(None),
        Some(r) if r.start_ms != start_ms => {
            Err(AppError::new("services.process_changed").with("pid", pid.to_string()))
        }
        Some(_) => Ok(Some(target)),
    }
}

/// Recusa ou alvo do sinal, sobre um retrato. Puro.
pub fn decide(scan: &Scan, ctx: &Ctx, pid: u32) -> Result<SignalTarget, AppError> {
    let world = world_of(scan, ctx);
    let index = Index::new(scan);
    let comm = index.comm(pid);
    let facts = index.facts(pid, &comm);
    let origin = classify::classify(&facts, &world);
    if let Some(reason) = classify::refusal(&facts, &origin, &world) {
        return Err(AppError::new("services.refused").with("reason", reason.code()));
    }
    let (target, _) = group_of(scan, &world, pid);
    target.ok_or_else(|| AppError::new("services.refused").with("reason", Refusal::Unknown.code()))
}

#[derive(Clone, Copy)]
enum Signal {
    Term,
    Kill,
}

#[cfg(unix)]
fn send(target: SignalTarget, sig: Signal) -> Result<(), AppError> {
    let sig = match sig {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    let rc = match target {
        SignalTarget::Group { pgid } if pgid > 1 => unsafe { libc::killpg(pgid as i32, sig) },
        SignalTarget::Process { pid } if pid > 1 => unsafe { libc::kill(pid as i32, sig) },
        _ => return Err(AppError::new("services.refused").with("reason", Refusal::Pid1.code())),
    };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(AppError::new("services.kill_failed").with("detail", err.to_string()))
}

#[cfg(not(unix))]
fn send(_target: SignalTarget, _sig: Signal) -> Result<(), AppError> {
    Err(AppError::new("services.unsupported"))
}

/// O estado de `/proc/<pid>/stat` é `Z` (zumbi: morreu, falta o pai colher)?
/// O campo vem logo depois do último `)` do comm.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn stat_is_zombie(stat: &str) -> bool {
    stat.rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().next())
        == Some("Z")
}

#[cfg(target_os = "linux")]
fn is_zombie(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| stat_is_zombie(&s))
}

/// No macOS o retrato já não traz zumbi.
#[cfg(not(target_os = "linux"))]
fn is_zombie(_pid: u32) -> bool {
    false
}

fn alive(pid: u32, start_ms: u64) -> bool {
    crate::agent::process_probe::snapshot()
        .iter()
        .any(|r| r.pid == pid && r.start_ms == start_ms)
        && !is_zombie(pid)
}

fn wait_exit(pid: u32, start_ms: u64, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    loop {
        if !alive(pid, start_ms) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(EXIT_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: u32 = 501;
    const TYBA: u32 = 900;

    fn sid(n: u128) -> SessionId {
        uuid::Uuid::from_u128(n)
    }

    fn row(pid: u32, ppid: u32, comm: &str) -> ProcRow {
        ProcRow {
            pid,
            ppid,
            comm: comm.into(),
            start_ms: pid as u64 * 10,
        }
    }

    fn psr(pid: u32, ppid: u32, pgid: u32, uid: u32, args: &str) -> ps::PsRow {
        ps::PsRow {
            pid,
            ppid,
            pgid,
            uid,
            cpu: 1.5,
            rss_kb: 100,
            args: args.into(),
        }
    }

    fn ctx() -> Ctx {
        Ctx {
            shells: vec![(sid(1), 1000)],
            tyba_pid: TYBA,
            tyba_pgid: TYBA,
            my_uid: ME,
        }
    }

    /// tyba(900) → zsh(1000) → bun(1010, grupo 1010) → vite(1020) :3000, esbuild(1030)
    /// node(2000) fora do TYBA em :5173 · ControlCenter(3000) em :7000
    fn scan() -> Scan {
        Scan {
            listeners: vec![
                listen::Listener {
                    pid: 1020,
                    port: 3000,
                    address: "*".into(),
                },
                listen::Listener {
                    pid: 1020,
                    port: 3000,
                    address: "::1".into(),
                },
                listen::Listener {
                    pid: 2000,
                    port: 5173,
                    address: "127.0.0.1".into(),
                },
                listen::Listener {
                    pid: 3000,
                    port: 7000,
                    address: "*".into(),
                },
            ],
            rows: vec![
                row(TYBA, 1, "tyba"),
                row(1000, TYBA, "zsh"),
                row(1010, 1000, "bun"),
                row(1020, 1010, "node"),
                row(1030, 1010, "esbuild"),
                row(2000, 1, "node"),
                row(3000, 1, "ControlCenter"),
            ],
            ps: vec![
                psr(
                    TYBA,
                    1,
                    TYBA,
                    ME,
                    "/Applications/TYBA.app/Contents/MacOS/tyba",
                ),
                psr(1000, TYBA, 1000, ME, "-zsh"),
                psr(1010, 1000, 1010, ME, "bun run dev --token=abc"),
                psr(1020, 1010, 1010, ME, "node vite"),
                psr(1030, 1010, 1010, ME, "esbuild --service"),
                psr(2000, 1, 2000, ME, "node vite --port 5173"),
                psr(
                    3000,
                    1,
                    3000,
                    ME,
                    "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter",
                ),
            ],
        }
    }

    #[test]
    fn ports_merge_addresses_and_classify() {
        let ports = build_ports(&scan(), &ctx());
        assert_eq!(ports.len(), 3);
        assert_eq!(ports[0].port, 3000);
        assert_eq!(ports[0].addresses, vec!["*", "::1"]);
        assert_eq!(ports[0].origin, Origin::Session { session_id: sid(1) });
        assert_eq!(ports[0].refusal, None);
        assert_eq!(ports[0].start_ms, 10200);
        assert_eq!(ports[0].pgid, Some(1010));
        assert_eq!(ports[0].rss_bytes, Some(100 * 1024));
        assert_eq!(ports[1].origin, Origin::Outside);
        assert_eq!(ports[2].origin, Origin::System);
        assert_eq!(ports[2].refusal, Some(Refusal::System));
    }

    #[test]
    fn detail_tree_starts_at_the_session_shell_and_lists_the_group() {
        let d = build_detail(&scan(), &ctx(), 1020);
        let tree: Vec<(u32, u32)> = d.tree.iter().map(|n| (n.pid, n.depth)).collect();
        assert_eq!(tree, vec![(1000, 0), (1010, 1), (1020, 2), (1030, 2)]);
        assert_eq!(d.tree[2].ports, vec![3000]);
        assert_eq!(d.tree[0].refusal, Some(Refusal::SessionShell));
        // O comando do bun sai redigido.
        assert_eq!(d.tree[1].command, "bun run dev --token=[REDACTED]");
        assert_eq!(d.target, Some(SignalTarget::Group { pgid: 1010 }));
        let group: Vec<u32> = d.group.iter().map(|m| m.pid).collect();
        assert_eq!(group, vec![1010, 1020, 1030]);
    }

    #[test]
    fn detail_outside_sessions_is_rooted_at_the_process() {
        let d = build_detail(&scan(), &ctx(), 2000);
        assert_eq!(d.tree.len(), 1);
        assert_eq!(d.target, Some(SignalTarget::Group { pgid: 2000 }));
    }

    #[test]
    fn processes_section_has_one_tree_per_session() {
        let procs = build_processes(&scan(), &ctx());
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].session_id, sid(1));
        assert_eq!(procs[0].nodes.len(), 4);
    }

    #[test]
    fn decide_refuses_in_the_core() {
        let s = scan();
        let refused = |pid| decide(&s, &ctx(), pid).unwrap_err().params["reason"].clone();
        assert_eq!(refused(1000), "session_shell");
        assert_eq!(refused(TYBA), "tyba");
        assert_eq!(refused(3000), "system");
        assert_eq!(refused(1), "pid1");
        assert_eq!(
            decide(&s, &ctx(), 1020).unwrap(),
            SignalTarget::Group { pgid: 1010 }
        );
    }

    #[test]
    fn decide_spares_the_shell_group_when_there_is_no_job_control() {
        let mut s = scan();
        // vite no grupo do shell (shell sem job control).
        for p in s.ps.iter_mut() {
            if p.pid == 1020 {
                p.pgid = 1000;
            }
        }
        assert_eq!(
            decide(&s, &ctx(), 1020).unwrap(),
            SignalTarget::Process { pid: 1020 }
        );
    }

    #[test]
    fn session_ports_feed_the_chip() {
        let by = session_ports(&scan(), &ctx());
        assert_eq!(by.get(&sid(1)), Some(&vec![3000]));
        assert_eq!(by.len(), 1);
    }

    #[test]
    fn force_needs_a_term_five_seconds_before() {
        let mut log = TermLog::default();
        let t0 = Instant::now();
        assert_eq!(log.may_force((5, 1), t0), Err(u64::MAX));
        log.record((5, 1), t0);
        let early = log.may_force((5, 1), t0 + Duration::from_secs(2));
        assert!(matches!(early, Err(ms) if ms > 2000 && ms <= 3000));
        assert_eq!(log.may_force((5, 1), t0 + FORCE_AFTER), Ok(()));
        // Outro processo no mesmo pid (start diferente) não herda o TERM.
        assert_eq!(log.may_force((5, 2), t0 + FORCE_AFTER), Err(u64::MAX));
        log.forget((5, 1));
        assert_eq!(log.may_force((5, 1), t0 + FORCE_AFTER), Err(u64::MAX));
    }

    #[test]
    fn the_confirmed_target_must_match() {
        let group = SignalTarget::Group { pgid: 10 };
        let only = SignalTarget::Process { pid: 11 };
        assert!(check_confirmed(group, Some(10)).is_ok());
        assert!(check_confirmed(only, None).is_ok());
        let changed = |t, c| check_confirmed(t, c).unwrap_err().code;
        assert_eq!(changed(group, None), "services.target_changed");
        assert_eq!(changed(group, Some(12)), "services.target_changed");
        assert_eq!(changed(only, Some(11)), "services.target_changed");
    }

    #[test]
    fn zombie_state_is_read_after_the_comm() {
        assert!(stat_is_zombie("42 (a) b) Z 1 42 42"));
        assert!(!stat_is_zombie("42 (Z) S 1 42 42"));
        assert!(!stat_is_zombie(""));
    }

    #[test]
    fn chip_reports_only_changes() {
        let state = ServicesState::default();
        let a = HashMap::from([(sid(1), vec![3000])]);
        assert!(state.set_chip(a.clone()));
        assert!(!state.set_chip(a));
        assert!(state.set_chip(HashMap::new()));
    }

    #[cfg(unix)]
    #[test]
    fn run_bounded_caps_output_and_runs_without_shell() {
        let out = run_bounded(Path::new("/bin/echo"), &["oi;", "$HOME"], 1024).unwrap();
        assert_eq!(String::from_utf8_lossy(&out), "oi; $HOME\n");
        let capped = run_bounded(Path::new("/bin/echo"), &["0123456789"], 4).unwrap();
        assert_eq!(capped, b"0123");
    }

    /// Prova viva do encerrar: um `sleep` no próprio grupo, conferido pelo
    /// start e encerrado com TERM ao grupo.
    #[cfg(unix)]
    #[test]
    fn terminate_kills_a_verified_process_group() {
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
        let mut child = cmd.spawn().expect("spawn sleep");
        let pid = child.id();
        let mut start = None;
        for _ in 0..50 {
            start = crate::agent::process_probe::snapshot()
                .into_iter()
                .find(|r| r.pid == pid)
                .map(|r| r.start_ms);
            if start.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let start = start.expect("sleep no retrato");
        let state = ServicesState::default();
        // O "TYBA" aqui não pode ser o runner de teste: o sleep é filho dele
        // e seria recusado como processo do próprio TYBA.
        let ctx = Ctx {
            tyba_pid: u32::MAX - 1,
            ..Ctx::current(Vec::new())
        };
        // Start errado: o core recusa, nada morre.
        let err = state
            .terminate(&ctx, pid, start + 1, Some(pid))
            .unwrap_err();
        assert_eq!(err.code, "services.process_changed");
        assert!(matches!(child.try_wait(), Ok(None)));
        // SIGKILL sem TERM antes: recusado.
        assert_eq!(
            state.force(&ctx, pid, start, Some(pid)).unwrap_err().code,
            "services.force_without_term"
        );
        let outcome = state
            .terminate(&ctx, pid, start, Some(pid))
            .expect("terminate");
        let _ = child.wait();
        assert_eq!(outcome.target, Some(SignalTarget::Group { pgid: pid }));
    }
}
