//! Fixtures sintéticas: títulos, nomes e ids escritos à mão.

use std::sync::Arc;

use super::server::{ControlHandler, ControlServer};
use super::*;
use crate::layout::{LayoutManager, SplitKind};
use crate::session::store::Store;
use crate::session::{ObservedAgent, SessionId};

fn session(kind: SessionKind, title: &str) -> Session {
    Session {
        id: SessionId::new_v4(),
        kind,
        title: title.into(),
        repo_root: None,
        worktree: None,
        status: SessionStatus::Running,
        attention: false,
        created_at: chrono::Utc::now(),
        cwd: None,
        connection: Default::default(),
        connection_failure: None,
        agent_conversation_id: None,
        observed: None,
        agent_model: None,
        status_note: None,
        subagents_working: false,
        agent_usage: None,
        last_turn: None,
        turn_open: false,
        last_words: None,
        turn_started_at_ms: None,
        shell_resume_offer: false,
        agent_conversation_cwd: None,
        owner: Default::default(),
    }
}

/// Dois workspaces: "api" com um shell e um claude lado a lado; "web" com um
/// shell onde a tela sugere um codex esperando.
fn fixture() -> (LayoutState, Vec<Session>) {
    let shell = session(SessionKind::Shell, "zsh");
    let mut agent = session(
        SessionKind::Agent {
            runner: AgentRunnerKind::ClaudeCode,
        },
        "corrige o parser",
    );
    agent.status = SessionStatus::AwaitingInput {
        hint: None,
        reason: Default::default(),
    };
    let mut observed = session(SessionKind::Shell, "web");
    observed.observed = Some(ObservedAgent {
        agent: "codex".into(),
        state: Some(ObservedState::AwaitingInput),
        kind: None,
    });

    let mgr = LayoutManager::new(Arc::new(Store::open_in_memory().unwrap()));
    mgr.create_workspace("api", None, shell.id).unwrap();
    let pane = mgr.state().workspaces[0].tabs[0].active_pane.unwrap();
    mgr.split_pane(pane, SplitKind::V, agent.id).unwrap();
    mgr.create_workspace("web", None, observed.id).unwrap();
    (mgr.state(), vec![shell, agent, observed])
}

#[test]
fn listing_follows_the_layout_with_global_positions_and_agent_state() {
    let (layout, sessions) = fixture();
    let listing = build_listing(&layout, &sessions);

    assert_eq!(listing.workspaces.len(), 2);
    let api = &listing.workspaces[0];
    assert_eq!(api.name, "api");
    let positions: Vec<usize> = listing
        .workspaces
        .iter()
        .flat_map(|w| w.panes.iter().map(|p| p.position))
        .collect();
    assert_eq!(positions, [1, 2, 3]);

    assert_eq!(api.panes[0].id, sessions[0].id.to_string());
    assert_eq!(api.panes[0].kind, "shell");
    assert_eq!(api.panes[0].agent, None);

    let claude = api.panes[1].agent.as_ref().unwrap();
    assert_eq!(claude.name, "claude");
    assert_eq!(claude.state.as_deref(), Some("awaiting_input"));
    assert!(!claude.observed);

    let codex = listing.workspaces[1].panes[0].agent.as_ref().unwrap();
    assert_eq!(codex.name, "codex");
    assert_eq!(codex.state.as_deref(), Some("awaiting_input"));
    assert!(codex.observed);
}

#[test]
fn listing_redacts_titles() {
    let (layout, mut sessions) = fixture();
    sessions[0].title = "export TOKEN=sk-ant-api03-FAKEFAKEFAKEFAKEFAKEFAKEFAKE".into();
    let listing = build_listing(&layout, &sessions);
    let label = &listing.workspaces[0].panes[0].label;
    assert!(!label.contains("FAKEFAKE"), "{label}");
}

