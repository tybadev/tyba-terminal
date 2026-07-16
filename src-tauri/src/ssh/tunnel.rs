use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelKind {
    Local,
    Remote,
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tunnel {
    pub kind: TunnelKind,
    pub listen_port: u16,
    #[serde(default)]
    pub listen_host: Option<String>,
    #[serde(default)]
    pub target_host: Option<String>,
    #[serde(default)]
    pub target_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelReach {
    Outbound,
    Inbound,
}

impl TunnelKind {
    pub fn reach(self) -> TunnelReach {
        match self {
            TunnelKind::Local => TunnelReach::Outbound,
            TunnelKind::Remote | TunnelKind::Dynamic => TunnelReach::Inbound,
        }
    }

    pub fn needs_confirmation(self) -> bool {
        self.reach() == TunnelReach::Inbound
    }

    pub fn flag(self) -> &'static str {
        match self {
            TunnelKind::Local => "-L",
            TunnelKind::Remote => "-R",
            TunnelKind::Dynamic => "-D",
        }
    }

    fn keyword(self) -> &'static str {
        match self {
            TunnelKind::Local => "LocalForward",
            TunnelKind::Remote => "RemoteForward",
            TunnelKind::Dynamic => "DynamicForward",
        }
    }
}

impl Tunnel {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.listen_port == 0 {
            return Err(AppError::new("ssh.tunnel_port_invalid"));
        }
        if let Some(h) = self.listen_host.as_deref() {
            if !valid_host(h) {
                return Err(AppError::new("ssh.tunnel_host_invalid").with("host", h.to_string()));
            }
        }
        match self.kind {
            TunnelKind::Dynamic => {
                if self.target_host.is_some() || self.target_port.is_some() {
                    return Err(AppError::new("ssh.tunnel_dynamic_has_target"));
                }
            }
            TunnelKind::Local | TunnelKind::Remote => {
                let host = self
                    .target_host
                    .as_deref()
                    .ok_or_else(|| AppError::new("ssh.tunnel_target_missing"))?;
                if !valid_host(host) {
                    return Err(
                        AppError::new("ssh.tunnel_host_invalid").with("host", host.to_string())
                    );
                }
                match self.target_port {
                    Some(0) | None => return Err(AppError::new("ssh.tunnel_target_missing")),
                    Some(_) => {}
                }
            }
        }
        Ok(())
    }

    fn listen(&self) -> String {
        match self.listen_host.as_deref() {
            Some(h) => format!("{h}:{}", self.listen_port),
            None => self.listen_port.to_string(),
        }
    }

    fn target(&self) -> String {
        format!(
            "{}:{}",
            self.target_host.as_deref().unwrap_or_default(),
            self.target_port.unwrap_or_default()
        )
    }

    fn spec(&self, sep: char) -> String {
        match self.kind {
            TunnelKind::Dynamic => self.listen(),
            _ => format!("{}{sep}{}", self.listen(), self.target()),
        }
    }

    pub fn config_line(&self) -> Result<String, AppError> {
        self.validate()?;
        Ok(format!("    {} {}\n", self.kind.keyword(), self.spec(' ')))
    }

    pub fn cli_args(&self) -> Result<Vec<String>, AppError> {
        self.validate()?;
        Ok(vec![self.kind.flag().to_string(), self.spec(':')])
    }
}

fn valid_host(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 255
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(port: u16) -> Tunnel {
        Tunnel {
            kind: TunnelKind::Local,
            listen_port: port,
            listen_host: None,
            target_host: Some("localhost".into()),
            target_port: Some(port),
        }
    }

    fn remote(port: u16) -> Tunnel {
        Tunnel {
            kind: TunnelKind::Remote,
            listen_port: port,
            listen_host: None,
            target_host: Some("localhost".into()),
            target_port: Some(3000),
        }
    }

    #[test]
    fn local_nao_pede_confirmacao() {
        assert!(
            !TunnelKind::Local.needs_confirmation(),
            "-L é o dono alcançando o que já alcança (ele tem shell no host): \
             confirmar seria imposto sem ganho, e gate que aparece toda hora \
             treina o dono a clicar Sim no automático"
        );
        assert_eq!(TunnelKind::Local.reach(), TunnelReach::Outbound);
    }

    #[test]
    fn remote_e_dynamic_pedem_confirmacao() {
        assert!(
            TunnelKind::Remote.needs_confirmation(),
            "-R abre porta no host que entra na máquina do dono: caminho da prod \
             para dentro do laptop"
        );
        assert!(
            TunnelKind::Dynamic.needs_confirmation(),
            "-D faz da máquina do dono um proxy SOCKS para dentro da rede do host"
        );
        assert_eq!(TunnelKind::Remote.reach(), TunnelReach::Inbound);
        assert_eq!(TunnelKind::Dynamic.reach(), TunnelReach::Inbound);
    }

    #[test]
    fn config_line_do_local() {
        assert_eq!(
            local(5432).config_line().unwrap(),
            "    LocalForward 5432 localhost:5432\n"
        );
    }

    #[test]
    fn config_line_do_remote() {
        assert_eq!(
            remote(8000).config_line().unwrap(),
            "    RemoteForward 8000 localhost:3000\n"
        );
    }

    #[test]
    fn config_line_do_dynamic_nao_tem_alvo() {
        let d = Tunnel {
            kind: TunnelKind::Dynamic,
            listen_port: 1080,
            listen_host: None,
            target_host: None,
            target_port: None,
        };
        assert_eq!(d.config_line().unwrap(), "    DynamicForward 1080\n");
    }

    #[test]
    fn listen_host_explicito_entra_na_linha() {
        let t = Tunnel {
            listen_host: Some("127.0.0.1".into()),
            ..local(5432)
        };
        assert_eq!(
            t.config_line().unwrap(),
            "    LocalForward 127.0.0.1:5432 localhost:5432\n"
        );
    }

    #[test]
    fn cli_args_batem_com_a_config_line() {
        assert_eq!(
            local(5432).cli_args().unwrap(),
            vec!["-L".to_string(), "5432:localhost:5432".to_string()]
        );
        assert_eq!(
            remote(8000).cli_args().unwrap(),
            vec!["-R".to_string(), "8000:localhost:3000".to_string()]
        );
    }

    #[test]
    fn porta_zero_e_recusada() {
        assert!(local(0).validate().is_err());
    }

    #[test]
    fn local_sem_alvo_e_recusado() {
        let t = Tunnel {
            target_host: None,
            ..local(5432)
        };
        assert!(t.validate().is_err());
    }

    #[test]
    fn dynamic_com_alvo_e_recusado() {
        let t = Tunnel {
            kind: TunnelKind::Dynamic,
            listen_port: 1080,
            listen_host: None,
            target_host: Some("localhost".into()),
            target_port: Some(80),
        };
        assert!(
            t.validate().is_err(),
            "-D não tem alvo; aceitar calado escreveria config inválida no ~/.ssh"
        );
    }

    #[test]
    fn host_com_injecao_e_recusado() {
        for evil in [
            "localhost\n    RemoteForward 22 localhost:22",
            "local host",
            "localhost;id",
            "$(id)",
            "",
        ] {
            let t = Tunnel {
                target_host: Some(evil.into()),
                ..local(5432)
            };
            assert!(
                t.validate().is_err(),
                "o alvo entra no ~/.ssh/config.d/tyba.conf e na linha de comando \
                 do ssh: {evil:?} não pode passar"
            );
        }
    }
}
