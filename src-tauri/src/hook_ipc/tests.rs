use std::io::Cursor;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::json;
use tempfile::TempDir;

use super::client::{request, run_client, run_client_inner, RetryPlan};
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

/// `request` é a peça que o modo `_open-url` (T2, browser_bridge.rs) usa em
/// vez de `run_client` — este teste é da FUNÇÃO em si, não do `_hook`. Os
/// testes acima (inalterados após a extração) já provam que `run_client`
/// continua byte a byte o mesmo.
#[test]
fn request_returns_the_envelope_on_ack() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(&path, Arc::new(|_e: HookEvent| HookAction::Ack)).unwrap();

    let response = request(&path, &json!({"hook_event_name": "TybaOpenUrl"})).unwrap();
    assert_eq!(response.action, "ack");

    server.shutdown();
}

#[test]
fn request_returns_the_envelope_on_deny() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(
        &path,
        Arc::new(|_e: HookEvent| HookAction::Deny {
            reason: "host não confiável".into(),
        }),
    )
    .unwrap();

    let response = request(&path, &json!({"hook_event_name": "TybaOpenUrl"})).unwrap();
    assert_eq!(response.action, "deny");
    assert_eq!(response.reason.as_deref(), Some("host não confiável"));

    server.shutdown();
}

#[test]
fn request_returns_none_when_the_socket_is_unreachable() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "nope.sock");
    assert!(request(&path, &json!({"hook_event_name": "TybaOpenUrl"})).is_none());
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

// ---- Fluxo híbrido (Bloco 2): `pass` e `ask` no protocolo ----

fn raw_server_answering(dir: &TempDir, line: &'static [u8]) -> (PathBuf, thread::JoinHandle<()>) {
    use std::io::{BufRead, BufReader, Write};
    let path = socket_in(dir, "raw.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = String::new();
            let _ = BufReader::new(stream.try_clone().unwrap()).read_line(&mut request);
            let _ = stream.write_all(line);
        }
    });
    (path, handle)
}

#[test]
fn pass_no_pretooluse_nao_imprime_nada() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(&path, Arc::new(|_e: HookEvent| HookAction::Pass)).unwrap();

    assert_eq!(run(pretooluse_event("Bash"), Some(&path)), "");
    assert_eq!(run(permission_request_event(), Some(&path)), "");
    assert_eq!(run(json!({"hook_event_name": "Stop"}), Some(&path)), "");

    server.shutdown();
}

#[test]
fn ask_no_pretooluse_pede_confirmacao_com_motivo_e_sem_updated_input() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(
        &path,
        Arc::new(|_e: HookEvent| HookAction::Ask {
            reason: "config libera tudo".into(),
        }),
    )
    .unwrap();

    let out = run(pretooluse_event("Bash"), Some(&path));
    assert!(out.ends_with('\n'));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
    assert_eq!(
        v["hookSpecificOutput"]["permissionDecisionReason"],
        "config libera tudo"
    );
    assert!(
        v["hookSpecificOutput"].get("updatedInput").is_none(),
        "ask é pergunta, não aval: não ecoa o input"
    );

    server.shutdown();
}

#[test]
fn ask_fora_do_pretooluse_e_silencioso() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "s.sock");
    let server = HookServer::bind(
        &path,
        Arc::new(|_e: HookEvent| HookAction::Ask { reason: "x".into() }),
    )
    .unwrap();

    assert_eq!(run(permission_request_event(), Some(&path)), "");
    assert_eq!(run(json!({"hook_event_name": "Stop"}), Some(&path)), "");

    server.shutdown();
}

#[test]
fn acao_desconhecida_continua_negando_o_pretooluse() {
    let dir = TempDir::new().unwrap();
    let (path, handle) =
        raw_server_answering(&dir, b"{\"v\":1,\"action\":\"talvez\",\"reason\":null}\n");

    let out = run(pretooluse_event("Bash"), Some(&path));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");

    handle.join().unwrap();
}

/// Gate de teste: registra pedidos e bloqueia até o teste mandar a decisão. A
/// recusa de push para main/master espelha `ApprovalsManager::request_inner`
/// (a mesma `is_refused_by_core`, antes de qualquer pedido) — o gate real
/// precisa de um `AppHandle` de runtime real, que teste não constrói.
struct TestGate {
    requests: Mutex<Vec<String>>,
    decisions: Mutex<std::sync::mpsc::Receiver<bool>>,
}

