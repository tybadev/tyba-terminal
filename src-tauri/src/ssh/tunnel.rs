use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub const DEFAULT_BIND: &str = "127.0.0.1";

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

    pub fn binds_locally(self) -> bool {
        match self {
            TunnelKind::Local | TunnelKind::Dynamic => true,
            TunnelKind::Remote => false,
        }
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

    pub fn bind_address(&self) -> &str {
        self.listen_host.as_deref().unwrap_or(DEFAULT_BIND)
    }

    fn listen(&self) -> String {
        format!("{}:{}", self.bind_address(), self.listen_port)
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

    fn exposure(&self) -> (TunnelKind, &str, u16, Option<&str>, Option<u16>) {
        (
            self.kind,
            self.bind_address(),
            self.listen_port,
            self.target_host.as_deref(),
            self.target_port,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum TunnelState {
    Opening,
    Live,
    Error { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTunnel {
    pub id: String,
    pub session_id: uuid::Uuid,
    #[serde(flatten)]
    pub tunnel: Tunnel,
    pub state: TunnelState,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Default)]
pub struct TunnelStates {
    by_id: parking_lot::Mutex<std::collections::HashMap<String, TunnelState>>,
}

pub type SharedTunnelStates = std::sync::Arc<TunnelStates>;

impl TunnelStates {
    pub fn set(&self, tunnel_id: &str, state: TunnelState) {
        self.by_id.lock().insert(tunnel_id.to_string(), state);
    }

    pub fn forget(&self, tunnel_id: &str) {
        self.by_id.lock().remove(tunnel_id);
    }

    pub fn apply(&self, tunnels: &mut [SessionTunnel]) {
        let map = self.by_id.lock();
        for t in tunnels {
            if let Some(state) = map.get(&t.id) {
                t.state = state.clone();
            }
        }
    }
}

pub fn added_risky_tunnel<'a>(prev: &[Tunnel], next: &'a [Tunnel]) -> Option<&'a Tunnel> {
    next.iter()
        .find(|t| t.kind.needs_confirmation() && !prev.iter().any(|p| p.exposure() == t.exposure()))
}

pub fn control_master_available() -> bool {
    cfg!(unix)
}

fn control(alias: &str, op: &str, t: &Tunnel) -> Result<std::process::Output, AppError> {
    let args = t.cli_args()?;
    std::process::Command::new("ssh")
        .args(["-O", op])
        .args(&args)
        .arg(alias)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| AppError::new("ssh.tunnel_control_failed").with("detail", e.to_string()))
}

pub fn open_on_master(alias: &str, t: &Tunnel) -> Result<(), AppError> {
    let out = control(alias, "forward", t)?;
    if out.status.success() {
        return Ok(());
    }
    Err(AppError::new("ssh.tunnel_open_failed")
        .with("port", t.listen_port.to_string())
        .with("detail", control_reason(&out.stderr)))
}

pub fn close_on_master(alias: &str, t: &Tunnel) -> Result<(), AppError> {
    control(alias, "cancel", t)?;
    Ok(())
}

pub fn local_port_free(t: &Tunnel) -> bool {
    if !t.kind.binds_locally() {
        return true;
    }
    std::net::TcpListener::bind((t.bind_address(), t.listen_port)).is_ok()
}

fn control_reason(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .find(|l| l.contains("forwarding request failed"))
        .and_then(|l| l.rsplit(':').next().map(str::trim))
        .filter(|s| !s.is_empty())
        .unwrap_or("o ssh recusou o forward")
        .to_string()
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
            "    LocalForward 127.0.0.1:5432 localhost:5432\n"
        );
    }

    #[test]
    fn config_line_do_remote() {
        assert_eq!(
            remote(8000).config_line().unwrap(),
            "    RemoteForward 127.0.0.1:8000 localhost:3000\n"
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
        assert_eq!(
            d.config_line().unwrap(),
            "    DynamicForward 127.0.0.1:1080\n"
        );
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
            vec![
                "-L".to_string(),
                "127.0.0.1:5432:localhost:5432".to_string()
            ]
        );
        assert_eq!(
            remote(8000).cli_args().unwrap(),
            vec![
                "-R".to_string(),
                "127.0.0.1:8000:localhost:3000".to_string()
            ]
        );
    }

    #[test]
    fn nenhuma_forma_de_porta_nua_escapa_do_writer_nem_do_cli() {
        let sem_host = Tunnel {
            listen_host: None,
            ..local(5432)
        };
        assert_eq!(
            sem_host.config_line().unwrap(),
            "    LocalForward 127.0.0.1:5432 localhost:5432\n",
            "medido na VPS: com porta nua o ssh resolve localhost para ::1 E \
             127.0.0.1, liga no que conseguir e devolve 0 — o dono acaba falando \
             com quem tomou a porta achando que está na prod. Ver ADR \
             ssh-tunel-bind-explicito"
        );
        assert_eq!(
            sem_host.cli_args().unwrap(),
            vec![
                "-L".to_string(),
                "127.0.0.1:5432:localhost:5432".to_string()
            ],
            "o bind explícito é o que faz o exit code do -O forward virar oráculo: \
             sem ele o ssh mente 0 com a porta tomada"
        );

        let dynamic = Tunnel {
            kind: TunnelKind::Dynamic,
            listen_port: 1080,
            listen_host: None,
            target_host: None,
            target_port: None,
        };
        assert_eq!(
            dynamic.config_line().unwrap(),
            "    DynamicForward 127.0.0.1:1080\n"
        );
    }

    #[test]
    fn bind_explicito_do_dono_vence_o_padrao() {
        let t = Tunnel {
            listen_host: Some("0.0.0.0".into()),
            ..local(5432)
        };
        assert_eq!(t.bind_address(), "0.0.0.0");
        assert!(t.config_line().unwrap().contains("0.0.0.0:5432"));
    }

    #[test]
    fn a_razao_da_recusa_do_ssh_vira_texto_do_dono() {
        let stderr = b"mux_client_forward: forwarding request failed: Port forwarding failed\n\
                       muxclient: master forward request failed\n";
        assert_eq!(control_reason(stderr), "Port forwarding failed");
    }

    #[test]
    fn recusa_sem_motivo_reconhecivel_ainda_diz_algo() {
        assert!(
            !control_reason(b"barulho qualquer").is_empty(),
            "erro sem detalhe não pode virar string vazia na UI: o dono ficaria \
             com um túnel vermelho e nenhuma pista"
        );
    }

    #[test]
    fn o_estado_do_tunel_e_eixo_proprio_e_serializa_com_o_motivo() {
        let json = serde_json::to_string(&TunnelState::Error {
            detail: "Port forwarding failed".into(),
        })
        .unwrap();
        assert!(json.contains("\"state\":\"error\""), "got: {json}");
        assert!(
            json.contains("Port forwarding failed"),
            "o motivo viaja junto do estado: falhar é ruidoso, nunca silencioso. got: {json}"
        );
    }

    #[test]
    #[ignore = "usa rede: exige TYBA_E2E_SSH_ALIAS com ControlMaster de pé"]
    fn na_vps_real_o_tunel_sobe_e_a_porta_tomada_vira_erro_visivel() {
        let alias = std::env::var("TYBA_E2E_SSH_ALIAS").expect("TYBA_E2E_SSH_ALIAS");
        let t = Tunnel {
            kind: TunnelKind::Local,
            listen_port: 15493,
            listen_host: None,
            target_host: Some("localhost".into()),
            target_port: Some(22),
        };
        let _ = close_on_master(&alias, &t);

        open_on_master(&alias, &t).expect("o túnel sobe no master vivo");
        let mut s = std::net::TcpStream::connect(("127.0.0.1", 15493))
            .expect("a porta que o dono digitou tem que ser a que atende");
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0u8; 8];
        let n = std::io::Read::read(&mut s, &mut buf).unwrap();
        assert!(
            buf[..n].starts_with(b"SSH-"),
            "o byte tem que atravessar até o sshd remoto"
        );
        close_on_master(&alias, &t).unwrap();

        let squatter = std::net::TcpListener::bind(("127.0.0.1", 15493)).expect("impostor senta");
        let err = open_on_master(&alias, &t)
            .expect_err("porta tomada TEM que virar erro: com bind explícito o ssh devolve 255");
        assert_eq!(err.code, "ssh.tunnel_open_failed");
        assert_eq!(
            err.params.get("port").map(String::as_str),
            Some("15493"),
            "o erro diz QUAL porta falhou; o painel mostra isso no túnel certo"
        );
        drop(squatter);
    }

    #[test]
    fn pre_voo_ve_a_porta_tomada_no_endereco_que_o_tunel_usa() {
        let squatter = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = squatter.local_addr().unwrap().port();
        let t = Tunnel {
            listen_port: port,
            ..local(port)
        };
        assert!(
            !local_port_free(&t),
            "o pré-voo pergunta pelo bind_address do túnel (127.0.0.1), que é \
             exatamente o endereço que o ssh vai tentar — perguntar por outro \
             endereço aprovaria uma porta que o ssh não consegue ligar"
        );
        drop(squatter);
        assert!(
            local_port_free(&t),
            "solta o impostor e a porta volta a estar livre — o pré-voo lê o \
             estado real, não um palpite. A porta vem de um bind em :0 (o SO dá \
             uma livre e exclusiva) em vez de número fixo, senão dois testes em \
             paralelo brigam pela mesma e um flakeia — foi o que derrubou o \
             macOS no corte da v0.1.4"
        );
    }

    #[test]
    fn o_estado_vivo_vence_o_opening_que_vem_do_disco() {
        let states = TunnelStates::default();
        let mut carregados = vec![SessionTunnel {
            id: "t1".into(),
            session_id: uuid::Uuid::nil(),
            tunnel: local(5432),
            state: TunnelState::Opening,
            created_at: chrono::Utc::now(),
        }];
        states.set("t1", TunnelState::Live);

        states.apply(&mut carregados);

        assert_eq!(
            carregados[0].state,
            TunnelState::Live,
            "o store não guarda estado (de propósito) e devolve Opening sempre; \
             sem o mapa vivo por cima, o painel mostra 'abrindo' eternamente num \
             túnel de pé — e o vermelho da porta tomada nunca apareceria"
        );
    }

    #[test]
    fn tunel_esquecido_volta_a_ser_o_que_o_disco_diz() {
        let states = TunnelStates::default();
        states.set("t1", TunnelState::Live);
        states.forget("t1");
        let mut carregados = vec![SessionTunnel {
            id: "t1".into(),
            session_id: uuid::Uuid::nil(),
            tunnel: local(5432),
            state: TunnelState::Opening,
            created_at: chrono::Utc::now(),
        }];

        states.apply(&mut carregados);

        assert_eq!(
            carregados[0].state,
            TunnelState::Opening,
            "estado de túnel fechado não pode ficar pendurado no mapa: um id \
             reusado herdaria Live sem ninguém ter aberto nada"
        );
    }

    #[test]
    fn o_pre_voo_nao_opina_sobre_o_remote_forward() {
        let squatter = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = squatter.local_addr().unwrap().port();
        let r = Tunnel {
            kind: TunnelKind::Remote,
            listen_port: port,
            listen_host: None,
            target_host: Some("localhost".into()),
            target_port: Some(3000),
        };

        assert!(
            local_port_free(&r),
            "o -R escuta no HOST REMOTO: perguntar pela porta local é pergunta \
             errada. Sem isto, abrir -R 15500 no Windows seria recusado porque o \
             próprio -L 15500 do dono segura a porta aqui"
        );
        assert!(
            !local_port_free(&Tunnel {
                kind: TunnelKind::Dynamic,
                listen_port: port,
                listen_host: None,
                target_host: None,
                target_port: None,
            }),
            "-D escuta local (é proxy SOCKS na máquina do dono), então o pré-voo \
             vale — este eixo NÃO é o reach() do ADR, onde -D anda com -R"
        );
        drop(squatter);
    }

    #[test]
    fn tunel_de_host_r_novo_exige_confirmacao() {
        let novo = remote(8000);
        assert_eq!(
            added_risky_tunnel(&[], std::slice::from_ref(&novo)),
            Some(&novo),
            "salvar o cadastro com -R novo é criar o túnel: o gate do ADR \
             risco-pela-direcao vale no create/update_host igual vale no \
             open_session_tunnel — sem isto a UI do cadastro vira bypass"
        );
    }

    #[test]
    fn tunel_de_host_r_ja_salvo_nao_repergunta() {
        let salvo = remote(8000);
        assert_eq!(
            added_risky_tunnel(std::slice::from_ref(&salvo), std::slice::from_ref(&salvo)),
            None,
            "o dono já disse sim na criação; reperguntar a cada edição de \
             cor/nota treina o clique automático — mesma regra do reattach"
        );
    }

    #[test]
    fn tunel_de_host_l_novo_passa_sem_gate() {
        assert_eq!(added_risky_tunnel(&[], &[local(5432)]), None);
    }

    #[test]
    fn remover_tunel_nunca_pergunta() {
        assert_eq!(added_risky_tunnel(&[remote(8000)], &[]), None);
    }

    #[test]
    fn bind_implicito_e_explicito_sao_a_mesma_exposicao() {
        let salvo = remote(8000);
        let explicito = Tunnel {
            listen_host: Some("127.0.0.1".into()),
            ..remote(8000)
        };
        assert_eq!(
            added_risky_tunnel(std::slice::from_ref(&salvo), &[explicito]),
            None,
            "a comparação é por exposição (kind, bind efetivo, portas, alvo), \
             não por igualdade de struct: None e Some(127.0.0.1) são o mesmo \
             forward, e campo cosmético futuro no Tunnel não pode reperguntar \
             um -R já aprovado — gate que repergunta à toa treina o Sim \
             automático"
        );
    }

    #[test]
    fn d_novo_ao_lado_de_r_antigo_e_o_que_gateia() {
        let antigo = remote(8000);
        let novo_d = Tunnel {
            kind: TunnelKind::Dynamic,
            listen_port: 1080,
            listen_host: None,
            target_host: None,
            target_port: None,
        };
        let next = vec![antigo.clone(), novo_d.clone()];
        assert_eq!(
            added_risky_tunnel(std::slice::from_ref(&antigo), &next),
            Some(&novo_d)
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
