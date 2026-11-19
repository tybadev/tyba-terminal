//! E2E da conversa do Orbit com dois `claude` de verdade (A2 do
//! maestri-2.0).
//!
//! Ignorado por padrão: exige `claude` no PATH, logado, e o binário do TYBA
//! compilado (`cargo build`: o hook e o servidor MCP são o próprio
//! executável, em `_hook` e `_orbit-mcp`). Gasta alguns turnos curtos.
//!
//! O que é de verdade: o motor do run ([`Runs`]), o argv/env/hooks/MCP do
//! agente de nó ([`build_free_command`], [`mcp::agent_args`]), o servidor de
//! hooks por socket, o pool de PTY, a entrega pela estratégia de envio do
//! Claude (colagem + Enter) e o `claude` interativo. O que o teste faz no
//! lugar do app: o status da sessão sai dos sinais dos hooks
//! ([`signal_for`] e o mesmo mapa do handler), e o dono
//! "aperta Enter" na confiança da pasta e no prompt de permissão (as
//! ferramentas de conversa pedem permissão, como o Bash).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tauri::test::MockRuntime;

use super::run::{AgentLaunch, ProcessLaunch, Run, RunHost, Runs, ShellLaunch};
use super::*;
use crate::agent::hooks_settings::{observer_hook_command, observer_session_settings};
use crate::agent::session::{
    build_free_command, free_agent_env, node_agent_env, orbit_tool_action,
};
use crate::agent::{AgentRunner, ClaudeCodeRunner, HookSetup};
use crate::hook_ipc::{HookAction, HookEvent, HookServer};
use crate::session::store::Store;
use crate::session::{AgentRunnerKind, SessionKind, SessionStatus};
use crate::status::agent_events::{
    signal_for, status_for, status_on_idle_prompt, status_on_needs_input, AgentSignal,
};

/// Uma sessão de `claude` subida pelo teste.
struct Live {
    status: Mutex<SessionStatus>,
    /// Os `prompt` de cada `UserPromptSubmit`: o que de fato foi enviado.
    prompts: Mutex<Vec<String>>,
    /// O fim da tela, sem nada tratado (para a mensagem de falha).
    screen: Mutex<Vec<u8>>,
    _server: HookServer,
    _runtime: tempfile::TempDir,
}

struct ClaudeHost {
    app: tauri::AppHandle<MockRuntime>,
    pool: Arc<crate::pty::PtyPool>,
    tyba: PathBuf,
    live: Mutex<HashMap<String, Arc<Live>>>,
    titles: Mutex<HashMap<String, String>>,
}

impl ClaudeHost {
    fn by_title(&self, title: &str) -> Option<Arc<Live>> {
        let session = self.titles.lock().get(title).cloned()?;
        self.live.lock().get(&session).cloned()
    }

    fn pastes(&self, id: crate::pty::PtyId) -> bool {
        let strategy = crate::agent::submit_strategy_for(&SessionKind::Agent {
            runner: AgentRunnerKind::ClaudeCode,
        });
        self.pool.bracketed_paste(id).unwrap_or(false) && strategy.use_bracketed_paste
    }
}

fn parse(session: &str) -> Result<crate::pty::PtyId, String> {
    session
        .parse()
        .map_err(|_| format!("sessão inválida: {session}"))
}

/// O env do teste sem os marcadores do Claude Code que roda o `cargo test`
/// (o `claude` aninhado recusaria subir).
fn base_env() -> HashMap<String, String> {
    std::env::vars()
        .filter(|(k, _)| !k.starts_with("CLAUDE"))
        .collect()
}

