//! O canal shim↔core (shim v2, passo 2): socket por usuário, um pedido —
//! "hospede este agente" — sem cwd nem session-id (o core deriva os dois do
//! peer da conexão, nunca do que o pedido afirma). Ver o `[!danger]` da spec
//! e a ADR 2026-09-02: esta é uma superfície de IPC NOVA que pode pedir ao
//! core que suba um agente com gate numa pasta, e por isso o protocolo é
//! deliberadamente pequeno — um único `op`, um único agente por vez.
//!
//! Reusa o ESTILO de `hook_ipc::framing`/`server` (linha JSON, inflight
//! guard, disciplina de dir privado), mas NÃO o tipo `HookAction`: dois
//! sockets, dois protocolos, dois públicos.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const CHANNEL_PROTOCOL_VERSION: u32 = 1;
const SOCKET_FILE: &str = "channel.sock";

/// Endereço do socket por usuário: `$XDG_RUNTIME_DIR/tyba/channel.sock`
/// quando o runtime dir existe (o caso comum em qualquer sessão de login
/// systemd), senão um dir por uid sob `temp_dir()` — o mesmo padrão que
/// `session::integration_dir` já usa para não colidir entre usuários numa
/// `/tmp` compartilhada. Puro sobre o valor de env e o uid: o caller resolve
/// os dois, para que o teste não precise mutar `std::env` (global e frágil
/// entre threads de teste).
pub fn channel_socket_path(xdg_runtime_dir: Option<&Path>, uid: &str) -> PathBuf {
    match xdg_runtime_dir {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join("tyba").join(SOCKET_FILE),
        _ => std::env::temp_dir()
            .join(format!("tyba-channel-{uid}"))
            .join(SOCKET_FILE),
    }
}

#[cfg(unix)]
fn current_uid_string() -> String {
    unsafe { libc::getuid() }.to_string()
}

#[cfg(not(unix))]
fn current_uid_string() -> String {
    "user".to_string()
}

