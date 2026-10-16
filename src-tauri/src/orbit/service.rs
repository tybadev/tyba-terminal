//! Navegador e Serviço: a URL local e o serviço que já roda.
//!
//! O canvas não vira navegador da internet: toda URL que o Orbit aceita
//! (a digitada no nó, a que chega por ligação, a montada de porta ou túnel)
//! passa por [`is_local_url`] — no salvar e na execução. O front repete a
//! regra para avisar, mas quem decide é aqui.

use serde::{Deserialize, Serialize};

/// De onde o nó de Serviço lê o serviço que já roda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ServiceSource {
    /// Contêiner docker local: log ao vivo e a porta publicada.
    Container {
        #[serde(default)]
        container_id: String,
        /// Nome mostrado (o id é o que vale).
        #[serde(default)]
        name: String,
    },
    /// Porta local; com sessão, a porta que o core detectou nela.
    Port {
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        port: u16,
    },
    /// Túnel SSH `-L` de uma sessão: o lado local dele.
    Tunnel {
        #[serde(default)]
        session_id: String,
        #[serde(default)]
        tunnel_id: String,
    },
}

/// `http://localhost:<porta>`.
pub fn local_url(port: u16) -> String {
    format!("http://localhost:{port}")
}

/// Os hosts que [`is_local_url`] aceita, como o agente e o dono leem: a
/// descrição das ferramentas e as mensagens de recusa saem daqui, para não
/// contradizerem a regra (o modelo lê o schema como contrato).
pub const LOCAL_HOSTS: &str = "localhost, 127.0.0.1, [::1], *.test, *.localhost";

