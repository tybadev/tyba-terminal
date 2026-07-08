use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection};

use crate::layout::{LayoutRows, PaneRow, TabRow, WorkspaceRow};
use crate::session::redact::redact;
use crate::session::{Session, SessionId, SessionKind, SessionStatus};
use crate::worktree::Worktree;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    repo_root TEXT,
    worktree TEXT,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    scrollback TEXT
);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    repo_root TEXT,
    color TEXT,
    group_name TEXT,
    position INTEGER NOT NULL,
    active_tab TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tabs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT,
    title TEXT,
    position INTEGER NOT NULL,
    active_pane TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS panes (
    id TEXT PRIMARY KEY,
    tab_id TEXT NOT NULL,
    parent_id TEXT,
    split TEXT,
    ratio REAL,
    position INTEGER,
    session_id TEXT
);
";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("uuid: {0}")]
    Uuid(#[from] uuid::Error),
    #[error("time: {0}")]
    Time(#[from] chrono::ParseError),
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        let _ = conn.execute("ALTER TABLE tabs ADD COLUMN workspace_id TEXT", []);
        let _ = conn.execute("ALTER TABLE workspaces ADD COLUMN color TEXT", []);
        let _ = conn.execute("ALTER TABLE workspaces ADD COLUMN group_name TEXT", []);
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn upsert_session(&self, s: &Session) -> Result<(), StoreError> {
        let kind = serde_json::to_string(&s.kind)?;
        let status = serde_json::to_string(&s.status)?;
        let worktree = match &s.worktree {
            Some(w) => Some(serde_json::to_string(w)?),
            None => None,
        };
        let repo_root = s
            .repo_root
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());

        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO sessions (id, kind, title, repo_root, worktree, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                kind = ?2, title = ?3, repo_root = ?4, worktree = ?5, status = ?6",
            params![
                s.id.to_string(),
                kind,
                s.title,
                repo_root,
                worktree,
                status,
                s.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn remove_session(&self, id: SessionId) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    pub fn save_scrollback(&self, id: SessionId, text: &str) -> Result<(), StoreError> {
        let redacted = redact(text);
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE sessions SET scrollback = ?2 WHERE id = ?1",
            params![id.to_string(), redacted.as_ref()],
        )?;
        Ok(())
    }

    pub fn load_scrollback(&self, id: SessionId) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock();
        let value = conn
            .query_row(
                "SELECT scrollback FROM sessions WHERE id = ?1",
                params![id.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(StoreError::from)?;
        Ok(value)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(StoreError::from)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn save_layout(&self, rows: &LayoutRows) -> Result<(), StoreError> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM panes", [])?;
        tx.execute("DELETE FROM tabs", [])?;
        tx.execute("DELETE FROM workspaces", [])?;
        for w in &rows.workspaces {
            tx.execute(
                "INSERT INTO workspaces
                     (id, name, repo_root, color, group_name, position, active_tab, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    w.id,
                    w.name,
                    w.repo_root,
                    w.color,
                    w.group_name,
                    w.position,
                    w.active_tab,
                    w.created_at
                ],
            )?;
        }
        for t in &rows.tabs {
            tx.execute(
                "INSERT INTO tabs (id, workspace_id, title, position, active_pane, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    t.id,
                    t.workspace_id,
                    t.title,
                    t.position,
                    t.active_pane,
                    t.created_at
                ],
            )?;
        }
        for p in &rows.panes {
            tx.execute(
                "INSERT INTO panes (id, tab_id, parent_id, split, ratio, position, session_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    p.id,
                    p.tab_id,
                    p.parent_id,
                    p.split,
                    p.ratio,
                    p.position,
                    p.session_id
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_layout(&self) -> Result<LayoutRows, StoreError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, repo_root, color, group_name, position, active_tab, created_at
             FROM workspaces ORDER BY position",
        )?;
        let workspaces = stmt
            .query_map([], |row| {
                Ok(WorkspaceRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    repo_root: row.get(2)?,
                    color: row.get(3)?,
                    group_name: row.get(4)?,
                    position: row.get(5)?,
                    active_tab: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, title, position, active_pane, created_at
             FROM tabs ORDER BY position",
        )?;
        let tabs = stmt
            .query_map([], |row| {
                Ok(TabRow {
                    id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    title: row.get(2)?,
                    position: row.get(3)?,
                    active_pane: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut stmt = conn.prepare(
            "SELECT id, tab_id, parent_id, split, ratio, position, session_id FROM panes",
        )?;
        let panes = stmt
            .query_map([], |row| {
                Ok(PaneRow {
                    id: row.get(0)?,
                    tab_id: row.get(1)?,
                    parent_id: row.get(2)?,
                    split: row.get(3)?,
                    ratio: row.get(4)?,
                    position: row.get(5)?,
                    session_id: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(LayoutRows {
            workspaces,
            tabs,
            panes,
        })
    }

    pub fn load_sessions(&self) -> Result<Vec<Session>, StoreError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, kind, title, repo_root, worktree, status, created_at
             FROM sessions ORDER BY created_at",
        )?;
        let raw = stmt
            .query_map([], |row| {
                Ok(RawSession {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    title: row.get(2)?,
                    repo_root: row.get(3)?,
                    worktree: row.get(4)?,
                    status: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        raw.into_iter().map(RawSession::into_session).collect()
    }
}

struct RawSession {
    id: String,
    kind: String,
    title: String,
    repo_root: Option<String>,
    worktree: Option<String>,
    status: String,
    created_at: String,
}

impl RawSession {
    fn into_session(self) -> Result<Session, StoreError> {
        let kind: SessionKind = serde_json::from_str(&self.kind)?;
        let status: SessionStatus = serde_json::from_str(&self.status)?;
        let worktree: Option<Worktree> = match self.worktree {
            Some(w) => Some(serde_json::from_str(&w)?),
            None => None,
        };
        let created_at: DateTime<Utc> =
            DateTime::parse_from_rfc3339(&self.created_at)?.with_timezone(&Utc);

        Ok(Session {
            id: SessionId::parse_str(&self.id)?,
            kind,
            title: self.title,
            repo_root: self.repo_root.map(PathBuf::from),
            worktree,
            status,
            created_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(title: &str) -> Session {
        Session {
            id: SessionId::new_v4(),
            kind: SessionKind::Shell,
            title: title.to_string(),
            repo_root: Some(PathBuf::from("/repo")),
            worktree: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn round_trips_a_session() {
        let store = Store::open_in_memory().unwrap();
        let s = sample("zsh");
        store.upsert_session(&s).unwrap();

        let loaded = store.load_sessions().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, s.id);
        assert_eq!(loaded[0].title, "zsh");
        assert!(matches!(loaded[0].status, SessionStatus::Running));
        assert_eq!(loaded[0].repo_root, Some(PathBuf::from("/repo")));
    }

    #[test]
    fn upsert_updates_existing_row_in_place() {
        let store = Store::open_in_memory().unwrap();
        let mut s = sample("zsh");
        store.upsert_session(&s).unwrap();

        s.status = SessionStatus::Exited { code: 0 };
        store.upsert_session(&s).unwrap();

        let loaded = store.load_sessions().unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(matches!(
            loaded[0].status,
            SessionStatus::Exited { code: 0 }
        ));
    }

    #[test]
    fn remove_deletes_the_session() {
        let store = Store::open_in_memory().unwrap();
        let s = sample("zsh");
        store.upsert_session(&s).unwrap();
        store.remove_session(s.id).unwrap();
        assert!(store.load_sessions().unwrap().is_empty());
    }

    #[test]
    fn settings_round_trip_and_overwrite() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_setting("theme.mode").unwrap(), None);

        store.set_setting("theme.mode", "light").unwrap();
        assert_eq!(
            store.get_setting("theme.mode").unwrap().as_deref(),
            Some("light")
        );

        store.set_setting("theme.mode", "system").unwrap();
        assert_eq!(
            store.get_setting("theme.mode").unwrap().as_deref(),
            Some("system")
        );
    }

    #[test]
    fn scrollback_is_redacted_before_persisting() {
        let store = Store::open_in_memory().unwrap();
        let s = sample("zsh");
        store.upsert_session(&s).unwrap();

        store
            .save_scrollback(s.id, "leaked AKIAIOSFODNN7EXAMPLE in output")
            .unwrap();

        let scrollback = store.load_scrollback(s.id).unwrap().unwrap();
        assert!(!scrollback.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(scrollback.contains("[REDACTED]"));
    }
}