/// Resolve o endereço real desta máquina, lendo `XDG_RUNTIME_DIR` e o uid uma
/// única vez. Fora de teste — [`channel_socket_path`] é a peça pura.
pub fn resolve_channel_socket_path() -> PathBuf {
    let xdg = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    channel_socket_path(xdg.as_deref(), &current_uid_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRequest {
    pub v: u32,
    pub op: String,
    pub agent: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusedReason {
    NoSession,
    UnknownAgent,
    PeerUnresolved,
    NoCwd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ChannelResponse {
    Host { plan_path: String, jailed: bool },
    Refused { reason: RefusedReason },
}

/// O que a entrega B (`agent::channel_host`) fornece: dado o pid do peer já
/// autenticado (uid conferido) e o pedido já desserializado, decide o que
/// responder. `find_owning_session`, o `TOCTOU` recheck, a allowlist e
/// `prepare_hosted_agent` moram do lado de dentro deste fecho — não aqui.
#[cfg(target_os = "linux")]
pub type ChannelHandler =
    std::sync::Arc<dyn Fn(u32, ChannelRequest) -> ChannelResponse + Send + Sync>;

#[cfg(target_os = "linux")]
const HOST_AGENT_OP: &str = "host_agent";
#[cfg(target_os = "linux")]
pub const MAX_INFLIGHT: usize = 32;
/// Review round 2, achado MINOR (invariante #8, bounded reads): um pedido
/// de verdade é uma linha JSON minúscula (`{"v":1,"op":"host_agent",
/// "agent":"claude"}`, bem sob 100 bytes). O peer é same-uid mas NÃO é
/// confiável — pode mandar um stream gigante sem `\n` de propósito. Sem
/// teto, `read_line` cresce sem limite; combinado com `MAX_INFLIGHT`
/// conexões lentas, as threads do accept-loop ficam presas e o canal fica
/// surdo — um `claude` legítimo cai em fail-open cru.
#[cfg(target_os = "linux")]
const MAX_REQUEST_BYTES: u64 = 8 * 1024;
/// Mesmo achado: sem timeout de leitura, um peer que conecta e nunca manda
/// nada (nem dados nem EOF) trava a thread para sempre, e o teto de bytes
/// sozinho não ajuda nesse caso.
#[cfg(target_os = "linux")]
const REQUEST_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(target_os = "linux")]
struct InflightGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[cfg(target_os = "linux")]
impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(target_os = "linux")]
fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

// O canal (servidor) é escopo Linux nesta entrega (§15 do tech-spec): em
// qualquer outra plataforma nada faz `bind`, então o tipo inteiro — junto
// com o accept loop, o inflight guard e o dispatch — só existe sob
// `target_os = "linux"` (não `cfg(unix)`: macOS é unix e o peer cred via
// SO_PEERCRED que o servidor exige é Linux-only, ver `hook_ipc::peercred`).
// Windows E macOS ficam só com o protocolo (`ChannelRequest`/
// `ChannelResponse`) e a resolução de endereço, que `session::spawn_session`
// usa sem branch de SO.
#[cfg(target_os = "linux")]
pub struct ChannelServer {
    socket_path: PathBuf,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    accept_handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(target_os = "linux")]
impl ChannelServer {
    /// Liga o socket sob um diretório PRIVADO 0700 (a defesa primária — FIX
    /// C6) e ainda assim aplica 0600 no PRÓPRIO arquivo do socket (defesa em
    /// profundidade; `UnixListener::bind` nasce 0775, medido). O diretório é
    /// criado/verificado aqui: quem chama só decide o endereço.
    pub fn bind(socket_path: &Path, handler: ChannelHandler) -> std::io::Result<ChannelServer> {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;

        let parent = socket_path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "socket do canal sem diretório pai",
            )
        })?;
        crate::session::create_private_dir(parent)?;
        crate::session::verify_private_dir(parent)?;

        if socket_path.exists() {
            let _ = std::fs::remove_file(socket_path);
        }
        let listener = UnixListener::bind(socket_path)?;
        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;

        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let accept_shutdown = shutdown.clone();
        let inflight = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let accept_handle = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if accept_shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                match stream {
                    Ok(stream) => dispatch(stream, &handler, &inflight),
                    Err(_) => break,
                }
            }
        });

        Ok(ChannelServer {
            socket_path: socket_path.to_path_buf(),
            shutdown,
            accept_handle: Some(accept_handle),
        })
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = std::os::unix::net::UnixStream::connect(&self.socket_path);
        if let Some(handle) = self.accept_handle.take() {
            let _ = handle.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

#[cfg(target_os = "linux")]
impl Drop for ChannelServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "linux")]
fn dispatch(
    stream: std::os::unix::net::UnixStream,
    handler: &ChannelHandler,
    inflight: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let handler = handler.clone();
    let inflight = inflight.clone();
    if inflight.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= MAX_INFLIGHT {
        let _guard = InflightGuard(inflight);
        // Canal sobrecarregado: sem resposta é fail-open no cliente, o mesmo
        // efeito de qualquer outra dúvida — não vale inventar um 5º motivo
        // de recusa só para isto.
        return;
    }
    std::thread::spawn(move || {
        let _guard = InflightGuard(inflight);
        serve_connection(stream, &handler);
    });
}