/// Domínio de desenvolvimento: `*.test` e `*.localhost` (RFC 6761: TLDs
/// reservados, nunca resolvem na internet). O dono usa `https://code.muriki.test`
/// pelo Caddy, e entrar por `localhost:5180` quebra o login. `.local` fica de
/// fora de propósito: é mDNS e alcança outras máquinas da rede.
fn is_dev_domain(host: &str) -> bool {
    let Some(name) = host
        .strip_suffix(".test")
        .or_else(|| host.strip_suffix(".localhost"))
    else {
        return false;
    };
    !name.is_empty()
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// Só `http`/`https` para [`LOCAL_HOSTS`], porta opcional (1–65535). Recusa
/// usuário na URL (`http://localhost@outro`), contrabarra (o navegador a lê
/// como `/`), espaço e caractere de controle: o que passa aqui é exatamente
/// o host que o webview vai abrir.
pub fn is_local_url(url: &str) -> bool {
    if url
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
    else {
        return false;
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.contains('@') {
        return false;
    }
    let (host, port) = match authority.strip_prefix("[::1]") {
        Some(after) => ("[::1]", after),
        None => match authority.rfind(':') {
            Some(i) => (&authority[..i], &authority[i..]),
            None => (authority, ""),
        },
    };
    if !matches!(host, "localhost" | "127.0.0.1" | "[::1]") && !is_dev_domain(host) {
        return false;
    }
    match port.strip_prefix(':') {
        None => port.is_empty(),
        Some(p) => {
            !p.is_empty()
                && p.len() <= 5
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u32>().is_ok_and(|n| (1..=65535).contains(&n))
        }
    }
}

/// URL do contêiner: a primeira porta publicada no host local, na forma que
/// o `docker inspect` do TYBA formata (`localhost:15432 → 5432`). Porta
/// publicada só em outro IP não conta.
pub fn container_url(ports: &[String]) -> Option<String> {
    ports.iter().find_map(|p| {
        let outside = p.split(" → ").next()?;
        let port = outside.strip_prefix("localhost:")?;
        port.parse::<u16>().ok().filter(|p| *p > 0).map(local_url)
    })
}

/// URL do túnel: só o `-L` serve uma página aqui (o `-R` escuta no servidor,
/// o `-D` é SOCKS).
pub fn tunnel_url(tunnel: &crate::ssh::tunnel::Tunnel) -> Result<String, String> {
    use crate::ssh::tunnel::TunnelKind;
    match tunnel.kind {
        TunnelKind::Local if tunnel.listen_port > 0 => Ok(local_url(tunnel.listen_port)),
        TunnelKind::Local => Err("túnel sem porta local".into()),
        TunnelKind::Remote => Err("túnel -R escuta no servidor, não aqui".into()),
        TunnelKind::Dynamic => Err("túnel -D é SOCKS, não serve página".into()),
    }
}

/// Primeira porta da lista (a menor), como URL. É a que o processo abriu.
pub fn first_port_url(ports: &[u16]) -> Option<String> {
    ports
        .iter()
        .copied()
        .filter(|p| *p > 0)
        .min()
        .map(local_url)
}

/// O que o serviço achou na execução.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceReady {
    pub url: Option<String>,
    /// Texto do primeiro lote de log (já redigido), quando o serviço tem log.
    pub log: Option<String>,
}

/// Seguidores de `docker logs -f` do Orbit. Separados dos da aba de logs:
/// aqueles o layout poda a cada mudança (`LogsManager::retain`), e o nó do
/// serviço não é aba. Estes vivem enquanto o canvas mostra o nó (o front chama
/// `orbit_service_logs_retain`) e morrem na saída do app.
#[derive(Default, Clone)]
pub struct ServiceLogs(pub std::sync::Arc<crate::docker::logs::LogsManager>);

/// Evento das linhas novas do contêiner no nó do serviço.
pub fn logs_event(container_id: &str) -> String {
    format!("orbit-logs://{container_id}")
}

/// Liga (ou reaproveita) o seguidor do contêiner: o mesmo caminho do
/// `docker_logs_start`, com evento próprio. Bloqueia no `docker inspect`.
pub fn start_logs(
    app: &tauri::AppHandle,
    container_id: &str,
) -> Result<crate::docker::logs::LogsSnapshot, String> {
    use tauri::{Emitter, Manager};
    if !crate::docker::is_valid_container_id(container_id) {
        return Err("id de contêiner inválido".into());
    }
    let state = app.state::<crate::AppState>();
    let logs = app.state::<ServiceLogs>();
    let mapping = state
        .docker
        .detail(container_id, None)
        .map(|d| crate::docker::logs::PathMapping {
            workdir: d.workdir,
            mounts: d.mounts,
        })
        .unwrap_or_default();
    let event = logs_event(container_id);
    let handle = app.clone();
    let emit: crate::docker::logs::Emit = std::sync::Arc::new(move |batch| {
        let _ = handle.emit(&event, batch);
    });
    logs.0.start(container_id, mapping, emit, |since| {
        crate::docker::spawn_logs_follow(container_id, since.as_deref(), None)
            .map_err(|e| e.to_string())
    })
}

/// Quanto o serviço de contêiner espera o primeiro lote de log. Contêiner
/// calado não segura o grafo: passado isto, fica pronto com log vazio.
const FIRST_LOGS_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Resolve o serviço no app. Pronto, por fonte:
/// - **Container**: `docker inspect` diz `running` e o primeiro lote do
///   `docker logs` chegou (ou [`FIRST_LOGS_WAIT`] sem linha). `log` = esse
///   lote, redigido; `url` = a primeira porta publicada no host local.
/// - **Port**: a porta escuta (na sessão, se escolhida). `url` = ela; sem log.
/// - **Tunnel**: o túnel `-L` existe e não está em erro. `url` = o lado local.
pub fn resolve(app: &tauri::AppHandle, source: &ServiceSource) -> Result<ServiceReady, String> {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    match source {
        ServiceSource::Container { container_id, name } => {
            let label = if name.is_empty() { container_id } else { name };
            if container_id.is_empty() {
                return Err("escolha o contêiner do serviço".into());
            }
            let detail = state
                .docker
                .detail(container_id, None)
                .map_err(|e| format!("contêiner {label} não encontrado: {e}"))?;
            if detail.state != "running" {
                return Err(format!(
                    "contêiner {label} não está rodando ({})",
                    detail.state
                ));
            }
            start_logs(app, container_id)?;
            let logs = app.state::<ServiceLogs>();
            let deadline = std::time::Instant::now() + FIRST_LOGS_WAIT;
            let lines = loop {
                let snap = logs.0.snapshot(container_id);
                let (lines, following) = snap.map(|s| (s.lines, s.following)).unwrap_or_default();
                if !lines.is_empty() || !following || std::time::Instant::now() > deadline {
                    break lines;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            };
            let text: Vec<String> = lines
                .iter()
                .map(|l| match &l.ts {
                    Some(ts) => format!("{ts} {}", l.text),
                    None => l.text.clone(),
                })
                .collect();
            eprintln!("[orbit] serviço de contêiner pronto: {} linhas", text.len());
            Ok(ServiceReady {
                url: container_url(&detail.ports),
                log: Some(crate::docker::logs::export_text(&text)),
            })
        }
        ServiceSource::Port { session_id, port } => {
            if *port == 0 {
                return Err("escolha a porta do serviço".into());
            }
            let listening = match session_id {
                Some(id) => {
                    let id: crate::session::SessionId = id
                        .parse()
                        .map_err(|_| "sessão do serviço inválida".to_string())?;
                    state
                        .services
                        .chip()
                        .get(&id)
                        .is_some_and(|ports| ports.contains(port))
                }
                None => crate::services::listen::listeners()
                    .map_err(|e| format!("ler as portas: {e}"))?
                    .iter()
                    .any(|l| l.port == *port),
            };
            if !listening {
                return Err(match session_id {
                    Some(_) => format!("a sessão não está escutando na porta :{port}"),
                    None => format!("nada escuta na porta :{port}"),
                });
            }
            Ok(ServiceReady {
                url: Some(local_url(*port)),
                log: None,
            })
        }
        ServiceSource::Tunnel {
            session_id,
            tunnel_id,
        } => {
            if tunnel_id.is_empty() {
                return Err("escolha o túnel do serviço".into());
            }
            let id: crate::session::SessionId = session_id
                .parse()
                .map_err(|_| "sessão do túnel inválida".to_string())?;
            let mut tunnels = state
                .store
                .load_session_tunnels(id)
                .map_err(|e| format!("ler os túneis: {e}"))?;
            state.tunnel_states.apply(&mut tunnels);
            let tunnel = tunnels
                .into_iter()
                .find(|t| &t.id == tunnel_id)
                .ok_or("túnel não encontrado (fechado ou de outra máquina)")?;
            if let crate::ssh::tunnel::TunnelState::Error { detail } = &tunnel.state {
                return Err(format!("túnel com erro: {detail}"));
            }
            Ok(ServiceReady {
                url: Some(tunnel_url(&tunnel.tunnel)?),
                log: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_domains_test_and_localhost_are_local() {
        assert!(is_local_url("https://code.muriki.test"));
        assert!(is_local_url("https://api.muriki.test:8443/health"));
        assert!(is_local_url("http://app.localhost:5173/"));
        assert!(!is_local_url("https://test"));
        assert!(!is_local_url("https://.test"));
        assert!(!is_local_url("https://a..test"));
        assert!(!is_local_url("https://-a.test"));
        assert!(!is_local_url("https://printer.local"));
        assert!(!is_local_url("https://evil.test.example.com"));
        assert!(!is_local_url("https://user@code.muriki.test"));
        assert!(!is_local_url("https://muriki.test_x"));
    }
    use crate::ssh::tunnel::{Tunnel, TunnelKind};

    #[test]
    fn local_urls_pass() {
        for ok in [
            "http://localhost:5173",
            "http://localhost:5173/agenda?x=1#y",
            "http://localhost",
            "https://localhost:8443/",
            "http://127.0.0.1:3000",
            "http://127.0.0.1",
            "http://[::1]:8080/",
            "http://[::1]",
            "HTTP://LOCALHOST:5173",
        ] {
            assert!(is_local_url(ok), "{ok}");
        }
    }

    #[test]
    fn other_urls_are_refused() {
        for bad in [
            "http://192.168.0.10:5173",
            "http://10.0.0.1",
            "http://example.com",
            "https://example.com/localhost",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,oi",
            "ftp://localhost",
            "localhost:5173",
            "http://localhost.example.com",
            "http://localhost@example.com",
            "http://example.com@localhost",
            "http://localhost\\@example.com",
            "http://127.0.0.2",
            "http://0.0.0.0:5173",
            "http://[::2]:80",
            "http://localhost:0",
            "http://localhost:65536",
            "http://localhost:",
            "http://localhost:80a",
            "http://local host",
            "http://localhost\n.example.com",
            "",
        ] {
            assert!(!is_local_url(bad), "{bad:?}");
        }
    }

    #[test]
    fn container_url_uses_first_local_published_port() {
        let ports = vec![
            "10.0.0.5:9000 → 9000".to_string(),
            "localhost:15432 → 5432".to_string(),
            "localhost:3000 → 3000".to_string(),
        ];
        assert_eq!(
            container_url(&ports).as_deref(),
            Some("http://localhost:15432")
        );
        assert_eq!(container_url(&["10.0.0.5:9000 → 9000".into()]), None);
        assert_eq!(container_url(&[]), None);
    }

    #[test]
    fn tunnel_url_only_for_local_forward() {
        let tunnel = |kind, port| Tunnel {
            kind,
            listen_port: port,
            listen_host: None,
            target_host: Some("localhost".into()),
            target_port: Some(5173),
        };
        assert_eq!(
            tunnel_url(&tunnel(TunnelKind::Local, 15173)).as_deref(),
            Ok("http://localhost:15173")
        );
        assert!(tunnel_url(&tunnel(TunnelKind::Remote, 15173)).is_err());
        assert!(tunnel_url(&tunnel(TunnelKind::Dynamic, 1080)).is_err());
        assert!(tunnel_url(&tunnel(TunnelKind::Local, 0)).is_err());
    }

    #[test]
    fn port_url_is_the_lowest_port() {
        assert_eq!(
            first_port_url(&[5173, 24678]).as_deref(),
            Some("http://localhost:5173")
        );
        assert_eq!(
            first_port_url(&[24678, 5173]).as_deref(),
            Some("http://localhost:5173")
        );
        assert_eq!(first_port_url(&[]), None);
        assert!(is_local_url(&local_url(5173)));
    }

    #[test]
    fn source_serde_shape() {
        let s = ServiceSource::Port {
            session_id: None,
            port: 5173,
        };
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            serde_json::json!({"type": "port", "sessionId": null, "port": 5173})
        );
        let c: ServiceSource = serde_json::from_value(
            serde_json::json!({"type": "container", "containerId": "abc", "name": "api"}),
        )
        .unwrap();
        assert_eq!(
            c,
            ServiceSource::Container {
                container_id: "abc".into(),
                name: "api".into()
            }
        );
    }
}
