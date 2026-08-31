//! Entrega B — a ponte de navegador (§5 do design).
//!
//! Fecha o "1-clique": o Claude (ou qualquer outra CLI) chama `$BROWSER url`
//! pra abrir o login; `$BROWSER` aponta pro shim que este módulo escreve; o
//! shim fala com o core pelo `hook.sock` (a mesma porta única de entrada da
//! jaula, `hook_ipc`); o core valida e emite um toast acionável — o clique é
//! quem de fato abre o navegador, sempre fora da jaula.
//!
//! Bônus por construção: o shim não é específico do Claude — `gh`/`vercel
//! login` também passam a funcionar, porque é setado pra toda sessão de
//! agente, sem branch por runner. Só o fluxo do Claude está sob a promessa.

use std::path::{Path, PathBuf};
use std::time::Duration;

use portable_pty::CommandBuilder;

pub const EVENT_AGENT_OPEN_URL: &str = "agent://open-url";
const OPEN_URL_HOOK_EVENT: &str = "TybaOpenUrl";
const MAX_URL_LEN: usize = 4096;
const OPEN_URL_TIMEOUT: Duration = Duration::from_secs(5);

/// §1.2 — hosts reais de login hoje: claude.ai (produto), claude.com/
/// www.claude.com (redirects que o dono às vezes vê), platform.claude.com
/// (console de API), console.anthropic.com/anthropic.com (legado/enterprise).
/// Não é allowlist de abertura — é só o que muda a cópia do toast; qualquer
/// host passa pela mesma validação e pelo mesmo clique (§1.2: allowlist
/// quebraria login de MCP/gh/vercel).
pub const KNOWN_LOGIN_HOSTS: [&str; 6] = [
    "claude.ai",
    "claude.com",
    "www.claude.com",
    "platform.claude.com",
    "console.anthropic.com",
    "anthropic.com",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedUrl {
    pub url: String,
    pub host: String,
    pub known_login: bool,
}

/// Core → webview (§1.2): toast ACIONÁVEL — carrega a decisão do dono, ao
/// contrário do `SandboxWarningPayload` de `credentials.rs` (T1), que é aviso
/// sem ação.
#[derive(Clone, serde::Serialize)]
pub struct OpenUrlPayload {
    pub session_id: crate::session::SessionId,
    pub url: String,
    pub host: String,
    pub known_login: bool,
}

/// §1.2 — as cinco regras, puras, sem IO: (1) scheme ∈ {http,https} —
/// `mailto:`/`javascript:`/`file:` não entram; (2) host não vazio, sem
/// `user:pass@`; (3) comprimento ≤ 4096; (4) sem caracteres de controle; (5)
/// `known_login` deriva do host. Roda no core (`handle_event`), nunca no
/// shim — o shim está dentro da jaula, e o agente pode falar com o socket
/// direto.
pub fn validate_open_url(raw: &str) -> Result<ValidatedUrl, String> {
    if raw.len() > MAX_URL_LEN {
        return Err(format!("url maior que {MAX_URL_LEN} caracteres"));
    }
    if raw.chars().any(|c| matches!(c as u32, 0x00..=0x1f | 0x7f)) {
        return Err("url com caractere de controle".to_string());
    }
    let Some((scheme, rest)) = raw.split_once("://") else {
        return Err("url sem esquema http(s)".to_string());
    };
    if scheme != "http" && scheme != "https" {
        return Err(format!("esquema não permitido: {scheme}"));
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.contains('@') {
        return Err("url com credenciais embutidas (user:pass@)".to_string());
    }
    let host = authority.split(':').next().unwrap_or("");
    if host.is_empty() {
        return Err("url sem host".to_string());
    }
    let known_login = KNOWN_LOGIN_HOSTS.contains(&host);
    Ok(ValidatedUrl {
        url: raw.to_string(),
        host: host.to_string(),
        known_login,
    })
}

fn shell_quote(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

/// §5.2 — o shim. `runtime_dir` e `tyba_exe` já entram `--ro-bind` na jaula
/// (bwrap.rs), `/bin/sh` existe no sistema base. Modo 0o500: legível e
/// executável só pelo dono, nem gravável por ele — não há por que reescrever
/// o próprio shim depois de criado.
pub fn write_browser_shim(runtime_dir: &Path, tyba_exe: &Path) -> Result<PathBuf, String> {
    let shim_path = runtime_dir.join("open-url");
    let script = format!(
        "#!/bin/sh\nexec {} _open-url \"$1\"\n",
        shell_quote(tyba_exe)
    );
    std::fs::write(&shim_path, script).map_err(|e| format!("escrita do shim de navegador: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim_path, std::fs::Permissions::from_mode(0o500))
            .map_err(|e| format!("permissão do shim de navegador: {e}"))?;
    }
    Ok(shim_path)
}

/// §5.2 — setado ao lado de `TYBA_HOOK_SOCKET` em `spawn_prepared`, DEPOIS da
/// aplicação do `env_allow` do repo: um repo hostil que liste `BROWSER` em
/// `env_allow` não sequestra nada — este `cmd.env` por cima sempre vence
/// (T3 fecha o resto: `BROWSER` também está na denylist de env, então nem
/// entra por aí).
pub fn set_browser_env(cmd: &mut CommandBuilder, shim_path: &Path) {
    cmd.env("BROWSER", shim_path);
}

/// §5.2 — padrão de `hook_ipc::maybe_run_hook_mode`: registrado em
/// `main.rs` (o design chama de "lib.rs" — na prática é onde
/// `maybe_run_hook_mode`/`maybe_run_seccomp_exec` já registram, e é lá que
/// este também entra, pelo mesmo motivo).
pub fn maybe_run_open_url_mode() -> Option<i32> {
    if std::env::args().nth(1).as_deref() != Some("_open-url") {
        return None;
    }
    let url = std::env::args().nth(2).unwrap_or_default();
    let socket = std::env::var_os("TYBA_HOOK_SOCKET").map(PathBuf::from);
    Some(run_open_url_mode(&url, socket.as_deref()))
}

/// Não reusa `run_client` (§5.2): aquele sempre devolve exit 0 — fail-closed
/// pro gate de tool use, que não pode travar um `PreToolUse`. Aqui o exit
/// code É a informação: ack→0, deny/timeout→1, e exit≠0 é o que faz o Claude
/// cair no bloco "Browser didn't open? Use the url below" (V4) — a ponte
/// falha para o estado de hoje, nunca pra um pior. Prazo de 5s aplicado por
/// fora (canal com `recv_timeout`), não só pelo retry interno do
/// `hook_ipc::request` — cobre tanto a conexão quanto a espera da resposta.
fn run_open_url_mode(url: &str, socket: Option<&Path>) -> i32 {
    let Some(socket) = socket.map(Path::to_path_buf) else {
        return 1;
    };
    let event = serde_json::json!({
        "hook_event_name": OPEN_URL_HOOK_EVENT,
        "url": url,
    });
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(crate::hook_ipc::request(&socket, &event));
    });
    match rx.recv_timeout(OPEN_URL_TIMEOUT) {
        Ok(Some(response)) if response.action == "ack" => 0,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_real_claude_authorize_url_and_marks_known_login() {
        let raw = "https://claude.ai/oauth/authorize?client_id=abc&redirect_uri=http%3A%2F%2Flocalhost%3A4321%2Fcallback&state=xyz";
        let v = validate_open_url(raw).unwrap();
        assert_eq!(v.url, raw);
        assert_eq!(v.host, "claude.ai");
        assert!(v.known_login);
    }

    #[test]
    fn github_login_is_accepted_but_not_known_login() {
        let v = validate_open_url("https://github.com/login/oauth/authorize?client_id=x").unwrap();
        assert_eq!(v.host, "github.com");
        assert!(!v.known_login);
    }

    #[test]
    fn platform_claude_com_is_known_login() {
        let v = validate_open_url("https://platform.claude.com/settings").unwrap();
        assert!(v.known_login);
    }

    /// Item 24 do contrato de cobertura, review r1: só claude.ai e
    /// platform.claude.com estavam pinados -- claude.com (redirect que o
    /// dono às vezes vê, §1.2) fica sem teste dedicado, e sair da lista sem
    /// nenhum teste reprovar é exatamente o risco que motivou o fix de
    /// phishing do toast (item MAJOR).
    #[test]
    fn claude_com_is_known_login() {
        let v = validate_open_url("https://claude.com/login").unwrap();
        assert!(v.known_login);
    }

    #[test]
    fn rejects_file_scheme() {
        assert!(validate_open_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn rejects_mailto_scheme() {
        assert!(validate_open_url("mailto:x@y.com").is_err());
    }

    #[test]
    fn rejects_javascript_scheme() {
        assert!(validate_open_url("javascript:alert(1)").is_err());
        assert!(validate_open_url("javascript://alert(1)").is_err());
    }

    #[test]
    fn rejects_empty_host() {
        assert!(validate_open_url("http:///path").is_err());
    }

    #[test]
    fn rejects_embedded_credentials() {
        assert!(validate_open_url("https://user:pass@evil.example/steal").is_err());
    }

    #[test]
    fn rejects_url_over_4096_chars() {
        let long = format!("https://claude.ai/{}", "a".repeat(4096));
        assert!(validate_open_url(&long).is_err());
    }

    #[test]
    fn accepts_url_at_exactly_4096_chars() {
        let padding = "a".repeat(4096 - "https://claude.ai/".len());
        let raw = format!("https://claude.ai/{padding}");
        assert_eq!(raw.len(), 4096);
        assert!(validate_open_url(&raw).is_ok());
    }

    #[test]
    fn rejects_control_characters() {
        assert!(validate_open_url("https://claude.ai/\x00x").is_err());
        assert!(validate_open_url("https://claude.ai/\x1bx").is_err());
    }

    #[test]
    fn preserves_ampersand_equals_and_percent_encoding_untouched() {
        let raw = "https://claude.ai/authorize?a=1&b=2%20x&c=3";
        let v = validate_open_url(raw).unwrap();
        assert_eq!(
            v.url, raw,
            "a url precisa chegar intacta, sem remontar nada"
        );
    }

    #[test]
    fn shim_script_is_mode_0500_and_execs_tyba_with_the_url_argument() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = PathBuf::from("/opt/tyba/tyba");
        let shim = write_browser_shim(tmp.path(), &exe).unwrap();
        assert_eq!(shim, tmp.path().join("open-url"));
        let body = std::fs::read_to_string(&shim).unwrap();
        assert!(body.starts_with("#!/bin/sh\n"));
        assert!(body.contains("exec '/opt/tyba/tyba' _open-url \"$1\""));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&shim).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o500);
        }
    }

    #[test]
    fn shim_script_quotes_a_path_with_spaces_and_single_quotes() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = PathBuf::from("/Apps/O'Brien App/tyba");
        let shim = write_browser_shim(tmp.path(), &exe).unwrap();
        let body = std::fs::read_to_string(&shim).unwrap();
        assert!(body.contains("exec '/Apps/O'\\''Brien App/tyba' _open-url \"$1\""));
    }

    #[test]
    fn browser_env_wins_over_whatever_env_allow_brought() {
        let mut cmd = CommandBuilder::new("claude");
        cmd.env("BROWSER", "attacker-controlled");
        set_browser_env(&mut cmd, Path::new("/rt/open-url"));
        assert_eq!(
            cmd.get_env("BROWSER"),
            Some(std::ffi::OsStr::new("/rt/open-url"))
        );
    }

    #[test]
    fn open_url_mode_returns_0_on_ack() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("hook.sock");
        let server = crate::hook_ipc::HookServer::bind(
            &socket,
            std::sync::Arc::new(|_e: crate::hook_ipc::HookEvent| crate::hook_ipc::HookAction::Ack),
        )
        .unwrap();

        let code = run_open_url_mode("https://claude.ai/oauth/authorize", Some(&socket));
        assert_eq!(code, 0);

        server.shutdown();
    }

    #[test]
    fn open_url_mode_returns_1_on_deny() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("hook.sock");
        let server = crate::hook_ipc::HookServer::bind(
            &socket,
            std::sync::Arc::new(|_e: crate::hook_ipc::HookEvent| {
                crate::hook_ipc::HookAction::Deny {
                    reason: "host desconhecido".into(),
                }
            }),
        )
        .unwrap();

        let code = run_open_url_mode("https://claude.ai/oauth/authorize", Some(&socket));
        assert_eq!(code, 1);

        server.shutdown();
    }

    #[test]
    fn open_url_mode_returns_1_immediately_when_socket_is_absent() {
        let start = std::time::Instant::now();
        let code = run_open_url_mode("https://claude.ai/oauth/authorize", None);
        assert_eq!(code, 1);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "sem socket, o retorno precisa ser imediato — não há o que esperar"
        );
    }

    /// A URL chega íntegra no envelope que atravessa o socket — não só na
    /// função de validação isolada (teste acima), mas no caminho real do
    /// shim até o `hook.sock`.
    #[test]
    fn open_url_mode_sends_the_envelope_with_the_url_intact() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("hook.sock");
        let seen: std::sync::Arc<std::sync::Mutex<Option<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let sink = seen.clone();
        let server = crate::hook_ipc::HookServer::bind(
            &socket,
            std::sync::Arc::new(move |e: crate::hook_ipc::HookEvent| {
                *sink.lock().unwrap() = e.raw.get("url").and_then(|u| u.as_str()).map(String::from);
                crate::hook_ipc::HookAction::Ack
            }),
        )
        .unwrap();

        let raw = "https://claude.ai/authorize?a=1&b=2%20x&state=abc%3D";
        let code = run_open_url_mode(raw, Some(&socket));
        assert_eq!(code, 0);
        assert_eq!(seen.lock().unwrap().as_deref(), Some(raw));

        server.shutdown();
    }
}
