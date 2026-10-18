//! Derrubar a sessão POSIX inteira de um PTY, não só o grupo do líder.
//!
//! Com controle de job ligado (`set -m`, todo shell interativo), cada job ganha
//! o próprio process group: `python -m http.server 8000 &` sobrevive ao
//! `killpg` do grupo do shell. Todos continuam na sessão (SID) que o
//! portable-pty abriu com `setsid` no spawn — é por ela que se acha cada grupo.
//!
//! Só para sessão efêmera (nó do Orbit, ver [`super::PtyPool::set_ephemeral`]).
//! A aba da grade segue com o `killpg` do líder: job com `nohup` sobreviver a
//! uma aba fechada é decisão de produto à parte.
//!
//! Windows: sem SID nem process group; fica como está (o `child.kill()` do
//! líder). Ver `kill_handle`.

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
