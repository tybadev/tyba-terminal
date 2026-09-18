use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod agent_keys;
pub mod broadcast;
pub mod classify;
pub mod command;
pub mod config;
pub mod query;
pub mod remote_rc;
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
    /// A integração de shell do TYBA no servidor (regra 11). **Ligada por
    /// padrão**, inclusive para o Host que já existia antes desta entrega — o
    /// `default` do serde e o do banco têm de concordar, senão um Host que
    /// chega sem o campo nasce sem integração e ninguém descobre por quê.
    #[serde(default = "default_true")]
    pub integration_enabled: bool,
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
    #[serde(default = "default_true")]
    pub integration_enabled: bool,
}

fn default_true() -> bool {
    true
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
            integration_enabled: self.integration_enabled,
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

/// O que o pane explica em uma linha, e o que o evento
/// `ssh://integration/<sessão>` carrega.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntegrationState {
    Integrated,
    Plain,
}

/// Por que a sessão é o que é. Código, não frase: a tela é quem traduz (i18n).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IntegrationReason {
    /// Integrada, sem ressalva.
    Ok,
    /// A chave do Host está desligada (regra 11).
    HostSwitchOff,
    /// O shell de login do servidor não é bash nem zsh (regra 8).
    UnsupportedShell,
    /// Não deu para detectar o shell — sem conexão compartilhada, Host de senha
    /// sem sessão aberta, servidor mudo. Pela regra 8 vale o mesmo que shell não
    /// suportado, mas o motivo é outro e o dono precisa saber qual é.
    Undetected,
    /// Sessão que já estava viva antes desta versão (regra 12).
    FromBefore,
}

/// A persistência da SSH Session, que é o tmux do servidor quem dá (regra 13).
///
/// Vive ao lado do estado e não dentro do motivo de propósito: o motivo responde
/// "por que esta sessão é integrada ou comum" e é um só, enquanto a persistência
/// é ortogonal — um Host com a chave desligada pode ter tmux, e um Host
/// integrado pode não ter. Espremer as duas num enum só obrigaria a escolher
/// qual dos dois fatos contar ao dono.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Persistence {
    /// O servidor tem tmux: a SSH Session sobrevive à queda do Cano.
    Persistent,
    /// Servidor sem tmux: integrada, mas o que está na tela morre com o Cano.
    Ephemeral,
    /// Não deu para perguntar — sem canal, Host de senha sem sessão aberta, ou
    /// plano gravado antes desta correção. O pane não afirma o que o core não
    /// sabe, e `true` NUNCA é o palpite seguro aqui.
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Integration {
    pub state: IntegrationState,
    pub reason: IntegrationReason,
    /// Ausente na linha gravada antes desta correção: ali vale `Unknown`, que é
    /// exatamente o que o core sabia então.
    #[serde(default)]
    pub persistence: Persistence,
    /// O nome do shell recusado, quando há um. É o que faz a linha do pane
    /// dizer "fish" em vez de "shell não suportado".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Integration {
    pub fn integrated() -> Self {
        Self {
            state: IntegrationState::Integrated,
            reason: IntegrationReason::Ok,
            persistence: Persistence::Unknown,
            detail: None,
        }
    }

    pub fn plain(reason: IntegrationReason, detail: Option<String>) -> Self {
        Self {
            state: IntegrationState::Plain,
            reason,
            persistence: Persistence::Unknown,
            detail,
        }
    }

    pub fn with_persistence(mut self, persistence: Persistence) -> Self {
        self.persistence = persistence;
        self
    }

    pub fn is_integrated(&self) -> bool {
        self.state == IntegrationState::Integrated
    }
}

