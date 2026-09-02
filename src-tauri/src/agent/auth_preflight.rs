//! Entrega C, mecanismo 1 — PREFLIGHT: `claude auth status --json` rodado no
//! host, fora da jaula, antes do spawn da sessão (§ do design). Só produz
//! `AuthAlertKind::NotLoggedIn` — os outros kinds (crédito, token expirado,
//! chave inválida) são erros de RUNTIME, que só aparecem quando o agente de
//! fato tenta falar com a API (ver `super::auth_watch`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Runtime};

use super::auth_alert::{AuthAlertKind, AuthAlertPayload, AuthPhase, EVENT_AGENT_AUTH_ALERT};
use crate::session::SessionId;

/// ~3s (§ do design): tempo suficiente pro `claude` ler `.credentials.json`
/// do disco e responder, curto o bastante pra não seguntar o dono se o
/// binário travar por qualquer motivo — o preflight roda numa thread à
/// parte (`spawn_preflight`), então nem este teto atrasa o spawn da sessão.
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(3);

/// C2 do design: roda `claude auth status --json` de verdade, com o env e o
/// cwd exatos que o chamador passou. Mata o processo e devolve `None`
/// (ambíguo) se estourar `timeout` — nunca trava esperando (P5).
///
/// `env_clear()` + `envs()`, não `env()` em cima do herdado: o filho só pode
/// ver o que está em `env` — nada do processo do TYBA vaza (P4), o mesmo
/// raciocínio do env filtrado que `AgentRunner::build_command` recebe.
pub(crate) fn run_status_json(
    binary: &Path,
    env: &HashMap<String, String>,
    cwd: &Path,
    timeout: Duration,
) -> Option<(String, Option<i32>)> {
    let mut child = Command::new(binary)
        .arg("auth")
        .arg("status")
        .arg("--json")
        .env_clear()
        .envs(env)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => return None,
        }
    }

    let out = child.wait_with_output().ok()?;
    Some((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code(),
    ))
}

/// Fio de produção, chamado de `session::spawn_prepared` no mesmo
/// ponto/condição do `credentials::emit_warnings` da Entrega B, mas
/// cross-platform e assíncrono: TUDO roda dentro do `std::thread::spawn`, e a
/// função em si devolve na hora (P3) — o chamador nunca espera nada daqui.
///
/// `binary` chega já resolvido (via `crate::agent::resolved_binary`) em vez
/// de esta função chamá-lo: `resolved_binary` depende do cache global de
/// `shell_path::agent_path()` (resolvido perguntando ao shell de login), que
/// não dá pra isolar por teste sem risco de corrida entre testes rodando em
/// paralelo -- receber `Option<PathBuf>` pronto deixa `spawn_preflight`
/// testável com um binário fake sem tocar nesse cache. `None` é "binário
/// ausente" (P5): silêncio, nem tenta spawnar.
pub(crate) fn spawn_preflight<R: Runtime>(
    app: AppHandle<R>,
    session_id: SessionId,
    binary: Option<PathBuf>,
    env: HashMap<String, String>,
    cwd: PathBuf,
) {
    std::thread::spawn(move || {
        let Some(binary) = binary else {
            return;
        };
        let Some((stdout, exit)) = run_status_json(&binary, &env, &cwd, PREFLIGHT_TIMEOUT) else {
            return;
        };
        if classify_status_json(&stdout, exit) == Some(AuthAlertKind::NotLoggedIn) {
            let _ = app.emit(
                EVENT_AGENT_AUTH_ALERT,
                AuthAlertPayload {
                    session_id,
                    phase: AuthPhase::Preflight,
                    kind: AuthAlertKind::NotLoggedIn,
                },
            );
        }
    });
}

