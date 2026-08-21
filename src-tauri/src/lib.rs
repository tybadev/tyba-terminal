pub mod agent;
pub mod approvals;
pub mod blocks;
pub mod boot;
pub mod completion;
pub mod docker;
pub mod editor;
pub mod error;
pub mod files;
pub mod forge;
pub mod history;
pub mod hook_ipc;
pub mod launch_config;
pub mod layout;
pub mod lsp;
pub mod menu;
pub mod pty;
pub mod repo;
pub mod repo_config;
pub mod rich_input;
pub mod sandbox;
pub mod session;
pub mod shell_path;
pub mod snippet;
pub mod ssh;
pub mod status;
pub mod theme;
pub mod update;
pub mod user_config;
pub mod worktree;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use tauri::{AppHandle, Emitter, Listener, Manager, State};

use approvals::{ApprovalRequest, Decision, SharedApprovals};
use error::AppError;
use pty::SharedPtyPool;
use session::store::Store;
use session::{
    CreateSessionOpts, Session, SessionId, SessionKind, SessionStatus, SharedSessionManager,
};

struct AppState {
    store: Arc<Store>,
    pty_pool: SharedPtyPool,
    sessions: SharedSessionManager,
    approvals: SharedApprovals,
    themes: theme::SharedThemes,
    layout: layout::SharedLayout,
    docker: docker::SharedDocker,
    repos: repo::SharedRepoWatcher,
    files: files::SharedFiles,
    remote_files: files::remote::SharedRemoteFiles,
    lsp: lsp::SharedLsp,
    managed_lsp: lsp::managed::SharedManaged,
    repo_reconcile: std::sync::mpsc::Sender<()>,
    rich_input_submit: parking_lot::Mutex<()>,
    worktree_files: rich_input::FilesCache,
    hook_servers: Arc<agent::session::HookServerRegistry>,
    subagents: agent::subagents::SharedSubagents,
    agent_prober: agent::process_probe::SharedAgentProber,
    disk_observer: agent::disk_observer::SharedDiskObserver,
    tunnel_states: crate::ssh::tunnel::SharedTunnelStates,
    /// Fecha em `false` e abre quando a thread de boot termina. Ver [`boot`].
    boot: Arc<boot::BootGate>,
}

/// Resposta que separa "carregou e está vazio" de "ainda não carregou".
///
/// Sem isto a UI leria a lista vazia do meio do boot como estado final e
/// desenharia "nenhuma sessão" por cima de vinte sessões que estão voltando.
#[derive(serde::Serialize)]
struct Loaded<T> {
    ready: bool,
    value: T,
}

impl<T> Loaded<T> {
    /// `ready` é lido **antes** do valor, e a ordem é a decisão. Se o boot
    /// terminar entre as duas leituras, o pior caso aqui é anunciar `false` com
    /// dado que já era bom — o front reconsulta e nada se perde. Na ordem
    /// inversa o pior caso seria anunciar `true` carregando o estado de antes do
    /// boot, e aí a UI trata como final o que era transitório.
    fn read(state: &AppState, value: impl FnOnce() -> T) -> Self {
        let ready = state.boot.is_ready();
        Self {
            ready,
            value: value(),
        }
    }
}

/// Teto para quem escreve antes do boot terminar. Não é um tempo esperado: é o
/// limite acima do qual preferimos agir com estado incompleto a deixar o clique
/// do usuário pendurado.
const BOOT_WAIT: Duration = Duration::from_secs(10);

fn watched_repo_roots(state: &AppState) -> std::collections::HashSet<std::path::PathBuf> {
    let mut roots: std::collections::HashSet<std::path::PathBuf> = state
        .layout
        .state()
        .workspaces
        .iter()
        .filter_map(|w| w.repo_root.as_deref())
        .map(|root| session::expand_home(std::path::Path::new(root)))
        .filter_map(|root| repo::toplevel(&root))
        .collect();

    for session in state.sessions.list() {
        if !matches!(session.status, SessionStatus::Running) {
            continue;
        }
        let Some(pid) = state.pty_pool.leader_pid(session.id) else {
            continue;
        };
        let Some(cwd) = repo::process_cwd(pid) else {
            continue;
        };
        if let Some(root) = repo::toplevel(&cwd) {
            roots.insert(root);
        }
    }
    roots
}

fn reconcile_repo_watchers(app: &AppHandle) {
    let state = app.state::<AppState>();
    let roots = watched_repo_roots(&state);
    state.repos.set_roots(app, roots);
    let _ = app.emit(repo::EVENT_RECONCILED, state.repos.snapshots());
}

/// Um tick do poll de detecção de agente: monta a lista de shells vivos com seu
/// `leader_pid`, sonda a árvore de processos e emite só as sessões que mudaram.
/// Quando não há shell aberto, nem varre os processos — custo zero (só um par de
/// locks curtos), como manda o performance-first.
fn poll_agent_probers(app: &AppHandle) {
    let state = app.state::<AppState>();
    let shells: Vec<(SessionId, u32)> = state
        .sessions
        .shell_ids()
        .into_iter()
        .filter_map(|id| state.pty_pool.leader_pid(id).map(|pid| (id, pid)))
        .collect();
    let rows = if shells.is_empty() {
        Vec::new()
    } else {
        agent::process_probe::snapshot()
    };
    let changes = state.agent_prober.reconcile(&shells, &rows);
    let leader_by_session: std::collections::HashMap<SessionId, u32> =
        shells.iter().copied().collect();
    let changed: std::collections::HashSet<SessionId> = changes.iter().map(|(id, _)| *id).collect();
    for (session_id, detected) in &changes {
        drive_disk_observer(
            app,
            &state,
            &leader_by_session,
            *session_id,
            detected.as_ref(),
        );
    }
    // Detecção estável não re-emite: uma sessão cujo transcript ainda não existia
    // na transição da F1 (janela de startup do claude) jamais ganharia observer.
    // Re-tenta a cada poll enquanto o agente segue detectado e o bind não é
    // forte — sem thread viva OU bind provisório (melhor-esforço por mtime),
    // que re-resolve até aparecer candidato datado pós-agente.
    for session_id in leader_by_session.keys().copied() {
        if changed.contains(&session_id) || state.disk_observer.is_settled(session_id) {
            continue;
        }
        if let Some(detected) = state.agent_prober.detected(session_id) {
            drive_disk_observer(app, &state, &leader_by_session, session_id, Some(&detected));
        }
    }
    let live: std::collections::HashSet<SessionId> = leader_by_session.keys().copied().collect();
    state
        .disk_observer
        .retain_live(app, &state.subagents, &live);
    for (session_id, detected) in changes {
        let _ = app.emit(
            agent::process_probe::EVENT_CHANGED,
            agent::process_probe::AgentDetectedPayload {
                session_id,
                detected,
            },
        );
    }
}

/// Liga a detecção de processo da F1 ao disk observer da F2. Claude cru num shell
/// ganha captura de subagentes por disco; agente sumido congela o painel. Só
/// Claude Code por ora (Codex disk-driven fica para depois). O cwd do transcript
/// é o do próprio processo do agente, com o líder do shell como reserva.
fn drive_disk_observer(
    app: &AppHandle,
    state: &AppState,
    leader_by_session: &std::collections::HashMap<SessionId, u32>,
    session_id: SessionId,
    detected: Option<&agent::process_probe::DetectedAgent>,
) {
    match detected {
        Some(agent) if matches!(agent.kind, crate::session::AgentRunnerKind::ClaudeCode) => {
            let cwd = repo::process_cwd(agent.pid).or_else(|| {
                leader_by_session
                    .get(&session_id)
                    .and_then(|pid| repo::process_cwd(*pid))
            });
            if let Some(cwd) = cwd {
                state.disk_observer.observe(
                    app,
                    &state.subagents,
                    session_id,
                    &cwd,
                    agent.start_ms,
                );
            }
        }
        _ => state.disk_observer.stop(app, &state.subagents, session_id),
    }
}

fn emit_layout(app: &AppHandle, state: &State<'_, AppState>) {
    let _ = app.emit(layout::EVENT_CHANGED, state.layout.state());
    let _ = state.repo_reconcile.send(());
}

fn dispose_shells(state: &State<'_, AppState>, ids: &[SessionId]) {
    for id in ids {
        if let Some(s) = state.sessions.get(*id) {
            if matches!(s.kind, SessionKind::Shell | SessionKind::Ssh { .. }) {
                end_ssh_session_on_host(state, &s);
                state.sessions.dispose(&state.pty_pool, *id);
            }
        }
    }
}

pub(crate) fn coordinate_subagent_viewer(
    app: &AppHandle,
    session: SessionId,
    coordination: agent::subagents::Coordination,
) {
    let state = app.state::<AppState>();
    let mut changed = false;
    if !coordination.viewer_disarmed {
        changed |= state.layout.ensure_agent_viewer(session);
    }
    if !coordination.panel_disarmed {
        changed |= state.layout.ensure_agents_panel(session);
    }
    if changed {
        state.subagents.mark_coordinated(session);
        emit_layout(app, &state);
    }
}

fn gc_host(state: &State<'_, AppState>, alias: &str) {
    let Ok(install) = crate::ssh::tmux::install_id(&state.store) else {
        return;
    };
    let known: std::collections::HashSet<SessionId> =
        state.sessions.list().into_iter().map(|s| s.id).collect();
    let alias = alias.to_string();
    std::thread::spawn(move || {
        let collected = crate::ssh::tmux::collect_orphans(&alias, &install, &known);
        if !collected.is_empty() {
            eprintln!(
                "{alias}: {} sessão(ões) órfã(s) recolhida(s)",
                collected.len()
            );
        }
    });
}

fn end_ssh_session_on_host(state: &State<'_, AppState>, session: &session::Session) {
    let SessionKind::Ssh { host_id } = &session.kind else {
        return;
    };
    let Ok(hosts) = state.store.load_hosts() else {
        return;
    };
    let Some(alias) = hosts
        .iter()
        .find(|h| &h.id == host_id)
        .map(|h| h.alias.clone())
    else {
        return;
    };
    let Ok(install) = crate::ssh::tmux::install_id(&state.store) else {
        return;
    };
    let name = crate::ssh::tmux::session_name(&install, session.id);
    let tunnels = state
        .store
        .load_session_tunnels(session.id)
        .unwrap_or_default();
    for t in &tunnels {
        state.tunnel_states.forget(&t.id);
    }
    let _ = state.store.remove_session_tunnels(session.id);
    std::thread::spawn(move || {
        for t in &tunnels {
            let _ = crate::ssh::tunnel::close_on_master(&alias, &t.tunnel);
        }
        let _ = crate::ssh::tmux::kill_remote(&alias, &name);
    });
}

fn dispose_all(app: &AppHandle, state: &State<'_, AppState>, ids: &[SessionId]) {
    for id in ids {
        teardown_agent_session(app, state, *id);
        if let Some(s) = state.sessions.get(*id) {
            end_ssh_session_on_host(state, &s);
        }
        state.sessions.dispose(&state.pty_pool, *id);
    }
}

fn session_exited(app: &AppHandle, id: SessionId) {
    let state = app.state::<AppState>();
    let Some(session) = state.sessions.get(id) else {
        return;
    };
    teardown_agent_session(app, &state, id);
    if matches!(session.kind, SessionKind::Ssh { .. }) {
        reattach_or_finish(app.clone(), id);
        return;
    }
    if matches!(session.kind, SessionKind::Shell) {
        state.sessions.dispose(&state.pty_pool, id);
        let _ = state.layout.session_disposed(id);
        emit_layout(app, &state);
    } else {
        state
            .sessions
            .set_status(app, id, SessionStatus::Exited { code: -1 });
        let _ = state.repo_reconcile.send(());
    }
}

fn ssh_target(state: &State<'_, AppState>, id: SessionId) -> Option<(String, String, String)> {
    let session = state.sessions.get(id)?;
    let SessionKind::Ssh { host_id } = &session.kind else {
        return None;
    };
    let hosts = state.store.load_hosts().ok()?;
    let alias = hosts.iter().find(|h| &h.id == host_id)?.alias.clone();
    let install = crate::ssh::tmux::install_id(&state.store).ok()?;
    let name = crate::ssh::tmux::session_name(&install, id);
    Some((host_id.clone(), alias, name))
}

#[derive(Clone, serde::Serialize)]
struct SessionTunnelPayload {
    session_id: SessionId,
    tunnels: Vec<crate::ssh::tunnel::SessionTunnel>,
}

fn emit_session_tunnels(
    app: &AppHandle,
    id: SessionId,
    tunnels: &[crate::ssh::tunnel::SessionTunnel],
) {
    let _ = app.emit(
        "session-tunnels",
        SessionTunnelPayload {
            session_id: id,
            tunnels: tunnels.to_vec(),
        },
    );
}

fn restore_session_tunnels(app: &AppHandle, id: SessionId, alias: &str) {
    let state = app.state::<AppState>();
    let Ok(mut tunnels) = state.store.load_session_tunnels(id) else {
        return;
    };
    if tunnels.is_empty() {
        return;
    }
    emit_session_tunnels(app, id, &tunnels);
    for t in &mut tunnels {
        t.state = if crate::ssh::tunnel::control_master_available() {
            match crate::ssh::tunnel::open_on_master(alias, &t.tunnel) {
                Ok(()) => crate::ssh::tunnel::TunnelState::Live,
                Err(e) => crate::ssh::tunnel::TunnelState::Error {
                    detail: e.params.get("detail").cloned().unwrap_or(e.code),
                },
            }
        } else {
            crate::ssh::tunnel::TunnelState::Opening
        };
        state.tunnel_states.set(&t.id, t.state.clone());
    }
    emit_session_tunnels(app, id, &tunnels);
}

fn reattach_or_finish(app: AppHandle, id: SessionId) {
    std::thread::spawn(move || {
        let Some((host_id, alias, name)) = ssh_target(&app.state::<AppState>(), id) else {
            return;
        };

        let mut attempt = 0u32;
        loop {
            if app.state::<AppState>().sessions.get(id).is_none() {
                return;
            }

            let verdict = crate::ssh::tmux::probe(&alias, &name);
            if !verdict.should_reattach() {
                let state = app.state::<AppState>();
                if verdict == crate::ssh::tmux::Probe::NoTmux {
                    eprintln!("{alias}: host sem tmux — sessão sem persistência");
                }
                state.sessions.dispose(&state.pty_pool, id);
                let _ = state.layout.session_disposed(id);
                emit_layout(&app, &state);
                return;
            }

            let Some(delay) = crate::ssh::tmux::retry_delay(attempt) else {
                app.state::<AppState>().sessions.set_connection(
                    &app,
                    id,
                    session::ConnectionState::Dropped,
                );
                return;
            };
            app.state::<AppState>().sessions.set_connection(
                &app,
                id,
                session::ConnectionState::Reconnecting,
            );
            std::thread::sleep(delay);

            if reattach_now(&app, id, &host_id, &alias).is_ok() {
                restore_session_tunnels(&app, id, &alias);
                return;
            }
            attempt += 1;
        }
    });
}