/// A decisão junto com o shell que a produziu: o comando remoto precisa dos
/// dois, e separá-los abriria a porta para montar o rc de um shell e anunciar
/// outro.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationPlan {
    pub integration: Integration,
    /// Guardado junto porque reatar tem de reproduzir o MESMO comando remoto:
    /// se o tmux do servidor tiver morrido, é o comando que recria o pane, e um
    /// shell diferente aqui faria a sessão renascer sem o rc enquanto o evento
    /// continuava dizendo "integrada".
    pub shell: remote_rc::RemoteShell,
}

impl IntegrationPlan {
    pub fn decide(enabled: bool, shell: remote_rc::RemoteShell, persistence: Persistence) -> Self {
        Self {
            integration: decide_integration(enabled, &shell, persistence),
            shell,
        }
    }

    /// O plano a partir do que o canal apurou do Host (regras 8, 11 e 13).
    ///
    /// Shell e persistência saem da MESMA resposta de propósito: decidir com
    /// uma sonda e anunciar com outra deixaria o pane dizer de um Host o que
    /// vale de outro momento dele.
    pub fn from_probe(enabled: bool, probe: query::HostProbe) -> Self {
        Self::decide(enabled, probe.shell, probe.persistence)
    }

    /// Sem canal para perguntar nada — é o que o boot e o religar de queda
    /// usam quando a decisão já está gravada e o shell não importa mais.
    pub fn undetected() -> Self {
        Self::decide(
            true,
            remote_rc::RemoteShell::Unsupported("desconhecido".into()),
            Persistence::Unknown,
        )
    }
}

/// A decisão da regra 8 + regra 11, num lugar só e sem I/O.
///
/// A persistência entra por fora porque não decide nada aqui: sem tmux a sessão
/// segue integrada (regra 13), e o que muda é só a ressalva que o pane escreve.
pub fn decide_integration(
    enabled: bool,
    shell: &remote_rc::RemoteShell,
    persistence: Persistence,
) -> Integration {
    use remote_rc::RemoteShell;
    let decidido = if !enabled {
        Integration::plain(IntegrationReason::HostSwitchOff, None)
    } else {
        match shell {
            RemoteShell::Bash | RemoteShell::Zsh => Integration::integrated(),
            RemoteShell::Unsupported(name) if name == "desconhecido" => {
                Integration::plain(IntegrationReason::Undetected, None)
            }
            RemoteShell::Unsupported(name) => {
                Integration::plain(IntegrationReason::UnsupportedShell, Some(name.to_string()))
            }
        }
    };
    decidido.with_persistence(persistence)
}

pub fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(std::path::PathBuf::from))
}

#[cfg(test)]
mod tests {
    use super::remote_rc::RemoteShell;
    use super::*;

    #[test]
    fn a_chave_desligada_ganha_do_shell_suportado() {
        let decidido = decide_integration(false, &RemoteShell::Bash, Persistence::Persistent);
        assert_eq!(decidido.state, IntegrationState::Plain);
        assert_eq!(decidido.reason, IntegrationReason::HostSwitchOff);
    }

    #[test]
    fn shell_recusado_diz_qual_era() {
        let decidido = decide_integration(
            true,
            &RemoteShell::Unsupported("fish".into()),
            Persistence::Unknown,
        );
        assert_eq!(decidido.reason, IntegrationReason::UnsupportedShell);
        assert_eq!(
            decidido.detail.as_deref(),
            Some("fish"),
            "a linha do pane precisa nomear o shell; sem isso ela não explica nada"
        );
    }

    #[test]
    fn deteccao_que_falhou_tem_motivo_proprio() {
        let decidido = decide_integration(
            true,
            &RemoteShell::Unsupported("desconhecido".into()),
            Persistence::Unknown,
        );
        assert_eq!(
            decidido.reason,
            IntegrationReason::Undetected,
            "não saber qual é o shell não é o mesmo que saber que ele não serve"
        );
        assert_eq!(decidido.detail, None);
    }