/// Peça pura (P6): interpreta o `stdout`/exit code de
/// `claude auth status --json`. `exit` é `Option<i32>` porque um processo
/// morto por timeout (`kill`) não tem exit code em Unix — isso já cai no
/// braço ambíguo abaixo, junto de qualquer exit fora de `{0, 1}` (os dois
/// únicos valores medidos no binário real).
///
/// Nunca produz nada além de `NotLoggedIn`: os demais `AuthAlertKind` são
/// erros de runtime, que só existem depois que o agente tenta falar com a
/// API — o preflight não fala com nada, só lê o estado local.
pub(crate) fn classify_status_json(stdout: &str, exit: Option<i32>) -> Option<AuthAlertKind> {
    if !matches!(exit, Some(0) | Some(1)) {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    let logged_in = value.get("loggedIn")?.as_bool()?;
    (!logged_in).then_some(AuthAlertKind::NotLoggedIn)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    #[cfg(unix)]
    use tauri::Listener;

    use super::*;

    /// Escreve um script shell executável — o "binário" fake do teste. Vira
    /// arquivo dentro do `TempDir` recebido, então o chamador controla o
    /// ciclo de vida (o script não pode sumir antes do `Command` rodar).
    ///
    /// `#[cfg(unix)]`: todo chamador é um teste `#[cfg(unix)]` (script shell
    /// não faz sentido de exe no Windows) — sem o gate aqui, `cargo clippy
    /// --target x86_64-pc-windows-gnu` reprova com `dead_code` (mesmo achado
    /// documentado em `credentials.rs` para `unclassified_claude_children`).
    #[cfg(unix)]
    fn fake_binary(dir: &std::path::Path, name: &str, script: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    /// P4: o env que chega ao processo filho é EXATAMENTE o que foi passado
    /// -- nem mais (o env do processo pai do teste, que sempre tem `HOME`,
    /// não pode vazar) nem menos (`CLAUDE_CONFIG_DIR` explícito precisa
    /// chegar). Prova o `env_clear()` + `envs()`, não um `assert` sobre o
    /// argv -- é o mesmo raciocínio do env filtrado que `build_command`
    /// recebe (§ do design).
    #[test]
    #[cfg(unix)]
    fn run_status_json_uses_only_the_given_env_never_the_process_env() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(
            dir.path(),
            "claude",
            "#!/bin/sh\nprintf '{\"loggedIn\": false, \"cfg\": \"%s\", \"home_leaked\": \"%s\"}' \"$CLAUDE_CONFIG_DIR\" \"${HOME:-no}\"\n",
        );
        // Pré-condição: o processo do teste TEM HOME -- se o filho também
        // visse, `home_leaked` sairia diferente de "no" mesmo sem vazamento
        // por acaso (variável ausente vira string vazia, não "no").
        assert!(std::env::var("HOME").is_ok());

        let mut env = HashMap::new();
        env.insert("CLAUDE_CONFIG_DIR".to_string(), "/tmp/xyz".to_string());

        let (stdout, exit) =
            run_status_json(&bin, &env, dir.path(), Duration::from_secs(3)).unwrap();
        assert_eq!(exit, Some(0));
        assert!(stdout.contains(r#""cfg": "/tmp/xyz""#), "{stdout}");
        assert!(stdout.contains(r#""home_leaked": "no""#), "{stdout}");
    }

    /// P5: o processo estoura o teto -- `run_status_json` mata e devolve
    /// `None` (ambíguo), nunca trava esperando.
    #[test]
    #[cfg(unix)]
    fn run_status_json_returns_none_on_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(dir.path(), "claude", "#!/bin/sh\nsleep 2\n");
        let started = std::time::Instant::now();
        let result = run_status_json(
            &bin,
            &HashMap::new(),
            dir.path(),
            Duration::from_millis(100),
        );
        assert!(result.is_none());
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "o teto de 100ms não segurou -- levou {:?}",
            started.elapsed()
        );
    }

    /// P3: `spawn_preflight` devolve o controle na hora, mesmo que o
    /// "binário" demore mais que o teto inteiro -- ele nunca é esperado
    /// nesta thread. É a prova de que o preflight não atrasa a subida da
    /// sessão: o trabalho de verdade corre numa thread à parte.
    #[test]
    #[cfg(unix)]
    fn spawn_preflight_returns_immediately_even_when_the_binary_hangs() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(dir.path(), "claude", "#!/bin/sh\nsleep 5\n");
        let app = tauri::test::mock_app();

        let started = std::time::Instant::now();
        spawn_preflight(
            app.handle().clone(),
            crate::session::SessionId::new_v4(),
            Some(bin),
            HashMap::new(),
            dir.path().to_path_buf(),
        );
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "spawn_preflight bloqueou o chamador: {:?}",
            started.elapsed()
        );
    }

    /// P5: binário ausente (`None`, o que `resolved_binary` devolve quando
    /// `claude` não está no PATH) -- silêncio, sem tentar spawnar nada.
    #[test]
    fn spawn_preflight_with_no_binary_does_nothing_and_returns_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let app = tauri::test::mock_app();
        let started = std::time::Instant::now();
        spawn_preflight(
            app.handle().clone(),
            crate::session::SessionId::new_v4(),
            None,
            HashMap::new(),
            dir.path().to_path_buf(),
        );
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    /// P2 fim-a-fim: com um binário fake que responde exatamente como o
    /// `claude` real sem credencial, o evento `agent://auth-alert` sai com
    /// `Preflight` + `NotLoggedIn`.
    #[test]
    #[cfg(unix)]
    fn spawn_preflight_emits_not_logged_in_when_the_binary_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(
            dir.path(),
            "claude",
            "#!/bin/sh\nprintf '{\"loggedIn\": false, \"authMethod\": \"none\"}'\nexit 1\n",
        );
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let session_id = crate::session::SessionId::new_v4();

        let received: std::sync::Arc<
            parking_lot::Mutex<Vec<super::super::auth_alert::AuthAlertPayload>>,
        > = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&received);
        handle.listen(
            super::super::auth_alert::EVENT_AGENT_AUTH_ALERT,
            move |event| {
                if let Ok(payload) = serde_json::from_str(event.payload()) {
                    sink.lock().push(payload);
                }
            },
        );

        spawn_preflight(
            handle,
            session_id,
            Some(bin),
            HashMap::new(),
            dir.path().to_path_buf(),
        );

        // Teto generoso (bem além do `PREFLIGHT_TIMEOUT` de produção): a
        // suíte inteira roda em paralelo sob `cargo test`, e este teste
        // spawna um processo real -- sob contenção pesada de CPU/IO no
        // runner de CI, 3s (o teto do PRÓPRIO preflight) já se mostrou
        // curto demais só pela fila de agendamento das threads, não por
        // lentidão da lógica em si.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while received.lock().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let seen = received.lock().clone();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert_eq!(seen[0].session_id, session_id);
        assert!(matches!(
            seen[0].phase,
            super::super::auth_alert::AuthPhase::Preflight
        ));
        assert!(matches!(
            seen[0].kind,
            super::super::auth_alert::AuthAlertKind::NotLoggedIn
        ));
    }

    /// P1/P6: `loggedIn:true` — logado de verdade OU chave falsa (que o
    /// binário real responde com `loggedIn:true, authMethod:"api_key"`
    /// mesmo offline, por design medido) — nunca produz alerta de preflight.
    /// O preflight afere PRESENÇA de credencial, nunca VALIDADE.
    #[test]
    fn logged_in_true_produces_nothing() {
        assert_eq!(
            classify_status_json(r#"{"loggedIn":true,"authMethod":"claude.ai"}"#, Some(0)),
            None
        );
    }

    /// P2: sem credencial nenhuma — o binário real devolve exit 1 e
    /// `{"loggedIn":false,"authMethod":"none"}`.
    #[test]
    fn logged_in_false_produces_not_logged_in() {
        assert_eq!(
            classify_status_json(r#"{"loggedIn":false,"authMethod":"none"}"#, Some(1)),
            Some(AuthAlertKind::NotLoggedIn)
        );
    }

    /// P5/P6: JSON malformado (saída inesperada, banner, truncado por
    /// timeout no meio) nunca vira alerta — ambiguidade é silêncio, nunca
    /// acusação.
    #[test]
    fn malformed_json_is_ambiguous() {
        assert_eq!(classify_status_json("não é json", Some(0)), None);
        assert_eq!(classify_status_json("", Some(0)), None);
        assert_eq!(classify_status_json(r#"{"loggedIn":"sim"}"#, Some(0)), None);
    }

    /// P5/P6: JSON válido mas sem o campo `loggedIn` — versão futura do
    /// binário que mudou o schema não pode virar falso positivo.
    #[test]
    fn json_without_logged_in_field_is_ambiguous() {
        assert_eq!(
            classify_status_json(r#"{"authMethod":"none"}"#, Some(1)),
            None
        );
    }

    /// P5: exit fora de `{0, 1}` — os dois únicos valores medidos no binário
    /// real — é ambíguo mesmo que o corpo pareça válido.
    #[test]
    fn exit_code_outside_the_measured_pair_is_ambiguous() {
        assert_eq!(classify_status_json(r#"{"loggedIn":false}"#, Some(2)), None);
    }

    /// P5: processo morto por timeout (`kill`) não tem exit code em Unix —
    /// `None` aqui é o mesmo braço ambíguo do exit fora do par medido.
    #[test]
    fn no_exit_code_at_all_is_ambiguous() {
        assert_eq!(classify_status_json(r#"{"loggedIn":false}"#, None), None);
    }
}
