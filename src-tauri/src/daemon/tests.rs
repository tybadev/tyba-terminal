use std::io::BufReader;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::endpoint::{probe, publish, Probe, PublishError};
use super::host::Host;
use super::protocol::{
    read_frame, write_frame, Frame, Reply, Request, MAX_HEADER_BYTES, PROTOCOL_VERSION,
};
use super::server::Server;

/// Pasta curta: socket unix tem teto de ~104 bytes no caminho.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("tdd-{}", &Uuid::new_v4().simple().to_string()[..8]));
        Dir(dir)
    }
    fn socket(&self) -> PathBuf {
        self.0.join("daemon.sock")
    }
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ============================== endpoint ==============================

#[test]
fn publishing_on_a_fresh_path_serves_and_leaves_only_the_public_name() {
    let dir = Dir::new();
    let _listener = publish(&dir.socket()).unwrap();
    assert_eq!(probe(&dir.socket()), Probe::Serving);
    assert_eq!(
        dir.names(),
        vec!["daemon.lock".to_string(), "daemon.sock".to_string()]
    );
}

#[test]
fn a_second_daemon_declines_while_the_first_serves() {
    let dir = Dir::new();
    let _first = publish(&dir.socket()).unwrap();
    let before = std::fs::metadata(dir.socket()).unwrap().ino();
    assert!(matches!(
        publish(&dir.socket()),
        Err(PublishError::AlreadyServing)
    ));
    assert_eq!(std::fs::metadata(dir.socket()).unwrap().ino(), before);
    assert_eq!(probe(&dir.socket()), Probe::Serving);
    assert_eq!(
        dir.names(),
        vec!["daemon.lock".to_string(), "daemon.sock".to_string()]
    );
}

#[test]
fn a_dead_leftover_is_replaced_in_place() {
    let dir = Dir::new();
    crate::session::create_private_dir(&dir.0).unwrap();
    drop(UnixListener::bind(dir.socket()).unwrap());
    let dead = std::fs::metadata(dir.socket()).unwrap().ino();
    assert_eq!(probe(&dir.socket()), Probe::Dead);

    let _listener = publish(&dir.socket()).unwrap();
    assert_ne!(std::fs::metadata(dir.socket()).unwrap().ino(), dead);
    assert_eq!(probe(&dir.socket()), Probe::Serving);
    assert_eq!(
        dir.names(),
        vec!["daemon.lock".to_string(), "daemon.sock".to_string()]
    );
}

#[test]
fn what_is_not_a_socket_is_never_touched() {
    let dir = Dir::new();
    crate::session::create_private_dir(&dir.0).unwrap();
    std::fs::write(dir.socket(), b"de outro").unwrap();
    assert_eq!(probe(&dir.socket()), Probe::Unknown);
    assert!(matches!(
        publish(&dir.socket()),
        Err(PublishError::Undecided(_))
    ));
    assert_eq!(std::fs::read(dir.socket()).unwrap(), b"de outro");
    assert_eq!(
        dir.names(),
        vec!["daemon.lock".to_string(), "daemon.sock".to_string()]
    );
}

#[test]
fn a_live_daemon_keeps_its_name_even_when_its_socket_looks_dead() {
    // O caso do macOS: fila de `accept` cheia responde `ECONNREFUSED`. Aqui
    // o socket some de vez, e mesmo assim a trava do vivo segura o nome.
    let dir = Dir::new();
    let _first = publish(&dir.socket()).unwrap();
    std::fs::remove_file(dir.socket()).unwrap();
    assert!(matches!(
        publish(&dir.socket()),
        Err(PublishError::AlreadyServing)
    ));
    assert!(!dir.socket().exists());
}

#[test]
fn the_lock_of_a_dead_daemon_is_free_for_the_next() {
    let dir = Dir::new();
    drop(publish(&dir.socket()).unwrap());
    let _next = publish(&dir.socket()).unwrap();
    assert_eq!(probe(&dir.socket()), Probe::Serving);
}

#[test]
fn a_missing_socket_is_dead() {
    let dir = Dir::new();
    assert_eq!(probe(&dir.socket()), Probe::Dead);
}

#[test]
fn stopping_the_server_leaves_the_public_name_for_the_next_daemon() {
    let dir = Dir::new();
    let (server, _host) = start(&dir);
    server.stop(&dir.socket());
    std::thread::sleep(Duration::from_millis(100));
    assert!(dir.socket().exists());
}

// ============================== protocolo ==============================

