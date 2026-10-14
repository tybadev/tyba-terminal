use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session::store::Store;
use crate::session::SessionId;

pub mod history;
pub mod screens;

pub const EVENT_CHANGED: &str = "layout://changed";

const KEY_ACTIVE_WORKSPACE: &str = "layout.active_workspace";
/// JSON `{ <workspace id>: FilesScreen }` — a tela de Arquivos de cada
/// workspace, que não cabe nas linhas de layout.
const KEY_FILE_SCREENS: &str = "layout.file_screens";
const MIN_RATIO: f64 = 0.1;
const MAX_RATIO: f64 = 0.9;

pub type WorkspaceId = Uuid;
pub type TabId = Uuid;
pub type PaneId = Uuid;

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("sessão não encontrada: {0}")]
    WorkspaceNotFound(WorkspaceId),
    #[error("nenhuma sessão ativa")]
    NoActiveWorkspace,
    #[error("tab não encontrada: {0}")]
    TabNotFound(TabId),
    #[error("pane não encontrado: {0}")]
    PaneNotFound(PaneId),
    #[error("pane não é um split: {0}")]
    NotASplit(PaneId),
    #[error("processo já aberto em um pane: {0}")]
    SessionAlreadyBound(SessionId),
    #[error("sessão sem pane no workspace ativo: {0}")]
    SessionNotVisible(SessionId),
    #[error("store: {0}")]
    Store(#[from] crate::session::store::StoreError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitKind {
    H,
    V,
}

