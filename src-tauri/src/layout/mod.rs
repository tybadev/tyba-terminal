use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session::store::Store;
use crate::session::SessionId;

pub const EVENT_CHANGED: &str = "layout://changed";

const KEY_ACTIVE_WORKSPACE: &str = "layout.active_workspace";
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
            PaneNode::Leaf { id, .. } | PaneNode::Split { id, .. } => *id,
        }
    }

    fn first_leaf(&self) -> PaneId {
        match self {
            PaneNode::Leaf { id, .. } => *id,
            PaneNode::Split { first, .. } => first.first_leaf(),
        }
    }

    fn contains(&self, pane: PaneId) -> bool {
        match self {
            PaneNode::Leaf { id, .. } => *id == pane,
            PaneNode::Split {
                id, first, second, ..
            } => *id == pane || first.contains(pane) || second.contains(pane),
        }
    }

    fn find_leaf_by_session(&self, session: SessionId) -> Option<PaneId> {
        match self {
            PaneNode::Leaf { id, session_id } if *session_id == session => Some(*id),
            PaneNode::Leaf { .. } => None,
            PaneNode::Split { first, second, .. } => first
                .find_leaf_by_session(session)
                .or_else(|| second.find_leaf_by_session(session)),
        }
    }

    fn leaf_session(&self, pane: PaneId) -> Option<SessionId> {
        match self {
            PaneNode::Leaf { id, session_id } if *id == pane => Some(*session_id),
            PaneNode::Leaf { .. } => None,
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
            PaneNode::Leaf { .. } => None,
            PaneNode::Split { first, second, .. } => first
                .split_leaf(target, kind, session)
                .or_else(|| second.split_leaf(target, kind, session)),
        }
    }

    fn remove_leaf(self, target: PaneId) -> Option<PaneNode> {
        match self {
            PaneNode::Leaf { id, .. } if id == target => None,
            leaf @ PaneNode::Leaf { .. } => Some(leaf),
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
            PaneNode::Leaf { .. } => false,
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
            PaneNode::Split { first, second, .. } => {
                first.leaf_sessions(out);
                second.leaf_sessions(out);
            }
        }
    }

    fn retain_sessions(self, valid: &HashSet<SessionId>) -> Option<PaneNode> {
        match self {
            PaneNode::Leaf { session_id, .. } if !valid.contains(&session_id) => None,
            leaf @ PaneNode::Leaf { .. } => Some(leaf),
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

pub const VIEW_CONTAINERS: &str = "containers";
pub const VIEW_SETTINGS: &str = "settings";
pub const VIEW_WORKSPACE: &str = "workspace";
pub const DOCKER_WORKSPACE_NAME: &str = "Docker";
pub const FALLBACK_WORKSPACE_NAME: &str = "tyba";

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
    pub repo_root: Option<String>,
    pub color: Option<String>,
    pub group: Option<String>,
    pub kind: WorkspaceKind,
    pub active_tab: Option<TabId>,
    pub tabs: Vec<Tab>,
    pub created_at: DateTime<Utc>,
}

impl Workspace {
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
    pub repo_root: Option<String>,
    pub color: Option<String>,
    pub group_name: Option<String>,
    pub kind: Option<String>,
    pub position: i64,
    pub active_tab: Option<String>,
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
        let rows = self.store.load_layout().unwrap_or_default();
        let workspaces = rows_to_workspaces(&rows, valid_sessions);

        let saved_active = self
            .store
            .get_setting(KEY_ACTIVE_WORKSPACE)
            .ok()
            .flatten()
            .and_then(|s| Uuid::parse_str(&s).ok());
        let active = saved_active
            .filter(|id| workspaces.iter().any(|w| w.id == *id))
            .or_else(|| workspaces.first().map(|w| w.id));

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
        let mut inner = self.inner.write();
        if find_session_pane(&inner.workspaces, session).is_some() {
            return Err(LayoutError::SessionAlreadyBound(session));
        }
        let tab = Tab::from_session(session);
        let workspace = Workspace {
            id: Uuid::new_v4(),
            name: name.trim().to_string(),
            repo_root,
            color: None,
            group: None,
            kind: WorkspaceKind::User,
            active_tab: Some(tab.id),
            tabs: vec![tab],
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
            repo_root: None,
            color: None,
            group: None,
            kind: WorkspaceKind::User,
            active_tab: Some(tab_id),
            tabs: vec![tab],
            created_at: Utc::now(),
        };
        let ws_id = workspace.id;
        inner.workspaces.push(workspace);
        inner.active = Some(ws_id);
        drop(inner);
        self.persist()
    }

    pub fn open_docker_dashboard(&self) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let ws_id = ensure_docker_workspace(&mut inner);
        let idx = ws_index(&inner.workspaces, ws_id)?;
        let view_tab = inner.workspaces[idx]
            .tabs
            .iter()
            .find(|t| t.view.as_deref() == Some(VIEW_CONTAINERS))
            .map(|t| t.id);
        inner.workspaces[idx].active_tab = view_tab.or(inner.workspaces[idx].active_tab);
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
        inner.workspaces[idx].active_tab = Some(tab_id);
        inner.active = Some(target);
        drop(inner);
        self.persist()?;
        Ok(tab_id)
    }

    pub fn close_tab(&self, tab: TabId) -> Result<Vec<SessionId>, LayoutError> {
        let mut inner = self.inner.write();
        let (w_idx, t_idx) = tab_index(&inner.workspaces, tab)?;
        let removed = inner.workspaces[w_idx].tabs.remove(t_idx);
        let ws = &mut inner.workspaces[w_idx];
        if ws.active_tab == Some(tab) {
            ws.active_tab = ws
                .tabs
                .get(t_idx.min(ws.tabs.len().saturating_sub(1)))
                .map(|t| t.id)
                .filter(|_| !ws.tabs.is_empty());
        }
        remove_workspace_if_empty(&mut inner, w_idx);
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
        inner.workspaces[w_idx].active_tab = Some(tab);
        inner.active = Some(ws_id);
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
                    inner.workspaces[idx].active_tab = Some(tab_id);
                    if let Some(tab) = inner.workspaces[idx]
                        .tabs
                        .iter_mut()
                        .find(|t| t.id == tab_id)
                    {
                        tab.active_pane = Some(pane_id);
                    }
                }
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
                inner.workspaces[wi].tabs.remove(ti);
                let ws = &mut inner.workspaces[wi];
                if ws.active_tab == Some(tab.id) {
                    ws.active_tab = ws
                        .tabs
                        .get(ti.min(ws.tabs.len().saturating_sub(1)))
                        .map(|t| t.id)
                        .filter(|_| !ws.tabs.is_empty());
                }
                remove_workspace_if_empty(&mut inner, wi);
            }
        }
        drop(inner);
        self.persist()?;
        Ok(vec![session])
    }

    pub fn focus_pane(&self, pane: PaneId) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        for ws in inner.workspaces.iter_mut() {
            let ws_id = ws.id;
            for tab in ws.tabs.iter_mut() {
                if tab.root.as_ref().is_some_and(|r| r.contains(pane)) {
                    tab.active_pane = Some(pane);
                    let tab_id = tab.id;
                    ws.active_tab = Some(tab_id);
                    inner.active = Some(ws_id);
                    drop(inner);
                    return self.persist();
                }
            }
        }
        Err(LayoutError::PaneNotFound(pane))
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

    pub fn session_disposed(&self, session: SessionId) -> Result<(), LayoutError> {
        loop {
            let target = {
                let inner = self.inner.read();
                find_session_pane(&inner.workspaces, session).map(|(_, _, pane)| pane)
            };
            match target {
                Some(pane) => {
                    self.close_pane(pane)?;
                }
                None => return Ok(()),
            }
        }
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
        Ok(())
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

fn ensure_docker_workspace(inner: &mut Inner) -> WorkspaceId {
    if let Some(idx) = inner
        .workspaces
        .iter()
        .position(|w| w.kind == WorkspaceKind::Docker)
    {
        let ws = &mut inner.workspaces[idx];
        if !ws
            .tabs
            .iter()
            .any(|t| t.view.as_deref() == Some(VIEW_CONTAINERS))
        {
            let tab = Tab::from_view(VIEW_CONTAINERS);
            let tab_id = tab.id;
            ws.tabs.insert(0, tab);
            if ws.active_tab.is_none() {
                ws.active_tab = Some(tab_id);
            }
        }
        return inner.workspaces[idx].id;
    }
    let tab = Tab::from_view(VIEW_CONTAINERS);
    let workspace = Workspace {
        id: Uuid::new_v4(),
        name: DOCKER_WORKSPACE_NAME.to_string(),
        repo_root: None,
        color: None,
        group: None,
        kind: WorkspaceKind::Docker,
        active_tab: Some(tab.id),
        tabs: vec![tab],
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

pub fn workspaces_to_rows(workspaces: &[Workspace]) -> LayoutRows {
    let mut rows = LayoutRows::default();
    for (wi, ws) in workspaces.iter().enumerate() {
        let ws_id = ws.id.to_string();
        rows.workspaces.push(WorkspaceRow {
            id: ws_id.clone(),
            name: ws.name.clone(),
            repo_root: ws.repo_root.clone(),
            color: ws.color.clone(),
            group_name: ws.group.clone(),
            kind: Some(ws.kind.as_str().to_string()),
            position: wi as i64,
            active_tab: ws.active_tab.map(|id| id.to_string()),
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
    match &row.split {
        None => {
            let session = row.session_id.as_deref()?;
            Some(PaneNode::Leaf {
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
            let tabs: Vec<Tab> = tab_rows
                .into_iter()
                .filter_map(|t| build_tab(t, &rows.panes, valid))
                .collect();
            let active_tab = w
                .active_tab
                .as_deref()
                .and_then(|s| Uuid::parse_str(s).ok())
                .filter(|id| tabs.iter().any(|t| t.id == *id))
                .or_else(|| tabs.first().map(|t| t.id));
            let created_at = DateTime::parse_from_rfc3339(&w.created_at)
                .ok()?
                .with_timezone(&Utc);
            Some(Workspace {
                id: ws_id,
                name: w.name.clone(),
                repo_root: w.repo_root.clone(),
                color: w.color.clone(),
                group: w.group_name.clone(),
                kind: WorkspaceKind::parse(w.kind.as_deref()),
                active_tab,
                tabs,
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
    fn docker_workspace_is_created_once_with_view_tab() {
        let mgr = manager();
        let first = mgr.docker_workspace().unwrap();
        let second = mgr.docker_workspace().unwrap();
        assert_eq!(first, second);
        let state = mgr.state();
        let ws = state.workspaces.iter().find(|w| w.id == first).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Docker);
        assert_eq!(ws.name, DOCKER_WORKSPACE_NAME);
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.tabs[0].view.as_deref(), Some(VIEW_CONTAINERS));
        assert!(ws.tabs[0].root.is_none());
    }

    #[test]
    fn open_dashboard_activates_docker_workspace_and_view_tab() {
        let mgr = manager();
        ws(&mgr);
        let docker_id = mgr.docker_workspace().unwrap();
        let session = sid();
        let logs_tab = mgr.create_tab(session, Some(docker_id)).unwrap();
        mgr.open_docker_dashboard().unwrap();
        let state = mgr.state();
        assert_eq!(state.active_workspace, Some(docker_id));
        let ws = state.workspaces.iter().find(|w| w.id == docker_id).unwrap();
        assert_ne!(ws.active_tab, Some(logs_tab));
        let active = ws
            .tabs
            .iter()
            .find(|t| Some(t.id) == ws.active_tab)
            .unwrap();
        assert_eq!(active.view.as_deref(), Some(VIEW_CONTAINERS));
    }

    #[test]
    fn closing_view_tab_closes_docker_workspace_and_ensure_recreates_it() {
        let mgr = manager();
        let docker_id = mgr.docker_workspace().unwrap();
        let tab = mgr.state().workspaces[0].tabs[0].id;
        let bound = mgr.close_tab(tab).unwrap();
        assert!(bound.is_empty());
        assert!(mgr.state().workspaces.is_empty());
        let recreated = mgr.docker_workspace().unwrap();
        assert_ne!(recreated, docker_id);
        assert_eq!(mgr.state().workspaces[0].tabs.len(), 1);
    }

    #[test]
    fn view_tab_and_kind_round_trip_through_store() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.docker_workspace().unwrap();
        }
        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::new());
        let state = mgr.state();
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[0].kind, WorkspaceKind::Docker);
        assert_eq!(state.workspaces[0].tabs.len(), 1);
        assert_eq!(
            state.workspaces[0].tabs[0].view.as_deref(),
            Some(VIEW_CONTAINERS)
        );
        assert!(state.workspaces[0].tabs[0].root.is_none());
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
}
