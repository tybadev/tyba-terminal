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
pub mod stats;
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

/// A recusa de quem precisa do estado carregado para agir sem estragar nada.
/// O usuário reclica; a alternativa é tratar "ainda não li" como "não existe".
const BOOT_NOT_READY: &str = "o app ainda está carregando; tente de novo em instantes";

/// Segura o comando até a thread de boot terminar, e devolve se ela terminou.
///
/// **A ordem em relação às leituras é o contrato.** Antes do `mark_ready`,
/// `state.sessions` e `state.layout` estão vazios porque nada foi carregado
/// ainda — não porque não haja nada. Quem lê antes desta espera lê "ainda não
/// li" e não tem como distinguir de "não existe"; quem decide a partir disso
/// duplica workspace, apaga worktree que tem dono, ou reescreve o layout salvo
/// por cima de uma memória vazia.
///
/// `false` significa que o teto de [`BOOT_WAIT`] estourou com o portão ainda
/// fechado. Quem escreve a partir de uma leitura de estado tem de tratar esse
/// caso à parte — ver [`BOOT_NOT_READY`].
///
/// `spawn_blocking` porque a espera é condvar bloqueante: num worker do runtime
/// ela dormiria até [`BOOT_WAIT`] segurando os outros comandos assíncronos. O
/// portão entra por valor para não haver empréstimo atravessando o `.await`.
async fn wait_for_boot(boot: Arc<boot::BootGate>) -> bool {
    tauri::async_runtime::spawn_blocking(move || boot.wait_ready(BOOT_WAIT))
        .await
        // Join que falhou é portão que não se sabe: `false` é a resposta
        // conservadora, e quem chama recusa em vez de agir no escuro.
        .unwrap_or(false)
}

/// As sessões que estão dentro de algum pane, em qualquer workspace ou tab.
///
/// `AgentViewer` conta junto com `Leaf`: os dois prendem uma sessão a um pane, e
/// os dois podem estar em foco.
fn pane_bound_sessions(layout: &layout::LayoutState) -> std::collections::HashSet<SessionId> {
    fn walk(node: &layout::PaneNode, out: &mut std::collections::HashSet<SessionId>) {
        match node {
            layout::PaneNode::Leaf { session_id, .. }
            | layout::PaneNode::AgentViewer { session_id, .. } => {
                out.insert(*session_id);
            }
            layout::PaneNode::Split { first, second, .. } => {
                walk(first, out);
                walk(second, out);
            }
        }
    }

    let mut out = std::collections::HashSet::new();
    for workspace in &layout.workspaces {
        for tab in &workspace.tabs {
            if let Some(root) = &tab.root {
                walk(root, &mut out);
            }
        }
    }
    out
}

/// O worktree das sessões presas a um pane, viva ou encerrada.
///
/// O worktree de um agente continua em disco depois que a sessão morre — é o
/// ponto do modelo, e é lá que o dono revisa o diff. Sem raiz observada ele não
/// ganhava `RepoSnapshot`, e o chip de branch ficava sem o que mostrar: não
/// porque o repositório sumiu, mas porque ninguém tinha olhado.
///
/// **O corte é o pane, não "toda sessão encerrada".** A tabela `sessions` cresce
/// monotonicamente no modo `Resume` (o padrão), e `restore` devolve todas: com o
/// corte por status, cada agente já encerrado viraria uma raiz nova, cada raiz um
/// watcher de FS mais `branch`/`status`/`ahead_behind` a cada evento — custo que
/// só cresce, num core que disputa CPU com os agentes. E o corte não perde nada
/// visível: o chip mostra a sessão ATIVA, e sessão ativa é sempre sessão num
/// pane.
///
/// Vale para a sessão viva também, de propósito. O caminho do `process_cwd`
/// abaixo já resolve o mesmo worktree enquanto o agente vive; registrar pelos
/// dois lados faz a raiz não sumir no instante em que ele termina, e o watcher
/// não é derrubado para ser recriado igual no reconcile seguinte.
fn session_worktree_roots(
    sessions: &[Session],
    bound: &std::collections::HashSet<SessionId>,
) -> std::collections::HashSet<std::path::PathBuf> {
    touchable_worktrees(sessions, bound)
        // `toplevel` é a checagem de existência, e é de propósito que não haja
        // um `is_dir()` antes: worktree removido — na mão ou pelo `gc_orphans` —
        // faz o `git` falhar, o caminho é descartado e nada aparece na tela. A
        // única retentativa é a cadência da própria reconciliação.
        .filter_map(repo::toplevel)
        .collect()
}

/// Os worktrees que a reconciliação pode mandar o `git` visitar.
///
/// **O `git` daqui bloqueia a thread `repo-reconcile`**: `repo::toplevel` faz
/// shell-out e espera no `output()`, e num mount NFS/SMB morto o processo fica
/// em I/O ininterrompível — sem diálogo para clicar e sem timeout para estourar.
/// A thread não volta, e o `EVENT_RECONCILED` para de sair para todos os
/// repositórios, não só para o do caminho ruim.
///
/// Por isso a pergunta é `may_hang_shared_thread`, e **não** a `reopen_policy`
/// que o `resume_startup` usa. As duas classificam caminho pelo texto e
/// compartilham a lista de prefixos, mas foram calibradas por custos opostos: lá
/// adiar à toa perde uma aba, e por isso `/mnt/c` (o disco do Windows no WSL)
/// passa; aqui adiar à toa perde o chip de branch de um repositório até o tick
/// seguinte, e tocar errado perde o de todos, para sempre. Reusar a política do
/// arranque aqui já foi exatamente esse bug.
fn touchable_worktrees<'a>(
    sessions: &'a [Session],
    bound: &'a std::collections::HashSet<SessionId>,
) -> impl Iterator<Item = &'a std::path::Path> {
    sessions
        .iter()
        .filter(|session| bound.contains(&session.id))
        .filter_map(|session| session.worktree.as_ref())
        .map(|worktree| worktree.path.as_path())
        .filter(|path| !session::cwd::may_hang_shared_thread(path))
}

