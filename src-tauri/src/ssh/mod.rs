use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod agent_keys;
pub mod broadcast;
pub mod classify;
pub mod command;
pub mod config;
pub mod test_conn;
pub mod tmux;
pub mod tunnel;

/// Como o Host autentica. `Auto` é o de antes desta entrega: nenhuma opção de
/// autenticação no bloco além do `IdentityFile` legado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    #[default]
    Auto,
    Agent,
    File,
    Password,
}

/// Chave pública escolhida no agente. Só a parte pública: a privada nunca
/// sai do agente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentKey {
    pub public_key: String,
    pub name: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    pub id: String,
    pub alias: String,
    pub hostname: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub identity_file: Option<String>,
    #[serde(default)]
    pub proxy_jump: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub tunnels: Vec<tunnel::Tunnel>,
    #[serde(default)]
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub agent_key: Option<AgentKey>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_connected_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub position: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HostInput {
    pub alias: String,
    pub hostname: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub identity_file: Option<String>,
    #[serde(default)]
    pub proxy_jump: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub tunnels: Vec<tunnel::Tunnel>,
    #[serde(default)]
    pub auth_method: AuthMethod,
    #[serde(default)]
    pub agent_key: Option<AgentKey>,
}

impl HostInput {
    pub fn into_host(self, id: String, position: i64, created_at: DateTime<Utc>) -> Host {
        Host {
            id,
            alias: self.alias,
            hostname: self.hostname,
            port: self.port,
            username: self.username,
            identity_file: self.identity_file,
            proxy_jump: self.proxy_jump,
            group_id: self.group_id,
            color: self.color,
            notes: self.notes,
            position,
            tunnels: self.tunnels,
            auth_method: self.auth_method,
            agent_key: self.agent_key,
            created_at,
            last_connected_at: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct HostGroupInput {
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

pub fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(std::path::PathBuf::from))
}
