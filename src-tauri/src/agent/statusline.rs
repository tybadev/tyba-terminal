//! `tyba _statusline`: o comando de `statusLine` das sessões de Claude que o
//! TYBA abre.
//!
//! O Claude roda o comando a cada mensagem do assistente com um JSON no stdin
//! e mostra o que ele imprimir. Daqui saem duas coisas:
//!
//! 1. os números de uso, para o core, pelo mesmo socket de hooks da sessão
//!    (`TYBA_HOOK_SOCKET`) — ver [`super::usage`];
//! 2. a linha do rodapé. O `--settings` da sessão tem precedência sobre o
//!    `statusLine` do usuário, então tomar o lugar dele sem devolvê-lo seria
//!    apagar a statusline de quem já tinha uma. O spawn lê a statusline
//!    efetiva do usuário ([`read_user_statusline`]) e a passa em hex no
//!    argumento; aqui ela roda com o MESMO stdin e o stdout dela é o que sai.
//!    Sem statusline do usuário, sai a linha mínima do TYBA.
//!
//! Nunca trava nem derruba o Claude: prazo curto para tudo, falha vira linha
//! vazia, exit sempre 0.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::usage::{minimal_line, parse_statusline, STATUSLINE_HOOK_EVENT};

pub const STATUSLINE_MODE: &str = "_statusline";
const USER_FLAG: &str = "--user-hex";

/// Prazo da statusline do usuário. O Claude cancela o comando em voo quando
/// chega atualização nova; isto é só o teto para um script que trava.
const USER_TIMEOUT: Duration = Duration::from_secs(5);
/// Prazo do envio ao core. Perder um ponto de uso não custa nada — o próximo
/// chega na próxima mensagem.
const CORE_TIMEOUT: Duration = Duration::from_millis(800);
const MAX_STDIN: usize = 1 << 20;
const MAX_STDOUT: usize = 64 * 1024;

/// A statusline que o usuário tinha configurado.
#[derive(Debug, Clone, PartialEq)]
pub struct UserStatusLine {
    pub command: String,
    pub padding: Option<u64>,
    pub refresh_interval: Option<u64>,
}

fn statusline_of(value: &Value) -> Option<UserStatusLine> {
    let sl = value.get("statusLine")?;
    if sl.get("type").and_then(Value::as_str) != Some("command") {
        return None;
    }
    let command = sl
        .get("command")
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    // Um `_statusline` do TYBA no arquivo do usuário rodaria a si mesmo.
    if command.is_empty() || command.contains(STATUSLINE_MODE) {
        return None;
    }
    Some(UserStatusLine {
        command,
        padding: sl.get("padding").and_then(Value::as_u64),
        refresh_interval: sl.get("refreshInterval").and_then(Value::as_u64),
    })
}

/// A statusline efetiva do usuário, na precedência do Claude: local do
/// projeto, projeto, usuário (`$HOME/.claude/settings.json`). `project_dirs`
/// vem em ordem de prioridade (o worktree antes da raiz do repo principal,
/// que é onde o `settings.local.json` mora). O primeiro arquivo legível com
/// `statusLine` de comando ganha. Managed settings ficam de fora: lá o
/// `statusLine` vence o `--settings` e o nosso nem roda.
pub fn read_user_statusline(home: Option<&Path>, project_dirs: &[&Path]) -> Option<UserStatusLine> {
    let mut files: Vec<PathBuf> = Vec::new();
    for name in ["settings.local.json", "settings.json"] {
        for dir in project_dirs {
            files.push(dir.join(".claude").join(name));
        }
    }
    if let Some(home) = home {
        files.push(home.join(".claude").join("settings.json"));
    }
    files.iter().find_map(|path| {
        let raw = std::fs::read_to_string(path).ok()?;
        let value: Value = serde_json::from_str(&raw).ok()?;
        statusline_of(&value)
    })
}

fn hex_encode(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Option<String> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

/// O objeto `statusLine` do `--settings` da sessão. O comando do usuário vai
/// em hex: atravessa o shell do Claude sem nenhuma regra de aspas. `padding`
/// e `refreshInterval` do usuário são copiados — senão a linha dele mudaria
/// de lugar ou pararia de atualizar.
pub fn statusline_settings(exe: &Path, user: Option<&UserStatusLine>) -> Value {
    let raw = exe.to_string_lossy();
    let quoted = format!("'{}'", raw.replace('\'', "'\\''"));
    let mut command = format!("{quoted} {STATUSLINE_MODE}");
    if let Some(user) = user {
        command.push_str(&format!(" {USER_FLAG} {}", hex_encode(&user.command)));
    }
    let mut out = json!({ "type": "command", "command": command });
    if let Some(p) = user.and_then(|u| u.padding) {
        out["padding"] = json!(p);
    }
    if let Some(r) = user.and_then(|u| u.refresh_interval) {
        out["refreshInterval"] = json!(r);
    }
    out
}

/// O comando do usuário decodificado dos argumentos (`--user-hex <hex>`).
fn user_command_from_args(args: &[String]) -> Option<String> {
    let at = args.iter().position(|a| a == USER_FLAG)?;
    hex_decode(args.get(at + 1)?)
}

pub fn maybe_run_statusline_mode() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(STATUSLINE_MODE) {
        return None;
    }
    let socket = std::env::var_os("TYBA_HOOK_SOCKET").map(PathBuf::from);
    let mut input = Vec::new();
    let _ = std::io::stdin()
        .lock()
        .take(MAX_STDIN as u64)
        .read_to_end(&mut input);
    let user = user_command_from_args(&args);
    let line = run_statusline(&input, user.as_deref(), socket, USER_TIMEOUT);
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(&line);
    let _ = stdout.flush();
    Some(0)
}

