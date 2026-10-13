//! Credencial do peer de um socket unix: `getsockopt(SO_PEERCRED)` no Linux,
//! `getpeereid` + `LOCAL_PEERPID` no macOS.
//!
//! FIX C1 do design-review: `std::os::unix::net::UnixStream::peer_cred` é
//! API instável (#42839) e não compila em Rust estável — por isso o
//! `getsockopt` cru via `libc`, não a stdlib.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCred {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
}

/// O peer é confiável para este canal quando o uid bate com o do processo
/// que aceitou a conexão — nunca root-vs-root por coincidência de zero, só
/// igualdade exata. Função pura: o teste não precisa de um processo de outro
/// uid de verdade (exigiria privilégio) para provar a decisão.
pub fn is_trusted_peer(cred: &PeerCred, expected_uid: u32) -> bool {
    cred.uid == expected_uid
}

#[cfg(target_os = "linux")]
pub fn peer_cred(stream: &std::os::unix::net::UnixStream) -> Option<PeerCred> {
    use std::os::unix::io::AsRawFd;

    let fd = stream.as_raw_fd();
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 || len as usize != std::mem::size_of::<libc::ucred>() {
        return None;
    }
    Some(PeerCred {
        pid: cred.pid as u32,
        uid: cred.uid,
        gid: cred.gid,
    })
}

/// macOS (M3 do claude-monitor): não há `SO_PEERCRED`. O uid/gid vêm de
/// `getpeereid` e o pid de `getsockopt(SOL_LOCAL, LOCAL_PEERPID)` — os dois
/// gravados pelo kernel no `connect`, nunca pelo que o peer afirma. Qualquer
/// das duas leituras falhando é `None`: o canal recusa (fail closed) e o
/// `_jail` do outro lado cai no `claude` cru de sempre.
#[cfg(target_os = "macos")]
pub fn peer_cred(stream: &std::os::unix::net::UnixStream) -> Option<PeerCred> {
    use std::os::unix::io::AsRawFd;

    let fd = stream.as_raw_fd();
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
        return None;
    }
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            &mut pid as *mut libc::pid_t as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 || len as usize != std::mem::size_of::<libc::pid_t>() || pid <= 0 {
        return None;
    }
    Some(PeerCred {
        pid: pid as u32,
        uid,
        gid,
    })
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub fn peer_cred(_stream: &std::os::unix::net::UnixStream) -> Option<PeerCred> {
    // Outros unix (BSDs) não têm o canal shim↔core: sem peer cred, recusa.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_trusted_peer_accepts_exact_uid_match() {
        let cred = PeerCred {
            pid: 123,
            uid: 1000,
            gid: 1000,
        };
        assert!(is_trusted_peer(&cred, 1000));
    }

    #[test]
    fn is_trusted_peer_rejects_any_uid_mismatch() {
        let cred = PeerCred {
            pid: 123,
            uid: 1000,
            gid: 1000,
        };
        assert!(
            !is_trusted_peer(&cred, 0),
            "uid 0 não é 1000, nem por engano de root-vs-root"
        );
        assert!(!is_trusted_peer(&cred, 1001));
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn peer_cred_reads_the_real_pid_and_uid_of_a_local_socket_pair() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();

        let connector =
            std::thread::spawn(move || std::os::unix::net::UnixStream::connect(&path).unwrap());
        let (server_side, _addr) = listener.accept().unwrap();
        let _client_side = connector.join().unwrap();

        let cred = peer_cred(&server_side).expect("SO_PEERCRED deveria responder");
        assert_eq!(cred.pid, std::process::id());
        assert_eq!(cred.uid, unsafe { libc::getuid() });
        assert_eq!(cred.gid, unsafe { libc::getgid() });
    }

    /// O pid vem do processo que fez `connect`, não de quem aceitou: um filho
    /// de verdade conectando tem que aparecer com o pid DELE. Só macOS: o
    /// `nc -U` faz parte do sistema lá; no Linux a CI pode não ter netcat.
    #[cfg(target_os = "macos")]
    #[test]
    fn peer_cred_reports_the_connecting_child_pid_not_the_listener() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();

        // `nc -U` conecta e fica parado lendo stdin (que fica aberto).
        let mut child = std::process::Command::new("nc")
            .arg("-U")
            .arg(&path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("nc");
        let (server_side, _addr) = listener.accept().unwrap();
        let cred = peer_cred(&server_side).expect("peer cred do filho");
        let child_pid = child.id();
        let _ = child.kill();
        let _ = child.wait();

        assert_eq!(cred.pid, child_pid);
        assert_ne!(cred.pid, std::process::id());
        assert!(is_trusted_peer(&cred, unsafe { libc::getuid() }));
    }
}
