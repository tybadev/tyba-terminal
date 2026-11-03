//! Derrubar a sessão POSIX inteira de um PTY, não só o grupo do líder.
//!
//! Com controle de job ligado (`set -m`, todo shell interativo), cada job ganha
//! o próprio process group: `python -m http.server 8000 &` sobrevive ao
//! `killpg` do grupo do shell. Todos continuam na sessão (SID) que o
//! portable-pty abriu com `setsid` no spawn — é por ela que se acha cada grupo.
//!
//! Só para sessão efêmera (nó do Orbit, ver [`super::PtyPool::set_ephemeral`]),
//! no kill (Fechar, apagar, ■, ⌘Q) de todo nó. No `exit` do próprio shell só
//! o nó Terminal varre ([`super::PtyPool::set_sweep_on_exit`]): pelo EOF do
//! PTY (`sweep_after_exit`, macOS) e por um vigia da saída do líder
//! (`watch_leader_exit`, que é o que funciona no Linux).
//! A aba da grade segue com o `killpg` do líder: job com `nohup` sobreviver a
//! uma aba fechada é decisão de produto à parte.
//!
//! Escapa da varredura: processo que sai da sessão (`setsid`, daemon que faz
//! double-fork + `setsid`). Ele não carrega mais o SID e não há como achá-lo
//! por ele — é a mesma fronteira de qualquer terminal.
//!
//! Windows: sem SID nem process group. O spawn do ConPTY (`conpty_jailed`)
//! põe o líder num Job Object `KILL_ON_JOB_CLOSE`, e o kill faz
//! `TerminateJobObject`: a árvore inteira cai no kill e quando o handle é
//! solto. No `exit` do shell o Job segue aberto (o handle fica no mapa até o
//! nó fechar), então os filhos vivem até lá.

/// O SID a varrer no kill: o próprio pid do líder, só se ele É líder de
/// sessão. Um líder que não abriu sessão própria está na sessão de outro — a
/// do app, num caso torto — e varrer essa seria matar o terminal do dono.
#[cfg(unix)]
pub(super) fn session_of_leader(leader_pid: u32) -> Option<u32> {
    let pid = libc::pid_t::try_from(leader_pid).ok()?;
    let sid = unsafe { libc::getsid(pid) };
    let own = unsafe { libc::getsid(0) };
    (sid == pid && sid > 1 && sid != own).then_some(leader_pid)
}

#[cfg(not(unix))]
pub(super) fn session_of_leader(_leader_pid: u32) -> Option<u32> {
    None
}

/// Onde está o líder (filho direto do app) do ponto de vista do `wait`.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Leader {
    Running,
    /// Saiu e segue zumbi, sem `wait`: enquanto não é colhido, o número dele
    /// — que é o SID — não volta para processo nenhum, então quem ainda
    /// carrega aquele SID é da sessão que abrimos.
    #[cfg_attr(not(unix), allow(dead_code))]
    ExitedUnreaped,
    /// Já colhido (ECHILD) ou não é filho nosso: o número pode ter voltado.
    Gone,
}

/// Pergunta sem colher (`WNOWAIT`): quem colhe continua sendo o `child.kill`.
#[cfg(unix)]
pub(super) fn leader_state(leader_pid: u32) -> Leader {
    let Ok(pid) = libc::pid_t::try_from(leader_pid) else {
        return Leader::Gone;
    };
    if pid <= 1 {
        return Leader::Gone;
    }
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    match (rc, unsafe { info.si_pid() }) {
        (0, p) if p == pid => Leader::ExitedUnreaped,
        (0, _) => Leader::Running,
        _ => Leader::Gone,
    }
}

#[cfg(not(unix))]
pub(super) fn leader_state(_leader_pid: u32) -> Leader {
    Leader::Gone
}

/// Bloqueia até o líder sair, sem colher (`WNOWAIT`). `true` quando saiu;
/// `false` se ele não é (mais) filho nosso.
#[cfg(unix)]
pub(super) fn wait_leader_exit(leader_pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(leader_pid) else {
        return false;
    };
    if pid <= 1 {
        return false;
    }
    loop {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if rc == 0 {
            return unsafe { info.si_pid() } == pid;
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return false;
        }
    }
}

#[cfg(not(unix))]
pub(super) fn wait_leader_exit(_leader_pid: u32) -> bool {
    false
}

/// Os processos vivos que ainda carregam o SID `sid`. Zumbi não conta: no
/// Linux o líder não colhido (e o job morto à espera do `init`) segue em
/// `/proc` com o SID.
#[cfg(all(test, unix))]
pub(super) fn pids_in_session(sid: u32) -> Vec<libc::pid_t> {
    let Ok(sid) = libc::pid_t::try_from(sid) else {
        return Vec::new();
    };
    all_pids()
        .into_iter()
        .filter(|pid| unsafe { libc::getsid(*pid) } == sid)
        .filter(|pid| !is_zombie(*pid))
        .collect()
}

#[cfg(all(test, target_os = "linux"))]
pub(super) fn is_zombie(pid: libc::pid_t) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| Some(stat.rsplit_once(')')?.1.split_whitespace().next()? == "Z"))
        .unwrap_or(false)
}

/// No macOS o zumbi já não responde ao `getsid`.
#[cfg(all(test, unix, not(target_os = "linux")))]
pub(super) fn is_zombie(_pid: libc::pid_t) -> bool {
    false
}

/// SIGKILL em cada process group da sessão `sid`, o do líder por último.
///
/// Mais de uma passada: um job pode ter acabado de criar grupo novo entre a
/// listagem e o sinal. Para quando uma passada não acha grupo nenhum.
#[cfg(unix)]
pub(super) fn kill_session(sid: u32) {
    let Ok(sid) = libc::pid_t::try_from(sid) else {
        return;
    };
    if sid <= 1 || sid == unsafe { libc::getsid(0) } {
        return;
    }
    let own_group = unsafe { libc::getpgid(0) };
    for _ in 0..3 {
        let groups = groups_in_session(sid, own_group);
        if groups.is_empty() {
            break;
        }
        for group in groups.iter().filter(|g| **g != sid) {
            unsafe { libc::killpg(*group, libc::SIGKILL) };
        }
        unsafe { libc::killpg(sid, libc::SIGKILL) };
    }
}

#[cfg(not(unix))]
pub(super) fn kill_session(_sid: u32) {}

/// Os grupos dos processos vivos cuja sessão é `sid`, nunca 0, 1 nem o do app.
#[cfg(unix)]
fn groups_in_session(sid: libc::pid_t, own_group: libc::pid_t) -> Vec<libc::pid_t> {
    let mut groups: Vec<libc::pid_t> = all_pids()
        .into_iter()
        .filter(|pid| unsafe { libc::getsid(*pid) } == sid)
        .map(|pid| unsafe { libc::getpgid(pid) })
        .filter(|group| *group > 1 && *group != own_group)
        .collect();
    groups.sort_unstable();
    groups.dedup();
    groups
}

#[cfg(target_os = "macos")]
fn all_pids() -> Vec<libc::pid_t> {
    const PROC_ALL_PIDS: u32 = 1;
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
    buf.into_iter().take(count).filter(|p| *p > 1).collect()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn all_pids() -> Vec<libc::pid_t> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<libc::pid_t>().ok())
        .filter(|p| *p > 1)
        .collect()
}