fn reattach_now(app: &AppHandle, id: SessionId, host_id: &str, alias: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let handle = app.clone();
    state
        .sessions
        .spawn_ssh(
            app.clone(),
            &state.pty_pool,
            id,
            host_id.to_string(),
            alias,
            None,
            100,
            30,
            move |id| session_exited(&handle, id),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[derive(Clone, serde::Serialize)]
struct SetupResultPayload {
    session_id: SessionId,
    ok: bool,
    log: String,
}

/// Roda o `.tyba/setup.sh` do worktree em background quando (e só quando)
/// o usuário consentiu com o hash atual do script. Nunca bloqueia a criação.
fn run_setup_if_consented(app: &AppHandle, state: &AppState, session: &Session) {
    let Some(wt) = &session.worktree else { return };
    let Some(script) = worktree::setup_script(&wt.path) else {
        return;
    };
    let repo_key = repo::canonicalize_or(session.repo_root.as_deref().unwrap_or(&wt.path))
        .to_string_lossy()
        .into_owned();
    let allowed = state
        .store
        .setup_consent(&repo_key, &script.hash)
        .ok()
        .flatten();
    if allowed != Some(true) {
        return;
    }
    let app = app.clone();
    let path = wt.path.clone();
    let env = worktree::setup_env(&path);
    let session_id = session.id;
    std::thread::Builder::new()
        .name("worktree-setup".into())
        .spawn(move || {
            let (ok, log) = match worktree::run_setup(&path, &script, &env) {
                Ok(log) => (true, log),
                Err(e) => (false, e),
            };
            let _ = app.emit(
                &format!("worktree://setup/{session_id}"),
                SetupResultPayload {
                    session_id,
                    ok,
                    log,
                },
            );
        })
        .ok();
}

fn teardown_agent_session(app: &AppHandle, state: &AppState, id: SessionId) {
    state.hook_servers.shutdown(id);
    state.subagents.remove_session(app, id);
    for request in state.approvals.expire_session(app, id) {
        agent::session::record_history(
            &state.store,
            request.session_id,
            request.command,
            request.cwd,
            request.risk,
            "expired",
            request.requested_at_ms,
        );
    }
}

/// Reabre no boot as sessões que morreram com o app anterior, conforme a pref de
/// startup. Devolve o mapa `sessão morta -> sessão nova` para o layout reapontar
/// os panes: sem isso o pane guarda um id que não existe mais e a tab abre vazia.
///
fn hosts_alias(store: &Arc<session::store::Store>, host_id: &str) -> Option<String> {
    store
        .load_hosts()
        .ok()?
        .iter()
        .find(|h| h.id == host_id)
        .map(|h| h.alias.clone())
}

/// Só shell é reaberto. Um agente não é um processo idempotente: religá-lo sozinho
/// no boot faria um agente começar a agir sem ninguém ter pedido — a sessão volta
/// morta, e o dono decide.
///
/// O cwd de cada sessão passa por [`session::cwd::reopen_policy`] antes de
/// qualquer syscall daqui: caminho em volume que pode não estar montado não é
/// reaberto, e pasta protegida pelo TCC é reaberta sem o `is_dir()` desta
/// função. O segundo caso **não** evita o diálogo de permissão do macOS — o
/// `resolve_cwd` do spawn stata o mesmo caminho poucas linhas depois, e o shell
/// ainda faz `chdir` para lá. O módulo explica o que cada variante compra.
fn resume_startup(
    app: &AppHandle,
    store: &Arc<session::store::Store>,
    sessions: &SharedSessionManager,
    pty_pool: &SharedPtyPool,
) -> std::collections::HashMap<SessionId, SessionId> {
    let mode = session::StartupMode::parse(
        store
            .get_setting(session::STARTUP_PREF_KEY)
            .ok()
            .flatten()
            .as_deref(),
    );
    let mut remap = std::collections::HashMap::new();
    let dead = sessions.dead_sessions();
    // Resolvido uma vez: a classificação é pura, e ler o env por sessão não
    // acrescentaria nada.
    let home = session::cwd::home();

    if mode == session::StartupMode::Fresh {
        for s in dead {
            if !s.kind.forgettable_on_fresh() {
                continue;
            }
            sessions.forget(s.id);
        }
        return remap;
    }
    if mode == session::StartupMode::KeepLayout {
        return remap;
    }

    for old in dead {
        if let SessionKind::Ssh { host_id } = &old.kind {
            let Some(alias) = hosts_alias(store, host_id) else {
                continue;
            };
            let handle = app.clone();
            if let Err(e) = sessions.spawn_ssh(
                app.clone(),
                pty_pool,
                old.id,
                host_id.clone(),
                &alias,
                None,
                100,
                30,
                move |id| session_exited(&handle, id),
            ) {
                eprintln!("reattach da sessão {}: {e}", old.id);
            } else {
                let handle = app.clone();
                let alias = alias.clone();
                let id = old.id;
                std::thread::spawn(move || restore_session_tunnels(&handle, id, &alias));
            }
            continue;
        }
        if !matches!(old.kind, SessionKind::Shell) {
            continue;
        }
        let Some(cwd) = old.cwd.clone() else {
            continue;
        };
        // Este `stat` era o congelamento da abertura enquanto o boot rodava na
        // main thread; hoje o que ele ainda pode fazer é pendurar a thread de
        // boot num volume de rede morto, sem diálogo e sem timeout. Leia
        // `session::cwd` antes de "consertar" isto de volta para um `is_dir()`
        // incondicional.
        match session::cwd::reopen_policy(&cwd, home.as_deref()) {
            session::cwd::ReopenPolicy::Checked if !cwd.is_dir() => continue,
            session::cwd::ReopenPolicy::Skip => {
                eprintln!(
                    "[tyba] sessão {} volta parada: {} está em volume que pode não estar montado",
                    old.id,
                    cwd.display()
                );
                continue;
            }
            _ => {}
        }
        let handle = app.clone();
        let opts = CreateSessionOpts {
            kind: SessionKind::Shell,
            title: Some(old.title.clone()),
            cwd: Some(cwd),
            cols: 100,
            rows: 30,
            worktree_task: None,
            attach_existing: false,
            shell: None,
            initial_prompt: None,
        };
        match sessions.create_shell_session(app.clone(), pty_pool, opts, move |id| {
            session_exited(&handle, id)
        }) {
            Ok(fresh) => {
                remap.insert(old.id, fresh.id);
                sessions.forget(old.id);
            }
            Err(e) => {
                eprintln!("reopen da sessão {}: {e}", old.id);
            }
        }
    }
    remap
}

/// `async` de propósito: comando síncrono roda na **main thread**, e este aqui
/// espera o boot terminar. O splash do front desiste em 4 s, então a janela fica
/// clicável enquanto a thread de boot ainda pode estar carregando — um ⌘T nessa
/// fresta congelaria a main thread, e com ela o webview, por até [`BOOT_WAIT`].
/// Nada abaixo precisa da main thread: menu e janela ficam no `setup()`.
#[tauri::command]
async fn create_session(
    app: AppHandle,
    state: State<'_, AppState>,
    opts: CreateSessionOpts,
) -> Result<Session, String> {
    // Leitura devolve "carregando"; escrita espera. Criar sessão antes do boot
    // terminar seria criá-la para o `load_remapped` da thread de boot passar por
    // cima logo em seguida — o pane nasceria apontando para o nada.
    //
    // A espera é condvar bloqueante: num worker do runtime ela dormiria até
    // `BOOT_WAIT` segurando os outros comandos assíncronos, daí o threadpool de
    // blocking. O resultado é descartado pelo mesmo motivo que o teto existe —
    // passado ele, agimos com estado incompleto em vez de pendurar o clique.
    let boot = Arc::clone(&state.boot);
    let _ = tauri::async_runtime::spawn_blocking(move || boot.wait_ready(BOOT_WAIT)).await;

    let handle = app.clone();
    let session = match &opts.kind {
        SessionKind::Agent { .. } => {
            let ctx = agent::session::AgentSessionCtx {
                app: app.clone(),
                sessions: Arc::clone(&state.sessions),
                pty_pool: Arc::clone(&state.pty_pool),
                approvals: Arc::clone(&state.approvals),
                store: Arc::clone(&state.store),
                servers: Arc::clone(&state.hook_servers),
                subagents: Arc::clone(&state.subagents),
            };
            let spawn = if opts.attach_existing {
                agent::session::attach_agent_session
            } else {
                agent::session::create_agent_session
            };
            spawn(&ctx, opts, move |id| session_exited(&handle, id))?
        }
        SessionKind::Shell => state
            .sessions
            .create_shell_session(app.clone(), &state.pty_pool, opts, move |id| {
                session_exited(&handle, id)
            })
            .map_err(|e| e.to_string())?,
        // Container nasce pelo painel de Docker (open_container_tab), não por aqui.
        SessionKind::Container { .. } => {
            return Err(crate::error::AppError::new("session.kind_unsupported").to_string())
        }
        SessionKind::Ssh { host_id } => {
            let host = state
                .store
                .load_hosts()
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|h| &h.id == host_id)
                .ok_or_else(|| {
                    crate::error::AppError::new("ssh.host_not_found")
                        .with("id", host_id.clone())
                        .to_string()
                })?;
            let home = crate::ssh::home_dir();
            let session = state
                .sessions
                .create_ssh_session(
                    app.clone(),
                    &state.pty_pool,
                    host.id.clone(),
                    &host.alias,
                    home.as_deref(),
                    opts.cols,
                    opts.rows,
                    move |id| session_exited(&handle, id),
                )
                .map_err(|e| e.to_string())?;
            let _ = state
                .store
                .touch_host_connected(&host.id, chrono::Utc::now());
            gc_host(&state, &host.alias);
            session
        }
    };
    run_setup_if_consented(&app, &state, &session);
    Ok(session)
}

fn store_err(e: session::store::StoreError) -> crate::error::AppError {
    crate::error::AppError::new("store.failed").with("detail", e.to_string())
}

fn validate_alias(alias: &str) -> Result<(), crate::error::AppError> {
    // `-` inicial faria o alias ser interpretado como opção do `ssh`
    // (`-oProxyCommand=...` = exec local): recusado na entrada, o que protege de
    // graça os sites que passam o alias pro ssh (tmux, túneis, SFTP).
    if alias.trim().is_empty() || alias.chars().any(|c| c.is_whitespace()) || alias.starts_with('-')
    {
        return Err(
            crate::error::AppError::new("ssh.alias_invalid").with("alias", alias.to_string())
        );
    }
    Ok(())
}

fn validate_tunnels(host: &crate::ssh::Host) -> Result<(), crate::error::AppError> {
    for t in &host.tunnels {
        t.validate()?;
    }
    Ok(())
}

fn rematerialize_hosts(state: &State<'_, AppState>) -> Result<(), crate::error::AppError> {
    let hosts = state.store.load_hosts().map_err(store_err)?;
    if let Some(home) = crate::ssh::home_dir() {
        crate::ssh::config::materialize(&home, &hosts)?;
    }
    Ok(())
}

#[tauri::command]
async fn list_hosts(
    state: State<'_, AppState>,
) -> Result<Vec<crate::ssh::Host>, crate::error::AppError> {
    state.store.load_hosts().map_err(store_err)
}

#[tauri::command]
fn list_host_groups(
    state: State<'_, AppState>,
) -> Result<Vec<crate::ssh::HostGroup>, crate::error::AppError> {
    state.store.load_host_groups().map_err(store_err)
}

fn gate_new_risky_tunnels(
    prev: &[crate::ssh::tunnel::Tunnel],
    next: &[crate::ssh::tunnel::Tunnel],
    confirmed: Option<bool>,
) -> Result<(), crate::error::AppError> {
    if confirmed == Some(true) {
        return Ok(());
    }
    match crate::ssh::tunnel::added_risky_tunnel(prev, next) {
        Some(t) => Err(crate::error::AppError::new("ssh.tunnel_needs_confirmation")
            .with("kind", t.kind.flag())
            .with("port", t.listen_port.to_string())),
        None => Ok(()),
    }
}

#[tauri::command]
fn create_host(
    state: State<'_, AppState>,
    input: crate::ssh::HostInput,
    confirmed: Option<bool>,
) -> Result<crate::ssh::Host, crate::error::AppError> {
    validate_alias(&input.alias)?;
    let existing = state.store.load_hosts().map_err(store_err)?;
    if existing.iter().any(|h| h.alias == input.alias) {
        return Err(crate::error::AppError::new("ssh.alias_duplicate").with("alias", input.alias));
    }
    let host = crate::ssh::Host {
        id: uuid::Uuid::new_v4().to_string(),
        alias: input.alias,
        hostname: input.hostname,
        port: input.port,
        username: input.username,
        identity_file: input.identity_file,
        proxy_jump: input.proxy_jump,
        group_id: input.group_id,
        color: input.color,
        notes: input.notes,
        position: existing.len() as i64,
        tunnels: input.tunnels,
        created_at: chrono::Utc::now(),
        last_connected_at: None,
    };
    validate_tunnels(&host)?;
    gate_new_risky_tunnels(&[], &host.tunnels, confirmed)?;
    state.store.upsert_host(&host).map_err(store_err)?;
    rematerialize_hosts(&state)?;
    Ok(host)
}

#[tauri::command]
fn update_host(
    state: State<'_, AppState>,
    host: crate::ssh::Host,
    confirmed: Option<bool>,
) -> Result<crate::ssh::Host, crate::error::AppError> {
    validate_alias(&host.alias)?;
    let existing = state.store.load_hosts().map_err(store_err)?;
    if existing
        .iter()
        .any(|h| h.alias == host.alias && h.id != host.id)
    {
        return Err(crate::error::AppError::new("ssh.alias_duplicate").with("alias", host.alias));
    }
    let prev = existing
        .iter()
        .find(|h| h.id == host.id)
        .map(|h| h.tunnels.as_slice())
        .unwrap_or(&[]);
    validate_tunnels(&host)?;
    gate_new_risky_tunnels(prev, &host.tunnels, confirmed)?;
    state.store.upsert_host(&host).map_err(store_err)?;
    rematerialize_hosts(&state)?;
    Ok(host)
}

#[tauri::command]
fn list_session_tunnels(
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<Vec<crate::ssh::tunnel::SessionTunnel>, crate::error::AppError> {
    let mut tunnels = state
        .store
        .load_session_tunnels(session_id)
        .map_err(store_err)?;
    state.tunnel_states.apply(&mut tunnels);
    Ok(tunnels)
}

#[tauri::command]
fn open_session_tunnel(
    state: State<'_, AppState>,
    session_id: SessionId,
    tunnel: crate::ssh::tunnel::Tunnel,
    confirmed: bool,
) -> Result<crate::ssh::tunnel::SessionTunnel, crate::error::AppError> {
    tunnel.validate()?;
    if tunnel.kind.needs_confirmation() && !confirmed {
        return Err(crate::error::AppError::new("ssh.tunnel_needs_confirmation")
            .with("kind", tunnel.kind.flag())
            .with("port", tunnel.listen_port.to_string()));
    }
    let alias = ssh_target(&state, session_id)
        .map(|(_, alias, _)| alias)
        .ok_or_else(|| crate::error::AppError::new("ssh.session_not_found"))?;

    let master = crate::ssh::tunnel::control_master_available();
    if master {
        crate::ssh::tunnel::open_on_master(&alias, &tunnel)?;
    } else if !crate::ssh::tunnel::local_port_free(&tunnel) {
        return Err(crate::error::AppError::new("ssh.tunnel_open_failed")
            .with("port", tunnel.listen_port.to_string())
            .with("detail", "a porta local já está em uso"));
    }

    let entry = crate::ssh::tunnel::SessionTunnel {
        id: uuid::Uuid::new_v4().to_string(),
        session_id,
        tunnel,
        state: if master {
            crate::ssh::tunnel::TunnelState::Live
        } else {
            crate::ssh::tunnel::TunnelState::Opening
        },
        created_at: chrono::Utc::now(),
    };
    state.store.add_session_tunnel(&entry).map_err(store_err)?;
    state.tunnel_states.set(&entry.id, entry.state.clone());

    if !master {
        let _ = state.pty_pool.kill(session_id);
    }
    Ok(entry)
}

#[tauri::command]
fn close_session_tunnel(
    state: State<'_, AppState>,
    session_id: SessionId,
    tunnel_id: String,
) -> Result<(), crate::error::AppError> {
    let tunnels = state
        .store
        .load_session_tunnels(session_id)
        .map_err(store_err)?;
    let Some(entry) = tunnels.into_iter().find(|t| t.id == tunnel_id) else {
        return Ok(());
    };
    if let Some((_, alias, _)) = ssh_target(&state, session_id) {
        let _ = crate::ssh::tunnel::close_on_master(&alias, &entry.tunnel);
    }
    state.tunnel_states.forget(&tunnel_id);
    state
        .store
        .remove_session_tunnel(&tunnel_id)
        .map_err(store_err)
}

#[tauri::command]
fn delete_host(state: State<'_, AppState>, id: String) -> Result<(), crate::error::AppError> {
    state.store.remove_host(&id).map_err(store_err)?;
    rematerialize_hosts(&state)?;
    Ok(())
}

#[tauri::command]
fn create_host_group(
    state: State<'_, AppState>,
    input: crate::ssh::HostGroupInput,
) -> Result<crate::ssh::HostGroup, crate::error::AppError> {
    let existing = state.store.load_host_groups().map_err(store_err)?;
    let group = crate::ssh::HostGroup {
        id: uuid::Uuid::new_v4().to_string(),
        name: input.name,
        color: input.color,
        notes: input.notes,
        position: existing.len() as i64,
        created_at: chrono::Utc::now(),
    };
    state.store.upsert_host_group(&group).map_err(store_err)?;
    Ok(group)
}

#[tauri::command]
fn update_host_group(
    state: State<'_, AppState>,
    group: crate::ssh::HostGroup,
) -> Result<crate::ssh::HostGroup, crate::error::AppError> {
    state.store.upsert_host_group(&group).map_err(store_err)?;
    Ok(group)
}

#[tauri::command]
fn delete_host_group(state: State<'_, AppState>, id: String) -> Result<(), crate::error::AppError> {
    state.store.remove_host_group(&id).map_err(store_err)?;
    Ok(())
}

#[derive(serde::Serialize)]
struct WorktreeStatus {
    path: std::path::PathBuf,
    branch: Option<String>,
    dirty: bool,
    ahead_of_head: u32,
    managed: bool,
    session_id: Option<SessionId>,
}

#[derive(serde::Serialize)]
struct WorktreeListing {
    root: std::path::PathBuf,
    worktrees: Vec<WorktreeStatus>,
}

#[tauri::command]
async fn worktree_list(
    state: State<'_, AppState>,
    repo_root: String,
) -> Result<WorktreeListing, String> {
    let param = repo::canonicalize_or(std::path::Path::new(&repo_root));
    let root = worktree::main_repo_of(&param)
        .map(|t| repo::canonicalize_or(&t))
        .map_err(|_| "fora de repositório git".to_string())?;
    let head = worktree::head_sha(&root)?;
    let by_path: std::collections::HashMap<std::path::PathBuf, SessionId> = state
        .sessions
        .list()
        .into_iter()
        .filter_map(|s| s.worktree.map(|wt| (repo::canonicalize_or(&wt.path), s.id)))
        .collect();

    let worktrees = worktree::list(&root)?
        .into_iter()
        .filter_map(|e| {
            let canonical = repo::canonicalize_or(&e.path);
            if canonical == root {
                return None;
            }
            Some(WorktreeStatus {
                dirty: worktree::is_dirty(&e.path).unwrap_or(false),
                ahead_of_head: worktree::ahead_count(&e.path, &head).unwrap_or(0),
                managed: worktree::is_managed(&e.path),
                session_id: by_path.get(&canonical).copied(),
                path: e.path,
                branch: e.branch,
            })
        })
        .collect();
    Ok(WorktreeListing { root, worktrees })
}

#[derive(serde::Serialize)]
struct SetupScriptInfo {
    path: std::path::PathBuf,
    content: String,
    hash: String,
    consent: Option<bool>,
}

#[tauri::command]
fn worktree_setup_script(state: State<'_, AppState>, repo_root: String) -> Option<SetupScriptInfo> {
    let param = repo::canonicalize_or(std::path::Path::new(&repo_root));
    let root = repo::toplevel(&param).map(|t| repo::canonicalize_or(&t))?;
    let script = worktree::setup_script(&root)?;
    let consent = state
        .store
        .setup_consent(&root.to_string_lossy(), &script.hash)
        .ok()
        .flatten();
    Some(SetupScriptInfo {
        path: script.path,
        content: script.content,
        hash: script.hash,
        consent,
    })
}

#[tauri::command]
fn worktree_set_setup_consent(
    state: State<'_, AppState>,
    repo_root: String,
    hash: String,
    allow: bool,
) -> Result<(), String> {
    let param = repo::canonicalize_or(std::path::Path::new(&repo_root));
    let root = repo::toplevel(&param)
        .map(|t| repo::canonicalize_or(&t))
        .ok_or("fora de repositório git")?;
    state
        .store
        .set_setup_consent(&root.to_string_lossy(), &hash, allow)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn worktree_remove(
    state: State<'_, AppState>,
    path: String,
    delete_branch: bool,
    force: bool,
) -> Result<(), String> {
    let path = std::path::PathBuf::from(&path);
    let canonical = repo::canonicalize_or(&path);
    let busy = state.sessions.list().into_iter().any(|s| {
        matches!(s.status, SessionStatus::Running)
            && s.worktree
                .map(|wt| repo::canonicalize_or(&wt.path) == canonical)
                .unwrap_or(false)
    });
    if busy {
        return Err("worktree tem sessão ativa — encerre a sessão antes de remover".into());
    }
    worktree::remove(&path, delete_branch, force)
}

#[tauri::command]
async fn worktree_gc(state: State<'_, AppState>) -> Result<worktree::GcReport, String> {
    Ok(worktree::gc_orphans(&known_worktree_paths(&state.sessions)))
}

fn session_worktree(state: &AppState, id: SessionId) -> Result<worktree::Worktree, String> {
    state
        .sessions
        .get(id)
        .ok_or_else(|| format!("sessão não encontrada: {id}"))?
        .worktree
        .ok_or_else(|| "sessão não está em worktree".to_string())
}

struct RepoContext {
    path: std::path::PathBuf,
    base_ref: String,
}

fn session_repo_context(state: &AppState, id: SessionId) -> Result<RepoContext, String> {
    let session = state
        .sessions
        .get(id)
        .ok_or_else(|| format!("sessão não encontrada: {id}"))?;
    if let Some(wt) = session.worktree {
        return Ok(RepoContext {
            path: wt.path,
            base_ref: wt.base_ref,
        });
    }
    let pid = state
        .pty_pool
        .leader_pid(id)
        .ok_or("a sessão não tem um processo ativo")?;
    let cwd = repo::process_cwd(pid).ok_or("não foi possível ler o diretório da sessão")?;
    let root = repo::toplevel(&cwd).ok_or("o diretório da sessão não é um repositório git")?;
    Ok(RepoContext {
        path: repo::canonicalize_or(&root),
        base_ref: "HEAD".to_string(),
    })
}

#[derive(serde::Serialize)]
struct SessionGitStatus {
    root: String,
    branch: Option<String>,
    dirty: bool,
}

#[tauri::command]
async fn session_git_status(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Option<SessionGitStatus>, String> {
    let Ok(ctx) = session_repo_context(&state, id) else {
        return Ok(None);
    };
    let dirty = worktree::is_dirty(&ctx.path).unwrap_or(false);
    let branch = repo::branch(&ctx.path);
    Ok(Some(SessionGitStatus {
        root: ctx.path.to_string_lossy().into_owned(),
        branch,
        dirty,
    }))
}

#[tauri::command]
fn open_diff_tab(app: AppHandle, state: State<'_, AppState>, id: SessionId) -> Result<(), String> {
    session_repo_context(&state, id)?;
    let previous = session_side_view(&state, id);
    state
        .layout
        .open_workspace_side_view(id, &layout::diff_view(id))
        .map_err(|e| e.to_string())?;
    close_orphaned_files_panel(&state, previous, None);
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn open_tunnels_panel(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<(), crate::error::AppError> {
    let session = state
        .sessions
        .get(id)
        .ok_or_else(|| crate::error::AppError::new("ssh.session_not_found"))?;
    if !matches!(session.kind, SessionKind::Ssh { .. }) {
        return Err(crate::error::AppError::new("ssh.not_an_ssh_session"));
    }
    let previous = session_side_view(&state, id);
    state
        .layout
        .open_workspace_side_view(id, &layout::tunnels_view(id))
        .map_err(|e| crate::error::AppError::new("layout.failed").with("detail", e.to_string()))?;
    close_orphaned_files_panel(&state, previous, None);
    emit_layout(&app, &state);
    Ok(())
}

fn resolve_files_root(
    state: &AppState,
    id: SessionId,
) -> Result<(std::path::PathBuf, files::Context), String> {
    let session = state
        .sessions
        .get(id)
        .ok_or_else(|| format!("sessão não encontrada: {id}"))?;
    if let Some(wt) = session.worktree {
        let root = repo::canonicalize_or(&wt.path);
        return Ok((
            root,
            files::Context::AgentWorktree {
                base_ref: wt.base_ref,
            },
        ));
    }
    let pid = state
        .pty_pool
        .leader_pid(id)
        .ok_or("a sessão não tem um processo ativo")?;
    let cwd = repo::process_cwd(pid).ok_or("não foi possível ler o diretório da sessão")?;
    match files::find_repo_root(&cwd) {
        Some(root) => Ok((repo::canonicalize_or(&root), files::Context::Repo)),
        None => Ok((repo::canonicalize_or(&cwd), files::Context::OutsideRepo)),
    }
}

/// Alias materializado + nome do tmux invisível de uma sessão SSH — o par que o
/// backend SFTP usa para abrir o canal na conexão multiplexada e para achar o
/// cwd remoto (`pane_current_path`). `None` quando a sessão não é SSH.
fn ssh_alias_tmux(state: &AppState, id: SessionId) -> Option<(String, String)> {
    let session = state.sessions.get(id)?;
    let SessionKind::Ssh { host_id } = &session.kind else {
        return None;
    };
    let hosts = state.store.load_hosts().ok()?;
    let alias = hosts.iter().find(|h| &h.id == host_id)?.alias.clone();
    let install = crate::ssh::tmux::install_id(&state.store).ok()?;
    let name = crate::ssh::tmux::session_name(&install, id);
    Some((alias, name))
}

type RemoteTarget = (files::remote::SharedRemoteFiles, String, String);

/// Despacho por tipo de sessão, SÍNCRONO e sem rede: `None` = sessão local
/// (segue o `FilesManager`); `Some(Ok((gestor, alias, tmux)))` = alvo remoto
/// (o painel é montado depois, fora da thread do executor); `Some(Err)` = SSH
/// cujo host não resolveu. Extrai só dados `Send` para que nada de `State` seja
/// segurado através do `.await` do build.
fn remote_target(state: &AppState, id: SessionId) -> Option<Result<RemoteTarget, String>> {
    let session = state.sessions.get(id)?;
    if !matches!(session.kind, SessionKind::Ssh { .. }) {
        return None;
    }
    match ssh_alias_tmux(state, id) {
        Some((alias, tmux)) => Some(Ok((
            std::sync::Arc::clone(&state.remote_files),
            alias,
            tmux,
        ))),
        None => Some(Err("host da sessão SSH não encontrado".to_string())),
    }
}

/// Monta (ou reusa) o painel remoto FORA da thread do command: `build_panel`
/// abre o canal SFTP e faz o handshake — rede pura, que não pode congelar um
/// worker do Tokio. `spawn_blocking` + `ConnectTimeout` no ssh cobrem host morto.
async fn open_remote_panel(
    remote_files: files::remote::SharedRemoteFiles,
    id: SessionId,
    alias: String,
    tmux: String,
) -> Result<std::sync::Arc<files::remote::RemotePanel>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        remote_files.ensure(id, || files::remote::build_panel(&alias, Some(&tmux)))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Teardown único do painel de arquivos: derruba o painel local, o remoto (SFTP)
/// e a sessão de LSP da fatia 3, num só lugar. Chamado por todos os sites que
/// encerram um painel — a lição do watcher órfão, terceira aparição.
fn close_files_panel(state: &AppState, id: SessionId) {
    state.files.close(id);
    state.remote_files.close(id);
    state.lsp.close_session(id);
}

/// Roda uma operação do painel remoto fora da thread do command (as chamadas
/// SFTP/exec são de rede — nunca podem travar a UI). `spawn_blocking` mantém o
/// core responsivo, mesmo padrão do docker-over-ssh.
async fn on_remote<T, F>(
    panel: std::sync::Arc<files::remote::RemotePanel>,
    f: F,
) -> Result<T, String>
where
    F: FnOnce(&files::remote::RemotePanel) -> T + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || f(&panel))
        .await
        .map_err(|e| e.to_string())
}

fn is_ssh_session(state: &AppState, id: SessionId) -> bool {
    matches!(
        state.sessions.get(id).map(|s| s.kind),
        Some(SessionKind::Ssh { .. })
    )
}

fn workspace_side_view(state: &AppState, workspace: layout::WorkspaceId) -> Option<String> {
    state
        .layout
        .state()
        .workspaces
        .into_iter()
        .find(|w| w.id == workspace)
        .and_then(|w| w.side_view)
}

fn session_side_view(state: &AppState, session: SessionId) -> Option<String> {
    let workspace = state.layout.workspace_of_session(session)?;
    workspace_side_view(state, workspace)
}

fn close_orphaned_files_panel(state: &AppState, previous: Option<String>, keep: Option<&str>) {
    let Some(prev) = previous else {
        return;
    };
    if keep == Some(prev.as_str()) {
        return;
    }
    if let Some(id) = prev
        .strip_prefix(layout::VIEW_FILES_PREFIX)
        .and_then(|s| s.parse::<SessionId>().ok())
    {
        close_files_panel(state, id);
    }
}

#[tauri::command]
async fn open_files_panel(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<String, String> {
    let root = match remote_target(&state, id) {
        Some(t) => {
            let (rf, alias, tmux) = t?;
            open_remote_panel(rf, id, alias, tmux).await?.info().root
        }
        None => {
            let (root, _ctx) = state
                .files
                .ensure(&app, id, || resolve_files_root(&state, id))?;
            root.to_string_lossy().into_owned()
        }
    };
    let new_view = layout::files_view(id);
    let previous = session_side_view(&state, id);
    state
        .layout
        .open_workspace_side_view(id, &new_view)
        .map_err(|e| e.to_string())?;
    close_orphaned_files_panel(&state, previous, Some(&new_view));
    emit_layout(&app, &state);
    Ok(root)
}

#[tauri::command]
async fn files_panel_info(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<files::PanelInfo, String> {
    if let Some(t) = remote_target(&state, id) {
        let (rf, alias, tmux) = t?;
        return Ok(open_remote_panel(rf, id, alias, tmux).await?.info());
    }
    let (root, context) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    // A raiz ancorada não segue o `cd` — é decisão, não descuido. Aqui só se
    // mede se ela ainda bate com o que a sessão resolveria agora, para a UI
    // poder oferecer o re-ancorar em vez de esperar que o usuário descubra o
    // botão sozinho. Falha de resolução (sessão sem líder, cwd ilegível) é
    // `None`: não se cutuca o usuário por causa de um estado transitório.
    let drifted_to = resolve_files_root(&state, id)
        .ok()
        .map(|(live, _)| live)
        .filter(|live| live != &root)
        .map(|live| live.to_string_lossy().into_owned());
    Ok(files::PanelInfo {
        root: root.to_string_lossy().into_owned(),
        kind: context.kind_str().to_string(),
        decorated: !matches!(context, files::Context::OutsideRepo),
        remote: false,
        host: None,
        drifted_to,
    })
}

#[tauri::command]
async fn files_list_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
) -> Result<files::DirListing, String> {
    if let Some(t) = remote_target(&state, id) {
        let (rf, alias, tmux) = t?;
        let panel = open_remote_panel(rf, id, alias, tmux).await?;
        return on_remote(panel, move |p| p.list_dir(&path)).await?;
    }
    let (root, context) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    files::list_dir(&root, &path, &context)
}

#[tauri::command]
async fn files_read(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    offset: Option<usize>,
) -> Result<files::FileContent, String> {
    if let Some(t) = remote_target(&state, id) {
        let (rf, alias, tmux) = t?;
        let panel = open_remote_panel(rf, id, alias, tmux).await?;
        let off = offset.unwrap_or(0);
        return on_remote(panel, move |p| p.read_file(&path, off)).await?;
    }
    let (root, _ctx) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    files::read_file(&root, &path, offset.unwrap_or(0))
}

#[tauri::command]
fn files_watch_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
) -> Result<(), String> {
    // Sessão SSH não tem watcher: refresh é sob demanda. No-op sem tocar a rede.
    if is_ssh_session(&state, id) {
        return Ok(());
    }
    let (root, _ctx) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    let dir = files::resolve_within(&root, &path)?;
    if !dir.is_dir() {
        return Err("não é um diretório".into());
    }
    state.files.watch_dir(id, dir);
    Ok(())
}

#[tauri::command]
fn files_unwatch_dir(state: State<'_, AppState>, id: SessionId, path: String) {
    if is_ssh_session(&state, id) {
        return;
    }
    if let Some((root, _)) = state.files.info(id) {
        if let Ok(dir) = files::resolve_within(&root, &path) {
            state.files.unwatch_dir(id, dir);
        }
    }
}

/// Refresh explícito do painel remoto — o substituto do watcher. Invalida o
/// cache de listagem; a próxima leitura reconsulta o host. No-op no local (lá o
/// watcher já mantém a árvore viva).
#[tauri::command]
fn files_refresh(state: State<'_, AppState>, id: SessionId) {
    if let Some(panel) = state.remote_files.get(id) {
        if panel.is_dead() {
            // Canal SFTP morto: derruba o painel para a próxima op reconstruir
            // sobre a conexão remultiplexada — o botão "Atualizar" recupera de fato.
            state.remote_files.close(id);
        } else {
            panel.refresh();
        }
    }
}

#[tauri::command]
async fn files_reanchor(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<String, String> {
    // Re-ancorar troca a raiz: derruba tudo da raiz antiga (local + remoto + LSP)
    // e reconstrói na nova. Teardown unificado.
    close_files_panel(&state, id);
    if is_ssh_session(&state, id) {
        let (rf, alias, tmux) = match remote_target(&state, id) {
            Some(t) => t?,
            None => return Err("host da sessão SSH não encontrado".into()),
        };
        let panel = open_remote_panel(rf, id, alias, tmux).await?;
        let root = panel.info().root;
        state.files.emit_tree_reset(&app, id);
        let decos = on_remote(std::sync::Arc::clone(&panel), |p| p.decorations()).await?;
        files::emit_decorations_to(&app, id, decos);
        return Ok(root);
    }
    let (root, context) = resolve_files_root(&state, id)?;
    state.files.seed(&app, id, root.clone(), context);
    state.files.emit_tree_reset(&app, id);
    state.files.emit_decorations(&app, id);
    Ok(root.to_string_lossy().into_owned())
}

#[tauri::command]
async fn files_decorations(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Vec<files::Decoration>, String> {
    if let Some(t) = remote_target(&state, id) {
        let (rf, alias, tmux) = t?;
        let panel = open_remote_panel(rf, id, alias, tmux).await?;
        return on_remote(panel, |p| p.decorations()).await;
    }
    state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    Ok(state.files.decorations(id))
}

#[tauri::command]
fn files_close(state: State<'_, AppState>, id: SessionId) {
    close_files_panel(&state, id);
}

#[tauri::command]
async fn files_write(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    content: String,
    expected_hash: String,
) -> Result<files::write::WriteResult, String> {
    if let Some(t) = remote_target(&state, id) {
        let (rf, alias, tmux) = t?;
        let panel = open_remote_panel(rf, id, alias, tmux).await?;
        let rel = path.clone();
        let result = on_remote(std::sync::Arc::clone(&panel), move |p| {
            p.write(&rel, &content, &expected_hash)
        })
        .await??;
        if matches!(result, files::write::WriteResult::Written { .. }) {
            let rel = path.clone();
            let markers = on_remote(std::sync::Arc::clone(&panel), move |p| p.gutter(&rel)).await?;
            let decos = on_remote(panel, |p| p.decorations()).await?;
            files::emit_gutter_to(&app, id, path, markers);
            files::emit_decorations_to(&app, id, decos);
        }
        return Ok(result);
    }
    let (root, _ctx) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    let result = files::write::write_file(&root, &path, &content, &expected_hash)?;
    if let files::write::WriteResult::Written { hash } = &result {
        state.files.note_written(id, &path, hash);
        state.files.emit_gutter(&app, id, &path);
        state.files.emit_decorations(&app, id);
    }
    Ok(result)
}

#[tauri::command]
async fn files_create(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    is_dir: bool,
) -> Result<(), String> {
    if let Some(t) = remote_target(&state, id) {
        let (rf, alias, tmux) = t?;
        let panel = open_remote_panel(rf, id, alias, tmux).await?;
        return on_remote(panel, move |p| p.create(&path, is_dir)).await?;
    }
    let (root, _ctx) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    files::write::create(&root, &path, is_dir)?;
    state.files.invalidate_index(id);
    Ok(())
}

#[tauri::command]
async fn files_rename(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    from: String,
    to: String,
) -> Result<(), String> {
    if let Some(t) = remote_target(&state, id) {
        let (rf, alias, tmux) = t?;
        let panel = open_remote_panel(rf, id, alias, tmux).await?;
        return on_remote(panel, move |p| p.rename(&from, &to)).await?;
    }
    let (root, _ctx) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    files::write::rename(&root, &from, &to)?;
    state.files.invalidate_index(id);
    Ok(())
}

#[tauri::command]
async fn files_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
) -> Result<(), String> {
    if let Some(t) = remote_target(&state, id) {
        let (rf, alias, tmux) = t?;
        let panel = open_remote_panel(rf, id, alias, tmux).await?;
        return on_remote(panel, move |p| p.delete(&path)).await?;
    }
    let (root, _ctx) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    files::write::delete(&root, &path)?;
    state.files.invalidate_index(id);
    Ok(())
}

#[tauri::command]
async fn files_search(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    query: String,
    limit: Option<usize>,
) -> Result<files::search::SearchOutcome, String> {
    let cap = limit.unwrap_or(50).min(500);
    if let Some(t) = remote_target(&state, id) {
        let (rf, alias, tmux) = t?;
        let panel = open_remote_panel(rf, id, alias, tmux).await?;
        return on_remote(panel, move |p| p.search(&query, cap)).await;
    }
    state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    Ok(state.files.search(id, &query, cap))
}

#[tauri::command]
async fn files_focus(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: Option<String>,
) -> Result<Vec<files::gutter::GutterMarker>, String> {
    if let Some(t) = remote_target(&state, id) {
        let (rf, alias, tmux) = t?;
        let panel = open_remote_panel(rf, id, alias, tmux).await?;
        panel.set_open(path.clone());
        return Ok(match path {
            Some(rel) => on_remote(panel, move |p| p.gutter(&rel)).await?,
            None => Vec::new(),
        });
    }
    state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    state.files.set_open(id, path.clone());
    Ok(match path {
        Some(rel) => state.files.gutter(id, &rel),
        None => Vec::new(),
    })
}

#[tauri::command]
async fn files_gutter(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
) -> Result<Vec<files::gutter::GutterMarker>, String> {
    if let Some(t) = remote_target(&state, id) {
        let (rf, alias, tmux) = t?;
        let panel = open_remote_panel(rf, id, alias, tmux).await?;
        return on_remote(panel, move |p| p.gutter(&path)).await;
    }
    state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    Ok(state.files.gutter(id, &path))
}

#[derive(serde::Serialize)]
struct EditContent {
    text: String,
    hash: String,
}

#[tauri::command]
async fn files_edit_begin(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
) -> Result<EditContent, String> {
    if let Some(t) = remote_target(&state, id) {
        let (rf, alias, tmux) = t?;
        let panel = open_remote_panel(rf, id, alias, tmux).await?;
        let (text, hash) = on_remote(panel, move |p| p.edit_begin(&path)).await??;
        return Ok(EditContent { text, hash });
    }
    state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    let (text, hash) = state.files.edit_begin(id, &path)?;
    Ok(EditContent { text, hash })
}

#[tauri::command]
fn files_edit_end(state: State<'_, AppState>, id: SessionId) {
    if let Some(panel) = state.remote_files.get(id) {
        panel.set_open(None);
        return;
    }
    state.files.edit_end(id);
}

#[tauri::command]
fn lsp_status(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
) -> Result<lsp::LspStatus, String> {
    // LSP é local-only (fatia 3): sessão SSH nunca sobe server (o LSP remoto é a
    // fatia 4b). Guarda no core — não resolve raiz local nem spawna. UI espelha.
    if is_ssh_session(&state, id) {
        return Ok(lsp::LspStatus::Unsupported);
    }
    let (root, _ctx) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    Ok(state.lsp.status(id, &path, &root))
}

#[tauri::command]
fn lsp_open(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    text: String,
    spawn: bool,
) -> Result<lsp::LspStatus, String> {
    if is_ssh_session(&state, id) {
        return Ok(lsp::LspStatus::Unsupported);
    }
    let (root, _ctx) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    files::resolve_within(&root, &path)?;
    Ok(state.lsp.open(&app, id, &root, &path, &text, spawn))
}

#[tauri::command]
fn lsp_retry(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
) -> Result<lsp::LspStatus, String> {
    if is_ssh_session(&state, id) {
        return Ok(lsp::LspStatus::Unsupported);
    }
    let (root, _ctx) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    files::resolve_within(&root, &path)?;
    Ok(state.lsp.retry(&app, id, &root, &path))
}

fn managed_progress_emitter(app: &AppHandle) -> Arc<lsp::managed::ProgressEmit> {
    let app = app.clone();
    Arc::new(move |server_id: &str, progress: &lsp::managed::Progress| {
        let event = match progress {
            lsp::managed::Progress::Error { .. } => format!("lsp://managed/error/{server_id}"),
            _ => format!("lsp://managed/progress/{server_id}"),
        };
        let _ = app.emit(&event, progress);
    })
}

#[tauri::command]
fn lsp_managed_registry(state: State<'_, AppState>) -> Vec<lsp::managed::Card> {
    state.managed_lsp.registry_cards()
}

#[tauri::command]
fn lsp_managed_consent(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    server_id: String,
    decision: lsp::managed::consent::Decision,
    path: String,
) -> Result<lsp::LspStatus, String> {
    if is_ssh_session(&state, id) {
        return Ok(lsp::LspStatus::Unsupported);
    }
    state.managed_lsp.record_decision(&server_id, decision)?;
    if decision == lsp::managed::consent::Decision::Accept {
        state
            .managed_lsp
            .start_download(&server_id, managed_progress_emitter(&app))?;
    }
    let (root, _ctx) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    Ok(state.lsp.status(id, &path, &root))
}

#[tauri::command]
fn lsp_managed_download(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    server_id: String,
    path: String,
) -> Result<lsp::LspStatus, String> {
    if is_ssh_session(&state, id) {
        return Ok(lsp::LspStatus::Unsupported);
    }
    if state.managed_lsp.is_consented(&server_id) {
        state
            .managed_lsp
            .start_download(&server_id, managed_progress_emitter(&app))?;
    }
    let (root, _ctx) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    Ok(state.lsp.status(id, &path, &root))
}

#[tauri::command]
fn lsp_managed_use_mine(
    state: State<'_, AppState>,
    server_id: String,
) -> Result<lsp::InstallHints, String> {
    state
        .managed_lsp
        .record_decision(&server_id, lsp::managed::consent::Decision::UseMine)?;
    Ok(state.lsp.install_hints(&server_id))
}

#[tauri::command]
fn lsp_managed_download_status(
    state: State<'_, AppState>,
    server_id: String,
) -> Option<lsp::managed::Progress> {
    state.managed_lsp.progress(&server_id)
}

#[tauri::command]
fn lsp_change(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    changes: Vec<lsp::document::ContentChange>,
) -> Result<(), String> {
    if let Some((root, _)) = state.files.info(id) {
        files::resolve_within(&root, &path)?;
    }
    state.lsp.change(id, &path, changes);
    Ok(())
}

#[tauri::command]
fn lsp_did_save(state: State<'_, AppState>, id: SessionId, path: String) {
    state.lsp.did_save(id, &path);
}

#[tauri::command]
fn lsp_close_doc(state: State<'_, AppState>, id: SessionId, path: String) {
    state.lsp.close_doc(id, &path);
}

#[tauri::command]
fn lsp_completion(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    line: u32,
    character: u32,
) -> Vec<lsp::CompletionItem> {
    state.lsp.completion(id, &path, line, character)
}

#[tauri::command]
fn lsp_hover(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    line: u32,
    character: u32,
) -> Option<lsp::Hover> {
    state.lsp.hover(id, &path, line, character)
}

#[tauri::command]
fn lsp_definition(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    line: u32,
    character: u32,
) -> Vec<lsp::LocationIpc> {
    state.lsp.definition(id, &path, line, character)
}

#[tauri::command]
fn lsp_signature(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    line: u32,
    character: u32,
) -> Option<lsp::SignatureHelp> {
    state.lsp.signature(id, &path, line, character)
}

#[tauri::command]
fn close_side_view(
    app: AppHandle,
    state: State<'_, AppState>,
    workspace_id: layout::WorkspaceId,
) -> Result<(), String> {
    let previous = workspace_side_view(&state, workspace_id);
    state
        .layout
        .close_side_view(workspace_id)
        .map_err(|e| e.to_string())?;
    if let Some(session) = previous
        .as_deref()
        .and_then(|v| v.strip_prefix(layout::VIEW_AGENTS_PREFIX))
        .and_then(|s| s.parse::<SessionId>().ok())
    {
        state.subagents.disarm_panel(session);
    }
    close_orphaned_files_panel(&state, previous, None);
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn set_side_view_expanded(
    app: AppHandle,
    state: State<'_, AppState>,
    workspace_id: layout::WorkspaceId,
    expanded: bool,
) -> Result<(), String> {
    state
        .layout
        .set_side_view_expanded(workspace_id, expanded)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn set_side_view_ratio(
    app: AppHandle,
    state: State<'_, AppState>,
    workspace_id: layout::WorkspaceId,
    ratio: f64,
    commit: bool,
) -> Result<(), String> {
    state
        .layout
        .set_side_view_ratio(workspace_id, ratio, commit)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
async fn worktree_stage(
    state: State<'_, AppState>,
    id: SessionId,
    paths: Vec<String>,
) -> Result<(), String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::ops::stage(&ctx.path, &paths)
}

#[tauri::command]
async fn worktree_unstage(
    state: State<'_, AppState>,
    id: SessionId,
    paths: Vec<String>,
) -> Result<(), String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::ops::unstage(&ctx.path, &paths)
}

#[tauri::command]
async fn worktree_discard(
    state: State<'_, AppState>,
    id: SessionId,
    paths: Vec<String>,
) -> Result<(), String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::ops::discard(&ctx.path, &paths)
}

#[tauri::command]
async fn worktree_commit(
    state: State<'_, AppState>,
    id: SessionId,
    message: String,
) -> Result<(), String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::ops::commit(&ctx.path, &message)
}

#[tauri::command]
async fn worktree_push(state: State<'_, AppState>, id: SessionId) -> Result<String, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    worktree::ops::push(&ctx.path)
}

#[tauri::command]
async fn worktree_merge_preview(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<worktree::ops::MergePreview, AppError> {
    let wt = session_worktree(&state, id).map_err(session_setup_error)?;
    worktree::ops::merge_preview(&wt.path)
}

#[tauri::command]
async fn worktree_merge_materialize(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<(), AppError> {
    let wt = session_worktree(&state, id).map_err(session_setup_error)?;
    worktree::ops::materialize_conflict(&wt.path)
}

#[tauri::command]
async fn worktree_merge_into_base(
    state: State<'_, AppState>,
    id: SessionId,
    strategy: worktree::ops::MergeStrategy,
    message: Option<String>,
) -> Result<String, AppError> {
    let wt = session_worktree(&state, id).map_err(session_setup_error)?;
    worktree::ops::merge_into_base(&wt.path, strategy, message.as_deref())
}

fn session_setup_error(detail: String) -> AppError {
    AppError::new("session.unavailable").with("detail", detail)
}

async fn forge_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::new("forge.task_failed").with("detail", e.to_string()))?
}

#[tauri::command]
async fn forge_status(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Option<forge::ForgeStatus>, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    let root = ctx.path.clone();
    let branch = repo::branch(&ctx.path);
    forge_blocking(move || Ok(forge::status(&root, branch.as_deref()))).await
}

#[tauri::command]
async fn forge_pr_for_session(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Option<forge::PullRequest>, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    let Some(branch) = repo::branch(&ctx.path) else {
        return Ok(None);
    };
    let path = ctx.path;
    forge_blocking(move || forge::pr_for_branch(&path, &branch)).await
}

#[tauri::command]
async fn forge_pr_comments(
    state: State<'_, AppState>,
    id: SessionId,
    number: u64,
) -> Result<Vec<forge::ReviewComment>, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    let path = ctx.path;
    forge_blocking(move || forge::pr_comments(&path, number)).await
}

#[tauri::command]
async fn forge_create_pr(
    state: State<'_, AppState>,
    id: SessionId,
    title: String,
    body: String,
) -> Result<forge::PullRequest, AppError> {
    let wt = session_worktree(&state, id).map_err(session_setup_error)?;
    let path = wt.path.clone();
    forge_blocking(move || forge::create_pr(&path, &title, &body)).await
}

#[tauri::command]
async fn forge_pr_list(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Vec<forge::PullRequest>, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    let path = ctx.path;
    forge_blocking(move || forge::pr_list(&path)).await
}

/// Limite deliberado: 10 runs. O painel responde "e agora?", não é um histórico
/// de CI — e cada item a mais é rede e processo competindo com os agentes.
const WORKFLOW_RUN_LIMIT: u32 = 10;

/// `fresh` distingue as duas chamadas: o painel ABERTO precisa do estado real
/// (senão o poll mostraria o mesmo retrato pela vida do cache), e o
/// pré-carregamento aceita o cache — é ele que faz o painel abrir instantâneo
/// sem gastar um processo `gh` por troca de aba.
#[tauri::command]
async fn forge_workflow_runs(
    state: State<'_, AppState>,
    id: SessionId,
    fresh: bool,
) -> Result<Option<Vec<forge::WorkflowRun>>, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    let path = ctx.path;
    forge_blocking(move || {
        if fresh {
            forge::workflow_runs_fresh(&path, WORKFLOW_RUN_LIMIT)
        } else {
            forge::workflow_runs_cached(&path, WORKFLOW_RUN_LIMIT)
        }
    })
    .await
}

#[tauri::command]
async fn forge_workflow_jobs(
    state: State<'_, AppState>,
    id: SessionId,
    run_id: u64,
) -> Result<Vec<forge::WorkflowJob>, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    let path = ctx.path;
    forge_blocking(move || forge::workflow_jobs(&path, run_id)).await
}

/// Abre um arquivo do worktree como TEXTO — nunca "executa" o arquivo.
/// `open <arquivo>`/`xdg-open` rodariam um script/app deixado por um
/// agente no worktree com um clique; aqui só editor configurado ou
/// visualizador de texto. Path do webview: resolve dentro do worktree
/// e recusa qualquer coisa fora.
fn open_path_in_editor(full: std::path::PathBuf, editor: Option<String>) -> Result<(), String> {
    let gui_editor = editor
        .filter(|id| !id.is_empty())
        .and_then(|id| editor::detect().into_iter().find(|e| e.id == id))
        .filter(|e| !e.terminal);
    let mut cmd = match gui_editor {
        Some(e) => {
            let mut c = std::process::Command::new(e.path);
            c.arg(&full);
            c
        }
        None => {
            #[cfg(target_os = "macos")]
            {
                let mut c = std::process::Command::new("open");
                c.arg("-t").arg(&full);
                c
            }
            #[cfg(not(target_os = "macos"))]
            {
                return Err(
                    "configure um editor em Settings → Geral pra abrir arquivos".to_string()
                );
            }
        }
    };
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("não deu pra abrir o arquivo: {e}"))?;
    // Reap fora do caminho do comando: opener sai rápido, mas Child
    // largado vira zumbi no Unix.
    std::thread::Builder::new()
        .name("open-file-reap".into())
        .spawn(move || {
            let _ = child.wait();
        })
        .ok();
    Ok(())
}

