use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection};

use crate::launch_config::{ConfigPaneRow, ConfigRow, ConfigTabRow, LaunchConfigRows, SlotRow};
use crate::layout::{LayoutRows, PaneRow, TabRow, WorkspaceRow};
use crate::session::redact::redact;
use crate::session::{Session, SessionId, SessionKind, SessionStatus};
use crate::ssh::tunnel::{SessionTunnel, Tunnel, TunnelState};
use crate::ssh::{Host, HostGroup};
use crate::worktree::Worktree;
use uuid::Uuid;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    repo_root TEXT,
    worktree TEXT,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    cwd TEXT,
    agent_conversation_id TEXT
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
    launch_config_id TEXT,
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
CREATE TABLE IF NOT EXISTS launch_config (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    repo_root TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'local',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS launch_config_slot (
    id TEXT PRIMARY KEY,
    config_id TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    cwd_rel TEXT,
    isolate INTEGER NOT NULL DEFAULT 0,
    initial_prompt TEXT,
    position INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS launch_config_tab (
    id TEXT PRIMARY KEY,
    config_id TEXT NOT NULL,
    title TEXT,
    view TEXT,
    position INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS launch_config_pane (
    id TEXT PRIMARY KEY,
    config_id TEXT NOT NULL,
    tab_id TEXT NOT NULL,
    parent_id TEXT,
    split TEXT,
    ratio REAL,
    position INTEGER,
    slot_id TEXT
);
CREATE INDEX IF NOT EXISTS launch_config_slot_by_config ON launch_config_slot (config_id);
CREATE INDEX IF NOT EXISTS launch_config_pane_by_config ON launch_config_pane (config_id);
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
    tunnels TEXT,
    created_at TEXT NOT NULL,
    last_connected_at TEXT
);
CREATE TABLE IF NOT EXISTS session_tunnel (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    listen_port INTEGER NOT NULL,
    listen_host TEXT,
    target_host TEXT,
    target_port INTEGER,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS session_tunnel_by_session ON session_tunnel (session_id);
CREATE TABLE IF NOT EXISTS lsp_managed_consents (
    server_id TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    decided_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS command_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT,
    cwd TEXT,
    command TEXT NOT NULL,
    exit_code INTEGER,
    started_at_ms INTEGER NOT NULL,
    duration_ms INTEGER
);
CREATE INDEX IF NOT EXISTS command_history_by_time ON command_history (started_at_ms DESC);
CREATE INDEX IF NOT EXISTS command_history_by_cwd ON command_history (cwd, started_at_ms DESC);
CREATE TABLE IF NOT EXISTS block (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    command TEXT NOT NULL,
    exit_code INTEGER,
    started_at_ms INTEGER NOT NULL,
    finished_at_ms INTEGER NOT NULL,
    truncated INTEGER NOT NULL,
    bytes INTEGER NOT NULL,
    lines TEXT NOT NULL,
    alt_screen INTEGER NOT NULL DEFAULT 0,
    cwd TEXT
);
CREATE INDEX IF NOT EXISTS block_by_session ON block (session_id, id DESC);
CREATE TABLE IF NOT EXISTS block_checkpoint (
    session_id TEXT PRIMARY KEY,
    command TEXT NOT NULL,
    started_at_ms INTEGER NOT NULL,
    cols INTEGER NOT NULL,
    rows INTEGER NOT NULL,
    bytes BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS snippet (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    command TEXT NOT NULL,
    description TEXT,
    tags TEXT,
    created_at_ms INTEGER NOT NULL,
    uses INTEGER NOT NULL DEFAULT 0,
    last_used_at_ms INTEGER
);
";

/// Teto do histórico. Sem ele a tabela cresce com o uso e nunca encolhe: é a
/// única do banco alimentada por cada Enter que o usuário dá.
const COMMAND_HISTORY_CAP: i64 = 20_000;

/// Retenção de bloco por sessão, nas duas dimensões. Contagem sozinha não
/// protege de um bloco gigante; tamanho sozinho deixa a tabela crescer em
/// número. Poda os mais antigos primeiro.
const BLOCK_CAP_COUNT: i64 = 1_000;
const BLOCK_CAP_BYTES: i64 = 64 * 1024 * 1024;

/// Formato do payload de linhas. Versão desconhecida é IGNORADA na leitura —
/// um bloco gravado por uma versão futura não pode derrubar a sessão.
const BLOCK_VERSION: i64 = 1;

/// Quantos comandos distintos entram no ranking. O corte é por recência, então o
/// que fica de fora é o que ninguém procuraria de qualquer forma — e o fuzzy
/// roda sobre um conjunto limitado, não sobre a tabela inteira.
const HISTORY_CANDIDATES: i64 = 2_000;

/// `_` e `%` são curinga no LIKE, e caminho de repo pode conter os dois — sem
/// escapar, `/tmp/a_b` casaria com `/tmp/axb`.
fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

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

/// Versão de schema que este binário espera, em `PRAGMA user_version`.
///
/// 1 — linha de base. Estas colunas eram `ALTER TABLE ... ADD COLUMN` disparado
///     a cada abertura do banco e engolido com `let _ =`: em toda máquina que já
///     tinha usado o app, as catorze falhavam, todo boot, para nada.
/// 2 — `sessions.scrollback` sai. Ninguém lia a coluna, e output de terminal
///     parado no disco contraria o princípio #10 do CLAUDE.md.
/// 3 — `sessions.agent_conversation_id` entra: o id nativo da conversa do
///     agente, que é o que permite oferecer retomá-la depois de reabrir o app.
///     Degrau próprio, e não linha nova na [`BASELINE_COLUMNS`], porque a base
///     só roda em banco na versão 0 — quem já está na 2 (todo mundo que abriu o
///     app depois da migração anterior) nunca mais passa por ela.
const SCHEMA_VERSION: i64 = 3;

/// Colunas da versão 1, na ordem em que nasceram. Guardadas por `table_info` em
/// vez de tentadas às cegas porque os três estados possíveis convergem aqui: o
/// banco novo já as tem pelo `SCHEMA`, o banco de quem usava o app as ganhou
/// pelos ALTERs antigos, e só um banco de origem incerta chega sem alguma.
const BASELINE_COLUMNS: &[(&str, &str, &str)] = &[
    ("tabs", "workspace_id", "TEXT"),
    ("tabs", "view", "TEXT"),
    ("workspaces", "color", "TEXT"),
    ("workspaces", "group_name", "TEXT"),
    ("workspaces", "kind", "TEXT"),
    ("workspaces", "side_view", "TEXT"),
    ("workspaces", "side_ratio", "REAL"),
    ("workspaces", "side_expanded", "INTEGER"),
    ("workspaces", "name_locked", "INTEGER"),
    ("workspaces", "launch_config_id", "TEXT"),
    // Sem o cwd não há como reabrir a sessão na mesma pasta: o PTY morre com o
    // app e o pane fica órfão. É o que faz a tab sumir no reopen (#50).
    ("sessions", "cwd", "TEXT"),
    ("host", "tunnels", "TEXT"),
    ("block", "alt_screen", "INTEGER NOT NULL DEFAULT 0"),
    ("block", "cwd", "TEXT"),
];

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, StoreError> {
    // `PRAGMA` não aceita parâmetro ligado; `table` vem de constante do próprio
    // código, nunca de entrada externa.
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Migração por degrau, guardada por `PRAGMA user_version`.
///
/// Roda depois do `SCHEMA`, que só cria o que falta: num banco novo tudo já
/// nasceu na forma final e os degraus são no-op — o que os deixa passar é
/// justamente a checagem de coluna, não a versão.
///
/// # Um degrau que não pega não é motivo para perder o banco
///
/// Erro de degrau **não** sobe como `Err`. As instruções que este `migrate`
/// substituiu eram `let _ = conn.execute(...)`, engolidas de propósito; trocar
/// isso por `?` transformaria estados de banco que hoje só degradam — índice,
/// view ou trigger sobrando sobre `sessions.scrollback`, banco editado à mão,
/// restauração parcial de backup — em `Store::open` devolvendo `Err`. E o que
/// há do outro lado do `Err` é o `open_store` caindo para um banco **em
/// memória**: o usuário abre o app depois de atualizar, não encontra nenhuma
/// sessão, nenhum layout e nenhum histórico, e tudo que fizer nessa abertura
/// morre com o processo. Isto rodaria na primeira abertura de todo mundo.
///
/// Então o degrau que falha vira linha no relatório, e o banco do disco segue
/// sendo o banco. Fatal de verdade — não dá para ler `user_version`, não dá
/// para carimbar a versão nova — continua subindo como `Err`: aí o arquivo não
/// é um banco utilizável e cair para memória é a única saída que ainda tem app.
///
/// # A versão só anda até o degrau contíguo que pegou
///
/// Carimbar `SCHEMA_VERSION` com degrau pendente marcaria como concluída uma
/// migração que não aconteceu, e ela nunca mais seria tentada. Os dois degraus
/// **rodam** de todo jeito (o 2 tira output de terminal do disco — princípio
/// #10 —, e adiar isso porque o 1 falhou seria o pior dos dois mundos), mas a
/// versão gravada é a do último que pegou sem buraco antes dele. O custo de
/// ficar para trás é reexecutar um punhado de `PRAGMA table_info` por abertura,
/// até a próxima que der certo.
fn migrate(conn: &Connection) -> Result<Vec<String>, StoreError> {
    let from: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if from >= SCHEMA_VERSION {
        return Ok(Vec::new());
    }

    let mut skipped: Vec<String> = Vec::new();

    let baseline_applied = if from < 1 {
        let mut applied = true;
        for (table, column, kind) in BASELINE_COLUMNS {
            match has_column(conn, table, column) {
                Ok(true) => continue,
                Ok(false) => {}
                Err(e) => {
                    skipped.push(format!("{table}.{column}: {e}"));
                    applied = false;
                    continue;
                }
            }
            if let Err(e) = conn.execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {kind}"),
                [],
            ) {
                skipped.push(format!("{table}.{column}: {e}"));
                applied = false;
            }
        }
        applied
    } else {
        true
    };

    let scrollback_applied = if from < 2 {
        match has_column(conn, "sessions", "scrollback") {
            Ok(true) => match conn.execute("ALTER TABLE sessions DROP COLUMN scrollback", []) {
                Ok(_) => true,
                Err(e) => {
                    skipped.push(format!("sessions.scrollback: {e}"));
                    false
                }
            },
            Ok(false) => true,
            Err(e) => {
                skipped.push(format!("sessions.scrollback: {e}"));
                false
            }
        }
    } else {
        true
    };

    let conversation_applied = if from < 3 {
        match has_column(conn, "sessions", "agent_conversation_id") {
            Ok(true) => true,
            Ok(false) => match conn.execute(
                "ALTER TABLE sessions ADD COLUMN agent_conversation_id TEXT",
                [],
            ) {
                Ok(_) => true,
                Err(e) => {
                    skipped.push(format!("sessions.agent_conversation_id: {e}"));
                    false
                }
            },
            Err(e) => {
                skipped.push(format!("sessions.agent_conversation_id: {e}"));
                false
            }
        }
    } else {
        true
    };

    let reached = match (baseline_applied, scrollback_applied, conversation_applied) {
        (true, true, true) => SCHEMA_VERSION,
        (true, true, false) => 2,
        (true, false, _) => 1,
        (false, _, _) => from,
    };
    if reached > from {
        conn.pragma_update(None, "user_version", reached)?;
    }

    Ok(skipped)
}

pub struct Store {
    conn: Mutex<Connection>,
    /// Ver [`Store::degraded`].
    degraded: Option<String>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let span = crate::boot::Span::start("store.connection_open");
        let conn = Connection::open(path)?;
        span.end();

        conn.pragma_update(None, "journal_mode", "WAL")?;
        // O par correto do WAL. O default (`FULL`) fazia um fsync por commit —
        // e este banco leva commit pequeno o tempo todo (bloco, histórico,
        // layout). Com `NORMAL` o fsync acontece no checkpoint: um corte de
        // energia pode custar as últimas transações, nunca o banco.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // Sem timeout, qualquer escrita concorrente devolve SQLITE_BUSY na hora
        // — e o app tem várias threads gravando (histórico, blocos, sessões).
        conn.pragma_update(None, "busy_timeout", 5_000)?;
        // Negativo = KiB em vez de páginas. 16 MiB cobre o banco inteiro do uso
        // real (~2,4 MB) com folga, então leitura repetida não volta ao disco.
        conn.pragma_update(None, "cache_size", -16_000)?;
        // Explícito porque o default (1000 páginas) é implícito demais para uma
        // decisão que se paga: no disco de quem usa o app o WAL já foi visto com
        // 5,8 MB para um banco de 2,4 MB. Meio disso segura o arquivo pequeno
        // sem transformar cada commit em checkpoint.
        conn.pragma_update(None, "wal_autocheckpoint", 512)?;

        let span = crate::boot::Span::start("store.init");
        let store = Self::init(conn);
        span.end();
        store
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        let skipped = migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            degraded: (!skipped.is_empty()).then(|| {
                format!(
                    "a migração do banco de sessões não aplicou {} passo(s): {}",
                    skipped.len(),
                    skipped.join("; ")
                )
            }),
        })
    }

    /// O banco abriu, mas o schema não chegou onde este binário espera.
    ///
    /// `Some` significa que algum degrau da migração não pegou: as colunas que
    /// ele criaria podem faltar, e a consulta que depender delas vai falhar
    /// sozinha. Não é motivo para recusar o banco — é motivo para o usuário
    /// **saber**, porque o sintoma é lista vazia e lista vazia é indistinguível
    /// de "não tem nada". Quem lê isto é o `open_store`, que transforma a
    /// mensagem em falha de boot (`app://boot-failed` + `bootFailure`).
    pub fn degraded(&self) -> Option<&str> {
        self.degraded.as_deref()
    }

    /// Devolve o WAL ao tamanho do que ele realmente precisa.
    ///
    /// Checkpoint passivo reaproveita o arquivo mas nunca o encolhe: o WAL fica
    /// na maior marca que já atingiu, para sempre. `TRUNCATE` é o único que
    /// devolve o espaço — e por isso roda uma vez, na thread de boot, fora da
    /// main thread e fora do caminho de qualquer clique.
    pub fn checkpoint_truncate(&self) {
        let conn = self.conn.lock();
        let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
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
            "INSERT INTO sessions (id, kind, title, repo_root, worktree, status, created_at, cwd,
                                   agent_conversation_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                kind = ?2, title = ?3, repo_root = ?4, worktree = ?5, status = ?6, cwd = ?8,
                -- Nunca apaga um id já gravado: o upsert corre a cada troca de
                -- status, e a `Session` em memória pode ainda não ter lido o
                -- transcript. Quem esquece a conversa é o descarte da sessão.
                agent_conversation_id = COALESCE(?9, agent_conversation_id)",
            params![
                s.id.to_string(),
                kind,
                s.title,
                repo_root,
                worktree,
                status,
                s.created_at.to_rfc3339(),
                cwd,
                s.agent_conversation_id,
            ],
        )?;
        Ok(())
    }

    /// Descarta a sessão e tudo que ela gravou.
    ///
    /// Blocos e checkpoint moram em tabelas separadas, sem FK, e ficariam para
    /// trás se só a linha de `sessions` saísse. Quem descarta uma sessão quer
    /// que a saída dela suma — deixá-la no disco esperando a retenção contraria
    /// justamente o gesto (princípio #10).
    ///
    /// Numa transação porque as três apagam a mesma coisa: meio descarte é
    /// output órfão que ninguém mais tem como listar nem apagar.
    pub fn remove_session(&self, id: SessionId) -> Result<(), StoreError> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let key = id.to_string();
        tx.execute("DELETE FROM sessions WHERE id = ?1", params![key])?;
        tx.execute("DELETE FROM block WHERE session_id = ?1", params![key])?;
        tx.execute(
            "DELETE FROM block_checkpoint WHERE session_id = ?1",
            params![key],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn upsert_host(&self, h: &Host) -> Result<(), StoreError> {
        let tunnels = serde_json::to_string(&h.tunnels)?;
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO host (id, alias, hostname, port, username, identity_file, proxy_jump, group_id, color, notes, position, created_at, last_connected_at, tunnels)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(id) DO UPDATE SET
                alias = ?2, hostname = ?3, port = ?4, username = ?5, identity_file = ?6,
                proxy_jump = ?7, group_id = ?8, color = ?9, notes = ?10, position = ?11,
                last_connected_at = ?13, tunnels = ?14",
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
                tunnels,
            ],
        )?;
        Ok(())
    }

    pub fn load_hosts(&self) -> Result<Vec<Host>, StoreError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, alias, hostname, port, username, identity_file, proxy_jump, group_id, color, notes, position, created_at, last_connected_at, tunnels
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
                tunnels: row.get(13)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?.into_host()?);
        }
        Ok(out)
    }

    pub fn add_session_tunnel(&self, t: &SessionTunnel) -> Result<(), StoreError> {
        let kind = serde_json::to_string(&t.tunnel.kind)?;
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO session_tunnel (id, session_id, kind, listen_port, listen_host, target_host, target_port, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                t.id,
                t.session_id.to_string(),
                kind,
                t.tunnel.listen_port,
                t.tunnel.listen_host,
                t.tunnel.target_host,
                t.tunnel.target_port,
                t.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn load_session_tunnels(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<SessionTunnel>, StoreError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, kind, listen_port, listen_host, target_host, target_port, created_at
             FROM session_tunnel WHERE session_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![session_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u16>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<u16>>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, sid, kind, listen_port, listen_host, target_host, target_port, created_at) =
                row?;
            out.push(SessionTunnel {
                id,
                session_id: Uuid::parse_str(&sid)?,
                tunnel: Tunnel {
                    kind: serde_json::from_str(&kind)?,
                    listen_port,
                    listen_host,
                    target_host,
                    target_port,
                },
                state: TunnelState::Opening,
                created_at: DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&Utc),
            });
        }
        Ok(out)
    }

    pub fn remove_session_tunnel(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM session_tunnel WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn remove_session_tunnels(&self, session_id: SessionId) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM session_tunnel WHERE session_id = ?1",
            params![session_id.to_string()],
        )?;
        Ok(())
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

    /// Todas as preferências de uma vez. O mount do front lia dezesseis chaves,
    /// uma por `invoke`: paralelo do lado do JS, fila do lado do core, porque
    /// cada uma pegava o mesmo `Mutex<Connection>`.
    pub fn prefs(&self) -> Result<std::collections::HashMap<String, String>, StoreError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT key, value FROM settings WHERE key LIKE 'pref.%'")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (key, value) = row?;
            out.insert(key, value);
        }
        Ok(out)
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

    pub fn lsp_managed_consent(&self, server_id: &str) -> Result<bool, StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self.conn.lock();
        let found: Option<String> = conn
            .query_row(
                "SELECT version FROM lsp_managed_consents WHERE server_id = ?1",
                params![server_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    pub fn set_lsp_managed_consent(
        &self,
        server_id: &str,
        version: &str,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO lsp_managed_consents (server_id, version, decided_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(server_id) DO UPDATE SET version = ?2, decided_at = ?3",
            params![server_id, version, Utc::now().to_rfc3339()],
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

    /// Redige antes de gravar: `export TOKEN=sk-…` é o caso comum de linha de
    /// comando com secret, não o exótico (princípio #10).
    pub fn insert_command(&self, record: &crate::history::CommandRecord) -> Result<(), StoreError> {
        let command = redact(&record.command);
        let cwd = record.cwd.as_ref().map(|c| redact(c).into_owned());
        let conn = self.conn.lock();
        // Repetir o comando anterior não vira linha nova — `ls` três vezes
        // seguidas polui o ranking e não diz nada.
        let previous: Option<String> = conn
            .query_row(
                "SELECT command FROM command_history ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok();
        if previous.as_deref() == Some(command.as_ref()) {
            return Ok(());
        }
        conn.execute(
            "INSERT INTO command_history
                 (session_id, cwd, command, exit_code, started_at_ms, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                record.session_id,
                cwd,
                command.as_ref(),
                record.exit_code,
                record.started_at_ms,
                record.duration_ms,
            ],
        )?;
        conn.execute(
            "DELETE FROM command_history
             WHERE id <= (SELECT MAX(id) FROM command_history) - ?1",
            params![COMMAND_HISTORY_CAP],
        )?;
        Ok(())
    }

    /// Candidatos crus do histórico, agregados por comando. O ranking (fuzzy +
    /// frecência) fica em `history::frecency` — aqui só o que o SQL faz melhor.
    pub fn history_candidates(
        &self,
        cwd: Option<&str>,
        repo_root: Option<&str>,
    ) -> Result<Vec<crate::history::HistoryCandidate>, StoreError> {
        let repo_prefix = repo_root.map(|root| format!("{}/", root.trim_end_matches('/')));
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT command,
                    MAX(started_at_ms),
                    COUNT(*),
                    SUM(CASE WHEN exit_code = 0 THEN 1 ELSE 0 END),
                    MAX(CASE WHEN cwd IS NOT NULL AND cwd = ?1 THEN 1 ELSE 0 END),
                    MAX(CASE WHEN ?2 IS NOT NULL AND cwd IS NOT NULL
                              AND (cwd = ?3 OR cwd LIKE ?2 ESCAPE '\\')
                             THEN 1 ELSE 0 END),
                    MAX(cwd)
             FROM command_history
             GROUP BY command
             ORDER BY MAX(started_at_ms) DESC
             LIMIT ?4",
        )?;
        let like = repo_prefix
            .as_deref()
            .map(|prefix| format!("{}%", escape_like(prefix)));
        let rows = stmt
            .query_map(params![cwd, like, repo_root, HISTORY_CANDIDATES], |row| {
                Ok(crate::history::HistoryCandidate {
                    command: row.get(0)?,
                    last_used_at_ms: row.get(1)?,
                    uses: row.get::<_, i64>(2)?.max(0) as u32,
                    successes: row.get::<_, i64>(3)?.max(0) as u32,
                    in_cwd: row.get::<_, i64>(4)? != 0,
                    in_repo: row.get::<_, i64>(5)? != 0,
                    cwd: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Comandos distintos que começam com o prefixo, mais recentes primeiro.
    /// Alimenta a completação de subcomando e flag.
    pub fn history_with_prefix(&self, prefix: &str, limit: i64) -> Result<Vec<String>, StoreError> {
        if prefix.trim().is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT command, MAX(started_at_ms) AS last_ms
             FROM command_history
             WHERE command LIKE ?1 ESCAPE '\\'
             GROUP BY command
             ORDER BY last_ms DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![format!("{}%", escape_like(prefix)), limit], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn clear_command_history(&self, repo_root: Option<&str>) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        match repo_root {
            Some(root) => {
                let prefix = format!("{}/", root.trim_end_matches('/'));
                conn.execute(
                    "DELETE FROM command_history
                     WHERE cwd = ?1 OR cwd LIKE ?2 ESCAPE '\\'",
                    params![root, format!("{}%", escape_like(&prefix))],
                )?;
            }
            None => {
                conn.execute("DELETE FROM command_history", [])?;
            }
        }
        Ok(())
    }

    pub fn insert_block(&self, block: &crate::blocks::Block) -> Result<usize, StoreError> {
        let lines = serde_json::to_string(&block.lines)?;
        let bytes = lines.len() as i64;
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO block
                 (session_id, version, command, exit_code, started_at_ms,
                  finished_at_ms, truncated, bytes, lines, alt_screen, cwd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                block.session_id,
                BLOCK_VERSION,
                block.command,
                block.exit_code,
                block.started_at_ms,
                block.finished_at_ms,
                block.truncated as i64,
                bytes,
                lines,
                block.alt_screen,
                block.cwd,
            ],
        )?;

        // Poda por contagem E por tamanho: um `cat` de log enorme estoura o
        // segundo teto muito antes do primeiro. Roda fora do caminho do que o
        // usuário vê — o bloco já foi emitido antes desta função ser chamada.
        let pruned = conn.execute(
            "DELETE FROM block WHERE session_id = ?1 AND id NOT IN (
                 SELECT id FROM (
                     SELECT id,
                            ROW_NUMBER() OVER (ORDER BY id DESC) AS pos,
                            SUM(bytes) OVER (ORDER BY id DESC) AS running
                     FROM block WHERE session_id = ?1
                 )
                 WHERE pos <= ?2 AND running <= ?3
             )",
            params![block.session_id, BLOCK_CAP_COUNT, BLOCK_CAP_BYTES],
        )?;
        Ok(pruned)
    }

    /// Maior id já gravado, de qualquer sessão.
    ///
    /// É o piso do contador de ids vivos: o bloco emitido para a tela e o bloco
    /// lido de volta do disco convivem na mesma lista.
    pub fn max_block_id(&self) -> Result<u64, StoreError> {
        let conn = self.conn.lock();
        let max: Option<i64> = conn.query_row("SELECT MAX(id) FROM block", [], |row| row.get(0))?;
        Ok(max.unwrap_or(0).max(0) as u64)
    }

    /// Blocos de uma sessão, do mais antigo para o mais novo (ordem de leitura).
    pub fn list_blocks(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<crate::blocks::Block>, StoreError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, version, command, exit_code, started_at_ms,
                    finished_at_ms, truncated, lines, alt_screen, cwd
             FROM block WHERE session_id = ?1
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![session_id, limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i32>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, bool>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut blocks: Vec<crate::blocks::Block> = rows
            .into_iter()
            .filter(|row| row.2 == BLOCK_VERSION)
            .filter_map(|row| {
                let lines = serde_json::from_str(&row.8).ok()?;
                Some(crate::blocks::Block {
                    id: row.0 as u64,
                    session_id: row.1,
                    command: row.3,
                    exit_code: row.4,
                    started_at_ms: row.5,
                    finished_at_ms: row.6,
                    lines,
                    truncated: row.7.max(0) as usize,
                    alt_screen: row.9,
                    cwd: row.10,
                })
            })
            .collect();
        blocks.reverse();
        Ok(blocks)
    }

    /// Apaga os blocos de uma sessão viva — é o `clear`.
    ///
    /// Diferente do descarte da sessão (`remove_session`), que leva a linha da
    /// sessão junto: aqui ela continua, só sem o que já rolou.
    pub fn drop_blocks(&self, session_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM block WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Fotografia do comando em execução.
    ///
    /// Sem isto, um crash no meio de um `cargo build` de cinco minutos perde a
    /// saída inteira — o bloco só nasce no `133;D`. Uma linha por sessão, sempre
    /// substituída.
    pub fn save_checkpoint(
        &self,
        session_id: &str,
        command: &str,
        started_at_ms: i64,
        cols: u16,
        rows: u16,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO block_checkpoint
                 (session_id, command, started_at_ms, cols, rows, bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
                 command = ?2, started_at_ms = ?3, cols = ?4, rows = ?5, bytes = ?6",
            params![session_id, command, started_at_ms, cols, rows, bytes],
        )?;
        Ok(())
    }

    pub fn clear_checkpoint(&self, session_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM block_checkpoint WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Checkpoints órfãos viram blocos interrompidos e são consumidos.
    ///
    /// Rodado no boot: se existe checkpoint, o app morreu com um comando
    /// rodando — `exit_code` nulo é justamente o "não terminou".
    pub fn drain_checkpoints(&self) -> Result<usize, StoreError> {
        let rows: Vec<(String, String, i64, u16, u16, Vec<u8>)> = {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare(
                "SELECT session_id, command, started_at_ms, cols, rows, bytes
                 FROM block_checkpoint",
            )?;
            let mapped = stmt
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            mapped
        };
        let count = rows.len();
        for (session_id, command, started_at_ms, cols, rows_n, bytes) in rows {
            let (lines, truncated) = crate::blocks::extract_lines(&bytes, cols, rows_n);
            let block = crate::blocks::Block {
                id: 0,
                session_id: session_id.clone(),
                command,
                exit_code: None,
                // Checkpoint só existe fora de tela alternada, e não carrega
                // cwd: quem o gravou é a thread emitter, no meio do comando.
                alt_screen: false,
                cwd: None,
                started_at_ms,
                finished_at_ms: started_at_ms,
                lines,
                truncated,
            };
            self.insert_block(&block)?;
            self.clear_checkpoint(&session_id)?;
        }
        Ok(count)
    }

    pub fn list_snippets(&self) -> Result<Vec<crate::snippet::Snippet>, StoreError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, command, description, tags FROM snippet
             ORDER BY uses DESC, last_used_at_ms DESC, name ASC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let tags: Option<String> = row.get(4)?;
                Ok(crate::snippet::Snippet {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    command: row.get(2)?,
                    description: row.get(3)?,
                    tags: tags
                        .map(|raw| {
                            raw.split('\u{1f}')
                                .filter(|tag| !tag.is_empty())
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    source: crate::snippet::Source::Local,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn save_snippet(&self, snippet: &crate::snippet::Snippet) -> Result<(), StoreError> {
        let tags = snippet.tags.join("\u{1f}");
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO snippet (id, name, command, description, tags, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                 name = ?2, command = ?3, description = ?4, tags = ?5",
            params![
                snippet.id,
                snippet.name,
                snippet.command,
                snippet.description,
                tags,
                crate::approvals::now_ms() as i64,
            ],
        )?;
        Ok(())
    }

    pub fn delete_snippet(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM snippet WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn touch_snippet(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE snippet SET uses = uses + 1, last_used_at_ms = ?2 WHERE id = ?1",
            params![id, crate::approvals::now_ms() as i64],
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
                     (id, name, name_locked, repo_root, color, group_name, kind, position,
                      active_tab, side_view, side_ratio, side_expanded, created_at,
                      launch_config_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
                    w.created_at,
                    w.launch_config_id
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
                    active_tab, side_view, side_ratio, side_expanded, created_at,
                    launch_config_id
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
                    launch_config_id: row.get(13)?,
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

    pub fn upsert_launch_config(&self, rows: &LaunchConfigRows) -> Result<(), StoreError> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        for cfg in &rows.configs {
            tx.execute(
                "DELETE FROM launch_config_pane WHERE config_id = ?1",
                params![cfg.id],
            )?;
            tx.execute(
                "DELETE FROM launch_config_tab WHERE config_id = ?1",
                params![cfg.id],
            )?;
            tx.execute(
                "DELETE FROM launch_config_slot WHERE config_id = ?1",
                params![cfg.id],
            )?;
            tx.execute(
                "INSERT INTO launch_config (id, name, slug, repo_root, source, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                    name = ?2, slug = ?3, repo_root = ?4, source = ?5, updated_at = ?7",
                params![
                    cfg.id,
                    cfg.name,
                    cfg.slug,
                    cfg.repo_root,
                    cfg.source,
                    cfg.created_at,
                    cfg.updated_at
                ],
            )?;
        }
        for s in &rows.slots {
            tx.execute(
                "INSERT INTO launch_config_slot
                     (id, config_id, name, kind, cwd_rel, isolate, initial_prompt, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    s.id,
                    s.config_id,
                    s.name,
                    s.kind,
                    s.cwd_rel,
                    s.isolate,
                    s.initial_prompt,
                    s.position
                ],
            )?;
        }
        for t in &rows.tabs {
            tx.execute(
                "INSERT INTO launch_config_tab (id, config_id, title, position)
                 VALUES (?1, ?2, ?3, ?4)",
                params![t.id, t.config_id, t.title, t.position],
            )?;
        }
        for p in &rows.panes {
            tx.execute(
                "INSERT INTO launch_config_pane
                     (id, config_id, tab_id, parent_id, split, ratio, position, slot_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    p.pane.id,
                    p.config_id,
                    p.pane.tab_id,
                    p.pane.parent_id,
                    p.pane.split,
                    p.pane.ratio,
                    p.pane.position,
                    p.pane.session_id
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn load_launch_configs(&self) -> Result<LaunchConfigRows, StoreError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, slug, repo_root, source, created_at, updated_at
             FROM launch_config ORDER BY name",
        )?;
        let configs = stmt
            .query_map([], |row| {
                Ok(ConfigRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    slug: row.get(2)?,
                    repo_root: row.get(3)?,
                    source: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut stmt = conn.prepare(
            "SELECT id, config_id, name, kind, cwd_rel, isolate, initial_prompt, position
             FROM launch_config_slot ORDER BY position",
        )?;
        let slots = stmt
            .query_map([], |row| {
                Ok(SlotRow {
                    id: row.get(0)?,
                    config_id: row.get(1)?,
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    cwd_rel: row.get(4)?,
                    isolate: row.get(5)?,
                    initial_prompt: row.get(6)?,
                    position: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut stmt = conn.prepare(
            "SELECT id, config_id, title, position FROM launch_config_tab ORDER BY position",
        )?;
        let tabs = stmt
            .query_map([], |row| {
                Ok(ConfigTabRow {
                    id: row.get(0)?,
                    config_id: row.get(1)?,
                    title: row.get(2)?,
                    position: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut stmt = conn.prepare(
            "SELECT id, config_id, tab_id, parent_id, split, ratio, position, slot_id
             FROM launch_config_pane",
        )?;
        let panes = stmt
            .query_map([], |row| {
                Ok(ConfigPaneRow {
                    config_id: row.get(1)?,
                    pane: PaneRow {
                        id: row.get(0)?,
                        tab_id: row.get(2)?,
                        parent_id: row.get(3)?,
                        split: row.get(4)?,
                        ratio: row.get(5)?,
                        position: row.get(6)?,
                        session_id: row.get(7)?,
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(LaunchConfigRows {
            configs,
            slots,
            tabs,
            panes,
        })
    }

    pub fn delete_launch_config(&self, id: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM launch_config_pane WHERE config_id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM launch_config_tab WHERE config_id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM launch_config_slot WHERE config_id = ?1",
            params![id],
        )?;
        tx.execute("DELETE FROM launch_config WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_sessions(&self) -> Result<Vec<Session>, StoreError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, kind, title, repo_root, worktree, status, created_at, cwd,
                    agent_conversation_id
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
                    agent_conversation_id: row.get(8)?,
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
    agent_conversation_id: Option<String>,
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
            connection: crate::session::ConnectionState::default(),
            agent_conversation_id: self
                .agent_conversation_id
                .filter(|id| crate::agent::conversation::is_plausible(id)),
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
    tunnels: Option<String>,
}

impl RawHost {
    fn into_host(self) -> Result<Host, StoreError> {
        Ok(Host {
            tunnels: match self.tunnels.as_deref() {
                Some(j) => serde_json::from_str(j)?,
                None => Vec::new(),
            },
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
            connection: crate::session::ConnectionState::default(),
            agent_conversation_id: None,
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
    fn conversation_id_survives_the_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let mut s = sample("agente");
        s.agent_conversation_id = Some("5f2a1c40-0000-4000-8000-00000000abcd".into());
        store.upsert_session(&s).unwrap();

        assert_eq!(
            store.load_sessions().unwrap()[0].agent_conversation_id,
            s.agent_conversation_id
        );
    }

    /// O upsert corre a cada troca de status. Um `Session` sem o id em memória
    /// não pode apagar o que o hook já gravou — seria perder a conversa entre
    /// dois eventos e o convite sumir sem motivo.
    #[test]
    fn a_status_upsert_never_erases_the_stored_conversation_id() {
        let store = Store::open_in_memory().unwrap();
        let mut s = sample("agente");
        s.agent_conversation_id = Some("5f2a1c40-0000-4000-8000-00000000abcd".into());
        store.upsert_session(&s).unwrap();

        s.agent_conversation_id = None;
        s.status = SessionStatus::Exited { code: 0 };
        store.upsert_session(&s).unwrap();

        assert_eq!(
            store.load_sessions().unwrap()[0]
                .agent_conversation_id
                .as_deref(),
            Some("5f2a1c40-0000-4000-8000-00000000abcd")
        );
    }

    /// Id implausível gravado por qualquer motivo (banco editado à mão, versão
    /// futura) não volta como id: ele viraria argumento de `claude --resume`.
    #[test]
    fn an_implausible_stored_id_is_dropped_on_load() {
        let store = Store::open_in_memory().unwrap();
        let mut s = sample("agente");
        s.agent_conversation_id = Some("--dangerously-skip-permissions".into());
        store.upsert_session(&s).unwrap();

        assert_eq!(
            store.load_sessions().unwrap()[0].agent_conversation_id,
            None
        );
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
            tunnels: Vec::new(),
            created_at: Utc::now(),
            last_connected_at: None,
        }
    }

    #[test]
    fn tuneis_do_host_sobrevivem_ao_round_trip() {
        use crate::ssh::tunnel::{Tunnel, TunnelKind};
        let store = Store::open_in_memory().unwrap();
        let mut h = sample_host("db-01");
        h.tunnels = vec![Tunnel {
            kind: TunnelKind::Local,
            listen_port: 5432,
            listen_host: None,
            target_host: Some("localhost".into()),
            target_port: Some(5432),
        }];
        store.upsert_host(&h).unwrap();
        assert_eq!(store.load_hosts().unwrap()[0].tunnels, h.tunnels);
    }

    #[test]
    fn host_de_antes_da_coluna_carrega_sem_tunel() {
        let store = Store::open_in_memory().unwrap();
        {
            let conn = store.conn.lock();
            conn.execute(
                "INSERT INTO host (id, alias, hostname, position, created_at)
                 VALUES ('x', 'legado', 'legado.host', 0, ?1)",
                params![Utc::now().to_rfc3339()],
            )
            .unwrap();
        }
        let loaded = store.load_hosts().unwrap();
        assert!(
            loaded[0].tunnels.is_empty(),
            "host gravado antes da coluna existir tem tunnels NULL: \
             carregar tem que dar lista vazia, nunca erro de JSON — \
             senão o gestor inteiro para de listar host no primeiro boot pós-update"
        );
    }

    fn sample_session_tunnel(session_id: SessionId, port: u16) -> SessionTunnel {
        SessionTunnel {
            id: Uuid::new_v4().to_string(),
            session_id,
            tunnel: Tunnel {
                kind: crate::ssh::tunnel::TunnelKind::Local,
                listen_port: port,
                listen_host: None,
                target_host: Some("localhost".into()),
                target_port: Some(5432),
            },
            state: TunnelState::Live,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn tunel_de_sessao_sobrevive_a_fechar_o_app() {
        let store = Store::open_in_memory().unwrap();
        let sid = Uuid::new_v4();
        let t = sample_session_tunnel(sid, 5432);
        store.add_session_tunnel(&t).unwrap();

        let loaded = store.load_session_tunnels(sid).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].tunnel, t.tunnel);
        assert_eq!(
            loaded[0].state,
            TunnelState::Opening,
            "estado não se persiste: ele é derivado da realidade. Voltar Live do \
             disco seria alegar que o túnel está de pé antes de alguém ter tentado \
             abrir — é a mentira silenciosa que a tech-spec §4 proíbe"
        );
    }

    #[test]
    fn tunel_de_sessao_e_privado_da_sua_sessao() {
        let store = Store::open_in_memory().unwrap();
        let minha = Uuid::new_v4();
        let outra = Uuid::new_v4();
        store
            .add_session_tunnel(&sample_session_tunnel(minha, 5432))
            .unwrap();
        store
            .add_session_tunnel(&sample_session_tunnel(outra, 6432))
            .unwrap();

        assert_eq!(store.load_session_tunnels(minha).unwrap().len(), 1);
        assert_eq!(
            store.load_session_tunnels(minha).unwrap()[0]
                .tunnel
                .listen_port,
            5432
        );
    }

    #[test]
    fn fechar_a_sessao_leva_os_tuneis_dela_e_so_os_dela() {
        let store = Store::open_in_memory().unwrap();
        let morta = Uuid::new_v4();
        let viva = Uuid::new_v4();
        store
            .add_session_tunnel(&sample_session_tunnel(morta, 5432))
            .unwrap();
        store
            .add_session_tunnel(&sample_session_tunnel(viva, 6432))
            .unwrap();

        store.remove_session_tunnels(morta).unwrap();

        assert!(store.load_session_tunnels(morta).unwrap().is_empty());
        assert_eq!(
            store.load_session_tunnels(viva).unwrap().len(),
            1,
            "o túnel pertence à SSH Session: matar uma não pode levar o da outra"
        );
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

    fn user_version(store: &Store) -> i64 {
        let conn = store.conn.lock();
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    fn column_exists(store: &Store, table: &str, column: &str) -> bool {
        let conn = store.conn.lock();
        has_column(&conn, table, column).unwrap()
    }

    /// Um banco como o que a versão anterior do app deixava no disco: schema
    /// antigo mais os catorze `ADD COLUMN` que rodavam a cada abertura. Escrito
    /// à mão de propósito — nada aqui sai de um banco real.
    fn write_legacy_database(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                repo_root TEXT,
                worktree TEXT,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                scrollback TEXT
            );
            CREATE TABLE workspaces (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                repo_root TEXT,
                position INTEGER NOT NULL,
                active_tab TEXT,
                created_at TEXT NOT NULL
            );
            CREATE TABLE tabs (
                id TEXT PRIMARY KEY,
                title TEXT,
                position INTEGER NOT NULL,
                active_pane TEXT,
                created_at TEXT NOT NULL
            );
            ALTER TABLE sessions ADD COLUMN cwd TEXT;
            ALTER TABLE workspaces ADD COLUMN color TEXT;
            ALTER TABLE workspaces ADD COLUMN group_name TEXT;
            ALTER TABLE workspaces ADD COLUMN kind TEXT;
            ALTER TABLE workspaces ADD COLUMN side_view TEXT;
            ALTER TABLE workspaces ADD COLUMN side_ratio REAL;
            ALTER TABLE workspaces ADD COLUMN side_expanded INTEGER;
            ALTER TABLE workspaces ADD COLUMN name_locked INTEGER;
            ALTER TABLE workspaces ADD COLUMN launch_config_id TEXT;
            ALTER TABLE tabs ADD COLUMN workspace_id TEXT;
            ALTER TABLE tabs ADD COLUMN view TEXT;
            INSERT INTO sessions (id, kind, title, status, created_at, scrollback)
            VALUES ('7b1f0c6e-0000-4000-8000-000000000001', '{\"type\":\"shell\"}', 'zsh',
                    '{\"state\":\"exited\",\"code\":0}', '2026-01-01T00:00:00Z',
                    'saída antiga que não deveria ter ido pro disco');",
        )
        .unwrap();
    }

    /// Um banco parado exatamente na versão 2: a forma em que este binário
    /// encontra a máquina de quem já abriu o app depois da migração anterior.
    /// Fabricado à mão porque é o estado que discrimina degrau de linha de base
    /// — a base só roda em banco na versão 0.
    fn write_v2_database(path: &Path) {
        write_legacy_database(path);
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "ALTER TABLE sessions DROP COLUMN scrollback;
             PRAGMA user_version = 2;",
        )
        .unwrap();
    }

    /// **Este é o teste que discrimina o degrau 3.**
    ///
    /// O banco chega na versão 2, e o bloco da linha de base só roda com
    /// `from < 1`. Se `sessions.agent_conversation_id` tivesse sido acrescentada
    /// à [`BASELINE_COLUMNS`] em vez de ganhar degrau próprio, ninguém que já
    /// usa o app veria a coluna nascer — e a sessão de agente nunca guardaria a
    /// conversa. Aqui isso falha: a coluna tem de existir depois da abertura.
    #[test]
    fn a_v2_database_gains_the_conversation_column_from_its_own_step() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tyba.db");
        write_v2_database(&path);

        // Pré-condição do teste: sem isto ele passaria por acidente.
        {
            let before = Connection::open(&path).unwrap();
            assert!(!has_column(&before, "sessions", "agent_conversation_id").unwrap());
            let version: i64 = before
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(version, 2, "o banco tem de chegar exatamente na versão 2");
        }

        let store = Store::open(&path).unwrap();

        assert!(
            column_exists(&store, "sessions", "agent_conversation_id"),
            "a coluna nasceu de um degrau, não da linha de base — e a linha de \
             base não roda em banco que já está na versão 2"
        );
        assert_eq!(user_version(&store), SCHEMA_VERSION);
        assert_eq!(store.degraded(), None);
        assert_eq!(store.load_sessions().unwrap().len(), 1);
    }

    /// O contrapeso do discriminador: a coluna nova NÃO pode estar na linha de
    /// base. Se estiver, o teste acima passa por acidente num banco versão 0 e a
    /// regressão só aparece na máquina do usuário.
    #[test]
    fn the_conversation_column_is_not_a_baseline_column() {
        assert!(
            !BASELINE_COLUMNS.iter().any(
                |(table, column, _)| *table == "sessions" && *column == "agent_conversation_id"
            ),
            "linha de base só roda em banco na versão 0"
        );
    }

    #[test]
    fn fresh_database_lands_on_the_current_schema_version() {
        let store = Store::open_in_memory().unwrap();

        assert_eq!(user_version(&store), SCHEMA_VERSION);
        assert!(!column_exists(&store, "sessions", "scrollback"));
        assert!(column_exists(&store, "sessions", "agent_conversation_id"));
        for (table, column, _) in BASELINE_COLUMNS {
            assert!(
                column_exists(&store, table, column),
                "{table}.{column} deveria vir do SCHEMA"
            );
        }
    }

    #[test]
    fn legacy_database_migrates_without_reapplying_the_baseline_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tyba.db");
        write_legacy_database(&path);

        let store = Store::open(&path).unwrap();

        assert_eq!(user_version(&store), SCHEMA_VERSION);
        assert!(!column_exists(&store, "sessions", "scrollback"));
        for (table, column, _) in BASELINE_COLUMNS {
            assert!(column_exists(&store, table, column));
        }
        // A sessão herdada sobrevive à queda da coluna: o que sai é o output,
        // não a linha.
        let sessions = store.load_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "zsh");
    }

    /// Um banco legado com um índice sobre a coluna que a versão 2 derruba.
    /// SQLite recusa o `DROP COLUMN` enquanto o índice existir — é o estado
    /// mais barato de fabricar entre os que a review lista (índice, view ou
    /// trigger sobrando, banco editado à mão, backup restaurado pela metade).
    fn write_database_that_refuses_the_drop(path: &Path) {
        write_legacy_database(path);
        Connection::open(path)
            .unwrap()
            .execute_batch("CREATE INDEX sessions_by_scrollback ON sessions(scrollback);")
            .unwrap();
    }

    /// **Atualizar o app não pode deixar alguém sem app.** Este degrau roda na
    /// primeira abertura de todo usuário depois de atualizar; se ele derrubasse
    /// o `Store::open`, o `open_store` cairia para um banco em memória e a
    /// abertura inteira — sessões, layout, histórico — sumiria em silêncio.
    #[test]
    fn a_migration_step_that_cannot_apply_keeps_the_database_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tyba.db");
        write_database_that_refuses_the_drop(&path);

        let store = Store::open(&path).expect("banco com degrau pendente ainda tem de abrir");

        // O banco continua sendo o do disco: a sessão herdada está lá.
        assert_eq!(store.load_sessions().unwrap().len(), 1);
        // O que não pegou continua não tendo pegado — nada de fingir sucesso.
        assert!(column_exists(&store, "sessions", "scrollback"));
    }

    /// E a falha não pode ser silenciosa: é ela que o `open_store` transforma
    /// em `app://boot-failed`, para o vazio na tela ler como falha e não como
    /// ausência de dado.
    #[test]
    fn a_skipped_migration_step_is_reported_by_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tyba.db");
        write_database_that_refuses_the_drop(&path);

        let store = Store::open(&path).unwrap();

        let degraded = store
            .degraded()
            .expect("o degrau que falhou tem de aparecer");
        assert!(
            degraded.contains("sessions.scrollback"),
            "a mensagem tem de dizer qual degrau ficou para trás: {degraded}"
        );
    }

    /// O contrapeso: banco saudável não pode reportar degradação, senão o
    /// banner de falha apareceria em toda abertura e deixaria de significar
    /// alguma coisa.
    #[test]
    fn a_healthy_database_reports_no_degradation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tyba.db");
        write_legacy_database(&path);

        assert_eq!(Store::open(&path).unwrap().degraded(), None);
        assert_eq!(Store::open_in_memory().unwrap().degraded(), None);
    }

    /// Carimbar `SCHEMA_VERSION` com degrau pendente marcaria como concluída
    /// uma migração que não aconteceu — e ela nunca mais seria tentada. A
    /// versão anda até o degrau contíguo que pegou, e o resto é retentado.
    #[test]
    fn a_pending_step_holds_the_schema_version_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tyba.db");
        write_database_that_refuses_the_drop(&path);

        // A linha de base pegou; o `DROP COLUMN` não. Versão 1, não 2.
        let store = Store::open(&path).unwrap();
        assert_eq!(user_version(&store), 1);
        for (table, column, _) in BASELINE_COLUMNS {
            assert!(column_exists(&store, table, column));
        }
        drop(store);

        // Some o obstáculo e a próxima abertura completa a migração sozinha.
        Connection::open(&path)
            .unwrap()
            .execute_batch("DROP INDEX sessions_by_scrollback;")
            .unwrap();
        let store = Store::open(&path).unwrap();
        assert_eq!(user_version(&store), SCHEMA_VERSION);
        assert_eq!(store.degraded(), None);
        assert!(!column_exists(&store, "sessions", "scrollback"));
        assert_eq!(store.load_sessions().unwrap().len(), 1);
    }

    /// A outra metade da distinção: banco **ilegível** continua sendo `Err`.
    /// Aqui não há o que degradar — o arquivo não é um banco —, e é o único
    /// caso em que cair para memória é a saída que ainda entrega um app.
    #[test]
    fn an_unreadable_file_is_still_a_hard_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tyba.db");
        std::fs::write(&path, b"isto nao e um banco sqlite").unwrap();

        assert!(Store::open(&path).is_err());
    }

    #[test]
    fn reopening_a_migrated_database_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tyba.db");
        write_legacy_database(&path);

        drop(Store::open(&path).unwrap());
        let store = Store::open(&path).unwrap();

        assert_eq!(user_version(&store), SCHEMA_VERSION);
        assert_eq!(store.load_sessions().unwrap().len(), 1);
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

    #[test]
    fn lsp_managed_consent_persists_per_server_and_survives_a_version_bump() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.lsp_managed_consent("rust-analyzer").unwrap());

        store
            .set_lsp_managed_consent("rust-analyzer", "2026-07-20")
            .unwrap();
        assert!(store.lsp_managed_consent("rust-analyzer").unwrap());
        assert!(
            !store.lsp_managed_consent("taplo").unwrap(),
            "consent é por server"
        );

        // Bump de versão numa release nova do TYBA não reperguntar: o lookup é por
        // server, não por versão.
        store
            .set_lsp_managed_consent("rust-analyzer", "2026-09-01")
            .unwrap();
        assert!(store.lsp_managed_consent("rust-analyzer").unwrap());
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

    fn command(cmd: &str, cwd: Option<&str>, at: i64) -> crate::history::CommandRecord {
        crate::history::CommandRecord {
            session_id: "s1".into(),
            cwd: cwd.map(str::to_string),
            command: cmd.into(),
            exit_code: Some(0),
            started_at_ms: at,
            duration_ms: Some(10),
        }
    }

    fn history_count(store: &Store) -> i64 {
        let conn = store.conn.lock();
        conn.query_row("SELECT COUNT(*) FROM command_history", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn command_history_redacts_secrets_before_persisting() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_command(&command(
                "export TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789",
                Some("/repo"),
                1,
            ))
            .unwrap();

        let stored: String = {
            let conn = store.conn.lock();
            conn.query_row("SELECT command FROM command_history", [], |row| row.get(0))
                .unwrap()
        };
        assert!(!stored.contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(stored.contains("[REDACTED]"));
    }

    #[test]
    fn repeated_command_is_not_stored_twice_in_a_row() {
        let store = Store::open_in_memory().unwrap();
        store.insert_command(&command("ls", Some("/a"), 1)).unwrap();
        store.insert_command(&command("ls", Some("/a"), 2)).unwrap();
        store
            .insert_command(&command("pwd", Some("/a"), 3))
            .unwrap();
        store.insert_command(&command("ls", Some("/a"), 4)).unwrap();
        assert_eq!(history_count(&store), 3);
    }

    #[test]
    fn history_candidates_flag_directory_and_repo_scope() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_command(&command("cargo test", Some("/repo/src"), 1))
            .unwrap();
        store
            .insert_command(&command("bun test", Some("/repo"), 2))
            .unwrap();
        store
            .insert_command(&command("brew upgrade", Some("/elsewhere"), 3))
            .unwrap();

        let found = store
            .history_candidates(Some("/repo/src"), Some("/repo"))
            .unwrap();
        let by = |cmd: &str| {
            found
                .iter()
                .find(|c| c.command == cmd)
                .unwrap_or_else(|| panic!("{cmd} ausente"))
                .clone()
        };
        assert!(by("cargo test").in_cwd);
        assert!(by("cargo test").in_repo);
        assert!(!by("bun test").in_cwd);
        assert!(by("bun test").in_repo, "cwd igual à raiz conta como repo");
        assert!(!by("brew upgrade").in_repo);
    }

    #[test]
    fn history_candidates_aggregate_uses_and_successes() {
        let store = Store::open_in_memory().unwrap();
        for at in 1..=3 {
            store
                .insert_command(&command("cargo test", Some("/repo"), at))
                .unwrap();
            store
                .insert_command(&command("pwd", Some("/repo"), at))
                .unwrap();
        }
        let mut failed = command("cargo test", Some("/repo"), 9);
        failed.exit_code = Some(101);
        store.insert_command(&failed).unwrap();

        let found = store.history_candidates(Some("/repo"), None).unwrap();
        let cargo = found.iter().find(|c| c.command == "cargo test").unwrap();
        assert_eq!(cargo.uses, 4);
        assert_eq!(cargo.successes, 3);
        assert_eq!(cargo.last_used_at_ms, 9);
    }

    #[test]
    fn repo_scope_does_not_leak_through_like_wildcards() {
        // `_` é curinga no LIKE: sem escapar, `/tmp/a_b` casaria com `/tmp/axb`.
        let store = Store::open_in_memory().unwrap();
        store
            .insert_command(&command("intruso", Some("/tmp/axb/sub"), 1))
            .unwrap();
        let found = store.history_candidates(None, Some("/tmp/a_b")).unwrap();
        assert!(!found.iter().any(|c| c.in_repo));
    }

    #[test]
    fn clear_by_repo_keeps_the_rest() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_command(&command("dentro", Some("/repo/src"), 1))
            .unwrap();
        store
            .insert_command(&command("fora", Some("/outro"), 2))
            .unwrap();
        store.clear_command_history(Some("/repo")).unwrap();

        let found = store.history_candidates(None, None).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "fora");

        store.clear_command_history(None).unwrap();
        assert_eq!(history_count(&store), 0);
    }

    fn block(session: &str, command: &str, lines: usize) -> crate::blocks::Block {
        crate::blocks::Block {
            id: 0,
            session_id: session.into(),
            command: command.into(),
            exit_code: Some(0),
            alt_screen: false,
            cwd: None,
            started_at_ms: 1,
            finished_at_ms: 2,
            lines: (0..lines)
                .map(|i| crate::blocks::LogicalLine {
                    text: format!("linha {i}"),
                    runs: Vec::new(),
                })
                .collect(),
            truncated: 0,
        }
    }

    #[test]
    fn block_round_trip_keeps_reading_order() {
        let store = Store::open_in_memory().unwrap();
        store.insert_block(&block("s1", "primeiro", 1)).unwrap();
        store.insert_block(&block("s1", "segundo", 2)).unwrap();
        store.insert_block(&block("s2", "outra sessão", 1)).unwrap();

        let found = store.list_blocks("s1", 100).unwrap();
        assert_eq!(
            found.iter().map(|b| b.command.as_str()).collect::<Vec<_>>(),
            vec!["primeiro", "segundo"],
            "do mais antigo para o mais novo, como se lê"
        );
        assert_eq!(found[1].lines.len(), 2);
        assert_eq!(store.list_blocks("s2", 100).unwrap().len(), 1);
    }

    #[test]
    fn block_retention_prunes_the_oldest_by_count() {
        let store = Store::open_in_memory().unwrap();
        for i in 0..(BLOCK_CAP_COUNT + 5) {
            store
                .insert_block(&block("s1", &format!("cmd{i}"), 1))
                .unwrap();
        }
        let found = store.list_blocks("s1", 5_000).unwrap();
        assert_eq!(found.len() as i64, BLOCK_CAP_COUNT);
        assert_eq!(found[0].command, "cmd5", "os mais antigos é que saem");
    }

    #[test]
    fn block_of_unknown_version_is_ignored_not_fatal() {
        // Bloco gravado por uma versão futura do TYBA não pode derrubar a
        // sessão de quem voltou para uma versão antiga.
        let store = Store::open_in_memory().unwrap();
        store.insert_block(&block("s1", "atual", 1)).unwrap();
        {
            let conn = store.conn.lock();
            conn.execute(
                "INSERT INTO block (session_id, version, command, exit_code,
                     started_at_ms, finished_at_ms, truncated, bytes, lines)
                 VALUES ('s1', 999, 'do futuro', 0, 1, 2, 0, 10, 'formato-que-nao-existe')",
                [],
            )
            .unwrap();
        }
        let found = store.list_blocks("s1", 100).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "atual");
    }

    #[test]
    fn a_full_screen_app_still_leaves_a_block_behind() {
        // `bat`, `vim`, `htop`: a saída é a tela de um programa e não se guarda,
        // mas o comando foi executado — sumir com ele apaga do registro algo que
        // a pessoa fez.
        let store = Store::open_in_memory().unwrap();
        let mut tui = block("s1", "bat README.md", 0);
        tui.alt_screen = true;
        store.insert_block(&tui).unwrap();

        let found = store.list_blocks("s1", 10).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "bat README.md");
        assert!(
            found[0].alt_screen,
            "o bloco precisa dizer por que está vazio"
        );
        assert!(found[0].lines.is_empty());
    }

    #[test]
    fn discarding_a_session_takes_its_blocks_and_checkpoint_along() {
        // Descartar a sessão é o gesto de fazer a saída dela sumir. Bloco e
        // checkpoint não têm FK; sem isto ficariam no disco até a retenção.
        let store = Store::open_in_memory().unwrap();
        let doomed = Uuid::new_v4();
        let other = Uuid::new_v4();
        store
            .insert_block(&block(&doomed.to_string(), "a", 1))
            .unwrap();
        store
            .insert_block(&block(&other.to_string(), "b", 1))
            .unwrap();
        store
            .save_checkpoint(&doomed.to_string(), "cargo build", 10, 80, 24, b"x")
            .unwrap();
        store
            .save_checkpoint(&other.to_string(), "cargo test", 10, 80, 24, b"y")
            .unwrap();

        store.remove_session(doomed).unwrap();

        assert!(store
            .list_blocks(&doomed.to_string(), 10)
            .unwrap()
            .is_empty());
        assert_eq!(store.list_blocks(&other.to_string(), 10).unwrap().len(), 1);
        // O checkpoint da descartada some; o da outra ainda vira bloco no boot.
        assert_eq!(store.drain_checkpoints().unwrap(), 1);
        assert_eq!(
            store.list_blocks(&other.to_string(), 10).unwrap().len(),
            2,
            "o checkpoint sobrevivente é o da sessão que ficou"
        );
    }

    #[test]
    fn orphan_checkpoint_becomes_an_unfinished_block_on_boot() {
        // O app morreu no meio de um comando longo: a saída não pode sumir.
        let store = Store::open_in_memory().unwrap();
        store
            .save_checkpoint("s1", "cargo build", 10, 80, 24, b"Compiling tyba\r\n")
            .unwrap();

        assert_eq!(store.drain_checkpoints().unwrap(), 1);
        let found = store.list_blocks("s1", 10).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "cargo build");
        assert_eq!(found[0].exit_code, None, "não terminou, e o bloco diz isso");
        assert_eq!(found[0].lines[0].text, "Compiling tyba");

        // Consumido: reabrir de novo não duplica o bloco.
        assert_eq!(store.drain_checkpoints().unwrap(), 0);
        assert_eq!(store.list_blocks("s1", 10).unwrap().len(), 1);
    }

    #[test]
    fn checkpoint_is_one_row_per_session_always_replaced() {
        let store = Store::open_in_memory().unwrap();
        store
            .save_checkpoint("s1", "cmd", 1, 80, 24, b"antes")
            .unwrap();
        store
            .save_checkpoint("s1", "cmd", 1, 80, 24, b"depois")
            .unwrap();
        store.drain_checkpoints().unwrap();
        let found = store.list_blocks("s1", 10).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].lines[0].text, "depois");
    }

    #[test]
    fn finished_command_leaves_no_checkpoint_behind() {
        let store = Store::open_in_memory().unwrap();
        store
            .save_checkpoint("s1", "cmd", 1, 80, 24, b"parcial")
            .unwrap();
        store.clear_checkpoint("s1").unwrap();
        assert_eq!(store.drain_checkpoints().unwrap(), 0);
    }

    #[test]
    fn snippet_round_trip_and_delete() {
        let store = Store::open_in_memory().unwrap();
        let snippet = crate::snippet::Snippet {
            id: "s-1".into(),
            name: "deploy".into(),
            command: "deploy {{env}}".into(),
            description: Some("sobe".into()),
            tags: vec!["ops".into(), "ci".into()],
            source: crate::snippet::Source::Local,
        };
        store.save_snippet(&snippet).unwrap();
        assert_eq!(store.list_snippets().unwrap(), vec![snippet.clone()]);

        let renamed = crate::snippet::Snippet {
            name: "deploy prod".into(),
            ..snippet.clone()
        };
        store.save_snippet(&renamed).unwrap();
        let listed = store.list_snippets().unwrap();
        assert_eq!(listed.len(), 1, "mesmo id não duplica");
        assert_eq!(listed[0].name, "deploy prod");
        assert_eq!(listed[0].tags, vec!["ops", "ci"]);

        store.delete_snippet("s-1").unwrap();
        assert!(store.list_snippets().unwrap().is_empty());
    }
}