#[test]
fn pane_of_unknown_session_is_left_out() {
    let (layout, sessions) = fixture();
    let listing = build_listing(&layout, &sessions[1..]);
    assert_eq!(listing.workspaces[0].panes.len(), 1);
    assert_eq!(listing.workspaces[0].panes[0].position, 1);
}

#[test]
fn unknown_op_is_usage_error() {
    let (layout, sessions) = fixture();
    let request = ControlRequest {
        v: PROTOCOL_VERSION,
        op: "rm".into(),
        ..Default::default()
    };
    assert_eq!(
        handle(&request, &layout, &sessions, fake_scrollback).code,
        Some(EXIT_USAGE)
    );
}

/// Scrollback sintético com um segredo de cada tipo que a redação conhece.
/// Os valores são de mentira, montados para casar com os padrões.
const FAKE_AWS: &str = "AKIAFAKEFAKEFAKEFAKE";
const FAKE_SK: &str = "sk-FAKEfakeFAKEfakeFAKEfake00";
const FAKE_KEY_BODY: &str = "RkFLRUZBS0VGQUtFRkFLRUZBS0VGQUtFRkFLRQ==";

fn fake_scrollback(_: SessionId) -> Option<Vec<String>> {
    Some(
        [
            "$ echo oi",
            "oi",
            &format!("export AWS_ACCESS_KEY_ID={FAKE_AWS}"),
            &format!("token: {FAKE_SK}"),
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            FAKE_KEY_BODY,
            FAKE_KEY_BODY,
            "-----END OPENSSH PRIVATE KEY-----",
            "fim",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
    )
}

fn capture_request(target: &str, lines: Option<usize>) -> ControlRequest {
    ControlRequest {
        v: PROTOCOL_VERSION,
        op: "capture".into(),
        target: Some(target.into()),
        lines,
        ..Default::default()
    }
}

fn assert_clean(lines: &[String]) {
    let all = lines.join("\n");
    for secret in [FAKE_AWS, FAKE_SK, FAKE_KEY_BODY] {
        assert!(!all.contains(secret), "vazou {secret}:\n{all}");
    }
}

#[test]
fn capture_is_redacted_before_leaving_the_app() {
    let (layout, sessions) = fixture();
    let response = handle(
        &capture_request("1", None),
        &layout,
        &sessions,
        fake_scrollback,
    );
    let result: CaptureResult = serde_json::from_value(response.result.unwrap()).unwrap();
    assert_eq!(result.pane, sessions[0].id.to_string());
    assert_clean(&result.lines);
    assert_eq!(result.lines.first().map(String::as_str), Some("$ echo oi"));
    assert_eq!(result.lines.last().map(String::as_str), Some("fim"));
}

#[test]
fn lines_cuts_after_redacting_so_a_cut_key_body_still_hides() {
    let (layout, sessions) = fixture();
    // As 3 últimas: corpo, END, fim. Sem o BEGIN na janela, o corpo só some
    // porque a redação rodou no scrollback inteiro.
    let response = handle(
        &capture_request("1", Some(3)),
        &layout,
        &sessions,
        fake_scrollback,
    );
    let result: CaptureResult = serde_json::from_value(response.result.unwrap()).unwrap();
    assert_eq!(result.lines.len(), 3);
    assert_clean(&result.lines);
    assert_eq!(result.lines[2], "fim");
}

#[test]
fn capture_target_errors_follow_the_contract() {
    let (layout, mut sessions) = fixture();
    sessions[1].title = "zsh".into(); // dois panes chamados "zsh"
    let code = |target: &str| {
        handle(
            &capture_request(target, None),
            &layout,
            &sessions,
            fake_scrollback,
        )
        .code
    };
    assert_eq!(code("zsh"), Some(EXIT_TARGET), "ambíguo");
    assert_eq!(code("99"), Some(EXIT_TARGET), "não achado");
    assert_eq!(code(""), Some(EXIT_USAGE), "vazio");
    assert_eq!(code("web"), None, "rótulo único resolve");
}

#[test]
fn capture_of_a_pane_without_terminal_is_a_failure() {
    let (layout, sessions) = fixture();
    let response = handle(&capture_request("1", None), &layout, &sessions, |_| None);
    assert_eq!(response.code, Some(EXIT_FAILURE));
}

#[test]
fn capture_args_are_validated_locally() {
    let parsed = parse_args(&args(&["capture", "api", "--lines", "5", "--json"])).unwrap();
    assert_eq!(parsed.request, capture_request("api", Some(5)));
    assert!(parsed.json);
    let parsed = parse_args(&args(&["capture", "--lines=2", "api"])).unwrap();
    assert_eq!(parsed.request.lines, Some(2));
    for bad in [
        &["capture"][..],
        &["capture", "a", "b"],
        &["capture", "a", "--lines"],
        &["capture", "a", "--lines", "0"],
        &["capture", "a", "--lines", "dez"],
        &["ls", "api"],
        &["ls", "--lines", "3"],
    ] {
        assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
    }
}

// ======================= socket de verdade num tempdir =======================

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn serve_fixture(dir: &Path) -> (ControlServer, PathBuf) {
    let socket = dir.join("ctl").join("control.sock");
    let (layout, sessions) = fixture();
    let handler: ControlHandler =
        Arc::new(move |req, _| handle(&req, &layout, &sessions, fake_scrollback));
    (ControlServer::bind(&socket, handler).unwrap(), socket)
}

fn run_capture(list: &[&str], socket: &Path) -> (i32, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run(&args(list), socket, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn ls_over_the_socket_prints_the_listing() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, socket) = serve_fixture(dir.path());

    let (code, out, err) = run_capture(&["ls"], &socket);
    assert_eq!(code, EXIT_OK, "{err}");
    assert!(
        out.contains("api (ativa)") || out.contains("api\n"),
        "{out}"
    );
    assert!(out.contains("claude · esperando você"), "{out}");
    assert!(out.contains("codex · esperando você"), "{out}");
}

#[test]
fn ls_json_is_the_listing_itself() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, socket) = serve_fixture(dir.path());

    let (code, out, _) = run_capture(&["ls", "--json"], &socket);
    assert_eq!(code, EXIT_OK);
    let listing: Listing = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(listing.workspaces.len(), 2);
    let value: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    let pane = &value["workspaces"][0]["panes"][1];
    for key in ["id", "position", "label", "kind", "agent"] {
        assert!(pane.get(key).is_some(), "campo {key} sumiu do --json");
    }
}