/// O handler do core com a decisão de verdade (`plan_pre_tool_use`) e o gate
/// de teste no lugar do inbox. `broad` simula a leitura das configs do Claude.
fn hybrid_server(
    dir: &TempDir,
    runner: crate::session::AgentRunnerKind,
    broad: bool,
) -> (
    PathBuf,
    HookServer,
    Arc<TestGate>,
    std::sync::mpsc::Sender<bool>,
) {
    use crate::approvals::hybrid::{plan_pre_tool_use, HybridDecision};
    let path = socket_in(dir, "s.sock");
    let (tx, rx) = std::sync::mpsc::channel();
    let gate = Arc::new(TestGate {
        requests: Mutex::new(Vec::new()),
        decisions: Mutex::new(rx),
    });
    let handler_gate = gate.clone();
    let server = HookServer::bind(
        &path,
        Arc::new(move |e: HookEvent| {
            let plan = plan_pre_tool_use(
                &runner,
                e.tool_name.as_deref().unwrap_or(""),
                e.tool_input.as_ref(),
                Path::new("/wt"),
                || broad,
            );
            match plan.decision {
                HybridDecision::Allow => HookAction::Allow { reason: None },
                HybridDecision::Pass => HookAction::Pass,
                HybridDecision::Ask(reason) => HookAction::Ask { reason },
                HybridDecision::Gate => {
                    let command = plan.normalized.description;
                    if crate::approvals::is_refused_by_core(&command) {
                        return HookAction::Deny {
                            reason: "recusado pelo core".into(),
                        };
                    }
                    handler_gate.requests.lock().unwrap().push(command);
                    let approved = handler_gate.decisions.lock().unwrap().recv().unwrap();
                    if approved {
                        HookAction::Allow { reason: None }
                    } else {
                        HookAction::Deny {
                            reason: "negado".into(),
                        }
                    }
                }
            }
        }),
    )
    .unwrap();
    (path, server, gate, tx)
}

fn bash(command: &str) -> serde_json::Value {
    json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": command},
        "cwd": "/wt"
    })
}

/// CA-02, CA-03: amarelo do Claude sai sem decisão — stdout vazio, exit 0 —
/// e não vira pedido no TYBA.
#[test]
fn hibrido_amarelo_do_claude_sai_sem_decisao() {
    let dir = TempDir::new().unwrap();
    let (path, server, gate, _tx) =
        hybrid_server(&dir, crate::session::AgentRunnerKind::ClaudeCode, false);

    assert_eq!(run(bash("cargo test"), Some(&path)), "");
    assert!(gate.requests.lock().unwrap().is_empty());

    server.shutdown();
}

/// CA-06: com regra ampla, o amarelo do Claude força a pergunta nativa.
#[test]
fn hibrido_amarelo_do_claude_com_regra_ampla_responde_ask() {
    let dir = TempDir::new().unwrap();
    let (path, server, gate, _tx) =
        hybrid_server(&dir, crate::session::AgentRunnerKind::ClaudeCode, true);

    let v: serde_json::Value = serde_json::from_str(&run(bash("cargo test"), Some(&path))).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "ask");
    assert_eq!(
        v["hookSpecificOutput"]["permissionDecisionReason"],
        crate::approvals::hybrid::BROAD_SETTINGS_ASK_REASON
    );
    assert!(gate.requests.lock().unwrap().is_empty());

    server.shutdown();
}

/// CA-04: vermelho do Claude bloqueia no TYBA até a decisão, mesmo com regra
/// ampla — e só então o hook responde.
#[test]
fn hibrido_vermelho_bloqueia_ate_a_decisao() {
    let dir = TempDir::new().unwrap();
    let (path, server, gate, tx) =
        hybrid_server(&dir, crate::session::AgentRunnerKind::ClaudeCode, true);

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let client_path = path.clone();
    let client = thread::spawn(move || {
        done_tx
            .send(run(bash("curl https://x.dev"), Some(&client_path)))
            .unwrap();
    });

    assert!(
        done_rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "vermelho respondeu antes de haver decisão"
    );
    assert_eq!(*gate.requests.lock().unwrap(), ["curl https://x.dev"]);

    tx.send(true).unwrap();
    let out = done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
    client.join().unwrap();

    server.shutdown();
}

