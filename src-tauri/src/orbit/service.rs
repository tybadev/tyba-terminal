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
        /// O id de quando o dono escolheu. Não é a identidade: o `compose up`
        /// que recria o contêiner troca o id (ver [`find_container`]).
        #[serde(default)]
        container_id: String,
        /// Nome do contêiner (único no docker): a identidade sem compose.
        #[serde(default)]
        name: String,
        /// Projeto e serviço do compose: a identidade que sobrevive ao
        /// contêiner recriado. Vazios fora do compose.
        #[serde(default)]
        project: String,
        #[serde(default)]
        service: String,
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
        /// Host SSH (id do cadastro) e porta local do `-L`: a identidade que
        /// sobrevive à sessão fechada e ao túnel aberto de novo (id novo).
        #[serde(default)]
        host_id: String,
        #[serde(default)]
        listen_port: u16,
    },
}

impl ServiceSource {
    /// Mesmo alvo com outro id efêmero: o contêiner recriado (mesmo nome ou
    /// mesmo serviço do compose) ou o túnel reaberto (mesmo host e porta).
    /// O card copia o id novo para o nó, e isso não é config mudada.
    pub fn same_target(&self, other: &ServiceSource) -> bool {
        use ServiceSource::*;
        match (self, other) {
            (
                Container {
                    name: n1,
                    project: p1,
                    service: s1,
                    ..
                },
                Container {
                    name: n2,
                    project: p2,
                    service: s2,
                    ..
                },
            ) => {
                if !p1.is_empty() || !p2.is_empty() {
                    p1 == p2 && s1 == s2
                } else {
                    !n1.is_empty() && n1 == n2
                }
            }
            (
                Tunnel {
                    host_id: h1,
                    listen_port: l1,
                    ..
                },
                Tunnel {
                    host_id: h2,
                    listen_port: l2,
                    ..
                },
            ) => !h1.is_empty() && h1 == h2 && *l1 > 0 && l1 == l2,
            _ => self == other,
        }
    }
}

/// Acha o contêiner do nó na lista do docker (`ps -a`): pelo serviço do
/// compose, depois pelo nome, depois pelo id gravado. Entre vários do mesmo
/// serviço (escala), o que roda.
pub fn find_container<'a>(
    list: &'a [crate::docker::ContainerInfo],
    container_id: &str,
    name: &str,
    project: &str,
    service: &str,
) -> Option<&'a crate::docker::ContainerInfo> {
    let pick = |hits: Vec<&'a crate::docker::ContainerInfo>| {
        hits.iter()
            .find(|c| c.state == "running")
            .or(hits.first())
            .copied()
    };
    if !project.is_empty() && !service.is_empty() {
        let hits: Vec<_> = list
            .iter()
            .filter(|c| {
                c.compose_project.as_deref() == Some(project)
                    && c.service.as_deref() == Some(service)
            })
            .collect();
        if let Some(c) = pick(hits) {
            return Some(c);
        }
    }
    if !name.is_empty() {
        if let Some(c) = list.iter().find(|c| c.name == name) {
            return Some(c);
        }
    }
    list.iter()
        .find(|c| !container_id.is_empty() && c.id == container_id)
}

/// O estado vivo do serviço no card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LiveState {
    /// No ar: o contêiner roda, a porta escuta, o túnel está de pé.
    Up,
    /// Caiu: saiu com erro, reinicia em laço, morreu, sumiu ou o túnel deu erro.
    Down,
    /// Parado: saiu limpo, nunca subiu, nada escuta, túnel fechado.
    Stopped,
}

/// O que o card mostra do serviço agora.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceLive {
    pub state: LiveState,
    /// O motivo curto ("saiu com 1", "túnel fechado"), redigido.
    pub detail: Option<String>,
    /// "Subir" vale: contêiner parado que o `docker start` sobe.
    pub can_start: bool,
    /// O id de agora do contêiner (o log ao vivo segue este).
    pub container_id: Option<String>,
}

impl ServiceLive {
    fn new(state: LiveState, detail: Option<String>) -> Self {
        Self {
            state,
            detail: detail.map(|d| crate::session::redact::redact(&d).into_owned()),
            can_start: false,
            container_id: None,
        }
    }
}

/// `Exited (137) 2 minutes ago` → 137.
fn exit_code(status: &str) -> Option<i32> {
    let rest = status.strip_prefix("Exited (")?;
    rest[..rest.find(')')?].parse().ok()
}