#[cfg(unix)]
#[test]
fn socket_and_its_dir_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (_server, socket) = serve_fixture(dir.path());
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(socket.parent().unwrap()), 0o700);
    assert_eq!(mode(&socket), 0o600);
}

#[test]
fn no_app_means_exit_4() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("ctl").join("control.sock");
    let (code, _, err) = run_capture(&["ls"], &socket);
    assert_eq!(code, EXIT_NOT_RUNNING);
    assert!(err.contains("não está aberto"), "{err}");
}

#[test]
fn bad_usage_is_exit_2_and_never_connects() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("nada.sock");
    assert_eq!(run_capture(&["ls", "--xpto"], &socket).0, EXIT_USAGE);
    assert_eq!(run_capture(&["voa"], &socket).0, EXIT_USAGE);
}

#[cfg(unix)]
#[test]
fn socket_in_a_dir_open_to_others_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (_server, socket) = serve_fixture(dir.path());
    std::fs::set_permissions(
        socket.parent().unwrap(),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let (code, _, err) = run_capture(&["ls"], &socket);
    assert_eq!(code, EXIT_FAILURE);
    assert!(err.contains("recusado"), "{err}");
}

#[test]
fn error_code_from_the_app_becomes_the_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("ctl").join("control.sock");
    let handler: ControlHandler =
        Arc::new(|_, _| ControlResponse::error(EXIT_TARGET, "alvo ambíguo: a, b"));
    let _server = ControlServer::bind(&socket, handler).unwrap();
    let (code, _, err) = run_capture(&["ls"], &socket);
    assert_eq!(code, EXIT_TARGET);
    assert!(err.contains("ambíguo"), "{err}");
}