/// O miolo, testável: manda o uso ao core em paralelo e devolve o que vai
/// para o stdout.
pub fn run_statusline(
    input: &[u8],
    user_command: Option<&str>,
    socket: Option<PathBuf>,
    user_timeout: Duration,
) -> Vec<u8> {
    let usage = parse_statusline(input);
    let sent = match (&usage, socket) {
        (Some(usage), Some(socket)) => {
            let event = json!({ "hook_event_name": STATUSLINE_HOOK_EVENT, "usage": usage });
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::hook_ipc::request(&socket, &event));
            });
            Some(rx)
        }
        _ => None,
    };
    let started = Instant::now();
    let line = match user_command {
        Some(cmd) => run_user_command(cmd, input, user_timeout).unwrap_or_default(),
        None => usage
            .as_ref()
            .map(minimal_line)
            .unwrap_or_default()
            .into_bytes(),
    };
    if let Some(rx) = sent {
        let _ = rx.recv_timeout(CORE_TIMEOUT.saturating_sub(started.elapsed()));
    }
    line
}

#[cfg(unix)]
fn shell(cmd: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(cmd);
    c
}

#[cfg(windows)]
fn shell(cmd: &str) -> Command {
    let mut c = Command::new("cmd");
    c.arg("/C").arg(cmd);
    c
}