#[tauri::command]
async fn open_worktree_file(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    editor: Option<String>,
) -> Result<(), String> {
    let wt = session_worktree(&state, id)?;
    let root = wt
        .path
        .canonicalize()
        .map_err(|e| format!("worktree inacessível: {e}"))?;
    let full = root
        .join(&path)
        .canonicalize()
        .map_err(|e| format!("arquivo inacessível: {e}"))?;
    if !full.starts_with(&root) {
        return Err("arquivo fora do worktree".into());
    }
    open_path_in_editor(full, editor)
}

#[tauri::command]
async fn files_open_external(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    editor: Option<String>,
) -> Result<(), String> {
    // Arquivo remoto não tem caminho local para o editor externo — o botão some
    // no painel remoto; aqui só barramos o caso por segurança.
    if is_ssh_session(&state, id) {
        return Err("arquivo remoto não tem caminho local para o editor externo".into());
    }
    let (root, _ctx) = state
        .files
        .ensure(&app, id, || resolve_files_root(&state, id))?;
    let full = files::resolve_within(&root, &path)?;
    let (_file, real) = files::open_verified(&root, &full)?;
    open_path_in_editor(real, editor)
}

#[tauri::command]
async fn lsp_open_external(path: String, editor: Option<String>) -> Result<(), String> {
    let full = std::path::PathBuf::from(&path);
    if !full.is_absolute() {
        return Err("caminho de goto-def não é absoluto".into());
    }
    let real = full
        .canonicalize()
        .map_err(|e| format!("arquivo inacessível: {e}"))?;
    open_path_in_editor(real, editor)
}