    /// Regra 13: sem tmux no servidor a sessão é integrada assim mesmo — o que
    /// ela perde é a persistência, e é isso que o pane tem de dizer.
    #[test]
    fn host_sem_tmux_abre_integrada_e_sem_persistencia() {
        let decidido = decide_integration(true, &RemoteShell::Bash, Persistence::Ephemeral);
        assert!(
            decidido.is_integrated(),
            "sem tmux não é sessão comum: continua integrada"
        );
        assert_eq!(decidido.persistence, Persistence::Ephemeral);
    }

    #[test]
    fn bash_e_zsh_integram() {
        for shell in [RemoteShell::Bash, RemoteShell::Zsh] {
            assert!(decide_integration(true, &shell, Persistence::Persistent).is_integrated());
        }
    }

    #[test]
    fn o_evento_sai_no_formato_que_a_tela_espera() {
        let json = serde_json::to_string(&decide_integration(
            true,
            &RemoteShell::Unsupported("fish".into()),
            Persistence::Unknown,
        ))
        .unwrap();
        assert_eq!(
            json,
            r#"{"state":"plain","reason":"unsupported-shell","persistence":"unknown","detail":"fish"}"#
        );
        let integrado = serde_json::to_string(
            &Integration::integrated().with_persistence(Persistence::Ephemeral),
        )
        .unwrap();
        assert_eq!(
            integrado,
            r#"{"state":"integrated","reason":"ok","persistence":"ephemeral"}"#
        );
    }

    /// Regra 13: a linha do pane precisa distinguir "integrada e persistente" de
    /// "integrada e some com o Cano" — e o evento é o único lugar de onde a tela
    /// pode saber disso.
    #[test]
    fn o_evento_leva_a_persistencia_para_a_tela() {
        let com_tmux = serde_json::to_value(decide_integration(
            true,
            &RemoteShell::Bash,
            Persistence::Persistent,
        ))
        .unwrap();
        assert_eq!(com_tmux["state"], "integrated");
        assert_eq!(com_tmux["persistence"], "persistent");

        let sem_tmux = serde_json::to_value(decide_integration(
            true,
            &RemoteShell::Bash,
            Persistence::Ephemeral,
        ))
        .unwrap();
        assert_eq!(
            sem_tmux["state"], "integrated",
            "sem tmux NÃO rebaixa a sessão para comum"
        );
        assert_eq!(sem_tmux["persistence"], "ephemeral");
    }

    /// O plano nasce da sonda INTEIRA do canal: shell e tmux saem da mesma
    /// resposta, e quando ela não veio nenhum dos dois é inventado.
    #[test]
    fn o_plano_sai_da_sonda_do_canal() {
        let plano = IntegrationPlan::from_probe(
            true,
            query::HostProbe {
                shell: RemoteShell::Bash,
                persistence: Persistence::Ephemeral,
            },
        );
        assert!(plano.integration.is_integrated());
        assert_eq!(plano.integration.persistence, Persistence::Ephemeral);
        assert_eq!(plano.shell, RemoteShell::Bash);

        let sem_canal = IntegrationPlan::from_probe(true, query::HostProbe::default());
        assert_eq!(sem_canal.integration.reason, IntegrationReason::Undetected);
        assert_eq!(sem_canal.integration.persistence, Persistence::Unknown);
    }

    /// Plano gravado antes desta correção (sem o campo): continua legível, e a
    /// persistência é desconhecida — nunca `true` por omissão.
    #[test]
    fn plano_gravado_antes_da_correcao_continua_legivel() {
        let antigo = r#"{"integration":{"state":"integrated","reason":"ok"},"shell":"bash"}"#;

        let plano: IntegrationPlan = serde_json::from_str(antigo).unwrap();

        assert!(plano.integration.is_integrated());
        assert_eq!(plano.shell, remote_rc::RemoteShell::Bash);
        assert_eq!(
            plano.integration.persistence,
            Persistence::Unknown,
            "o core não sabia: o pane não pode afirmar persistência que ninguém apurou"
        );
    }
}