impl SplitKind {
    fn as_str(self) -> &'static str {
        match self {
            SplitKind::H => "h",
            SplitKind::V => "v",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "h" => Some(SplitKind::H),
            "v" => Some(SplitKind::V),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PaneNode {
    Leaf {
        id: PaneId,
        session_id: SessionId,
    },
    AgentViewer {
        id: PaneId,
        session_id: SessionId,
    },
    Split {
        id: PaneId,
        split: SplitKind,
        ratio: f64,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

impl PaneNode {
    fn id(&self) -> PaneId {
        match self {
            PaneNode::Leaf { id, .. }
            | PaneNode::AgentViewer { id, .. }
            | PaneNode::Split { id, .. } => *id,
        }
    }

    fn first_leaf(&self) -> PaneId {
        match self {
            PaneNode::Leaf { id, .. } | PaneNode::AgentViewer { id, .. } => *id,
            PaneNode::Split { first, .. } => first.first_leaf(),
        }
    }

    fn contains(&self, pane: PaneId) -> bool {
        match self {
            PaneNode::Leaf { id, .. } | PaneNode::AgentViewer { id, .. } => *id == pane,
            PaneNode::Split {
                id, first, second, ..
            } => *id == pane || first.contains(pane) || second.contains(pane),
        }
    }

    fn find_leaf_by_session(&self, session: SessionId) -> Option<PaneId> {
        match self {
            PaneNode::Leaf { id, session_id } if *session_id == session => Some(*id),
            PaneNode::Leaf { .. } | PaneNode::AgentViewer { .. } => None,
            PaneNode::Split { first, second, .. } => first
                .find_leaf_by_session(session)
                .or_else(|| second.find_leaf_by_session(session)),
        }
    }

    fn find_pane_of_session(&self, session: SessionId) -> Option<PaneId> {
        match self {
            PaneNode::Leaf { id, session_id } | PaneNode::AgentViewer { id, session_id }
                if *session_id == session =>
            {
                Some(*id)
            }
            PaneNode::Leaf { .. } | PaneNode::AgentViewer { .. } => None,
            PaneNode::Split { first, second, .. } => first
                .find_pane_of_session(session)
                .or_else(|| second.find_pane_of_session(session)),
        }
    }

    fn collect_agent_viewer_panes(&self, session: SessionId, out: &mut Vec<PaneId>) {
        match self {
            PaneNode::AgentViewer { id, session_id } if *session_id == session => out.push(*id),
            PaneNode::Leaf { .. } | PaneNode::AgentViewer { .. } => {}
            PaneNode::Split { first, second, .. } => {
                first.collect_agent_viewer_panes(session, out);
                second.collect_agent_viewer_panes(session, out);
            }
        }
    }

    fn contains_agent_viewer(&self, session: SessionId) -> bool {
        match self {
            PaneNode::AgentViewer { session_id, .. } => *session_id == session,
            PaneNode::Leaf { .. } => false,
            PaneNode::Split { first, second, .. } => {
                first.contains_agent_viewer(session) || second.contains_agent_viewer(session)
            }
        }
    }

    fn agent_viewer_session(&self, pane: PaneId) -> Option<SessionId> {
        match self {
            PaneNode::AgentViewer { id, session_id } if *id == pane => Some(*session_id),
            PaneNode::Leaf { .. } | PaneNode::AgentViewer { .. } => None,
            PaneNode::Split { first, second, .. } => first
                .agent_viewer_session(pane)
                .or_else(|| second.agent_viewer_session(pane)),
        }
    }

    fn leaf_session(&self, pane: PaneId) -> Option<SessionId> {
        match self {
            PaneNode::Leaf { id, session_id } | PaneNode::AgentViewer { id, session_id }
                if *id == pane =>
            {
                Some(*session_id)
            }
            PaneNode::Leaf { .. } | PaneNode::AgentViewer { .. } => None,
            PaneNode::Split { first, second, .. } => first
                .leaf_session(pane)
                .or_else(|| second.leaf_session(pane)),
        }
    }

    fn split_leaf(
        &mut self,
        target: PaneId,
        kind: SplitKind,
        session: SessionId,
    ) -> Option<PaneId> {
        match self {
            PaneNode::Leaf { id, .. } if *id == target => {
                let new_leaf = PaneNode::Leaf {
                    id: Uuid::new_v4(),
                    session_id: session,
                };
                let new_id = new_leaf.id();
                let old = std::mem::replace(
                    self,
                    PaneNode::Leaf {
                        id: Uuid::new_v4(),
                        session_id: session,
                    },
                );
                *self = PaneNode::Split {
                    id: Uuid::new_v4(),
                    split: kind,
                    ratio: 0.5,
                    first: Box::new(old),
                    second: Box::new(new_leaf),
                };
                Some(new_id)
            }
            PaneNode::Leaf { .. } | PaneNode::AgentViewer { .. } => None,
            PaneNode::Split { first, second, .. } => first
                .split_leaf(target, kind, session)
                .or_else(|| second.split_leaf(target, kind, session)),
        }
    }

    fn remove_leaf(self, target: PaneId) -> Option<PaneNode> {
        match self {
            PaneNode::Leaf { id, .. } | PaneNode::AgentViewer { id, .. } if id == target => None,
            leaf @ (PaneNode::Leaf { .. } | PaneNode::AgentViewer { .. }) => Some(leaf),
            PaneNode::Split {
                id,
                split,
                ratio,
                first,
                second,
            } => {
                let first = first.remove_leaf(target);
                let second = second.remove_leaf(target);
                match (first, second) {
                    (Some(a), Some(b)) => Some(PaneNode::Split {
                        id,
                        split,
                        ratio,
                        first: Box::new(a),
                        second: Box::new(b),
                    }),
                    (Some(only), None) | (None, Some(only)) => Some(only),
                    (None, None) => None,
                }
            }
        }
    }

    fn set_ratio(&mut self, target: PaneId, value: f64) -> bool {
        match self {
            PaneNode::Leaf { .. } | PaneNode::AgentViewer { .. } => false,
            PaneNode::Split {
                id,
                ratio,
                first,
                second,
                ..
            } => {
                if *id == target {
                    *ratio = value.clamp(MIN_RATIO, MAX_RATIO);
                    true
                } else {
                    first.set_ratio(target, value) || second.set_ratio(target, value)
                }
            }
        }
    }

    fn leaf_sessions(&self, out: &mut Vec<SessionId>) {
        match self {
            PaneNode::Leaf { session_id, .. } => out.push(*session_id),
            PaneNode::AgentViewer { .. } => {}
            PaneNode::Split { first, second, .. } => {
                first.leaf_sessions(out);
                second.leaf_sessions(out);
            }
        }
    }

    fn retain_sessions(self, valid: &HashSet<SessionId>) -> Option<PaneNode> {
        match self {
            PaneNode::Leaf { session_id, .. } | PaneNode::AgentViewer { session_id, .. }
                if !valid.contains(&session_id) =>
            {
                None
            }
            leaf @ (PaneNode::Leaf { .. } | PaneNode::AgentViewer { .. }) => Some(leaf),
            PaneNode::Split {
                id,
                split,
                ratio,
                first,
                second,
            } => {
                let first = first.retain_sessions(valid);
                let second = second.retain_sessions(valid);
                match (first, second) {
                    (Some(a), Some(b)) => Some(PaneNode::Split {
                        id,
                        split,
                        ratio,
                        first: Box::new(a),
                        second: Box::new(b),
                    }),
                    (Some(only), None) | (None, Some(only)) => Some(only),
                    (None, None) => None,
                }
            }
        }
    }
}

/// Identidade que o workspace já nasce com (nome travado, cor, grupo) em vez de
/// receber por mutações em sequência.
#[derive(Debug, Clone, Default)]
pub struct Tag {
    pub lock_name: bool,
    pub color: Option<String>,
    pub group: Option<String>,
}

/// Aba do workspace Docker antigo (projetos compose), aposentada pela seção
/// Compose da tela de Docker. Só existe para migrar layout salvo: ao carregar,
/// a aba some e quem estava nela cai na seção Compose.
const RETIRED_VIEW_CONTAINERS: &str = "containers";
pub const VIEW_SETTINGS: &str = "settings";
pub const VIEW_WORKSPACE: &str = "workspace";
pub const VIEW_CONNECTIONS: &str = "connections";
pub const VIEW_STATS: &str = "stats";
pub const VIEW_DIFF_PREFIX: &str = "diff:";
pub const VIEW_TUNNELS_PREFIX: &str = "tunnels:";
pub const VIEW_FILES_PREFIX: &str = "files:";
pub const VIEW_AGENTS_PREFIX: &str = "agents:";
/// Painel Git da sessão (branch, à frente/atrás, alterações, commit).
pub const VIEW_GIT_PREFIX: &str = "git:";
/// Painel Docker da sessão: os containers do alvo dela (local, ou o servidor
/// numa sessão SSH) — por isso é da sessão e morre com ela.
pub const VIEW_DOCKER_PREFIX: &str = "docker:";
/// Painel Histórico da sessão: as execuções gravadas (OSC 633).
pub const VIEW_HISTORY_PREFIX: &str = "history:";
/// O quadro de agentes de **todos** os workspaces. Nome sem dois-pontos de
/// propósito: `VIEW_AGENTS_PREFIX` acima é o visualizador de subagente de UMA
/// sessão, e são coisas diferentes.
pub const VIEW_AGENT_BOARD: &str = "agent-board";
/// A fila de agentes, como painel lateral.
///
/// Único `side_view` **sem sessão**: os outros (`diff:`, `files:`, `agents:`)
/// pertencem a uma sessão e morrem com ela. Este é do workspace inteiro, e a
/// limpeza de layout já o preserva de graça — o filtro usa `is_none_or`, ou
/// seja, view sem sessão sobrevive por desenho.
pub const VIEW_AGENT_QUEUE: &str = "agent-queue";
/// Aba de conteúdo: um arquivo aberto no editor, `editor:<sessão>:<caminho
/// relativo à raiz de arquivos da sessão>`. Pertence à sessão — a raiz, o LSP e
/// o watcher são dela — e morre com ela.
pub const VIEW_EDITOR_PREFIX: &str = "editor:";
/// Aba de conteúdo: logs estruturados de um container, `container-logs:<id>`.
pub const VIEW_CONTAINER_LOGS_PREFIX: &str = "container-logs:";
/// Painel lateral: arquivos de um container (só leitura), `container-files:<id>`.
pub const VIEW_CONTAINER_FILES_PREFIX: &str = "container-files:";
/// Aba de conteúdo: o diff da sessão como aba (não painel), `session-diff:<sessão>`.
/// Prefixo próprio porque `diff:` em aba é a forma antiga, descartada no load.
pub const VIEW_SESSION_DIFF_PREFIX: &str = "session-diff:";
/// Aba de conteúdo: histórico (log + grafo) do repo da sessão, `git-log:<sessão>`.
pub const VIEW_GIT_LOG_PREFIX: &str = "git-log:";
/// Aba de conteúdo: lista de PRs do repo da sessão, `prs:<sessão>`.
pub const VIEW_PR_LIST_PREFIX: &str = "prs:";
/// Aba de conteúdo: um PR do repo da sessão, `pr:<sessão>:<número>`.
pub const VIEW_PR_PREFIX: &str = "pr:";
pub const DOCKER_WORKSPACE_NAME: &str = "Docker";
pub const FALLBACK_WORKSPACE_NAME: &str = "tyba";

const DEFAULT_SIDE_RATIO: f64 = 0.5;
const AGENT_VIEWER_RATIO: f64 = 0.62;
const AGENT_VIEWER_ROW: &str = "agent";

pub fn diff_view(session: SessionId) -> String {
    format!("{VIEW_DIFF_PREFIX}{session}")
}

pub fn tunnels_view(session: SessionId) -> String {
    format!("{VIEW_TUNNELS_PREFIX}{session}")
}

pub fn files_view(session: SessionId) -> String {
    format!("{VIEW_FILES_PREFIX}{session}")
}

pub fn agents_view(session: SessionId) -> String {
    format!("{VIEW_AGENTS_PREFIX}{session}")
}

pub fn git_view(session: SessionId) -> String {
    format!("{VIEW_GIT_PREFIX}{session}")
}

pub fn docker_view(session: SessionId) -> String {
    format!("{VIEW_DOCKER_PREFIX}{session}")
}

pub fn history_view(session: SessionId) -> String {
    format!("{VIEW_HISTORY_PREFIX}{session}")
}

/// Aba só leitura da tela de Arquivos com um arquivo compose de qualquer
/// projeto: `compose-file:<sessão>:<projeto>:<índice>:<nome>`. Não volta no
/// reinício (o projeto é do retrato de containers desta execução).
pub const VIEW_COMPOSE_FILE_PREFIX: &str = "compose-file:";

pub fn compose_file_view(session: SessionId, project: &str, index: usize, name: &str) -> String {
    format!("{VIEW_COMPOSE_FILE_PREFIX}{session}:{project}:{index}:{name}")
}

pub fn editor_view(session: SessionId, rel: &str) -> String {
    format!("{VIEW_EDITOR_PREFIX}{session}:{rel}")
}

/// Sessão dona de uma aba de editor. `None` para qualquer outra view.
pub fn editor_view_session(view: &str) -> Option<SessionId> {
    let rest = view.strip_prefix(VIEW_EDITOR_PREFIX)?;
    let (sid, _) = rest.split_once(':')?;
    Uuid::parse_str(sid).ok()
}

/// Sessão dona de uma aba da tela de Arquivos: editor ou compose só leitura.
pub fn screen_file_session(view: &str) -> Option<SessionId> {
    editor_view_session(view).or_else(|| {
        let rest = view.strip_prefix(VIEW_COMPOSE_FILE_PREFIX)?;
        let (sid, _) = rest.split_once(':')?;
        Uuid::parse_str(sid).ok()
    })
}

/// Containers com aba de logs aberta em algum workspace — o que pode ter um
/// `docker logs -f` vivo.
pub fn open_log_containers(state: &LayoutState) -> HashSet<String> {
    let tabs = state
        .workspaces
        .iter()
        .flat_map(|w| w.tabs.iter())
        .filter_map(|t| t.view.as_deref()?.strip_prefix(VIEW_CONTAINER_LOGS_PREFIX));
    // A tela de Docker aberta na seção Logs também segura o seguidor.
    let screens = state
        .workspaces
        .iter()
        .filter(|w| {
            w.side_view
                .as_deref()
                .is_some_and(|v| v.starts_with(VIEW_DOCKER_PREFIX))
        })
        .filter_map(|w| w.screens.docker.following());
    tabs.chain(screens).map(str::to_string).collect()
}

pub fn session_diff_view(session: SessionId) -> String {
    format!("{VIEW_SESSION_DIFF_PREFIX}{session}")
}

pub fn git_log_view(session: SessionId) -> String {
    format!("{VIEW_GIT_LOG_PREFIX}{session}")
}

pub fn pr_list_view(session: SessionId) -> String {
    format!("{VIEW_PR_LIST_PREFIX}{session}")
}

pub fn pr_view(session: SessionId, number: u64) -> String {
    format!("{VIEW_PR_PREFIX}{session}:{number}")
}

fn pr_view_session(view: &str) -> Option<SessionId> {
    let (sid, _) = view.strip_prefix(VIEW_PR_PREFIX)?.split_once(':')?;
    Uuid::parse_str(sid).ok()
}

/// Sessão dona de uma aba de conteúdo (editor, diff, histórico ou PRs da sessão).
pub fn content_view_session(view: &str) -> Option<SessionId> {
    editor_view_session(view)
        .or_else(|| pr_view_session(view))
        .or_else(|| {
            view.strip_prefix(VIEW_SESSION_DIFF_PREFIX)
                .or_else(|| view.strip_prefix(VIEW_GIT_LOG_PREFIX))
                .or_else(|| view.strip_prefix(VIEW_PR_LIST_PREFIX))
                .and_then(|s| Uuid::parse_str(s).ok())
        })
}

fn diff_view_session(view: &str) -> Option<SessionId> {
    view.strip_prefix(VIEW_DIFF_PREFIX)
        .and_then(|s| Uuid::parse_str(s).ok())
}

fn side_view_session(view: &str) -> Option<SessionId> {
    view.strip_prefix(VIEW_DIFF_PREFIX)
        .or_else(|| view.strip_prefix(VIEW_TUNNELS_PREFIX))
        .or_else(|| view.strip_prefix(VIEW_FILES_PREFIX))
        .or_else(|| view.strip_prefix(VIEW_AGENTS_PREFIX))
        .or_else(|| view.strip_prefix(VIEW_GIT_PREFIX))
        .or_else(|| view.strip_prefix(VIEW_DOCKER_PREFIX))
        .or_else(|| view.strip_prefix(VIEW_HISTORY_PREFIX))
        .and_then(|s| Uuid::parse_str(s).ok())
}

#[derive(Debug, Clone, Serialize)]
pub struct Tab {
    pub id: TabId,
    pub title: Option<String>,
    pub view: Option<String>,
    pub active_pane: Option<PaneId>,
    pub root: Option<PaneNode>,
    pub created_at: DateTime<Utc>,
}

impl Tab {
    fn from_session(session: SessionId) -> Self {
        let leaf = PaneNode::Leaf {
            id: Uuid::new_v4(),
            session_id: session,
        };
        Tab {
            id: Uuid::new_v4(),
            title: None,
            view: None,
            active_pane: Some(leaf.id()),
            root: Some(leaf),
            created_at: Utc::now(),
        }
    }

    fn from_view(view: &str) -> Self {
        Tab {
            id: Uuid::new_v4(),
            title: None,
            view: Some(view.to_string()),
            active_pane: None,
            root: None,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceKind {
    User,
    Docker,
}

impl WorkspaceKind {
    fn as_str(self) -> &'static str {
        match self {
            WorkspaceKind::User => "user",
            WorkspaceKind::Docker => "docker",
        }
    }

    fn parse(s: Option<&str>) -> Self {
        match s {
            Some("docker") => WorkspaceKind::Docker,
            _ => WorkspaceKind::User,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    /// true = o dono nomeou de propósito (rename); false = nome automático,
    /// a UI pode exibir o cwd vivo no lugar.
    pub name_locked: bool,
    pub repo_root: Option<String>,
    pub color: Option<String>,
    pub group: Option<String>,
    pub kind: WorkspaceKind,
    pub launch_config_id: Option<Uuid>,
    pub active_tab: Option<TabId>,
    pub tabs: Vec<Tab>,
    /// Painel lateral do workspace (ex.: "diff:<session>") — vive AO LADO
    /// da coluna de tabs, não dentro de um tab.
    pub side_view: Option<String>,
    pub side_ratio: f64,
    pub side_expanded: bool,
    /// Abas usadas antes da ativa, da mais antiga para a mais recente — o
    /// "voltar" (⌘[) e o destino de fechar a ativa. Memória de uso, não
    /// persiste (ver [`history`]).
    #[serde(skip)]
    pub history: Vec<TabId>,
    /// Telas cheias de Arquivos e Docker: abas internas, seleção. Vai no
    /// evento; a de Arquivos persiste à parte (`KEY_FILE_SCREENS`), a de
    /// Docker não.
    pub screens: screens::Screens,
    pub created_at: DateTime<Utc>,
}

impl Workspace {
    /// Sessão em foco: a do pane ativo — ou, numa aba de editor, a dona do
    /// arquivo (o diff e o painel de arquivos daquela sessão continuam valendo).
    fn focused_session(&self) -> Option<SessionId> {
        let tab = self.tabs.iter().find(|t| Some(t.id) == self.active_tab)?;
        if let Some(view) = tab.view.as_deref() {
            return content_view_session(view);
        }
        tab.root.as_ref()?.leaf_session(tab.active_pane?)
    }

    /// Troca a aba ativa registrando a anterior no histórico.
    fn set_active_tab(&mut self, tab: TabId) {
        history::record(&mut self.history, self.active_tab, tab);
        self.active_tab = Some(tab);
    }

    /// Remove a aba `t_idx` e, se era a ativa, passa o foco para a usada antes
    /// dela (ou a vizinha). Devolve a aba removida.
    fn remove_tab_at(&mut self, t_idx: usize) -> Tab {
        let removed = self.tabs.remove(t_idx);
        let remaining: Vec<TabId> = self.tabs.iter().map(|t| t.id).collect();
        if self.active_tab == Some(removed.id) {
            self.active_tab =
                history::successor_after_close(&mut self.history, removed.id, &remaining, t_idx);
        } else {
            self.history.retain(|t| *t != removed.id);
        }
        removed
    }

    fn bound_sessions(&self) -> Vec<SessionId> {
        let mut out = Vec::new();
        for tab in &self.tabs {
            if let Some(root) = &tab.root {
                root.leaf_sessions(&mut out);
            }
        }
        out
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LayoutState {
    pub workspaces: Vec<Workspace>,
    pub active_workspace: Option<WorkspaceId>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceRow {
    pub id: String,
    pub name: String,
    pub name_locked: Option<i64>,
    pub repo_root: Option<String>,
    pub color: Option<String>,
    pub group_name: Option<String>,
    pub kind: Option<String>,
    pub launch_config_id: Option<String>,
    pub position: i64,
    pub active_tab: Option<String>,
    pub side_view: Option<String>,
    pub side_ratio: Option<f64>,
    pub side_expanded: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct TabRow {
    pub id: String,
    pub workspace_id: Option<String>,
    pub title: Option<String>,
    pub view: Option<String>,
    pub position: i64,
    pub active_pane: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct PaneRow {
    pub id: String,
    pub tab_id: String,
    pub parent_id: Option<String>,
    pub split: Option<String>,
    pub ratio: Option<f64>,
    pub position: Option<i64>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutRows {
    pub workspaces: Vec<WorkspaceRow>,
    pub tabs: Vec<TabRow>,
    pub panes: Vec<PaneRow>,
}

struct Inner {
    workspaces: Vec<Workspace>,
    active: Option<WorkspaceId>,
}

/// O painel de diff mostra staged/unstaged do repo da SESSÃO — aberto com o
/// foco em outra sessão, induz stage/descarte no repo errado. Toda mutação
/// que muda o foco varre os workspaces e fecha o painel que ficou órfão.
fn drop_stale_diff_views(inner: &mut Inner) {
    for ws in inner.workspaces.iter_mut() {
        let Some(target) = ws.side_view.as_deref().and_then(diff_view_session) else {
            continue;
        };
        if ws.focused_session() != Some(target) {
            ws.side_view = None;
            ws.side_expanded = false;
        }
    }
}

pub struct LayoutManager {
    store: Arc<Store>,
    inner: RwLock<Inner>,
}

pub type SharedLayout = Arc<LayoutManager>;

impl LayoutManager {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            inner: RwLock::new(Inner {
                workspaces: Vec::new(),
                active: None,
            }),
        }
    }

    pub fn load(&self, valid_sessions: &HashSet<SessionId>) {
        self.load_remapped(valid_sessions, &std::collections::HashMap::new());
    }

    /// `remap` liga a sessão morta do processo anterior à que acabou de nascer no
    /// mesmo cwd. Sem isso o pane aponta pra um id que não existe mais, o filtro
    /// de `valid_sessions` o descarta, e a tab abre vazia — é exatamente o que
    /// fazia o reopen perder as tabs (#50).
    pub fn load_remapped(
        &self,
        valid_sessions: &HashSet<SessionId>,
        remap: &std::collections::HashMap<SessionId, SessionId>,
    ) {
        let mut rows = self.store.load_layout().unwrap_or_default();
        if !remap.is_empty() {
            for pane in rows.panes.iter_mut() {
                let Some(old) = pane
                    .session_id
                    .as_deref()
                    .and_then(|s| Uuid::parse_str(s).ok())
                else {
                    continue;
                };
                if let Some(new) = remap.get(&old) {
                    pane.session_id = Some(new.to_string());
                }
            }
        }
        if !remap.is_empty() {
            for tab in rows.tabs.iter_mut() {
                let Some(view) = tab.view.as_deref() else {
                    continue;
                };
                let Some(old) = content_view_session(view) else {
                    continue;
                };
                if let Some(new) = remap.get(&old) {
                    tab.view = Some(view.replacen(&old.to_string(), &new.to_string(), 1));
                }
            }
        }
        let mut workspaces = rows_to_workspaces(&rows, valid_sessions);
        let saved_files: std::collections::HashMap<String, screens::FilesScreen> = self
            .store
            .get_setting(KEY_FILE_SCREENS)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        for ws in workspaces.iter_mut() {
            let Some(mut files) = saved_files.get(&ws.id.to_string()).cloned() else {
                continue;
            };
            files.map_views(|view| remap_editor_view(view, remap));
            files.retain(|view| {
                editor_view_session(view).is_some_and(|s| valid_sessions.contains(&s))
            });
            // As abas de editor de layout antigo (migradas em
            // `rows_to_workspaces`) entram depois das salvas.
            let migrated = std::mem::replace(&mut ws.screens.files, files);
            ws.screens.files.absorb(migrated);
        }

        let saved_active = self
            .store
            .get_setting(KEY_ACTIVE_WORKSPACE)
            .ok()
            .flatten()
            .and_then(|s| Uuid::parse_str(&s).ok());
        let active = saved_active
            .filter(|id| workspaces.iter().any(|w| w.id == *id))
            .or_else(|| workspaces.first().map(|w| w.id));
        // Quem fechou o app no painel antigo de containers volta para a
        // seção Compose da tela de Docker, onde os projetos moram agora.
        if on_retired_dashboard(&rows, saved_active) {
            if let Some(ws) = workspaces.iter_mut().find(|w| Some(w.id) == active) {
                ws.screens.docker.section = screens::DockerSection::Compose;
            }
        }

        let mut inner = self.inner.write();
        inner.workspaces = workspaces;
        inner.active = active;
        drop(inner);
        let _ = self.persist();
    }

    pub fn state(&self) -> LayoutState {
        let inner = self.inner.read();
        LayoutState {
            workspaces: inner.workspaces.clone(),
            active_workspace: inner.active,
        }
    }

    pub fn create_workspace(
        &self,
        name: &str,
        repo_root: Option<String>,
        session: SessionId,
    ) -> Result<WorkspaceId, LayoutError> {
        self.create_workspace_tagged(name, repo_root, session, Tag::default())
    }

    /// Nasce com nome travado, cor e grupo de uma vez: criar e depois mutar em
    /// três chamadas persiste e emite layout a cada passo, e a sidebar pisca
    /// entre os estados intermediários antes de assentar.
    pub fn create_workspace_tagged(
        &self,
        name: &str,
        repo_root: Option<String>,
        session: SessionId,
        tag: Tag,
    ) -> Result<WorkspaceId, LayoutError> {
        let mut inner = self.inner.write();
        if find_session_pane(&inner.workspaces, session).is_some() {
            return Err(LayoutError::SessionAlreadyBound(session));
        }
        let tab = Tab::from_session(session);
        let workspace = Workspace {
            id: Uuid::new_v4(),
            name: name.trim().to_string(),
            name_locked: tag.lock_name,
            repo_root,
            color: tag.color,
            group: tag.group,
            kind: WorkspaceKind::User,
            launch_config_id: None,
            active_tab: Some(tab.id),
            tabs: vec![tab],
            side_view: None,
            side_ratio: DEFAULT_SIDE_RATIO,
            side_expanded: false,
            history: Vec::new(),
            screens: Default::default(),
            created_at: Utc::now(),
        };
        let id = workspace.id;
        inner.workspaces.push(workspace);
        inner.active = Some(id);
        drop(inner);
        self.persist()?;
        Ok(id)
    }

    pub fn docker_workspace(&self) -> Result<WorkspaceId, LayoutError> {
        let mut inner = self.inner.write();
        let id = ensure_docker_workspace(&mut inner);
        drop(inner);
        self.persist()?;
        Ok(id)
    }

    pub fn open_view_tab(&self, view: &str) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let existing = inner.workspaces.iter().find_map(|w| {
            w.tabs
                .iter()
                .find(|t| t.view.as_deref() == Some(view))
                .map(|t| (w.id, t.id))
        });
        if let Some((ws_id, tab_id)) = existing {
            let idx = ws_index(&inner.workspaces, ws_id)?;
            inner.workspaces[idx].active_tab = Some(tab_id);
            inner.active = Some(ws_id);
            drop(inner);
            return self.persist();
        }
        // Uma view (ex.: Configurações) é uma página, não uma sessão: vive
        // no seu próprio workspace dedicado (aparece na sidebar, sem tab),
        // nunca como aba de um workspace de terminal.
        let tab = Tab::from_view(view);
        let tab_id = tab.id;
        let workspace = Workspace {
            id: Uuid::new_v4(),
            name: FALLBACK_WORKSPACE_NAME.to_string(),
            name_locked: true,
            repo_root: None,
            color: None,
            group: None,
            kind: WorkspaceKind::User,
            launch_config_id: None,
            active_tab: Some(tab_id),
            tabs: vec![tab],
            side_view: None,
            side_ratio: DEFAULT_SIDE_RATIO,
            side_expanded: false,
            history: Vec::new(),
            screens: Default::default(),
            created_at: Utc::now(),
        };
        let ws_id = workspace.id;
        inner.workspaces.push(workspace);
        inner.active = Some(ws_id);
        drop(inner);
        self.persist()
    }

    pub fn close_workspace(&self, id: WorkspaceId) -> Result<Vec<SessionId>, LayoutError> {
        let mut inner = self.inner.write();
        let idx = ws_index(&inner.workspaces, id)?;
        let removed = inner.workspaces.remove(idx);
        if inner.active == Some(id) {
            inner.active = inner
                .workspaces
                .get(idx.min(inner.workspaces.len().saturating_sub(1)))
                .map(|w| w.id)
                .filter(|_| !inner.workspaces.is_empty());
        }
        drop(inner);
        self.persist()?;
        Ok(removed.bound_sessions())
    }

    pub fn workspace_of_launch_config(&self, config: Uuid) -> Option<WorkspaceId> {
        self.inner
            .read()
            .workspaces
            .iter()
            .find(|w| w.launch_config_id == Some(config))
            .map(|w| w.id)
    }

    pub fn insert_workspace(&self, workspace: Workspace) -> Result<WorkspaceId, LayoutError> {
        let id = workspace.id;
        let mut inner = self.inner.write();
        inner.workspaces.push(workspace);
        inner.active = Some(id);
        drop(inner);
        self.persist()?;
        Ok(id)
    }

    pub fn activate_workspace(&self, id: WorkspaceId) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        ws_index(&inner.workspaces, id)?;
        inner.active = Some(id);
        drop(inner);
        self.persist()
    }

    pub fn rename_workspace(&self, id: WorkspaceId, name: &str) -> Result<(), LayoutError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let mut inner = self.inner.write();
        let idx = ws_index(&inner.workspaces, id)?;
        inner.workspaces[idx].name = trimmed.to_string();
        inner.workspaces[idx].name_locked = true;
        drop(inner);
        self.persist()
    }

    pub fn set_workspace_color(
        &self,
        id: WorkspaceId,
        color: Option<String>,
    ) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let idx = ws_index(&inner.workspaces, id)?;
        inner.workspaces[idx].color = color.filter(|c| !c.trim().is_empty());
        drop(inner);
        self.persist()
    }

    pub fn set_workspace_group(
        &self,
        id: WorkspaceId,
        group: Option<String>,
    ) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let idx = ws_index(&inner.workspaces, id)?;
        inner.workspaces[idx].group = group
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty());
        drop(inner);
        self.persist()
    }

    pub fn create_tab(
        &self,
        session: SessionId,
        workspace: Option<WorkspaceId>,
    ) -> Result<TabId, LayoutError> {
        let mut inner = self.inner.write();
        if find_session_pane(&inner.workspaces, session).is_some() {
            return Err(LayoutError::SessionAlreadyBound(session));
        }
        let target = workspace
            .or(inner.active)
            .ok_or(LayoutError::NoActiveWorkspace)?;
        let idx = ws_index(&inner.workspaces, target)?;
        let tab = Tab::from_session(session);
        let tab_id = tab.id;
        inner.workspaces[idx].tabs.push(tab);
        inner.workspaces[idx].set_active_tab(tab_id);
        inner.active = Some(target);
        drop_stale_diff_views(&mut inner);
        drop(inner);
        self.persist()?;
        Ok(tab_id)
    }

    pub fn close_tab(&self, tab: TabId) -> Result<Vec<SessionId>, LayoutError> {
        let mut inner = self.inner.write();
        let (w_idx, t_idx) = tab_index(&inner.workspaces, tab)?;
        let removed = inner.workspaces[w_idx].remove_tab_at(t_idx);
        remove_workspace_if_empty(&mut inner, w_idx);
        drop_stale_diff_views(&mut inner);
        drop(inner);
        self.persist()?;
        let mut bound = Vec::new();
        if let Some(root) = &removed.root {
            root.leaf_sessions(&mut bound);
        }
        Ok(bound)
    }

    pub fn activate_tab(&self, tab: TabId) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let (w_idx, _) = tab_index(&inner.workspaces, tab)?;
        let ws_id = inner.workspaces[w_idx].id;
        inner.workspaces[w_idx].set_active_tab(tab);
        inner.active = Some(ws_id);
        drop_stale_diff_views(&mut inner);
        drop(inner);
        self.persist()
    }

    pub fn move_tab(&self, tab: TabId, to: usize) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let (w_idx, t_idx) = tab_index(&inner.workspaces, tab)?;
        let ws = &mut inner.workspaces[w_idx];
        let moved = ws.tabs.remove(t_idx);
        let to = to.min(ws.tabs.len());
        ws.tabs.insert(to, moved);
        drop(inner);
        self.persist()
    }

    pub fn workspace_of_session(&self, session: SessionId) -> Option<WorkspaceId> {
        let inner = self.inner.read();
        find_session_pane(&inner.workspaces, session).map(|(ws, _, _)| ws)
    }

    pub fn pane_of_session(&self, session: SessionId) -> Option<PaneId> {
        let inner = self.inner.read();
        find_session_pane(&inner.workspaces, session).map(|(_, _, pane)| pane)
    }

    pub fn open_session(&self, session: SessionId) -> Result<(), LayoutError> {
        let existing = {
            let inner = self.inner.read();
            find_session_pane(&inner.workspaces, session)
        };
        match existing {
            Some((ws_id, tab_id, pane_id)) => {
                let mut inner = self.inner.write();
                inner.active = Some(ws_id);
                if let Ok(idx) = ws_index(&inner.workspaces, ws_id) {
                    inner.workspaces[idx].set_active_tab(tab_id);
                    if let Some(tab) = inner.workspaces[idx]
                        .tabs
                        .iter_mut()
                        .find(|t| t.id == tab_id)
                    {
                        tab.active_pane = Some(pane_id);
                    }
                }
                drop_stale_diff_views(&mut inner);
                drop(inner);
                self.persist()
            }
            None => {
                self.create_tab(session, None)?;
                Ok(())
            }
        }
    }

    pub fn split_pane(
        &self,
        pane: PaneId,
        kind: SplitKind,
        session: SessionId,
    ) -> Result<PaneId, LayoutError> {
        let mut inner = self.inner.write();
        if find_session_pane(&inner.workspaces, session).is_some() {
            return Err(LayoutError::SessionAlreadyBound(session));
        }
        for ws in inner.workspaces.iter_mut() {
            let ws_id = ws.id;
            for tab in ws.tabs.iter_mut() {
                let Some(root) = tab.root.as_mut() else {
                    continue;
                };
                if root.contains(pane) {
                    let new_pane = root
                        .split_leaf(pane, kind, session)
                        .ok_or(LayoutError::PaneNotFound(pane))?;
                    tab.active_pane = Some(new_pane);
                    let tab_id = tab.id;
                    ws.active_tab = Some(tab_id);
                    inner.active = Some(ws_id);
                    drop_stale_diff_views(&mut inner);
                    drop(inner);
                    self.persist()?;
                    return Ok(new_pane);
                }
            }
        }
        Err(LayoutError::PaneNotFound(pane))
    }

    pub fn close_pane(&self, pane: PaneId) -> Result<Vec<SessionId>, LayoutError> {
        let unbound = {
            let inner = self.inner.read();
            inner
                .workspaces
                .iter()
                .flat_map(|w| w.tabs.iter())
                .find_map(|t| t.root.as_ref().and_then(|r| r.leaf_session(pane)))
        };
        let Some(session) = unbound else {
            return Err(LayoutError::PaneNotFound(pane));
        };

        let mut inner = self.inner.write();
        let mut target: Option<(usize, usize)> = None;
        'outer: for (wi, ws) in inner.workspaces.iter().enumerate() {
            for (ti, tab) in ws.tabs.iter().enumerate() {
                if tab.root.as_ref().is_some_and(|r| r.contains(pane)) {
                    target = Some((wi, ti));
                    break 'outer;
                }
            }
        }
        let (wi, ti) = target.ok_or(LayoutError::PaneNotFound(pane))?;
        let tab = inner.workspaces[wi].tabs[ti].clone();
        let tab_root = tab.root.clone().ok_or(LayoutError::PaneNotFound(pane))?;
        match tab_root.remove_leaf(pane) {
            Some(root) => {
                let active_pane = match tab.active_pane {
                    Some(current) if root.contains(current) => Some(current),
                    _ => Some(root.first_leaf()),
                };
                inner.workspaces[wi].tabs[ti] = Tab {
                    root: Some(root),
                    active_pane,
                    ..tab
                };
            }
            None => {
                inner.workspaces[wi].remove_tab_at(ti);
                remove_workspace_if_empty(&mut inner, wi);
            }
        }
        drop_stale_diff_views(&mut inner);
        drop(inner);
        self.persist()?;
        Ok(vec![session])
    }

    pub fn focus_pane(&self, pane: PaneId) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let found = inner.workspaces.iter().enumerate().find_map(|(wi, ws)| {
            ws.tabs
                .iter()
                .position(|t| t.root.as_ref().is_some_and(|r| r.contains(pane)))
                .map(|ti| (wi, ti))
        });
        let Some((wi, ti)) = found else {
            return Err(LayoutError::PaneNotFound(pane));
        };
        let ws = &mut inner.workspaces[wi];
        ws.tabs[ti].active_pane = Some(pane);
        let tab_id = ws.tabs[ti].id;
        ws.set_active_tab(tab_id);
        let ws_id = ws.id;
        inner.active = Some(ws_id);
        drop_stale_diff_views(&mut inner);
        drop(inner);
        self.persist()
    }

    pub fn set_split_ratio(
        &self,
        pane: PaneId,
        ratio: f64,
        commit: bool,
    ) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let found = inner
            .workspaces
            .iter_mut()
            .flat_map(|w| w.tabs.iter_mut())
            .any(|t| t.root.as_mut().is_some_and(|r| r.set_ratio(pane, ratio)));
        if !found {
            return Err(LayoutError::NotASplit(pane));
        }
        drop(inner);
        if commit {
            self.persist()?;
        }
        Ok(())
    }

    /// Abre (ou foca) o painel lateral do workspace que contém a sessão.
    /// O painel vive AO LADO da coluna de tabs (altura total), nunca dentro
    /// de um tab — fechar o painel não toca nas sessões.
    pub fn open_workspace_side_view(
        &self,
        session: SessionId,
        view: &str,
    ) -> Result<WorkspaceId, LayoutError> {
        let mut inner = self.inner.write();
        let Some((ws_id, tab_id, pane_id)) = find_session_pane(&inner.workspaces, session) else {
            return Err(LayoutError::NoActiveWorkspace);
        };
        let idx = ws_index(&inner.workspaces, ws_id)?;
        inner.workspaces[idx].side_view = Some(view.to_string());
        // Aba de editor da mesma sessão em foco: o painel abre ao lado dela,
        // sem arrancar o usuário do arquivo para o terminal.
        let editing_same = inner.workspaces[idx].focused_session() == Some(session)
            && inner.workspaces[idx]
                .tabs
                .iter()
                .any(|t| Some(t.id) == inner.workspaces[idx].active_tab && t.view.is_some());
        if !editing_same {
            inner.workspaces[idx].set_active_tab(tab_id);
            if let Some(tab) = inner.workspaces[idx]
                .tabs
                .iter_mut()
                .find(|t| t.id == tab_id)
            {
                tab.active_pane = Some(pane_id);
            }
        }
        inner.active = Some(ws_id);
        drop(inner);
        self.persist()?;
        Ok(ws_id)
    }

    /// Abre (ou fecha) a fila de agentes no workspace ativo.
    ///
    /// Alterna em vez de só abrir: o atalho que a chama é o mesmo que o usuário
    /// aperta para "ver quem me espera", e apertar de novo tem de devolver a
    /// tela — senão o painel vira uma coisa que só se fecha com o mouse.
    pub fn toggle_agent_queue(&self) -> Result<WorkspaceId, LayoutError> {
        let mut inner = self.inner.write();
        let ws_id = inner.active.ok_or(LayoutError::NoActiveWorkspace)?;
        let idx = ws_index(&inner.workspaces, ws_id)?;
        let ja_aberta = inner.workspaces[idx].side_view.as_deref() == Some(VIEW_AGENT_QUEUE);
        if ja_aberta {
            inner.workspaces[idx].side_view = None;
            inner.workspaces[idx].side_expanded = false;
        } else {
            inner.workspaces[idx].side_view = Some(VIEW_AGENT_QUEUE.to_string());
        }
        drop(inner);
        self.persist()?;
        Ok(ws_id)
    }

    pub fn close_side_view(&self, workspace: WorkspaceId) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let idx = ws_index(&inner.workspaces, workspace)?;
        inner.workspaces[idx].side_view = None;
        inner.workspaces[idx].side_expanded = false;
        drop(inner);
        self.persist()
    }

