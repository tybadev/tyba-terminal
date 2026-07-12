use std::io::Cursor;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::json;
use tempfile::TempDir;

use super::client::{run_client, run_client_inner, RetryPlan};
use super::server::{HookServer, MAX_INFLIGHT};
use super::{HookAction, HookEvent};

fn socket_in(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

fn fast_retry() -> RetryPlan {
    RetryPlan {
        attempts: 3,
        base: Duration::from_millis(1),
    }
}

fn pretooluse_event(tool: &str) -> serde_json::Value {
    json!({
        "hook_event_name": "PreToolUse",
        "tool_name": tool,
        "tool_input": {"command": "ls"},
        "cwd": "/work"
    })
}

fn run(input: serde_json::Value, socket: Option<&Path>) -> String {
    let bytes = serde_json::to_vec(&input).unwrap();
    let mut out = Vec::new();
    let code = run_client(Cursor::new(bytes), &mut out, socket);
    assert_eq!(code, 0);
    String::from_utf8(out).unwrap()
}

#[test]
fn round_trip_allow_pretooluse() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(
        &path,
        Arc::new(|_e: HookEvent| HookAction::Allow {
            reason: Some("ok".into()),
        }),
    )
    .unwrap();

    let out = run(pretooluse_event("Bash"), Some(&path));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
    assert_eq!(v["hookSpecificOutput"]["permissionDecisionReason"], "ok");

    server.shutdown();
}

#[test]
fn round_trip_deny_pretooluse() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(
        &path,
        Arc::new(|_e: HookEvent| HookAction::Deny {
            reason: "nope".into(),
        }),
    )
    .unwrap();

    let out = run(pretooluse_event("Bash"), Some(&path));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(v["hookSpecificOutput"]["permissionDecisionReason"], "nope");

    server.shutdown();
}

fn permission_request_event() -> serde_json::Value {
    json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "shell",
        "tool_input": {"command": "curl https://x.dev"},
        "cwd": "/work"
    })
}

#[test]
fn allow_pretooluse_echoes_tool_input_so_codex_aceita_o_allow() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(
        &path,
        Arc::new(|_e: HookEvent| HookAction::Allow { reason: None }),
    )
    .unwrap();

    let out = run(pretooluse_event("Bash"), Some(&path));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
    assert_eq!(
        v["hookSpecificOutput"]["updatedInput"],
        json!({"command": "ls"}),
        "sem updatedInput o Codex trata o allow como unsupported e cai no prompt nativo"
    );

    server.shutdown();
}

#[test]
fn deny_pretooluse_nao_manda_updated_input() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(
        &path,
        Arc::new(|_e: HookEvent| HookAction::Deny {
            reason: "nope".into(),
        }),
    )
    .unwrap();

    let out = run(pretooluse_event("Bash"), Some(&path));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["hookSpecificOutput"].get("updatedInput").is_none());

    server.shutdown();
}

#[test]
fn round_trip_allow_permission_request() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(
        &path,
        Arc::new(|_e: HookEvent| HookAction::Allow { reason: None }),
    )
    .unwrap();

    let out = run(permission_request_event(), Some(&path));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["hookSpecificOutput"]["hookEventName"],
        "PermissionRequest"
    );
    assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "allow");
    assert!(v["hookSpecificOutput"]["decision"].get("message").is_none());

    server.shutdown();
}

#[test]
fn round_trip_deny_permission_request_carries_message() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(
        &path,
        Arc::new(|_e: HookEvent| HookAction::Deny {
            reason: "sem rede".into(),
        }),
    )
    .unwrap();

    let out = run(permission_request_event(), Some(&path));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "deny");
    assert_eq!(v["hookSpecificOutput"]["decision"]["message"], "sem rede");

    server.shutdown();
}

#[test]
fn deny_pretooluse_with_empty_reason_gets_fallback_text() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(
        &path,
        Arc::new(|_e: HookEvent| HookAction::Deny { reason: "".into() }),
    )
    .unwrap();

    let out = run(pretooluse_event("Bash"), Some(&path));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(
        v["hookSpecificOutput"]["permissionDecisionReason"],
        "negado no TYBA"
    );

    server.shutdown();
}

#[test]
fn transport_failure_is_silent_for_permission_request() {
    let out = run(permission_request_event(), None);
    assert_eq!(out, "");
}

#[test]
fn decision_output_ends_with_newline_so_line_buffered_stdout_flushes() {
    let out = run(pretooluse_event("Bash"), None);
    assert!(out.ends_with('\n'));
}