#[test]
fn garbage_line_gets_a_usage_answer_not_a_crash() {
    use std::io::{BufRead, BufReader, Write};
    let dir = tempfile::tempdir().unwrap();
    let (_server, socket) = serve_fixture(dir.path());
    let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    stream.write_all(b"isto nao e json\n").unwrap();
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).unwrap();
    let response: ControlResponse = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(response.code, Some(EXIT_USAGE));
}

#[test]
fn capture_over_the_socket_prints_redacted_text() {
    let dir = tempfile::tempdir().unwrap();
    let (_server, socket) = serve_fixture(dir.path());

    let (code, out, err) = run_capture(&["capture", "1", "--lines", "4"], &socket);
    assert_eq!(code, EXIT_OK, "{err}");
    let lines: Vec<String> = out.lines().map(str::to_string).collect();
    assert_eq!(lines.len(), 4);
    assert_clean(&lines);

    let (code, out, _) = run_capture(&["capture", "1", "--json"], &socket);
    assert_eq!(code, EXIT_OK);
    let result: CaptureResult = serde_json::from_str(out.trim()).unwrap();
    assert_clean(&result.lines);

    assert_eq!(run_capture(&["capture", "nada"], &socket).0, EXIT_TARGET);
}

// ================================ `tyba run` ================================

fn run_request(command: &str) -> ControlRequest {
    ControlRequest {
        v: PROTOCOL_VERSION,
        op: "run".into(),
        command: Some(command.into()),
        ..Default::default()
    }
}

