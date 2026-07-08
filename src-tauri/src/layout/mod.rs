use std::collections::HashSet;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session::store::Store;
use crate::session::SessionId;

pub const EVENT_CHANGED: &str = "layout://changed";

const KEY_ACTIVE_TAB: &str = "layout.active_tab";
const MIN_RATIO: f64 = 0.1;
const MAX_RATIO: f64 = 0.9;

pub type TabId = Uuid;
pub type PaneId = Uuid;

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("tab não encontrada: {0}")]
    TabNotFound(TabId),
    #[error("pane não encontrado: {0}")]
    PaneNotFound(PaneId),
    #[error("pane não é um split: {0}")]
    NotASplit(PaneId),
    #[error("sessão já aberta em um pane: {0}")]
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
            PaneNode::Split { id, first, second, .. } => {
                *id == pane || first.contains(pane) || second.contains(pane)
            }
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
                id, ratio, first, second, ..
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

    #[cfg(test)]
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

#[derive(Debug, Clone, Serialize)]
pub struct Tab {
    pub id: TabId,
    pub title: Option<String>,
    pub active_pane: PaneId,
    pub root: PaneNode,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayoutState {
    pub tabs: Vec<Tab>,
    pub active_tab: Option<TabId>,
}

#[derive(Debug, Clone)]
pub struct TabRow {
    pub id: String,
    pub title: Option<String>,
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
    pub tabs: Vec<TabRow>,
    pub panes: Vec<PaneRow>,
}

struct Inner {
    tabs: Vec<Tab>,
    active_tab: Option<TabId>,
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
                tabs: Vec::new(),
                active_tab: None,
            }),
        }
    }

    pub fn load(&self, valid_sessions: &HashSet<SessionId>) {
        let rows = self.store.load_layout().unwrap_or_default();
        let mut tabs = rows_to_tabs(&rows);
        tabs = tabs
            .into_iter()
            .filter_map(|tab| {
                let root = tab.root.retain_sessions(valid_sessions)?;
                let active_pane = if root.contains(tab.active_pane) {
                    tab.active_pane
                } else {
                    root.first_leaf()
                };
                Some(Tab {
                    root,
                    active_pane,
                    ..tab
                })
            })
            .collect();

        let saved_active = self
            .store
            .get_setting(KEY_ACTIVE_TAB)
            .ok()
            .flatten()
            .and_then(|s| Uuid::parse_str(&s).ok());
        let active_tab = saved_active
            .filter(|id| tabs.iter().any(|t| t.id == *id))
            .or_else(|| tabs.first().map(|t| t.id));

        let mut inner = self.inner.write();
        inner.tabs = tabs;
        inner.active_tab = active_tab;
        drop(inner);
        let _ = self.persist();
    }

    pub fn state(&self) -> LayoutState {
        let inner = self.inner.read();
        LayoutState {
            tabs: inner.tabs.clone(),
            active_tab: inner.active_tab,
        }
    }

    pub fn create_tab(&self, session: SessionId) -> Result<TabId, LayoutError> {
        let mut inner = self.inner.write();
        if find_session_pane(&inner.tabs, session).is_some() {
            return Err(LayoutError::SessionAlreadyBound(session));
        }
        let leaf = PaneNode::Leaf {
            id: Uuid::new_v4(),
            session_id: session,
        };
        let tab = Tab {
            id: Uuid::new_v4(),
            title: None,
            active_pane: leaf.id(),
            root: leaf,
            created_at: Utc::now(),
        };
        let id = tab.id;
        inner.tabs.push(tab);
        inner.active_tab = Some(id);
        drop(inner);
        self.persist()?;
        Ok(id)
    }

    pub fn close_tab(&self, tab: TabId) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let idx = index_of(&inner.tabs, tab)?;
        inner.tabs.remove(idx);
        if inner.active_tab == Some(tab) {
            inner.active_tab = inner
                .tabs
                .get(idx.min(inner.tabs.len().saturating_sub(1)))
                .map(|t| t.id)
                .filter(|_| !inner.tabs.is_empty());
        }
        drop(inner);
        self.persist()
    }

    pub fn activate_tab(&self, tab: TabId) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        index_of(&inner.tabs, tab)?;
        inner.active_tab = Some(tab);
        drop(inner);
        self.persist()
    }

    pub fn move_tab(&self, tab: TabId, to: usize) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let idx = index_of(&inner.tabs, tab)?;
        let moved = inner.tabs.remove(idx);
        let to = to.min(inner.tabs.len());
        inner.tabs.insert(to, moved);
        drop(inner);
        self.persist()
    }

    pub fn open_session(&self, session: SessionId) -> Result<TabId, LayoutError> {
        let existing = {
            let inner = self.inner.read();
            find_session_pane(&inner.tabs, session)
        };
        if let Some((tab_id, pane_id)) = existing {
            let mut inner = self.inner.write();
            inner.active_tab = Some(tab_id);
            if let Ok(idx) = index_of(&inner.tabs, tab_id) {
                inner.tabs[idx].active_pane = pane_id;
            }
            drop(inner);
            self.persist()?;
            return Ok(tab_id);
        }
        self.create_tab(session)
    }

    pub fn split_pane(
        &self,
        pane: PaneId,
        kind: SplitKind,
        session: SessionId,
    ) -> Result<PaneId, LayoutError> {
        let mut inner = self.inner.write();
        if find_session_pane(&inner.tabs, session).is_some() {
            return Err(LayoutError::SessionAlreadyBound(session));
        }
        let tab = inner
            .tabs
            .iter_mut()
            .find(|t| t.root.contains(pane))
            .ok_or(LayoutError::PaneNotFound(pane))?;
        let tab_id = tab.id;
        let new_pane = tab
            .root
            .split_leaf(pane, kind, session)
            .ok_or(LayoutError::PaneNotFound(pane))?;
        tab.active_pane = new_pane;
        inner.active_tab = Some(tab_id);
        drop(inner);
        self.persist()?;
        Ok(new_pane)
    }

    pub fn close_pane(&self, pane: PaneId) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let idx = inner
            .tabs
            .iter()
            .position(|t| t.root.contains(pane))
            .ok_or(LayoutError::PaneNotFound(pane))?;
        let tab = inner.tabs[idx].clone();
        match tab.root.remove_leaf(pane) {
            Some(root) => {
                let active_pane = if root.contains(tab.active_pane) {
                    tab.active_pane
                } else {
                    root.first_leaf()
                };
                inner.tabs[idx] = Tab {
                    root,
                    active_pane,
                    ..tab
                };
            }
            None => {
                inner.tabs.remove(idx);
                if inner.active_tab == Some(tab.id) {
                    inner.active_tab = inner
                        .tabs
                        .get(idx.min(inner.tabs.len().saturating_sub(1)))
                        .map(|t| t.id)
                        .filter(|_| !inner.tabs.is_empty());
                }
            }
        }
        drop(inner);
        self.persist()
    }

    pub fn focus_pane(&self, pane: PaneId) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let tab = inner
            .tabs
            .iter_mut()
            .find(|t| t.root.contains(pane))
            .ok_or(LayoutError::PaneNotFound(pane))?;
        let tab_id = tab.id;
        tab.active_pane = pane;
        inner.active_tab = Some(tab_id);
        drop(inner);
        self.persist()
    }

    pub fn set_split_ratio(&self, pane: PaneId, ratio: f64) -> Result<(), LayoutError> {
        let mut inner = self.inner.write();
        let found = inner
            .tabs
            .iter_mut()
            .any(|t| t.root.set_ratio(pane, ratio));
        if !found {
            return Err(LayoutError::NotASplit(pane));
        }
        drop(inner);
        self.persist()
    }

    pub fn session_disposed(&self, session: SessionId) -> Result<(), LayoutError> {
        loop {
            let target = {
                let inner = self.inner.read();
                find_session_pane(&inner.tabs, session).map(|(_, pane)| pane)
            };
            match target {
                Some(pane) => self.close_pane(pane)?,
                None => return Ok(()),
            }
        }
    }

    fn persist(&self) -> Result<(), LayoutError> {
        let state = self.state();
        let rows = tabs_to_rows(&state.tabs);
        self.store.save_layout(&rows)?;
        self.store.set_setting(
            KEY_ACTIVE_TAB,
            &state
                .active_tab
                .map(|id| id.to_string())
                .unwrap_or_default(),
        )?;
        Ok(())
    }
}