    pub fn set_side_view_expanded(
        &self,
        workspace: WorkspaceId,
        expanded: bool,
    ) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let idx = ws_index(&inner.workspaces, workspace)?;
        inner.workspaces[idx].side_expanded = expanded;
        drop(inner);
        self.persist()
    }

    pub fn set_side_view_ratio(
        &self,
        workspace: WorkspaceId,
        ratio: f64,
        commit: bool,
    ) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let idx = ws_index(&inner.workspaces, workspace)?;
        inner.workspaces[idx].side_ratio = ratio.clamp(MIN_RATIO, MAX_RATIO);
        drop(inner);
        if commit {
            self.persist()?;
        }
        Ok(())
    }

    pub fn session_disposed(&self, session: SessionId) -> Result<(), LayoutError> {
        loop {
            let target = {
                let inner = self.inner.read();
                find_session_any_pane(&inner.workspaces, session)
            };
            match target {
                Some(pane) => {
                    self.close_pane(pane)?;
                }
                None => break,
            }
        }
        // Todo painel de sessão (diff, arquivos, agentes, túneis, git, docker)
        // referencia a sessão; sem ela vira casca órfã.
        let mut inner = self.inner.write();
        let mut changed = false;
        for ws in inner.workspaces.iter_mut() {
            if ws.side_view.as_deref().and_then(side_view_session) == Some(session) {
                ws.side_view = None;
                ws.side_expanded = false;
                changed = true;
            }
        }
        // Abas de editor da sessão: a raiz de arquivos e o LSP eram dela.
        for ws in inner.workspaces.iter_mut() {
            ws.screens
                .files
                .retain(|view| screen_file_session(view) != Some(session));
        }
        let mut wi = 0;
        while wi < inner.workspaces.len() {
            while let Some(ti) = inner.workspaces[wi]
                .tabs
                .iter()
                .position(|t| t.view.as_deref().and_then(content_view_session) == Some(session))
            {
                inner.workspaces[wi].remove_tab_at(ti);
                changed = true;
            }
            let before = inner.workspaces.len();
            remove_workspace_if_empty(&mut inner, wi);
            if inner.workspaces.len() == before {
                wi += 1;
            }
        }
        drop(inner);
        if changed {
            self.persist()?;
        }
        Ok(())
    }

    /// Abre (ou foca, se já existe no workspace) uma aba de conteúdo — editor
    /// de arquivo, logs de container. Vive ao lado das abas de terminal do
    /// workspace, não num workspace dedicado como as páginas (Configurações).
    pub fn open_content_tab(
        &self,
        workspace: Option<WorkspaceId>,
        view: &str,
    ) -> Result<TabId, LayoutError> {
        let mut inner = self.inner.write();
        let target = workspace
            .or(inner.active)
            .ok_or(LayoutError::NoActiveWorkspace)?;
        let idx = ws_index(&inner.workspaces, target)?;
        let existing = inner.workspaces[idx]
            .tabs
            .iter()
            .find(|t| t.view.as_deref() == Some(view))
            .map(|t| t.id);
        let tab_id = match existing {
            Some(id) => id,
            None => {
                let tab = Tab::from_view(view);
                let id = tab.id;
                // Ao lado da aba de onde veio, como um link aberto no navegador.
                let ws = &mut inner.workspaces[idx];
                let at = ws
                    .active_tab
                    .and_then(|a| ws.tabs.iter().position(|t| t.id == a))
                    .map(|p| p + 1)
                    .unwrap_or(ws.tabs.len());
                ws.tabs.insert(at, tab);
                id
            }
        };
        inner.workspaces[idx].set_active_tab(tab_id);
        inner.active = Some(target);
        drop_stale_diff_views(&mut inner);
        drop(inner);
        self.persist()?;
        Ok(tab_id)
    }

    /// ⌘[ — volta para a aba usada antes da atual no workspace ativo. `false`
    /// quando não há para onde voltar.
    pub fn back_tab(&self) -> Result<bool, LayoutError> {
        let mut inner = self.inner.write();
        let ws_id = inner.active.ok_or(LayoutError::NoActiveWorkspace)?;
        let idx = ws_index(&inner.workspaces, ws_id)?;
        let ws = &mut inner.workspaces[idx];
        let alive: Vec<TabId> = ws.tabs.iter().map(|t| t.id).collect();
        let Some(target) =
            history::pop_back(&mut ws.history, ws.active_tab, |t| alive.contains(&t))
        else {
            return Ok(false);
        };
        // Sem registrar a atual: voltar duas vezes anda duas abas para trás,
        // como no navegador, em vez de alternar entre as duas últimas.
        ws.active_tab = Some(target);
        drop_stale_diff_views(&mut inner);
        drop(inner);
        self.persist()?;
        Ok(true)
    }

    /// Sessão em foco no workspace ativo (pane ativo ou dona da aba de editor).
    pub fn active_session(&self) -> Option<SessionId> {
        self.active_sessions().first().copied()
    }

    /// Sessões do workspace ativo em ordem de preferência para resolver um
    /// arquivo: a em foco, as das abas usadas por último, depois as demais.
    /// Com a aba de logs em foco, "a sessão" é a do terminal de onde se veio.
    pub fn active_sessions(&self) -> Vec<SessionId> {
        let inner = self.inner.read();
        let Some(ws) = inner
            .active
            .and_then(|id| inner.workspaces.iter().find(|w| w.id == id))
        else {
            return Vec::new();
        };
        let tab_session = |tab: &Tab| match tab.view.as_deref() {
            Some(view) => content_view_session(view),
            None => tab
                .root
                .as_ref()
                .and_then(|r| tab.active_pane.and_then(|p| r.leaf_session(p))),
        };
        let mut out: Vec<SessionId> = ws.focused_session().into_iter().collect();
        let recent = ws
            .history
            .iter()
            .rev()
            .filter_map(|id| ws.tabs.iter().find(|t| t.id == *id))
            .filter_map(tab_session);
        for s in recent.chain(ws.bound_sessions()) {
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }

    /// Há aba de editor aberta para a sessão em algum workspace? O painel de
    /// arquivos da sessão (watcher, LSP) não pode morrer enquanto houver.
    pub fn has_editor_tabs(&self, session: SessionId) -> bool {
        let of_session = |view: &str| editor_view_session(view) == Some(session);
        self.inner.read().workspaces.iter().any(|w| {
            w.screens.files.has(of_session)
                || w.tabs
                    .iter()
                    .any(|t| t.view.as_deref().is_some_and(of_session))
        })
    }

    /// Abre a tela de Arquivos da sessão com o arquivo `view` (`editor:…`) na
    /// aba interna. Devolve o workspace dela.
    pub fn open_file_screen(
        &self,
        session: SessionId,
        view: &str,
        preview: bool,
    ) -> Result<WorkspaceId, LayoutError> {
        let ws = self.open_workspace_side_view(session, &files_view(session))?;
        self.with_screens(ws, |s| s.files.open(view, preview))?;
        Ok(ws)
    }

    /// Muda o estado das telas cheias do workspace. Grava a tela de Arquivos
    /// (a de Docker não persiste, ver [`screens`]).
    pub fn with_screens<R>(
        &self,
        workspace: WorkspaceId,
        f: impl FnOnce(&mut screens::Screens) -> R,
    ) -> Result<R, LayoutError> {
        let mut inner = self.inner.write();
        let idx = ws_index(&inner.workspaces, workspace)?;
        let out = f(&mut inner.workspaces[idx].screens);
        drop(inner);
        self.persist_file_screens()?;
        Ok(out)
    }

    /// Painel lateral sem sessão dona (ex.: arquivos de um container) no
    /// workspace ativo.
    pub fn open_active_side_view(&self, view: &str) -> Result<WorkspaceId, LayoutError> {
        let mut inner = self.inner.write();
        let ws_id = inner.active.ok_or(LayoutError::NoActiveWorkspace)?;
        let idx = ws_index(&inner.workspaces, ws_id)?;
        inner.workspaces[idx].side_view = Some(view.to_string());
        drop(inner);
        self.persist()?;
        Ok(ws_id)
    }

    pub fn agent_viewer_session(&self, pane: PaneId) -> Option<SessionId> {
        let inner = self.inner.read();
        inner
            .workspaces
            .iter()
            .flat_map(|w| w.tabs.iter())
            .find_map(|t| t.root.as_ref().and_then(|r| r.agent_viewer_session(pane)))
    }

    /// Panes de viewer de subagente da sessão, em qualquer workspace/tab —
    /// viewer e painel Agentes são UMA feature: quem fecha a rodada fecha os
    /// dois.
    pub fn agent_viewer_panes(&self, session: SessionId) -> Vec<PaneId> {
        let inner = self.inner.read();
        let mut out = Vec::new();
        for ws in &inner.workspaces {
            for tab in &ws.tabs {
                if let Some(root) = &tab.root {
                    root.collect_agent_viewer_panes(session, &mut out);
                }
            }
        }
        out
    }

    pub fn ensure_agent_viewer(&self, session: SessionId) -> bool {
        let mut inner = self.inner.write();
        if has_agent_viewer(&inner.workspaces, session) {
            return false;
        }
        let Some(active_ws) = inner.active else {
            return false;
        };
        let Some((wi, ti, leaf)) =
            find_session_leaf_in_active_tab_of(&inner.workspaces, active_ws, session)
        else {
            return false;
        };
        let Some(root) = inner.workspaces[wi].tabs[ti].root.as_mut() else {
            return false;
        };
        if split_leaf_with_viewer(root, leaf, session, AGENT_VIEWER_RATIO).is_none() {
            return false;
        }
        drop(inner);
        let _ = self.persist();
        true
    }

    /// Garante o painel Agentes da sessão aberto no workspace ativo. Devolve
    /// `true` quando, depois da chamada, o painel mostra esta sessão (aberto
    /// agora ou já aberto) — é o que diz ao chamador que a rodada foi
    /// coordenada.
    ///
    /// A fila de agentes cede o lugar: é a mesma família ("Agentes" no menu
    /// lateral) e o dono que a abriu queria ver os agentes. Antes ela segurava
    /// o slot e a rodada nova nascia sem painel (A5.3, tela 4). Qualquer outra
    /// side view (diff, arquivos, túneis) é escolha do usuário e não é tomada.
    pub fn ensure_agents_panel(&self, session: SessionId) -> bool {
        let mut inner = self.inner.write();
        let Some(active_ws) = inner.active else {
            return false;
        };
        let Ok(idx) = ws_index(&inner.workspaces, active_ws) else {
            return false;
        };
        if !session_in_workspace(&inner.workspaces[idx], session) {
            return false;
        }
        let wanted = agents_view(session);
        match inner.workspaces[idx].side_view.as_deref() {
            Some(current) if current == wanted => return true,
            Some(VIEW_AGENT_QUEUE) | None => {}
            Some(_) => return false,
        }
        inner.workspaces[idx].side_view = Some(wanted);
        drop(inner);
        let _ = self.persist();
        true
    }

    pub fn open_agent_viewer(&self, session: SessionId) -> Result<bool, LayoutError> {
        let mut inner = self.inner.write();
        let active_ws = inner.active.ok_or(LayoutError::NoActiveWorkspace)?;
        let wi = ws_index(&inner.workspaces, active_ws)?;
        let found = inner.workspaces[wi]
            .tabs
            .iter()
            .enumerate()
            .find_map(|(ti, tab)| {
                tab.root
                    .as_ref()
                    .and_then(|r| r.find_leaf_by_session(session))
                    .map(|pane| (ti, tab.id, pane))
            });
        let Some((ti, tab_id, leaf)) = found else {
            return Err(LayoutError::SessionNotVisible(session));
        };
        if has_agent_viewer(&inner.workspaces, session) {
            inner.workspaces[wi].active_tab = Some(tab_id);
            drop(inner);
            self.persist()?;
            return Ok(false);
        }
        let Some(root) = inner.workspaces[wi].tabs[ti].root.as_mut() else {
            return Err(LayoutError::SessionNotVisible(session));
        };
        split_leaf_with_viewer(root, leaf, session, AGENT_VIEWER_RATIO)
            .ok_or(LayoutError::SessionNotVisible(session))?;
        inner.workspaces[wi].active_tab = Some(tab_id);
        inner.active = Some(active_ws);
        drop(inner);
        self.persist()?;
        Ok(true)
    }

    fn persist(&self) -> Result<(), LayoutError> {
        let state = self.state();
        let rows = workspaces_to_rows(&state.workspaces);
        self.store.save_layout(&rows)?;
        self.store.set_setting(
            KEY_ACTIVE_WORKSPACE,
            &state
                .active_workspace
                .map(|id| id.to_string())
                .unwrap_or_default(),
        )?;
        self.persist_file_screens()
    }

    fn persist_file_screens(&self) -> Result<(), LayoutError> {
        let json = {
            let inner = self.inner.read();
            let saved: std::collections::HashMap<String, &screens::FilesScreen> = inner
                .workspaces
                .iter()
                .map(|w| (w.id.to_string(), &w.screens.files))
                .collect();
            serde_json::to_string(&saved).unwrap_or_default()
        };
        self.store.set_setting(KEY_FILE_SCREENS, &json)?;
        Ok(())
    }
}