/// Roda inteira na thread `repo-reconcile`, e os três blocos abaixo fazem
/// shell-out de `git`.
///
/// Só o dos worktrees de sessão filtra caminho antes de tocar. Os outros dois —
/// a raiz que o dono escolheu para o workspace e o cwd de um shell vivo —
/// continuam expostos a um mount morto, de propósito: ali o chip perdido é o do
/// repositório principal, e adiar `/mnt/c` cobraria isso de todo usuário de WSL
/// no caso saudável, que é o comum. Fechar a classe inteira é tirar o `git` de
/// dentro desta thread, não alongar a lista de prefixos.
fn watched_repo_roots(state: &AppState) -> std::collections::HashSet<std::path::PathBuf> {
    let layout = state.layout.state();
    let mut roots: std::collections::HashSet<std::path::PathBuf> = layout
        .workspaces
        .iter()
        .filter_map(|w| w.repo_root.as_deref())
        .map(|root| session::expand_home(std::path::Path::new(root)))
        .filter_map(|root| repo::toplevel(&root))
        .collect();

    let sessions = state.sessions.list();
    roots.extend(session_worktree_roots(
        &sessions,
        &pane_bound_sessions(&layout),
    ));

    for session in sessions {
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

/// Liga o produtor do palpite de tela: sessão sem hook — shell, ssh, container —
/// ganha um observador que avalia manifesto na thread do próprio PTY e escreve
/// em `Session.observed`.
///
/// Instalado no `setup`, antes do boot: a restauração de sessões já sobe PTY, e
/// uma fábrica instalada depois deixaria essas sessões sem observador para
/// sempre. É também o único lugar onde as quatro peças coexistem — registro,
/// `SessionManager`, `AppHandle` e prober —, e por isso a fábrica mora aqui e
/// não no `PtyPool`.
fn install_screen_observers(
    app: &AppHandle,
    pty_pool: &SharedPtyPool,
    sessions: &SharedSessionManager,
    store: &Arc<session::store::Store>,
    prober: &agent::process_probe::SharedAgentProber,
    manifests_dir: &std::path::Path,
) {
    // Uma varredura só, no boot: reler o disco a cada avaliação colocaria IO no
    // caminho quente do PTY. Trocar manifesto exige reabrir o app.
    let registry = Arc::new(status::registry::ManifestRegistry::load(manifests_dir));

    // Daqui para baixo é só adaptador: cada `Arc` traduz uma peça do Tauri para
    // a assinatura que o `ObserverDeps` pede. A montagem em si mora no
    // `status::observer`, onde teste alcança — ver `ObserverDeps`.
    let probe_prober = Arc::clone(prober);
    let observed_app = app.clone();
    let observed_sessions = Arc::clone(sessions);
    let notify_app = app.clone();
    let notify_sessions = Arc::clone(sessions);
    let notify_store = Arc::clone(store);

    let deps = status::observer::ObserverDeps {
        registry,
        // Leitura do que o prober JÁ sabe. Quem varre a árvore de processos é o
        // poll de 2 s; aqui só se consulta o resultado dele, senão a identidade
        // custaria uma varredura por avaliação.
        process: Arc::new(move |id| {
            probe_prober
                .detected(id)
                .and_then(|detected| agent::runner_binary(&detected.kind))
                .map(str::to_string)
        }),
        observed: Arc::new(move |id, observed| {
            observed_sessions.set_observed(&observed_app, id, observed)
        }),
        // O palpite sai pelo mesmo `notify_native` do agente com hook — mesma
        // checagem de foco, mesma redação, mesma leitura de preferência.
        notify: Arc::new(move |id, kind, body| {
            agent::session::notify_native(
                &notify_app,
                &notify_sessions,
                &notify_store,
                kind,
                id,
                body,
            );
        }),
        scheduler: status::observed_notify::sleeping_scheduler(),
    };

    pty_pool.set_screen_observers(Arc::new(move |id, kind| {
        status::observer::observer_for(&deps, id, kind)
    }));
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
/// O que o dono decide, hoje, tem nome: o pane restaurado mostra um convite de
/// retomar a conversa nativa do agente (ver [`resume_agent_session`]), e ela só
/// sobe no clique. Isto aqui continua não religando nada.
///
/// O cwd de cada sessão passa por [`session::cwd::reopen_policy`] antes de
/// qualquer syscall daqui: caminho em volume que pode não estar montado não é
/// reaberto, e pasta protegida pelo TCC é reaberta sem o `is_dir()` desta
/// função. **É a política do chamador barato**: adiar à toa aqui custa uma aba
/// que não volta, e é por isso que `/mnt/c` do WSL passa. Quem paga a thread de
/// todo mundo pergunta a `may_hang_shared_thread` — ver `touchable_worktrees`.
/// O segundo caso **não** evita o diálogo de permissão do macOS — o
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
    // O resultado é descartado pelo mesmo motivo que o teto existe: passado ele,
    // agimos com estado incompleto em vez de pendurar o clique. Aqui isso é
    // aceitável porque a sessão nova não se decide a partir do que já estava
    // salvo — ao contrário do `apply_launch_config`, que precisa do layout lido
    // para não duplicar workspace.
    let _ = wait_for_boot(Arc::clone(&state.boot)).await;

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
    // O `busy` abaixo é a única coisa entre um `git worktree remove --force` e o
    // worktree de uma sessão viva, e ele se decide por `sessions.list()`. Antes
    // do boot essa lista está vazia: toda sessão parece encerrada, a guarda
    // deixa passar e o worktree em uso vai embora. Esperar o portão é o que faz
    // dela uma guarda.
    if !wait_for_boot(Arc::clone(&state.boot)).await {
        return Err(BOOT_NOT_READY.into());
    }

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
    // A regra já está escrita no `run_boot`, onde o GC roda depois do restore:
    // ele decide o que é órfão a partir de `sessions.list()`, e com a lista
    // vazia **todo** worktree gerenciado parece abandonado. Aqui quem dispara é
    // o usuário, e o teto pode estourar antes do restore — recusar é a única
    // saída, porque o que este comando faz quando erra é apagar o que tem dono.
    if !wait_for_boot(Arc::clone(&state.boot)).await {
        return Err(BOOT_NOT_READY.into());
    }
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
#[serde(rename_all = "camelCase")]
struct BootSnapshot {
    ready: bool,
    /// O arranque não entregou o estado inteiro, e esta é a mensagem.
    ///
    /// Segunda via do `app://boot-failed` — ver [`boot::EVENT_FAILED`] para as
    /// duas origens (thread de boot morta, banco de sessões degradado). Com este
    /// campo preenchido, as listas ao lado estão incompletas por falha, não por
    /// ausência de dado.
    ///
    /// **Pode vir com `ready: false`.** O banco degradado é conhecido no
    /// `.setup()`, antes de a thread de boot começar, e `note_failure` registra
    /// sem abrir o portão — o estado ainda vai carregar. `ready` continua sendo
    /// a pergunta "as listas já são finais?", e este campo, "o que veio dá para
    /// confiar?"; são independentes.
    ///
    /// O `kind` é o que diz QUAL das duas origens — ver [`boot::FailureKind`].
    /// Sem ele o front escolhia o mesmo título para as duas, e ele está errado
    /// para o banco degradado: ali o arranque terminou inteiro.
    boot_failure: Option<boot::Failure>,
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
///
/// `bootFailure` preenchido significa a terceira possibilidade: a thread morreu,
/// e o que vem ao lado está vazio por falha. É a rede do `app://boot-failed`,
/// que pode ter sido emitido antes de o listener do front existir.
///
/// **Lê tudo mesmo com `ready: false`, e é de propósito.** É a chamada do mount,
/// uma por abertura do app, e as prefs são o motivo: elas saem de um `SELECT`
/// direto, não dependem da thread de boot, e o front as aplica **uma vez** — não
/// há caminho que as releia depois do `app://ready`. Devolver prefs vazias no
/// boot lento — que é justamente quando `ready` chega `false` aqui — abriria o
/// app com fonte, toolbar, atalhos e modo de arranque no padrão, em silêncio.
/// Quem repergunta é o [`boot_gate`], e é lá que o retorno cedo mora.
#[tauri::command]
async fn boot_snapshot(state: State<'_, AppState>) -> Result<BootSnapshot, String> {
    // Lido primeiro de propósito — ver [`Loaded::read`].
    let ready = state.boot.is_ready();
    Ok(BootSnapshot {
        ready,
        // Depois do `ready`, e é a ordem que faz o campo valer: o portão só abre
        // depois de a mensagem estar gravada (ver `BootGate::mark_failed`), então
        // ler o `ready` primeiro nunca devolve `true` com falha em branco.
        boot_failure: state.boot.failure(),
        prefs: state.store.prefs().map_err(|e| e.to_string())?,
        sessions: state.sessions.list(),
        layout: state.layout.state(),
    })
}

/// Estado que só existe depois da thread de boot: sessões reabertas e layout.
///
/// Os dois juntos num objeto de propósito — é um `null` só para o front
/// conferir, e não a chance de ler um e esquecer o outro.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BootLoaded {
    sessions: Vec<Session>,
    layout: layout::LayoutState,
}

/// A resposta do poll do boot: o portão, e só.
///
/// Separado do [`BootSnapshot`] porque os dois chamadores querem coisas
/// diferentes. O mount chama uma vez e precisa das prefs mesmo com
/// `ready: false` — elas saem de um `SELECT` direto e não dependem da thread de
/// boot. O poll repergunta a cada ~150ms enquanto o portão está fechado e
/// **descarta** tudo que não seja o portão, porque o `mergeLoaded` do front
/// recusa payload sem `ready`.
///
/// Servir os dois pelo mesmo comando cobrava do poll, a cada tick, o `SELECT`
/// das prefs mais um clone de `sessions.list()` e outro de `layout.state()` —
/// os três sob os mesmos locks que a thread de boot está usando para `restore`
/// e `drain_checkpoints`. Quem espera o boot terminar virava quem o atrasa.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BootGateSnapshot {
    ready: bool,
    /// Mesmo campo, e mesma rede, do [`BootSnapshot::boot_failure`]: o
    /// `app://boot-failed` sai de dentro da thread de boot, antes de o listener
    /// do front existir, e o core não reemite. Vale aqui a mesma ressalva de
    /// lá — pode chegar com `ready: false`, e é o caso do banco degradado.
    boot_failure: Option<boot::Failure>,
    /// **Ausente** — e não vazio — enquanto `ready == false`.
    ///
    /// Vazio é indistinguível de "carregou e não tem", e foi essa confusão que
    /// gerou o [`Loaded`] em primeiro lugar. O `ready: false` já resolve para
    /// quem lê direito; a ausência resolve também para quem esquecer de olhar,
    /// que quebra em vez de desenhar "nenhuma sessão" por cima de vinte que
    /// estão voltando.
    loaded: Option<BootLoaded>,
}

/// Monta a resposta do poll sem tocar no que o portão fechado ainda não tem.
///
/// `loaded` chega como closure porque o ponto do desvio é justamente **não
/// executá-la** — é o que o teste consegue contar.
fn boot_gate_snapshot(
    gate: &boot::BootGate,
    loaded: impl FnOnce() -> BootLoaded,
) -> BootGateSnapshot {
    // Lido antes do resto, e a ordem é a decisão — ver [`Loaded::read`].
    let ready = gate.is_ready();
    // Lido antes do desvio, e agora isso importa de verdade: o banco degradado
    // é registrado no `.setup()` com `note_failure`, que NÃO abre o portão, então
    // o retorno cedo é o caminho por onde essa notícia chega — o poll roda a
    // cada ~150ms justamente enquanto `ready` é `false`. Antes só `mark_failed`
    // produzia falha, e ela implicava `ready: true`; ler o portão aqui já era o
    // certo por um `Mutex<Option<String>>` de custo, e virou o único caminho.
    let boot_failure = gate.failure();
    if !ready {
        return BootGateSnapshot {
            ready,
            boot_failure,
            loaded: None,
        };
    }
    BootGateSnapshot {
        ready,
        boot_failure,
        loaded: Some(loaded()),
    }
}

/// "O boot já terminou?", mais a rede do `app://boot-failed`. Ver
/// [`BootGateSnapshot`] para por que não é o [`boot_snapshot`].
///
/// `async` pelo mesmo motivo do `boot_snapshot`: comando síncrono do Tauri roda
/// na main thread do macOS — a mesma que desenha o webview —, e um tick a cada
/// 150ms ali entra na fila da pintura mesmo custando quase nada.
#[tauri::command]
async fn boot_gate(state: State<'_, AppState>) -> Result<BootGateSnapshot, String> {
    Ok(boot_gate_snapshot(&state.boot, || BootLoaded {
        sessions: state.sessions.list(),
        layout: state.layout.state(),
    }))
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

/// O que a busca de workspace de uma launch config permite concluir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchReuse {
    /// Já existe workspace desta configuração: ativa e devolve.
    Reuse(layout::WorkspaceId),
    /// Layout carregado e sem workspace desta configuração: pode criar.
    Create,
    /// O portão de boot não abriu dentro do teto: o `load_remapped` ainda não
    /// rodou, e um layout vazio não é resposta para "existe workspace?".
    Unknown,
}

/// Achar é prova. Não achar, não.
///
/// A armadilha: `state.layout` só é populado pelo `load_remapped` da thread de
/// boot, e até lá `workspace_of_launch_config` responde `None` para **toda**
/// configuração — inclusive as que já têm workspace. Ler esse `None` como
/// "não existe" tem dois preços, e o segundo é o caro:
///
/// 1. o comando cria um SEGUNDO workspace para a mesma launch config, com todas
///    as sessões e worktrees do primeiro repetidas;
/// 2. o `insert_workspace` que vem depois persiste via `LayoutManager::persist`,
///    que reescreve o layout inteiro a partir da memória — e `save_layout` é
///    `DELETE`+`INSERT`. Com a memória ainda vazia, o que estava salvo no banco
///    e ainda não foi lido some junto.
///
/// Por isso `None` com o portão fechado é [`LaunchReuse::Unknown`], nunca
/// `Create`: a recusa é reversível (o usuário reclica), a duplicata e a perda de
/// layout não são.
///
/// `Some` dispensa o portão de propósito: entre o `load_remapped` e o
/// `mark_ready` existe uma fresta em que o layout já está lido e o portão ainda
/// não abriu. Achar ali é achar de verdade, e recusar o reuso criaria
/// exatamente a duplicata que esta função existe para impedir.
fn decide_launch_reuse(boot_ready: bool, found: Option<layout::WorkspaceId>) -> LaunchReuse {
    match found {
        Some(ws) => LaunchReuse::Reuse(ws),
        None if boot_ready => LaunchReuse::Create,
        None => LaunchReuse::Unknown,
    }
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

    // A espera vem ANTES da leitura do layout, e a ordem é o conserto. O
    // `create_session` de cada slot também espera o boot, mas lá embaixo, dentro
    // do `spawn_slot`: ler o layout antes disso é lê-lo vazio na fresta entre o
    // splash desistir (4 s) e a thread de boot terminar.
    //
    // No caminho normal não custa nada — com o portão aberto o `wait_ready`
    // retorna sem dormir. Na fresta, é a mesma espera que o clique já pagava
    // dentro do `spawn_slot`, só que agora do lado certo da leitura.
    //
    // O resultado importa, ao contrário do `create_session`: é ele que diz se o
    // layout pode ser lido.
    let ready = wait_for_boot(Arc::clone(&state.boot)).await;

    // Com `clean` o reuso é recusado de propósito: o pedido é um workspace novo.
    // A busca é pulada, mas o portão continua valendo — o `insert_workspace` lá
    // embaixo persiste o layout inteiro, e fazer isso com a memória vazia apaga
    // o que estava salvo. Ver [`decide_launch_reuse`].
    let found = if clean {
        None
    } else {
        state.layout.workspace_of_launch_config(id)
    };

    match decide_launch_reuse(ready, found) {
        LaunchReuse::Reuse(ws) => {
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
        // Estourou o teto com a thread de boot ainda presa (diálogo de TCC,
        // disco lento). Recusar é a resposta menos ruim: o usuário reclica,
        // enquanto seguir em frente duplica workspace e apaga o layout salvo.
        LaunchReuse::Unknown => return Err(BOOT_NOT_READY.into()),
        LaunchReuse::Create => {}
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
        && view != layout::VIEW_STATS
        && view != layout::VIEW_AGENT_BOARD
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

/// Se a sessão morta ainda dá para retomar pela conversa nativa do agente.
///
/// O front pergunta antes de desenhar o convite: a decisão é do core, porque só
/// ele sabe se o id foi capturado, se o binário do runner continua no PATH e se
/// a pasta ainda existe. Falso é silêncio — nada de convite que abre em erro.
#[tauri::command]
async fn can_resume_agent_session(
    state: State<'_, AppState>,
    id: SessionId,
) -> Result<bool, String> {
    let _ = wait_for_boot(Arc::clone(&state.boot)).await;
    Ok(state
        .sessions
        .get(id)
        .map(|s| agent::session::can_resume(&s))
        .unwrap_or(false))
}

/// Retoma a conversa nativa do agente na sessão morta `id`, com o comando de
/// resume da CLI dele.
///
/// Só por clique do usuário: o boot devolve a sessão de agente parada de
/// propósito (ver `resume_startup`), porque religá-la sozinha levantaria um
/// processo com contexto que pode voltar a agir sem ninguém ter pedido.
#[tauri::command]
async fn resume_agent_session(
    app: AppHandle,
    state: State<'_, AppState>,
    id: SessionId,
    cols: u16,
    rows: u16,
) -> Result<Session, String> {
    let _ = wait_for_boot(Arc::clone(&state.boot)).await;
    // Mensagens deste caminho são texto direto, como as do resto do
    // `agent::session`: o `translateError` do front repassa string crua.
    let dead = state.sessions.get(id).ok_or("a sessão não existe mais")?;
    let ctx = agent::session::AgentSessionCtx {
        app: app.clone(),
        sessions: Arc::clone(&state.sessions),
        pty_pool: Arc::clone(&state.pty_pool),
        approvals: Arc::clone(&state.approvals),
        store: Arc::clone(&state.store),
        servers: Arc::clone(&state.hook_servers),
        subagents: Arc::clone(&state.subagents),
    };
    let handle = app.clone();
    agent::session::resume_agent_session(&ctx, &dead, cols, rows, move |id| {
        session_exited(&handle, id)
    })
}

/// Abre ou fecha a fila de agentes no workspace ativo.
#[tauri::command]
fn toggle_agent_queue(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state
        .layout
        .toggle_agent_queue()
        .map_err(|e| e.to_string())?;
    emit_layout(&app, &state);
    Ok(())
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

/// Teto da espera pelo editor de linha do shell. O rc do dono leva 1,4 s; um
/// `.zshrc` pesado em máquina fria leva mais.
///
/// O teto existe para o shell que **nunca** chega lá — sem integração, ou morto
/// carregando. Estourado, a linha é escrita assim mesmo: sem o `633;P` não há
/// como saber, e escrever é o que o comando fazia antes do portão. Recusar
/// tornaria pior um caso que hoje funciona.
const LINE_EDITOR_WAIT: Duration = Duration::from_secs(5);

/// Segura a submissão até o shell alcançar o editor de linha, e devolve se
/// alcançou.
///
/// `spawn_blocking` e portão por valor pelas mesmas duas razões do
/// [`wait_for_boot`]: a espera é condvar bloqueante, e empréstimo não atravessa
/// `.await`.
async fn wait_for_line_editor(gate: Arc<pty::LineEditorGate>) -> bool {
    tauri::async_runtime::spawn_blocking(move || gate.wait_open(LINE_EDITOR_WAIT))
        .await
        .unwrap_or(false)
}

/// `async` de propósito: a espera é o que mantém a injeção fora do eco do
/// terminal.
///
/// Antes daqui a linha era escrita no PTY sem nada garantir que houvesse quem a
/// lesse. Isso não a perdia — o driver enfileira e o zsh executa quando assume
/// —, mas durante o carregamento do rc o tty está canônico e ECOA os bytes
/// crus: a injeção aparecia como `^[=<comando>` no topo da sessão, fora de
/// qualquer bloco, e o comando só rodava 1,4 s depois. Ver
/// [`pty::LineEditorGate`] para a medição e para o que já foi medido e é falso.
#[tauri::command]
async fn submit_shell_line(
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

    // A espera vem ANTES do lock de submissão: um shell que demora seguraria a
    // fila de todas as outras sessões por [`LINE_EDITOR_WAIT`].
    //
    // O valor devolvido é ignorado de propósito — ver [`LINE_EDITOR_WAIT`]:
    // estourar o teto significa "não dá para saber", e a resposta a isso é
    // escrever, que é o que este comando fazia antes de existir portão.
    let gate = state
        .pty_pool
        .line_editor_gate(id)
        .ok_or_else(|| format!("sessão não encontrada: {id}"))?;
    let _ = wait_for_line_editor(gate).await;

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

/// As fontes de histórico que existem no disco, com a contagem de cada uma.
///
/// Só conta: nada entra no banco antes de o usuário mandar importar. É o que o
/// convite de primeiro uso mostra.
#[tauri::command]
fn scan_shell_history_sources() -> Result<Vec<history::import::source::SourceScan>, String> {
    let Some(home) = ssh::home_dir() else {
        return Ok(Vec::new());
    };
    let sources = history::import::source::resolve(&home, &|name| std::env::var(name).ok());
    Ok(sources
        .iter()
        .filter_map(|source| history::import::source::scan(source).ok())
        .collect())
}

/// Importa o histórico do shell para dentro do `command_history`.
///
/// Superfície **humana**: nenhum hook, tool ou comando de sessão de agente chega
/// aqui. O único canal que o agente tem com o core é o socket de hook, e a
/// resposta dele é uma decisão (`HookAction`), nunca uma ação como esta.
#[tauri::command]
fn import_shell_history(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<history::import::ImportReport, String> {
    let Some(home) = ssh::home_dir() else {
        return Err("history_import_no_home".into());
    };
    let sources = history::import::source::resolve(&home, &|name| std::env::var(name).ok());
    let report = history::import::run(&state.store, &sources, &mut |progress| {
        let _ = app.emit(history::import::EVENT_PROGRESS, progress);
    })
    .map_err(|error| error.to_string())?;
    Ok(report)
}

/// Fuzzy + frecência no core: o webview recebe a lista já ordenada (princípio #1).
#[tauri::command]
async fn search_command_history(
    state: State<'_, AppState>,
    query: String,
    cwd: Option<String>,
    repo_root: Option<String>,
    session_id: Option<String>,
    filter: Option<history::HistoryFilter>,
    limit: usize,
) -> Result<Vec<HistoryHit>, String> {
    // Janela que começa no futuro devolveria vazio sem explicação, e o usuário
    // leria isso como "não tenho histórico". É erro de quem chamou: cai para
    // sem filtro de período em vez de mentir com uma lista vazia.
    let mut filter = filter.unwrap_or_default();
    if !filter.since_is_sane(approvals::now_ms() as i64) {
        filter.since_ms = None;
    }
    history_hits_filtered(
        &state,
        &query,
        cwd.as_deref(),
        repo_root.as_deref(),
        session_id.as_deref(),
        &filter,
        limit,
    )
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
    history_hits_filtered(
        state,
        query,
        cwd,
        repo_root,
        None,
        &history::HistoryFilter::default(),
        limit,
    )
}

#[allow(clippy::too_many_arguments)]
fn history_hits_filtered(
    state: &AppState,
    query: &str,
    cwd: Option<&str>,
    repo_root: Option<&str>,
    session_id: Option<&str>,
    filter: &history::HistoryFilter,
    limit: usize,
) -> Result<Vec<HistoryHit>, String> {
    use fuzzy_matcher::skim::SkimMatcherV2;
    use fuzzy_matcher::FuzzyMatcher;

    let query = query.trim();
    let candidates = state
        .store
        .history_candidates_filtered(Some(query), cwd, repo_root, session_id, filter)
        .map_err(|e| e.to_string())?;
    let matcher = SkimMatcherV2::default();
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
                    // Mesma regra da frecência: sem exit code conhecido não há
                    // fracasso a marcar. Comando importado não grava código, e
                    // carimbá-lo de "falhou" seria mentira na UI.
                    failed: candidate.known_exit_codes > 0 && candidate.successes == 0,
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
            argument_candidates(&state, prefix, token, cwd.as_deref(), repo_root.as_deref())?
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
/// Candidatos vindos de quem sabe: o git, o `package.json`, o `Makefile`, o
/// gestor de conexões.
///
/// Só roda quando a tabela de `completion::argument` reconhece o prefixo, o que
/// é raro — só depois de `git checkout `, `npm run ` e afins. Fora disso não há
/// leitura de disco nem processo novo, e a completação segue pelo histórico
/// como antes. É o que mantém isto fora do caminho quente da digitação.
fn provider_candidates(
    state: &AppState,
    prefix: &str,
    cwd: Option<&str>,
    repo_root: Option<&str>,
) -> Vec<String> {
    use completion::argument::{find_upwards, parse_make_targets, parse_package_scripts, Provider};

    let Some(provider) = completion::argument::provider_for(prefix) else {
        return Vec::new();
    };
    let root = repo_root.map(std::path::Path::new);
    let here = cwd.map(std::path::Path::new);

    match provider {
        Provider::GitBranch => {
            let Some(repo) = root else {
                return Vec::new();
            };
            worktree::branches::list(repo)
                .map(|list| list.branches.into_iter().map(|b| b.name).collect())
                .unwrap_or_default()
        }
        Provider::NpmScript => here
            .and_then(|dir| find_upwards(dir, "package.json", root))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|source| parse_package_scripts(&source))
            .unwrap_or_default(),
        Provider::MakeTarget => here
            .and_then(|dir| find_upwards(dir, "Makefile", root))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|source| parse_make_targets(&source))
            .unwrap_or_default(),
        Provider::SshHost => state
            .store
            .load_hosts()
            .map(|hosts| hosts.into_iter().map(|h| h.alias).collect())
            .unwrap_or_default(),
        Provider::DockerContainer => state.docker.completion_names(),
    }
}

/// Os candidatos de argumento, do provedor e do histórico, sem repetir.
///
/// O provedor manda **quem existe**; o histórico manda **em que ordem**. O que
/// o histórico conhece e o provedor não (flag, argumento livre) entra no fim —
/// perder isso seria trocar um buraco por outro.
fn argument_candidates(
    state: &AppState,
    prefix: &str,
    token: &str,
    cwd: Option<&str>,
    repo_root: Option<&str>,
) -> Result<Vec<String>, String> {
    let used = state
        .store
        .history_with_prefix(prefix, 400)
        .map_err(|e| e.to_string())?;
    let from_history = completion::next_tokens(&used, prefix, token);

    let mut found = completion::argument::rank(
        provider_candidates(state, prefix, cwd, repo_root)
            .into_iter()
            .filter(|c| c.starts_with(token) && c != token)
            .collect(),
        &from_history,
    );
    for extra in from_history {
        if !found.contains(&extra) {
            found.push(extra);
        }
    }
    found.truncate(40);
    Ok(found)
}

/// Assíncrono **de propósito**: o provedor de Docker pode custar um `docker ps`
/// (~45 ms medidos nesta máquina), e comando síncrono do Tauri roda na main
/// thread do macOS — a mesma que desenha o webview. Síncrono aqui seria engasgo
/// visível na digitação, e o cache do cliente não cobre a primeira chamada.
#[tauri::command]
async fn complete_argument(
    state: State<'_, AppState>,
    prefix: String,
    token: String,
    cwd: Option<String>,
    repo_root: Option<String>,
) -> Result<Vec<String>, String> {
    argument_candidates(
        &state,
        &prefix,
        &token,
        cwd.as_deref(),
        repo_root.as_deref(),
    )
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

/// Painel de estatísticas de agente: agregação em SQL, no core (princípio #1).
///
/// `days` é a janela (`None` = tudo) e vira um corte em epoch ms AQUI, não no
/// webview: "os últimos 7 dias" contados no relógio de quem desenha renderia um
/// recorte diferente do que o banco filtrou.
///
/// Só leitura — este comando não executa, não re-roda e não apaga nada.
#[tauri::command]
async fn agent_stats(
    state: State<'_, AppState>,
    days: Option<u32>,
    repo_root: Option<String>,
) -> Result<stats::AgentStats, String> {
    let since = stats::window_start_ms(days, approvals::now_ms());
    state
        .store
        .agent_stats(since, repo_root.as_deref())
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

/// Abre o banco de sessões — e devolve, junto, o que se perdeu no caminho.
///
/// O segundo membro é a mensagem de degradação, `None` quando abriu inteiro.
/// Ela existe porque **as duas quedas daqui são silenciosas por natureza**: um
/// banco em memória responde a tudo, e um schema com degrau pendente também. O
/// sintoma, nos dois casos, é o app abrir sem sessão, sem layout e sem
/// histórico — indistinguível de uma instalação nova. E isto roda na primeira
/// abertura de todo usuário depois de atualizar.
///
/// Os dois níveis, do mais grave para o menos:
///
/// - **O arquivo do disco não abriu.** Não é um banco legível: corrompido,
///   permissão, meio backup. Cair para memória é a única saída que ainda
///   entrega um app, mas o que o usuário tinha continua no disco e **não** está
///   ali — daí a mensagem.
/// - **Abriu, com degrau de migração pendente.** O banco é o do disco e os
///   dados estão lá; o schema é que ficou atrás do que este binário espera. Ver
///   [`session::store::Store::degraded`].
///
/// O `expect` que sobrou é de outra natureza: `open_in_memory` falhando
/// significa que o SQLite embutido não subiu, e nesse ponto não existe app para
/// degradar.
fn open_store(app: &AppHandle) -> (session::store::Store, Option<String>) {
    let db_path = std::env::var_os("TYBA_DATA_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| app.path().app_data_dir().ok())
        .map(|dir| {
            let _ = std::fs::create_dir_all(&dir);
            dir.join("tyba.db")
        });

    let cause = match &db_path {
        Some(path) => match session::store::Store::open(path) {
            Ok(store) => {
                let degraded = store.degraded().map(str::to_owned);
                return (store, degraded);
            }
            Err(e) => e.to_string(),
        },
        None => "não há diretório de dados para o app".to_string(),
    };

    let store = session::store::Store::open_in_memory()
        .expect("nem o banco em memória abriu: o SQLite embutido não subiu");
    (
        store,
        Some(format!(
            "o banco de sessões não abriu e esta janela está usando um banco temporário: \
             sessões, layout e histórico não foram carregados, e nada do que você fizer agora \
             será salvo ({cause})"
        )),
    )
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
    // Falha registrada ANTES da thread — banco degradado, ver `open_store` — só
    // pode virar aviso aqui: o `.setup()` roda antes de o webview existir, e um
    // `emit` de lá não teria ninguém do outro lado. A ordem é a mesma do caminho
    // de pânico (falha antes do `ready`), para o front não tratar como final o
    // vazio que já dá para explicar. Pânico não passa por aqui — quem o reporta
    // é o `spawn_boot`, que é onde o unwind chega.
    if let Some(failure) = boot.failure() {
        let _ = app.emit(boot::EVENT_FAILED, failure);
    }
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
///
/// A mesma mensagem fica retida no portão, e não só no valor de retorno: o
/// retorno vira um evento, que pode não ser entregue — ver
/// [`boot::EVENT_FAILED`].
fn guard_boot(gate: &boot::BootGate, body: impl FnOnce()) -> Option<String> {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    let Err(payload) = outcome else {
        return None;
    };
    let message = panic_message(&*payload);
    eprintln!("[tyba] a thread de boot morreu: {message}");
    if gate.is_ready() {
        return None;
    }
    gate.mark_failed(&message);
    Some(message)
}

/// O fim da thread de boot quando ela morreu antes de abrir o portão.
///
/// Separado do [`spawn_boot`] porque é a parte com contrato: o caminho de
/// pânico tem de deixar o app no mesmo estado observável do caminho feliz, e
/// isso são três avisos, nesta ordem — e não dois, que era o buraco.
///
/// O `reconcile.send` é o terceiro. Ele mora no fim do [`run_boot`], que o
/// pânico nunca alcança: sem ele a thread `repo-reconcile` fica parada no
/// `recv()`, o `repo://reconciled` nunca sai, e os chips de branch e de diff
/// ficam vazios em cima de um app que já perdeu as sessões — até que um
/// `pty::EVENT_CWD_CHANGED` qualquer cutuque o canal por acaso, o que exige o
/// usuário abrir um terminal e trocar de pasta.
///
/// **Cutucar antes do `guard_boot` cobriria os dois caminhos com uma linha só,
/// e é por isso que não está lá.** A thread de reconcile dorme 300 ms e drena
/// antes de agir, então o pontapé antecipado só se funde com o do fim do boot
/// quando o boot inteiro cabe nesses 300 ms — e ele não cabe: SQLite, `ssh -G`,
/// restore de sessão e layout. Fora dessa janela, o `reconcile_repo_watchers`
/// roda com `layout.state()` e `sessions.list()` ainda vazios, emite um
/// `repo://reconciled` sem nenhum repo — exatamente o vazio-que-parece-final
/// que o `BootGate` existe para não produzir — e ainda paga um `set_roots` com
/// os `git` que ele dispara, competindo por IO com o boot que se está
/// esperando. O ramo de falha custa uma linha a mais e não tem nada disso.
///
/// Enviar duas vezes não é risco: `guard_boot` só devolve `Some` quando o
/// pânico precedeu o `mark_ready`, que precede o `send` do [`run_boot`]. Um
/// pânico na manutenção pós-`ready` já teve o seu pontapé e não chega aqui.
fn finish_failed_boot<R: tauri::Runtime>(
    app: &AppHandle<R>,
    reconcile: &std::sync::mpsc::Sender<()>,
    failure: boot::Failure,
) {
    // Mesma ordem do caminho de sucesso: portão (já aberto pelo `guard_boot`),
    // depois aviso. O `ready` vai junto para o splash não ficar esperando os 4 s
    // de teto por um boot que já morreu.
    let _ = app.emit(boot::EVENT_FAILED, failure);
    let _ = app.emit(boot::EVENT_READY, ());
    let _ = reconcile.send(());
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
    let failure_reconcile = reconcile.clone();
    let gate = Arc::clone(&boot);
    std::thread::Builder::new()
        .name("boot".into())
        .spawn(move || {
            let failed = guard_boot(&gate, || {
                run_boot(app, store, sessions, layout, pty_pool, reconcile, boot);
            });
            // O `guard_boot` diz SE reportar; o portão diz O QUÊ. Quando uma
            // falha anterior já estava anotada — banco degradado, ver
            // `open_store` —, é a mensagem DELA que o `boot_snapshot` devolve, e
            // o evento tem de dizer o mesmo: duas vias com mensagens diferentes
            // viram dois avisos para uma falha só, e o front dedupe pela
            // primeira. O `kind`, esse, já veio escalado pelo `mark_failed` —
            // ver a ressalva lá.
            if failed.is_some() {
                if let Some(failure) = gate.failure() {
                    finish_failed_boot(&failure_app, &failure_reconcile, failure);
                }
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
            let (opened_store, store_degraded) = open_store(app.handle());
            let store = Arc::new(opened_store);

            // O portão nasce aqui, e não dentro do `AppState`, porque a notícia
            // do banco degradado é mais velha que ele: `note_failure` registra
            // sem abrir o portão — o estado ainda vai carregar —, e assim o
            // `mark_ready` do fim do boot publica `ready` com a mensagem já
            // gravada, que é o que o `boot_snapshot` conta como contrato.
            let boot_gate = Arc::new(boot::BootGate::new());
            if let Some(message) = store_degraded {
                eprintln!("[tyba] {message}");
                boot_gate.note_failure(message);
            }

            let pty_pool: SharedPtyPool = Arc::new(pty::PtyPool::new());
            let sessions: SharedSessionManager =
                Arc::new(session::SessionManager::new(Arc::clone(&store)));
            let layout: layout::SharedLayout =
                Arc::new(layout::LayoutManager::new(Arc::clone(&store)));

            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("tyba"));
            let themes: theme::SharedThemes = Arc::new(theme::ThemeManager::new(
                Arc::clone(&store),
                config_dir.join("themes"),
            ));

            let agent_prober: agent::process_probe::SharedAgentProber =
                Arc::new(agent::process_probe::AgentProber::default());
            install_screen_observers(
                app.handle(),
                &pty_pool,
                &sessions,
                &store,
                &agent_prober,
                &config_dir.join("manifests"),
            );

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
                agent_prober: Arc::clone(&agent_prober),
                disk_observer: Arc::new(agent::disk_observer::DiskObserver::with_coordinator({
                    let coordinate_app = app.handle().clone();
                    Arc::new(move |session, coordination| {
                        coordinate_subagent_viewer(&coordinate_app, session, coordination);
                    })
                })),
                tunnel_states: Arc::new(crate::ssh::tunnel::TunnelStates::default()),
                boot: Arc::clone(&boot_gate),
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
            boot_gate,
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
            toggle_agent_queue,
            kill_shell_agent,
            can_resume_agent_session,
            resume_agent_session,
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
            scan_shell_history_sources,
            import_shell_history,
            suggest_commands,
            complete_path,
            complete_argument,
            suggest_line,
            clear_command_history,
            set_history_enabled,
            agent_stats,
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
    /// desistir (4 s) e o boot terminar — a regressão que este `assert_future`
    /// impede de voltar em silêncio.
    ///
    /// O `apply_launch_config` está aqui por espera **própria**: ele chama
    /// `wait_for_boot` na primeira linha, porque precisa do layout carregado
    /// para decidir entre reusar e criar workspace. Antes só herdava a espera do
    /// `create_session` de cada slot, e a leitura do layout ficava do lado
    /// errado dela.
    ///
    /// Os dois de worktree entraram pelo mesmo motivo, com o defeito virado ao
    /// contrário: eles não travam nada, mas decidem o que **apagar** a partir de
    /// `sessions.list()`. Com a lista ainda vazia, o `remove` não vê a sessão
    /// viva que a guarda existe para proteger e o `gc` acha que todo worktree
    /// gerenciado é órfão.
    #[allow(dead_code)]
    fn comandos_que_esperam_o_boot_seguem_assincronos(
        app: AppHandle,
        state: State<'_, AppState>,
        opts: CreateSessionOpts,
        id: launch_config::LaunchConfigId,
        path: String,
    ) {
        // Uma chamada por comando, e não uma tupla de `impl Future`: a tupla
        // cresce a cada comando que entra no conjunto e o `type_complexity` do
        // clippy passa a reclamar. O `T` fixo em cada chamada mantém a guarda no
        // mesmo lugar — prende o tipo do `Output`, e um `fn` síncrono devolvendo
        // `Result` não é `Future` nenhum.
        fn assert_future<T>(_: impl std::future::Future<Output = T>) {}

        assert_future::<Result<Session, String>>(create_session(app.clone(), state.clone(), opts));
        assert_future::<Result<AppliedLaunchConfig, String>>(apply_launch_config(
            app,
            state.clone(),
            id,
            None,
            80,
            24,
        ));
        assert_future::<Result<(), String>>(worktree_remove(state.clone(), path, false, false));
        assert_future::<Result<worktree::GcReport, String>>(worktree_gc(state));
    }

    /// A irmã da guarda acima, para o OUTRO portão.
    ///
    /// `submit_shell_line` espera o shell abrir o editor de linha dele. Voltar
    /// a ser síncrono não quebraria compilação em lugar nenhum, e nenhum teste
    /// falharia por comando não executado — ele executa. O que volta é o eco
    /// cru da injeção no topo da sessão e o atraso de um `.zshrc` inteiro entre
    /// o Enter e o comando. Defeito que não quebra teste é o que esta guarda
    /// existe para prender.
    #[allow(dead_code)]
    fn comandos_que_esperam_o_shell_seguem_assincronos(
        state: State<'_, AppState>,
        id: SessionId,
        text: String,
    ) {
        fn assert_future<T>(_: impl std::future::Future<Output = T>) {}

        assert_future::<Result<(), String>>(submit_shell_line(state, id, text));
    }

    /// O `None` do layout antes do boot não é "não existe", é "ainda não li":
    /// `state.layout` só é populado pelo `load_remapped` da thread de boot, e
    /// até lá `workspace_of_launch_config` responde `None` para toda
    /// configuração — inclusive as que já têm workspace. Tratar isso como
    /// ausência faz o `apply_launch_config` criar um SEGUNDO workspace para a
    /// mesma launch config.
    #[test]
    fn layout_nao_carregado_nao_autoriza_criar_workspace() {
        assert_eq!(decide_launch_reuse(false, None), LaunchReuse::Unknown);
    }

    /// O contrapeso do teste acima: com o layout carregado, "não achei" volta a
    /// significar "não existe". Sem este, recusar sempre passaria.
    #[test]
    fn layout_carregado_e_sem_workspace_autoriza_criar() {
        assert_eq!(decide_launch_reuse(true, None), LaunchReuse::Create);
    }

    /// Achar é prova positiva, e o portão fechado não a desfaz. Existe a fresta
    /// entre o `load_remapped` e o `mark_ready` em que o layout já está lido e o
    /// portão ainda não abriu; recusar o reuso ali criaria a duplicata que a
    /// recusa existe para evitar.
    #[test]
    fn workspace_encontrado_dispensa_o_portao() {
        let ws = uuid::Uuid::new_v4();
        assert_eq!(decide_launch_reuse(false, Some(ws)), LaunchReuse::Reuse(ws));
    }

    #[test]
    fn workspace_encontrado_com_layout_carregado_e_reusado() {
        let ws = uuid::Uuid::new_v4();
        assert_eq!(decide_launch_reuse(true, Some(ws)), LaunchReuse::Reuse(ws));
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

    /// O evento sozinho não basta: `spawn_boot` começa a thread dentro do
    /// `.setup()`, antes de o webview carregar, e o `listen()` do Tauri é
    /// assíncrono. Um pânico dentro do `ssh::config::materialize` ou do
    /// `sessions.restore()` dispara em milissegundos, antes de o listener
    /// existir, e o core não reemite. A mensagem retida no portão é o que o
    /// `boot_snapshot` devolve para quem perdeu o evento.
    #[test]
    fn a_falha_do_boot_continua_legivel_depois_do_evento() {
        let gate = boot::BootGate::new();
        let failed = guard_boot(&gate, || panic!("materialize explodiu"));

        assert_eq!(failed.as_deref(), Some("materialize explodiu"));
        assert_eq!(
            gate.failure().map(|f| f.message).as_deref(),
            Some("materialize explodiu")
        );
    }

    #[test]
    fn boot_sem_panico_nao_deixa_falha_no_portao() {
        let gate = boot::BootGate::new();
        let _ = guard_boot(&gate, || gate.mark_ready());
        assert_eq!(gate.failure(), None);
    }

    /// Mesma razão de `panico_depois_do_ready_nao_vira_falha_de_boot`: o que
    /// roda depois do `mark_ready` é manutenção, e um banner de "o app não
    /// carregou" por causa dela seria mentira — inclusive no snapshot.
    #[test]
    fn panico_depois_do_ready_nao_aparece_no_snapshot() {
        let gate = boot::BootGate::new();
        let _ = guard_boot(&gate, || {
            gate.mark_ready();
            panic!("gc de worktree explodiu");
        });
        assert_eq!(gate.failure(), None);
    }

    fn empty_boot_loaded() -> BootLoaded {
        BootLoaded {
            sessions: Vec::new(),
            layout: layout::LayoutState {
                workspaces: Vec::new(),
                active_workspace: None,
            },
        }
    }

    /// O poll do boot roda a cada ~150ms **enquanto o boot não termina**, e o
    /// front descarta tudo que não venha `ready`. Cada tick que lê prefs, sessões
    /// e layout paga o `Mutex<Connection>` e os locks de sessão/layout que a
    /// thread de boot está usando naquele exato instante para `restore` e
    /// `drain_checkpoints`: quem espera o boot terminar não pode ser quem o
    /// atrasa. Contar as leituras é o que prova que o portão fechado responde de
    /// graça — a economia não aparece em nenhuma asserção sobre o payload.
    #[test]
    fn a_closed_gate_answers_without_reading_any_state() {
        let gate = boot::BootGate::new();
        let reads = std::cell::Cell::new(0u32);

        for _ in 0..10 {
            let snapshot = boot_gate_snapshot(&gate, || {
                reads.set(reads.get() + 1);
                empty_boot_loaded()
            });
            assert!(!snapshot.ready);
            assert!(snapshot.loaded.is_none());
        }

        assert_eq!(reads.get(), 0, "o poll leu estado que o front descarta");
    }

    /// O contrapeso do teste acima: sem ele, um retorno cedo incondicional —
    /// que nunca entregaria sessão nem layout — passaria igual.
    #[test]
    fn an_open_gate_reads_the_state_once() {
        let gate = boot::BootGate::new();
        gate.mark_ready();
        let reads = std::cell::Cell::new(0u32);

        let snapshot = boot_gate_snapshot(&gate, || {
            reads.set(reads.get() + 1);
            empty_boot_loaded()
        });

        assert!(snapshot.ready);
        assert!(snapshot.loaded.is_some());
        assert_eq!(reads.get(), 1);
    }

    /// O retorno cedo não pode engolir a notícia da falha: ela é a única segunda
    /// via do `app://boot-failed`, que dispara antes de o listener do front
    /// existir. E não engole porque não chega a ser o portador — `mark_failed`
    /// grava a mensagem e abre o portão sob o mesmo lock, então toda falha sai
    /// por aqui, pelo caminho `ready: true`, com o payload junto.
    #[test]
    fn a_dead_boot_thread_reports_through_the_gate_snapshot() {
        let gate = boot::BootGate::new();
        gate.mark_failed("restore explodiu");

        let snapshot = boot_gate_snapshot(&gate, empty_boot_loaded);

        assert!(snapshot.ready);
        assert_eq!(
            snapshot.boot_failure.map(|f| f.message).as_deref(),
            Some("restore explodiu")
        );
        assert!(snapshot.loaded.is_some());
    }

    /// Boot em andamento não é falha: `bootFailure` nulo com `ready: false` é o
    /// caso normal do poll, e reportá-lo viraria banner de "o app não carregou"
    /// em cima de um app que está carregando.
    #[test]
    fn a_pending_boot_is_not_a_failure() {
        let gate = boot::BootGate::new();
        let snapshot = boot_gate_snapshot(&gate, empty_boot_loaded);
        assert!(!snapshot.ready);
        assert_eq!(snapshot.boot_failure, None);
    }

    /// O banco degradado é conhecido no `.setup()`, antes de a thread de boot
    /// começar, e registrá-lo não abre o portão — o estado ainda vai carregar.
    /// O retorno cedo do poll é por onde essa notícia chega, então ele não pode
    /// engoli-la: o `loaded` continua ausente (não há o que entregar), mas a
    /// falha viaja.
    #[test]
    fn a_degraded_store_reports_before_the_gate_opens() {
        let gate = boot::BootGate::new();
        gate.note_failure("o banco de sessões não abriu");

        let snapshot = boot_gate_snapshot(&gate, empty_boot_loaded);

        assert!(!snapshot.ready);
        assert!(snapshot.loaded.is_none());
        assert_eq!(
            snapshot.boot_failure.map(|f| f.message).as_deref(),
            Some("o banco de sessões não abriu")
        );
    }

    /// E continua reportando depois que o boot termina bem — que é o caso real
    /// do banco degradado: o arranque completa, e o que falta falta porque o
    /// banco não tinha para dar.
    #[test]
    fn a_degraded_store_still_reports_after_a_clean_boot() {
        let gate = boot::BootGate::new();
        gate.note_failure("o banco de sessões não abriu");
        let failed = guard_boot(&gate, || gate.mark_ready());

        // Não houve pânico: quem reporta é o campo do portão, não o retorno.
        assert_eq!(failed, None);
        let snapshot = boot_gate_snapshot(&gate, empty_boot_loaded);
        assert!(snapshot.ready);
        assert_eq!(
            snapshot.boot_failure.map(|f| f.message).as_deref(),
            Some("o banco de sessões não abriu")
        );
    }

    /// O caminho de pânico tem de deixar o app no mesmo estado observável do
    /// caminho feliz — e o pontapé da reconciliação faz parte dele. Sem este
    /// `send`, a thread `repo-reconcile` fica parada no `recv()` e os chips de
    /// branch e de diff nunca se preenchem, em cima de um app que já perdeu as
    /// sessões, até que um `EVENT_CWD_CHANGED` qualquer chegue por acaso.
    #[test]
    fn a_dead_boot_still_kicks_the_reconciler() {
        let app = tauri::test::mock_app();
        let (tx, rx) = std::sync::mpsc::channel::<()>();

        finish_failed_boot(
            app.handle(),
            &tx,
            boot::Failure {
                kind: boot::FailureKind::BootThreadDied,
                message: "restore explodiu".into(),
            },
        );

        assert!(
            rx.try_recv().is_ok(),
            "o ramo de falha do boot não cutucou o canal de reconciliação"
        );
    }

    fn git_in_test(dir: &std::path::Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?} falhou em {}", dir.display());
    }

    /// Repositório de verdade num temp: `repo::toplevel` chama o binário do git,
    /// e um fixture de mentira não distinguiria "existe" de "sumiu" — que é
    /// exatamente a distinção sob teste.
    fn temp_repo() -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("tyba-roots-{}", uuid::Uuid::new_v4()));
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git_in_test(&repo, &["init", "-q", "-b", "main"]);
        git_in_test(&repo, &["config", "user.email", "t@t.com"]);
        git_in_test(&repo, &["config", "user.name", "t"]);
        // Assinatura desligada, como nos outros fixtures de git do repo. Com
        // `commit.gpgsign` ligado no global — e é o caso de quem assina com
        // 1Password ou chave em hardware —, o `git commit` daqui vai ao agente
        // e pendura o teste esperando uma aprovação que ninguém vai dar.
        git_in_test(&repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git_in_test(&repo, &["add", "-A"]);
        git_in_test(&repo, &["commit", "-qm", "init"]);
        repo
    }

    fn agent_session(id: SessionId, worktree: Option<std::path::PathBuf>) -> Session {
        Session {
            id,
            kind: SessionKind::Agent {
                runner: session::AgentRunnerKind::ClaudeCode,
            },
            title: "agente".into(),
            repo_root: None,
            worktree: worktree.map(|path| worktree::Worktree {
                path,
                branch: "tyba/x".into(),
                base_ref: "0".repeat(40),
                dirty: false,
                ahead: 0,
            }),
            // Encerrada: é o caso que não tinha raiz observada.
            status: SessionStatus::Exited { code: 0 },
            attention: false,
            created_at: chrono::Utc::now(),
            cwd: None,
            connection: session::ConnectionState::Live,
            agent_conversation_id: None,
            observed: None,
        }
    }

    fn leaf(session: SessionId) -> layout::PaneNode {
        layout::PaneNode::Leaf {
            id: uuid::Uuid::new_v4(),
            session_id: session,
        }
    }

    fn workspace_with(roots: Vec<layout::PaneNode>) -> layout::LayoutState {
        let tabs = roots
            .into_iter()
            .map(|root| layout::Tab {
                id: uuid::Uuid::new_v4(),
                title: None,
                view: None,
                active_pane: None,
                root: Some(root),
                created_at: chrono::Utc::now(),
            })
            .collect();
        layout::LayoutState {
            workspaces: vec![layout::Workspace {
                id: uuid::Uuid::new_v4(),
                name: "w".into(),
                name_locked: false,
                repo_root: None,
                color: None,
                group: None,
                kind: layout::WorkspaceKind::User,
                launch_config_id: None,
                active_tab: None,
                tabs,
                side_view: None,
                side_ratio: 0.5,
                side_expanded: false,
                created_at: chrono::Utc::now(),
            }],
            active_workspace: None,
        }
    }

    /// Split aninhado, tab que não é a ativa e pane de `AgentViewer` contam: o
    /// que decide é estar preso a um pane, não estar visível agora.
    #[test]
    fn as_sessoes_de_todos_os_panes_entram_no_corte() {
        let visible = uuid::Uuid::new_v4();
        let nested = uuid::Uuid::new_v4();
        let other_tab = uuid::Uuid::new_v4();
        let viewer = uuid::Uuid::new_v4();

        let split = layout::PaneNode::Split {
            id: uuid::Uuid::new_v4(),
            split: layout::SplitKind::H,
            ratio: 0.5,
            first: Box::new(leaf(visible)),
            second: Box::new(layout::PaneNode::Split {
                id: uuid::Uuid::new_v4(),
                split: layout::SplitKind::V,
                ratio: 0.5,
                first: Box::new(leaf(nested)),
                second: Box::new(layout::PaneNode::AgentViewer {
                    id: uuid::Uuid::new_v4(),
                    session_id: viewer,
                }),
            }),
        };
        let layout = workspace_with(vec![split, leaf(other_tab)]);

        let bound = pane_bound_sessions(&layout);
        assert_eq!(bound.len(), 4);
        for id in [visible, nested, other_tab, viewer] {
            assert!(bound.contains(&id), "{id} ficou de fora");
        }
    }

    #[test]
    fn layout_sem_pane_nenhum_nao_prende_sessao() {
        assert!(pane_bound_sessions(&workspace_with(Vec::new())).is_empty());
    }

    /// O achado: sessão de agente ENCERRADA com worktree passa a ganhar raiz
    /// observada — sem ela o chip de branch não tinha o que mostrar.
    #[test]
    fn o_worktree_de_uma_sessao_encerrada_num_pane_vira_raiz_observada() {
        let repo = temp_repo();
        let id = uuid::Uuid::new_v4();
        let sessions = vec![agent_session(id, Some(repo.clone()))];
        let bound: std::collections::HashSet<SessionId> = [id].into_iter().collect();

        let roots = session_worktree_roots(&sessions, &bound);
        assert_eq!(
            roots,
            [repo::canonicalize_or(&repo)].into_iter().collect(),
            "worktree de sessão encerrada não virou raiz"
        );
        std::fs::remove_dir_all(repo.parent().unwrap()).ok();
    }

    /// O corte. Sessão fora de qualquer pane não pode ser mostrada por chip
    /// nenhum, e a tabela `sessions` guarda todas as de todos os tempos.
    #[test]
    fn sessao_fora_de_pane_nao_vira_raiz_observada() {
        let repo = temp_repo();
        let sessions = vec![agent_session(uuid::Uuid::new_v4(), Some(repo.clone()))];

        let roots = session_worktree_roots(&sessions, &std::collections::HashSet::new());
        assert!(roots.is_empty(), "{roots:?}");
        std::fs::remove_dir_all(repo.parent().unwrap()).ok();
    }

    /// `gc_orphans` remove órfão, e o usuário pode apagar a pasta na mão.
    /// Caminho morto sai da lista em silêncio — nem erro, nem raiz fantasma.
    #[test]
    fn worktree_que_sumiu_do_disco_e_descartado_em_silencio() {
        let id = uuid::Uuid::new_v4();
        let ghost = std::env::temp_dir().join(format!("tyba-sumiu-{}", uuid::Uuid::new_v4()));
        let sessions = vec![agent_session(id, Some(ghost))];
        let bound: std::collections::HashSet<SessionId> = [id].into_iter().collect();

        assert!(session_worktree_roots(&sessions, &bound).is_empty());
    }

    fn worktrees_bound_to_panes(
        paths: &[&str],
    ) -> (Vec<Session>, std::collections::HashSet<SessionId>) {
        let sessions: Vec<Session> = paths
            .iter()
            .map(|path| agent_session(uuid::Uuid::new_v4(), Some(std::path::PathBuf::from(*path))))
            .collect();
        let bound = sessions.iter().map(|s| s.id).collect();
        (sessions, bound)
    }

    /// Nenhum destes caminhos chega ao `git rev-parse`.
    ///
    /// O chamador é a thread `repo-reconcile`: `repo::toplevel` faz shell-out e
    /// bloqueia no `output()`. Num mount morto o `git` fica em I/O
    /// ininterrompível, a thread nunca volta e o `EVENT_RECONCILED` para de sair
    /// para **todos** os repositórios. `/mnt` e `/media` entram na lista por
    /// causa desse custo — o arranque, que paga uma aba, os deixa passar. Ver
    /// `session::cwd::may_hang_shared_thread`.
    #[test]
    fn worktree_em_mount_suspeito_nao_chega_ao_git() {
        let (sessions, bound) = worktrees_bound_to_panes(&[
            "/Volumes/NAS/repo",
            "/Network/Servers/ci/repo",
            "/net/host/share/repo",
            "/mnt/nas/repo",
            "/media/nas/repo",
            r"\\servidor\share\repo",
            "//servidor/share/repo",
        ]);

        let touchable: Vec<_> = touchable_worktrees(&sessions, &bound).collect();
        assert!(touchable.is_empty(), "{touchable:?}");
    }

    /// O contraponto que separa o filtro certo de um `filter` que descarta tudo:
    /// worktree local — inclusive sob pasta do TCC, onde o diálogo tem fim —
    /// continua chegando ao `git`.
    #[test]
    fn worktree_local_continua_chegando_ao_git() {
        let (sessions, bound) = worktrees_bound_to_panes(&[
            "/Users/tester/code/tyba",
            "/Users/tester/Documents/tyba",
            "/tmp/tyba",
        ]);

        let touchable: Vec<_> = touchable_worktrees(&sessions, &bound).collect();
        assert_eq!(touchable.len(), 3, "{touchable:?}");
    }

    #[test]
    fn sessao_sem_worktree_nao_acrescenta_raiz() {
        let id = uuid::Uuid::new_v4();
        let sessions = vec![agent_session(id, None)];
        let bound: std::collections::HashSet<SessionId> = [id].into_iter().collect();

        assert!(session_worktree_roots(&sessions, &bound).is_empty());
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