#[test]
fn ack_non_pretooluse_prints_nothing() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(&path, Arc::new(|_e: HookEvent| HookAction::Ack)).unwrap();

    let out = run(json!({"hook_event_name": "Stop"}), Some(&path));
    assert_eq!(out, "");

    server.shutdown();
}

#[test]
fn handler_receives_parsed_fields() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let seen: Arc<Mutex<Option<HookEvent>>> = Arc::new(Mutex::new(None));
    let sink = seen.clone();
    let server = HookServer::bind(
        &path,
        Arc::new(move |e: HookEvent| {
            *sink.lock().unwrap() = Some(e);
            HookAction::Allow { reason: None }
        }),
    )
    .unwrap();

    let _ = run(pretooluse_event("Bash"), Some(&path));

    let guard = seen.lock().unwrap();
    let e = guard.as_ref().unwrap();
    assert_eq!(e.hook_event_name, "PreToolUse");
    assert_eq!(e.tool_name.as_deref(), Some("Bash"));
    assert_eq!(e.cwd.as_deref(), Some("/work"));
    assert_eq!(e.tool_input.as_ref().unwrap()["command"], "ls");
    assert_eq!(e.raw["hook_event_name"], "PreToolUse");

    drop(guard);
    server.shutdown();
}

#[test]
fn notification_type_is_parsed() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let seen: Arc<Mutex<Option<HookEvent>>> = Arc::new(Mutex::new(None));
    let sink = seen.clone();
    let server = HookServer::bind(
        &path,
        Arc::new(move |e: HookEvent| {
            *sink.lock().unwrap() = Some(e);
            HookAction::Ack
        }),
    )
    .unwrap();

    let _ = run(
        json!({"hook_event_name": "Notification", "notification_type": "permission"}),
        Some(&path),
    );

    let guard = seen.lock().unwrap();
    let e = guard.as_ref().unwrap();
    assert_eq!(e.hook_event_name, "Notification");
    assert_eq!(e.notification_type.as_deref(), Some("permission"));

    drop(guard);
    server.shutdown();
}

#[test]
fn allow_pretooluse_with_null_reason_emits_empty_string() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(
        &path,
        Arc::new(|_e: HookEvent| HookAction::Allow { reason: None }),
    )
    .unwrap();

    let out = run(pretooluse_event("Bash"), Some(&path));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecisionReason"], "");

    server.shutdown();
}

#[test]
fn missing_socket_env_denies_pretooluse() {
    let out = run(pretooluse_event("Bash"), None);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
}

#[test]
fn missing_socket_env_silent_for_stop() {
    let out = run(json!({"hook_event_name": "Stop"}), None);
    assert_eq!(out, "");
}

#[test]
fn unreachable_server_denies_pretooluse() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "nope.sock");
    let bytes = serde_json::to_vec(&pretooluse_event("Bash")).unwrap();
    let mut out = Vec::new();
    let code = run_client_inner(Cursor::new(bytes), &mut out, Some(&path), fast_retry());
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out).unwrap()).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
}

#[test]
fn unreachable_server_silent_for_stop() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "nope.sock");
    let bytes = serde_json::to_vec(&json!({"hook_event_name": "Stop"})).unwrap();
    let mut out = Vec::new();
    let code = run_client_inner(Cursor::new(bytes), &mut out, Some(&path), fast_retry());
    assert_eq!(code, 0);
    assert_eq!(out.len(), 0);
}

#[test]
fn malformed_stdin_is_silent() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(&path, Arc::new(|_e: HookEvent| HookAction::Ack)).unwrap();

    let mut out = Vec::new();
    let code = run_client(
        Cursor::new(b"not json at all".to_vec()),
        &mut out,
        Some(&path),
    );
    assert_eq!(code, 0);
    assert_eq!(out.len(), 0);

    server.shutdown();
}

#[test]
fn non_object_json_is_silent() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(&path, Arc::new(|_e: HookEvent| HookAction::Ack)).unwrap();

    let mut out = Vec::new();
    let code = run_client(Cursor::new(b"[1,2,3]".to_vec()), &mut out, Some(&path));
    assert_eq!(code, 0);
    assert_eq!(out.len(), 0);

    server.shutdown();
}