/// `editor:<antiga>:<rel>` → `editor:<nova>:<rel>` quando a sessão reabriu.
fn remap_editor_view(
    view: &str,
    remap: &std::collections::HashMap<SessionId, SessionId>,
) -> String {
    match editor_view_session(view).and_then(|old| remap.get(&old).map(|new| (old, new))) {
        Some((old, new)) => view.replacen(&old.to_string(), &new.to_string(), 1),
        None => view.to_string(),
    }
}

fn remove_workspace_if_empty(inner: &mut Inner, w_idx: usize) {
    if !inner.workspaces[w_idx].tabs.is_empty() {
        return;
    }
    let id = inner.workspaces[w_idx].id;
    inner.workspaces.remove(w_idx);
    if inner.active == Some(id) {
        inner.active = inner
            .workspaces
            .get(w_idx.min(inner.workspaces.len().saturating_sub(1)))
            .map(|w| w.id)
            .filter(|_| !inner.workspaces.is_empty());
    }
}

fn ws_index(workspaces: &[Workspace], id: WorkspaceId) -> Result<usize, LayoutError> {
    workspaces
        .iter()
        .position(|w| w.id == id)
        .ok_or(LayoutError::WorkspaceNotFound(id))
}

fn tab_index(workspaces: &[Workspace], tab: TabId) -> Result<(usize, usize), LayoutError> {
    for (wi, ws) in workspaces.iter().enumerate() {
        if let Some(ti) = ws.tabs.iter().position(|t| t.id == tab) {
            return Ok((wi, ti));
        }
    }
    Err(LayoutError::TabNotFound(tab))
}

fn find_session_pane(
    workspaces: &[Workspace],
    session: SessionId,
) -> Option<(WorkspaceId, TabId, PaneId)> {
    workspaces.iter().find_map(|w| {
        w.tabs.iter().find_map(|t| {
            t.root
                .as_ref()
                .and_then(|r| r.find_leaf_by_session(session))
                .map(|pane| (w.id, t.id, pane))
        })
    })
}

fn find_session_any_pane(workspaces: &[Workspace], session: SessionId) -> Option<PaneId> {
    workspaces.iter().find_map(|w| {
        w.tabs.iter().find_map(|t| {
            t.root
                .as_ref()
                .and_then(|r| r.find_pane_of_session(session))
        })
    })
}

fn has_agent_viewer(workspaces: &[Workspace], session: SessionId) -> bool {
    workspaces.iter().any(|w| {
        w.tabs.iter().any(|t| {
            t.root
                .as_ref()
                .is_some_and(|r| r.contains_agent_viewer(session))
        })
    })
}

fn find_session_leaf_in_active_tab_of(
    workspaces: &[Workspace],
    workspace: WorkspaceId,
    session: SessionId,
) -> Option<(usize, usize, PaneId)> {
    let wi = workspaces.iter().position(|w| w.id == workspace)?;
    let ws = &workspaces[wi];
    let active_tab = ws.active_tab?;
    let (ti, tab) = ws
        .tabs
        .iter()
        .enumerate()
        .find(|(_, t)| t.id == active_tab)?;
    let pane = tab
        .root
        .as_ref()
        .and_then(|r| r.find_leaf_by_session(session))?;
    Some((wi, ti, pane))
}