/// Estado do `docker ps` → card. `docker stop` de quem ignora o SIGTERM sai
/// com 137 e conta como caiu: o docker não diz quem parou.
pub fn container_live(c: &crate::docker::ContainerInfo) -> ServiceLive {
    let mut live = match c.state.as_str() {
        "running" => ServiceLive::new(LiveState::Up, None),
        "restarting" => ServiceLive::new(LiveState::Down, Some("reiniciando em laço".into())),
        "dead" => ServiceLive::new(LiveState::Down, Some("morto".into())),
        "exited" => match exit_code(&c.status) {
            Some(0) | None => ServiceLive::new(LiveState::Stopped, None),
            Some(code) => ServiceLive::new(LiveState::Down, Some(format!("saiu com {code}"))),
        },
        "paused" => ServiceLive::new(LiveState::Stopped, Some("pausado".into())),
        _ => ServiceLive::new(LiveState::Stopped, None),
    };
    live.can_start = matches!(c.state.as_str(), "exited" | "created");
    live.container_id = Some(c.id.clone());
    live
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
        ServiceSource::Container {
            container_id,
            name,
            project,
            service,
        } => {
            let label = if name.is_empty() { container_id } else { name };
            if container_id.is_empty() && name.is_empty() && project.is_empty() {
                return Err("escolha o contêiner do serviço".into());
            }
            let list = state
                .docker
                .list(None, true, None)
                .map_err(|e| format!("ler os contêineres: {e}"))?;
            let container_id = &find_container(&list, container_id, name, project, service)
                .ok_or_else(|| format!("contêiner {label} não encontrado"))?
                .id;
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
            // Sessão que já fechou: vale quem escutar na porta (a porta é a
            // identidade; a sessão, um filtro).
            let session_id = session_id.as_ref().filter(|id| {
                id.parse::<crate::session::SessionId>()
                    .is_ok_and(|id| state.sessions.get(id).is_some())
            });
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
        ServiceSource::Tunnel { tunnel_id, .. } => {
            if tunnel_id.is_empty() {
                return Err("escolha o túnel do serviço".into());
            }
            let tunnel = find_tunnel(&state, source)?
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

/// O túnel do nó: pelo id na sessão gravada; senão, num túnel `-L` de
/// qualquer sessão viva do mesmo host com a mesma porta local (a sessão
/// fechou ou o túnel foi reaberto). `None`: não há.
fn find_tunnel(
    state: &crate::AppState,
    source: &ServiceSource,
) -> Result<Option<crate::ssh::tunnel::SessionTunnel>, String> {
    let ServiceSource::Tunnel {
        session_id,
        tunnel_id,
        host_id,
        listen_port,
    } = source
    else {
        return Ok(None);
    };
    let load = |id: crate::session::SessionId| {
        let mut tunnels = state
            .store
            .load_session_tunnels(id)
            .map_err(|e| format!("ler os túneis: {e}"))?;
        state.tunnel_states.apply(&mut tunnels);
        Ok::<_, String>(tunnels)
    };
    if let Ok(id) = session_id.parse::<crate::session::SessionId>() {
        if let Some(t) = load(id)?.into_iter().find(|t| &t.id == tunnel_id) {
            return Ok(Some(t));
        }
    }
    if host_id.is_empty() || *listen_port == 0 {
        return Ok(None);
    }
    for session in state.sessions.list() {
        let crate::session::SessionKind::Ssh { host_id: h } = &session.kind else {
            continue;
        };
        if h != host_id {
            continue;
        }
        if let Some(t) = load(session.id)?.into_iter().find(|t| {
            t.tunnel.kind == crate::ssh::tunnel::TunnelKind::Local
                && t.tunnel.listen_port == *listen_port
        }) {
            return Ok(Some(t));
        }
    }
    Ok(None)
}

/// O estado vivo do serviço para o card (no ar / caiu / parado), lido agora.
/// Bloqueia (docker, store).
pub fn probe(app: &tauri::AppHandle, source: &ServiceSource) -> ServiceLive {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    match source {
        ServiceSource::Container {
            container_id,
            name,
            project,
            service,
        } => {
            if container_id.is_empty() && name.is_empty() && project.is_empty() {
                return ServiceLive::new(LiveState::Stopped, None);
            }
            match state.docker.list(None, true, None) {
                Ok(list) => match find_container(&list, container_id, name, project, service) {
                    Some(c) => container_live(c),
                    None => {
                        ServiceLive::new(LiveState::Down, Some("contêiner não existe mais".into()))
                    }
                },
                Err(e) => ServiceLive::new(LiveState::Down, Some(format!("docker: {e}"))),
            }
        }
        ServiceSource::Port { port: 0, .. } => ServiceLive::new(LiveState::Stopped, None),
        ServiceSource::Port { .. } => match resolve(app, source) {
            Ok(_) => ServiceLive::new(LiveState::Up, None),
            Err(reason) => ServiceLive::new(LiveState::Stopped, Some(reason)),
        },
        ServiceSource::Tunnel { tunnel_id, .. } if tunnel_id.is_empty() => {
            ServiceLive::new(LiveState::Stopped, None)
        }
        ServiceSource::Tunnel { .. } => match find_tunnel(&state, source) {
            Ok(Some(t)) => match t.state {
                crate::ssh::tunnel::TunnelState::Live => ServiceLive::new(LiveState::Up, None),
                crate::ssh::tunnel::TunnelState::Opening => {
                    ServiceLive::new(LiveState::Stopped, Some("abrindo".into()))
                }
                crate::ssh::tunnel::TunnelState::Error { detail } => {
                    ServiceLive::new(LiveState::Down, Some(detail))
                }
            },
            Ok(None) => ServiceLive::new(LiveState::Stopped, Some("túnel fechado".into())),
            Err(e) => ServiceLive::new(LiveState::Down, Some(e)),
        },
    }
}

/// Contêineres que o Orbit subiu ("Subir") nesta execução do TYBA, e que
/// não rodavam antes. O ⌘Q para só estes; o que já rodava antes do Orbit
/// não é tocado.
#[derive(Default)]
pub struct StartedContainers(parking_lot::Mutex<Vec<String>>);

impl StartedContainers {
    pub fn record(&self, id: &str) {
        let mut ids = self.0.lock();
        if !ids.iter().any(|x| x == id) {
            ids.push(id.to_string());
        }
    }

    /// Os registrados, esvaziando o registro (a saída do app).
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock())
    }
}

/// O "Subir" precisa do `docker start`? Rodando: não (e não entra no
/// registro do ⌘Q: já rodava antes do Orbit). Parado ou criado: sim. O resto
/// (pausado, reiniciando, morto) recusa.
fn needs_start(c: &crate::docker::ContainerInfo) -> Result<bool, String> {
    if c.state == "running" {
        return Ok(false);
    }
    if !container_live(c).can_start {
        return Err(format!("contêiner {} está {}", c.name, c.state));
    }
    Ok(true)
}

/// "Subir": `docker start` do contêiner do nó, se ele não roda. Só
/// contêiner (porta e túnel não têm o que subir aqui). Registra em
/// [`StartedContainers`] para o ⌘Q. Bloqueia.
pub fn start(app: &tauri::AppHandle, source: &ServiceSource) -> Result<(), String> {
    use tauri::Manager;
    let ServiceSource::Container {
        container_id,
        name,
        project,
        service,
    } = source
    else {
        return Err("só o contêiner sobe pelo Orbit".into());
    };
    let state = app.state::<crate::AppState>();
    let list = state
        .docker
        .list(None, true, None)
        .map_err(|e| format!("ler os contêineres: {e}"))?;
    let c = find_container(&list, container_id, name, project, service)
        .ok_or("contêiner não existe mais: suba pelo compose na tela Docker")?;
    if !needs_start(c)? {
        return Ok(());
    }
    state
        .docker
        .container_op(&c.id, crate::docker::ContainerOp::Start, None)
        .map_err(|e| format!("subir {}: {e}", c.name))?;
    app.state::<StartedContainers>().record(&c.id);
    eprintln!("[orbit] contêiner {} subido pelo Orbit", c.name);
    Ok(())
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
                name: "api".into(),
                project: String::new(),
                service: String::new(),
            },
            "grafo salvo antes da identidade estável ainda lê"
        );
    }

    fn info(
        id: &str,
        name: &str,
        state: &str,
        status: &str,
        compose: Option<(&str, &str)>,
    ) -> crate::docker::ContainerInfo {
        crate::docker::ContainerInfo {
            id: id.into(),
            name: name.into(),
            image: "img".into(),
            state: state.into(),
            status: status.into(),
            ports: String::new(),
            compose_project: compose.map(|c| c.0.into()),
            compose_working_dir: None,
            service: compose.map(|c| c.1.into()),
            config_files: None,
            working_dir_status: None,
            compose_file_status: None,
            in_repo: false,
        }
    }

    /// O `compose up` recriou o contêiner: id novo, mesmo serviço. O nó o
    /// acha pelo compose; sem compose, pelo nome; só por último pelo id.
    #[test]
    fn container_is_found_by_compose_then_name_then_id() {
        let list = vec![
            info(
                "new1",
                "muriki-api-1",
                "exited",
                "Exited (0) 1 minute ago",
                Some(("muriki", "api")),
            ),
            info(
                "new2",
                "muriki-api-2",
                "running",
                "Up 3 minutes",
                Some(("muriki", "api")),
            ),
            info("pg", "pg", "running", "Up", None),
        ];
        let id = |c: Option<&crate::docker::ContainerInfo>| c.map(|c| c.id.clone());
        assert_eq!(
            id(find_container(
                &list,
                "old",
                "muriki-api-1",
                "muriki",
                "api"
            )),
            Some("new2".into()),
            "o que roda"
        );
        assert_eq!(
            id(find_container(&list, "old", "pg", "", "")),
            Some("pg".into())
        );
        assert_eq!(
            id(find_container(&list, "pg", "", "", "")),
            Some("pg".into())
        );
        assert_eq!(id(find_container(&list, "old", "sumiu", "", "")), None);
        assert_eq!(id(find_container(&list, "", "", "outro", "api")), None);
    }

    #[test]
    fn container_live_maps_docker_state() {
        let live = |state, status| container_live(&info("c", "c", state, status, None));
        assert_eq!(live("running", "Up 2 minutes").state, LiveState::Up);
        assert!(!live("running", "Up").can_start);
        let crashed = live("exited", "Exited (1) 3 seconds ago");
        assert_eq!(
            (crashed.state, crashed.detail.as_deref(), crashed.can_start),
            (LiveState::Down, Some("saiu com 1"), true)
        );
        let clean = live("exited", "Exited (0) 1 hour ago");
        assert_eq!((clean.state, clean.can_start), (LiveState::Stopped, true));
        assert_eq!(live("created", "Created").state, LiveState::Stopped);
        assert!(live("created", "Created").can_start);
        assert_eq!(live("restarting", "Restarting (1)").state, LiveState::Down);
        assert_eq!(live("dead", "Dead").state, LiveState::Down);
        assert!(!live("paused", "Up (Paused)").can_start);
        assert_eq!(live("running", "Up").container_id.as_deref(), Some("c"));
    }

    #[test]
    fn same_target_ignores_the_ephemeral_id() {
        let c = |id: &str, name: &str, project: &str| ServiceSource::Container {
            container_id: id.into(),
            name: name.into(),
            project: project.into(),
            service: "api".into(),
        };
        assert!(c("a", "x", "muriki").same_target(&c("b", "y", "muriki")));
        assert!(!c("a", "x", "muriki").same_target(&c("a", "x", "outro")));
        assert!(c("a", "pg", "").same_target(&c("b", "pg", "")));
        assert!(!c("a", "", "").same_target(&c("b", "", "")));
        let t = |tid: &str, port| ServiceSource::Tunnel {
            session_id: "s".into(),
            tunnel_id: tid.into(),
            host_id: "h".into(),
            listen_port: port,
        };
        assert!(t("1", 5173).same_target(&t("2", 5173)));
        assert!(!t("1", 5173).same_target(&t("2", 5174)));
    }

    /// Só o que estava parado sobe e entra no registro do ⌘Q; o que já
    /// rodava fica fora dele.
    #[test]
    fn only_a_stopped_container_needs_start() {
        let c = |state, status| info("c", "api", state, status, None);
        assert_eq!(needs_start(&c("running", "Up 2 minutes")), Ok(false));
        assert_eq!(
            needs_start(&c("exited", "Exited (1) 1 minute ago")),
            Ok(true)
        );
        assert_eq!(needs_start(&c("exited", "Exited (0) 1 hour ago")), Ok(true));
        assert_eq!(needs_start(&c("created", "Created")), Ok(true));
        assert!(needs_start(&c("paused", "Up (Paused)")).is_err());
        assert!(needs_start(&c("restarting", "Restarting (1)")).is_err());
        assert!(needs_start(&c("dead", "Dead")).is_err());
    }

    #[test]
    fn started_containers_are_recorded_once_and_taken() {
        let started = StartedContainers::default();
        started.record("a");
        started.record("a");
        started.record("b");
        assert_eq!(started.take(), vec!["a".to_string(), "b".to_string()]);
        assert!(started.take().is_empty());
    }
}