#[test]
fn truncated_response_denies_pretooluse() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "raw.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let handle = thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            drop(stream);
        }
    });

    let bytes = serde_json::to_vec(&pretooluse_event("Bash")).unwrap();
    let mut out = Vec::new();
    let code = run_client(Cursor::new(bytes), &mut out, Some(&path));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out).unwrap()).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");

    handle.join().unwrap();
}

#[test]
fn garbage_response_denies_pretooluse() {
    use std::io::Write;
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "raw.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.write_all(b"this is not json\n");
        }
    });

    let bytes = serde_json::to_vec(&pretooluse_event("Bash")).unwrap();
    let mut out = Vec::new();
    let code = run_client(Cursor::new(bytes), &mut out, Some(&path));
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out).unwrap()).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");

    handle.join().unwrap();
}

#[test]
fn orphan_socket_file_is_rebound() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    std::fs::write(&path, b"stale").unwrap();
    assert!(path.exists());

    let server = HookServer::bind(
        &path,
        Arc::new(|_e: HookEvent| HookAction::Allow { reason: None }),
    )
    .unwrap();

    let out = run(pretooluse_event("Bash"), Some(&path));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");

    server.shutdown();
}

#[test]
fn shutdown_removes_socket_file() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(&path, Arc::new(|_e: HookEvent| HookAction::Ack)).unwrap();
    assert!(path.exists());
    server.shutdown();
    assert!(!path.exists());
}

#[test]
fn drop_removes_socket_file() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    {
        let _server = HookServer::bind(&path, Arc::new(|_e: HookEvent| HookAction::Ack)).unwrap();
        assert!(path.exists());
    }
    assert!(!path.exists());
}

struct ReverseGate {
    lock: Mutex<usize>,
    cvar: Condvar,
    arrived: AtomicUsize,
}

#[test]
fn concurrent_connections_are_not_serialized() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");

    let gate = Arc::new(ReverseGate {
        lock: Mutex::new(2),
        cvar: Condvar::new(),
        arrived: AtomicUsize::new(0),
    });
    let server_gate = gate.clone();

    let server = HookServer::bind(
        &path,
        Arc::new(move |e: HookEvent| {
            let id: usize = e.tool_name.as_deref().unwrap().parse().unwrap();
            server_gate.arrived.fetch_add(1, Ordering::SeqCst);
            let mut turn = server_gate.lock.lock().unwrap();
            while *turn != id {
                turn = server_gate.cvar.wait(turn).unwrap();
            }
            *turn = turn.wrapping_sub(1);
            server_gate.cvar.notify_all();
            HookAction::Allow {
                reason: Some(id.to_string()),
            }
        }),
    )
    .unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let mut handles = Vec::new();
    for id in 0..3usize {
        let path = path.clone();
        let tx = tx.clone();
        handles.push(thread::spawn(move || {
            let bytes = serde_json::to_vec(&pretooluse_event(&id.to_string())).unwrap();
            let mut out = Vec::new();
            let code = run_client(Cursor::new(bytes), &mut out, Some(&path));
            tx.send((id, code, String::from_utf8(out).unwrap()))
                .unwrap();
        }));
    }
    drop(tx);

    let mut results = Vec::new();
    for _ in 0..3 {
        let r = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("client did not complete — server serialized connections");
        results.push(r);
    }
    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(gate.arrived.load(Ordering::SeqCst), 3);
    for (id, code, out) in results {
        assert_eq!(code, 0);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
        assert_eq!(
            v["hookSpecificOutput"]["permissionDecisionReason"],
            id.to_string()
        );
    }

    server.shutdown();
}

#[test]
fn excesso_de_conexoes_simultaneas_e_negado_sem_travar_o_servidor() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let handler_gate = gate.clone();
    let server = HookServer::bind(
        &path,
        Arc::new(move |_e: HookEvent| {
            let (lock, cvar) = &*handler_gate;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = cvar.wait(released).unwrap();
            }
            HookAction::Allow { reason: None }
        }),
    )
    .unwrap();

    let mut blocked = Vec::new();
    for _ in 0..MAX_INFLIGHT {
        let p = path.clone();
        blocked.push(thread::spawn(move || {
            run(pretooluse_event("Bash"), Some(&p))
        }));
    }
    thread::sleep(Duration::from_millis(200));

    let overflow = run(pretooluse_event("Bash"), Some(&path));
    let v: serde_json::Value = serde_json::from_str(&overflow).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");

    let (lock, cvar) = &*gate;
    *lock.lock().unwrap() = true;
    cvar.notify_all();
    for h in blocked {
        let out = h.join().unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
    }

    server.shutdown();
}