#[test]
fn a_frame_carries_header_and_raw_bytes() {
    let id = Uuid::new_v4();
    let frame = Frame::with_body(Request::Write { seq: 7, id }, vec![0, 27, b'[', 255]);
    let mut wire = Vec::new();
    write_frame(&mut wire, &frame).unwrap();
    let back = read_frame::<Request>(&mut wire.as_slice())
        .unwrap()
        .unwrap();
    assert_eq!(back, frame);
    assert!(read_frame::<Request>(&mut &[][..]).unwrap().is_none());
}

#[test]
fn an_oversized_header_is_refused_before_reading_it() {
    let mut wire = Vec::new();
    wire.extend_from_slice(&(MAX_HEADER_BYTES + 1).to_be_bytes());
    wire.extend_from_slice(&0u32.to_be_bytes());
    assert!(read_frame::<Request>(&mut wire.as_slice()).is_err());
}

// ============================== servidor ==============================

fn start(dir: &Dir) -> (Arc<Server>, Arc<Host>) {
    let published = publish(&dir.socket()).unwrap();
    let host = Arc::new(Host::new());
    let server = Server::new(Arc::clone(&host));
    let serving = Arc::clone(&server);
    std::thread::spawn(move || serving.serve(&published.listener));
    (server, host)
}

struct Client {
    out: UnixStream,
    input: BufReader<UnixStream>,
    seq: u64,
}

impl Client {
    fn connect(socket: &Path) -> Client {
        let mut client = Client::raw(socket);
        client.send(
            Request::Hello {
                v: PROTOCOL_VERSION,
            },
            vec![],
        );
        assert!(matches!(
            client.next(),
            Reply::HelloOk {
                v: PROTOCOL_VERSION,
                ..
            }
        ));
        client
    }

    fn raw(socket: &Path) -> Client {
        let out = UnixStream::connect(socket).unwrap();
        out.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let input = BufReader::new(out.try_clone().unwrap());
        Client { out, input, seq: 0 }
    }

    fn send(&mut self, request: Request, body: Vec<u8>) {
        write_frame(&mut self.out, &Frame::with_body(request, body)).unwrap();
    }

    fn seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn frame(&mut self) -> Option<Frame<Reply>> {
        read_frame::<Reply>(&mut self.input).ok().flatten()
    }

    fn next(&mut self) -> Reply {
        self.frame().expect("o daemon respondeu").header
    }

    fn spawn(&mut self, id: Uuid, argv: &[&str], env: &[(&str, &str)]) -> Option<u32> {
        let seq = self.seq();
        self.send(
            Request::Spawn {
                seq,
                id,
                argv: argv.iter().map(|s| s.to_string()).collect(),
                env: env
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                cwd: None,
                cols: 80,
                rows: 24,
            },
            vec![],
        );
        match self.next() {
            Reply::Spawned { pid, .. } => pid,
            other => panic!("spawn: {other:?}"),
        }
    }

    /// O anel devolvido no `attach`.
    fn attach(&mut self, id: Uuid) -> Vec<u8> {
        let seq = self.seq();
        self.send(Request::Attach { seq, id }, vec![]);
        let frame = self.frame().expect("attach respondido");
        assert!(
            matches!(frame.header, Reply::Attached { .. }),
            "{:?}",
            frame.header
        );
        frame.body
    }

    /// Junta saída até `needle` aparecer; devolve tudo que viu.
    fn read_until(&mut self, mut seen: Vec<u8>, needle: &str) -> Vec<u8> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !String::from_utf8_lossy(&seen).contains(needle) {
            assert!(
                Instant::now() < deadline,
                "sem {needle:?} em {:?}",
                String::from_utf8_lossy(&seen)
            );
            match self.frame() {
                Some(Frame {
                    header: Reply::Output { .. },
                    body,
                }) => seen.extend(body),
                Some(_) => {}
                None => panic!("conexão fechou antes de {needle:?}"),
            }
        }
        seen
    }

    fn exit_code(&mut self) -> Option<i32> {
        loop {
            match self.next() {
                Reply::Exit { code, .. } => return code,
                Reply::Output { .. } | Reply::Ok { .. } => {}
                other => panic!("esperava a saída: {other:?}"),
            }
        }
    }
}

const PATH: (&str, &str) = ("PATH", "/usr/bin:/bin");

#[test]
fn a_spawned_shell_takes_input_and_reports_its_output_and_exit() {
    let dir = Dir::new();
    let _server = start(&dir);
    let mut client = Client::connect(&dir.socket());
    let id = Uuid::new_v4();
    client.spawn(
        id,
        &[
            "/bin/sh",
            "-c",
            "printf pronto; read x; printf 'eco:%s' \"$x\"",
        ],
        &[PATH],
    );
    let ring = client.attach(id);
    client.read_until(ring, "pronto");
    let seq = client.seq();
    client.send(Request::Write { seq, id }, b"abc\n".to_vec());
    client.read_until(vec![], "eco:abc");
    assert_eq!(client.exit_code(), Some(0));
}