fn session_in_workspace(ws: &Workspace, session: SessionId) -> bool {
    ws.tabs.iter().any(|t| {
        t.root
            .as_ref()
            .is_some_and(|r| r.find_leaf_by_session(session).is_some())
    })
}

fn split_leaf_with_viewer(
    node: &mut PaneNode,
    target: PaneId,
    session: SessionId,
    ratio: f64,
) -> Option<PaneId> {
    match node {
        PaneNode::Leaf { id, .. } if *id == target => {
            let viewer = PaneNode::AgentViewer {
                id: Uuid::new_v4(),
                session_id: session,
            };
            let viewer_id = viewer.id();
            let old = std::mem::replace(
                node,
                PaneNode::Leaf {
                    id: Uuid::new_v4(),
                    session_id: session,
                },
            );
            *node = PaneNode::Split {
                id: Uuid::new_v4(),
                split: SplitKind::V,
                ratio,
                first: Box::new(old),
                second: Box::new(viewer),
            };
            Some(viewer_id)
        }
        PaneNode::Leaf { .. } | PaneNode::AgentViewer { .. } => None,
        PaneNode::Split { first, second, .. } => {
            split_leaf_with_viewer(first, target, session, ratio)
                .or_else(|| split_leaf_with_viewer(second, target, session, ratio))
        }
    }
}

/// O workspace Docker: onde cai a aba de container sem workspace ativo. Nasce
/// vazio — quem pede cria a aba em seguida — e some quando a última fecha.
fn ensure_docker_workspace(inner: &mut Inner) -> WorkspaceId {
    if let Some(ws) = inner
        .workspaces
        .iter()
        .find(|w| w.kind == WorkspaceKind::Docker)
    {
        return ws.id;
    }
    let workspace = Workspace {
        id: Uuid::new_v4(),
        name: DOCKER_WORKSPACE_NAME.to_string(),
        name_locked: true,
        repo_root: None,
        color: None,
        group: None,
        kind: WorkspaceKind::Docker,
        launch_config_id: None,
        active_tab: None,
        tabs: Vec::new(),
        side_view: None,
        side_ratio: DEFAULT_SIDE_RATIO,
        side_expanded: false,
        history: Vec::new(),
        screens: Default::default(),
        created_at: Utc::now(),
    };
    let id = workspace.id;
    inner.workspaces.push(workspace);
    id
}

fn push_pane_rows(
    node: &PaneNode,
    tab_id: &str,
    parent: Option<String>,
    position: Option<i64>,
    out: &mut Vec<PaneRow>,
) {
    match node {
        PaneNode::Leaf { id, session_id } => out.push(PaneRow {
            id: id.to_string(),
            tab_id: tab_id.to_string(),
            parent_id: parent,
            split: None,
            ratio: None,
            position,
            session_id: Some(session_id.to_string()),
        }),
        PaneNode::AgentViewer { id, session_id } => out.push(PaneRow {
            id: id.to_string(),
            tab_id: tab_id.to_string(),
            parent_id: parent,
            split: Some(AGENT_VIEWER_ROW.to_string()),
            ratio: None,
            position,
            session_id: Some(session_id.to_string()),
        }),
        PaneNode::Split {
            id,
            split,
            ratio,
            first,
            second,
        } => {
            let id_str = id.to_string();
            out.push(PaneRow {
                id: id_str.clone(),
                tab_id: tab_id.to_string(),
                parent_id: parent,
                split: Some(split.as_str().to_string()),
                ratio: Some(*ratio),
                position,
                session_id: None,
            });
            push_pane_rows(first, tab_id, Some(id_str.clone()), Some(0), out);
            push_pane_rows(second, tab_id, Some(id_str), Some(1), out);
        }
    }
}

pub fn pane_rows(root: &PaneNode, tab_id: &str) -> Vec<PaneRow> {
    let mut out = Vec::new();
    push_pane_rows(root, tab_id, None, None, &mut out);
    out
}

pub fn workspaces_to_rows(workspaces: &[Workspace]) -> LayoutRows {
    let mut rows = LayoutRows::default();
    for (wi, ws) in workspaces.iter().enumerate() {
        let ws_id = ws.id.to_string();
        rows.workspaces.push(WorkspaceRow {
            id: ws_id.clone(),
            name: ws.name.clone(),
            name_locked: Some(ws.name_locked as i64),
            repo_root: ws.repo_root.clone(),
            color: ws.color.clone(),
            group_name: ws.group.clone(),
            kind: Some(ws.kind.as_str().to_string()),
            launch_config_id: ws.launch_config_id.map(|id| id.to_string()),
            position: wi as i64,
            active_tab: ws.active_tab.map(|id| id.to_string()),
            side_view: ws.side_view.clone(),
            side_ratio: Some(ws.side_ratio),
            side_expanded: Some(ws.side_expanded as i64),
            created_at: ws.created_at.to_rfc3339(),
        });
        for (ti, tab) in ws.tabs.iter().enumerate() {
            let tab_id = tab.id.to_string();
            rows.tabs.push(TabRow {
                id: tab_id.clone(),
                workspace_id: Some(ws_id.clone()),
                title: tab.title.clone(),
                view: tab.view.clone(),
                position: ti as i64,
                active_pane: tab.active_pane.map(|id| id.to_string()),
                created_at: tab.created_at.to_rfc3339(),
            });
            if let Some(root) = &tab.root {
                push_pane_rows(root, &tab_id, None, None, &mut rows.panes);
            }
        }
    }
    rows
}

fn build_node(id: &str, panes: &[PaneRow]) -> Option<PaneNode> {
    let row = panes.iter().find(|p| p.id == id)?;
    let pane_id = Uuid::parse_str(&row.id).ok()?;
    match row.split.as_deref() {
        None => {
            let session = row.session_id.as_deref()?;
            Some(PaneNode::Leaf {
                id: pane_id,
                session_id: Uuid::parse_str(session).ok()?,
            })
        }
        Some(AGENT_VIEWER_ROW) => {
            let session = row.session_id.as_deref()?;
            Some(PaneNode::AgentViewer {
                id: pane_id,
                session_id: Uuid::parse_str(session).ok()?,
            })
        }
        Some(kind) => {
            let kind = SplitKind::parse(kind)?;
            let mut children: Vec<&PaneRow> = panes
                .iter()
                .filter(|p| p.parent_id.as_deref() == Some(id))
                .collect();
            children.sort_by_key(|p| p.position.unwrap_or(0));
            if children.len() != 2 {
                return None;
            }
            let first = build_node(&children[0].id, panes)?;
            let second = build_node(&children[1].id, panes)?;
            Some(PaneNode::Split {
                id: pane_id,
                split: kind,
                ratio: row.ratio.unwrap_or(0.5).clamp(MIN_RATIO, MAX_RATIO),
                first: Box::new(first),
                second: Box::new(second),
            })
        }
    }
}

fn build_tab(row: &TabRow, panes: &[PaneRow], valid: &HashSet<SessionId>) -> Option<Tab> {
    let tab_id = Uuid::parse_str(&row.id).ok()?;
    let created_at = DateTime::parse_from_rfc3339(&row.created_at)
        .ok()?
        .with_timezone(&Utc);
    if let Some(view) = &row.view {
        // Tabs de diff eram a forma antiga (pré-painel lateral); ao carregar,
        // somem — o diff hoje é side_view do workspace.
        if view.starts_with(VIEW_DIFF_PREFIX) {
            return None;
        }
        // O painel antigo de containers virou a seção Compose da tela de
        // Docker (ver `load_remapped`).
        if view == RETIRED_VIEW_CONTAINERS {
            return None;
        }
        // Editor de sessão que não voltou: sem a raiz dela, o arquivo não abre.
        if content_view_session(view).is_some_and(|s| !valid.contains(&s)) {
            return None;
        }
        return Some(Tab {
            id: tab_id,
            title: row.title.clone(),
            view: Some(view.clone()),
            active_pane: None,
            root: None,
            created_at,
        });
    }
    let root_row = panes
        .iter()
        .find(|p| p.tab_id == row.id && p.parent_id.is_none())?;
    let root = build_node(&root_row.id, panes)?.retain_sessions(valid)?;
    let active_pane = row
        .active_pane
        .as_deref()
        .and_then(|s| Uuid::parse_str(s).ok())
        .filter(|id| root.contains(*id))
        .unwrap_or_else(|| root.first_leaf());
    Some(Tab {
        id: tab_id,
        title: row.title.clone(),
        view: None,
        active_pane: Some(active_pane),
        root: Some(root),
        created_at,
    })
}

/// O workspace ativo salvo estava com a aba aposentada de containers em foco?
fn on_retired_dashboard(rows: &LayoutRows, active: Option<WorkspaceId>) -> bool {
    let Some(active) = active.map(|id| id.to_string()) else {
        return false;
    };
    rows.workspaces
        .iter()
        .filter(|w| w.id == active)
        .filter_map(|w| w.active_tab.as_deref())
        .any(|tab| {
            rows.tabs
                .iter()
                .any(|t| t.id == tab && t.view.as_deref() == Some(RETIRED_VIEW_CONTAINERS))
        })
}

