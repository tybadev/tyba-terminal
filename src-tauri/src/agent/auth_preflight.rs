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
    let mut command = Command::new(binary);
    command
        .arg("auth")
        .arg("status")
        .arg("--json")
        .env_clear()
        .envs(env)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Defesa em profundidade (review de segurança round 2, MINOR): o filho
    // vira líder do PRÓPRIO grupo (pgid == pid dele), decidido ANTES do
    // `exec` — não é uma corrida de `setpgid` chamado pelo pai depois do
    // `fork`, que poderia perder pro filho já ter saído. Sem isto, um neto
    // que o `claude auth status` chegasse a subir (por ex. um helper
    // interno) sobreviveria ao timeout como órfão vivo — contra o espírito
    // da invariante #9 (kill mata o GRUPO, não só o pai). Barato aqui
    // porque o probe é o único filho direto que este `Command` sobe; não é
    // a mesma máquina do `kill_process_group` do PTY (aquele resolve o
    // pgid de um líder que talvez não tenhamos criado nós — aqui criamos,
    // então já sabemos o pgid de cara).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().ok()?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                kill_probe_group(&mut child);
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

/// Mata o grupo inteiro do probe no timeout, não só o processo direto —
/// ver o comentário de `process_group(0)` acima. `killpg` já alcança o
/// líder (o próprio `child`), então não há `child.kill()` redundante aqui.
#[cfg(unix)]
fn kill_probe_group(child: &mut std::process::Child) {
    let pgid = child.id() as libc::pid_t;
    // SAFETY: `pgid` é o pid do nosso próprio filho, que por construção
    // (`process_group(0)`) é também o pgid do grupo dele — não há aliasing
    // com pid/pgid de outro processo do sistema.
    unsafe {
        libc::killpg(pgid, libc::SIGKILL);
    }
}