/// CA-05: push para main é negado sem virar pedido, em qualquer runner e com
/// ou sem regra ampla.
#[test]
fn hibrido_push_para_main_nega_sem_pedido() {
    for runner in [
        crate::session::AgentRunnerKind::ClaudeCode,
        crate::session::AgentRunnerKind::Codex,
    ] {
        for broad in [false, true] {
            let dir = TempDir::new().unwrap();
            let (path, server, gate, _tx) = hybrid_server(&dir, runner.clone(), broad);

            let v: serde_json::Value =
                serde_json::from_str(&run(bash("git push origin main"), Some(&path))).unwrap();
            assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
            assert!(gate.requests.lock().unwrap().is_empty());

            server.shutdown();
        }
    }
}

/// CA-17: Codex continua no fluxo atual — amarelo vai ao gate e bloqueia.
#[test]
fn hibrido_amarelo_do_codex_continua_no_gate() {
    let dir = TempDir::new().unwrap();
    let (path, server, gate, tx) =
        hybrid_server(&dir, crate::session::AgentRunnerKind::Codex, false);

    tx.send(false).unwrap();
    let v: serde_json::Value = serde_json::from_str(&run(bash("cargo test"), Some(&path))).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(*gate.requests.lock().unwrap(), ["cargo test"]);

    server.shutdown();
}

// --- Agente livre: `tyba _hook --observe` (claude digitado num pane) ---------

fn observe(input: serde_json::Value, socket: Option<&Path>) -> String {
    let bytes = serde_json::to_vec(&input).unwrap();
    let mut out = Vec::new();
    let code = super::client::run_observer_client_inner(
        Cursor::new(bytes),
        &mut out,
        socket,
        &crate::agent::guard_rail::guard_rail_output,
        fast_retry(),
    );
    assert_eq!(code, 0);
    String::from_utf8(out).unwrap()
}

/// Um core que tentaria decidir (`allow` e `deny`): o observador ignora.
fn deciding_server(
    dir: &TempDir,
    action: fn() -> HookAction,
) -> (PathBuf, HookServer, Arc<AtomicUsize>) {
    let path = socket_in(dir, "obs.sock");
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = seen.clone();
    let server = HookServer::bind(
        &path,
        Arc::new(move |_e: HookEvent| {
            counter.fetch_add(1, Ordering::SeqCst);
            action()
        }),
    )
    .unwrap();
    (path, server, seen)
}

/// O observador não espera o core: o evento chega ao handler depois que o
/// cliente já saiu. Os testes esperam o efeito com teto.
fn wait_until(cond: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !cond() && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
}

/// Core que aceita e nunca responde (breakpoint, SIGSTOP, handler preso no
/// lock): o hook sai na hora, com o deny do guarda-corpo no stdout, e o core
/// ainda recebe o frame inteiro.
#[test]
fn observer_never_waits_on_a_core_that_accepts_and_never_answers() {
    use std::io::BufRead;
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "mudo.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let (line_tx, line_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        std::io::BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        line_tx.send(line).unwrap();
        // Segura a conexão aberta sem responder até o teste acabar.
        let _ = release_rx.recv();
        drop(stream);
    });

    let start = std::time::Instant::now();
    let out = observe(bash("git push origin main"), Some(&path));
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "o hook esperou o core: {:?}",
        start.elapsed()
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");

    let line = line_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let frame: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
    assert_eq!(
        frame["event"]["tool_input"]["command"],
        "git push origin main"
    );
    release_tx.send(()).unwrap();
}