pub fn rows_to_workspaces(rows: &LayoutRows, valid: &HashSet<SessionId>) -> Vec<Workspace> {
    let mut ws_rows: Vec<&WorkspaceRow> = rows.workspaces.iter().collect();
    ws_rows.sort_by_key(|w| w.position);
    ws_rows
        .into_iter()
        .filter_map(|w| {
            let ws_id = Uuid::parse_str(&w.id).ok()?;
            let mut tab_rows: Vec<&TabRow> = rows
                .tabs
                .iter()
                .filter(|t| t.workspace_id.as_deref() == Some(w.id.as_str()))
                .collect();
            tab_rows.sort_by_key(|t| t.position);
            let (editor_tabs, tabs): (Vec<Tab>, Vec<Tab>) = tab_rows
                .into_iter()
                .filter_map(|t| build_tab(t, &rows.panes, valid))
                .partition(|t| t.view.as_deref().and_then(editor_view_session).is_some());
            // Aba de editor de layout antigo: o arquivo volta, mas dentro da
            // tela de Arquivos — a barra de cima é só de terminal e agente.
            let mut screens = screens::Screens::default();
            for tab in &editor_tabs {
                if let Some(view) = tab.view.as_deref() {
                    screens.files.open(view, false);
                }
            }
            let active_tab = w
                .active_tab
                .as_deref()
                .and_then(|s| Uuid::parse_str(s).ok())
                .filter(|id| tabs.iter().any(|t| t.id == *id))
                .or_else(|| tabs.first().map(|t| t.id));
            let kind = WorkspaceKind::parse(w.kind.as_deref());
            // Workspace Docker sem aba (era só o painel de containers): some.
            if kind == WorkspaceKind::Docker && tabs.is_empty() {
                return None;
            }
            let created_at = DateTime::parse_from_rfc3339(&w.created_at)
                .ok()?
                .with_timezone(&Utc);
            let side_view = w
                .side_view
                .clone()
                .filter(|view| side_view_session(view).is_none_or(|s| valid.contains(&s)));
            Some(Workspace {
                id: ws_id,
                name: w.name.clone(),
                name_locked: w.name_locked.unwrap_or(0) != 0,
                repo_root: w.repo_root.clone(),
                color: w.color.clone(),
                group: w.group_name.clone(),
                kind,
                launch_config_id: w
                    .launch_config_id
                    .as_deref()
                    .and_then(|s| Uuid::parse_str(s).ok()),
                active_tab,
                tabs,
                side_view,
                side_ratio: w
                    .side_ratio
                    .unwrap_or(DEFAULT_SIDE_RATIO)
                    .clamp(MIN_RATIO, MAX_RATIO),
                side_expanded: w.side_expanded.unwrap_or(0) != 0,
                history: Vec::new(),
                screens,
                created_at,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> LayoutManager {
        LayoutManager::new(Arc::new(Store::open_in_memory().unwrap()))
    }

    fn sid() -> SessionId {
        Uuid::new_v4()
    }

    fn ws(mgr: &LayoutManager) -> WorkspaceId {
        mgr.create_workspace("dev", None, sid()).unwrap()
    }

    #[test]
    fn create_workspace_starts_with_one_tab_and_activates() {
        let mgr = manager();
        let s = sid();
        let id = mgr
            .create_workspace("api", Some("/repo".into()), s)
            .unwrap();
        let state = mgr.state();
        assert_eq!(state.active_workspace, Some(id));
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].name, "api");
        assert_eq!(state.workspaces[0].tabs.len(), 1);
        assert!(matches!(
            state.workspaces[0].tabs[0].root,
            Some(PaneNode::Leaf { session_id, .. }) if session_id == s
        ));
    }

    #[test]
    fn create_tab_defaults_to_active_workspace() {
        let mgr = manager();
        let w1 = ws(&mgr);
        let w2 = ws(&mgr);
        let tab = mgr.create_tab(sid(), None).unwrap();
        let state = mgr.state();
        let target = state.workspaces.iter().find(|w| w.id == w2).unwrap();
        assert_eq!(target.tabs.len(), 2);
        assert_eq!(target.active_tab, Some(tab));
        let other = state.workspaces.iter().find(|w| w.id == w1).unwrap();
        assert_eq!(other.tabs.len(), 1);
    }

    #[test]
    fn create_tab_without_workspace_fails_when_none_active() {
        let mgr = manager();
        assert!(matches!(
            mgr.create_tab(sid(), None).unwrap_err(),
            LayoutError::NoActiveWorkspace
        ));
    }

    #[test]
    fn close_tab_returns_bound_sessions_and_keeps_workspace() {
        let mgr = manager();
        ws(&mgr);
        let s2 = sid();
        let tab = mgr.create_tab(s2, None).unwrap();
        let pane = mgr.state().workspaces[0]
            .tabs
            .iter()
            .find(|t| t.id == tab)
            .unwrap()
            .active_pane
            .unwrap();
        let s3 = sid();
        mgr.split_pane(pane, SplitKind::V, s3).unwrap();

        let bound = mgr.close_tab(tab).unwrap();
        assert_eq!(bound.len(), 2);
        assert!(bound.contains(&s2) && bound.contains(&s3));
        let state = mgr.state();
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].tabs.len(), 1);
    }

    #[test]
    fn closing_last_tab_closes_workspace() {
        let mgr = manager();
        ws(&mgr);
        let tab = mgr.state().workspaces[0].tabs[0].id;
        mgr.close_tab(tab).unwrap();
        let state = mgr.state();
        assert!(state.workspaces.is_empty());
        assert_eq!(state.active_workspace, None);
    }

    #[test]
    fn closing_last_tab_activates_neighbor_workspace() {
        let mgr = manager();
        let w1 = ws(&mgr);
        let w2 = ws(&mgr);
        let tab = mgr
            .state()
            .workspaces
            .iter()
            .find(|w| w.id == w2)
            .unwrap()
            .tabs[0]
            .id;
        mgr.close_tab(tab).unwrap();
        let state = mgr.state();
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.active_workspace, Some(w1));
    }

    #[test]
    fn closing_last_pane_of_last_tab_closes_workspace() {
        let mgr = manager();
        ws(&mgr);
        let pane = mgr.state().workspaces[0].tabs[0].active_pane.unwrap();
        mgr.close_pane(pane).unwrap();
        let state = mgr.state();
        assert!(state.workspaces.is_empty());
        assert_eq!(state.active_workspace, None);
    }

    #[test]
    fn close_workspace_returns_all_bound_sessions() {
        let mgr = manager();
        let s1 = sid();
        let id = mgr.create_workspace("x", None, s1).unwrap();
        let s2 = sid();
        mgr.create_tab(s2, Some(id)).unwrap();

        let bound = mgr.close_workspace(id).unwrap();
        assert_eq!(bound.len(), 2);
        assert!(mgr.state().workspaces.is_empty());
        assert_eq!(mgr.state().active_workspace, None);
    }

    #[test]
    fn activate_tab_also_activates_its_workspace() {
        let mgr = manager();
        let w1 = ws(&mgr);
        let w1_tab = mgr.state().workspaces[0].tabs[0].id;
        ws(&mgr);
        mgr.activate_tab(w1_tab).unwrap();
        assert_eq!(mgr.state().active_workspace, Some(w1));
    }

    #[test]
    fn open_session_focuses_existing_binding_across_workspaces() {
        let mgr = manager();
        let s1 = sid();
        let w1 = mgr.create_workspace("a", None, s1).unwrap();
        ws(&mgr);
        mgr.open_session(s1).unwrap();
        let state = mgr.state();
        assert_eq!(state.active_workspace, Some(w1));
    }

    #[test]
    fn open_session_unbound_creates_tab_in_active_workspace() {
        let mgr = manager();
        let id = ws(&mgr);
        let s = sid();
        mgr.open_session(s).unwrap();
        let state = mgr.state();
        let target = state.workspaces.iter().find(|w| w.id == id).unwrap();
        assert_eq!(target.tabs.len(), 2);
    }

    #[test]
    fn split_and_close_pane_promote_sibling() {
        let mgr = manager();
        ws(&mgr);
        let first_pane = mgr.state().workspaces[0].tabs[0].active_pane.unwrap();
        let s2 = sid();
        let second = mgr.split_pane(first_pane, SplitKind::H, s2).unwrap();

        let unbound = mgr.close_pane(second).unwrap();
        assert_eq!(unbound, vec![s2]);
        let state = mgr.state();
        assert_eq!(
            state.workspaces[0].tabs[0].root.as_ref().unwrap().id(),
            first_pane
        );
        assert_eq!(state.workspaces[0].tabs[0].active_pane, Some(first_pane));
    }

    #[test]
    fn session_disposed_removes_bindings_everywhere() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("a", None, s).unwrap();
        mgr.session_disposed(s).unwrap();
        let state = mgr.state();
        assert!(state.workspaces.is_empty());
    }

    #[test]
    fn ratio_is_clamped_and_leaf_rejected() {
        let mgr = manager();
        ws(&mgr);
        let pane = mgr.state().workspaces[0].tabs[0].active_pane.unwrap();
        mgr.split_pane(pane, SplitKind::V, sid()).unwrap();
        let split_id = mgr.state().workspaces[0].tabs[0]
            .root
            .as_ref()
            .unwrap()
            .id();

        mgr.set_split_ratio(split_id, 0.01, true).unwrap();
        match &mgr.state().workspaces[0].tabs[0].root {
            Some(PaneNode::Split { ratio, .. }) => {
                assert!((ratio - MIN_RATIO).abs() < f64::EPSILON)
            }
            _ => panic!("esperava split"),
        }
        assert!(matches!(
            mgr.set_split_ratio(pane, 0.5, true).unwrap_err(),
            LayoutError::NotASplit(_)
        ));
    }

    #[test]
    fn move_tab_reorders_within_workspace() {
        let mgr = manager();
        let id = ws(&mgr);
        let t1 = mgr.state().workspaces[0].tabs[0].id;
        let t2 = mgr.create_tab(sid(), Some(id)).unwrap();
        let t3 = mgr.create_tab(sid(), Some(id)).unwrap();

        mgr.move_tab(t3, 0).unwrap();
        let ids: Vec<TabId> = mgr.state().workspaces[0]
            .tabs
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, vec![t3, t1, t2]);
    }

    #[test]
    fn layout_round_trips_through_store() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let s1 = sid();
        let s2 = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("api", Some("/repo".into()), s1)
                .unwrap();
            let pane = mgr.state().workspaces[0].tabs[0].active_pane.unwrap();
            mgr.split_pane(pane, SplitKind::H, s2).unwrap();
        }

        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::from([s1, s2]));
        let state = mgr.state();
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].name, "api");
        assert_eq!(state.workspaces[0].repo_root.as_deref(), Some("/repo"));
        assert!(matches!(
            state.workspaces[0].tabs[0].root,
            Some(PaneNode::Split { .. })
        ));
    }

    #[test]
    fn load_gc_drops_dead_panes_but_keeps_empty_workspace() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let dead = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("api", Some("/repo".into()), dead)
                .unwrap();
        }

        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::new());
        let state = mgr.state();
        assert_eq!(state.workspaces.len(), 1);
        assert!(state.workspaces[0].tabs.is_empty());
        assert_eq!(state.workspaces[0].active_tab, None);
        assert_eq!(state.active_workspace, Some(state.workspaces[0].id));
    }

    #[test]
    fn workspace_of_session_acha_o_dono_e_ignora_sessao_solta() {
        let mgr = manager();
        let session = SessionId::new_v4();
        let ws = mgr.create_workspace("x", None, session).unwrap();
        assert_eq!(mgr.workspace_of_session(session), Some(ws));
        assert_eq!(mgr.workspace_of_session(SessionId::new_v4()), None);
    }

    #[test]
    fn open_view_tab_uses_dedicated_workspace_and_is_singleton() {
        let mgr = manager();
        let w1 = ws(&mgr);
        mgr.open_view_tab(VIEW_SETTINGS).unwrap();
        let state = mgr.state();
        // A view não vira aba do workspace de terminal: ganha o seu próprio.
        let terminal = state.workspaces.iter().find(|w| w.id == w1).unwrap();
        assert_eq!(terminal.tabs.len(), 1);
        let settings_ws = state
            .workspaces
            .iter()
            .find(|w| {
                w.id != w1
                    && w.tabs
                        .iter()
                        .any(|t| t.view.as_deref() == Some(VIEW_SETTINGS))
            })
            .unwrap();
        assert_eq!(settings_ws.tabs.len(), 1);
        assert_eq!(state.active_workspace, Some(settings_ws.id));
        let settings_ws_id = settings_ws.id;
        let total = state.workspaces.len();

        // Reabrir foca o mesmo workspace de settings (singleton), sem duplicar.
        ws(&mgr);
        mgr.open_view_tab(VIEW_SETTINGS).unwrap();
        let state = mgr.state();
        assert_eq!(
            state
                .workspaces
                .iter()
                .filter(|w| w
                    .tabs
                    .iter()
                    .any(|t| t.view.as_deref() == Some(VIEW_SETTINGS)))
                .count(),
            1
        );
        assert_eq!(state.active_workspace, Some(settings_ws_id));
        assert_eq!(state.workspaces.len(), total + 1);
    }

    #[test]
    fn open_view_tab_without_workspace_creates_fallback() {
        let mgr = manager();
        mgr.open_view_tab(VIEW_SETTINGS).unwrap();
        let state = mgr.state();
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].name, FALLBACK_WORKSPACE_NAME);
        assert_eq!(
            state.workspaces[0].tabs[0].view.as_deref(),
            Some(VIEW_SETTINGS)
        );
        assert_eq!(state.active_workspace, Some(state.workspaces[0].id));
    }

    #[test]
    fn docker_workspace_is_created_once_and_empty() {
        let mgr = manager();
        let first = mgr.docker_workspace().unwrap();
        let second = mgr.docker_workspace().unwrap();
        assert_eq!(first, second);
        let state = mgr.state();
        let ws = state.workspaces.iter().find(|w| w.id == first).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Docker);
        assert_eq!(ws.name, DOCKER_WORKSPACE_NAME);
        assert!(ws.tabs.is_empty());
    }

    #[test]
    fn closing_last_container_tab_closes_docker_workspace() {
        let mgr = manager();
        let docker_id = mgr.docker_workspace().unwrap();
        let tab = mgr.create_tab(sid(), Some(docker_id)).unwrap();
        mgr.close_tab(tab).unwrap();
        assert!(mgr.state().workspaces.is_empty());
        assert_ne!(mgr.docker_workspace().unwrap(), docker_id);
    }

    /// Layout salvo com o workspace Docker antigo (aba `containers`).
    fn retired_dashboard(extra_tab: Option<SessionId>) -> Workspace {
        let view = Tab::from_view(RETIRED_VIEW_CONTAINERS);
        let mut tabs = vec![view.clone()];
        if let Some(s) = extra_tab {
            tabs.push(Tab::from_session(s));
        }
        Workspace {
            id: Uuid::new_v4(),
            name: DOCKER_WORKSPACE_NAME.to_string(),
            name_locked: true,
            repo_root: None,
            color: None,
            group: None,
            kind: WorkspaceKind::Docker,
            launch_config_id: None,
            active_tab: Some(view.id),
            tabs,
            side_view: None,
            side_ratio: DEFAULT_SIDE_RATIO,
            side_expanded: false,
            history: Vec::new(),
            screens: Default::default(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn retired_dashboard_workspace_is_dropped_and_lands_on_compose() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let s = sid();
        let user = {
            let mgr = LayoutManager::new(Arc::clone(&store));
            let user = mgr.create_workspace("api", None, s).unwrap();
            // Inserir ativa o Docker: o app fechou no painel antigo.
            mgr.insert_workspace(retired_dashboard(None)).unwrap();
            user
        };
        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::from([s]));
        let state = mgr.state();
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.active_workspace, Some(user));
        assert_eq!(
            state.workspaces[0].screens.docker.section,
            screens::DockerSection::Compose
        );
    }

    #[test]
    fn retired_view_tab_goes_but_container_tabs_stay() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let s = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.insert_workspace(retired_dashboard(Some(s))).unwrap();
        }
        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::from([s]));
        let state = mgr.state();
        assert_eq!(state.workspaces.len(), 1);
        let ws = &state.workspaces[0];
        assert_eq!(ws.kind, WorkspaceKind::Docker);
        assert_eq!(ws.tabs.len(), 1);
        assert!(ws.tabs[0].view.is_none());
        assert_eq!(ws.active_tab, Some(ws.tabs[0].id));
    }

    #[test]
    fn untouched_layout_keeps_containers_section() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let s = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("api", None, s).unwrap();
        }
        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::from([s]));
        assert_eq!(
            mgr.state().workspaces[0].screens.docker.section,
            screens::DockerSection::Containers
        );
    }

    #[test]
    fn side_view_opens_in_session_workspace_and_closes_clean() {
        let mgr = manager();
        let s = sid();
        let ws_id = mgr.create_workspace("api", None, s).unwrap();
        ws(&mgr); // outro workspace ativo

        let opened = mgr.open_workspace_side_view(s, &diff_view(s)).unwrap();
        assert_eq!(opened, ws_id);
        let state = mgr.state();
        assert_eq!(state.active_workspace, Some(ws_id));
        let target = state.workspaces.iter().find(|w| w.id == ws_id).unwrap();
        assert_eq!(target.side_view.as_deref(), Some(diff_view(s).as_str()));

        mgr.set_side_view_expanded(ws_id, true).unwrap();
        assert!(mgr.state().workspaces[0].side_expanded);

        mgr.close_side_view(ws_id).unwrap();
        let target = &mgr.state().workspaces[0];
        assert_eq!(target.side_view, None);
        assert!(!target.side_expanded);
        // Fechar o painel nunca toca nas sessões/tabs.
        assert_eq!(target.tabs.len(), 1);
    }

    #[test]
    fn side_view_ratio_is_clamped() {
        let mgr = manager();
        let s = sid();
        let ws_id = mgr.create_workspace("api", None, s).unwrap();
        mgr.set_side_view_ratio(ws_id, 0.01, true).unwrap();
        assert!((mgr.state().workspaces[0].side_ratio - MIN_RATIO).abs() < f64::EPSILON);
    }

    #[test]
    fn creating_a_tab_closes_the_diff_panel_of_the_previous_focus() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        mgr.open_workspace_side_view(s, &diff_view(s)).unwrap();

        mgr.create_tab(sid(), None).unwrap();

        let ws = &mgr.state().workspaces[0];
        assert_eq!(ws.side_view, None);
        assert!(!ws.side_expanded);
    }

    #[test]
    fn activating_another_tab_closes_the_diff_panel() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        let s_tab = mgr.state().workspaces[0].tabs[0].id;
        let other_tab = mgr.create_tab(sid(), None).unwrap();
        mgr.activate_tab(s_tab).unwrap();
        mgr.open_workspace_side_view(s, &diff_view(s)).unwrap();

        mgr.activate_tab(other_tab).unwrap();

        assert_eq!(mgr.state().workspaces[0].side_view, None);
    }

    #[test]
    fn reactivating_the_diff_tab_keeps_the_panel() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        let s_tab = mgr.state().workspaces[0].tabs[0].id;
        mgr.open_workspace_side_view(s, &diff_view(s)).unwrap();

        mgr.activate_tab(s_tab).unwrap();

        assert_eq!(
            mgr.state().workspaces[0].side_view,
            Some(diff_view(s)),
            "foco continua na sessão do diff — o painel não pode fechar"
        );
    }

    #[test]
    fn focusing_a_pane_of_another_session_closes_the_diff_panel() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        let s_pane = mgr.state().workspaces[0].tabs[0].active_pane.unwrap();
        let s2 = sid();
        let s2_pane = mgr.split_pane(s_pane, SplitKind::V, s2).unwrap();
        mgr.focus_pane(s_pane).unwrap();
        mgr.open_workspace_side_view(s, &diff_view(s)).unwrap();

        mgr.focus_pane(s2_pane).unwrap();

        assert_eq!(mgr.state().workspaces[0].side_view, None);
    }

    #[test]
    fn opening_the_panel_focuses_the_pane_of_its_session() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        let s_pane = mgr.state().workspaces[0].tabs[0].active_pane.unwrap();
        let s2 = sid();
        mgr.split_pane(s_pane, SplitKind::V, s2).unwrap();

        mgr.open_workspace_side_view(s, &diff_view(s)).unwrap();

        let ws = &mgr.state().workspaces[0];
        assert_eq!(ws.tabs[0].active_pane, Some(s_pane));
        assert_eq!(ws.side_view, Some(diff_view(s)));
    }

    #[test]
    fn rename_locks_the_workspace_name_and_it_round_trips() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let s = sid();
        let ws_id = {
            let mgr = LayoutManager::new(Arc::clone(&store));
            let ws_id = mgr.create_workspace("poc-git", None, s).unwrap();
            assert!(
                !mgr.state().workspaces[0].name_locked,
                "nome de criação é automático — a UI pode seguir o cwd"
            );
            mgr.rename_workspace(ws_id, "meu projeto").unwrap();
            assert!(mgr.state().workspaces[0].name_locked);
            ws_id
        };

        let mgr = LayoutManager::new(store);
        mgr.load(&HashSet::from([s]));
        let ws = &mgr.state().workspaces[0];
        assert_eq!(ws.id, ws_id);
        assert_eq!(ws.name, "meu projeto");
        assert!(ws.name_locked, "flag precisa sobreviver ao restart");
    }

    #[test]
    fn session_disposed_clears_its_side_view() {
        let mgr = manager();
        let s = sid();
        let other = sid();
        mgr.create_workspace("api", None, s).unwrap();
        mgr.create_tab(other, None).unwrap();
        mgr.open_workspace_side_view(s, &diff_view(s)).unwrap();

        mgr.session_disposed(s).unwrap();
        let state = mgr.state();
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].side_view, None);
    }

    #[test]
    fn session_disposed_clears_every_session_panel() {
        for view in [
            git_view,
            docker_view,
            tunnels_view,
            files_view,
            agents_view,
            history_view,
        ] {
            let mgr = manager();
            let s = sid();
            let other = sid();
            mgr.create_workspace("api", None, s).unwrap();
            mgr.create_tab(other, None).unwrap();
            mgr.open_workspace_side_view(s, &view(s)).unwrap();

            mgr.session_disposed(s).unwrap();
            assert_eq!(mgr.state().workspaces[0].side_view, None, "{}", view(s));
        }
    }

    #[test]
    fn session_disposed_keeps_panel_of_other_session() {
        let mgr = manager();
        let s = sid();
        let other = sid();
        mgr.create_workspace("api", None, s).unwrap();
        mgr.create_tab(other, None).unwrap();
        mgr.open_workspace_side_view(other, &git_view(other))
            .unwrap();

        mgr.session_disposed(s).unwrap();
        assert_eq!(mgr.state().workspaces[0].side_view, Some(git_view(other)));
    }

    #[test]
    fn git_and_docker_panels_of_dead_session_do_not_come_back_on_boot() {
        for view in [git_view, docker_view, history_view] {
            let store = Arc::new(Store::open_in_memory().unwrap());
            let dead = sid();
            {
                let mgr = LayoutManager::new(Arc::clone(&store));
                mgr.create_workspace("b", None, dead).unwrap();
                mgr.open_workspace_side_view(dead, &view(dead)).unwrap();
            }
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.load(&HashSet::new());
            for w in &mgr.state().workspaces {
                assert_eq!(w.side_view, None, "{}", view(dead));
            }
        }
    }

    #[test]
    fn opening_another_panel_replaces_the_slot() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        mgr.open_workspace_side_view(s, &git_view(s)).unwrap();
        mgr.open_workspace_side_view(s, &docker_view(s)).unwrap();
        assert_eq!(
            mgr.state().workspaces[0].side_view.as_deref(),
            Some(docker_view(s).as_str()),
            "um painel por vez: abrir outro troca o que está no slot"
        );
    }

    #[test]
    fn painel_de_tuneis_de_sessao_morta_nao_volta_no_boot() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let dead = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("b", None, dead).unwrap();
            mgr.open_workspace_side_view(dead, &tunnels_view(dead))
                .unwrap();
        }

        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::new());

        for w in &mgr.state().workspaces {
            assert_eq!(
                w.side_view, None,
                "o filtro tem que enxergar o prefixo tunnels: também — só o diff: \
                 deixaria um painel de túneis pendurado numa sessão que não existe mais"
            );
        }
    }

    #[test]
    fn side_view_round_trips_through_store_and_drops_dead_session() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let alive = sid();
        let dead = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            let ws1 = mgr.create_workspace("a", None, alive).unwrap();
            mgr.create_workspace("b", None, dead).unwrap();
            mgr.open_workspace_side_view(alive, &diff_view(alive))
                .unwrap();
            mgr.open_workspace_side_view(dead, &diff_view(dead))
                .unwrap();
            mgr.set_side_view_ratio(ws1, 0.7, true).unwrap();
            mgr.set_side_view_expanded(ws1, true).unwrap();
        }

        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::from([alive]));
        let state = mgr.state();
        let a = state.workspaces.iter().find(|w| w.name == "a").unwrap();
        assert_eq!(a.side_view.as_deref(), Some(diff_view(alive).as_str()));
        assert!((a.side_ratio - 0.7).abs() < 1e-9);
        assert!(a.side_expanded);
        // Workspace do dead perde tabs (GC) e o side_view do morto some.
        for w in &state.workspaces {
            if w.name == "b" {
                assert_eq!(w.side_view, None);
            }
        }
    }

    #[test]
    fn load_drops_legacy_diff_view_tab() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let s = sid();
        let mut rows = {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("api", None, s).unwrap();
            workspaces_to_rows(&mgr.state().workspaces)
        };
        rows.tabs.push(TabRow {
            id: Uuid::new_v4().to_string(),
            workspace_id: Some(rows.workspaces[0].id.clone()),
            title: None,
            view: Some(diff_view(s)),
            position: 1,
            active_pane: None,
            created_at: Utc::now().to_rfc3339(),
        });
        store.save_layout(&rows).unwrap();

        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::from([s]));
        let state = mgr.state();
        assert_eq!(state.workspaces[0].tabs.len(), 1);
        assert!(state.workspaces[0].tabs[0].view.is_none());
    }

    #[test]
    fn load_partial_gc_collapses_split_with_dead_leaf() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let alive = sid();
        let dead = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("api", None, alive).unwrap();
            let pane = mgr.state().workspaces[0].tabs[0].active_pane.unwrap();
            mgr.split_pane(pane, SplitKind::V, dead).unwrap();
        }

        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::from([alive]));
        let state = mgr.state();
        assert_eq!(state.workspaces[0].tabs.len(), 1);
        match &state.workspaces[0].tabs[0].root {
            Some(PaneNode::Leaf { session_id, .. }) => assert_eq!(*session_id, alive),
            other => panic!("esperava leaf colapsado, veio {other:?}"),
        }
    }
    /// O bug do reopen (#50): o PTY morre com o app, o pane guarda o id da sessão
    /// morta, o filtro de `valid_sessions` o descarta e a tab abre vazia. O remap
    /// religa o pane à sessão que nasceu no mesmo cwd.
    #[test]
    fn load_remapped_rebinds_the_pane_to_the_reopened_session() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let dead = sid();
        let reopened = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("poc", None, dead).unwrap();
        }

        // Sem remap: é o comportamento quebrado que o usuário via.
        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::from([reopened]));
        assert!(
            mgr.state().workspaces[0].tabs.is_empty(),
            "sem remap a tab some — este é o bug"
        );

        let store = Arc::new(Store::open_in_memory().unwrap());
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("poc", None, dead).unwrap();
        }
        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load_remapped(
            &HashSet::from([reopened]),
            &std::collections::HashMap::from([(dead, reopened)]),
        );

        let state = mgr.state();
        assert_eq!(state.workspaces[0].tabs.len(), 1, "a tab precisa voltar");
        match &state.workspaces[0].tabs[0].root {
            Some(PaneNode::Leaf { session_id, .. }) => assert_eq!(
                *session_id, reopened,
                "o pane precisa apontar pra sessão nova, não pra morta"
            ),
            other => panic!("esperava leaf com a sessão reaberta, veio {other:?}"),
        }
    }

    fn active_root(mgr: &LayoutManager) -> PaneNode {
        let state = mgr.state();
        let ws = &state.workspaces[0];
        let tab = ws
            .tabs
            .iter()
            .find(|t| Some(t.id) == ws.active_tab)
            .unwrap();
        tab.root.clone().unwrap()
    }

    fn count_agent_viewers(node: &PaneNode) -> usize {
        match node {
            PaneNode::AgentViewer { .. } => 1,
            PaneNode::Leaf { .. } => 0,
            PaneNode::Split { first, second, .. } => {
                count_agent_viewers(first) + count_agent_viewers(second)
            }
        }
    }

    #[test]
    fn ensure_agent_viewer_splits_active_pane_with_viewer_in_second() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();

        assert!(mgr.ensure_agent_viewer(s));

        match active_root(&mgr) {
            PaneNode::Split {
                split,
                ratio,
                first,
                second,
                ..
            } => {
                assert_eq!(split, SplitKind::V);
                assert!((ratio - 0.62).abs() < 1e-9);
                assert!(matches!(*first, PaneNode::Leaf { session_id, .. } if session_id == s));
                assert!(
                    matches!(*second, PaneNode::AgentViewer { session_id, .. } if session_id == s)
                );
            }
            other => panic!("esperava split, veio {other:?}"),
        }
    }

    #[test]
    fn ensure_agent_viewer_is_idempotent_per_session() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        assert!(mgr.ensure_agent_viewer(s));
        assert!(!mgr.ensure_agent_viewer(s));
        assert_eq!(count_agent_viewers(&active_root(&mgr)), 1);
    }

    #[test]
    fn ensure_agent_viewer_skips_session_outside_active_tab() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        mgr.create_tab(sid(), None).unwrap();

        assert!(!mgr.ensure_agent_viewer(s));
        for tab in &mgr.state().workspaces[0].tabs {
            if let Some(root) = &tab.root {
                assert_eq!(count_agent_viewers(root), 0);
            }
        }
    }

    #[test]
    fn coordination_skips_session_in_inactive_workspace() {
        let mgr = manager();
        let background = sid();
        let foreground = sid();
        mgr.create_workspace("bg", None, background).unwrap();
        let bg_ws = mgr.state().active_workspace.unwrap();
        mgr.create_workspace("fg", None, foreground).unwrap();
        let fg_ws = mgr.state().active_workspace.unwrap();

        assert!(!mgr.ensure_agent_viewer(background));
        assert!(!mgr.ensure_agents_panel(background));

        let state = mgr.state();
        assert_eq!(state.active_workspace, Some(fg_ws));
        assert!(!has_agent_viewer(&state.workspaces, background));
        let bg = state.workspaces.iter().find(|w| w.id == bg_ws).unwrap();
        assert!(bg.side_view.is_none());
    }

    #[test]
    fn ensure_agents_panel_opens_only_in_active_workspace() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();

        assert!(mgr.ensure_agents_panel(s));
        assert_eq!(
            mgr.state().workspaces[0].side_view.as_deref(),
            Some(agents_view(s).as_str())
        );
        assert!(
            mgr.ensure_agents_panel(s),
            "já aberto conta como coordenado"
        );
    }

    #[test]
    fn ensure_agents_panel_takes_the_queue_slot_but_not_other_views() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();

        // A fila aberta pelo menu lateral não segura a rodada nova (tela 4).
        mgr.toggle_agent_queue().unwrap();
        assert!(mgr.ensure_agents_panel(s));
        assert_eq!(
            mgr.state().workspaces[0].side_view.as_deref(),
            Some(agents_view(s).as_str())
        );

        // Uma view que o usuário escolheu (diff) não é tomada.
        mgr.open_workspace_side_view(s, &format!("diff:{s}"))
            .unwrap();
        assert!(!mgr.ensure_agents_panel(s));
        assert_eq!(
            mgr.state().workspaces[0].side_view,
            Some(format!("diff:{s}"))
        );
    }

    #[test]
    fn ensure_agent_viewer_does_not_change_active_workspace() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        let active_before = mgr.state().active_workspace;

        assert!(mgr.ensure_agent_viewer(s));
        assert_eq!(mgr.state().active_workspace, active_before);
    }

    #[test]
    fn closing_the_agent_viewer_keeps_the_terminal_leaf() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        mgr.ensure_agent_viewer(s);
        let viewer_pane = match active_root(&mgr) {
            PaneNode::Split { second, .. } => second.id(),
            other => panic!("esperava split, veio {other:?}"),
        };

        assert_eq!(mgr.close_pane(viewer_pane).unwrap(), vec![s]);
        match active_root(&mgr) {
            PaneNode::Leaf { session_id, .. } => assert_eq!(session_id, s),
            other => panic!("esperava leaf colapsado, veio {other:?}"),
        }
    }

    #[test]
    fn agent_viewer_session_resolves_only_the_viewer_pane() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        mgr.ensure_agent_viewer(s);
        let (leaf_pane, viewer_pane) = match active_root(&mgr) {
            PaneNode::Split { first, second, .. } => (first.id(), second.id()),
            other => panic!("esperava split, veio {other:?}"),
        };
        assert_eq!(mgr.agent_viewer_session(viewer_pane), Some(s));
        assert_eq!(mgr.agent_viewer_session(leaf_pane), None);
    }

    #[test]
    fn agent_viewer_round_trips_through_store() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let alive = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("api", None, alive).unwrap();
            assert!(mgr.ensure_agent_viewer(alive));
        }
        let mgr = LayoutManager::new(store);
        mgr.load(&HashSet::from([alive]));
        match active_root(&mgr) {
            PaneNode::Split { split, second, .. } => {
                assert_eq!(split, SplitKind::V);
                assert!(
                    matches!(*second, PaneNode::AgentViewer { session_id, .. } if session_id == alive)
                );
            }
            other => panic!("esperava split preservado, veio {other:?}"),
        }
    }

    #[test]
    fn agent_viewer_is_dropped_on_load_when_its_session_is_dead() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let dead = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("api", None, dead).unwrap();
            assert!(mgr.ensure_agent_viewer(dead));
        }
        let mgr = LayoutManager::new(store);
        mgr.load(&HashSet::new());
        assert!(mgr.state().workspaces[0].tabs.is_empty());
    }

    #[test]
    fn session_disposed_removes_agent_viewer_and_agents_panel() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        assert!(mgr.ensure_agent_viewer(s));
        mgr.open_workspace_side_view(s, &agents_view(s)).unwrap();

        mgr.session_disposed(s).unwrap();

        assert!(mgr.state().workspaces.is_empty());
    }

    #[test]
    fn open_agent_viewer_errors_when_session_not_in_active_workspace() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        mgr.create_workspace("other", None, sid()).unwrap();

        let err = mgr.open_agent_viewer(s).unwrap_err();
        assert!(matches!(err, LayoutError::SessionNotVisible(id) if id == s));
    }

    #[test]
    fn open_agent_viewer_works_from_inactive_tab_of_active_workspace() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        mgr.create_tab(sid(), None).unwrap();

        assert!(!mgr.ensure_agent_viewer(s));
        assert!(mgr.open_agent_viewer(s).unwrap());

        let root = active_root(&mgr);
        assert_eq!(count_agent_viewers(&root), 1);
    }

    #[test]
    fn leaf_sessions_ignores_agent_viewer() {
        let term = sid();
        let node = PaneNode::Split {
            id: Uuid::new_v4(),
            split: SplitKind::V,
            ratio: 0.62,
            first: Box::new(PaneNode::Leaf {
                id: Uuid::new_v4(),
                session_id: term,
            }),
            second: Box::new(PaneNode::AgentViewer {
                id: Uuid::new_v4(),
                session_id: term,
            }),
        };
        let mut out = Vec::new();
        node.leaf_sessions(&mut out);
        assert_eq!(out, vec![term]);
    }

    fn active_ws(mgr: &LayoutManager) -> Workspace {
        let state = mgr.state();
        state
            .workspaces
            .iter()
            .find(|w| Some(w.id) == state.active_workspace)
            .unwrap()
            .clone()
    }

    #[test]
    fn content_tab_opens_next_to_current_and_is_singleton() {
        let mgr = manager();
        let s = sid();
        let ws_id = mgr.create_workspace("api", None, s).unwrap();
        mgr.create_tab(sid(), None).unwrap();
        let first = active_ws(&mgr).tabs[0].id;
        mgr.activate_tab(first).unwrap();
        let view = editor_view(s, "src/server.js");
        let a = mgr.open_content_tab(None, &view).unwrap();
        let ws = active_ws(&mgr);
        assert_eq!(ws.id, ws_id);
        assert_eq!(ws.tabs[1].id, a, "abre ao lado da aba de onde veio");
        assert_eq!(ws.active_tab, Some(a));
        let again = mgr.open_content_tab(None, &view).unwrap();
        assert_eq!(again, a);
        assert_eq!(active_ws(&mgr).tabs.len(), 3);
    }

    #[test]
    fn closing_active_tab_goes_to_previously_used_one() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        let term = active_ws(&mgr).tabs[0].id;
        let logs = mgr
            .open_content_tab(None, &format!("{VIEW_CONTAINER_LOGS_PREFIX}abc"))
            .unwrap();
        let file = mgr.open_content_tab(None, &editor_view(s, "a.rs")).unwrap();
        // Usou: term → logs → file → term → file. Fecha file: volta para term,
        // não para a vizinha (logs).
        mgr.activate_tab(term).unwrap();
        mgr.activate_tab(file).unwrap();
        mgr.close_tab(file).unwrap();
        assert_eq!(active_ws(&mgr).active_tab, Some(term));
        let _ = logs;
    }

    #[test]
    fn back_tab_walks_history_like_a_browser() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        let term = active_ws(&mgr).tabs[0].id;
        let logs = mgr
            .open_content_tab(None, &format!("{VIEW_CONTAINER_LOGS_PREFIX}abc"))
            .unwrap();
        let file = mgr.open_content_tab(None, &editor_view(s, "a.rs")).unwrap();
        assert_eq!(active_ws(&mgr).active_tab, Some(file));
        assert!(mgr.back_tab().unwrap());
        assert_eq!(active_ws(&mgr).active_tab, Some(logs));
        assert!(mgr.back_tab().unwrap());
        assert_eq!(active_ws(&mgr).active_tab, Some(term));
        assert!(!mgr.back_tab().unwrap(), "sem histórico, não anda");
    }

    #[test]
    fn editor_tab_keeps_its_session_diff_panel_and_dies_with_session() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        mgr.open_workspace_side_view(s, &diff_view(s)).unwrap();
        let file = mgr.open_content_tab(None, &editor_view(s, "a.rs")).unwrap();
        let ws = active_ws(&mgr);
        assert_eq!(ws.side_view, Some(diff_view(s)), "editor é da mesma sessão");
        assert_eq!(ws.active_tab, Some(file));
        assert_eq!(mgr.active_session(), Some(s));
        assert!(mgr.has_editor_tabs(s));
        // Abrir o painel de arquivos com o editor em foco não troca de aba.
        mgr.open_workspace_side_view(s, &files_view(s)).unwrap();
        assert_eq!(active_ws(&mgr).active_tab, Some(file));

        let other = sid();
        mgr.create_tab(other, None).unwrap();
        mgr.session_disposed(s).unwrap();
        assert!(!mgr.has_editor_tabs(s));
        let ws = active_ws(&mgr);
        assert_eq!(ws.tabs.len(), 1);
    }

    #[test]
    fn editor_view_round_trips_and_parses_session() {
        let s = sid();
        let v = editor_view(s, "dir/with:colon.rs");
        assert_eq!(editor_view_session(&v), Some(s));
        assert_eq!(editor_view_session("container-logs:abc"), None);
        assert_eq!(editor_view_session("editor:not-a-uuid:x"), None);
        let c = compose_file_view(s, "poc", 0, "compose.yml");
        assert_eq!(c, format!("compose-file:{s}:poc:0:compose.yml"));
        assert_eq!(screen_file_session(&c), Some(s));
        assert_eq!(editor_view_session(&c), None);
        assert_eq!(screen_file_session(&v), Some(s));
    }

    #[test]
    fn editor_tab_of_dead_session_is_dropped_on_load_and_remapped_when_reopened() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let s = sid();
        let other = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("api", None, s).unwrap();
            mgr.create_tab(other, None).unwrap();
            mgr.open_content_tab(None, &editor_view(s, "a.rs")).unwrap();
        }
        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::from([other]));
        assert!(mgr.state().workspaces[0]
            .tabs
            .iter()
            .all(|t| t.view.is_none()));

        let store = Arc::new(Store::open_in_memory().unwrap());
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("api", None, s).unwrap();
            mgr.open_content_tab(None, &editor_view(s, "a.rs")).unwrap();
        }
        let reborn = sid();
        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load_remapped(
            &HashSet::from([reborn]),
            &std::collections::HashMap::from([(s, reborn)]),
        );
        // Volta remapeado, agora dentro da tela de Arquivos.
        let views: Vec<_> = mgr.state().workspaces[0]
            .screens
            .files
            .tabs
            .iter()
            .map(|t| t.view.clone())
            .collect();
        assert_eq!(views, vec![editor_view(reborn, "a.rs")]);
    }

    #[test]
    fn file_screen_survives_reload_remapped_and_drops_dead_sessions() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let s = sid();
        let other = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_workspace("api", None, s).unwrap();
            mgr.create_tab(other, None).unwrap();
            mgr.open_file_screen(s, &editor_view(s, "a.rs"), true)
                .unwrap();
            mgr.open_file_screen(other, &editor_view(other, "b.rs"), false)
                .unwrap();
        }
        let files_of = |mgr: &LayoutManager| {
            let state = mgr.state();
            let f = state.workspaces[0].screens.files.clone();
            (
                f.tabs
                    .iter()
                    .map(|t| (t.view.clone(), t.preview))
                    .collect::<Vec<_>>(),
                f.active,
            )
        };

        // Mesma sessão viva: volta igual, e de novo no reinício seguinte.
        for _ in 0..2 {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.load(&HashSet::from([s, other]));
            let (tabs, active) = files_of(&mgr);
            assert_eq!(
                tabs,
                vec![
                    (editor_view(s, "a.rs"), true),
                    (editor_view(other, "b.rs"), false)
                ]
            );
            assert_eq!(active, Some(editor_view(other, "b.rs")));
        }

        // Sessão reaberta com outro id: volta remapeada; a morta some.
        let reborn = sid();
        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load_remapped(
            &HashSet::from([reborn]),
            &std::collections::HashMap::from([(s, reborn)]),
        );
        let (tabs, active) = files_of(&mgr);
        assert_eq!(tabs, vec![(editor_view(reborn, "a.rs"), true)]);
        assert_eq!(active, Some(editor_view(reborn, "a.rs")));
        assert_eq!(
            mgr.state().workspaces[0].screens.files.recent,
            vec![editor_view(reborn, "a.rs")]
        );
    }

    #[test]
    fn session_diff_tab_belongs_to_its_session() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        let tab = mgr.open_content_tab(None, &session_diff_view(s)).unwrap();
        assert_eq!(content_view_session(&session_diff_view(s)), Some(s));
        assert_eq!(mgr.active_session(), Some(s));
        assert_eq!(active_ws(&mgr).active_tab, Some(tab));
        mgr.create_tab(sid(), None).unwrap();
        mgr.session_disposed(s).unwrap();
        assert!(active_ws(&mgr)
            .tabs
            .iter()
            .all(|t| t.view.as_deref() != Some(session_diff_view(s).as_str())));
    }

    #[test]
    fn git_log_tab_belongs_to_its_session_and_is_singleton() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        let tab = mgr.open_content_tab(None, &git_log_view(s)).unwrap();
        assert_eq!(mgr.open_content_tab(None, &git_log_view(s)).unwrap(), tab);
        assert_eq!(content_view_session(&git_log_view(s)), Some(s));
        assert_eq!(mgr.active_session(), Some(s));
        mgr.create_tab(sid(), None).unwrap();
        mgr.session_disposed(s).unwrap();
        assert!(active_ws(&mgr)
            .tabs
            .iter()
            .all(|t| t.view.as_deref() != Some(git_log_view(s).as_str())));
    }

    #[test]
    fn pr_tabs_belong_to_their_session_and_are_singletons() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        let list = mgr.open_content_tab(None, &pr_list_view(s)).unwrap();
        assert_eq!(mgr.open_content_tab(None, &pr_list_view(s)).unwrap(), list);
        let pr = mgr.open_content_tab(None, &pr_view(s, 42)).unwrap();
        assert_eq!(mgr.open_content_tab(None, &pr_view(s, 42)).unwrap(), pr);
        assert_ne!(mgr.open_content_tab(None, &pr_view(s, 7)).unwrap(), pr);
        assert_ne!(list, pr);
        assert_eq!(pr_view(s, 42), format!("pr:{s}:42"));
        assert_eq!(content_view_session(&pr_list_view(s)), Some(s));
        assert_eq!(content_view_session(&pr_view(s, 42)), Some(s));
        assert_eq!(content_view_session("pr:nao-e-uuid:1"), None);
        mgr.create_tab(sid(), None).unwrap();
        mgr.session_disposed(s).unwrap();
        assert!(active_ws(&mgr).tabs.iter().all(|t| t
            .view
            .as_deref()
            .and_then(content_view_session)
            != Some(s)));
    }

    #[test]
    fn closing_workspace_or_tab_stops_the_log_follower() {
        use crate::docker::logs::{LogsManager, PathMapping};
        use std::process::{Command, Stdio};
        let spawn = |_: Option<String>| {
            Command::new("/bin/sh")
                .args(["-c", "sleep 30"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())
        };
        let mgr = manager();
        let logs = LogsManager::new();
        let s = sid();
        let ws = mgr.create_workspace("api", None, s).unwrap();
        let logs_view = format!("{VIEW_CONTAINER_LOGS_PREFIX}abc");
        let tab = mgr.open_content_tab(None, &logs_view).unwrap();
        logs.start("abc", PathMapping::default(), Arc::new(|_| {}), spawn)
            .unwrap();
        logs.retain(&open_log_containers(&mgr.state()));
        assert!(logs.is_following("abc"), "aba aberta mantém o seguidor");

        // A sessão dona do workspace morre: a aba de logs continua lá.
        mgr.session_disposed(s).unwrap();
        logs.retain(&open_log_containers(&mgr.state()));
        assert!(logs.is_following("abc"));

        mgr.close_workspace(ws).unwrap();
        logs.retain(&open_log_containers(&mgr.state()));
        assert!(
            !logs.is_following("abc"),
            "fechar o workspace mata o seguidor"
        );

        // Fechar só a aba também.
        let s2 = sid();
        mgr.create_workspace("web", None, s2).unwrap();
        let tab2 = mgr.open_content_tab(None, &logs_view).unwrap();
        logs.start("abc", PathMapping::default(), Arc::new(|_| {}), spawn)
            .unwrap();
        mgr.close_tab(tab2).unwrap();
        logs.retain(&open_log_containers(&mgr.state()));
        assert!(!logs.is_following("abc"));
        let _ = tab;
    }

    #[test]
    fn open_file_screen_opens_files_view_without_creating_tabs() {
        let mgr = manager();
        let s = sid();
        let ws = mgr.create_workspace("api", None, s).unwrap();
        let view = editor_view(s, "src/main.rs");
        mgr.open_file_screen(s, &view, true).unwrap();
        let state = mgr.state();
        let w = &state.workspaces[0];
        assert_eq!(w.id, ws);
        assert_eq!(
            w.tabs.len(),
            1,
            "abrir arquivo não cria aba ao lado do terminal"
        );
        assert_eq!(w.side_view.as_deref(), Some(files_view(s).as_str()));
        assert_eq!(w.screens.files.active.as_deref(), Some(view.as_str()));
        assert!(w.screens.files.tabs[0].preview);
        assert!(mgr.has_editor_tabs(s));
    }

    #[test]
    fn disposed_session_drops_its_screen_files() {
        let mgr = manager();
        let s = sid();
        mgr.create_workspace("api", None, s).unwrap();
        let other = sid();
        mgr.create_tab(other, None).unwrap();
        mgr.open_file_screen(s, &editor_view(s, "a.rs"), false)
            .unwrap();
        mgr.open_file_screen(other, &editor_view(other, "b.rs"), false)
            .unwrap();
        mgr.session_disposed(s).unwrap();
        let w = &mgr.state().workspaces[0];
        assert_eq!(w.screens.files.tabs.len(), 1);
        assert!(!mgr.has_editor_tabs(s));
        assert!(mgr.has_editor_tabs(other));
    }

    #[test]
    fn legacy_editor_tabs_load_into_files_screen() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let s = sid();
        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.create_workspace("api", None, s).unwrap();
        let view = editor_view(s, "src/lib.rs");
        mgr.open_content_tab(None, &view).unwrap();
        assert_eq!(mgr.state().workspaces[0].tabs.len(), 2);

        let reloaded = LayoutManager::new(store);
        reloaded.load(&HashSet::from([s]));
        let w = &reloaded.state().workspaces[0];
        assert_eq!(w.tabs.len(), 1, "a aba de editor sai da barra de cima");
        assert!(w.tabs[0].view.is_none());
        assert_eq!(w.screens.files.tabs.len(), 1);
        assert_eq!(w.screens.files.tabs[0].view, view);
        assert!(!w.screens.files.tabs[0].preview);
    }

    #[test]
    fn docker_screen_holds_log_follower_only_while_open_on_logs() {
        let mgr = manager();
        let s = sid();
        let ws = mgr.create_workspace("api", None, s).unwrap();
        mgr.with_screens(ws, |sc| sc.docker.container = Some("abc".into()))
            .unwrap();
        assert!(
            open_log_containers(&mgr.state()).is_empty(),
            "tela fechada não segura seguidor"
        );
        mgr.open_workspace_side_view(s, &docker_view(s)).unwrap();
        assert!(open_log_containers(&mgr.state()).contains("abc"));
        mgr.with_screens(ws, |sc| sc.docker.pane = screens::ContainerPane::Details)
            .unwrap();
        assert!(open_log_containers(&mgr.state()).is_empty());
    }
}