#[tauri::command]
async fn session_diff(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<worktree::diff::SessionDiff, String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::diff::session_diff(&ctx.path, &ctx.base_ref)
}

#[tauri::command]
async fn session_diff_hunks(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    scope: worktree::diff::DiffScope,
    old_path: Option<String>,
) -> Result<worktree::diff::FileHunks, String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::diff::file_hunks(&ctx.path, &ctx.base_ref, scope, &path, old_path.as_deref())
}

#[tauri::command]
async fn session_conflicts(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Option<worktree::conflicts::ConflictState>, String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::conflicts::session_conflicts(&ctx.path)
}

#[tauri::command]
async fn session_conflict_choose(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
    side: String,
) -> Result<(), AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    worktree::conflicts::choose_side(&ctx.path, &path, &side)
}

#[tauri::command]
async fn session_conflict_mark_resolved(
    state: State<'_, AppState>,
    id: SessionId,
    path: String,
) -> Result<(), AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    worktree::conflicts::mark_resolved(&ctx.path, &path)
}

#[tauri::command]
async fn session_branches(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<worktree::branches::BranchList, String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::branches::list(&ctx.path)
}

#[tauri::command]
async fn session_fetch(state: State<'_, AppState>, id: SessionId) -> Result<(), String> {
    let ctx = session_repo_context(&state, id)?;
    worktree::branches::fetch(&ctx.path)
}

#[tauri::command]
async fn suggest_commit_message(
    state: State<'_, AppState>,
    id: SessionId,
    agent: String,
) -> Result<String, AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    agent::suggest::suggest_commit_message(&ctx.path, &agent)
}

#[tauri::command]
async fn session_checkout(
    state: State<'_, AppState>,
    id: SessionId,
    branch: String,
    is_remote: bool,
) -> Result<(), AppError> {
    let ctx = session_repo_context(&state, id).map_err(session_setup_error)?;
    worktree::branches::checkout(&ctx.path, &branch, is_remote)
}

#[tauri::command]
async fn session_branch_diff(
    state: State<'_, AppState>,
    id: SessionId,
    branch: String,
) -> Result<worktree::diff::SessionDiff, String> {
    worktree::branches::validate_ref_name(&branch)?;
    let ctx = session_repo_context(&state, id)?;
    let base = worktree::branches::default_base(&ctx.path);
    let merge_base = worktree::branches::merge_base(&ctx.path, &base, &branch)?;
    worktree::diff::branch_diff(&ctx.path, &merge_base, &branch)
}

#[tauri::command]
async fn session_branch_hunks(
    state: State<'_, AppState>,
    id: SessionId,
    branch: String,
    path: String,
    old_path: Option<String>,
) -> Result<worktree::diff::FileHunks, String> {
    worktree::branches::validate_ref_name(&branch)?;
    let ctx = session_repo_context(&state, id)?;
    let base = worktree::branches::default_base(&ctx.path);
    let merge_base = worktree::branches::merge_base(&ctx.path, &base, &branch)?;
    worktree::diff::range_file_hunks(
        &ctx.path,
        &format!("{merge_base}..{branch}"),
        &path,
        old_path.as_deref(),
    )
}

fn known_worktree_paths(
    sessions: &SharedSessionManager,
) -> std::collections::HashSet<std::path::PathBuf> {
    sessions
        .list()
        .into_iter()
        .filter_map(|s| s.worktree.map(|wt| wt.path))
        .collect()
}

#[tauri::command]
fn write_to_session(state: State<'_, AppState>, id: SessionId, data: String) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| e.to_string())?;
    state.pty_pool.write(id, &bytes).map_err(|e| e.to_string())
}

/// Abre um Host Group inteiro como **panes de um workspace só** — é o que dá
/// sentido visual ao broadcast: digitar uma vez e conferir as N saídas lado a
/// lado. Um workspace por host deixaria a rajada acontecendo fora da tela.
#[tauri::command]
fn connect_host_group(
    app: AppHandle,
    state: State<'_, AppState>,
    host_ids: Vec<String>,
    name: String,
    color: Option<String>,
    group: Option<String>,
) -> Result<Vec<Session>, String> {
    let hosts = state.store.load_hosts().map_err(|e| e.to_string())?;
    let picked: Vec<_> = host_ids
        .iter()
        .filter_map(|id| hosts.iter().find(|h| &h.id == id))
        .collect();
    if picked.is_empty() {
        return Err(crate::error::AppError::new("ssh.host_not_found").to_string());
    }

    let home = crate::ssh::home_dir();
    let mut opened: Vec<Session> = Vec::new();
    let mut last_pane: Option<layout::PaneId> = None;

    for host in picked {
        let handle = app.clone();
        let session = state
            .sessions
            .create_ssh_session(
                app.clone(),
                &state.pty_pool,
                host.id.clone(),
                &host.alias,
                home.as_deref(),
                100,
                30,
                move |id| session_exited(&handle, id),
            )
            .map_err(|e| e.to_string())?;
        let _ = state
            .store
            .touch_host_connected(&host.id, chrono::Utc::now());
        gc_host(&state, &host.alias);

        match last_pane {
            None => {
                state
                    .layout
                    .create_workspace_tagged(
                        &name,
                        None,
                        session.id,
                        layout::Tag {
                            lock_name: true,
                            color: color.clone(),
                            group: group.clone(),
                        },
                    )
                    .map_err(|e| e.to_string())?;
            }
            Some(pane) => {
                // Vertical: as saídas ficam lado a lado, que é o ponto de olhar
                // N hosts ao mesmo tempo.
                state
                    .layout
                    .split_pane(pane, layout::SplitKind::V, session.id)
                    .map_err(|e| e.to_string())?;
            }
        }
        last_pane = state.layout.pane_of_session(session.id);
        opened.push(session);
    }
    emit_layout(&app, &state);
    Ok(opened)
}

/// Espelha tecla crua nos alvos. Sem gate de propósito: tecla não executa nada
/// — o que executa é o Enter, e é lá que o core decide (ver [`broadcast_submit`]).
#[tauri::command]
fn broadcast_write(
    state: State<'_, AppState>,
    ids: Vec<SessionId>,
    data: String,
) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| e.to_string())?;
    // Falha de um alvo não pode sumir calada: a tecla ir para 1 de 3 hosts e o
    // app dizer "ok" é como o usuário descobre tarde que a rajada não foi.
    let mut failed = Vec::new();
    for id in ids {
        if let Err(e) = state.pty_pool.write(id, &bytes) {
            failed.push(format!("{id}: {e}"));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(crate::error::AppError::new("broadcast.write_failed")
            .with("detail", failed.join("; "))
            .to_string())
    }
}

/// Enter da rajada: o core classifica e **recusa vermelho sem confirmação
/// humana**. A regra vive aqui, não na UI — o webview não dispara `rm -rf` em N
/// máquinas nem que queira (regra #4).
#[tauri::command]
fn broadcast_submit(
    state: State<'_, AppState>,
    ids: Vec<SessionId>,
    command: String,
    confirmed: bool,
) -> Result<ssh::broadcast::BroadcastVerdict, String> {
    let verdict = ssh::broadcast::decide(&command, ids.len(), confirmed);
    if let ssh::broadcast::BroadcastVerdict::Sent { .. } = verdict {
        for id in ids {
            let _ = state.pty_pool.write(id, b"\r");
        }
    }
    Ok(verdict)
}