#[test]
fn the_pty_outlives_its_client_and_the_next_one_gets_the_ring() {
    let dir = Dir::new();
    let _server = start(&dir);
    let id = Uuid::new_v4();
    {
        let mut first = Client::connect(&dir.socket());
        first.spawn(
            id,
            &["/bin/sh", "-c", "printf 'antes do app fechar'; sleep 30"],
            &[PATH],
        );
        let ring = first.attach(id);
        first.read_until(ring, "antes do app fechar");
    }
    let mut second = Client::connect(&dir.socket());
    let seq = second.seq();
    second.send(Request::List { seq }, vec![]);
    match second.next() {
        Reply::Ptys { ptys, .. } => {
            assert_eq!(ptys.len(), 1);
            assert_eq!(ptys[0].id, id);
            assert_eq!(ptys[0].exited, None);
        }
        other => panic!("list: {other:?}"),
    }
    let ring = second.attach(id);
    assert!(String::from_utf8_lossy(&ring).contains("antes do app fechar"));
    let seq = second.seq();
    second.send(Request::Kill { seq, id }, vec![]);
    second.exit_code();
}

#[test]
fn kill_takes_the_whole_process_group_down() {
    let dir = Dir::new();
    let _server = start(&dir);
    let mut client = Client::connect(&dir.socket());
    let id = Uuid::new_v4();
    let pid = client
        .spawn(
            id,
            &["/bin/sh", "-c", "sleep 300 & sleep 300 & printf ok; wait"],
            &[PATH],
        )
        .expect("pid do líder") as libc::pid_t;
    let ring = client.attach(id);
    client.read_until(ring, "ok");
    let group = unsafe { libc::getpgid(pid) };
    assert!(group > 0);
    let seq = client.seq();
    client.send(Request::Kill { seq, id }, vec![]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while unsafe { libc::killpg(group, 0) } == 0 {
        assert!(Instant::now() < deadline, "o grupo {group} continua vivo");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn killing_a_pty_whose_process_already_exited_signals_nobody() {
    let dir = Dir::new();
    let _server = start(&dir);
    let mut client = Client::connect(&dir.socket());
    let id = Uuid::new_v4();
    client.spawn(id, &["/bin/sh", "-c", "exit 3"], &[PATH]);
    let ring = client.attach(id);
    let _ = ring;
    assert_eq!(client.exit_code(), Some(3));
    let seq = client.seq();
    client.send(Request::Kill { seq, id }, vec![]);
    assert!(matches!(client.next(), Reply::Ok { .. }));
}

#[test]
fn the_process_gets_exactly_the_env_the_app_sent() {
    let dir = Dir::new();
    let _server = start(&dir);
    let mut client = Client::connect(&dir.socket());
    let id = Uuid::new_v4();
    client.spawn(id, &["/usr/bin/env"], &[("TYBA_TESTE", "sim")]);
    let ring = client.attach(id);
    let seen = client.read_until(ring, "TYBA_TESTE=sim");
    let text = String::from_utf8_lossy(&seen);
    assert!(!text.contains("HOME="), "env do daemon vazou: {text}");
    assert!(!text.contains("PATH="), "env do daemon vazou: {text}");
}

#[test]
fn a_client_of_another_protocol_version_is_refused() {
    let dir = Dir::new();
    let _server = start(&dir);
    let mut client = Client::raw(&dir.socket());
    client.send(
        Request::Hello {
            v: PROTOCOL_VERSION + 1,
        },
        vec![],
    );
    assert!(matches!(client.next(), Reply::Error { .. }));
    assert!(client.frame().is_none());
}

#[test]
fn nothing_is_served_before_the_hello() {
    let dir = Dir::new();
    let _server = start(&dir);
    let mut client = Client::raw(&dir.socket());
    client.send(Request::List { seq: 1 }, vec![]);
    assert!(matches!(client.next(), Reply::Error { .. }));
    assert!(client.frame().is_none());
}

#[test]
fn the_same_id_cannot_be_spawned_twice() {
    let dir = Dir::new();
    let _server = start(&dir);
    let mut client = Client::connect(&dir.socket());
    let id = Uuid::new_v4();
    client.spawn(id, &["/bin/sh", "-c", "sleep 30"], &[PATH]);
    let seq = client.seq();
    client.send(
        Request::Spawn {
            seq,
            id,
            argv: vec!["/bin/sh".into()],
            env: vec![],
            cwd: None,
            cols: 80,
            rows: 24,
        },
        vec![],
    );
    assert!(matches!(client.next(), Reply::Error { .. }));
    let seq = client.seq();
    client.send(Request::Kill { seq, id }, vec![]);
}