fn index_of(tabs: &[Tab], tab: TabId) -> Result<usize, LayoutError> {
    tabs.iter()
        .position(|t| t.id == tab)
        .ok_or(LayoutError::TabNotFound(tab))
}

fn find_session_pane(tabs: &[Tab], session: SessionId) -> Option<(TabId, PaneId)> {
    tabs.iter().find_map(|t| {
        t.root
            .find_leaf_by_session(session)
            .map(|pane| (t.id, pane))
    })
}

fn push_pane_rows(node: &PaneNode, tab_id: &str, parent: Option<String>, position: Option<i64>, out: &mut Vec<PaneRow>) {
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

pub fn tabs_to_rows(tabs: &[Tab]) -> LayoutRows {
    let mut rows = LayoutRows::default();
    for (i, tab) in tabs.iter().enumerate() {
        let tab_id = tab.id.to_string();
        rows.tabs.push(TabRow {
            id: tab_id.clone(),
            title: tab.title.clone(),
            position: i as i64,
            active_pane: Some(tab.active_pane.to_string()),
            created_at: tab.created_at.to_rfc3339(),
        });
        push_pane_rows(&tab.root, &tab_id, None, None, &mut rows.panes);
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

pub fn rows_to_tabs(rows: &LayoutRows) -> Vec<Tab> {
    let mut tabs: Vec<&TabRow> = rows.tabs.iter().collect();
    tabs.sort_by_key(|t| t.position);
    tabs.into_iter()
        .filter_map(|t| {
            let tab_id = Uuid::parse_str(&t.id).ok()?;
            let root_row = rows
                .panes
                .iter()
                .find(|p| p.tab_id == t.id && p.parent_id.is_none())?;
            let root = build_node(&root_row.id, &rows.panes)?;
            let active_pane = t
                .active_pane
                .as_deref()
                .and_then(|s| Uuid::parse_str(s).ok())
                .filter(|id| root.contains(*id))
                .unwrap_or_else(|| root.first_leaf());
            let created_at = DateTime::parse_from_rfc3339(&t.created_at)
                .ok()?
                .with_timezone(&Utc);
            Some(Tab {
                id: tab_id,
                title: t.title.clone(),
                active_pane,
                root,
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

    #[test]
    fn create_tab_activates_it() {
        let mgr = manager();
        let s = sid();
        let tab = mgr.create_tab(s).unwrap();
        let state = mgr.state();
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.active_tab, Some(tab));
        assert!(matches!(state.tabs[0].root, PaneNode::Leaf { session_id, .. } if session_id == s));
    }

    #[test]
    fn create_tab_rejects_already_bound_session() {
        let mgr = manager();
        let s = sid();
        mgr.create_tab(s).unwrap();
        assert!(matches!(
            mgr.create_tab(s).unwrap_err(),
            LayoutError::SessionAlreadyBound(_)
        ));
    }

    #[test]
    fn split_pane_replaces_leaf_and_focuses_new_pane() {
        let mgr = manager();
        mgr.create_tab(sid()).unwrap();
        let root_pane = mgr.state().tabs[0].active_pane;

        let new_pane = mgr.split_pane(root_pane, SplitKind::V, sid()).unwrap();
        let state = mgr.state();
        let tab = &state.tabs[0];
        assert_eq!(tab.active_pane, new_pane);
        match &tab.root {
            PaneNode::Split { split, ratio, first, second, .. } => {
                assert_eq!(*split, SplitKind::V);
                assert!((ratio - 0.5).abs() < f64::EPSILON);
                assert_eq!(first.id(), root_pane);
                assert_eq!(second.id(), new_pane);
            }
            other => panic!("esperava split, veio {other:?}"),
        }
    }

    #[test]
    fn close_pane_promotes_sibling_and_refocuses() {
        let mgr = manager();
        mgr.create_tab(sid()).unwrap();
        let first_pane = mgr.state().tabs[0].active_pane;
        let second_pane = mgr.split_pane(first_pane, SplitKind::H, sid()).unwrap();

        mgr.close_pane(second_pane).unwrap();
        let state = mgr.state();
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.tabs[0].root.id(), first_pane);
        assert_eq!(state.tabs[0].active_pane, first_pane);
    }

    #[test]
    fn closing_last_pane_closes_the_tab() {
        let mgr = manager();
        let tab = mgr.create_tab(sid()).unwrap();
        let pane = mgr.state().tabs[0].active_pane;
        mgr.close_pane(pane).unwrap();
        let state = mgr.state();
        assert!(state.tabs.is_empty());
        assert_ne!(state.active_tab, Some(tab));
        assert_eq!(state.active_tab, None);
    }

    #[test]
    fn open_session_focuses_existing_binding_instead_of_new_tab() {
        let mgr = manager();
        let s1 = sid();
        let t1 = mgr.create_tab(s1).unwrap();
        mgr.create_tab(sid()).unwrap();

        let opened = mgr.open_session(s1).unwrap();
        assert_eq!(opened, t1);
        let state = mgr.state();
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.active_tab, Some(t1));
    }

    #[test]
    fn open_session_creates_tab_when_unbound() {
        let mgr = manager();
        mgr.create_tab(sid()).unwrap();
        let s = sid();
        mgr.open_session(s).unwrap();
        assert_eq!(mgr.state().tabs.len(), 2);
    }

    #[test]
    fn session_disposed_removes_every_binding() {
        let mgr = manager();
        let s = sid();
        mgr.create_tab(s).unwrap();
        let other = sid();
        let pane = mgr.state().tabs[0].active_pane;
        mgr.split_pane(pane, SplitKind::V, other).unwrap();

        mgr.session_disposed(s).unwrap();
        let state = mgr.state();
        assert_eq!(state.tabs.len(), 1);
        let mut sessions = Vec::new();
        state.tabs[0].root.leaf_sessions(&mut sessions);
        assert_eq!(sessions, vec![other]);
    }

    #[test]
    fn ratio_is_clamped() {
        let mgr = manager();
        mgr.create_tab(sid()).unwrap();
        let pane = mgr.state().tabs[0].active_pane;
        mgr.split_pane(pane, SplitKind::V, sid()).unwrap();
        let split_id = mgr.state().tabs[0].root.id();

        mgr.set_split_ratio(split_id, 0.01).unwrap();
        match &mgr.state().tabs[0].root {
            PaneNode::Split { ratio, .. } => assert!((ratio - MIN_RATIO).abs() < f64::EPSILON),
            _ => panic!("esperava split"),
        }

        assert!(matches!(
            mgr.set_split_ratio(pane, 0.5).unwrap_err(),
            LayoutError::NotASplit(_)
        ));
    }

    #[test]
    fn move_tab_reorders() {
        let mgr = manager();
        let t1 = mgr.create_tab(sid()).unwrap();
        let t2 = mgr.create_tab(sid()).unwrap();
        let t3 = mgr.create_tab(sid()).unwrap();

        mgr.move_tab(t3, 0).unwrap();
        let ids: Vec<TabId> = mgr.state().tabs.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![t3, t1, t2]);
    }

    #[test]
    fn close_tab_moves_activation_to_neighbor() {
        let mgr = manager();
        let t1 = mgr.create_tab(sid()).unwrap();
        let t2 = mgr.create_tab(sid()).unwrap();
        assert_eq!(mgr.state().active_tab, Some(t2));

        mgr.close_tab(t2).unwrap();
        assert_eq!(mgr.state().active_tab, Some(t1));
    }

    #[test]
    fn nested_tree_round_trips_through_rows() {
        let mgr = manager();
        mgr.create_tab(sid()).unwrap();
        let p1 = mgr.state().tabs[0].active_pane;
        let p2 = mgr.split_pane(p1, SplitKind::V, sid()).unwrap();
        mgr.split_pane(p2, SplitKind::H, sid()).unwrap();

        let before = mgr.state();
        let rows = tabs_to_rows(&before.tabs);
        let rebuilt = rows_to_tabs(&rows);
        assert_eq!(rebuilt.len(), 1);
        assert_eq!(rebuilt[0].root, before.tabs[0].root);
        assert_eq!(rebuilt[0].active_pane, before.tabs[0].active_pane);
    }

    #[test]
    fn load_persists_and_restores_from_store() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let s1 = sid();
        let s2 = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            let tab = mgr.create_tab(s1).unwrap();
            let pane = mgr.state().tabs[0].active_pane;
            mgr.split_pane(pane, SplitKind::H, s2).unwrap();
            mgr.activate_tab(tab).unwrap();
        }

        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::from([s1, s2]));
        let state = mgr.state();
        assert_eq!(state.tabs.len(), 1);
        assert!(matches!(state.tabs[0].root, PaneNode::Split { .. }));
    }

    #[test]
    fn load_drops_orphan_leaves_and_collapses_splits() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let alive = sid();
        let dead = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_tab(alive).unwrap();
            let pane = mgr.state().tabs[0].active_pane;
            mgr.split_pane(pane, SplitKind::V, dead).unwrap();
            mgr.create_tab(dead).unwrap_err();
        }

        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::from([alive]));
        let state = mgr.state();
        assert_eq!(state.tabs.len(), 1);
        match &state.tabs[0].root {
            PaneNode::Leaf { session_id, .. } => assert_eq!(*session_id, alive),
            other => panic!("esperava leaf colapsado, veio {other:?}"),
        }
        assert_eq!(state.tabs[0].active_pane, state.tabs[0].root.id());
    }

    #[test]
    fn load_drops_tab_whose_sessions_all_died() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let dead = sid();
        {
            let mgr = LayoutManager::new(Arc::clone(&store));
            mgr.create_tab(dead).unwrap();
        }
        let mgr = LayoutManager::new(Arc::clone(&store));
        mgr.load(&HashSet::new());
        let state = mgr.state();
        assert!(state.tabs.is_empty());
        assert_eq!(state.active_tab, None);
    }
}