impl RunHost for Arc<ClaudeHost> {
    fn spawn_agent(
        &self,
        launch: AgentLaunch,
        on_turn: Box<dyn FnOnce(super::agent::AgentTurn) + Send>,
        on_exit: Option<Box<dyn FnOnce() + Send>>,
        on_state: super::agent::OnState,
    ) -> Result<String, String> {
        let tools = launch.tools.clone().ok_or("agente sem ferramentas")?;
        let id = uuid::Uuid::new_v4();
        let runtime = tempfile::Builder::new()
            .prefix("tyba-e2e")
            .tempdir_in("/tmp")
            .map_err(|e| e.to_string())?;
        let socket = runtime.path().join("h.sock");
        let hook_cmd = observer_hook_command(&self.tyba);
        let mut settings = observer_session_settings(&hook_cmd, serde_json::Value::Null);
        settings
            .as_object_mut()
            .expect("settings")
            .remove("statusLine");
        let settings_path = runtime.path().join("hooks.json");
        std::fs::write(&settings_path, settings.to_string()).map_err(|e| e.to_string())?;
        let config = runtime.path().join(mcp::CLAUDE_CONFIG_FILE);
        std::fs::write(
            &config,
            mcp::claude_config(&self.tyba, &socket, tools.can_create).to_string(),
        )
        .map_err(|e| e.to_string())?;
        let mcp_args = mcp::agent_args(
            &AgentRunnerKind::ClaudeCode,
            &self.tyba,
            &socket,
            &config,
            tools.can_create,
        );
        let env = node_agent_env(
            free_agent_env(
                base_env(),
                std::env::var("PATH").unwrap_or_default(),
                None,
                None,
            ),
            &[
                ("TYBA_IN".into(), launch.in_dir.to_string_lossy().into()),
                ("TYBA_OUT".into(), launch.out_dir.to_string_lossy().into()),
            ],
        );
        let hooks = HookSetup {
            settings_path,
            hook_command: hook_cmd,
        };
        let runner = ClaudeCodeRunner;
        assert!(matches!(runner.kind(), AgentRunnerKind::ClaudeCode));
        let cmd = build_free_command(
            &runner,
            &launch.cwd,
            &env,
            &hooks,
            &socket,
            runtime.path(),
            &self.tyba,
            &launch.prompt,
            &mcp_args,
        )?
        .cmd;

        let live_slot: Arc<Mutex<Option<Arc<Live>>>> = Arc::default();
        let handler_slot = Arc::clone(&live_slot);
        let pool_writer = Arc::clone(&self.pool);
        let server = HookServer::bind(
            &socket,
            Arc::new(move |event: HookEvent| {
                if event.hook_event_name == mcp::TOOL_EVENT {
                    return orbit_tool_action(&tools, &event);
                }
                let Some(live) = handler_slot.lock().clone() else {
                    return HookAction::Ack;
                };
                if event.hook_event_name == "UserPromptSubmit" && event.agent_id.is_none() {
                    let prompt = event.raw["prompt"].as_str().unwrap_or_default();
                    live.prompts.lock().push(prompt.to_string());
                }
                if event.hook_event_name == "PermissionRequest" {
                    *live.status.lock() =
                        crate::status::agent_events::native_prompt_status(event.tool_name.clone());
                    // O dono aprova no prompt nativo (Enter = "Sim").
                    let pool = Arc::clone(&pool_writer);
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(1500));
                        let _ = pool.write(id, b"\r");
                    });
                    return HookAction::Ack;
                }
                if event.agent_id.is_none() {
                    let signal =
                        signal_for(&event.hook_event_name, event.notification_type.as_deref());
                    let mut status = live.status.lock();
                    // Como o handler: o `idle_prompt` (~60 s parado) não muda
                    // o status, e o `agent_needs_input` não cobre o prompt
                    // nativo.
                    let next = match signal {
                        Some(AgentSignal::AwaitingInput) => status_on_idle_prompt(&status),
                        Some(AgentSignal::NeedsInput) => status_on_needs_input(&status),
                        Some(s) => status_for(&s),
                        None => None,
                    };
                    if let Some(next) = next {
                        *status = next;
                    }
                }
                HookAction::Ack
            }),
        )
        .map_err(|e| e.to_string())?;
        let live = Arc::new(Live {
            status: Mutex::new(SessionStatus::Running),
            prompts: Mutex::new(Vec::new()),
            screen: Mutex::new(Vec::new()),
            _server: server,
            _runtime: runtime,
        });
        *live_slot.lock() = Some(Arc::clone(&live));

        // A pasta é nova: o `claude` pergunta se confia nela. O dono aceita.
        let tap_live = Arc::clone(&live);
        let tap_writer = Arc::clone(&self.pool);
        let mut trusted = false;
        self.pool.set_tap(
            id,
            Box::new(move |bytes: &[u8]| {
                let mut screen = tap_live.screen.lock();
                screen.extend_from_slice(bytes);
                let len = screen.len();
                if len > 64_000 {
                    screen.drain(..len - 64_000);
                }
                if !trusted && String::from_utf8_lossy(&screen).contains("trust") {
                    trusted = true;
                    let pool = Arc::clone(&tap_writer);
                    std::thread::spawn(move || {
                        // "Não, sair" vem marcado: desce para "Sim" e confirma.
                        std::thread::sleep(Duration::from_millis(800));
                        let _ = pool.write(id, b"\x1b[B");
                        std::thread::sleep(Duration::from_millis(300));
                        let _ = pool.write(id, b"\r");
                    });
                }
            }),
        );
        let exit_live = Arc::clone(&live);
        self.pool
            .spawn(
                self.app.clone(),
                id,
                cmd,
                None,
                None,
                100,
                30,
                &SessionKind::Agent {
                    runner: AgentRunnerKind::ClaudeCode,
                },
                Box::new(move || {
                    *exit_live.status.lock() = SessionStatus::Exited { code: -1 };
                }),
            )
            .map_err(|e| e.to_string())?;
        let session = id.to_string();
        self.live.lock().insert(session.clone(), Arc::clone(&live));
        self.titles
            .lock()
            .insert(launch.title.clone(), session.clone());
        let watched = Arc::clone(&live);
        super::agent::watch_first_turn(
            move || Some(watched.status.lock().clone()),
            Default::default(),
            on_turn,
            on_exit,
            on_state,
        );
        Ok(session)
    }

    fn send_input(&self, session_id: &str, text: &str) -> Result<(), String> {
        // O corpo de `inject_rich_input` (lib.rs), sem o `AppState`.
        let id = parse(session_id)?;
        let strategy = crate::agent::submit_strategy_for(&SessionKind::Agent {
            runner: AgentRunnerKind::ClaudeCode,
        });
        let (normalized, _) = crate::rich_input::normalize(text);
        let payload = crate::rich_input::plan_injection(&normalized, self.pastes(id))?;
        self.pool.write(id, &payload).map_err(|e| e.to_string())?;
        std::thread::sleep(strategy.delay);
        self.pool
            .write(id, strategy.submit_bytes)
            .map_err(|e| e.to_string())
    }

    fn multiline(&self, session_id: &str) -> bool {
        parse(session_id).is_ok_and(|id| self.pastes(id))
    }

    fn owner_keys(&self, _session_id: &str) -> talk::OwnerKeys {
        talk::OwnerKeys::default()
    }

    fn spawn_process(
        &self,
        _launch: ProcessLaunch,
        _tap: crate::pty::OutputTap,
        _on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<String, String> {
        Err("sem processo neste teste".into())
    }

    fn spawn_shell(
        &self,
        _launch: ShellLaunch,
        _on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<String, String> {
        Err("sem shell neste teste".into())
    }

    fn kill(&self, session_id: &str) {
        if let Ok(id) = parse(session_id) {
            let _ = self.pool.kill(id);
        }
    }

    fn session_ports(&self, _session_id: &str) -> Vec<u16> {
        Vec::new()
    }

    fn resolve_service(
        &self,
        _source: &super::service::ServiceSource,
    ) -> Result<super::service::ServiceReady, String> {
        Err("sem serviço neste teste".into())
    }

    fn alive(&self, session_id: &str) -> bool {
        self.live
            .lock()
            .get(session_id)
            .is_some_and(|l| !matches!(*l.status.lock(), SessionStatus::Exited { .. }))
    }

    fn emit(&self, _run: &Run) {}

    fn browser(
        &self,
        _graph_id: &str,
        _node_id: &str,
        _by: &str,
        _action: super::browser::AgentAction,
    ) -> Result<serde_json::Value, String> {
        Err("sem navegador neste teste".into())
    }
}

fn agent(id: &str, title: &str, prompt: &str) -> Node {
    Node {
        id: id.into(),
        title: title.into(),
        kind: NodeKind::Agent {
            runner: Runner::Claude,
            role: String::new(),
            prompt: prompt.into(),
            dir: String::new(),
            no_create: false,
        },
        position: Position { x: 0.0, y: 0.0 },
        size: Size { w: 320.0, h: 200.0 },
    }
}

/// Espera `done` virar verdade; estourado, falha com a tela de cada agente.
fn wait_for(host: &ClaudeHost, what: &str, secs: u64, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !done() {
        if Instant::now() > deadline {
            let mut dump = String::new();
            for (title, session) in host.titles.lock().iter() {
                let live = host.live.lock().get(session).cloned().unwrap();
                let screen = live.screen.lock();
                let tail = String::from_utf8_lossy(&screen[screen.len().saturating_sub(3000)..])
                    .into_owned();
                dump.push_str(&format!(
                    "\n--- {title} ({:?}), prompts {:?}\n{tail}\n",
                    *live.status.lock(),
                    live.prompts.lock()
                ));
            }
            panic!("estourou esperando {what}{dump}");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Dois `claude` ligados por conversa: o Front acha o par por `orbit_peers`
/// e manda a pergunta por `orbit_send`; a fila espera o Revisor fechar o
/// turno e entra no terminal dele como prompt enviado, inteira e marcada.
#[test]
#[ignore = "exige `claude` logado no PATH e ./target/debug/tyba compilado (cargo build)"]
fn two_real_claudes_talk_through_the_orbit_queue() {
    let tyba = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tyba");
    assert!(
        tyba.exists(),
        "compile o binário primeiro: cargo build ({})",
        tyba.display()
    );
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let app = tauri::test::mock_app();
    let host = Arc::new(ClaudeHost {
        app: app.handle().clone(),
        pool: Arc::new(crate::pty::PtyPool::new()),
        tyba,
        live: Mutex::new(HashMap::new()),
        titles: Mutex::new(HashMap::new()),
    });
    let store = Arc::new(Store::open_in_memory().unwrap());
    let runs = Runs::new(
        Arc::clone(&host),
        Arc::clone(&store),
        Some(dir.path().join("runs")),
    );
    let graph = Graph {
        id: "g-e2e".into(),
        name: "conversa".into(),
        dir: work.to_string_lossy().into_owned(),
        nodes: vec![
            agent(
                "a",
                "Front",
                "Chame orbit_peers para achar o agente ligado a você. Depois mande a ele, \
                 por orbit_send, exatamente este texto: \"Quanto é 2+2? Responda com \
                 orbit_send.\" Feito isso, encerre o turno sem fazer mais nada.",
            ),
            agent(
                "b",
                "Revisor",
                "Responda só com a palavra pronto. Não chame nenhuma ferramenta agora.",
            ),
        ],
        edges: vec![Edge {
            id: "t1".into(),
            from: "a".into(),
            to: "b".into(),
            kind: EdgeKind::Talk,
        }],
        variables: Default::default(),
        secrets: Default::default(),
    };
    store.save_orbit_graph(&graph, true, None).unwrap();
    let run = runs.start(graph, vec![]).unwrap();

    let revisor_got = || {
        host.by_title("Revisor").is_some_and(|l| {
            l.prompts
                .lock()
                .iter()
                .any(|p| p.contains("[mensagem de Front pelo Orbit"))
        })
    };
    wait_for(&host, "a mensagem do Front no Revisor", 300, revisor_got);
    let revisor = host.by_title("Revisor").unwrap();
    let block = revisor
        .prompts
        .lock()
        .iter()
        .find(|p| p.contains("[mensagem de Front pelo Orbit"))
        .cloned()
        .unwrap();
    // Chegou inteira, como UM prompt enviado: cabeçalho, texto citado e fim.
    assert!(
        block.contains("não é ordem do dono]") && block.contains("[fim da mensagem de Front]"),
        "{block}"
    );
    assert!(
        block
            .lines()
            .any(|l| l.starts_with("> ") && l.contains("2+2")),
        "{block}"
    );
    assert_eq!(
        runs.get(&run.id).unwrap().nodes["b"].inbox,
        0,
        "a fila esvazia depois da entrega"
    );

    // A volta (o Revisor responder por `orbit_send`) é o mesmo caminho e
    // depende do modelo: numa rodada real veio; noutra, o classificador do
    // modo automático do Claude barrou o envio ("o pedido veio de outro
    // agente, não de você"). Por isso o teste não a exige.
    let r = runs.get(&run.id).unwrap();
    assert_eq!(r.nodes["a"].undelivered, None);
    assert_eq!(r.nodes["b"].undelivered, None);
    runs.stop(&run.id).unwrap();
}