/// Windows não tem grupo de processo POSIX — mata só o filho direto, o
/// mesmo braço que já existia antes deste fix. Nenhuma sessão passa pelo
/// preflight na Camada A do Windows de um jeito que dependa disto: o
/// probe é efêmero (stdin null, sem tty), e não é aqui que a invariante
/// #9 vale — ela é sobre matar SESSÃO, não sobre este processo avulso.
#[cfg(not(unix))]
fn kill_probe_group(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// Review de segurança round 2, REQUERIDO: o cwd do probe NUNCA é o
/// worktree (a área não-confiável — `writable_root` do sandbox, repo que o
/// dono pode não ter escrito). O probe roda FORA da jaula (é irmão de
/// `binary_available()`, não de uma sessão que AGE — §6 do design), e o
/// estado que `claude auth status` lê vem do `HOME` (já presente no `env`
/// filtrado que o chamador passa), nunca do cwd — então apontar o cwd pro
/// worktree é exposição desnecessária: se o binário `claude` chegar a ler
/// config de projeto a partir do cwd (`.claude/settings.json` com hooks,
/// `.mcp.json`), um repo hostil ganharia execução de código no HOST, fora
/// do sandbox, só por o dono ter aberto a sessão — e não dependemos de
/// saber se a versão de hoje faz isso: o `claude` se auto-atualiza, e isto
/// fecha por construção (nenhum diretório de projeto chega a ser cwd).
///
/// Dir dedicado (não reusa o `runtime_dir` da sessão, que serve o socket de
/// hooks e tem outro ciclo de vida): mesma disciplina de criação
/// (`create_private_dir`/`verify_private_dir`, modo 0700 + dono conferido
/// no Unix) que os demais diretórios privados do TYBA. `None` em qualquer
/// falha de criação — o preflight silencia e não roda (P5: ambiguidade é
/// silêncio, nunca acusação, e aqui nem sequer é uma acusação, é a
/// impossibilidade de preparar um cwd seguro).
fn neutral_preflight_cwd(session_id: SessionId) -> Option<PathBuf> {
    let short = session_id.simple().to_string();
    let dir = std::env::temp_dir().join(format!("tyba-preflight-{}", &short[..12]));
    crate::session::create_private_dir(&dir).ok()?;
    crate::session::verify_private_dir(&dir).ok()?;
    Some(dir)
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
///
/// Sem parâmetro de `cwd`: de propósito, desde o review de segurança round
/// 2 -- o chamador NÃO decide mais o cwd do probe (era assim que
/// `worktree.path` vazava pra cá). `neutral_preflight_cwd` é a única fonte.
pub(crate) fn spawn_preflight<R: Runtime>(
    app: AppHandle<R>,
    session_id: SessionId,
    binary: Option<PathBuf>,
    env: HashMap<String, String>,
) {
    std::thread::spawn(move || {
        let Some(binary) = binary else {
            return;
        };
        let Some(cwd) = neutral_preflight_cwd(session_id) else {
            return;
        };
        let result = run_status_json(&binary, &env, &cwd, PREFLIGHT_TIMEOUT);
        // Dir efêmero, dedicado só a este probe -- nada escreve nele de
        // propósito, e ele não precisa sobreviver ao processo (ao contrário
        // do `runtime_dir`, que serve o socket de hooks pela vida da
        // sessão). `remove_dir` falha em silêncio se não estiver vazio ou
        // já tiver sumido -- não é um caminho de erro que importa aqui.
        let _ = std::fs::remove_dir(&cwd);
        let Some((stdout, exit)) = result else {
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

    /// Review de segurança round 2, MINOR (defesa em profundidade): sem
    /// `process_group(0)` + `killpg`, o timeout matava só o processo
    /// direto -- um "neto" que ele tivesse subido em background sobreviveria
    /// como órfão vivo. O script fake sobe um neto em background que faz
    /// BATIMENTO (append num arquivo a cada 50ms) em vez de só dormir.
    ///
    /// `kill(pid, 0)` foi cogitado e DESCARTADO como sensor: um zumbi (já
    /// morto por SIGKILL, ainda não colhido pelo pai) continua respondendo
    /// `kill(pid, 0) == 0` até algum `wait()` o colher -- e o container de
    /// teste roda `sleep infinity` como PID 1, que nunca colhe órfão nenhum
    /// (achado ao investigar um falso positivo real: o zumbi aparecia como
    /// "vivo" na checagem por PID mesmo tendo recebido o SIGKILL). O
    /// batimento mede a coisa que importa de verdade -- o processo ainda
    /// está FAZENDO algo? -- e não depende de reaping nem é Linux-only
    /// (`/proc` não existe no macOS, que também roda esta suíte no CI).
    #[test]
    #[cfg(unix)]
    fn run_status_json_timeout_kills_the_whole_group_not_just_the_direct_child() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_binary(
            dir.path(),
            "claude",
            "#!/bin/sh\n\
             (i=0; while [ $i -lt 200 ]; do echo tick >> heartbeat; i=$((i+1)); sleep 0.05; done) &\n\
             echo $! > grandchild-pid\n\
             sleep 10\n",
        );
        let pid_file = dir.path().join("grandchild-pid");
        let heartbeat = dir.path().join("heartbeat");

        let result = run_status_json(
            &bin,
            &HashMap::new(),
            dir.path(),
            Duration::from_millis(500),
        );
        assert!(result.is_none());

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !pid_file.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(pid_file.exists(), "o neto nunca chegou a subir");

        let ticks_at = || {
            std::fs::read_to_string(&heartbeat)
                .unwrap_or_default()
                .lines()
                .count()
        };
        let before = ticks_at();
        assert!(
            before > 0,
            "o neto não bateu nenhuma vez antes do timeout -- o teste não provaria nada"
        );
        std::thread::sleep(Duration::from_millis(300));
        let after = ticks_at();
        assert_eq!(
            before, after,
            "o neto seguiu batendo depois do timeout ({before} -> {after} ticks) -- \
             killpg não alcançou o grupo"
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
        let app = tauri::test::mock_app();
        let started = std::time::Instant::now();
        spawn_preflight(
            app.handle().clone(),
            crate::session::SessionId::new_v4(),
            None,
            HashMap::new(),
        );
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    /// Review de segurança round 2, REQUERIDO: o cwd do probe fica sob o
    /// tempdir do SO, namespaced pelo `session_id` -- nunca um diretório de
    /// projeto (worktree, repo) que o chamador poderia ter passado. Prova
    /// isolada da função pura, sem depender do fim-a-fim (que prova a
    /// FIAÇÃO, este prova o CÁLCULO).
    #[test]
    #[cfg(unix)]
    fn neutral_preflight_cwd_lives_under_the_os_tmp_dir_privately() {
        use std::os::unix::fs::PermissionsExt;

        let id = crate::session::SessionId::new_v4();
        let dir = neutral_preflight_cwd(id).expect("cwd neutro deveria ter sido criado");

        let tmp = std::env::temp_dir()
            .canonicalize()
            .unwrap_or_else(|_| std::env::temp_dir());
        assert!(
            dir.canonicalize()
                .unwrap_or_else(|_| dir.clone())
                .starts_with(&tmp),
            "cwd neutro fora do tempdir do SO: {}",
            dir.display()
        );
        let meta = std::fs::metadata(&dir).unwrap();
        assert!(meta.is_dir());
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o700,
            "cwd neutro precisa ser privado (0700), como os outros dirs do TYBA"
        );

        // Recém-criado: vazio. Nenhum `.claude/settings.json`/`.mcp.json`
        // de projeto nenhum pode estar em escopo aqui -- é justamente o
        // ponto do fix.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);

        std::fs::remove_dir(&dir).ok();
    }

    /// Review de segurança round 2, REQUERIDO -- o teste fim-a-fim: o
    /// "binário" fake grava o PRÓPRIO `$PWD` (canal separado do stdout, que
    /// precisa seguir parecendo `{"loggedIn": ...}` pro `classify`) e o
    /// teste confirma que o cwd de verdade usado pelo processo filho é
    /// EXATAMENTE o que `neutral_preflight_cwd` calcula pro mesmo
    /// `session_id` -- nunca um diretório de projeto. Como `spawn_preflight`
    /// não recebe mais `cwd` nenhum do chamador (o parâmetro foi removido),
    /// não há mais como um worktree entrar aqui por acidente -- fechado por
    /// construção, não só por convenção de chamada.
    #[test]
    #[cfg(unix)]
    fn spawn_preflight_runs_the_probe_with_the_neutral_cwd_not_a_project_dir() {
        let session_id = crate::session::SessionId::new_v4();
        let expected_cwd = neutral_preflight_cwd(session_id).expect("cwd neutro deveria existir");

        let bin = fake_binary(
            &expected_cwd,
            "claude",
            "#!/bin/sh\npwd > cwd-seen\nprintf '{\"loggedIn\": true}'\n",
        );
        let app = tauri::test::mock_app();

        spawn_preflight(app.handle().clone(), session_id, Some(bin), HashMap::new());

        let marker = expected_cwd.join("cwd-seen");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let seen_cwd =
            std::fs::read_to_string(&marker).unwrap_or_else(|e| panic!("o probe nunca rodou: {e}"));
        assert_eq!(
            seen_cwd.trim(),
            expected_cwd
                .canonicalize()
                .unwrap_or(expected_cwd.clone())
                .to_string_lossy(),
            "o cwd de verdade do processo não bateu com o cwd neutro calculado"
        );

        std::fs::remove_dir_all(&expected_cwd).ok();
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

        spawn_preflight(handle, session_id, Some(bin), HashMap::new());

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