#[test]
fn run_args_take_the_command_after_the_double_dash() {
    let parsed = parse_args(&args(&[
        "run", "--wait", "--cwd", "/tmp", "--", "cargo", "test",
    ]))
    .unwrap();
    assert_eq!(parsed.request.op, "run");
    assert_eq!(parsed.request.command.as_deref(), Some("cargo test"));
    assert_eq!(parsed.request.cwd.as_deref(), Some("/tmp"));
    assert!(parsed.request.wait);
    // Depois do `--`, flag é do comando, não da CLI.
    let parsed = parse_args(&args(&["run", "--", "ls", "--json"])).unwrap();
    assert_eq!(parsed.request.command.as_deref(), Some("ls --json"));
    assert!(!parsed.json && !parsed.request.wait);
    for bad in [
        &["run"][..],
        &["run", "--"],
        &["run", "--", " "],
        &["run", "ls"],
        &["run", "--cwd"],
        &["run", "--lines", "3", "--", "ls"],
        &["ls", "--wait"],
        &["capture", "1", "--", "ls"],
    ] {
        assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn run_refuses_what_the_guard_rail_refuses() {
    for command in [
        "gh pr merge 12",
        "git push --no-verify origin feat",
        "git -c core.hooksPath=/dev/null push",
        "sh -c 'gh pr review 3 --approve'",
    ] {
        let response = checked_run_command(&run_request(command)).unwrap_err();
        assert_eq!(response.code, Some(EXIT_FAILURE), "{command}");
        assert!(response.error.unwrap().starts_with("TYBA:"), "{command}");
    }
    assert_eq!(
        checked_run_command(&run_request("  cargo test --lib  ")).unwrap(),
        "cargo test --lib"
    );
}

#[test]
fn run_refuses_more_than_one_line_or_control_characters() {
    // Um `\n` digitado viraria um segundo comando, fora do guarda-corpo.
    // E um TAB completaria `unset GIT_CONFI` para `GIT_CONFIG_COUNT` no zsh.
    for command in [
        "ls\ngh pr merge 1",
        "ls\rx",
        "ls\u{1b}[A",
        "ls\u{3}",
        "unset GIT_CONFI\t\t",
        "",
    ] {
        let response = checked_run_command(&run_request(command)).unwrap_err();
        assert_eq!(response.code, Some(EXIT_USAGE), "{command:?}");
    }
    assert!(checked_run_command(&run_request("printf 'a\\tb'")).is_ok());
}

#[test]
fn run_output_is_redacted_before_leaving_the_app() {
    let bytes = format!("ok\r\nexport AWS_ACCESS_KEY_ID={FAKE_AWS}\r\ntoken: {FAKE_SK}\r\nfim\r\n");
    let result = run_result(
        "pane-1".into(),
        waiters::Done {
            exit_code: Some(2),
            bytes: bytes.into_bytes(),
            cols: 120,
            rows: 32,
        },
    );
    assert_eq!(result.exit_code, Some(2));
    let lines = result.lines.unwrap();
    assert!(lines.iter().any(|l| l == "ok"), "{lines:?}");
    assert!(lines.iter().any(|l| l == "fim"), "{lines:?}");
    assert_clean(&lines);
}

#[test]
fn run_wait_exits_with_the_command_code() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("ctl").join("control.sock");
    let handler: ControlHandler = Arc::new(|req, _| {
        let code = if req.wait { Some(7) } else { None };
        ControlResponse::ok(
            serde_json::to_value(RunResult {
                pane: "pane-1".into(),
                exit_code: code,
                lines: req.wait.then(|| vec!["saida".to_string()]),
            })
            .unwrap(),
        )
    });
    let _server = ControlServer::bind(&socket, handler).unwrap();

    let (code, out, err) = run_capture(&["run", "--wait", "--", "false"], &socket);
    assert_eq!(code, 7, "{err}");
    assert_eq!(out, "saida\n");

    let (code, out, _) = run_capture(&["run", "--", "sleep", "9"], &socket);
    assert_eq!(code, EXIT_OK);
    assert_eq!(out, "pane-1\n");

    let (code, out, _) = run_capture(&["run", "--wait", "--json", "--", "false"], &socket);
    assert_eq!(code, 7);
    let result: RunResult = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(result.exit_code, Some(7));
}

#[test]
fn the_typed_line_keeps_the_command_inside_single_quotes() {
    // `!` fora de aspas simples seria expandido pelo histórico do shell
    // interativo DEPOIS do guarda-corpo ter olhado.
    assert_eq!(
        typed_run_line("echo mer ge; gh pr !#:1!#:2 12"),
        "eval 'echo mer ge; gh pr !#:1!#:2 12'"
    );
    assert_eq!(typed_run_line("echo 'a b'"), r"eval 'echo '\''a b'\'''");
}

#[test]
fn relative_cwd_is_resolved_where_the_cli_runs() {
    let parsed = parse_args(&args(&["run", "--cwd", "sub", "--", "ls"])).unwrap();
    let here = std::env::current_dir().unwrap().join("sub");
    assert_eq!(parsed.request.cwd.as_deref(), here.to_str());
    let parsed = parse_args(&args(&["run", "--cwd", "/abs", "--", "ls"])).unwrap();
    assert_eq!(parsed.request.cwd.as_deref(), Some("/abs"));
}

#[test]
fn the_handler_sees_when_the_client_goes_away() {
    use std::io::Write;
    use std::sync::mpsc;
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("ctl").join("control.sock");
    let (tx, rx) = mpsc::channel();
    let handler: ControlHandler = Arc::new(move |_, connected| {
        let alive_at_start = connected();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while connected() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = tx.send((alive_at_start, connected()));
        ControlResponse::error(EXIT_FAILURE, "fim")
    });
    let _server = ControlServer::bind(&socket, handler).unwrap();
    let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    stream.write_all(b"{\"v\":1,\"op\":\"run\"}\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
    drop(stream);
    let (alive_at_start, alive_at_end) =
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert!(alive_at_start);
    assert!(!alive_at_end);
}

// ============================== `tyba events` ==============================

/// O fixture e, depois, o mesmo com o shell fechado e o claude de volta a
/// rodar.
fn before_and_after() -> (Listing, Listing, Vec<Session>) {
    let (layout, sessions) = fixture();
    let before = build_listing(&layout, &sessions);
    let mut later: Vec<Session> = sessions[1..].to_vec();
    later[0].status = SessionStatus::Running;
    let after = build_listing(&layout, &later);
    (before, after, sessions)
}

#[test]
fn events_diff_names_what_closed_and_which_agent_changed() {
    let (before, after, sessions) = before_and_after();
    let events = events::diff(&before, &after);
    assert_eq!(
        events,
        vec![
            events::Event::PaneClosed {
                pane: sessions[0].id.to_string()
            },
            events::Event::Agent {
                pane: sessions[1].id.to_string(),
                agent: Some(AgentEntry {
                    name: "claude".into(),
                    state: Some("running".into()),
                    observed: false,
                }),
            },
        ]
    );
    assert!(events::diff(&after, &after).is_empty(), "sem mudança, nada");
}

#[test]
fn events_diff_reports_a_new_pane_with_its_redacted_label() {
    let (layout, mut sessions) = fixture();
    let after = build_listing(&layout, &sessions[1..]);
    sessions[0].title = "export TOKEN=sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789".into();
    let opened = build_listing(&layout, &sessions);
    let events = events::diff(&after, &opened);
    let Some(events::Event::PaneOpened {
        pane, label, kind, ..
    }) = events.first()
    else {
        panic!("esperava pane_opened: {events:?}");
    };
    assert_eq!(pane, &sessions[0].id.to_string());
    assert_eq!(kind, "shell");
    assert!(
        !label.contains("abcdefghijklmnop"),
        "rótulo sem redação: {label}"
    );
}

#[test]
fn events_json_lines_carry_the_event_name() {
    let line = serde_json::to_value(events::Event::PaneClosed { pane: "p1".into() }).unwrap();
    assert_eq!(
        line,
        serde_json::json!({"event": "pane_closed", "pane": "p1"})
    );
}

/// Aceita `limit` linhas e depois falha, como um `| head -n limit`.
struct Head {
    text: Arc<std::sync::Mutex<String>>,
    limit: usize,
}

impl Write for Head {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut text = self.text.lock().unwrap();
        if text.matches('\n').count() >= self.limit {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        text.push_str(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn events_over_the_socket_print_changes_and_stop_when_the_output_closes() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("ctl").join("control.sock");
    let (before, after, _) = before_and_after();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = calls.clone();
    let handler: ControlHandler = Arc::new(move |req, _| {
        assert_eq!(req.op, "ls", "events só lê o ls");
        let n = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // Vai e volta, para sempre haver mudança e o `Head` encher.
        let listing = if n.is_multiple_of(2) { &before } else { &after };
        ControlResponse::ok(serde_json::to_value(listing).unwrap())
    });
    let _server = ControlServer::bind(&socket, handler).unwrap();

    let text = Arc::new(std::sync::Mutex::new(String::new()));
    let mut out = Head {
        text: text.clone(),
        limit: 2,
    };
    let mut err = Vec::new();
    let code = run(&args(&["events"]), &socket, &mut out, &mut err);
    assert_eq!(code, EXIT_OK, "{}", String::from_utf8_lossy(&err));
    assert!(String::from_utf8_lossy(&err).starts_with("observando o TYBA"));
    let text = text.lock().unwrap();
    assert!(text.starts_with("fechou "), "{text}");
    assert!(text.contains("claude · rodando"), "{text}");
}

#[test]
fn events_without_the_app_is_exit_4_and_extra_args_are_usage() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("ctl").join("control.sock");
    assert_eq!(run_capture(&["events"], &socket).0, EXIT_NOT_RUNNING);
    assert_eq!(run_capture(&["events", "p1"], &socket).0, EXIT_USAGE);
    assert_eq!(
        run_capture(&["events", "--lines", "3"], &socket).0,
        EXIT_USAGE
    );
}

// =============================== `tyba send` ===============================

fn send_request(text: &str) -> ControlRequest {
    ControlRequest {
        v: PROTOCOL_VERSION,
        op: "send".into(),
        target: Some("2".into()),
        text: Some(text.into()),
        ..Default::default()
    }
}

#[test]
fn send_args_take_the_pane_then_the_text_with_or_without_double_dash() {
    for list in [
        &["send", "2", "rode", "os", "testes"][..],
        &["send", "2", "--", "rode", "os", "testes"][..],
        &["send", "--json", "2", "rode os testes"][..],
    ] {
        let invocation = parse_args(&args(list)).unwrap();
        assert_eq!(invocation.request.op, "send");
        assert_eq!(invocation.request.target.as_deref(), Some("2"));
        assert_eq!(invocation.request.text.as_deref(), Some("rode os testes"));
    }
    for bad in [
        &["send"][..],
        &["send", "2"][..],
        &["send", "2", "--wait", "oi"][..],
        &["send", "2", "--lines", "3", "oi"][..],
        &["send", "2", "a", "--", "b"][..],
    ] {
        assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn send_refuses_what_the_guard_rail_refuses() {
    for text in [
        "gh pr merge 12",
        "git push --no-verify origin main",
        "git -c core.hooksPath=/dev/null push",
    ] {
        let response = checked_send_text(&send_request(text)).unwrap_err();
        assert_eq!(response.code, Some(EXIT_FAILURE), "{text}");
    }
}

#[test]
fn send_refuses_slash_commands_exit_and_control_characters() {
    for text in ["/exit", " /quit", "/meu-comando x", "exit", "QUIT"] {
        let response = checked_send_text(&send_request(text)).unwrap_err();
        assert_eq!(response.code, Some(EXIT_FAILURE), "{text}");
    }
    for text in ["", "   ", "oi\nrm -rf x", "oi\r", "a\tb", "\x1b[A"] {
        let response = checked_send_text(&send_request(text)).unwrap_err();
        assert_eq!(response.code, Some(EXIT_USAGE), "{text:?}");
    }
    assert_eq!(
        checked_send_text(&send_request("  rode os testes e me diga  ")).unwrap(),
        "rode os testes e me diga"
    );
    assert!(checked_send_text(&send_request("exit code do teste?")).is_ok());
}

#[test]
fn send_only_reaches_an_idle_agent_and_never_a_shell_prompt() {
    // agente no pane, parado, shell de trás fora do prompt: entrega.
    assert_eq!(send_refusal(true, true, Some(false)), None);
    assert_eq!(send_refusal(true, true, None), None);
    // sem agente: o texto viraria comando de shell.
    assert!(send_refusal(false, true, Some(false)).is_some());
    // o agente saiu e o shell voltou ao prompt, mesmo com status velho.
    assert!(send_refusal(true, true, Some(true)).is_some());
    // trabalhando ou pedindo permissão (não `Idle`).
    assert!(send_refusal(true, false, Some(false)).is_some());
}

#[test]
fn send_exits_by_the_delivery_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    for (delivered, code, text) in [
        (true, EXIT_OK, "entregue a p1"),
        (false, EXIT_FAILURE, "não começou"),
    ] {
        let socket = dir
            .path()
            .join(format!("ctl-{delivered}"))
            .join("control.sock");
        let handler: ControlHandler = Arc::new(move |req, _| {
            assert_eq!(req.op, "send");
            assert_eq!(req.text.as_deref(), Some("oi"));
            ControlResponse::ok(
                serde_json::to_value(SendResult {
                    pane: "p1".into(),
                    delivered,
                })
                .unwrap(),
            )
        });
        let _server = ControlServer::bind(&socket, handler).unwrap();
        let (exit, out, _) = run_capture(&["send", "2", "oi"], &socket);
        assert_eq!(exit, code);
        assert!(out.contains(text), "{out}");
    }
}
