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
    };
    assert_eq!(handle(&request, &layout, &sessions).code, Some(EXIT_USAGE));
}

// ======================= socket de verdade num tempdir =======================

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn serve_fixture(dir: &Path) -> (ControlServer, PathBuf) {
    let socket = dir.join("ctl").join("control.sock");
    let (layout, sessions) = fixture();
    let handler: ControlHandler = Arc::new(move |req| handle(&req, &layout, &sessions));
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
        Arc::new(|_| ControlResponse::error(EXIT_TARGET, "alvo ambíguo: a, b"));
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