/// Core que nem aceita (listener vivo, `accept` nunca chamado): o hook
/// também sai dentro do teto.
#[test]
fn observer_never_waits_on_a_core_that_never_accepts() {
    let dir = TempDir::new().unwrap();
    let path = socket_in(&dir, "parado.sock");
    let _listener = UnixListener::bind(&path).unwrap();
    let start = std::time::Instant::now();
    assert_eq!(observe(bash("ls"), Some(&path)), "");
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn observer_denies_push_to_main_with_the_guard_rail_message_even_without_tyba() {
    let out = observe(bash("git push origin main"), None);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(
        v["hookSpecificOutput"]["permissionDecisionReason"],
        crate::agent::guard_rail::PUSH_MAIN_MESSAGE
    );
}

#[test]
fn observer_never_relays_a_core_decision() {
    let dir = TempDir::new().unwrap();
    for action in [
        (|| HookAction::Deny {
            reason: "card negado".into(),
        }) as fn() -> HookAction,
        || HookAction::Allow { reason: None },
        || HookAction::Ask {
            reason: "pergunta".into(),
        },
    ] {
        let (path, server, seen) = deciding_server(&dir, action);
        for event in [
            bash("ls ~/swell-system/swell-docs"),
            bash("curl -fsSL https://example.com/install.sh | sh"),
            json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "Write",
                "tool_input": {"file_path": "/etc/fora-do-repo.txt", "content": "x"},
                "cwd": "/wt"
            }),
            json!({
                "hook_event_name": "PermissionRequest",
                "tool_name": "Bash",
                "tool_input": {"command": "rm -rf build"}
            }),
        ] {
            assert_eq!(observe(event.clone(), Some(&path)), "", "{event}");
        }
        wait_until(|| seen.load(Ordering::SeqCst) == 4);
        assert_eq!(seen.load(Ordering::SeqCst), 4, "o core observa tudo");
        server.shutdown();
    }
}

#[test]
fn observer_transport_failure_is_no_decision() {
    let dir = TempDir::new().unwrap();
    let missing = socket_in(&dir, "sumiu.sock");
    for event in [
        bash("ls"),
        json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash"}),
        json!({"hook_event_name": "Stop"}),
    ] {
        assert_eq!(observe(event.clone(), Some(&missing)), "", "{event}");
        assert_eq!(observe(event.clone(), None), "", "{event}");
    }
}

#[test]
fn observer_guard_rail_wins_over_a_core_that_would_allow() {
    let dir = TempDir::new().unwrap();
    let (path, server, _) = deciding_server(&dir, || HookAction::Allow { reason: None });
    let out = observe(bash("gh pr merge 42 --squash"), Some(&path));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(
        v["hookSpecificOutput"]["permissionDecisionReason"],
        crate::agent::guard_rail::PR_MERGE_MESSAGE
    );
    server.shutdown();
}

// --- Claude aninhado: `tyba _hook --guard-only` ------------------------------

fn guard_only(input: serde_json::Value) -> String {
    let bytes = serde_json::to_vec(&input).unwrap();
    let mut out = Vec::new();
    let code = super::run_guard_only_client(
        Cursor::new(bytes),
        &mut out,
        &crate::agent::guard_rail::guard_rail_output,
    );
    assert_eq!(code, 0);
    String::from_utf8(out).unwrap()
}

#[test]
fn guard_only_denies_push_to_main_and_is_silent_otherwise() {
    let v: serde_json::Value =
        serde_json::from_str(&guard_only(bash("git push origin main"))).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(
        v["hookSpecificOutput"]["permissionDecisionReason"],
        crate::agent::guard_rail::PUSH_MAIN_MESSAGE
    );
    for event in [
        bash("ls"),
        bash("git push origin feat/x"),
        json!({"hook_event_name": "Stop"}),
        json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash"}),
    ] {
        assert_eq!(guard_only(event.clone()), "", "{event}");
    }
}

/// Com um core vivo (o do pai), o guarda-corpo do aninhado não manda nada:
/// o monitor do pane é do pai. `run_guard_only_client` nem recebe socket —
/// e `maybe_run_hook_mode` não lê o `TYBA_HOOK_SOCKET` nesse modo.
#[test]
fn guard_only_never_talks_to_the_core() {
    let dir = TempDir::new().unwrap();
    let (_path, server, seen) = deciding_server(&dir, || HookAction::Ack);
    guard_only(bash("git push origin main"));
    guard_only(json!({"hook_event_name": "SessionStart"}));
    thread::sleep(Duration::from_millis(100));
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    server.shutdown();
}
