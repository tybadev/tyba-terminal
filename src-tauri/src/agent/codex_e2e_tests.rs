//! E2E do runner Codex contra o binário `codex` real.
//!
//! Ignorado por padrão: exige `codex` no PATH e o binário do Tyba compilado
//! (o hook é o próprio executável em modo `_hook`). Trava a regressão mais
//! frágil da integração — o fingerprint de trust que o Codex exige para
//! executar um hook injetado por `-c`. Se o Codex mudar o algoritmo do hash,
//! o hook é silenciosamente ignorado e este teste falha.
//!
//! Roda sem consumir quota: `SessionStart` dispara antes da chamada ao modelo.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use crate::agent::{AgentRunner, CodexRunner, HookSetup};
use crate::hook_ipc::{HookAction, HookEvent, HookServer};

fn tyba_binary() -> std::path::PathBuf {
    std::env::current_exe()
        .expect("exe de teste")
        .parent()
        .expect("dir do exe")
        .parent()
        .expect("target/debug")
        .join("tyba")
}

#[test]
#[ignore = "exige binário codex no PATH e ./target/debug/tyba compilado"]
fn codex_real_executa_o_hook_do_tyba_e_o_session_start_chega_no_core() {
    let tyba = tyba_binary();
    assert!(
        tyba.exists(),
        "compile o binário primeiro: cargo build (esperado em {})",
        tyba.display()
    );

    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let socket = dir.path().join("h.sock");

    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let server = HookServer::bind(
        &socket,
        Arc::new(move |event: HookEvent| {
            sink.lock().unwrap().push(event.hook_event_name.clone());
            HookAction::Ack
        }),
    )
    .expect("hook server");

    let hooks = HookSetup {
        settings_path: dir.path().join("hooks.json"),
        hook_command: crate::agent::hooks_settings::hook_command(&tyba),
    };
    let mut env = HashMap::new();
    for key in ["PATH", "HOME", "TMPDIR"] {
        if let Ok(value) = std::env::var(key) {
            env.insert(key.to_string(), value);
        }
    }
    let built = CodexRunner.build_command(&repo, &env, &hooks, None);

    let argv: Vec<String> = built
        .get_argv()
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let overrides: Vec<&String> = argv
        .iter()
        .enumerate()
        .filter(|(i, _)| *i > 0 && argv[i - 1] == "--config")
        .map(|(_, a)| a)
        .collect();

    let mut cmd = Command::new("codex");
    cmd.arg("exec")
        .arg("--skip-git-repo-check")
        .arg("--sandbox")
        .arg("workspace-write");
    for over in &overrides {
        cmd.arg("--config").arg(over);
    }
    cmd.arg("diga oi");
    cmd.current_dir(&repo);
    cmd.env_clear();
    for (k, v) in &env {
        cmd.env(k, v);
    }
    cmd.env("TYBA_HOOK_SOCKET", &socket);
    cmd.stdin(std::process::Stdio::null());

    let out = cmd.output().expect("rodar codex");
    server.shutdown();

    let events = seen.lock().unwrap().clone();
    assert!(
        events.iter().any(|e| e == "SessionStart"),
        "o hook do Tyba não chegou no core — trust rejeitado ou hook ignorado.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore = "exige binário codex no PATH"]
fn tyba_hook_responde_deny_estruturado_para_pre_tool_use() {
    let tyba = tyba_binary();
    let dir = tempfile::TempDir::new().unwrap();
    let socket = dir.path().join("h.sock");

    let server = HookServer::bind(
        &socket,
        Arc::new(|_e: HookEvent| HookAction::Deny {
            reason: "git push bloqueado".into(),
        }),
    )
    .expect("hook server");

    let out = run_hook(
        &tyba,
        &socket,
        r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git push"},"cwd":"/wt"}"#,
    );
    server.shutdown();

    let v: serde_json::Value = serde_json::from_str(out.trim()).expect("json do hook");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(
        v["hookSpecificOutput"]["permissionDecisionReason"],
        "git push bloqueado"
    );
}

fn run_hook(tyba: &Path, socket: &Path, payload: &str) -> String {
    use std::io::Write;
    let mut child = Command::new(tyba)
        .arg("_hook")
        .env("TYBA_HOOK_SOCKET", socket)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn tyba _hook");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("tyba _hook");
    String::from_utf8_lossy(&out.stdout).into_owned()
}