#[cfg(target_os = "linux")]
fn serve_connection(stream: std::os::unix::net::UnixStream, handler: &ChannelHandler) {
    use std::io::{BufRead, Read, Write};

    // Achado MINOR do review round 2: timeout ANTES de ler nada — o peer
    // pode conectar e nunca mandar um byte.
    let _ = stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT));
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    // `.take(MAX_REQUEST_BYTES)` teta o total lido: sem `\n` dentro do
    // teto, `read_line` para (o `Take` sinaliza EOF), a linha meio-lida
    // falha o parse de JSON logo abaixo e a conexão fecha sem resposta —
    // o mesmo caminho que já existe para qualquer linha malformada.
    let mut reader = std::io::BufReader::new(read_half.take(MAX_REQUEST_BYTES));
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => return,
        Ok(_) => {}
    }
    let Ok(request) = serde_json::from_str::<ChannelRequest>(line.trim_end()) else {
        return;
    };
    if request.v != CHANNEL_PROTOCOL_VERSION || request.op != HOST_AGENT_OP {
        return;
    }

    let response = match super::peercred::peer_cred(&stream) {
        Some(cred) if super::peercred::is_trusted_peer(&cred, current_uid()) => {
            handler(cred.pid, request)
        }
        _ => ChannelResponse::Refused {
            reason: RefusedReason::NoSession,
        },
    };

    let Ok(mut payload) = serde_json::to_vec(&response) else {
        return;
    };
    payload.push(b'\n');
    let mut writer = stream;
    let _ = writer.write_all(&payload);
    let _ = writer.flush();
}

/// O plano que o shim executa: argv/env/cwd já resolvidos pelo core, porque
/// `/proc/cmdline` é 0444 world (medido) — não dá pra herdar por processo, e
/// o shim roda como FILHO do shell (nunca inspeciona o que o core decidiu por
/// fora deste arquivo). O mesmo tipo é usado pelos dois lados: quem escreve
/// (`agent::channel_host`, Track B) e quem lê (aqui).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedPlan {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JailOutcome {
    /// Qualquer dúvida — sem socket, socket surdo, recusado, plano ilegível —
    /// e o cliente sai do caminho: `tyba _jail` NÃO tenta rodar nada aqui,
    /// devolve o veredito para quem exec o binário de verdade.
    RunReal,
    Exec {
        argv: Vec<String>,
        env: Vec<(String, String)>,
        cwd: String,
    },
}

/// Timeouts curtos de propósito: isto roda INLINE no que o usuário digitou.
/// Um app que não está rodando falha a conexão na hora (ECONNREFUSED/ENOENT);
/// a única corrida que a retentativa cobre é o app subindo bem naquele
/// instante — por isso poucas tentativas, não a espera longa do hook.
#[cfg(target_os = "linux")]
const JAIL_CONNECT_ATTEMPTS: u32 = 3;
#[cfg(target_os = "linux")]
const JAIL_CONNECT_BASE: std::time::Duration = std::time::Duration::from_millis(20);

