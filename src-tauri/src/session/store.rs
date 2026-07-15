use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection};

use crate::layout::{LayoutRows, PaneRow, TabRow, WorkspaceRow};
use crate::session::redact::redact;
use crate::session::{Session, SessionId, SessionKind, SessionStatus};
use crate::ssh::{Host, HostGroup};
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
CREATE TABLE IF NOT EXISTS setup_consents (
    repo_root TEXT NOT NULL,
    script_hash TEXT NOT NULL,
    allowed INTEGER NOT NULL,
    decided_at TEXT NOT NULL,
    PRIMARY KEY (repo_root, script_hash)
);
CREATE TABLE IF NOT EXISTS config_consents (
    repo_root TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    allowed INTEGER NOT NULL,
    decided_at TEXT NOT NULL,
    PRIMARY KEY (repo_root, config_hash)
);
CREATE TABLE IF NOT EXISTS approval_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    command TEXT NOT NULL,
    cwd TEXT,
    risk TEXT NOT NULL,
    decision TEXT NOT NULL,
    requested_at_ms INTEGER NOT NULL,
    resolved_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    name_locked INTEGER,
    repo_root TEXT,
    color TEXT,
    group_name TEXT,
    kind TEXT,
    position INTEGER NOT NULL,
    active_tab TEXT,
    side_view TEXT,
    side_ratio REAL,
    side_expanded INTEGER,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tabs (
    id TEXT PRIMARY KEY,
    workspace_id TEXT,
    title TEXT,
    view TEXT,
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
CREATE TABLE IF NOT EXISTS host_group (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    color TEXT,
    notes TEXT,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS host (
    id TEXT PRIMARY KEY,
    alias TEXT NOT NULL UNIQUE,
    hostname TEXT NOT NULL,
    port INTEGER,
    username TEXT,
    identity_file TEXT,
    proxy_jump TEXT,
    group_id TEXT REFERENCES host_group(id) ON DELETE SET NULL,
    color TEXT,
    notes TEXT,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    last_connected_at TEXT
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
        let _ = conn.execute("ALTER TABLE tabs ADD COLUMN view TEXT", []);
        let _ = conn.execute("ALTER TABLE workspaces ADD COLUMN color TEXT", []);
        let _ = conn.execute("ALTER TABLE workspaces ADD COLUMN group_name TEXT", []);
        let _ = conn.execute("ALTER TABLE workspaces ADD COLUMN kind TEXT", []);
        let _ = conn.execute("ALTER TABLE workspaces ADD COLUMN side_view TEXT", []);
        let _ = conn.execute("ALTER TABLE workspaces ADD COLUMN side_ratio REAL", []);
        let _ = conn.execute(
            "ALTER TABLE workspaces ADD COLUMN side_expanded INTEGER",
            [],
        );
        let _ = conn.execute("ALTER TABLE workspaces ADD COLUMN name_locked INTEGER", []);
        // Sem o cwd não há como reabrir a sessão na mesma pasta: o PTY morre com o
        // app e o pane fica órfão. É o que faz a tab sumir no reopen (#50).
        let _ = conn.execute("ALTER TABLE sessions ADD COLUMN cwd TEXT", []);
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
        let cwd = s.cwd.as_ref().map(|p| p.to_string_lossy().into_owned());

        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO sessions (id, kind, title, repo_root, worktree, status, created_at, cwd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                kind = ?2, title = ?3, repo_root = ?4, worktree = ?5, status = ?6, cwd = ?8",
            params![
                s.id.to_string(),
                kind,
                s.title,
                repo_root,
                worktree,
                status,
                s.created_at.to_rfc3339(),
                cwd,
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

    pub fn upsert_host(&self, h: &Host) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO host (id, alias, hostname, port, username, identity_file, proxy_jump, group_id, color, notes, position, created_at, last_connected_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET
                alias = ?2, hostname = ?3, port = ?4, username = ?5, identity_file = ?6,
                proxy_jump = ?7, group_id = ?8, color = ?9, notes = ?10, position = ?11,
                last_connected_at = ?13",
            params![
                h.id,
                h.alias,
                h.hostname,
                h.port,
                h.username,
                h.identity_file,
                h.proxy_jump,
                h.group_id,
                h.color,
                h.notes,
                h.position,
                h.created_at.to_rfc3339(),
                h.last_connected_at.map(|t| t.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    pub fn load_hosts(&self) -> Result<Vec<Host>, StoreError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, alias, hostname, port, username, identity_file, proxy_jump, group_id, color, notes, position, created_at, last_connected_at
             FROM host ORDER BY position, alias",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RawHost {
                id: row.get(0)?,
                alias: row.get(1)?,
                hostname: row.get(2)?,
                port: row.get(3)?,
                username: row.get(4)?,
                identity_file: row.get(5)?,
                proxy_jump: row.get(6)?,
                group_id: row.get(7)?,
                color: row.get(8)?,
                notes: row.get(9)?,
                position: row.get(10)?,
                created_at: row.get(11)?,
                last_connected_at: row.get(12)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?.into_host()?);
        }
        Ok(out)
    }

    pub fn remove_host(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM host WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn touch_host_connected(&self, id: &str, when: DateTime<Utc>) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE host SET last_connected_at = ?2 WHERE id = ?1",
            params![id, when.to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn upsert_host_group(&self, g: &HostGroup) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO host_group (id, name, color, notes, position, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                name = ?2, color = ?3, notes = ?4, position = ?5",
            params![
                g.id,
                g.name,
                g.color,
                g.notes,
                g.position,
                g.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn load_host_groups(&self) -> Result<Vec<HostGroup>, StoreError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, color, notes, position, created_at
             FROM host_group ORDER BY position, name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RawHostGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                notes: row.get(3)?,
                position: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?.into_group()?);
        }
        Ok(out)
    }

    pub fn remove_host_group(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM host_group WHERE id = ?1", params![id])?;
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

    pub fn setup_consent(
        &self,
        repo_root: &str,
        script_hash: &str,
    ) -> Result<Option<bool>, StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT allowed FROM setup_consents WHERE repo_root = ?1 AND script_hash = ?2",
            params![repo_root, script_hash],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .map_err(StoreError::from)
    }

    pub fn set_setup_consent(
        &self,
        repo_root: &str,
        script_hash: &str,
        allowed: bool,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO setup_consents (repo_root, script_hash, allowed, decided_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(repo_root, script_hash) DO UPDATE SET allowed = ?3, decided_at = ?4",
            params![repo_root, script_hash, allowed, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn config_consent(
        &self,
        repo_root: &str,
        config_hash: &str,
    ) -> Result<Option<bool>, StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT allowed FROM config_consents WHERE repo_root = ?1 AND config_hash = ?2",
            params![repo_root, config_hash],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .map_err(StoreError::from)
    }

    pub fn set_config_consent(
        &self,
        repo_root: &str,
        config_hash: &str,
        allowed: bool,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO config_consents (repo_root, config_hash, allowed, decided_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(repo_root, config_hash) DO UPDATE SET allowed = ?3, decided_at = ?4",
            params![repo_root, config_hash, allowed, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn insert_approval_history(&self, entry: &ApprovalHistoryEntry) -> Result<(), StoreError> {
        let command = redact(&entry.command);
        let cwd = entry.cwd.as_ref().map(|c| redact(c).into_owned());
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO approval_history
                 (session_id, command, cwd, risk, decision, requested_at_ms, resolved_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.session_id,
                command.as_ref(),
                cwd,
                entry.risk,
                entry.decision,
                entry.requested_at_ms,
                entry.resolved_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn list_approval_history(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ApprovalHistoryEntry>, StoreError> {
        let conn = self.conn.lock();
        let map_row = |row: &rusqlite::Row| {
            Ok(ApprovalHistoryEntry {
                session_id: row.get(0)?,
                command: row.get(1)?,
                cwd: row.get(2)?,
                risk: row.get(3)?,
                decision: row.get(4)?,
                requested_at_ms: row.get(5)?,
                resolved_at_ms: row.get(6)?,
            })
        };
        let entries = match session_id {
            Some(sid) => {
                let mut stmt = conn.prepare(
                    "SELECT session_id, command, cwd, risk, decision, requested_at_ms, resolved_at_ms
                     FROM approval_history WHERE session_id = ?1
                     ORDER BY id DESC LIMIT ?2",
                )?;
                let rows = stmt
                    .query_map(params![sid, limit as i64], map_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                rows
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT session_id, command, cwd, risk, decision, requested_at_ms, resolved_at_ms
                     FROM approval_history
                     ORDER BY id DESC LIMIT ?1",
                )?;
                let rows = stmt
                    .query_map(params![limit as i64], map_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                rows
            }
        };
        Ok(entries)
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
                     (id, name, name_locked, repo_root, color, group_name, kind, position,
                      active_tab, side_view, side_ratio, side_expanded, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    w.id,
                    w.name,
                    w.name_locked,
                    w.repo_root,
                    w.color,
                    w.group_name,
                    w.kind,
                    w.position,
                    w.active_tab,
                    w.side_view,
                    w.side_ratio,
                    w.side_expanded,
                    w.created_at
                ],
            )?;
        }
        for t in &rows.tabs {
            tx.execute(
                "INSERT INTO tabs (id, workspace_id, title, view, position, active_pane, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    t.id,
                    t.workspace_id,
                    t.title,
                    t.view,
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
            "SELECT id, name, name_locked, repo_root, color, group_name, kind, position,
                    active_tab, side_view, side_ratio, side_expanded, created_at
             FROM workspaces ORDER BY position",
        )?;
        let workspaces = stmt
            .query_map([], |row| {
                Ok(WorkspaceRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    name_locked: row.get(2)?,
                    repo_root: row.get(3)?,
                    color: row.get(4)?,
                    group_name: row.get(5)?,
                    kind: row.get(6)?,
                    position: row.get(7)?,
                    active_tab: row.get(8)?,
                    side_view: row.get(9)?,
                    side_ratio: row.get(10)?,
                    side_expanded: row.get(11)?,
                    created_at: row.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, title, view, position, active_pane, created_at
             FROM tabs ORDER BY position",
        )?;
        let tabs = stmt
            .query_map([], |row| {
                Ok(TabRow {
                    id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    title: row.get(2)?,
                    view: row.get(3)?,
                    position: row.get(4)?,
                    active_pane: row.get(5)?,
                    created_at: row.get(6)?,
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
            "SELECT id, kind, title, repo_root, worktree, status, created_at, cwd
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
                    cwd: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        raw.into_iter().map(RawSession::into_session).collect()
    }
}

pub struct ApprovalHistoryEntry {
    pub session_id: String,
    pub command: String,
    pub cwd: Option<String>,
    pub risk: String,
    pub decision: String,
    pub requested_at_ms: u64,
    pub resolved_at_ms: u64,
}

struct RawSession {
    id: String,
    kind: String,
    title: String,
    repo_root: Option<String>,
    worktree: Option<String>,
    status: String,
    created_at: String,
    cwd: Option<String>,
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
            attention: false,
            created_at,
            cwd: self.cwd.map(PathBuf::from),
        })
    }
}

struct RawHost {
    id: String,
    alias: String,
    hostname: String,
    port: Option<i64>,
    username: Option<String>,
    identity_file: Option<String>,
    proxy_jump: Option<String>,
    group_id: Option<String>,
    color: Option<String>,
    notes: Option<String>,
    position: i64,
    created_at: String,
    last_connected_at: Option<String>,
}

impl RawHost {
    fn into_host(self) -> Result<Host, StoreError> {
        Ok(Host {
            id: self.id,
            alias: self.alias,
            hostname: self.hostname,
            port: self.port.map(|p| p as u16),
            username: self.username,
            identity_file: self.identity_file,
            proxy_jump: self.proxy_jump,
            group_id: self.group_id,
            color: self.color,
            notes: self.notes,
            position: self.position,
            created_at: DateTime::parse_from_rfc3339(&self.created_at)?.with_timezone(&Utc),
            last_connected_at: match self.last_connected_at {
                Some(s) => Some(DateTime::parse_from_rfc3339(&s)?.with_timezone(&Utc)),
                None => None,
            },
        })
    }
}

struct RawHostGroup {
    id: String,
    name: String,
    color: Option<String>,
    notes: Option<String>,
    position: i64,
    created_at: String,
}

impl RawHostGroup {
    fn into_group(self) -> Result<HostGroup, StoreError> {
        Ok(HostGroup {
            id: self.id,
            name: self.name,
            color: self.color,
            notes: self.notes,
            position: self.position,
            created_at: DateTime::parse_from_rfc3339(&self.created_at)?.with_timezone(&Utc),
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
            attention: false,
            created_at: Utc::now(),
            cwd: Some(PathBuf::from("/repo/sub")),
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

    fn sample_host(alias: &str) -> Host {
        Host {
            id: uuid::Uuid::new_v4().to_string(),
            alias: alias.to_string(),
            hostname: format!("{alias}.example.com"),
            port: Some(22),
            username: Some("deploy".into()),
            identity_file: None,
            proxy_jump: None,
            group_id: None,
            color: None,
            notes: None,
            position: 0,
            created_at: Utc::now(),
            last_connected_at: None,
        }
    }

    #[test]
    fn round_trips_a_host() {
        let store = Store::open_in_memory().unwrap();
        let h = sample_host("web-01");
        store.upsert_host(&h).unwrap();
        let loaded = store.load_hosts().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].alias, "web-01");
        assert_eq!(loaded[0].port, Some(22));
        assert_eq!(loaded[0].username.as_deref(), Some("deploy"));
    }

    #[test]
    fn upsert_host_updates_in_place_then_remove_deletes() {
        let store = Store::open_in_memory().unwrap();
        let mut h = sample_host("db-01");
        store.upsert_host(&h).unwrap();
        h.hostname = "10.0.0.9".into();
        store.upsert_host(&h).unwrap();
        let loaded = store.load_hosts().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].hostname, "10.0.0.9");
        store.remove_host(&h.id).unwrap();
        assert!(store.load_hosts().unwrap().is_empty());
    }

    #[test]
    fn host_group_round_trips_and_fk_nulls_on_group_delete() {
        let store = Store::open_in_memory().unwrap();
        let g = HostGroup {
            id: uuid::Uuid::new_v4().to_string(),
            name: "prod".into(),
            color: Some("#f00".into()),
            notes: None,
            position: 0,
            created_at: Utc::now(),
        };
        store.upsert_host_group(&g).unwrap();
        let mut h = sample_host("web-01");
        h.group_id = Some(g.id.clone());
        store.upsert_host(&h).unwrap();
        assert_eq!(store.load_host_groups().unwrap().len(), 1);
        assert_eq!(
            store.load_hosts().unwrap()[0].group_id.as_deref(),
            Some(g.id.as_str())
        );
        store.remove_host_group(&g.id).unwrap();
        assert_eq!(store.load_hosts().unwrap()[0].group_id, None);
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

    #[test]
    fn setup_consent_is_scoped_to_repo_and_hash() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.setup_consent("/repo", "abc").unwrap(), None);

        store.set_setup_consent("/repo", "abc", true).unwrap();
        assert_eq!(store.setup_consent("/repo", "abc").unwrap(), Some(true));
        assert_eq!(
            store.setup_consent("/repo", "novo-hash").unwrap(),
            None,
            "script mudou: consent antigo não vale"
        );
        assert_eq!(store.setup_consent("/outro", "abc").unwrap(), None);

        store.set_setup_consent("/repo", "abc", false).unwrap();
        assert_eq!(store.setup_consent("/repo", "abc").unwrap(), Some(false));
    }

    #[test]
    fn config_consent_is_scoped_to_repo_and_hash() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.config_consent("/repo", "abc").unwrap(), None);

        store.set_config_consent("/repo", "abc", true).unwrap();
        assert_eq!(store.config_consent("/repo", "abc").unwrap(), Some(true));
        assert_eq!(
            store.config_consent("/repo", "novo-hash").unwrap(),
            None,
            "config mudou: consent antigo não vale"
        );
        assert_eq!(store.config_consent("/outro", "abc").unwrap(), None);

        store.set_config_consent("/repo", "abc", false).unwrap();
        assert_eq!(store.config_consent("/repo", "abc").unwrap(), Some(false));
    }

    fn approval(session_id: &str, command: &str, requested_at_ms: u64) -> ApprovalHistoryEntry {
        ApprovalHistoryEntry {
            session_id: session_id.to_string(),
            command: command.to_string(),
            cwd: Some("/repo".to_string()),
            risk: "red".to_string(),
            decision: "approved".to_string(),
            requested_at_ms,
            resolved_at_ms: requested_at_ms + 10,
        }
    }

    #[test]
    fn approval_history_round_trips() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_approval_history(&approval("s1", "git push", 100))
            .unwrap();

        let list = store.list_approval_history(None, 10).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].session_id, "s1");
        assert_eq!(list[0].command, "git push");
        assert_eq!(list[0].cwd, Some("/repo".to_string()));
        assert_eq!(list[0].risk, "red");
        assert_eq!(list[0].decision, "approved");
        assert_eq!(list[0].requested_at_ms, 100);
        assert_eq!(list[0].resolved_at_ms, 110);
    }

    #[test]
    fn approval_history_is_filtered_by_session() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_approval_history(&approval("s1", "cmd a", 100))
            .unwrap();
        store
            .insert_approval_history(&approval("s2", "cmd b", 101))
            .unwrap();

        let only_s1 = store.list_approval_history(Some("s1"), 10).unwrap();
        assert_eq!(only_s1.len(), 1);
        assert_eq!(only_s1[0].session_id, "s1");
        assert_eq!(only_s1[0].command, "cmd a");
    }

    #[test]
    fn approval_history_respects_limit_and_most_recent_first() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_approval_history(&approval("s1", "first", 100))
            .unwrap();
        store
            .insert_approval_history(&approval("s1", "second", 200))
            .unwrap();
        store
            .insert_approval_history(&approval("s1", "third", 300))
            .unwrap();

        let list = store.list_approval_history(None, 2).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].command, "third");
        assert_eq!(list[1].command, "second");
    }

    #[test]
    fn approval_history_redacts_secrets_before_persisting() {
        let store = Store::open_in_memory().unwrap();
        let mut entry = approval(
            "s1",
            "deploy --key sk-abcdef1234567890ABCDEFghijkl now",
            100,
        );
        entry.cwd = Some("/repo AKIAIOSFODNN7EXAMPLE dir".to_string());
        store.insert_approval_history(&entry).unwrap();

        let conn = store.conn.lock();
        let (command, cwd): (String, Option<String>) = conn
            .query_row(
                "SELECT command, cwd FROM approval_history WHERE session_id = ?1",
                params!["s1"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(!command.contains("sk-abcdef1234567890ABCDEFghijkl"));
        assert!(command.contains("[REDACTED]"));
        assert!(!cwd.as_deref().unwrap().contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(cwd.as_deref().unwrap().contains("[REDACTED]"));
    }
}
