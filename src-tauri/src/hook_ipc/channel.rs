//! O canal shim↔core (shim v2, passo 2): socket por usuário, um pedido —
//! "hospede este agente" — sem cwd nem session-id (o core deriva os dois do
//! peer da conexão, nunca do que o pedido afirma). Ver o `[!danger]` da spec
//! e a ADR 2026-09-02: esta é uma superfície de IPC NOVA que pode pedir ao
//! core que suba um agente numa pasta, e por isso o protocolo é
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
        // `tyba` ou `tyba-dev` (ver [`crate::profile`]): o app instalado e o
        // build de dev têm canais separados, e o `_jail` de cada um acha o seu.
        Some(dir) if !dir.as_os_str().is_empty() => {
            dir.join(crate::profile::CHANNEL_TAG).join(SOCKET_FILE)
        }
        _ => std::env::temp_dir()
            .join(format!("{}-channel-{uid}", crate::profile::CHANNEL_TAG))
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
    #[cfg(target_os = "macos")]
    let base = darwin_user_temp_dir();
    #[cfg(not(target_os = "macos"))]
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    channel_socket_path(base.as_deref(), &current_uid_string())
}

/// macOS não tem `XDG_RUNTIME_DIR`. O equivalente é o temp dir por usuário
/// do sistema (`/var/folders/.../T/`, 0700 e do dono), lido por `confstr` —
/// nunca de `$TMPDIR`: um `.envrc` ou profile mudando o env não pode apontar
/// o `_jail` para um socket de outro lugar, e app (lançado pelo launchd) e
/// shell chegam no MESMO caminho.
#[cfg(target_os = "macos")]
fn darwin_user_temp_dir() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    let len = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
        )
    };
    if len == 0 || len > buf.len() {
        return None;
    }
    buf.truncate(len - 1);
    Some(PathBuf::from(std::ffi::OsString::from_vec(buf)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRequest {
    pub v: u32,
    pub op: String,
    pub agent: String,
    /// Argumentos que o usuário digitou depois de `claude` (M6-A), um por
    /// elemento — nunca uma string de shell. Ausente num cliente antigo:
    /// `claude` puro. O core reclassifica com `agent::claude_cli::route`
    /// antes de usar, nunca confia no cliente.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// M6-B (`op: "plugin_event"`): o token do pane (`TYBA_PANE_TOKEN`), o
    /// evento do hook, a ferramenta e um resumo já redigido. Só isso — o
    /// payload inteiro do hook nunca atravessa o canal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Fatia 5: o `notification_type` do `Notification` (só identificador).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusedReason {
    NoSession,
    UnknownAgent,
    PeerUnresolved,
    NoCwd,
    /// Os args pedem algo que desligaria o gate (M6-A): o cliente mostra o
    /// motivo e NÃO cai no `claude` cru.
    ForbiddenArgs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ChannelResponse {
    Host {
        plan_path: String,
        jailed: bool,
    },
    Refused {
        reason: RefusedReason,
    },
    /// M6-B: o core recebeu o evento do plugin. Não há decisão: o plugin só
    /// observa.
    Plugin,
}

/// O que a entrega B (`agent::channel_host`) fornece: dado o pid do peer já
/// autenticado (uid conferido) e o pedido já desserializado, decide o que
/// responder. `find_owning_session`, o `TOCTOU` recheck, a allowlist e
/// `prepare_hosted_agent` moram do lado de dentro deste fecho — não aqui.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub type ChannelHandler =
    std::sync::Arc<dyn Fn(u32, ChannelRequest) -> ChannelResponse + Send + Sync>;

#[cfg(any(target_os = "linux", target_os = "macos"))]
const HOST_AGENT_OP: &str = "host_agent";
pub const PLUGIN_EVENT_OP: &str = "plugin_event";
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub const MAX_INFLIGHT: usize = 32;
/// Review round 2, achado MINOR (invariante #8, bounded reads): um pedido
/// de verdade é uma linha JSON minúscula (`{"v":1,"op":"host_agent",
/// "agent":"claude"}`, bem sob 100 bytes). O peer é same-uid mas NÃO é
/// confiável — pode mandar um stream gigante sem `\n` de propósito. Sem
/// teto, `read_line` cresce sem limite; combinado com `MAX_INFLIGHT`
/// conexões lentas, as threads do accept-loop ficam presas e o canal fica
/// surdo — um `claude` legítimo cai em fail-open cru. Desde o M6-A o pedido
/// carrega os args do `claude` (um prompt de `-p` cabe aqui), então o teto
/// subiu de 8 KiB para 256 KiB — ainda limitado.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MAX_REQUEST_BYTES: u64 = 256 * 1024;
/// Mesmo achado: sem timeout de leitura, um peer que conecta e nunca manda
/// nada (nem dados nem EOF) trava a thread para sempre, e o teto de bytes
/// sozinho não ajuda nesse caso.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const REQUEST_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct InflightGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

// O canal (servidor) existe onde há peer cred do kernel: Linux
// (SO_PEERCRED) e, desde o M3 do claude-monitor, macOS (getpeereid +
// LOCAL_PEERPID), ver `hook_ipc::peercred`. Windows fica só com o protocolo
// (`ChannelRequest`/`ChannelResponse`) e a resolução de endereço.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub struct ChannelServer {
    socket_path: PathBuf,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    accept_handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for ChannelServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn serve_connection(stream: std::os::unix::net::UnixStream, handler: &ChannelHandler) {
    use std::io::{BufRead, Read, Write};

    // M3 (pid reuse): a credencial e o start time do peer são lidos JÁ no
    // accept, antes de esperar a linha (que pode levar até
    // REQUEST_READ_TIMEOUT). Se o processo que conectou sair e o pid for
    // reciclado nesse meio-tempo, o start time muda e a conexão é recusada —
    // o handler nunca vê um pid que não é mais o de quem conectou.
    let peer = super::peercred::peer_cred(&stream);
    let peer_start = peer.and_then(|cred| crate::repo::process_start_time(cred.pid));

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
    if request.v != CHANNEL_PROTOCOL_VERSION
        || (request.op != HOST_AGENT_OP && request.op != PLUGIN_EVENT_OP)
    {
        return;
    }

    let response = match peer {
        Some(cred)
            if super::peercred::is_trusted_peer(&cred, current_uid())
                && peer_start.is_some()
                && crate::repo::process_start_time(cred.pid) == peer_start =>
        {
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
///
/// Env (agente livre): o claude sobe com o env do PRÓPRIO shim — o do shell
/// do pane —, menos `env_remove`, com `env` (só o que o TYBA acrescenta:
/// socket de hooks, marcadores) por cima. Ver [`hosted_child_env`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedPlan {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_remove: Vec<String>,
    pub cwd: String,
}

/// O env final do claude hospedado: o herdado do shell do pane, sem as
/// chaves de `remove`, com `set` por cima. Pura sobre `OsString`, para que
/// valor não-UTF-8 do shell atravesse intacto.
pub fn hosted_child_env(
    inherited: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    remove: &[String],
    set: &[(String, String)],
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let overridden = |k: &std::ffi::OsString| {
        remove.iter().any(|r| k.as_os_str() == r.as_str())
            || set.iter().any(|(s, _)| k.as_os_str() == s.as_str())
    };
    inherited
        .into_iter()
        .filter(|(k, _)| !overridden(k))
        .chain(set.iter().map(|(k, v)| (k.into(), v.into())))
        .collect()
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
        env_remove: Vec<String>,
        cwd: String,
    },
    /// O core recusou os args porque desligariam o gate (M6-A): nada roda,
    /// nem cru.
    Forbidden,
}

/// Timeouts curtos de propósito: isto roda INLINE no que o usuário digitou.
/// Um app que não está rodando falha a conexão na hora (ECONNREFUSED/ENOENT);
/// a única corrida que a retentativa cobre é o app subindo bem naquele
/// instante — por isso poucas tentativas, não a espera longa do hook.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const JAIL_CONNECT_ATTEMPTS: u32 = 3;
#[cfg(any(target_os = "linux", target_os = "macos"))]
const JAIL_CONNECT_BASE: std::time::Duration = std::time::Duration::from_millis(20);
/// Teto da espera pela resposta: o core monta o plano (git, sandbox) antes de
/// responder, mas um core travado nunca pode prender o terminal — estourou,
/// cai no `claude` cru.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const JAIL_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[cfg(any(target_os = "linux", target_os = "macos"))]
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

/// O diretório do socket tem que ser do usuário e fechado para os outros
/// (0700) — o mesmo que o servidor exige ao ligar. Só leitura: o cliente
/// nunca corrige permissão, recusa. Cobre o caso de o endereço cair num
/// `/tmp` compartilhado, onde outro usuário poderia ter criado o diretório
/// antes e plantado um socket que devolve um plano dele.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn socket_dir_is_ours(socket_path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Some(meta) = socket_path
        .parent()
        .and_then(|dir| std::fs::symlink_metadata(dir).ok())
    else {
        return false;
    };
    meta.is_dir() && meta.uid() == current_uid() && meta.mode() & 0o077 == 0
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
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
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn resolve_jail_outcome(socket_path: Option<&Path>, args: &[String]) -> JailOutcome {
    let Some(socket_path) = socket_path else {
        return JailOutcome::RunReal;
    };
    if !socket_dir_is_ours(socket_path) {
        return JailOutcome::RunReal;
    }
    let Some(stream) = connect_with_retry(socket_path) else {
        return JailOutcome::RunReal;
    };
    // O cliente também confere quem está do outro lado: o plano que ele vai
    // executar só vale se veio de um processo do MESMO uid (o core).
    let server_is_ours = super::peercred::peer_cred(&stream)
        .is_some_and(|cred| super::peercred::is_trusted_peer(&cred, current_uid()));
    if !server_is_ours {
        return JailOutcome::RunReal;
    }
    let request = ChannelRequest {
        v: CHANNEL_PROTOCOL_VERSION,
        op: HOST_AGENT_OP.into(),
        agent: "claude".into(),
        args: args.to_vec(),
        token: None,
        event: None,
        tool: None,
        summary: None,
        notification_type: None,
    };
    let Some(response) = exchange(stream, &request) else {
        return JailOutcome::RunReal;
    };
    match response {
        ChannelResponse::Refused {
            reason: RefusedReason::ForbiddenArgs,
        } => JailOutcome::Forbidden,
        ChannelResponse::Refused { .. } | ChannelResponse::Plugin => JailOutcome::RunReal,
        ChannelResponse::Host { plan_path, .. } => match read_and_unlink_plan(&plan_path) {
            Some(plan) => JailOutcome::Exec {
                argv: plan.argv,
                env: plan.env,
                env_remove: plan.env_remove,
                cwd: plan.cwd,
            },
            None => JailOutcome::RunReal,
        },
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn exchange(
    stream: std::os::unix::net::UnixStream,
    request: &ChannelRequest,
) -> Option<ChannelResponse> {
    use std::io::{BufRead, Write};

    stream.set_read_timeout(Some(JAIL_RESPONSE_TIMEOUT)).ok()?;
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

/// M6-B: manda um evento do plugin do Claude Code ao core e devolve a
/// resposta, ou `None` se o core não respondeu (o evento se perde e o Claude
/// segue). Mesmas checagens do `_jail`: dir do socket do usuário e
/// servidor do mesmo uid.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn send_plugin_event(
    token: String,
    event: String,
    tool: Option<String>,
    summary: Option<String>,
    notification_type: Option<String>,
) -> Option<ChannelResponse> {
    let socket_path = jail_connect_socket_path();
    if !socket_dir_is_ours(&socket_path) {
        return None;
    }
    let stream = connect_with_retry(&socket_path)?;
    let server_is_ours = super::peercred::peer_cred(&stream)
        .is_some_and(|cred| super::peercred::is_trusted_peer(&cred, current_uid()));
    if !server_is_ours {
        return None;
    }
    let request = ChannelRequest {
        v: CHANNEL_PROTOCOL_VERSION,
        op: PLUGIN_EVENT_OP.into(),
        agent: "claude".into(),
        args: Vec::new(),
        token: Some(token),
        event: Some(event),
        tool,
        summary,
        notification_type,
    };
    exchange(stream, &request)
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
/// nenhuma das checagens do servidor de verdade. Depois desse achado a
/// variável virou env morto (o rc nunca a leu — só olha `TYBA_BIN`), então
/// o core parou de exportá-la; o teste abaixo continua provando que, mesmo
/// que alguém a injete, ela é ignorada.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn jail_connect_socket_path() -> PathBuf {
    resolve_channel_socket_path()
}

/// Os args de `tyba _jail [--] <args do claude>`: tudo depois do `--`
/// (M6-A), ou nada num shim antigo que chamava `_jail` puro.
pub fn jail_args(argv: &[String]) -> Vec<String> {
    let rest = argv.get(2..).unwrap_or_default();
    match rest.first().map(String::as_str) {
        Some("--") => rest[1..].to_vec(),
        _ => rest.to_vec(),
    }
}

/// Ponto de entrada do binário quando invocado como `tyba _jail -- <args>` —
/// o alvo do shim do rc e do wrapper `claude` no PATH do pane (M6-A). Toda
/// forma de chamar `claude` chega aqui; [`crate::agent::claude_cli::route`]
/// decide o caminho. Nunca trava o terminal: qualquer dúvida no canal cai no
/// binário de verdade, com um aviso no terminal.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn maybe_run_jail_mode() -> Option<i32> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) != Some("_jail") {
        return None;
    }
    Some(run_jail_client(
        Some(&jail_connect_socket_path()),
        &jail_args(&argv),
    ))
}

/// Fora do Linux e do macOS o canal não existe, mas o shim do rc é o mesmo em todo unix
/// e chama `tyba _jail` sem `||` de volta (a shell decide ANTES de lançar,
/// nunca pelo status de saída — ver o comentário do shim). Então `_jail`
/// tem que ser fail-open AQUI também: `exec claude` sem argumento (Q1).
/// Deixar cair em `tyba_lib::run()` abriria a janela do app em vez de rodar
/// o que foi digitado.
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub fn maybe_run_jail_mode() -> Option<i32> {
    use std::os::unix::process::CommandExt;

    if std::env::args().nth(1).as_deref() != Some("_jail") {
        return None;
    }
    let argv: Vec<String> = std::env::args().collect();
    let err = std::process::Command::new("claude")
        .args(jail_args(&argv))
        .exec();
    eprintln!("tyba _jail: exec de claude falhou: {err}");
    Some(126)
}

/// Windows não tem shim (PowerShell não passa por função de shell), então
/// `_jail` nunca é invocado aqui.
#[cfg(windows)]
pub fn maybe_run_jail_mode() -> Option<i32> {
    None
}

/// Marcador que versões antigas punham no env do `claude` cru (M6-A). Ninguém
/// lê mais; só é retirado do env do claude, para não vazar de um shell antigo.
pub const SHIM_ENV: &str = "TYBA_SHIM";

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_jail_client(socket_path: Option<&Path>, args: &[String]) -> i32 {
    use crate::agent::claude_cli::{route, ClaudeRoute};
    use std::os::unix::process::CommandExt;

    // Já dentro de um claude hospedado (o Bash dele, um script, `claude -p`):
    // roda o binário de verdade direto — nunca pede outro hosting, e nunca
    // entra em loop se o wrapper estiver no PATH do agente. Sem jaula para
    // herdar, a sessão aninhada leva o guarda-corpo num `--settings` próprio
    // (`_hook --guard-only`, que não fala com o core: o aninhado não mexe no
    // monitor nem na conversa do pane). Ver `claude_cli::nested_args` para o
    // que fica de fora.
    if std::env::var("TYBA_HOSTED").as_deref() == Ok("1") {
        let args = match std::env::current_exe() {
            Ok(exe) => {
                use crate::agent::hooks_settings::{guard_only_hook_command, guard_only_settings};
                let settings = guard_only_settings(&guard_only_hook_command(&exe));
                crate::agent::claude_cli::nested_args(args, &settings.to_string())
            }
            Err(_) => args.to_vec(),
        };
        return exec_real(&args);
    }
    match route(args) {
        ClaudeRoute::Forbidden(reason) => {
            eprintln!("tyba: {reason}. Dentro do TYBA o claude roda sempre com os hooks do TYBA.");
            return 2;
        }
        ClaudeRoute::Passthrough => return exec_real(args),
        ClaudeRoute::Ungated => {
            eprintln!("tyba: sessão em segundo plano roda fora do pane, sem os hooks do TYBA.");
            return exec_real(args);
        }
        ClaudeRoute::Host => {}
    }

    match resolve_jail_outcome(socket_path, args) {
        JailOutcome::Exec {
            argv,
            env,
            env_remove,
            cwd,
        } => {
            if let Some((program, rest)) = argv.split_first() {
                let mut cmd = std::process::Command::new(program);
                cmd.args(rest);
                cmd.current_dir(&cwd);
                cmd.env_clear();
                cmd.envs(hosted_child_env(std::env::vars_os(), &env_remove, &env));
                let err = cmd.exec();
                eprintln!("tyba: exec do agente hospedado falhou: {err}");
            }
        }
        JailOutcome::Forbidden => {
            eprintln!("tyba: o TYBA recusou estes argumentos — eles desligariam os hooks do TYBA.");
            return 2;
        }
        JailOutcome::RunReal => {}
    }
    // Fail-open final, mas VISÍVEL: o core não hospedou (ou o exec acima
    // falhou), o terminal não pode travar, e o claude sobe cru, com aviso.
    eprintln!("tyba: o TYBA não conseguiu hospedar este claude; ele roda sem os hooks do TYBA.");
    exec_real(args)
}

/// `exec` do `claude` de verdade com os args intactos (argv, nunca shell).
/// Nunca o wrapper do TYBA — ver [`crate::agent::claude_cli::real_claude`].
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn exec_real(args: &[String]) -> i32 {
    use std::os::unix::process::CommandExt;

    let wrapper_dir = std::env::var_os("TYBA_WRAPPER_DIR").map(PathBuf::from);
    let Some(real) = crate::agent::claude_cli::real_claude(
        std::env::var("TYBA_REAL_CLAUDE").ok().as_deref(),
        std::env::var("PATH").ok().as_deref(),
        wrapper_dir.as_deref(),
        is_executable_file,
    ) else {
        eprintln!("tyba _jail: claude não encontrado no PATH");
        return 127;
    };
    let mut cmd = std::process::Command::new(real);
    cmd.args(args);
    // Um marcador herdado de versão antiga não chega ao claude.
    cmd.env_remove(SHIM_ENV);
    let err = cmd.exec();
    eprintln!("tyba _jail: exec de claude falhou: {err}");
    126
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

// Gate consistente com o servidor/cliente que exercitam: sem ele, Windows não
// compila estes testes (`ChannelServer`/`resolve_jail_outcome`/
// `MAX_REQUEST_BYTES` não existem lá). Rodam no Linux e no macOS, com o peer
// cred real de cada um.
#[cfg(test)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
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
        assert_eq!(resolve_jail_outcome(None, &[]), JailOutcome::RunReal);
    }

    /// M3: o cliente só conversa com um socket num diretório do usuário e
    /// fechado para os outros. Um servidor de verdade ali, mas com o diretório
    /// aberto (0755), é recusado sem nem conectar — e o `claude` cai cru.
    #[test]
    fn a_socket_in_a_dir_open_to_others_fails_open_without_connecting() {
        use std::os::unix::fs::PermissionsExt;
        let reached = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sink = reached.clone();
        let (dir, path, server) = bound_server(std::sync::Arc::new(move |_pid, _req| {
            sink.store(true, std::sync::atomic::Ordering::SeqCst);
            ChannelResponse::Refused {
                reason: RefusedReason::NoSession,
            }
        }));
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(resolve_jail_outcome(Some(&path), &[]), JailOutcome::RunReal);
        assert!(!reached.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!socket_dir_is_ours(&path));

        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(socket_dir_is_ours(&path));
        server.shutdown();
    }

    #[test]
    fn an_unreachable_socket_fails_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nada-aqui.sock");
        assert_eq!(resolve_jail_outcome(Some(&path), &[]), JailOutcome::RunReal);
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
                resolve_jail_outcome(Some(&path), &[]),
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
        assert_eq!(resolve_jail_outcome(Some(&path), &[]), JailOutcome::RunReal);
        server.shutdown();
    }

    #[test]
    fn jail_args_take_everything_after_the_double_dash() {
        let argv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(jail_args(&argv(&["tyba", "_jail"])).is_empty());
        assert_eq!(
            jail_args(&argv(&["tyba", "_jail", "--", "-c"])),
            argv(&["-c"])
        );
        // Um `--` do próprio usuário depois do nosso continua lá.
        assert_eq!(
            jail_args(&argv(&["tyba", "_jail", "--", "-p", "--", "x"])),
            argv(&["-p", "--", "x"])
        );
    }

    /// M6-A: os args chegam ao core como argv, e a recusa por args vira
    /// `Forbidden` — o cliente não cai no claude cru.
    #[test]
    fn args_reach_the_core_and_a_forbidden_refusal_never_runs_raw() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let (_dir, path, server) = bound_server(std::sync::Arc::new(move |_pid, req| {
            *sink.lock().unwrap() = req.args.clone();
            ChannelResponse::Refused {
                reason: RefusedReason::ForbiddenArgs,
            }
        }));
        let args: Vec<String> = vec!["-p".into(), "oi \"$(whoami)\"".into()];
        assert_eq!(
            resolve_jail_outcome(Some(&path), &args),
            JailOutcome::Forbidden
        );
        assert_eq!(*seen.lock().unwrap(), args);
        server.shutdown();
    }

    #[test]
    fn a_host_response_with_a_real_plan_reads_argv_env_cwd_and_unlinks_it() {
        let plan_dir = tempfile::tempdir().unwrap();
        let plan_path = plan_dir.path().join("launch.json");
        let plan = HostedPlan {
            argv: vec!["claude".into(), "--settings".into(), "/r/s.json".into()],
            env: vec![
                ("TYBA_HOSTED".into(), "1".into()),
                ("TYBA_JAILED".into(), "0".into()),
            ],
            env_remove: vec!["CLAUDE_CODE_SIMPLE".into()],
            cwd: "/work".into(),
        };
        std::fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();

        let plan_path_str = plan_path.to_string_lossy().into_owned();
        let (_dir, path, server) = bound_server(std::sync::Arc::new(move |_pid, _req| {
            ChannelResponse::Host {
                plan_path: plan_path_str.clone(),
                jailed: false,
            }
        }));

        let outcome = resolve_jail_outcome(Some(&path), &[]);
        assert_eq!(
            outcome,
            JailOutcome::Exec {
                argv: plan.argv.clone(),
                env: plan.env.clone(),
                env_remove: plan.env_remove.clone(),
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

#[cfg(test)]
mod hosted_env_tests {
    use super::*;
    use std::ffi::OsString;

    fn pairs(list: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        list.iter().map(|(k, v)| (k.into(), v.into())).collect()
    }

    /// Agente livre: o env do shell do pane chega ao claude; o TYBA só tira
    /// o que quebraria os hooks dele e acrescenta o socket e os marcadores.
    #[test]
    fn the_pane_env_reaches_the_child_with_tyba_vars_on_top() {
        let pane = pairs(&[
            ("TYBA_TEST_PANE_VAR", "1"),
            ("PATH", "/pane/bin:/usr/bin"),
            ("CLAUDE_CODE_SIMPLE", "1"),
            ("TYBA_SHIM", "fallback"),
            ("TYBA_HOOK_SOCKET", "/velho.sock"),
        ]);
        let set = vec![
            ("TYBA_HOOK_SOCKET".to_string(), "/r/hook.sock".to_string()),
            ("TYBA_HOSTED".to_string(), "1".to_string()),
        ];
        let remove = vec!["CLAUDE_CODE_SIMPLE".to_string(), "TYBA_SHIM".to_string()];
        let env = hosted_child_env(pane, &remove, &set);
        let get = |k: &str| {
            let hits: Vec<_> = env.iter().filter(|(key, _)| key == k).collect();
            assert!(hits.len() <= 1, "{k} duplicada: {env:?}");
            hits.first().map(|(_, v)| v.to_string_lossy().into_owned())
        };
        assert_eq!(get("TYBA_TEST_PANE_VAR").as_deref(), Some("1"));
        assert_eq!(get("PATH").as_deref(), Some("/pane/bin:/usr/bin"));
        assert_eq!(get("TYBA_HOOK_SOCKET").as_deref(), Some("/r/hook.sock"));
        assert_eq!(get("TYBA_HOSTED").as_deref(), Some("1"));
        assert_eq!(get("CLAUDE_CODE_SIMPLE"), None);
        assert_eq!(get("TYBA_SHIM"), None);
    }
}

// Mesmo motivo de `client_tests`: exercita `ChannelServer`/peer cred real de
// ponta a ponta, no Linux e no macOS.
#[cfg(test)]
#[cfg(any(target_os = "linux", target_os = "macos"))]
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
                args: Vec::new(),
                token: None,
                event: None,
                tool: None,
                summary: None,
                notification_type: None,
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

    /// M6-B: o evento do plugin chega ao handler com o token e o resumo, e a
    /// resposta `plugin` (sem decisão) volta ao cliente.
    #[test]
    fn a_plugin_event_reaches_the_handler_with_its_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_in(&dir, "channel.sock");
        let seen = Arc::new(std::sync::Mutex::new(None));
        let sink = seen.clone();
        let server = ChannelServer::bind(
            &path,
            Arc::new(move |_pid, req| {
                *sink.lock().unwrap() = Some((req.op.clone(), req.token.clone(), req.tool.clone()));
                ChannelResponse::Plugin
            }),
        )
        .unwrap();
        let response = connect_and_request(
            &path,
            &ChannelRequest {
                v: CHANNEL_PROTOCOL_VERSION,
                op: PLUGIN_EVENT_OP.into(),
                agent: "claude".into(),
                args: Vec::new(),
                token: Some("tok".into()),
                event: Some("PermissionRequest".into()),
                tool: Some("Bash".into()),
                summary: Some("Bash: ls".into()),
                notification_type: None,
            },
        );
        assert_eq!(response, Some(ChannelResponse::Plugin));
        assert_eq!(
            *seen.lock().unwrap(),
            Some((
                "plugin_event".to_string(),
                Some("tok".to_string()),
                Some("Bash".to_string())
            ))
        );
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
                args: Vec::new(),
                token: None,
                event: None,
                tool: None,
                summary: None,
                notification_type: None,
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
                args: Vec::new(),
                token: None,
                event: None,
                tool: None,
                summary: None,
                notification_type: None,
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

    /// M3: no macOS o endereço mora no temp dir por usuário do sistema
    /// (`confstr`), não no `$TMPDIR` do env de quem chama.
    #[test]
    #[cfg(target_os = "macos")]
    fn macos_socket_lives_in_the_darwin_user_temp_dir() {
        use std::os::unix::fs::MetadataExt;
        let base = darwin_user_temp_dir().expect("confstr devolve o temp dir do usuário");
        assert_eq!(
            resolve_channel_socket_path(),
            base.join("tyba").join(SOCKET_FILE)
        );
        let meta = std::fs::metadata(&base).unwrap();
        assert_eq!(meta.uid(), unsafe { libc::getuid() });
        assert_eq!(meta.mode() & 0o077, 0, "o temp dir do usuário é 0700");
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
    #[cfg(any(target_os = "linux", target_os = "macos"))]
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
            args: Vec::new(),
            token: None,
            event: None,
            tool: None,
            summary: None,
            notification_type: None,
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
            args: Vec::new(),
            token: None,
            event: None,
            tool: None,
            summary: None,
            notification_type: None,
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
