//! Credencial do peer de um socket unix, via `getsockopt(SO_PEERCRED)`.
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

#[cfg(all(unix, not(target_os = "linux")))]
pub fn peer_cred(_stream: &std::os::unix::net::UnixStream) -> Option<PeerCred> {
    // O canal shim↔core é escopo Linux nesta entrega (§15 do tech-spec):
    // macOS pede LOCAL_PEERCRED/getpeereid, não SO_PEERCRED — follow-on.
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

    #[cfg(target_os = "linux")]
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
}
