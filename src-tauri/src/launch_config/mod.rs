use std::borrow::Cow;
use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::layout::{PaneRow, SplitKind, Workspace};
use crate::session::redact::redact;
use crate::session::SessionKind;
use crate::worktree::slugify;

pub type LaunchConfigId = Uuid;
pub type SlotId = Uuid;

const MIN_RATIO: f64 = 0.1;
const MAX_RATIO: f64 = 0.9;

#[derive(Debug, thiserror::Error)]
pub enum LaunchConfigError {
    #[error("nome da configuração vazio")]
    EmptyName,
    #[error("configuração sem slots")]
    NoSlots,
    #[error("nome de slot vazio")]
    EmptySlotName,
    #[error("nome de slot repetido: {0}")]
    DuplicateSlotName(String),
    #[error("slot fora da árvore: {0}")]
    UnknownSlot(SlotId),
    #[error("slot sem pane: {0}")]
    OrphanSlot(String),
    #[error("configuração não encontrada: {0}")]
    NotFound(LaunchConfigId),
    #[error("workspace sem repositório: launch config exige um repo")]
    NoRepoRoot,
    #[error("store: {0}")]
    Store(#[from] crate::session::store::StoreError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConfigSource {
    #[default]
    Local,
}

impl ConfigSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigSource::Local => "local",
        }
    }

    pub fn parse(_s: Option<&str>) -> Self {
        ConfigSource::Local
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Slot {
    pub id: SlotId,
    pub name: String,
    pub kind: SessionKind,
    #[serde(default)]
    pub cwd_rel: Option<String>,
    #[serde(default)]
    pub isolate: bool,
    #[serde(default)]
    pub initial_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SlotNode {
    Leaf {
        id: Uuid,
        slot_id: SlotId,
    },
    Split {
        id: Uuid,
        split: SplitKind,
        ratio: f64,
        first: Box<SlotNode>,
        second: Box<SlotNode>,
    },
}

impl SlotNode {
    fn slots(&self, out: &mut Vec<SlotId>) {
        match self {
            SlotNode::Leaf { slot_id, .. } => out.push(*slot_id),
            SlotNode::Split { first, second, .. } => {
                first.slots(out);
                second.slots(out);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigTab {
    pub id: Uuid,
    #[serde(default)]
    pub title: Option<String>,
    pub root: SlotNode,
}

#[derive(Debug, Clone, Serialize)]
pub struct LaunchConfig {
    pub id: LaunchConfigId,
    pub name: String,
    pub slug: String,
    pub repo_root: String,
    pub source: ConfigSource,
    pub slots: Vec<Slot>,
    pub tabs: Vec<ConfigTab>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LaunchConfigDraft {
    pub name: String,
    pub repo_root: String,
    pub slots: Vec<Slot>,
    pub tabs: Vec<ConfigTab>,
}

#[derive(Debug, Clone)]
pub struct ConfigRow {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub repo_root: String,
    pub source: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct SlotRow {
    pub id: String,
    pub config_id: String,
    pub name: String,
    pub kind: String,
    pub cwd_rel: Option<String>,
    pub isolate: i64,
    pub initial_prompt: Option<String>,
    pub position: i64,
}

#[derive(Debug, Clone)]
pub struct ConfigTabRow {
    pub id: String,
    pub config_id: String,
    pub title: Option<String>,
    pub position: i64,
}

#[derive(Debug, Clone)]
pub struct ConfigPaneRow {
    pub config_id: String,
    pub pane: PaneRow,
}

#[derive(Debug, Clone, Default)]
pub struct LaunchConfigRows {
    pub configs: Vec<ConfigRow>,
    pub slots: Vec<SlotRow>,
    pub tabs: Vec<ConfigTabRow>,
    pub panes: Vec<ConfigPaneRow>,
}

pub fn to_rows(config: &LaunchConfig) -> LaunchConfigRows {
    let id = config.id.to_string();
    let mut rows = LaunchConfigRows {
        configs: vec![ConfigRow {
            id: id.clone(),
            name: config.name.clone(),
            slug: config.slug.clone(),
            repo_root: config.repo_root.clone(),
            source: Some(config.source.as_str().to_string()),
            created_at: config.created_at.to_rfc3339(),
            updated_at: config.updated_at.to_rfc3339(),
        }],
        ..Default::default()
    };
    for (i, slot) in config.slots.iter().enumerate() {
        rows.slots.push(SlotRow {
            id: slot.id.to_string(),
            config_id: id.clone(),
            name: slot.name.clone(),
            kind: serde_json::to_string(&slot.kind).unwrap_or_else(|_| "null".into()),
            cwd_rel: slot.cwd_rel.clone(),
            isolate: slot.isolate as i64,
            initial_prompt: slot.initial_prompt.clone(),
            position: i as i64,
        });
    }
    for (i, tab) in config.tabs.iter().enumerate() {
        let tab_id = tab.id.to_string();
        rows.tabs.push(ConfigTabRow {
            id: tab_id.clone(),
            config_id: id.clone(),
            title: tab.title.clone(),
            position: i as i64,
        });
        let mut panes = Vec::new();
        tree_to_rows(&tab_id, &tab.root, &mut panes);
        for pane in panes {
            rows.panes.push(ConfigPaneRow {
                config_id: id.clone(),
                pane,
            });
        }
    }
    rows
}

pub fn from_rows(rows: &LaunchConfigRows) -> Vec<LaunchConfig> {
    let mut out = Vec::new();
    for cfg in &rows.configs {
        let Ok(id) = Uuid::parse_str(&cfg.id) else {
            continue;
        };
        let Ok(created_at) = DateTime::parse_from_rfc3339(&cfg.created_at) else {
            continue;
        };
        let Ok(updated_at) = DateTime::parse_from_rfc3339(&cfg.updated_at) else {
            continue;
        };

        let mut slot_rows: Vec<&SlotRow> = rows
            .slots
            .iter()
            .filter(|s| s.config_id == cfg.id)
            .collect();
        slot_rows.sort_by_key(|s| s.position);
        let slots: Vec<Slot> = slot_rows
            .into_iter()
            .filter_map(|s| {
                Some(Slot {
                    id: Uuid::parse_str(&s.id).ok()?,
                    name: s.name.clone(),
                    kind: serde_json::from_str(&s.kind).ok()?,
                    cwd_rel: s.cwd_rel.clone(),
                    isolate: s.isolate != 0,
                    initial_prompt: s.initial_prompt.clone(),
                })
            })
            .collect();
        if slots.is_empty() {
            continue;
        }

        let panes: Vec<PaneRow> = rows
            .panes
            .iter()
            .filter(|p| p.config_id == cfg.id)
            .map(|p| p.pane.clone())
            .collect();
        let mut tab_rows: Vec<&ConfigTabRow> =
            rows.tabs.iter().filter(|t| t.config_id == cfg.id).collect();
        tab_rows.sort_by_key(|t| t.position);
        let tabs: Vec<ConfigTab> = tab_rows
            .into_iter()
            .filter_map(|t| {
                let scoped: Vec<PaneRow> =
                    panes.iter().filter(|p| p.tab_id == t.id).cloned().collect();
                let root = scoped.iter().find(|p| p.parent_id.is_none())?;
                Some(ConfigTab {
                    id: Uuid::parse_str(&t.id).ok()?,
                    title: t.title.clone(),
                    root: rows_to_tree(&root.id, &scoped)?,
                })
            })
            .collect();
        if tabs.is_empty() {
            continue;
        }

        out.push(LaunchConfig {
            id,
            name: cfg.name.clone(),
            slug: cfg.slug.clone(),
            repo_root: cfg.repo_root.clone(),
            source: ConfigSource::parse(cfg.source.as_deref()),
            slots,
            tabs,
            created_at: created_at.with_timezone(&Utc),
            updated_at: updated_at.with_timezone(&Utc),
        });
    }
    out
}

pub fn slot_branch(config_slug: &str, slot_name: &str) -> String {
    format!("tyba/{}/{}", slugify(config_slug), slugify(slot_name))
}

pub fn prompt_looks_secret(prompt: &str) -> bool {
    matches!(redact(prompt), Cow::Owned(_))
}

pub fn secret_warnings(slots: &[Slot]) -> Vec<String> {
    slots
        .iter()
        .filter(|s| s.initial_prompt.as_deref().is_some_and(prompt_looks_secret))
        .map(|s| s.name.clone())
        .collect()
}

pub fn validate(draft: &LaunchConfigDraft) -> Result<(), LaunchConfigError> {
    if draft.name.trim().is_empty() {
        return Err(LaunchConfigError::EmptyName);
    }
    if draft.slots.is_empty() {
        return Err(LaunchConfigError::NoSlots);
    }
    let mut seen = HashSet::new();
    for slot in &draft.slots {
        let name = slugify(&slot.name);
        if slot.name.trim().is_empty() {
            return Err(LaunchConfigError::EmptySlotName);
        }
        if !seen.insert(name) {
            return Err(LaunchConfigError::DuplicateSlotName(slot.name.clone()));
        }
    }

    let mut placed = Vec::new();
    for tab in &draft.tabs {
        tab.root.slots(&mut placed);
    }
    let placed: HashSet<SlotId> = placed.into_iter().collect();
    let declared: HashSet<SlotId> = draft.slots.iter().map(|s| s.id).collect();
    for slot in &placed {
        if !declared.contains(slot) {
            return Err(LaunchConfigError::UnknownSlot(*slot));
        }
    }
    for slot in &draft.slots {
        if !placed.contains(&slot.id) {
            return Err(LaunchConfigError::OrphanSlot(slot.name.clone()));
        }
    }
    Ok(())
}

pub fn unique_slug(name: &str, taken: &HashSet<String>) -> String {
    let base = slugify(name);
    if !taken.contains(&base) {
        return base;
    }
    for n in 2..1000 {
        let candidate = format!("{base}-{n}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", Uuid::new_v4().simple())
}

pub fn tree_to_rows(tab_id: &str, node: &SlotNode, out: &mut Vec<PaneRow>) {
    push_rows(tab_id, node, None, None, out);
}

fn push_rows(
    tab_id: &str,
    node: &SlotNode,
    parent: Option<String>,
    position: Option<i64>,
    out: &mut Vec<PaneRow>,
) {
    match node {
        SlotNode::Leaf { id, slot_id } => out.push(PaneRow {
            id: id.to_string(),
            tab_id: tab_id.to_string(),
            parent_id: parent,
            split: None,
            ratio: None,
            position,
            session_id: Some(slot_id.to_string()),
        }),
        SlotNode::Split {
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
                split: Some(split_str(*split).to_string()),
                ratio: Some(*ratio),
                position,
                session_id: None,
            });
            push_rows(tab_id, first, Some(id_str.clone()), Some(0), out);
            push_rows(tab_id, second, Some(id_str), Some(1), out);
        }
    }
}

fn split_str(kind: SplitKind) -> &'static str {
    match kind {
        SplitKind::H => "h",
        SplitKind::V => "v",
    }
}

fn split_parse(s: &str) -> Option<SplitKind> {
    match s {
        "h" => Some(SplitKind::H),
        "v" => Some(SplitKind::V),
        _ => None,
    }
}

pub fn rows_to_tree(root_id: &str, rows: &[PaneRow]) -> Option<SlotNode> {
    let row = rows.iter().find(|r| r.id == root_id)?;
    let id = Uuid::parse_str(&row.id).ok()?;
    match &row.split {
        None => Some(SlotNode::Leaf {
            id,
            slot_id: Uuid::parse_str(row.session_id.as_deref()?).ok()?,
        }),
        Some(kind) => {
            let split = split_parse(kind)?;
            let mut children: Vec<&PaneRow> = rows
                .iter()
                .filter(|r| r.parent_id.as_deref() == Some(root_id))
                .collect();
            children.sort_by_key(|r| r.position.unwrap_or(0));
            if children.len() != 2 {
                return None;
            }
            Some(SlotNode::Split {
                id,
                split,
                ratio: row.ratio.unwrap_or(0.5).clamp(MIN_RATIO, MAX_RATIO),
                first: Box::new(rows_to_tree(&children[0].id, rows)?),
                second: Box::new(rows_to_tree(&children[1].id, rows)?),
            })
        }
    }
}

pub fn bind_slots_to_sessions(
    rows: &[PaneRow],
    bind: &dyn Fn(SlotId) -> Option<Uuid>,
) -> Vec<PaneRow> {
    rows.iter()
        .filter_map(|row| {
            let session_id = match row.session_id.as_deref() {
                None => None,
                Some(slot) => {
                    let slot = Uuid::parse_str(slot).ok()?;
                    Some(bind(slot)?.to_string())
                }
            };
            Some(PaneRow {
                id: row.id.clone(),
                tab_id: row.tab_id.clone(),
                parent_id: row.parent_id.clone(),
                split: row.split.clone(),
                ratio: row.ratio,
                position: row.position,
                session_id,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotSeed {
    pub name: String,
    pub repo_root: String,
    pub slots: Vec<Slot>,
    pub tabs: Vec<ConfigTab>,
}

pub fn snapshot_workspace(
    workspace: &Workspace,
    describe: &dyn Fn(Uuid) -> Option<(SessionKind, Option<String>, bool)>,
) -> Result<SnapshotSeed, LaunchConfigError> {
    let repo_root = workspace
        .repo_root
        .clone()
        .ok_or(LaunchConfigError::NoRepoRoot)?;

    let mut slots: Vec<Slot> = Vec::new();
    let mut tabs: Vec<ConfigTab> = Vec::new();
    let mut used_names: HashSet<String> = HashSet::new();

    for tab in &workspace.tabs {
        let Some(root) = &tab.root else { continue };
        let rows = crate::layout::pane_rows(root, &tab.id.to_string());
        let mut mapped = Vec::new();
        let mut ok = true;
        for row in &rows {
            let Some(session) = row.session_id.as_deref() else {
                mapped.push(row.clone());
                continue;
            };
            let Some(session) = Uuid::parse_str(session).ok() else {
                ok = false;
                break;
            };
            let Some((kind, cwd_rel, isolate)) = describe(session) else {
                ok = false;
                break;
            };
            let base = slot_name_for(&kind, cwd_rel.as_deref(), slots.len());
            let name = unique_slug(&base, &used_names);
            used_names.insert(name.clone());
            let slot = Slot {
                id: Uuid::new_v4(),
                name,
                kind,
                cwd_rel,
                isolate,
                initial_prompt: None,
            };
            let mut row = row.clone();
            row.session_id = Some(slot.id.to_string());
            slots.push(slot);
            mapped.push(row);
        }
        if !ok {
            continue;
        }
        let Some(root_row) = mapped.iter().find(|r| r.parent_id.is_none()) else {
            continue;
        };
        let Some(tree) = rows_to_tree(&root_row.id, &mapped) else {
            continue;
        };
        tabs.push(ConfigTab {
            id: Uuid::new_v4(),
            title: tab.title.clone(),
            root: tree,
        });
    }

    if slots.is_empty() {
        return Err(LaunchConfigError::NoSlots);
    }

    Ok(SnapshotSeed {
        name: workspace.name.clone(),
        repo_root,
        slots,
        tabs,
    })
}

fn slot_name_for(kind: &SessionKind, cwd_rel: Option<&str>, index: usize) -> String {
    if let Some(cwd) = cwd_rel.filter(|c| !c.is_empty() && *c != ".") {
        return cwd.trim_matches('/').replace('/', "-");
    }
    match kind {
        SessionKind::Agent { runner } => format!("{runner:?}").to_lowercase(),
        SessionKind::Shell => "shell".to_string(),
        _ => format!("slot-{}", index + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::AgentRunnerKind;

    fn slot(name: &str) -> Slot {
        Slot {
            id: Uuid::new_v4(),
            name: name.to_string(),
            kind: SessionKind::Shell,
            cwd_rel: None,
            isolate: false,
            initial_prompt: None,
        }
    }

    fn leaf(slot: &Slot) -> SlotNode {
        SlotNode::Leaf {
            id: Uuid::new_v4(),
            slot_id: slot.id,
        }
    }

    #[test]
    fn branch_is_deterministic_and_namespaced_by_config() {
        assert_eq!(
            slot_branch("tyba feature", "Core Rust"),
            "tyba/tyba-feature/core-rust"
        );
        assert_eq!(
            slot_branch("tyba feature", "Core Rust"),
            slot_branch("tyba-feature", "core-rust")
        );
        assert_ne!(slot_branch("api", "backend"), slot_branch("web", "backend"),);
    }

    #[test]
    fn round_trip_preserves_tree_shape_and_ratio() {
        let a = slot("a");
        let b = slot("b");
        let c = slot("c");
        let tree = SlotNode::Split {
            id: Uuid::new_v4(),
            split: SplitKind::V,
            ratio: 0.62,
            first: Box::new(leaf(&a)),
            second: Box::new(SlotNode::Split {
                id: Uuid::new_v4(),
                split: SplitKind::H,
                ratio: 0.25,
                first: Box::new(leaf(&b)),
                second: Box::new(leaf(&c)),
            }),
        };
        let mut rows = Vec::new();
        tree_to_rows("tab", &tree, &mut rows);
        let root = rows.iter().find(|r| r.parent_id.is_none()).unwrap();
        let back = rows_to_tree(&root.id, &rows).unwrap();
        assert_eq!(back, tree);
    }

    #[test]
    fn ratio_out_of_range_is_clamped_on_load() {
        let a = slot("a");
        let b = slot("b");
        let tree = SlotNode::Split {
            id: Uuid::new_v4(),
            split: SplitKind::H,
            ratio: 0.99,
            first: Box::new(leaf(&a)),
            second: Box::new(leaf(&b)),
        };
        let mut rows = Vec::new();
        tree_to_rows("tab", &tree, &mut rows);
        let root = rows.iter().find(|r| r.parent_id.is_none()).unwrap();
        let back = rows_to_tree(&root.id, &rows).unwrap();
        match back {
            SlotNode::Split { ratio, .. } => assert_eq!(ratio, MAX_RATIO),
            _ => panic!("esperava split"),
        }
    }

    #[test]
    fn split_with_one_child_does_not_rebuild() {
        let a = slot("a");
        let mut rows = Vec::new();
        tree_to_rows(
            "tab",
            &SlotNode::Split {
                id: Uuid::new_v4(),
                split: SplitKind::H,
                ratio: 0.5,
                first: Box::new(leaf(&a)),
                second: Box::new(leaf(&a)),
            },
            &mut rows,
        );
        let root_id = rows
            .iter()
            .find(|r| r.parent_id.is_none())
            .unwrap()
            .id
            .clone();
        rows.retain(|r| r.parent_id.is_none() || r.position == Some(0));
        assert!(rows_to_tree(&root_id, &rows).is_none());
    }

    #[test]
    fn binding_drops_panes_whose_slot_has_no_session() {
        let a = slot("a");
        let b = slot("b");
        let tree = SlotNode::Split {
            id: Uuid::new_v4(),
            split: SplitKind::V,
            ratio: 0.5,
            first: Box::new(leaf(&a)),
            second: Box::new(leaf(&b)),
        };
        let mut rows = Vec::new();
        tree_to_rows("tab", &tree, &mut rows);
        let session = Uuid::new_v4();
        let a_id = a.id;
        let bound = bind_slots_to_sessions(&rows, &move |slot| {
            if slot == a_id {
                Some(session)
            } else {
                None
            }
        });
        assert_eq!(bound.len(), 2);
        assert!(bound
            .iter()
            .any(|r| r.session_id.as_deref() == Some(session.to_string().as_str())));
    }

    #[test]
    fn validate_rejects_duplicate_slot_names_after_slugify() {
        let a = slot("Core Rust");
        let b = slot("core-rust");
        let draft = LaunchConfigDraft {
            name: "tyba".into(),
            repo_root: "/repo".into(),
            slots: vec![a.clone(), b],
            tabs: vec![ConfigTab {
                id: Uuid::new_v4(),
                title: None,
                root: leaf(&a),
            }],
        };
        assert!(matches!(
            validate(&draft),
            Err(LaunchConfigError::DuplicateSlotName(_))
        ));
    }

    #[test]
    fn validate_rejects_slot_without_pane() {
        let a = slot("a");
        let b = slot("b");
        let draft = LaunchConfigDraft {
            name: "tyba".into(),
            repo_root: "/repo".into(),
            slots: vec![a.clone(), b],
            tabs: vec![ConfigTab {
                id: Uuid::new_v4(),
                title: None,
                root: leaf(&a),
            }],
        };
        assert!(matches!(
            validate(&draft),
            Err(LaunchConfigError::OrphanSlot(_))
        ));
    }

    #[test]
    fn validate_rejects_pane_pointing_to_unknown_slot() {
        let a = slot("a");
        let ghost = slot("ghost");
        let draft = LaunchConfigDraft {
            name: "tyba".into(),
            repo_root: "/repo".into(),
            slots: vec![a],
            tabs: vec![ConfigTab {
                id: Uuid::new_v4(),
                title: None,
                root: leaf(&ghost),
            }],
        };
        assert!(matches!(
            validate(&draft),
            Err(LaunchConfigError::UnknownSlot(_))
        ));
    }

    #[test]
    fn secret_in_prompt_is_flagged_not_redacted() {
        let mut s = slot("core");
        s.initial_prompt =
            Some("use a chave sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into());
        let warned = secret_warnings(std::slice::from_ref(&s));
        assert_eq!(warned, vec!["core".to_string()]);
        assert!(s.initial_prompt.unwrap().contains("sk-ant-api03"));
    }

    #[test]
    fn clean_prompt_is_not_flagged() {
        let mut s = slot("core");
        s.initial_prompt = Some("Você cuida do core Rust. Leia docs/ARCHITECTURE.md".into());
        assert!(secret_warnings(&[s]).is_empty());
    }

    fn config_with_tree(tree: SlotNode, slots: Vec<Slot>) -> LaunchConfig {
        let now = Utc::now();
        LaunchConfig {
            id: Uuid::new_v4(),
            name: "tyba: feature".into(),
            slug: "tyba-feature".into(),
            repo_root: "/repo".into(),
            source: ConfigSource::Local,
            slots,
            tabs: vec![ConfigTab {
                id: Uuid::new_v4(),
                title: Some("dev".into()),
                root: tree,
            }],
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn store_round_trip_preserves_config() {
        let store = crate::session::store::Store::open_in_memory().unwrap();
        let mut core = slot("core");
        core.isolate = true;
        core.cwd_rel = Some("src-tauri".into());
        core.initial_prompt = Some("cuida do core".into());
        core.kind = SessionKind::Agent {
            runner: AgentRunnerKind::ClaudeCode,
        };
        let shell = slot("shell");
        let tree = SlotNode::Split {
            id: Uuid::new_v4(),
            split: SplitKind::V,
            ratio: 0.7,
            first: Box::new(leaf(&core)),
            second: Box::new(leaf(&shell)),
        };
        let config = config_with_tree(tree, vec![core.clone(), shell.clone()]);

        store.upsert_launch_config(&to_rows(&config)).unwrap();
        let back = from_rows(&store.load_launch_configs().unwrap());

        assert_eq!(back.len(), 1);
        let back = &back[0];
        assert_eq!(back.id, config.id);
        assert_eq!(back.slug, "tyba-feature");
        assert_eq!(back.repo_root, "/repo");
        assert_eq!(back.slots.len(), 2);
        assert_eq!(back.tabs[0].root, config.tabs[0].root);
        let core_back = back.slots.iter().find(|s| s.name == "core").unwrap();
        assert!(core_back.isolate);
        assert_eq!(core_back.cwd_rel.as_deref(), Some("src-tauri"));
        assert_eq!(core_back.initial_prompt.as_deref(), Some("cuida do core"));
        assert!(matches!(
            core_back.kind,
            SessionKind::Agent {
                runner: AgentRunnerKind::ClaudeCode
            }
        ));
    }

    #[test]
    fn upsert_replaces_slots_instead_of_accumulating() {
        let store = crate::session::store::Store::open_in_memory().unwrap();
        let a = slot("a");
        let b = slot("b");
        let mut config = config_with_tree(
            SlotNode::Split {
                id: Uuid::new_v4(),
                split: SplitKind::H,
                ratio: 0.5,
                first: Box::new(leaf(&a)),
                second: Box::new(leaf(&b)),
            },
            vec![a.clone(), b],
        );
        store.upsert_launch_config(&to_rows(&config)).unwrap();

        config.slots = vec![a.clone()];
        config.tabs[0].root = leaf(&a);
        store.upsert_launch_config(&to_rows(&config)).unwrap();

        let back = from_rows(&store.load_launch_configs().unwrap());
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].slots.len(), 1);
        assert!(matches!(back[0].tabs[0].root, SlotNode::Leaf { .. }));
    }

    #[test]
    fn delete_removes_slots_and_panes_too() {
        let store = crate::session::store::Store::open_in_memory().unwrap();
        let a = slot("a");
        let config = config_with_tree(leaf(&a), vec![a.clone()]);
        store.upsert_launch_config(&to_rows(&config)).unwrap();
        store.delete_launch_config(&config.id.to_string()).unwrap();

        let rows = store.load_launch_configs().unwrap();
        assert!(rows.configs.is_empty());
        assert!(rows.slots.is_empty());
        assert!(rows.panes.is_empty());
        assert!(rows.tabs.is_empty());
    }

    #[test]
    fn unique_slug_suffixes_on_collision() {
        let mut taken = HashSet::new();
        taken.insert("tyba-feature".to_string());
        assert_eq!(unique_slug("tyba feature", &taken), "tyba-feature-2");
        taken.insert("tyba-feature-2".to_string());
        assert_eq!(unique_slug("tyba feature", &taken), "tyba-feature-3");
    }

    #[test]
    fn agent_slot_name_falls_back_to_runner() {
        let name = slot_name_for(
            &SessionKind::Agent {
                runner: AgentRunnerKind::ClaudeCode,
            },
            None,
            0,
        );
        assert_eq!(name, "claudecode");
    }

    #[test]
    fn slot_name_prefers_subfolder() {
        let name = slot_name_for(&SessionKind::Shell, Some("src-tauri/src"), 0);
        assert_eq!(name, "src-tauri-src");
    }
}