fn session_cwd_of(state: &AppState, id: SessionId) -> Result<std::path::PathBuf, String> {
    let pid = state
        .pty_pool
        .leader_pid(id)
        .ok_or_else(|| format!("sessão sem processo vivo: {id}"))?;
    repo::process_cwd(pid).ok_or_else(|| "cwd da sessão indisponível".to_string())
}

#[tauri::command]
fn submit_rich_input(
    state: State<'_, AppState>,
    id: SessionId,
    text: String,
    submit: bool,
) -> Result<(), String> {
    let bracketed = state
        .pty_pool
        .bracketed_paste(id)
        .ok_or_else(|| format!("sessão não encontrada: {id}"))?;
    let strategy = state
        .sessions
        .get(id)
        .map(|s| agent::submit_strategy_for(&s.kind))
        .unwrap_or_default();
    let (normalized, _) = rich_input::normalize(&text);
    let payload =
        rich_input::plan_injection(&normalized, bracketed && strategy.use_bracketed_paste)?;
    if payload.is_empty() {
        return Ok(());
    }

    let _submitting = state.rich_input_submit.lock();
    state
        .pty_pool
        .write(id, &payload)
        .map_err(|e| e.to_string())?;
    if submit {
        if !strategy.delay.is_zero() {
            std::thread::sleep(strategy.delay);
        }
        state
            .pty_pool
            .write(id, strategy.submit_bytes)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn set_agent_match_pattern(pattern: String) -> bool {
    rich_input::agent_matcher().set_pattern(&pattern)
}

#[tauri::command]
fn prompt_mentions_sensitive(text: String) -> bool {
    rich_input::mentions_sensitive(&text)
}

#[tauri::command]
fn session_bracketed_paste(state: State<'_, AppState>, id: SessionId) -> bool {
    state.pty_pool.bracketed_paste(id).unwrap_or(false)
}

#[tauri::command]
fn session_rel_path(state: State<'_, AppState>, id: SessionId, path: String) -> String {
    match session_cwd_of(&state, id) {
        Ok(cwd) => rich_input::rel_path(std::path::Path::new(&path), &cwd),
        Err(_) => path,
    }
}

#[tauri::command]
fn list_worktree_files(
    state: State<'_, AppState>,
    id: SessionId,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    let cwd = session_cwd_of(&state, id)?;
    state
        .worktree_files
        .files_for(&cwd, &query, limit.unwrap_or(50).min(500))
}

#[tauri::command]
async fn attach_session(
    app: AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<(), String> {
    state
        .pty_pool
        .attach(&app, window.label(), id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn detach_session(window: tauri::Window, state: State<'_, AppState>, id: SessionId) {
    state.pty_pool.detach(window.label(), id);
}

#[tauri::command]
fn repo_snapshots(state: State<'_, AppState>) -> Vec<repo::RepoSnapshot> {
    state.repos.snapshots()
}

#[tauri::command]
async fn session_cwd(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Option<pty::SessionCwdPayload>, String> {
    let Some(pid) = state.pty_pool.leader_pid(id) else {
        return Ok(None);
    };
    Ok(repo::process_cwd(pid).map(|p| pty::SessionCwdPayload::of(&p)))
}

#[tauri::command]
fn resize_session(
    state: State<'_, AppState>,
    id: SessionId,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    state
        .pty_pool
        .resize(id, cols, rows)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_sessions(state: State<'_, AppState>) -> Result<Loaded<Vec<Session>>, String> {
    Ok(Loaded::read(&state, || state.sessions.list()))
}

#[tauri::command]
fn session_mark_seen(app: AppHandle, state: State<'_, AppState>, id: SessionId) {
    state.sessions.mark_seen(&app, id);
}

#[tauri::command]
fn dispose_session(app: AppHandle, state: State<'_, AppState>, id: SessionId) {
    teardown_agent_session(&app, &state, id);
    state.sessions.dispose(&state.pty_pool, id);
    close_files_panel(&state, id);
    let _ = state.layout.session_disposed(id);
    emit_layout(&app, &state);
}

#[tauri::command]
async fn layout_state(state: State<'_, AppState>) -> Result<Loaded<layout::LayoutState>, String> {
    Ok(Loaded::read(&state, || state.layout.state()))
}

#[derive(serde::Serialize)]
struct BootSnapshot {
    ready: bool,
    prefs: std::collections::HashMap<String, String>,
    sessions: Vec<Session>,
    layout: layout::LayoutState,
}

/// Tudo que o mount do front precisa, numa chamada.
///
/// Eram dezoito `invoke` — dezesseis deles `get_pref`, um `SELECT` cada, todos
/// disputando o mesmo `Mutex<Connection>`. Paralelo no JS, serial do lado de cá.
///
/// `ready: false` significa "a thread de boot ainda não terminou": as listas
/// podem estar vazias por isso, não por não haver nada. O front espera
/// [`boot::EVENT_READY`] e reconsulta.
#[tauri::command]
async fn boot_snapshot(state: State<'_, AppState>) -> Result<BootSnapshot, String> {
    // Lido primeiro de propósito — ver [`Loaded::read`].
    let ready = state.boot.is_ready();
    Ok(BootSnapshot {
        ready,
        prefs: state.store.prefs().map_err(|e| e.to_string())?,
        sessions: state.sessions.list(),
        layout: state.layout.state(),
    })
}

#[derive(serde::Serialize)]
struct SavedLaunchConfig {
    id: launch_config::LaunchConfigId,
    slug: String,
    secret_warnings: Vec<String>,
}

#[tauri::command]
fn list_launch_configs(
    state: State<'_, AppState>,
) -> Result<Vec<launch_config::LaunchConfig>, String> {
    let rows = state
        .store
        .load_launch_configs()
        .map_err(|e| e.to_string())?;
    Ok(launch_config::from_rows(&rows))
}

#[tauri::command]
fn save_launch_config(
    state: State<'_, AppState>,
    id: Option<launch_config::LaunchConfigId>,
    draft: launch_config::LaunchConfigDraft,
) -> Result<SavedLaunchConfig, String> {
    launch_config::validate(&draft).map_err(|e| e.to_string())?;

    let rows = state
        .store
        .load_launch_configs()
        .map_err(|e| e.to_string())?;
    let existing = launch_config::from_rows(&rows);
    let taken: std::collections::HashSet<String> = existing
        .iter()
        .filter(|c| Some(c.id) != id)
        .map(|c| c.slug.clone())
        .collect();

    let now = chrono::Utc::now();
    let previous = id.and_then(|id| existing.iter().find(|c| c.id == id));
    let config = launch_config::LaunchConfig {
        id: id.unwrap_or_else(uuid::Uuid::new_v4),
        slug: previous
            .map(|p| p.slug.clone())
            .unwrap_or_else(|| launch_config::unique_slug(&draft.name, &taken)),
        name: draft.name.trim().to_string(),
        repo_root: draft.repo_root.clone(),
        source: launch_config::ConfigSource::Local,
        slots: draft.slots.clone(),
        tabs: draft.tabs.clone(),
        created_at: previous.map(|p| p.created_at).unwrap_or(now),
        updated_at: now,
    };

    state
        .store
        .upsert_launch_config(&launch_config::to_rows(&config))
        .map_err(|e| e.to_string())?;

    Ok(SavedLaunchConfig {
        id: config.id,
        slug: config.slug,
        secret_warnings: launch_config::secret_warnings(&config.slots),
    })
}

#[tauri::command]
fn delete_launch_config(
    state: State<'_, AppState>,
    id: launch_config::LaunchConfigId,
) -> Result<(), String> {
    state
        .store
        .delete_launch_config(&id.to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn launch_config_seed(
    state: State<'_, AppState>,
    workspace_id: Option<layout::WorkspaceId>,
) -> Result<launch_config::SnapshotSeed, String> {
    let layout_state = state.layout.state();
    let workspace = match workspace_id.or(layout_state.active_workspace) {
        Some(id) => layout_state.workspaces.iter().find(|w| w.id == id),
        None => None,
    }
    .ok_or("nenhum workspace ativo")?;

    let repo_root = workspace
        .repo_root
        .clone()
        .ok_or("o workspace não está ancorado num repositório")?;
    let repo_root = std::path::PathBuf::from(&repo_root);

    launch_config::snapshot_workspace(workspace, &|session_id| {
        let session = state.sessions.get(session_id)?;
        let cwd_rel = session.cwd.as_ref().and_then(|cwd| {
            cwd.strip_prefix(&repo_root)
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
                .filter(|p| !p.is_empty())
        });
        Some((session.kind.clone(), cwd_rel, session.worktree.is_some()))
    })
    .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
struct SlotFailure {
    slot: String,
    message: String,
}

#[derive(serde::Serialize)]
struct AppliedLaunchConfig {
    workspace_id: layout::WorkspaceId,
    reused: bool,
    failures: Vec<SlotFailure>,
}

#[derive(Clone, serde::Serialize)]
struct PrefillPayload {
    session_id: SessionId,
    prompt: String,
}

struct SlotSpawn<'a> {
    config: &'a launch_config::LaunchConfig,
    repo_root: &'a std::path::Path,
    clean: bool,
    cols: u16,
    rows: u16,
}

/// `async` porque chama [`create_session`], que espera o boot — e porque cria
/// worktree pelo caminho isolado, o que é `git` em subprocesso. Nenhuma das duas
/// coisas pode acontecer na main thread.
async fn spawn_slot(
    app: &AppHandle,
    state: &State<'_, AppState>,
    ctx: &SlotSpawn<'_>,
    slot: &launch_config::Slot,
) -> Result<Session, String> {
    let SlotSpawn {
        config,
        repo_root,
        clean,
        cols,
        rows,
    } = *ctx;
    let cwd = match &slot.cwd_rel {
        Some(rel) if !rel.is_empty() && rel != "." => repo_root.join(rel),
        _ => repo_root.to_path_buf(),
    };

    if !slot.isolate {
        return create_session(
            app.clone(),
            state.clone(),
            CreateSessionOpts {
                kind: slot.kind.clone(),
                title: Some(slot.name.clone()),
                cwd: Some(cwd),
                cols,
                rows,
                worktree_task: None,
                attach_existing: true,
                shell: None,
                initial_prompt: slot.initial_prompt.clone(),
            },
        )
        .await;
    }

    let branch = launch_config::slot_branch(&config.slug, &slot.name);
    let existing = if clean {
        None
    } else {
        worktree::find_by_branch(repo_root, &branch).unwrap_or(None)
    };

    let worktree_path = match existing {
        Some(path) if path.exists() => path,
        Some(_) => {
            let _ = worktree::prune(repo_root);
            worktree::create_named(repo_root, &branch)?.path
        }
        None => {
            let branch = if clean {
                format!("{branch}-{}", uuid::Uuid::new_v4().simple())
            } else {
                branch
            };
            worktree::create_named(repo_root, &branch)?.path
        }
    };

    let sub = match &slot.cwd_rel {
        Some(rel) if !rel.is_empty() && rel != "." => worktree_path.join(rel),
        _ => worktree_path,
    };

    create_session(
        app.clone(),
        state.clone(),
        CreateSessionOpts {
            kind: slot.kind.clone(),
            title: Some(slot.name.clone()),
            cwd: Some(sub),
            cols,
            rows,
            worktree_task: None,
            attach_existing: true,
            shell: None,
            initial_prompt: slot.initial_prompt.clone(),
        },
    )
    .await
}

/// `async` pelo mesmo motivo de [`create_session`]: cada slot passa por ele, e
/// um comando síncrono seguraria a main thread durante toda a espera do boot e
/// toda a criação de worktree.
#[tauri::command]
async fn apply_launch_config(
    app: AppHandle,
    state: State<'_, AppState>,
    id: launch_config::LaunchConfigId,
    clean: Option<bool>,
    cols: u16,
    rows: u16,
) -> Result<AppliedLaunchConfig, String> {
    let clean = clean.unwrap_or(false);

    if !clean {
        if let Some(ws) = state.layout.workspace_of_launch_config(id) {
            state
                .layout
                .activate_workspace(ws)
                .map_err(|e| e.to_string())?;
            let _ = app.emit(layout::EVENT_CHANGED, state.layout.state());
            return Ok(AppliedLaunchConfig {
                workspace_id: ws,
                reused: true,
                failures: Vec::new(),
            });
        }
    }

    let stored = state
        .store
        .load_launch_configs()
        .map_err(|e| e.to_string())?;
    let config = launch_config::from_rows(&stored)
        .into_iter()
        .find(|c| c.id == id)
        .ok_or("configuração não encontrada")?;

    let repo_root = std::path::PathBuf::from(&config.repo_root);
    if !repo_root.is_dir() {
        return Err(format!("repositório indisponível: {}", config.repo_root));
    }

    let mut bindings: std::collections::HashMap<launch_config::SlotId, SessionId> =
        std::collections::HashMap::new();
    let mut prefills: Vec<PrefillPayload> = Vec::new();
    let mut failures: Vec<SlotFailure> = Vec::new();

    let ctx = SlotSpawn {
        config: &config,
        repo_root: &repo_root,
        clean,
        cols,
        rows,
    };
    for slot in &config.slots {
        match spawn_slot(&app, &state, &ctx, slot).await {
            Ok(session) => {
                if let Some(prompt) = &slot.initial_prompt {
                    prefills.push(PrefillPayload {
                        session_id: session.id,
                        prompt: prompt.clone(),
                    });
                }
                bindings.insert(slot.id, session.id);
            }
            Err(message) => failures.push(SlotFailure {
                slot: slot.name.clone(),
                message,
            }),
        }
    }

    if bindings.is_empty() {
        return Err("nenhum slot pôde subir".into());
    }

    let ws_id = uuid::Uuid::new_v4();
    let now = chrono::Utc::now().to_rfc3339();
    let mut layout_rows = layout::LayoutRows {
        workspaces: vec![layout::WorkspaceRow {
            id: ws_id.to_string(),
            name: config.name.clone(),
            name_locked: Some(1),
            repo_root: Some(config.repo_root.clone()),
            color: None,
            group_name: None,
            kind: Some("user".to_string()),
            launch_config_id: Some(config.id.to_string()),
            position: 0,
            active_tab: None,
            side_view: None,
            side_ratio: None,
            side_expanded: None,
            created_at: now.clone(),
        }],
        ..Default::default()
    };

    for (i, tab) in config.tabs.iter().enumerate() {
        let tab_id = uuid::Uuid::new_v4().to_string();
        let mut panes = Vec::new();
        launch_config::tree_to_rows(&tab_id, &tab.root, &mut panes);
        let bound =
            launch_config::bind_slots_to_sessions(&panes, &|slot| bindings.get(&slot).copied());
        layout_rows.tabs.push(layout::TabRow {
            id: tab_id,
            workspace_id: Some(ws_id.to_string()),
            title: tab.title.clone(),
            view: None,
            position: i as i64,
            active_pane: None,
            created_at: now.clone(),
        });
        layout_rows.panes.extend(bound);
    }

    let valid: std::collections::HashSet<SessionId> = bindings.values().copied().collect();
    let workspace = layout::rows_to_workspaces(&layout_rows, &valid)
        .pop()
        .ok_or("não foi possível montar o layout da configuração")?;
    let workspace_id = state
        .layout
        .insert_workspace(workspace)
        .map_err(|e| e.to_string())?;

    let _ = app.emit(layout::EVENT_CHANGED, state.layout.state());
    for prefill in prefills {
        let _ = app.emit(launch_config::EVENT_PREFILL, prefill);
    }

    Ok(AppliedLaunchConfig {
        workspace_id,
        reused: false,
        failures,
    })
}

#[tauri::command]
fn create_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    repo_root: Option<String>,
    session_id: SessionId,
    // Identidade junto na criação: renomear/colorir/agrupar em chamadas
    // separadas faz a sidebar piscar entre os estados intermediários.
    #[allow(unused_variables)] tag: Option<WorkspaceTag>,
) -> Result<layout::WorkspaceId, String> {
    let repo_root = repo_root.map(|r| {
        let expanded = session::expand_home(std::path::Path::new(&r));
        // Sem o prefixo verbatim (`\\?\`) que o canonicalize adiciona no Windows:
        // é este repo_root que a sidebar exibe.
        session::strip_verbatim_prefix(&repo::canonicalize_or(&expanded))
            .to_string_lossy()
            .into_owned()
    });
    let id = state
        .layout
        .create_workspace_tagged(
            &name,
            repo_root,
            session_id,
            tag.map(Into::into).unwrap_or_default(),
        )
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(id)
}

/// O painel de containers é singleton e o alvo dele muda: o workspace segue o
/// que está na tela (nome, cor e grupo do host, ou "Docker" quando é local).
/// Numa chamada só — nome/cor/grupo em três round-trips fazem a sidebar piscar.
#[tauri::command]
fn tag_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::WorkspaceId,
    name: String,
    tag: WorkspaceTag,
) -> Result<(), String> {
    state
        .layout
        .tag_workspace(id, &name, tag.into())
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
struct WorkspaceTag {
    #[serde(default)]
    lock_name: bool,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    group: Option<String>,
}

impl From<WorkspaceTag> for layout::Tag {
    fn from(t: WorkspaceTag) -> Self {
        layout::Tag {
            lock_name: t.lock_name,
            color: t.color,
            group: t.group,
        }
    }
}

#[tauri::command]
fn close_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::WorkspaceId,
) -> Result<(), String> {
    let bound = state
        .layout
        .close_workspace(id)
        .map_err(|e| e.to_string())?;
    dispose_all(&app, &state, &bound);
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn activate_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::WorkspaceId,
) -> Result<(), String> {
    state
        .layout
        .activate_workspace(id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn create_tab(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: SessionId,
    workspace_id: Option<layout::WorkspaceId>,
) -> Result<layout::TabId, String> {
    let id = state
        .layout
        .create_tab(session_id, workspace_id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(id)
}

#[tauri::command]
fn close_tab(app: AppHandle, state: State<'_, AppState>, id: layout::TabId) -> Result<(), String> {
    let bound = state.layout.close_tab(id).map_err(|e| e.to_string())?;
    dispose_shells(&state, &bound);
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn reconnect_ssh(app: AppHandle, state: State<'_, AppState>, id: SessionId) -> Result<(), String> {
    let session = state
        .sessions
        .get(id)
        .ok_or_else(|| crate::error::AppError::new("ssh.session_not_found").to_string())?;
    if !matches!(session.kind, SessionKind::Ssh { .. }) {
        return Err(crate::error::AppError::new("ssh.not_an_ssh_session").to_string());
    }
    // O canal SFTP do painel morreu com a conexão anterior; derruba o painel
    // (teardown unificado) para a próxima operação reconstruí-lo sobre a conexão
    // remultiplexada.
    close_files_panel(&state, id);
    reattach_or_finish(app, id);
    Ok(())
}

#[tauri::command]
fn rename_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::WorkspaceId,
    name: String,
) -> Result<(), String> {
    state
        .layout
        .rename_workspace(id, &name)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn set_workspace_color(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::WorkspaceId,
    color: Option<String>,
) -> Result<(), String> {
    state
        .layout
        .set_workspace_color(id, color)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn set_workspace_group(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::WorkspaceId,
    group: Option<String>,
) -> Result<(), String> {
    state
        .layout
        .set_workspace_group(id, group)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn new_window(app: AppHandle) -> Result<(), String> {
    let label = format!("tyba-{}", uuid::Uuid::new_v4().simple());
    let builder = tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::default())
        .title("Tyba")
        .inner_size(1100.0, 720.0);
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.decorations(false);
    builder.build().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn list_editors() -> Vec<editor::Editor> {
    editor::detect()
}

/// Shells que o usuário pode escolher ao abrir uma sessão. No Windows: PowerShell
/// 7, Windows PowerShell, Prompt de Comando e cada distro WSL instalada. Fora do
/// Windows: uma entrada com o `$SHELL` do usuário.
#[tauri::command]
fn list_shells() -> Vec<session::ShellOption> {
    session::available_shells()
}

#[tauri::command]
async fn get_pref(state: State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    if !key.starts_with("pref.") {
        return Err("chave de preferência inválida".into());
    }
    state.store.get_setting(&key).map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_pref(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    if !key.starts_with("pref.") {
        return Err("chave de preferência inválida".into());
    }
    state
        .store
        .set_setting(&key, &value)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BuildInfo {
    pub version: String,
    pub commit: String,
    pub commit_date: String,
    pub os: String,
    pub arch: String,
    pub webview: String,
}

fn build_info() -> BuildInfo {
    BuildInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        commit: option_env!("TYBA_COMMIT").unwrap_or_default().to_string(),
        commit_date: option_env!("TYBA_COMMIT_DATE")
            .unwrap_or_default()
            .to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        webview: tauri::webview_version().unwrap_or_default(),
    }
}

#[tauri::command]
fn app_build_info() -> BuildInfo {
    build_info()
}

#[tauri::command]
async fn update_check(state: State<'_, AppState>) -> Result<Option<update::UpdateStatus>, String> {
    let now = chrono::Utc::now().timestamp();
    Ok(update::check(&state.store, env!("CARGO_PKG_VERSION"), now).await)
}

#[tauri::command]
fn update_dismiss(state: State<'_, AppState>, version: String) -> Result<(), String> {
    update::dismiss(&state.store, &version)
}

#[tauri::command]
fn activate_tab(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::TabId,
) -> Result<(), String> {
    state.layout.activate_tab(id).map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn move_tab(
    app: AppHandle,
    state: State<'_, AppState>,
    id: layout::TabId,
    to: usize,
) -> Result<(), String> {
    state.layout.move_tab(id, to).map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn open_session_in_tab(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<(), String> {
    state
        .layout
        .open_session(session_id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn split_pane(
    app: AppHandle,
    state: State<'_, AppState>,
    pane_id: layout::PaneId,
    kind: layout::SplitKind,
    session_id: SessionId,
) -> Result<layout::PaneId, String> {
    let id = state
        .layout
        .split_pane(pane_id, kind, session_id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(id)
}

#[tauri::command]
fn close_pane(
    app: AppHandle,
    state: State<'_, AppState>,
    pane_id: layout::PaneId,
) -> Result<(), String> {
    let viewer_session = state.layout.agent_viewer_session(pane_id);
    let unbound = state
        .layout
        .close_pane(pane_id)
        .map_err(|e| e.to_string())?;
    match viewer_session {
        Some(session) => state.subagents.disarm_viewer(session),
        None => dispose_shells(&state, &unbound),
    }
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn focus_pane(
    app: AppHandle,
    state: State<'_, AppState>,
    pane_id: layout::PaneId,
) -> Result<(), String> {
    state
        .layout
        .focus_pane(pane_id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn set_split_ratio(
    app: AppHandle,
    state: State<'_, AppState>,
    pane_id: layout::PaneId,
    ratio: f64,
    commit: Option<bool>,
) -> Result<(), String> {
    state
        .layout
        .set_split_ratio(pane_id, ratio, commit.unwrap_or(true))
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
// Docker num host remoto fala por ssh: a chamada leva segundos, não milissegundos.
// Comando síncrono do Tauri roda na main thread e congelaria a janela inteira —
// terminal junto. O trabalho bloqueante sai da main thread.
async fn docker_blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn docker_available(
    state: State<'_, AppState>,
    host: Option<String>,
) -> Result<bool, String> {
    let docker = Arc::clone(&state.docker);
    docker_blocking(move || docker.available(host.as_deref())).await
}

#[tauri::command]
async fn docker_list_containers(
    state: State<'_, AppState>,
    repo_root: Option<String>,
    all: bool,
    host: Option<String>,
) -> Result<Vec<docker::ContainerInfo>, String> {
    let docker = Arc::clone(&state.docker);
    docker_blocking(move || {
        docker
            // Filtro por projeto compara caminho local do compose: num host
            // remoto o caminho é de lá, então filtrar por repo daqui zeraria a
            // lista.
            .list(
                if host.is_some() {
                    None
                } else {
                    repo_root.as_deref()
                },
                all,
                host.as_deref(),
            )
            .map_err(|e| e.to_string())
    })
    .await?
}

fn open_container_tab(
    app: &AppHandle,
    state: &State<'_, AppState>,
    container_id: &str,
    tab: docker::ContainerTab,
    host: Option<&str>,
) -> Result<(), String> {
    let name = state
        .docker
        .container_name(container_id)
        .map_err(|e| e.to_string())?;

    if let Some(existing) = state.docker.tab_session(container_id, tab) {
        if state.sessions.get(existing).is_some() {
            state
                .layout
                .open_session(existing)
                .map_err(|e| e.to_string())?;
            emit_layout(app, state);
            return Ok(());
        }
    }
    // Container remoto mora dentro do host: cai no grupo e na cor daquele host,
    // com o alias no nome. O workspace do Docker é o balaio da máquina local —
    // misturar os dois deixa `sh: postgres` do Mac igual ao da VPS, e o estrago
    // de confundir é grande.
    let remote = host.and_then(|alias| {
        let host = state
            .store
            .load_hosts()
            .ok()?
            .into_iter()
            .find(|h| h.alias == alias)?;
        Some(host)
    });
    let workspace_id = match &remote {
        Some(_) => None,
        None => Some(state.layout.docker_workspace().map_err(|e| e.to_string())?),
    };

    let bin = docker::docker_bin().ok_or("binário docker não encontrado")?;
    let (args, title) = match tab {
        docker::ContainerTab::Logs => (
            vec![
                "logs".to_string(),
                "-f".to_string(),
                "--tail".to_string(),
                "200".to_string(),
                container_id.to_string(),
            ],
            format!("logs: {name}"),
        ),
        docker::ContainerTab::Shell => (
            vec![
                "exec".to_string(),
                "-it".to_string(),
                container_id.to_string(),
                "sh".to_string(),
                "-c".to_string(),
                "command -v bash >/dev/null && exec bash || exec sh".to_string(),
            ],
            format!("sh: {name}"),
        ),
    };

    // `-H` é flag global do docker e vem antes do subcomando: evita carregar
    // env pelo spawn e deixa o alvo explícito na própria linha de comando.
    let args = match docker::docker_host_env(host) {
        Some(target) => {
            let mut with_host = vec!["-H".to_string(), target];
            with_host.extend(args);
            with_host
        }
        None => args,
    };

    let session = match &remote {
        // Container do host tem workspace próprio: `create_tab` sem alvo cai no
        // workspace ATIVO — foi assim que o shell entrou (e renomeou) a sessão
        // ssh do usuário em vez de nascer do lado dela.
        Some(h) => {
            let title = format!("{} · {title}", h.alias);
            let handle = app.clone();
            let session = state
                .sessions
                .create_command_session(
                    app.clone(),
                    &state.pty_pool,
                    bin.as_path(),
                    &args,
                    title.clone(),
                    None,
                    100,
                    30,
                    SessionKind::Container {
                        host_id: Some(h.id.clone()),
                        container_id: container_id.to_string(),
                    },
                    move |id| session_exited(&handle, id),
                )
                .map_err(|e| e.to_string())?;
            let group = h.group_id.as_ref().and_then(|gid| {
                state
                    .store
                    .load_host_groups()
                    .ok()?
                    .into_iter()
                    .find(|g| &g.id == gid)
                    .map(|g| g.name)
            });
            state
                .layout
                .create_workspace_tagged(
                    &title,
                    None,
                    session.id,
                    layout::Tag {
                        lock_name: true,
                        color: h.color.clone(),
                        group,
                    },
                )
                .map_err(|e| e.to_string())?;
            emit_layout(app, state);
            session.id
        }
        None => spawn_tab_session(
            app,
            state,
            bin.as_path(),
            &args,
            title.clone(),
            None,
            &title,
            workspace_id,
            SessionKind::Container {
                host_id: None,
                container_id: container_id.to_string(),
            },
        )?,
    };
    state.docker.remember_tab(container_id, tab, session);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn spawn_tab_session(
    app: &AppHandle,
    state: &State<'_, AppState>,
    program: &std::path::Path,
    args: &[String],
    title: String,
    cwd: Option<&std::path::Path>,
    fallback_workspace: &str,
    workspace_id: Option<layout::WorkspaceId>,
    kind: SessionKind,
) -> Result<SessionId, String> {
    let handle = app.clone();
    let session = state
        .sessions
        .create_command_session(
            app.clone(),
            &state.pty_pool,
            program,
            args,
            title,
            cwd,
            100,
            30,
            kind,
            move |id| session_exited(&handle, id),
        )
        .map_err(|e| e.to_string())?;

    if let Err(e) = state.layout.create_tab(session.id, workspace_id) {
        if matches!(e, layout::LayoutError::NoActiveWorkspace) {
            state
                .layout
                .create_workspace(fallback_workspace, None, session.id)
                .map_err(|e| e.to_string())?;
        } else {
            return Err(e.to_string());
        }
    }
    emit_layout(app, state);
    Ok(session.id)
}

#[tauri::command]
fn docker_open_logs(
    app: AppHandle,
    state: State<'_, AppState>,
    container_id: String,
    host: Option<String>,
) -> Result<(), String> {
    open_container_tab(
        &app,
        &state,
        &container_id,
        docker::ContainerTab::Logs,
        host.as_deref(),
    )
}

#[tauri::command]
fn docker_open_shell(
    app: AppHandle,
    state: State<'_, AppState>,
    container_id: String,
    host: Option<String>,
) -> Result<(), String> {
    open_container_tab(
        &app,
        &state,
        &container_id,
        docker::ContainerTab::Shell,
        host.as_deref(),
    )
}

#[tauri::command]
fn open_view_tab(app: AppHandle, state: State<'_, AppState>, view: String) -> Result<(), String> {
    if view != layout::VIEW_SETTINGS
        && view != layout::VIEW_WORKSPACE
        && view != layout::VIEW_CONNECTIONS
    {
        return Err(format!("view desconhecida: {view}"));
    }
    state
        .layout
        .open_view_tab(&view)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn docker_open_dashboard(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state
        .layout
        .open_docker_dashboard()
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn docker_compose_op(
    app: AppHandle,
    state: State<'_, AppState>,
    project: String,
    op: docker::ComposeOp,
) -> Result<(), String> {
    let info = state
        .docker
        .project_info(&project)
        .map_err(|e| e.to_string())?;
    docker::validate_working_dir(&info.working_dir).map_err(|e| e.to_string())?;
    let bin = docker::docker_bin().ok_or("binário docker não encontrado")?;
    let workspace_id = Some(state.layout.docker_workspace().map_err(|e| e.to_string())?);

    let script = format!(
        "'{}' {}; ec=$?; if [ $ec -ne 0 ]; then printf '\\n[falhou — enter para fechar]\\n'; read _; fi",
        bin.display(),
        op.compose_args(),
    );
    let title = format!("compose {}: {}", op.label(), project);
    spawn_tab_session(
        &app,
        &state,
        std::path::Path::new("/bin/sh"),
        &["-c".to_string(), script],
        title,
        Some(std::path::Path::new(&info.working_dir)),
        &project,
        workspace_id,
        SessionKind::Shell,
    )?;
    Ok(())
}

#[tauri::command]
fn docker_open_project(
    app: AppHandle,
    state: State<'_, AppState>,
    project: String,
) -> Result<(), String> {
    let info = state
        .docker
        .project_info(&project)
        .map_err(|e| e.to_string())?;
    docker::validate_working_dir(&info.working_dir).map_err(|e| e.to_string())?;

    let existing = state
        .layout
        .state()
        .workspaces
        .iter()
        .find(|w| w.repo_root.as_deref() == Some(info.working_dir.as_str()))
        .map(|w| w.id);
    if let Some(id) = existing {
        state
            .layout
            .activate_workspace(id)
            .map_err(|e| e.to_string())?;
        emit_layout(&app, &state);
        return Ok(());
    }

    let handle = app.clone();
    let session = state
        .sessions
        .create_shell_session(
            app.clone(),
            &state.pty_pool,
            CreateSessionOpts {
                kind: SessionKind::Shell,
                title: None,
                cwd: Some(std::path::PathBuf::from(&info.working_dir)),
                cols: 100,
                rows: 30,
                worktree_task: None,
                attach_existing: false,
                shell: None,
                initial_prompt: None,
            },
            move |id| session_exited(&handle, id),
        )
        .map_err(|e| e.to_string())?;
    state
        .layout
        .create_workspace(&project, Some(info.working_dir), session.id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn docker_open_compose_file(
    app: AppHandle,
    state: State<'_, AppState>,
    project: String,
) -> Result<(), String> {
    let info = state
        .docker
        .project_info(&project)
        .map_err(|e| e.to_string())?;
    let files = info.config_files;
    if files.is_empty() {
        return Err("projeto sem arquivo compose".into());
    }
    for file in &files {
        docker::validate_compose_file(file).map_err(|e| e.to_string())?;
    }
    docker::validate_working_dir(&info.working_dir).map_err(|e| e.to_string())?;
    let workspace_id = Some(state.layout.docker_workspace().map_err(|e| e.to_string())?);

    let shell = session::default_shell();
    let quoted = files
        .iter()
        .map(|file| format!("'{file}'"))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!("exec \"${{EDITOR:-vi}}\" {quoted}");
    let title = format!("compose: {project}");
    spawn_tab_session(
        &app,
        &state,
        std::path::Path::new(&shell),
        &["-lc".to_string(), script],
        title,
        Some(std::path::Path::new(&info.working_dir)),
        &project,
        workspace_id,
        SessionKind::Shell,
    )?;
    Ok(())
}

#[tauri::command]
async fn docker_remove_container(
    state: State<'_, AppState>,
    container_id: String,
    host: Option<String>,
) -> Result<(), String> {
    let docker = Arc::clone(&state.docker);
    docker_blocking(move || {
        docker
            .remove(&container_id, host.as_deref())
            .map_err(|e| e.to_string())
    })
    .await?
}

#[tauri::command]
fn docker_open_desktop() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let attempts: [&[&str]; 2] = [&["-a", "Docker"], &["-b", "com.docker.docker"]];
        let mut last_error = String::new();
        for args in attempts {
            match std::process::Command::new("/usr/bin/open")
                .args(args)
                .output()
            {
                Ok(out) if out.status.success() => return Ok(()),
                Ok(out) => {
                    last_error = String::from_utf8_lossy(&out.stderr).trim().to_string();
                }
                Err(e) => last_error = e.to_string(),
            }
        }
        if last_error.is_empty() {
            last_error = "não foi possível abrir o Docker Desktop".into();
        }
        Err(last_error)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("disponível apenas no macOS".into())
    }
}

#[tauri::command]
fn request_approval(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: SessionId,
    command: String,
    cwd: Option<String>,
    context: Option<String>,
) -> Result<ApprovalRequest, String> {
    state
        .approvals
        .request(&app, session_id, command, cwd, context)
}

#[tauri::command]
fn list_approvals(state: State<'_, AppState>) -> Vec<ApprovalRequest> {
    state.approvals.list_pending()
}

#[tauri::command]
fn resolve_approval(
    app: AppHandle,
    state: State<'_, AppState>,
    id: u64,
    decision: Decision,
    feedback: Option<String>,
) -> Result<(), String> {
    let request = state.approvals.resolve(&app, id, decision, feedback)?;
    agent::session::record_history(
        &state.store,
        request.session_id,
        request.command,
        request.cwd,
        request.risk,
        agent::session::decision_label(decision),
        request.requested_at_ms,
    );
    Ok(())
}

#[tauri::command]
fn list_subagents(
    state: State<'_, AppState>,
    session_id: SessionId,
) -> agent::subagents::SubagentSnapshot {
    state.subagents.snapshot(session_id)
}

#[tauri::command]
fn focus_subagent(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: SessionId,
    agent_id: String,
) -> agent::subagents::SubagentSnapshot {
    state.subagents.focus(&app, session_id, agent_id);
    state.subagents.snapshot(session_id)
}

/// Mata o agente cru detectado numa sessão de shell (F3 do
/// detectar-agente-no-shell) e devolve o cwd do agente pro front encadear o
/// attach gerenciado. Valida ANTES de matar que a pasta é um repo git — nunca
/// derruba o agente pra depois falhar o reopen. O shell da sessão sobrevive.
#[tauri::command]
async fn kill_shell_agent(
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<String, crate::error::AppError> {
    let agent = state
        .agent_prober
        .detected(session_id)
        .ok_or_else(|| crate::error::AppError::new("agent.reopen_not_detected"))?;
    let leader = state.pty_pool.leader_pid(session_id);
    let cwd = repo::process_cwd(agent.pid)
        .or_else(|| leader.and_then(repo::process_cwd))
        .ok_or_else(|| crate::error::AppError::new("agent.reopen_no_cwd"))?;
    if repo::toplevel(&cwd).is_none() {
        return Err(
            crate::error::AppError::new("agent.reopen_not_git").with("path", cwd.to_string_lossy())
        );
    }
    // O grace do SIGTERM (~1,5s) roda fora da thread do command — mesmo padrão
    // do painel remoto: worker do Tokio nunca dorme.
    tauri::async_runtime::spawn_blocking(move || {
        agent::process_probe::terminate_detected(agent.pid, agent.start_ms, leader)
    })
    .await
    .map_err(|e| {
        crate::error::AppError::new("agent.reopen_kill_failed").with("detail", e.to_string())
    })??;
    Ok(cwd.to_string_lossy().into_owned())
}

#[tauri::command]
fn open_agents_panel(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<(), String> {
    let previous = session_side_view(&state, id);
    state
        .layout
        .open_workspace_side_view(id, &layout::agents_view(id))
        .map_err(|e| e.to_string())?;
    close_orphaned_files_panel(&state, previous, None);
    emit_layout(&app, &state);
    Ok(())
}

/// Fecha os splits de viewer de subagente da sessão — o par do auto-fechar do
/// painel Agentes no fim da rodada: viewer e painel são uma feature só.
#[tauri::command]
fn close_agent_viewers(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<(), String> {
    for pane in state.layout.agent_viewer_panes(session_id) {
        let _ = state.layout.close_pane(pane);
    }
    emit_layout(&app, &state);
    Ok(())
}

#[tauri::command]
fn open_subagent_viewer(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<(), String> {
    state
        .layout
        .open_agent_viewer(session_id)
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
}

#[derive(serde::Serialize)]
struct SubagentTranscript {
    entries: Vec<status::subagent_transcript::TranscriptEntry>,
    cursor: u64,
    complete: bool,
}

#[tauri::command]
fn subagent_transcript(
    state: State<'_, AppState>,
    session_id: SessionId,
    agent_id: String,
    cursor: Option<u64>,
) -> Result<SubagentTranscript, String> {
    let path = state
        .subagents
        .resolve_transcript_path(session_id, &agent_id)
        .ok_or("subagente sem transcript disponível")?;
    let (entries, cursor, complete) = status::subagent_transcript::read_entries(
        &path,
        cursor,
        status::subagent_transcript::TAIL_ENTRIES,
    );
    Ok(SubagentTranscript {
        entries,
        cursor,
        complete,
    })
}

#[derive(serde::Serialize)]
struct AgentConfigInfo {
    hash: String,
    default_agent: Option<String>,
    env_allow: Vec<String>,
    consent: Option<bool>,
}

/// Fora do threadpool de blocking isto é uma bomba armada: por baixo,
/// `agent_path()` resolve o PATH de login spawnando `$SHELL -lic` — shell
/// **interativo**, roda o `.zshrc` inteiro — com busy-wait de até 3 s, e é
/// `OnceLock`, então a primeira chamada segura quem passar por ela. Nesta
/// máquina, `zsh -lic` custa ~660 ms. Na main thread isso era beachball; num
/// worker do runtime seria um worker sequestrado.
#[tauri::command]
async fn agent_binary_available(runner: crate::session::AgentRunnerKind) -> bool {
    tauri::async_runtime::spawn_blocking(move || agent::binary_available(&runner))
        .await
        .unwrap_or(false)
}

/// Agente (claude/codex) detectado rodando na sessão de shell `session_id`, se
/// houver. Estado alimentado pelo poll de [`poll_agent_probers`].
#[tauri::command]
async fn detected_agent(
    state: State<'_, AppState>,
    session_id: SessionId,
) -> Result<Option<agent::process_probe::DetectedAgent>, String> {
    Ok(state.agent_prober.detected(session_id))
}

#[tauri::command]
fn agent_repo_config(
    state: State<'_, AppState>,
    repo_root: String,
) -> Result<Option<AgentConfigInfo>, String> {
    let param = repo::canonicalize_or(std::path::Path::new(&repo_root));
    let root = repo::toplevel(&param)
        .map(|t| repo::canonicalize_or(&t))
        .ok_or("fora de repositório git")?;
    let Some((config, hash)) = repo_config::load(&root).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let consent = state
        .store
        .config_consent(&root.to_string_lossy(), &hash)
        .ok()
        .flatten();
    Ok(Some(AgentConfigInfo {
        hash,
        default_agent: config.default_agent,
        env_allow: config.env_allow,
        consent,
    }))
}

#[tauri::command]
fn set_agent_config_consent(
    state: State<'_, AppState>,
    repo_root: String,
    hash: String,
    allowed: bool,
) -> Result<(), String> {
    let param = repo::canonicalize_or(std::path::Path::new(&repo_root));
    let root = repo::toplevel(&param)
        .map(|t| repo::canonicalize_or(&t))
        .ok_or("fora de repositório git")?;
    state
        .store
        .set_config_consent(&root.to_string_lossy(), &hash, allowed)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_app_menu(app: AppHandle, spec: menu::MenuSpec) -> Result<(), String> {
    menu::install(&app, &spec)
}

/// Limpa a linha do shell antes de escrever nela (`bindkey '\e=' kill-buffer`).
/// Em modo prompt o usuário não digita ali, mas um widget do próprio zsh ou um
/// toggle no meio do caminho pode ter deixado texto — que se somaria ao nosso.
const KILL_LINE: &[u8] = b"\x1b=";

/// Alterna o modo prompt dentro da sessão viva (`bindkey '\e~'`). É a válvula
/// de escape: toda heurística falha, e o caminho de volta não pode ser fechar o
/// app.
const TOGGLE_PROMPT_MODE: &[u8] = b"\x1b~";

#[tauri::command]
fn submit_shell_line(
    state: State<'_, AppState>,
    id: SessionId,
    text: String,
) -> Result<(), String> {
    let bracketed = state
        .pty_pool
        .bracketed_paste(id)
        .ok_or_else(|| format!("sessão não encontrada: {id}"))?;
    let (normalized, _) = rich_input::normalize(&text);
    if normalized.trim().is_empty() {
        return Ok(());
    }
    let payload = rich_input::plan_injection(&normalized, bracketed)?;

    let _submitting = state.rich_input_submit.lock();
    let mut bytes = Vec::with_capacity(KILL_LINE.len() + payload.len() + 1);
    bytes.extend_from_slice(KILL_LINE);
    bytes.extend_from_slice(&payload);
    bytes.push(b'\n');
    state.pty_pool.write(id, &bytes).map_err(|e| e.to_string())
}

/// Bytes crus para o PTY: sinais (Ctrl+C/D/Z) que a linha do TYBA nunca consome.
#[tauri::command]
fn write_control(state: State<'_, AppState>, id: SessionId, bytes: String) -> Result<(), String> {
    if bytes.len() > 8 || !bytes.chars().all(|c| c.is_control()) {
        return Err("apenas caracteres de controle".into());
    }
    state
        .pty_pool
        .write(id, bytes.as_bytes())
        .map_err(|e| e.to_string())
}

/// Consulta o modo prompt em vez de esperar o evento: quem assina depois do
/// primeiro prompt nunca receberia o `633;P` e ficaria sem a linha de comando.
/// Blocos já gravados de uma sessão, para reabrir mostrando o que aconteceu
/// antes. Nada é gravado sem ser usado (ADR de 2026-07-10).
#[tauri::command]
async fn session_blocks(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<Vec<blocks::Block>, String> {
    state
        .store
        .list_blocks(&id.to_string(), 200)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn session_prompt_mode(state: State<'_, AppState>, id: SessionId) -> bool {
    state.pty_pool.prompt_mode(id).unwrap_or(false)
}

/// O tty está entregando linhas (eco ligado) ou teclas (raw)?
///
/// Decide para onde vai a seta enquanto um comando roda: em modo linha ela é
/// byte literal que ninguém interpreta e ainda é ecoada, virando `^[[A` na
/// saída gravada do bloco. Ver `PtyPool::line_echo`.
///
/// `false` quando não há como saber (Windows, sessão morta): é o estado de
/// sempre, com a tecla indo para o PTY.
#[tauri::command]
fn session_line_echo(state: State<'_, AppState>, id: SessionId) -> bool {
    state.pty_pool.line_echo(id).unwrap_or(false)
}

#[tauri::command]
fn toggle_prompt_mode(state: State<'_, AppState>, id: SessionId) -> Result<(), String> {
    state
        .pty_pool
        .write(id, TOGGLE_PROMPT_MODE)
        .map_err(|e| e.to_string())
}

pub const HISTORY_PREF_KEY: &str = "pref.commandHistory";

fn history_enabled(store: &Store) -> bool {
    store
        .get_setting(HISTORY_PREF_KEY)
        .ok()
        .flatten()
        .map(|value| value != "off")
        .unwrap_or(true)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryHit {
    command: String,
    cwd: Option<String>,
    uses: u32,
    last_used_at_ms: i64,
    in_cwd: bool,
    in_repo: bool,
    /// Nunca saiu com exit code 0. Continua no histórico e ainda completa no
    /// ghost text quando é prefixo do que o usuário digitou — mas não é
    /// oferecido numa lista: `lljh` foi um erro de digitação, e devolvê-lo como
    /// opção é sugerir o próprio engano.
    failed: bool,
}

/// Fuzzy + frecência no core: o webview recebe a lista já ordenada (princípio #1).
#[tauri::command]
async fn search_command_history(
    state: State<'_, AppState>,
    query: String,
    cwd: Option<String>,
    repo_root: Option<String>,
    limit: usize,
) -> Result<Vec<HistoryHit>, String> {
    history_hits(&state, &query, cwd.as_deref(), repo_root.as_deref(), limit)
}

/// O corpo do comando, síncrono, para quem já está numa thread — `suggest_line`
/// chama daqui em vez de `.await` no comando só para não virar duas travessias.
fn history_hits(
    state: &AppState,
    query: &str,
    cwd: Option<&str>,
    repo_root: Option<&str>,
    limit: usize,
) -> Result<Vec<HistoryHit>, String> {
    use fuzzy_matcher::skim::SkimMatcherV2;
    use fuzzy_matcher::FuzzyMatcher;

    let candidates = state
        .store
        .history_candidates(cwd, repo_root)
        .map_err(|e| e.to_string())?;
    let matcher = SkimMatcherV2::default();
    let query = query.trim();
    let now = approvals::now_ms() as i64;
    let mut scored: Vec<(f64, HistoryHit)> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let fuzzy = if query.is_empty() {
                1.0
            } else {
                matcher.fuzzy_match(&candidate.command, query)? as f64
            };
            let score = fuzzy * history::frecency(now, &candidate);
            Some((
                score,
                HistoryHit {
                    failed: candidate.uses > 0 && candidate.successes == 0,
                    command: candidate.command,
                    cwd: candidate.cwd,
                    uses: candidate.uses,
                    last_used_at_ms: candidate.last_used_at_ms,
                    in_cwd: candidate.in_cwd,
                    in_repo: candidate.in_repo,
                },
            ))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(scored
        .into_iter()
        .take(limit.min(500))
        .map(|(_, hit)| hit)
        .collect())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CommandSuggestion {
    command: String,
    /// Só serve para o ghost text; a lista filtra.
    failed: bool,
    /// `history` ou `snippet` — a UI marca a origem; o usuário precisa saber que
    /// aquele comando é um snippet, não algo que ele já rodou.
    kind: &'static str,
    label: Option<String>,
}

const SUGGEST_LIMIT: usize = 8;

/// O que aparece embaixo da linha de comando: histórico ranqueado por frecência
/// e snippets, numa lista só. O ranking fica no core (princípio #1) — o webview
/// recebe pronto e só desenha.
#[tauri::command]
async fn suggest_commands(
    state: State<'_, AppState>,
    query: String,
    cwd: Option<String>,
    repo_root: Option<String>,
) -> Result<Vec<CommandSuggestion>, String> {
    command_suggestions(&state, &query, cwd.as_deref(), repo_root.as_deref())
}

fn command_suggestions(
    state: &AppState,
    query: &str,
    cwd: Option<&str>,
    repo_root: Option<&str>,
) -> Result<Vec<CommandSuggestion>, String> {
    use fuzzy_matcher::skim::SkimMatcherV2;
    use fuzzy_matcher::FuzzyMatcher;

    let query = query.trim();
    let matcher = SkimMatcherV2::default();
    let mut out: Vec<CommandSuggestion> = Vec::new();

    // Snippet primeiro quando o nome casa: foi o usuário que o nomeou, então é
    // mais intencional do que um comando que ele rodou uma vez sem querer.
    if !query.is_empty() {
        for snippet in state.store.list_snippets().unwrap_or_default() {
            if matcher.fuzzy_match(&snippet.name, query).is_some()
                || snippet.command.starts_with(query)
            {
                out.push(CommandSuggestion {
                    command: snippet.command,
                    failed: false,
                    kind: "snippet",
                    label: Some(snippet.name),
                });
            }
            if out.len() >= 3 {
                break;
            }
        }
    }

    let hits = history_hits(state, query, cwd, repo_root, SUGGEST_LIMIT)?;
    for hit in hits {
        if out.iter().any(|s| s.command == hit.command) {
            continue;
        }
        out.push(CommandSuggestion {
            command: hit.command,
            failed: hit.failed,
            kind: "history",
            label: None,
        });
    }
    out.truncate(SUGGEST_LIMIT);
    Ok(out)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LineSuggestions {
    commands: Vec<CommandSuggestion>,
    paths: Vec<String>,
    arguments: Vec<String>,
}

/// Uma chamada por tecla, não três.
///
/// Histórico, caminho e argumento eram três `invoke` separados a cada
/// digitação: três travessias da ponte do webview e três consultas, para uma
/// única mudança de estado na tela.
#[tauri::command]
async fn suggest_line(
    state: State<'_, AppState>,
    query: String,
    cwd: Option<String>,
    repo_root: Option<String>,
    path_token: Option<String>,
    arg_prefix: Option<String>,
    arg_token: Option<String>,
) -> Result<LineSuggestions, String> {
    let paths = match (&cwd, &path_token) {
        (Some(cwd), Some(token)) if !cwd.is_empty() => {
            completion::complete_path(std::path::Path::new(cwd), token)
        }
        _ => Vec::new(),
    };
    let arguments = match (&arg_prefix, &arg_token) {
        (Some(prefix), Some(token)) => {
            let commands = state
                .store
                .history_with_prefix(prefix, 400)
                .map_err(|e| e.to_string())?;
            completion::next_tokens(&commands, prefix, token)
        }
        _ => Vec::new(),
    };
    let commands = command_suggestions(&state, &query, cwd.as_deref(), repo_root.as_deref())?;
    Ok(LineSuggestions {
        commands,
        paths,
        arguments,
    })
}

/// O `cwd` vem do front, que o recebe por `OSC 7` — atacante-controlável, e por
/// isso **display-only** (ADR de 2026-07-08). Aqui ele só escolhe qual diretório
/// listar para sugerir: nenhuma decisão de segurança sai daqui, e o usuário
/// alcança qualquer caminho digitando de qualquer jeito.
#[tauri::command]
fn complete_path(cwd: String, token: String) -> Vec<String> {
    if cwd.is_empty() {
        return Vec::new();
    }
    completion::complete_path(std::path::Path::new(&cwd), &token)
}

/// Subcomando e flag vêm do histórico do próprio dono: para `git co`, os
/// comandos que começaram com `git `. Personalizado, sem base externa e sem
/// manutenção — a do Warp é AGPL e está fora de alcance.
#[tauri::command]
fn complete_argument(
    state: State<'_, AppState>,
    prefix: String,
    token: String,
) -> Result<Vec<String>, String> {
    let commands = state
        .store
        .history_with_prefix(&prefix, 400)
        .map_err(|e| e.to_string())?;
    Ok(completion::next_tokens(&commands, &prefix, &token))
}

#[tauri::command]
fn clear_command_history(
    state: State<'_, AppState>,
    repo_root: Option<String>,
) -> Result<(), String> {
    state
        .store
        .clear_command_history(repo_root.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_history_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    state
        .store
        .set_setting(HISTORY_PREF_KEY, if enabled { "on" } else { "off" })
        .map_err(|e| e.to_string())?;
    history::set_enabled(enabled);
    Ok(())
}

/// Locais sempre; do repositório **só depois do consentimento** por hash — o
/// mesmo de `.tyba/config.toml`. Clonar um repo não coloca comando na paleta.
#[tauri::command]
async fn list_snippets(
    state: State<'_, AppState>,
    repo_root: Option<String>,
) -> Result<Vec<snippet::Snippet>, String> {
    let mut snippets = state.store.list_snippets().map_err(|e| e.to_string())?;
    let Some(root) = repo_root else {
        return Ok(snippets);
    };
    let param = repo::canonicalize_or(std::path::Path::new(&root));
    let Some(top) = repo::toplevel(&param).map(|t| repo::canonicalize_or(&t)) else {
        return Ok(snippets);
    };
    let Ok(Some((config, hash))) = repo_config::load(&top) else {
        return Ok(snippets);
    };
    let consented = state
        .store
        .config_consent(&top.to_string_lossy(), &hash)
        .ok()
        .flatten()
        .unwrap_or(false);
    if consented {
        snippets.extend(config.snippets);
    }
    Ok(snippets)
}

#[tauri::command]
fn save_snippet(state: State<'_, AppState>, snippet: snippet::Snippet) -> Result<(), String> {
    if snippet.source != snippet::Source::Local {
        return Err("snippet de repositório é editado no .tyba/config.toml".into());
    }
    snippet::validate(&snippet.name, &snippet.command).map_err(|e| e.to_string())?;
    state
        .store
        .save_snippet(&snippet)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_snippet(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.store.delete_snippet(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn snippet_placeholders(command: String) -> Vec<snippet::Placeholder> {
    snippet::placeholders(&command)
}

/// Renderiza no core: o parser de placeholder é testado num lugar só, e o front
/// não reimplementa substituição em cima de texto que vai virar linha de comando.
#[tauri::command]
fn render_snippet(
    state: State<'_, AppState>,
    id: String,
    command: String,
    values: Vec<(String, String)>,
) -> Result<String, String> {
    let rendered = snippet::render(&command, &values);
    let _ = state.store.touch_snippet(&id);
    Ok(rendered)
}

#[tauri::command]
async fn list_themes(state: State<'_, AppState>) -> Result<Vec<theme::Theme>, String> {
    Ok(state.themes.list())
}

#[tauri::command]
fn get_theme_state(state: State<'_, AppState>) -> theme::ThemeState {
    state.themes.state()
}

#[tauri::command]
fn set_theme_mode(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: theme::ThemeMode,
) -> Result<(), String> {
    state.themes.set_mode(&app, mode).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_theme_slot(
    app: AppHandle,
    state: State<'_, AppState>,
    base: theme::ThemeBase,
    id: String,
) -> Result<(), String> {
    state
        .themes
        .set_slot(&app, base, &id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn import_theme(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<theme::Theme, String> {
    state.themes.import(&app, &path).map_err(|e| e.to_string())
}

fn open_store(app: &AppHandle) -> session::store::Store {
    let db_path = std::env::var_os("TYBA_DATA_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| app.path().app_data_dir().ok())
        .map(|dir| {
            let _ = std::fs::create_dir_all(&dir);
            dir.join("tyba.db")
        });

    db_path
        .and_then(|path| session::store::Store::open(&path).ok())
        .or_else(|| session::store::Store::open_in_memory().ok())
        .expect("failed to open session store")
}

/// Tudo que o arranque faz e que não precisa da main thread.
///
/// Isto morava no closure `.setup()`, que roda na main thread com o event loop
/// parado e a janela já visível — ver [`boot`]. Fora dali, o webview carrega em
/// paralelo: a UI aparece "carregando" em vez de congelada, e ao receber
/// [`boot::EVENT_READY`] reconsulta o estado.
///
/// A ordem interna não é livre. `restore` precede `resume_startup`, que precisa
/// saber quais sessões morreram; o layout entra depois, porque é ele que reaponta
/// os panes para as sessões recém-nascidas; e `drain_checkpoints` fica antes de
/// `blocks::install` porque o dreno grava blocos e o contador de ids vivos parte
/// do maior id gravado.
fn run_boot(
    app: AppHandle,
    store: Arc<Store>,
    sessions: SharedSessionManager,
    layout: layout::SharedLayout,
    pty_pool: SharedPtyPool,
    reconcile: std::sync::mpsc::Sender<()>,
    boot: Arc<boot::BootGate>,
) {
    let total = boot::Span::start("boot.thread");

    // O tyba.conf é derivado do banco, então se regenera no boot: quem
    // cadastrou host numa versão antiga recebe o que mudou no formato
    // (multiplexing, p.ex.) sem ter que reeditar host por host.
    let span = boot::Span::start("ssh.materialize");
    if let (Ok(hosts), Some(home)) = (store.load_hosts(), ssh::home_dir()) {
        if !hosts.is_empty() {
            if let Err(e) = ssh::config::materialize(&home, &hosts) {
                eprintln!("tyba: ssh config não materializou: {e}");
            }
        }
    }
    span.end();

    let span = boot::Span::start("sessions.restore");
    let _ = sessions.restore();
    span.end();

    let span = boot::Span::start("resume_startup");
    let remap = resume_startup(&app, &store, &sessions, &pty_pool);
    let valid: std::collections::HashSet<SessionId> =
        sessions.list().iter().map(|s| s.id).collect();
    span.end_with(format!(
        "{} sessões, {} reabertas",
        valid.len(),
        remap.len()
    ));

    let span = boot::Span::start("layout.load");
    layout.load_remapped(&valid, &remap);
    span.end();

    let enabled = history_enabled(&store);
    history::install(Arc::clone(&store), enabled);

    // Checkpoint órfão = o app morreu com um comando rodando. Vira bloco sem
    // exit code, que é exatamente o "não terminou".
    let span = boot::Span::start("checkpoints.drain");
    let _ = store.drain_checkpoints();
    span.end();
    blocks::install(app.clone(), Arc::clone(&store));

    // Abrir o portão antes de avisar: quem for reconsultar por causa do evento
    // precisa achar `ready: true`, não uma corrida.
    //
    // O portão chega por parâmetro, e não por `try_state::<AppState>()`, porque
    // ali um `None` — estado não gerenciado ainda — deixava o portão fechado
    // para sempre sem nenhum sinal. Como parâmetro, não há caminho em que a
    // thread chegue até aqui sem ter o que abrir.
    boot.mark_ready();
    let _ = app.emit(layout::EVENT_CHANGED, layout.state());
    let _ = app.emit(boot::EVENT_READY, ());
    let _ = reconcile.send(());

    total.end();

    // Daqui para baixo é manutenção, fora do tempo de arranque — e depois do
    // `ready` de propósito, para não empurrá-lo.

    // O GC só pode rodar DEPOIS do restore: ele decide o que é órfão a partir de
    // `sessions.list()`, e antes do restore essa lista está vazia — todo worktree
    // gerenciado pareceria abandonado.
    let report = worktree::gc_orphans(&known_worktree_paths(&sessions));
    for removed in &report.removed {
        eprintln!("[tyba] worktree órfão removido: {}", removed.display());
    }

    store.checkpoint_truncate();
}

#[derive(Clone, serde::Serialize)]
struct BootFailure {
    message: String,
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        return (*text).to_string();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "pânico sem mensagem".into()
}

/// Roda o corpo do boot e garante que o portão abre mesmo se ele explodir.
///
/// A thread de boot é a única que chama `mark_ready`. Se ela morrer antes
/// disso, o portão fica fechado PARA SEMPRE, e o sintoma não é um crash: o
/// splash desiste em `SPLASH_CEILING_MS` e entrega um app sem sessões e sem
/// layout, enquanto todo `create_session`/`apply_launch_config` paga
/// [`BOOT_WAIT`] antes de fazer qualquer coisa. Falha disfarçada de lentidão.
/// Enquanto o boot morava no `.setup()`, o mesmo erro matava o app na hora, com
/// stack trace — a regressão foi trocar barulho por silêncio.
///
/// `catch_unwind` é o único jeito de reagir DEPOIS do pânico e ainda abrir o
/// portão, e `AssertUnwindSafe` é honesto aqui porque nada do estado capturado
/// é lido no caminho de unwind: ele abre o portão e avisa, não continua o
/// arranque em cima de estado meio escrito. O que sobrar quebrado (mutex
/// envenenado, sessão sem PTY) quebra alto no primeiro uso, que é melhor do que
/// uma janela que nunca responde.
///
/// Devolve a mensagem do pânico só quando ele aconteceu ANTES do `mark_ready`.
/// Depois dele o arranque já cumpriu o contrato — o que roda ali é manutenção
/// (GC de worktree órfão, truncate do WAL), e derrubar um banner de "o app não
/// carregou" por causa dela seria mentira.
fn guard_boot(gate: &boot::BootGate, body: impl FnOnce()) -> Option<String> {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    let Err(payload) = outcome else {
        return None;
    };
    let message = panic_message(&*payload);
    eprintln!("[tyba] a thread de boot morreu: {message}");
    let before_ready = !gate.is_ready();
    gate.mark_ready();
    before_ready.then_some(message)
}

fn spawn_boot(
    app: AppHandle,
    store: Arc<Store>,
    sessions: SharedSessionManager,
    layout: layout::SharedLayout,
    pty_pool: SharedPtyPool,
    reconcile: std::sync::mpsc::Sender<()>,
    boot: Arc<boot::BootGate>,
) {
    let failure_app = app.clone();
    let gate = Arc::clone(&boot);
    std::thread::Builder::new()
        .name("boot".into())
        .spawn(move || {
            let failed = guard_boot(&gate, || {
                run_boot(app, store, sessions, layout, pty_pool, reconcile, boot);
            });
            if let Some(message) = failed {
                // Mesma ordem do caminho de sucesso: portão (já aberto pelo
                // `guard_boot`), depois aviso. O `ready` vai junto para o splash
                // não ficar esperando os 4 s de teto por um boot que já morreu.
                let _ = failure_app.emit(boot::EVENT_FAILED, BootFailure { message });
                let _ = failure_app.emit(boot::EVENT_READY, ());
            }
        })
        .expect("failed to spawn boot thread");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Sem isto o Tauri instala o `Menu::default` dele (Ajuda → tauri.app).
        // O menu do produto é montado pelo front via `set_app_menu`.
        .enable_macos_default_menu(false)
        // O menu do macOS é do app inteiro: sem escolher a janela em foco, um
        // clique em "Nova aba" abriria uma aba em cada janela aberta.
        .on_menu_event(|app, event| {
            let action = event.id().0.clone();
            let focused = app
                .webview_windows()
                .into_values()
                .find(|window| window.is_focused().unwrap_or(false));
            match focused {
                Some(window) => {
                    let _ = window.emit(menu::MENU_EVENT, action);
                }
                None => {
                    let _ = app.emit(menu::MENU_EVENT, action);
                }
            }
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(state) = window.try_state::<AppState>() {
                    state.pty_pool.drop_window_attachers(window.label());
                }
            }
        })
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                if let Some(state) = webview.try_state::<AppState>() {
                    state.pty_pool.drop_window_attachers(webview.label());
                }
            }
        })
        .setup(|app| {
            let setup_span = boot::Span::start("setup.total");
            let store = Arc::new(open_store(app.handle()));
            let pty_pool: SharedPtyPool = Arc::new(pty::PtyPool::new());
            let sessions: SharedSessionManager =
                Arc::new(session::SessionManager::new(Arc::clone(&store)));
            let layout: layout::SharedLayout =
                Arc::new(layout::LayoutManager::new(Arc::clone(&store)));

            let themes_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("tyba"))
                .join("themes");
            let themes: theme::SharedThemes =
                Arc::new(theme::ThemeManager::new(Arc::clone(&store), themes_dir));

            let repos: repo::SharedRepoWatcher = Arc::new(repo::RepoWatcher::new());
            let (reconcile_tx, reconcile_rx) = std::sync::mpsc::channel::<()>();

            let lsp: lsp::SharedLsp = Arc::new(lsp::LspManager::new());
            lsp::spawn_reaper(Arc::clone(&lsp));

            let managed_data_dir = lsp::resolve_data_dir()
                .unwrap_or_else(|| std::env::temp_dir().join("dev.tyba.app"));
            let managed_lsp: lsp::managed::SharedManaged = Arc::new(
                lsp::managed::ManagedManager::new(Arc::clone(&store), managed_data_dir),
            );
            lsp.attach_managed(Arc::clone(&managed_lsp));

            app.manage(AppState {
                store: Arc::clone(&store),
                pty_pool: Arc::clone(&pty_pool),
                sessions: Arc::clone(&sessions),
                approvals: Arc::new(approvals::ApprovalsManager::new()),
                themes,
                layout: Arc::clone(&layout),
                docker: Arc::new(docker::DockerManager::new()),
                repos,
                files: Arc::new(files::FilesManager::new()),
                remote_files: Arc::new(files::remote::RemoteFilesManager::new()),
                lsp,
                managed_lsp,
                repo_reconcile: reconcile_tx.clone(),
                rich_input_submit: parking_lot::Mutex::new(()),
                worktree_files: rich_input::FilesCache::default(),
                hook_servers: Arc::new(agent::session::HookServerRegistry::default()),
                subagents: Arc::new(agent::subagents::SubagentTracker::new()),
                agent_prober: Arc::new(agent::process_probe::AgentProber::default()),
                disk_observer: Arc::new(agent::disk_observer::DiskObserver::with_coordinator({
                    let coordinate_app = app.handle().clone();
                    Arc::new(move |session, coordination| {
                        coordinate_subagent_viewer(&coordinate_app, session, coordination);
                    })
                })),
                tunnel_states: Arc::new(crate::ssh::tunnel::TunnelStates::default()),
                boot: Arc::new(boot::BootGate::new()),
            });

            // Fim de subagente async detectado por arquivo desce a sessão a Idle
            // — mas só se ela ainda estiver segurada em Running (não pisa em
            // AwaitingInput/Exited nem em turno novo já iniciado).
            {
                let state = app.state::<AppState>();
                let coord_sessions = Arc::clone(&state.sessions);
                let coord_app = app.handle().clone();
                state
                    .subagents
                    .set_idle_coordinator(Arc::new(move |session, summary| {
                        if coord_sessions
                            .get(session)
                            .is_some_and(|s| matches!(s.status, SessionStatus::Running))
                        {
                            coord_sessions.set_status(
                                &coord_app,
                                session,
                                SessionStatus::Idle { summary },
                            );
                        }
                    }));
            }

            let cwd_tx = reconcile_tx.clone();
            app.listen_any(pty::EVENT_CWD_CHANGED, move |_| {
                let _ = cwd_tx.send(());
            });

            let reconcile_handle = app.handle().clone();
            std::thread::Builder::new()
                .name("repo-reconcile".into())
                .spawn(move || {
                    while reconcile_rx.recv().is_ok() {
                        std::thread::sleep(Duration::from_millis(300));
                        while reconcile_rx.try_recv().is_ok() {}
                        reconcile_repo_watchers(&reconcile_handle);
                    }
                })
                .expect("failed to spawn repo reconcile thread");

            let probe_handle = app.handle().clone();
            std::thread::Builder::new()
                .name("agent-probe".into())
                .spawn(move || loop {
                    std::thread::sleep(agent::process_probe::POLL_INTERVAL);
                    poll_agent_probers(&probe_handle);
                })
                .expect("failed to spawn agent probe thread");

            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                let hidden = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = hidden.hide();
                    }
                });
            }

            // Único passo caro que fica: montar o NSMenu é operação de main
            // thread, não há para onde mover.
            let span = boot::Span::start("menu.install");
            let _ = menu::install_fallback(app.handle());
            span.end();

            let boot_gate = Arc::clone(&app.state::<AppState>().boot);
            spawn_boot(
                app.handle().clone(),
                Arc::clone(&store),
                Arc::clone(&sessions),
                Arc::clone(&layout),
                Arc::clone(&pty_pool),
                reconcile_tx,
                boot_gate,
            );

            setup_span.end();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            boot_snapshot,
            agent_binary_available,
            detected_agent,
            create_session,
            broadcast_write,
            broadcast_submit,
            connect_host_group,
            reconnect_ssh,
            list_hosts,
            list_host_groups,
            create_host,
            update_host,
            delete_host,
            list_session_tunnels,
            open_session_tunnel,
            close_session_tunnel,
            create_host_group,
            update_host_group,
            delete_host_group,
            write_to_session,
            submit_rich_input,
            set_agent_match_pattern,
            prompt_mentions_sensitive,
            session_bracketed_paste,
            session_rel_path,
            list_worktree_files,
            attach_session,
            detach_session,
            worktree_list,
            worktree_setup_script,
            worktree_set_setup_consent,
            worktree_remove,
            worktree_gc,
            session_diff,
            session_diff_hunks,
            session_conflicts,
            session_conflict_choose,
            session_conflict_mark_resolved,
            session_branches,
            session_fetch,
            session_checkout,
            suggest_commit_message,
            session_branch_diff,
            session_branch_hunks,
            open_diff_tab,
            open_tunnels_panel,
            open_files_panel,
            files_panel_info,
            files_list_dir,
            files_read,
            files_watch_dir,
            files_unwatch_dir,
            files_refresh,
            files_reanchor,
            files_decorations,
            files_open_external,
            files_close,
            files_write,
            files_create,
            files_rename,
            files_delete,
            files_search,
            files_focus,
            files_gutter,
            files_edit_begin,
            files_edit_end,
            lsp_status,
            lsp_open,
            lsp_retry,
            lsp_managed_registry,
            lsp_managed_consent,
            lsp_managed_download,
            lsp_managed_use_mine,
            lsp_managed_download_status,
            lsp_change,
            lsp_did_save,
            lsp_close_doc,
            lsp_completion,
            lsp_hover,
            lsp_definition,
            lsp_signature,
            lsp_open_external,
            close_side_view,
            set_side_view_expanded,
            set_side_view_ratio,
            worktree_stage,
            worktree_unstage,
            worktree_discard,
            worktree_commit,
            worktree_push,
            worktree_merge_preview,
            worktree_merge_materialize,
            worktree_merge_into_base,
            forge_status,
            forge_pr_for_session,
            forge_pr_comments,
            forge_create_pr,
            forge_pr_list,
            forge_workflow_runs,
            forge_workflow_jobs,
            session_git_status,
            open_worktree_file,
            repo_snapshots,
            session_cwd,
            resize_session,
            list_sessions,
            session_mark_seen,
            dispose_session,
            request_approval,
            list_approvals,
            resolve_approval,
            list_subagents,
            focus_subagent,
            subagent_transcript,
            open_agents_panel,
            kill_shell_agent,
            open_subagent_viewer,
            close_agent_viewers,
            agent_repo_config,
            set_agent_config_consent,
            set_app_menu,
            submit_shell_line,
            write_control,
            toggle_prompt_mode,
            session_prompt_mode,
            session_line_echo,
            session_blocks,
            search_command_history,
            suggest_commands,
            complete_path,
            complete_argument,
            suggest_line,
            clear_command_history,
            set_history_enabled,
            list_snippets,
            save_snippet,
            delete_snippet,
            snippet_placeholders,
            render_snippet,
            list_themes,
            get_theme_state,
            set_theme_mode,
            set_theme_slot,
            import_theme,
            layout_state,
            create_workspace,
            list_launch_configs,
            apply_launch_config,
            save_launch_config,
            delete_launch_config,
            launch_config_seed,
            tag_workspace,
            close_workspace,
            activate_workspace,
            rename_workspace,
            set_workspace_color,
            set_workspace_group,
            new_window,
            create_tab,
            close_tab,
            activate_tab,
            move_tab,
            open_session_in_tab,
            split_pane,
            close_pane,
            focus_pane,
            set_split_ratio,
            get_pref,
            app_version,
            app_build_info,
            update_check,
            update_dismiss,
            set_pref,
            list_editors,
            list_shells,
            docker_available,
            docker_list_containers,
            docker_open_logs,
            docker_open_shell,
            docker_remove_container,
            docker_open_desktop,
            docker_compose_op,
            docker_open_project,
            docker_open_compose_file,
            docker_open_dashboard,
            open_view_tab,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tyba")
        .run(|app_handle, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    state.pty_pool.kill_all();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guarda de compilação, não teste: os dois comandos abaixo esperam o boot,
    /// e comando síncrono roda na main thread. Devolvê-los para `fn` congela o
    /// webview por até `BOOT_WAIT` em quem apertar ⌘T na fresta entre o splash
    /// desistir (4 s) e o boot terminar — a regressão que este `impl Future`
    /// impede de voltar em silêncio.
    #[allow(dead_code)]
    fn comandos_que_esperam_o_boot_seguem_assincronos(
        app: AppHandle,
        state: State<'_, AppState>,
        opts: CreateSessionOpts,
        id: launch_config::LaunchConfigId,
    ) -> (
        impl std::future::Future<Output = Result<Session, String>> + '_,
        impl std::future::Future<Output = Result<AppliedLaunchConfig, String>> + '_,
    ) {
        (
            create_session(app.clone(), state.clone(), opts),
            apply_launch_config(app, state, id, None, 80, 24),
        )
    }

    /// Pânico na thread de boot não pode deixar o portão fechado: fechado para
    /// sempre é o splash desistindo em 4 s, o app sem sessões e sem layout, e
    /// todo comando de escrita pagando `BOOT_WAIT` — falha vestida de lentidão.
    ///
    /// O pânico impresso no output destes dois testes é esperado, e o hook fica
    /// como está de propósito: silenciá-lo é `set_hook` global, que numa suíte
    /// paralela pode vazar para outro teste e engolir a mensagem de uma falha
    /// de verdade.
    #[test]
    fn panico_no_boot_abre_o_portao_e_reporta() {
        let gate = boot::BootGate::new();
        let failed = guard_boot(&gate, || panic!("restore explodiu"));

        assert!(gate.is_ready());
        assert_eq!(failed.as_deref(), Some("restore explodiu"));
    }

    /// Depois do `mark_ready` o arranque já cumpriu o contrato — o que roda ali
    /// é manutenção. Reportar "o app não carregou" por causa dela seria mentira.
    #[test]
    fn panico_depois_do_ready_nao_vira_falha_de_boot() {
        let gate = boot::BootGate::new();
        let failed = guard_boot(&gate, || {
            gate.mark_ready();
            panic!("gc de worktree explodiu");
        });

        assert!(gate.is_ready());
        assert_eq!(failed, None);
    }

    #[test]
    fn boot_sem_panico_nao_reporta_falha() {
        let gate = boot::BootGate::new();
        let failed = guard_boot(&gate, || gate.mark_ready());
        assert!(gate.is_ready());
        assert_eq!(failed, None);
    }

    #[test]
    fn build_info_carries_version_and_platform() {
        let info = build_info();
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert!(!info.os.is_empty());
        assert!(!info.arch.is_empty());
    }

    #[test]
    fn build_info_serializes_without_git() {
        let info = BuildInfo {
            version: "0.3.0".into(),
            commit: String::new(),
            commit_date: String::new(),
            os: "linux".into(),
            arch: "x86_64".into(),
            webview: String::new(),
        };
        let json = serde_json::to_string(&info).expect("serializa sem git");
        assert!(json.contains("\"commit\":\"\""));
        assert!(json.contains("\"commit_date\":\"\""));
    }
}