#[cfg(target_os = "linux")]
fn connect_with_retry(socket_path: &Path) -> Option<std::os::unix::net::UnixStream> {
    for attempt in 0..JAIL_CONNECT_ATTEMPTS {
        if let Ok(stream) = std::os::unix::net::UnixStream::connect(socket_path) {
            return Some(stream);
        }
        if attempt + 1 < JAIL_CONNECT_ATTEMPTS {
            std::thread::sleep(JAIL_CONNECT_BASE * 2u32.pow(attempt));
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn read_and_unlink_plan(plan_path: &str) -> Option<HostedPlan> {
    let path = Path::new(plan_path);
    let contents = std::fs::read(path).ok();
    // Unlink IMEDIATO, ganhe ou perca a leitura: o arquivo carrega argv/env
    // em claro (0600, mas ainda assim) e não deve sobreviver ao consumo.
    let _ = std::fs::remove_file(path);
    serde_json::from_slice(&contents?).ok()
}

/// A decisão pura do cliente: dado o socket (se houver), o que fazer. Não
/// executa nada — quem chama decide se roda [`JailOutcome::Exec`] ou cai no
/// binário de verdade. Separado de `run_jail_client` para que o teste prove
/// a decisão sem substituir o processo de teste por um exec real.
#[cfg(target_os = "linux")]
pub fn resolve_jail_outcome(socket_path: Option<&Path>) -> JailOutcome {
    let Some(socket_path) = socket_path else {
        return JailOutcome::RunReal;
    };
    let Some(stream) = connect_with_retry(socket_path) else {
        return JailOutcome::RunReal;
    };
    let request = ChannelRequest {
        v: CHANNEL_PROTOCOL_VERSION,
        op: HOST_AGENT_OP.into(),
        agent: "claude".into(),
    };
    let Some(response) = exchange(stream, &request) else {
        return JailOutcome::RunReal;
    };
    match response {
        ChannelResponse::Refused { .. } => JailOutcome::RunReal,
        ChannelResponse::Host { plan_path, .. } => match read_and_unlink_plan(&plan_path) {
            Some(plan) => JailOutcome::Exec {
                argv: plan.argv,
                env: plan.env,
                cwd: plan.cwd,
            },
            None => JailOutcome::RunReal,
        },
    }
}

#[cfg(target_os = "linux")]
fn exchange(
    stream: std::os::unix::net::UnixStream,
    request: &ChannelRequest,
) -> Option<ChannelResponse> {
    use std::io::{BufRead, Write};

    let mut writer = stream.try_clone().ok()?;
    let mut payload = serde_json::to_vec(request).ok()?;
    payload.push(b'\n');
    writer.write_all(&payload).ok()?;
    writer.flush().ok()?;

    let mut reader = std::io::BufReader::new(stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => serde_json::from_str(line.trim_end()).ok(),
    }
}

/// O endereço REAL que `tyba _jail` usa para conectar — sempre o canônico
/// (`resolve_channel_socket_path`, o MESMO cálculo que o core usa para
/// bindar, no MESMO binário), nunca o valor de `TYBA_CHANNEL_SOCK` do env.
///
/// Review round 2, achado MINOR: todo o hardening do canal (SO_PEERCRED,
/// recheck de starttime, allowlist, plano 0600) é do lado do SERVIDOR — o
/// cliente não validava a origem do endereço. Um `.envrc` (`direnv allow`)
/// ou profile sourceado com `export TYBA_CHANNEL_SOCK=/tmp/x.sock` faria o
/// `_jail` conectar cegamente num socket de OUTRO uid, ler o `plan_path`
/// que esse socket devolvesse e dar `exec` como o usuário — sem jaula, sem
/// nenhuma das checagens do servidor de verdade. `TYBA_CHANNEL_SOCK`
/// continua indo no env da sessão (`session::spawn_session`) como sinal de
/// "shim ligado" para o script do rc; só não é mais lido aqui.
#[cfg(target_os = "linux")]
pub fn jail_connect_socket_path() -> PathBuf {
    resolve_channel_socket_path()
}

/// Ponto de entrada do binário quando invocado como `tyba _jail` — o alvo do
/// `exec` que o shim faz depois de o shell interceptar `claude` puro (§6).
/// Fail-open vive INTEIRAMENTE aqui: qualquer dúvida (sem socket, recusado,
/// plano ilegível, exec falhou) cai no binário de verdade, sem argumento
/// nenhum — Q1 já garantiu que só `claude` sem args chega até aqui.
#[cfg(target_os = "linux")]
pub fn maybe_run_jail_mode() -> Option<i32> {
    if std::env::args().nth(1).as_deref() != Some("_jail") {
        return None;
    }
    Some(run_jail_client(Some(&jail_connect_socket_path())))
}

#[cfg(not(target_os = "linux"))]
pub fn maybe_run_jail_mode() -> Option<i32> {
    None
}

#[cfg(target_os = "linux")]
fn run_jail_client(socket_path: Option<&Path>) -> i32 {
    use std::os::unix::process::CommandExt;

    match resolve_jail_outcome(socket_path) {
        JailOutcome::Exec { argv, env, cwd } => {
            if let Some((program, rest)) = argv.split_first() {
                let mut cmd = std::process::Command::new(program);
                cmd.args(rest);
                cmd.current_dir(&cwd);
                cmd.env_clear();
                cmd.envs(env);
                let err = cmd.exec();
                eprintln!("tyba: exec do agente hospedado falhou: {err}");
            }
        }
        JailOutcome::RunReal => {}
    }
    // Fail-open final: ou a resposta foi RunReal, ou o exec acima falhou —
    // os dois caem aqui, e o caminho de volta é o mesmo: rodar o binário de
    // verdade sem nenhum argumento (Q1).
    let err = std::process::Command::new("claude").exec();
    eprintln!("tyba _jail: exec de claude falhou: {err}");
    126
}

// Gate consistente com o servidor/cliente que exercitam (§15): sem ele,
// Windows não compila estes testes (`ChannelServer`/`resolve_jail_outcome`/
// `MAX_REQUEST_BYTES` não existem lá) e macOS os compila mas os faz falhar
// em RUNTIME — `hook_ipc::peercred::peer_cred` real (SO_PEERCRED) é
// Linux-only, então o handshake completo que estes testes fazem de ponta a
// ponta não tem como funcionar fora do Linux.
#[cfg(test)]
#[cfg(target_os = "linux")]
mod client_tests {
    use super::*;

    fn bound_server(handler: ChannelHandler) -> (tempfile::TempDir, PathBuf, ChannelServer) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channel.sock");
        let server = ChannelServer::bind(&path, handler).unwrap();
        (dir, path, server)
    }

    #[test]
    fn no_socket_path_fails_open() {
        assert_eq!(resolve_jail_outcome(None), JailOutcome::RunReal);
    }

    #[test]
    fn an_unreachable_socket_fails_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nada-aqui.sock");
        assert_eq!(resolve_jail_outcome(Some(&path)), JailOutcome::RunReal);
    }

    #[test]
    fn a_refused_response_fails_open_regardless_of_reason() {
        for reason in [
            RefusedReason::NoSession,
            RefusedReason::UnknownAgent,
            RefusedReason::PeerUnresolved,
            RefusedReason::NoCwd,
        ] {
            let (_dir, path, server) = bound_server(std::sync::Arc::new(move |_pid, _req| {
                ChannelResponse::Refused { reason }
            }));
            assert_eq!(
                resolve_jail_outcome(Some(&path)),
                JailOutcome::RunReal,
                "recusa ({reason:?}) tem que sair do caminho, nunca travar o terminal"
            );
            server.shutdown();
        }
    }

    #[test]
    fn a_host_response_pointing_at_a_missing_plan_file_fails_open() {
        let (_dir, path, server) =
            bound_server(std::sync::Arc::new(|_pid, _req| ChannelResponse::Host {
                plan_path: "/tmp/tyba-plan-que-nao-existe-jamais.json".into(),
                jailed: true,
            }));
        assert_eq!(resolve_jail_outcome(Some(&path)), JailOutcome::RunReal);
        server.shutdown();
    }

    #[test]
    fn a_host_response_with_a_real_plan_reads_argv_env_cwd_and_unlinks_it() {
        let plan_dir = tempfile::tempdir().unwrap();
        let plan_path = plan_dir.path().join("launch.json");
        let plan = HostedPlan {
            argv: vec!["/usr/bin/tyba".into(), "_seccomp-exec".into()],
            env: vec![
                ("TYBA_HOSTED".into(), "1".into()),
                ("TYBA_JAILED".into(), "1".into()),
            ],
            cwd: "/work".into(),
        };
        std::fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();

        let plan_path_str = plan_path.to_string_lossy().into_owned();
        let (_dir, path, server) = bound_server(std::sync::Arc::new(move |_pid, _req| {
            ChannelResponse::Host {
                plan_path: plan_path_str.clone(),
                jailed: true,
            }
        }));

        let outcome = resolve_jail_outcome(Some(&path));
        assert_eq!(
            outcome,
            JailOutcome::Exec {
                argv: plan.argv.clone(),
                env: plan.env.clone(),
                cwd: plan.cwd.clone(),
            }
        );
        assert!(
            !plan_path.exists(),
            "o plano tem que ser removido assim que lido — /proc/cmdline é 0444 world, ninguém mais pode reler"
        );
        server.shutdown();
    }
}

// Mesmo motivo de `client_tests`: exercita `ChannelServer`/peer cred real de
// ponta a ponta, escopo Linux (§15).
#[cfg(test)]
#[cfg(target_os = "linux")]
mod server_tests {
    use super::*;
    use std::sync::Arc;

    fn socket_in(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        dir.path().join(name)
    }

    #[test]
    fn a_well_formed_request_reaches_the_handler_with_the_real_peer_pid() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_in(&dir, "channel.sock");
        let seen_pid = Arc::new(std::sync::Mutex::new(None));
        let sink = seen_pid.clone();
        let server = ChannelServer::bind(
            &path,
            Arc::new(move |pid, req| {
                *sink.lock().unwrap() = Some(pid);
                assert_eq!(req.op, "host_agent");
                assert_eq!(req.agent, "claude");
                ChannelResponse::Host {
                    plan_path: "/tmp/launch.json".into(),
                    jailed: true,
                }
            }),
        )
        .unwrap();

        let response = connect_and_request(
            &path,
            &ChannelRequest {
                v: CHANNEL_PROTOCOL_VERSION,
                op: "host_agent".into(),
                agent: "claude".into(),
            },
        );
        assert_eq!(
            response,
            Some(ChannelResponse::Host {
                plan_path: "/tmp/launch.json".into(),
                jailed: true,
            })
        );
        assert_eq!(*seen_pid.lock().unwrap(), Some(std::process::id()));
        server.shutdown();
    }

    #[test]
    fn a_malformed_line_closes_the_connection_without_a_response() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_in(&dir, "channel.sock");
        let server = ChannelServer::bind(
            &path,
            Arc::new(|_pid, _req| ChannelResponse::Refused {
                reason: RefusedReason::NoSession,
            }),
        )
        .unwrap();

        use std::io::Write;
        let mut stream = std::os::unix::net::UnixStream::connect(&path).unwrap();
        stream.write_all(b"not json at all\n").unwrap();
        stream.flush().unwrap();
        let mut out = String::new();
        std::io::Read::read_to_string(&mut stream, &mut out).unwrap();
        assert_eq!(out, "", "linha malformada não deve gerar resposta nenhuma");

        server.shutdown();
    }

    #[test]
    fn an_unknown_op_closes_the_connection_without_a_response() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_in(&dir, "channel.sock");
        let server = ChannelServer::bind(
            &path,
            Arc::new(|_pid, _req| ChannelResponse::Refused {
                reason: RefusedReason::NoSession,
            }),
        )
        .unwrap();

        let response = connect_and_request(
            &path,
            &ChannelRequest {
                v: CHANNEL_PROTOCOL_VERSION,
                op: "delete_everything".into(),
                agent: "claude".into(),
            },
        );
        assert_eq!(response, None);

        server.shutdown();
    }

    #[test]
    fn the_socket_is_born_0600_after_the_c6_fix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = socket_in(&dir, "channel.sock");
        let server = ChannelServer::bind(
            &path,
            Arc::new(|_pid, _req| ChannelResponse::Refused {
                reason: RefusedReason::NoSession,
            }),
        )
        .unwrap();

        let mode = std::fs::symlink_metadata(&path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);

        server.shutdown();
    }

    /// Review round 2, achado MINOR (invariante #8, bounded reads): um peer
    /// same-uid mas NÃO confiável manda um stream gigante sem `\n` de
    /// propósito. Sem o teto de `MAX_REQUEST_BYTES` + `set_read_timeout`,
    /// essa conexão prenderia a thread do accept-loop para sempre — e com
    /// `MAX_INFLIGHT` delas, o canal inteiro fica surdo e um `claude`
    /// legítimo cai em fail-open cru. Prova as DUAS metades: a conexão
    /// gigante é cortada sem travar (limitada por `recv_timeout`, pra virar
    /// FALHA de teste, nunca um hang de verdade, se o fix for revertido) E
    /// o servidor segue respondendo normalmente depois — nada ficou preso
    /// atrás dela.
    #[test]
    fn an_oversized_request_without_a_newline_is_cut_off_without_blocking_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_in(&dir, "channel.sock");
        let server = ChannelServer::bind(
            &path,
            Arc::new(|_pid, _req| ChannelResponse::Host {
                plan_path: "/tmp/launch.json".into(),
                jailed: false,
            }),
        )
        .unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let giant_path = path.clone();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&giant_path) else {
                let _ = tx.send(Vec::new());
                return;
            };
            // Bem maior que MAX_REQUEST_BYTES, e SEM "\n" nenhum.
            let filler = vec![b'x'; (MAX_REQUEST_BYTES * 4) as usize];
            // Pode falhar no meio (EPIPE) se o servidor já tiver fechado a
            // conexão ao bater o teto — aceitável, é exatamente o efeito
            // esperado do corte.
            let _ = stream.write_all(&filler);
            let mut out = Vec::new();
            let _ = stream.read_to_end(&mut out);
            let _ = tx.send(out);
        });

        let out = rx.recv_timeout(std::time::Duration::from_secs(10)).expect(
            "a conexão gigante sem \\n travou o servidor — achado MINOR do \
             review round 2 (bounded reads, invariante #8)",
        );
        assert!(
            out.is_empty(),
            "pedido cortado não pode gerar resposta nenhuma: {out:?}"
        );

        // O servidor segue vivo: um pedido normal, DEPOIS do gigante,
        // responde como sempre — nada ficou preso atrás dele.
        let response = connect_and_request(
            &path,
            &ChannelRequest {
                v: CHANNEL_PROTOCOL_VERSION,
                op: "host_agent".into(),
                agent: "claude".into(),
            },
        );
        assert_eq!(
            response,
            Some(ChannelResponse::Host {
                plan_path: "/tmp/launch.json".into(),
                jailed: false,
            }),
            "o servidor precisa continuar respondendo normalmente depois do pedido cortado"
        );

        server.shutdown();
    }

    /// Helper de teste: conecta, escreve o pedido, lê a resposta — o mesmo
    /// papel que o cliente `tyba _jail` cumpre, mas sem a política de
    /// fail-open (o teste quer distinguir "sem resposta" de "resposta X").
    fn connect_and_request(path: &Path, req: &ChannelRequest) -> Option<ChannelResponse> {
        use std::io::{BufRead, Write};
        let mut stream = std::os::unix::net::UnixStream::connect(path).ok()?;
        let mut line = serde_json::to_vec(req).ok()?;
        line.push(b'\n');
        stream.write_all(&line).ok()?;
        stream.flush().ok()?;
        let read_half = stream.try_clone().ok()?;
        let mut reader = std::io::BufReader::new(read_half);
        let mut out = String::new();
        match reader.read_line(&mut out) {
            Ok(0) | Err(_) => None,
            Ok(_) => serde_json::from_str(out.trim_end()).ok(),
        }
    }
}