/// Roda a statusline do usuário com o mesmo stdin. `None` — e linha vazia —
/// quando ela não sobe, sai com erro ou passa do prazo: é o que o Claude
/// mostraria nesses casos.
pub fn run_user_command(cmd: &str, input: &[u8], timeout: Duration) -> Option<Vec<u8>> {
    let mut child = shell(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let input = input.to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = (&mut stdout).take(MAX_STDOUT as u64).read_to_end(&mut out);
        let _ = tx.send(out);
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                return rx.recv_timeout(left.max(Duration::from_millis(100))).ok();
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INPUT: &str = r#"{"model":{"id":"claude-opus-5-5","display_name":"Opus 5.5"},"context_window":{"used_percentage":13}}"#;

    fn write(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn hex_ida_e_volta() {
        let cmd = "bash -c 'echo \"$x\" | jq .model' ; ação";
        assert_eq!(hex_decode(&hex_encode(cmd)).as_deref(), Some(cmd));
        assert_eq!(hex_decode("abc"), None);
        assert_eq!(hex_decode("zz"), None);
    }

    #[test]
    fn settings_sem_statusline_do_usuario() {
        let v = statusline_settings(Path::new("/Apps/My App/tyba"), None);
        assert_eq!(v["type"], "command");
        assert_eq!(v["command"], "'/Apps/My App/tyba' _statusline");
        assert!(v.get("padding").is_none());
    }

    #[test]
    fn settings_carrega_o_comando_e_o_layout_do_usuario() {
        let user = UserStatusLine {
            command: "~/.claude/sl.sh --x 'y'".into(),
            padding: Some(2),
            refresh_interval: Some(5),
        };
        let v = statusline_settings(Path::new("/usr/bin/tyba"), Some(&user));
        let cmd = v["command"].as_str().unwrap();
        assert!(cmd.starts_with("'/usr/bin/tyba' _statusline --user-hex "));
        let args: Vec<String> = cmd.split(' ').map(str::to_string).collect();
        assert_eq!(
            user_command_from_args(&args).as_deref(),
            Some(user.command.as_str())
        );
        assert_eq!(v["padding"], 2);
        assert_eq!(v["refreshInterval"], 5);
    }

    #[test]
    fn precedencia_local_projeto_usuario() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let wt = tmp.path().join("wt");
        let repo = tmp.path().join("repo");
        let sl = |c: &str| format!(r#"{{"statusLine":{{"type":"command","command":"{c}"}}}}"#);
        write(&home, ".claude/settings.json", &sl("user"));
        assert_eq!(
            read_user_statusline(Some(&home), &[&wt, &repo])
                .unwrap()
                .command,
            "user"
        );
        write(&wt, ".claude/settings.json", &sl("project"));
        assert_eq!(
            read_user_statusline(Some(&home), &[&wt, &repo])
                .unwrap()
                .command,
            "project"
        );
        write(&repo, ".claude/settings.local.json", &sl("local"));
        assert_eq!(
            read_user_statusline(Some(&home), &[&wt, &repo])
                .unwrap()
                .command,
            "local"
        );
    }

    #[test]
    fn ignora_statusline_invalida_ou_do_proprio_tyba() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        write(
            home,
            ".claude/settings.json",
            r#"{"statusLine":{"type":"command","command":"'/x/tyba' _statusline"}}"#,
        );
        assert_eq!(read_user_statusline(Some(home), &[]), None);
        write(
            home,
            ".claude/settings.json",
            r#"{"statusLine":{"type":"other","command":"x"}}"#,
        );
        assert_eq!(read_user_statusline(Some(home), &[]), None);
        write(home, ".claude/settings.json", "{ quebrado");
        assert_eq!(read_user_statusline(Some(home), &[]), None);
        assert_eq!(read_user_statusline(None, &[]), None);
    }

    #[test]
    fn sem_statusline_do_usuario_sai_a_linha_minima() {
        let out = run_statusline(INPUT.as_bytes(), None, None, Duration::from_secs(1));
        assert_eq!(String::from_utf8(out).unwrap(), "Opus 5.5 · ctx 13%");
    }

    #[test]
    fn entrada_lixo_sem_usuario_imprime_nada() {
        let out = run_statusline(b"garbage", None, None, Duration::from_secs(1));
        assert!(out.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn statusline_do_usuario_recebe_o_mesmo_stdin_e_vira_a_saida() {
        // Comando falso: devolve o stdin com prefixo — prova que ele recebeu
        // o JSON inteiro, não o que o TYBA extraiu.
        let out = run_statusline(
            INPUT.as_bytes(),
            Some("printf 'meu: '; cat"),
            None,
            Duration::from_secs(5),
        );
        assert_eq!(String::from_utf8(out).unwrap(), format!("meu: {INPUT}"));
    }

    #[cfg(unix)]
    #[test]
    fn statusline_do_usuario_que_falha_ou_trava_imprime_nada() {
        let failed = run_statusline(
            INPUT.as_bytes(),
            Some("echo oi; exit 3"),
            None,
            Duration::from_secs(5),
        );
        assert!(failed.is_empty());
        let started = Instant::now();
        let hung = run_statusline(
            INPUT.as_bytes(),
            Some("sleep 30"),
            None,
            Duration::from_millis(200),
        );
        assert!(hung.is_empty());
        assert!(started.elapsed() < Duration::from_secs(5));
        let missing = run_statusline(
            INPUT.as_bytes(),
            Some("/nao/existe/xyz"),
            None,
            Duration::from_secs(5),
        );
        assert!(missing.is_empty());
    }

    /// Socket que não existe não trava: o envio tem prazo e a linha sai.
    #[cfg(unix)]
    #[test]
    fn socket_ausente_nao_trava_a_linha() {
        let tmp = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let out = run_statusline(
            INPUT.as_bytes(),
            None,
            Some(tmp.path().join("nada.sock")),
            Duration::from_secs(1),
        );
        assert_eq!(String::from_utf8(out).unwrap(), "Opus 5.5 · ctx 13%");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    /// Ponta a ponta pelo socket de hooks: chega o evento sintético com os
    /// números — e nada de `cwd`, `transcript_path` ou `session_id`.
    #[cfg(unix)]
    #[test]
    fn manda_ao_core_so_os_numeros() {
        use crate::hook_ipc::{HookAction, HookServer};
        use std::sync::{Arc, Mutex};

        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("h.sock");
        let seen = Arc::new(Mutex::new(None));
        let sink = seen.clone();
        let _server = HookServer::bind(
            &socket,
            Arc::new(move |event| {
                *sink.lock().unwrap() = Some((event.hook_event_name.clone(), event.raw.clone()));
                HookAction::Ack
            }),
        )
        .unwrap();
        let input = r#"{"cwd":"/segredo","session_id":"s-1","transcript_path":"/segredo/t.jsonl","model":{"display_name":"Opus 5.5"},"cost":{"total_cost_usd":0.42}}"#;
        let out = run_statusline(input.as_bytes(), None, Some(socket), Duration::from_secs(1));
        assert_eq!(String::from_utf8(out).unwrap(), "Opus 5.5");
        let (name, raw) = seen.lock().unwrap().clone().expect("core não recebeu");
        assert_eq!(name, STATUSLINE_HOOK_EVENT);
        assert_eq!(raw["usage"]["cost_usd"], 0.42);
        let text = raw.to_string();
        assert!(!text.contains("segredo") && !text.contains("s-1"), "{text}");
    }
}