#[cfg(test)]
mod address_tests {
    use super::*;

    #[test]
    fn prefers_xdg_runtime_dir_when_present() {
        let path = channel_socket_path(Some(Path::new("/run/user/1000")), "1000");
        assert_eq!(
            path,
            Path::new("/run/user/1000/tyba/channel.sock"),
            "deve morar sob {{XDG_RUNTIME_DIR}}/tyba, não direto na raiz do runtime dir"
        );
    }

    #[test]
    fn falls_back_to_a_per_uid_temp_dir_without_xdg_runtime_dir() {
        let path = channel_socket_path(None, "1000");
        assert_eq!(
            path,
            std::env::temp_dir().join("tyba-channel-1000/channel.sock")
        );
    }

    #[test]
    fn falls_back_when_xdg_runtime_dir_is_present_but_empty() {
        let path = channel_socket_path(Some(Path::new("")), "1000");
        assert_eq!(
            path,
            std::env::temp_dir().join("tyba-channel-1000/channel.sock"),
            "XDG_RUNTIME_DIR vazio não é um diretório utilizável"
        );
    }

    /// Review round 2, achado MINOR: `_jail` conecta no socket CANÔNICO
    /// mesmo com `TYBA_CHANNEL_SOCK` apontando para outro lugar — o env não
    /// pode ser a fonte de verdade do endereço de connect, senão um
    /// `.envrc`/profile hostil aponta o cliente para o socket de outro uid.
    /// `resolve_channel_socket_path` nem olha `TYBA_CHANNEL_SOCK` (só
    /// `XDG_RUNTIME_DIR` + uid), então esta prova é uma barreira de
    /// regressão: se alguém no futuro reintroduzir a leitura do env aqui,
    /// este teste denuncia o desvio comparando os dois valores.
    #[test]
    #[cfg(target_os = "linux")]
    fn jail_connect_socket_path_ignores_a_spoofed_tyba_channel_sock() {
        // Não precisa nem mutar o env de verdade: o ponto é que a função
        // nunca lê essa variável — mas se ela existir no ambiente deste
        // processo de teste apontando para outro lugar, o comportamento
        // continua o mesmo.
        std::env::set_var("TYBA_CHANNEL_SOCK", "/tmp/socket-de-outro-uid.sock");
        let canonical = resolve_channel_socket_path();
        let used_by_jail = jail_connect_socket_path();
        std::env::remove_var("TYBA_CHANNEL_SOCK");

        assert_eq!(
            used_by_jail, canonical,
            "_jail tem que usar o endereço canônico, nunca o do env"
        );
        assert_ne!(
            used_by_jail,
            PathBuf::from("/tmp/socket-de-outro-uid.sock"),
            "o endereço de connect não pode ser influenciável pelo env"
        );
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_wire_shape_has_no_cwd_and_no_session_id() {
        let req = ChannelRequest {
            v: CHANNEL_PROTOCOL_VERSION,
            op: "host_agent".into(),
            agent: "claude".into(),
        };
        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(
            value,
            json!({"v": 1, "op": "host_agent", "agent": "claude"})
        );
        assert!(value.get("cwd").is_none());
        assert!(value.get("session_id").is_none());
    }

    #[test]
    fn request_round_trips_through_json() {
        let req = ChannelRequest {
            v: 1,
            op: "host_agent".into(),
            agent: "claude".into(),
        };
        let line = serde_json::to_string(&req).unwrap();
        let back: ChannelRequest = serde_json::from_str(&line).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn host_response_wire_shape() {
        let resp = ChannelResponse::Host {
            plan_path: "/tmp/x/launch.json".into(),
            jailed: true,
        };
        let value = serde_json::to_value(&resp).unwrap();
        assert_eq!(
            value,
            json!({"status": "host", "plan_path": "/tmp/x/launch.json", "jailed": true})
        );
    }

    #[test]
    fn refused_response_wire_shape_for_every_reason() {
        for (reason, wire) in [
            (RefusedReason::NoSession, "no_session"),
            (RefusedReason::UnknownAgent, "unknown_agent"),
            (RefusedReason::PeerUnresolved, "peer_unresolved"),
            (RefusedReason::NoCwd, "no_cwd"),
        ] {
            let resp = ChannelResponse::Refused { reason };
            let value = serde_json::to_value(&resp).unwrap();
            assert_eq!(value, json!({"status": "refused", "reason": wire}));
        }
    }
}
