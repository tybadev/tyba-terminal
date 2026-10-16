//! Orbit: o Workspace como harness de agentes.
//!
//! Um grafo de nós (agente, processo, nota, integração) ligados por ligações
//! tipadas. Cada nó executável (agente, processo, integração) roda na própria
//! pasta (`dir` do nó); `Graph::dir` é só o padrão para nó sem pasta (grafos
//! salvos antes da pasta por nó). Não há worktree. O estado vive aqui no core; o front desenha e chama comandos.
//!
//! A validação deste módulo é a barreira: o front só oferece ligações válidas,
//! mas quem decide é `validate_graph`, chamado antes de gravar.

pub mod agent;
pub mod export;
pub mod integration;
pub mod mcp;
pub mod orchestrator;
pub mod run;
pub mod service;

pub use service::ServiceSource;

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Teto de nós por grafo (ver security: tetos).
pub const MAX_NODES: usize = 200;
/// Teto do script de uma integração, em bytes.
pub const MAX_SCRIPT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Graph {
    pub id: String,
    pub name: String,
    /// Pasta padrão: vale para nó executável sem `dir` próprio (grafos de
    /// antes da pasta por nó) e é onde o front sugere a pasta de nó novo.
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Substituem `{{nome}}` em `prompt`, `command` e `script` antes de rodar.
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: String,
    pub title: String,
    pub kind: NodeKind,
    pub position: Position,
    pub size: Size,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum NodeKind {
    Agent {
        runner: Runner,
        #[serde(default)]
        role: String,
        prompt: String,
        /// Pasta onde o agente roda. Vazio: `Graph::dir`.
        #[serde(default)]
        dir: String,
        /// Orquestrador: a sessão recebe as ferramentas do Orbit (servidor
        /// MCP `tyba _orbit-mcp`) e cria nós no run em vez de rodar tudo
        /// dentro dela. Ver [`orchestrator`].
        #[serde(default)]
        orchestrate: bool,
    },
    Process {
        command: String,
        #[serde(default)]
        output: OutputKind,
        #[serde(default)]
        file: Option<String>,
        /// Pasta onde o comando roda e de onde `file` é lido. Vazio: `Graph::dir`.
        #[serde(default)]
        dir: String,
    },
    Note {
        markdown: String,
    },
    Integration {
        integration_id: String,
        /// Pasta onde o script roda. Vazio: `Graph::dir`.
        #[serde(default)]
        dir: String,
    },
    /// Mostra uma página local. Recebe `url` (ou usa a dele); não produz nada.
    Browser {
        /// Endereço digitado no nó; vazio espera a ligação url.
        #[serde(default)]
        url: String,
    },
    /// Liga num serviço que já roda: produz `log` e `url`.
    Service {
        source: ServiceSource,
    },
}

impl NodeKind {
    /// A pasta própria do nó executável (pode estar vazia); `None` na nota.
    pub fn dir(&self) -> Option<&str> {
        match self {
            NodeKind::Agent { dir, .. }
            | NodeKind::Process { dir, .. }
            | NodeKind::Integration { dir, .. } => Some(dir),
            NodeKind::Note { .. } | NodeKind::Browser { .. } | NodeKind::Service { .. } => None,
        }
    }

    pub fn dir_mut(&mut self) -> Option<&mut String> {
        match self {
            NodeKind::Agent { dir, .. }
            | NodeKind::Process { dir, .. }
            | NodeKind::Integration { dir, .. } => Some(dir),
            NodeKind::Note { .. } | NodeKind::Browser { .. } | NodeKind::Service { .. } => None,
        }
    }
}

impl Graph {
    /// Onde o nó roda: a pasta dele, ou a padrão do grafo se ele não tem.
    pub fn node_dir<'a>(&'a self, node: &'a Node) -> &'a str {
        match node.kind.dir() {
            Some(dir) if !dir.is_empty() => dir,
            _ => &self.dir,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Runner {
    Claude,
    Codex,
}

/// Tipo do valor que uma ligação leva.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EdgeKind {
    Context,
    Diff,
    File,
    Log,
    Json,
    /// Texto com uma URL local (`out/url.txt`).
    Url,
    /// "plano": Orquestrador → a nota de plano dele (`orbit_plan`).
    Plan,
    /// "controla": agente → navegador; quem pode mexer no navegador.
    Control,
}

impl EdgeKind {
    const ALL: [EdgeKind; 8] = [
        EdgeKind::Context,
        EdgeKind::Diff,
        EdgeKind::File,
        EdgeKind::Log,
        EdgeKind::Json,
        EdgeKind::Url,
        EdgeKind::Plan,
        EdgeKind::Control,
    ];

    /// Leva valor na execução. `plan` e `control` só dizem quem é de quem:
    /// não viram entrada nem seguram o nó de destino.
    pub fn carries_data(self) -> bool {
        !matches!(self, EdgeKind::Plan | EdgeKind::Control)
    }

    /// Os tipos que levam valor (os que processo, agente e integração manual
    /// aceitam).
    fn data() -> Vec<EdgeKind> {
        Self::ALL.into_iter().filter(|k| k.carries_data()).collect()
    }
}

/// Saída declarada de um processo ou integração. `context` só o agente produz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputKind {
    #[default]
    Log,
    Json,
    File,
    Diff,
}

impl From<OutputKind> for EdgeKind {
    fn from(kind: OutputKind) -> Self {
        match kind {
            OutputKind::Log => EdgeKind::Log,
            OutputKind::Json => EdgeKind::Json,
            OutputKind::File => EdgeKind::File,
            OutputKind::Diff => EdgeKind::Diff,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Integration {
    pub id: String,
    pub name: String,
    /// Script bash do dono, rodado como processo na pasta do nó.
    pub script: String,
    pub trigger: Trigger,
    #[serde(default)]
    pub output: OutputKind,
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Trigger {
    /// Roda quando chega a entrada desse tipo.
    OnInput { kind: EdgeKind },
    /// Fica esperando o dono clicar ▶ no nó.
    Manual,
}

/// Grafo ou integração como saiu do banco. `consented = false` é conteúdo
/// importado que o dono ainda não confirmou: não roda.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored<T> {
    pub value: T,
    pub consented: bool,
    pub imported_hash: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OrbitError {
    #[error("grafo sem id")]
    EmptyGraphId,
    #[error("grafo com {0} nós passa do teto de {MAX_NODES}")]
    TooManyNodes(usize),
    #[error("nó sem id")]
    EmptyNodeId,
    #[error("id de nó repetido: {0}")]
    DuplicateNode(String),
    #[error("id de ligação repetido: {0}")]
    DuplicateEdge(String),
    #[error("ligação {edge} aponta para nó inexistente: {node}")]
    UnknownNode { edge: String, node: String },
    #[error("ligação {edge}: o nó {node} não produz {kind:?}")]
    NotProduced {
        edge: String,
        node: String,
        kind: EdgeKind,
    },
    #[error("ligação {edge}: o nó {node} não aceita {kind:?}")]
    NotAccepted {
        edge: String,
        node: String,
        kind: EdgeKind,
    },
    #[error("ligação {0} fecha um ciclo")]
    Cycle(String),
    #[error("nó {node} usa integração inexistente: {integration}")]
    UnknownIntegration { node: String, integration: String },
    #[error("integração sem id")]
    EmptyIntegrationId,
    #[error("integração sem nome")]
    EmptyIntegrationName,
    #[error("script da integração {0} passa de 64 KB")]
    ScriptTooLarge(String),
    #[error("{0} contém caractere de controle")]
    ControlChars(String),
    #[error("{0}: o arquivo de saída tem que ficar dentro da pasta (sem caminho absoluto nem ..)")]
    FileOutsideDir(String),
    #[error("nó {0}: escolha a pasta onde ele roda (caminho absoluto)")]
    NodeDir(String),
    #[error("nó {0}: só endereço local (http/https em {hosts})", hosts = service::LOCAL_HOSTS)]
    NotLocalUrl(String),
    #[error("nó {0}: id de contêiner inválido")]
    InvalidContainer(String),
}

/// Mesma regra de `snippet::validate`: `\n` e `\t` passam, o resto dos
/// caracteres de controle não. Texto daqui vira linha de comando ou prompt.
pub(crate) fn has_control_chars(text: &str) -> bool {
    text.chars()
        .any(|c| c != '\n' && c != '\t' && c.is_control())
}

fn check_text(what: impl FnOnce() -> String, text: &str) -> Result<(), OrbitError> {
    if has_control_chars(text) {
        return Err(OrbitError::ControlChars(what()));
    }
    Ok(())
}

/// O `file` de saída é lido de `dir.join(file)`: absoluto ou com `..` lê
/// fora da pasta do nó. Barra e contrabarra contam como separador nos
/// dois sistemas, e `C:` também é raiz.
fn check_file(what: impl FnOnce() -> String, file: Option<&str>) -> Result<(), OrbitError> {
    let Some(file) = file else { return Ok(()) };
    let bytes = file.as_bytes();
    let rooted = file.starts_with(['/', '\\'])
        || std::path::Path::new(file).is_absolute()
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':');
    if rooted || file.split(['/', '\\']).any(|part| part == "..") {
        return Err(OrbitError::FileOutsideDir(what()));
    }
    check_text(what, file)
}

pub fn validate_integration(integration: &Integration) -> Result<(), OrbitError> {
    if integration.id.trim().is_empty() {
        return Err(OrbitError::EmptyIntegrationId);
    }
    if integration.name.trim().is_empty() {
        return Err(OrbitError::EmptyIntegrationName);
    }
    if integration.script.len() > MAX_SCRIPT_BYTES {
        return Err(OrbitError::ScriptTooLarge(integration.id.clone()));
    }
    check_text(
        || format!("script da integração {}", integration.id),
        &integration.script,
    )?;
    check_file(
        || format!("integração {}", integration.id),
        integration.file.as_deref(),
    )
}

/// O que o nó produz (tabela de rules).
pub(crate) fn produces(
    kind: &NodeKind,
    integrations: &HashMap<&str, &Integration>,
) -> Vec<EdgeKind> {
    match kind {
        NodeKind::Agent { .. } => vec![
            EdgeKind::Context,
            EdgeKind::Diff,
            EdgeKind::Plan,
            EdgeKind::Control,
        ],
        // A url só fica disponível quando a sessão do processo abre porta.
        NodeKind::Process { output, .. } => vec![(*output).into(), EdgeKind::Url],
        // O texto da nota entra no prompt de quem a recebe (context) ou como
        // arquivo para um processo ler (file).
        NodeKind::Note { .. } => vec![EdgeKind::Context, EdgeKind::File],
        NodeKind::Integration { integration_id, .. } => integrations
            .get(integration_id.as_str())
            .map(|i| vec![i.output.into()])
            .unwrap_or_default(),
        NodeKind::Browser { .. } => Vec::new(),
        NodeKind::Service { .. } => vec![EdgeKind::Log, EdgeKind::Url],
    }
}

/// O que o nó aceita (tabela de rules).
fn accepts(kind: &NodeKind, integrations: &HashMap<&str, &Integration>) -> Vec<EdgeKind> {
    match kind {
        NodeKind::Agent { .. } | NodeKind::Process { .. } => EdgeKind::data(),
        NodeKind::Note { .. } => vec![EdgeKind::Plan],
        NodeKind::Service { .. } => Vec::new(),
        NodeKind::Browser { .. } => vec![EdgeKind::Url, EdgeKind::Control],
        NodeKind::Integration { integration_id, .. } => {
            match integrations.get(integration_id.as_str()).map(|i| i.trigger) {
                Some(Trigger::OnInput { kind }) if kind.carries_data() => vec![kind],
                Some(Trigger::OnInput { .. }) | None => Vec::new(),
                Some(Trigger::Manual) => EdgeKind::data(),
            }
        }
    }
}

/// Barreira do core antes de gravar ou rodar um grafo: a estrutura
/// ([`validate_structure`]) e a pasta de cada nó executável — não vazia e
/// absoluta (a do nó ou, sem ela, a padrão do grafo).
pub fn validate_graph(graph: &Graph, integrations: &[Integration]) -> Result<(), OrbitError> {
    validate_structure(graph, integrations)?;
    for node in &graph.nodes {
        if node.kind.dir().is_none() {
            continue;
        }
        let dir = graph.node_dir(node);
        if dir.trim().is_empty() || !std::path::Path::new(dir).is_absolute() {
            return Err(OrbitError::NodeDir(node.id.clone()));
        }
    }
    Ok(())
}

/// Ids únicos, teto de nós, integrações existentes, ligações com tipo aceito,
/// DAG e texto (inclusive as pastas) sem caractere de controle. Não olha se a
/// pasta existe nem se é absoluta: o arquivo importado traz a pasta de outra
/// máquina, e a importação pede a pasta antes de gravar.
pub fn validate_structure(graph: &Graph, integrations: &[Integration]) -> Result<(), OrbitError> {
    if graph.id.trim().is_empty() {
        return Err(OrbitError::EmptyGraphId);
    }
    if graph.nodes.len() > MAX_NODES {
        return Err(OrbitError::TooManyNodes(graph.nodes.len()));
    }
    // Variável é substituída dentro de comando, script e prompt: o valor
    // passaria por fora da checagem do texto se não fosse checado aqui.
    for (name, value) in &graph.variables {
        check_text(|| format!("variável {name}"), name)?;
        check_text(|| format!("variável {name}"), value)?;
    }
    check_text(|| "pasta do grafo".to_string(), &graph.dir)?;

    let by_id: HashMap<&str, &Integration> =
        integrations.iter().map(|i| (i.id.as_str(), i)).collect();

    let mut nodes: HashMap<&str, &Node> = HashMap::new();
    for node in &graph.nodes {
        if node.id.trim().is_empty() {
            return Err(OrbitError::EmptyNodeId);
        }
        if nodes.insert(node.id.as_str(), node).is_some() {
            return Err(OrbitError::DuplicateNode(node.id.clone()));
        }
        let what = || format!("nó {}", node.id);
        if let Some(dir) = node.kind.dir() {
            check_text(|| format!("pasta do nó {}", node.id), dir)?;
        }
        match &node.kind {
            NodeKind::Agent { prompt, .. } => check_text(what, prompt)?,
            NodeKind::Process { command, file, .. } => {
                check_text(what, command)?;
                check_file(|| format!("nó {}", node.id), file.as_deref())?;
            }
            NodeKind::Note { markdown } => check_text(what, markdown)?,
            NodeKind::Integration { integration_id, .. } => {
                let integration = by_id.get(integration_id.as_str()).ok_or_else(|| {
                    OrbitError::UnknownIntegration {
                        node: node.id.clone(),
                        integration: integration_id.clone(),
                    }
                })?;
                validate_integration(integration)?;
            }
            NodeKind::Browser { url } => {
                check_text(what, url)?;
                if !url.is_empty() && !service::is_local_url(url) {
                    return Err(OrbitError::NotLocalUrl(node.id.clone()));
                }
            }
            NodeKind::Service { source } => check_source(&node.id, source)?,
        }
    }

    let mut edge_ids: HashSet<&str> = HashSet::new();
    for edge in &graph.edges {
        if !edge_ids.insert(edge.id.as_str()) {
            return Err(OrbitError::DuplicateEdge(edge.id.clone()));
        }
        let lookup = |id: &str| {
            nodes
                .get(id)
                .copied()
                .ok_or_else(|| OrbitError::UnknownNode {
                    edge: edge.id.clone(),
                    node: id.to_string(),
                })
        };
        let from = lookup(&edge.from)?;
        let to = lookup(&edge.to)?;
        if !produces(&from.kind, &by_id).contains(&edge.kind) {
            return Err(OrbitError::NotProduced {
                edge: edge.id.clone(),
                node: from.id.clone(),
                kind: edge.kind,
            });
        }
        if !accepts(&to.kind, &by_id).contains(&edge.kind) {
            return Err(OrbitError::NotAccepted {
                edge: edge.id.clone(),
                node: to.id.clone(),
                kind: edge.kind,
            });
        }
    }

    check_acyclic(graph)
}

/// Fonte do serviço: texto sem controle e id de contêiner na forma do docker.
/// Fonte incompleta (porta 0, contêiner vazio) grava: o nó acabou de nascer e
/// o dono ainda escolhe; a execução falha com o motivo.
fn check_source(node: &str, source: &ServiceSource) -> Result<(), OrbitError> {
    let what = || format!("nó {node}");
    match source {
        ServiceSource::Container { container_id, name } => {
            check_text(what, name)?;
            if !container_id.is_empty() && !crate::docker::is_valid_container_id(container_id) {
                return Err(OrbitError::InvalidContainer(node.to_string()));
            }
        }
        ServiceSource::Port { session_id, .. } => {
            check_text(what, session_id.as_deref().unwrap_or_default())?;
        }
        ServiceSource::Tunnel {
            session_id,
            tunnel_id,
        } => {
            check_text(what, session_id)?;
            check_text(what, tunnel_id)?;
        }
    }
    Ok(())
}

/// Kahn: o que sobra com grau de entrada > 0 depois de esvaziar a fila está
/// num ciclo. Aponta uma ligação entre dois nós que sobraram.
fn check_acyclic(graph: &Graph) -> Result<(), OrbitError> {
    let mut indegree: HashMap<&str, usize> =
        graph.nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    for edge in &graph.edges {
        *indegree.entry(edge.to.as_str()).or_default() += 1;
    }
    let mut queue: Vec<&str> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| *id)
        .collect();
    while let Some(id) = queue.pop() {
        for edge in graph.edges.iter().filter(|e| e.from == id) {
            let d = indegree.get_mut(edge.to.as_str()).expect("nó contado");
            *d -= 1;
            if *d == 0 {
                queue.push(edge.to.as_str());
            }
        }
    }
    let stuck = |id: &str| indegree.get(id).is_some_and(|d| *d > 0);
    match graph.edges.iter().find(|e| stuck(&e.from) && stuck(&e.to)) {
        Some(edge) => Err(OrbitError::Cycle(edge.id.clone())),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            title: id.into(),
            kind,
            position: Position { x: 0.0, y: 0.0 },
            size: Size { w: 320.0, h: 200.0 },
        }
    }

    fn process(id: &str, output: OutputKind) -> Node {
        node(
            id,
            NodeKind::Process {
                command: "cargo test".into(),
                output,
                file: None,
                dir: String::new(),
            },
        )
    }

    fn agent(id: &str) -> Node {
        node(
            id,
            NodeKind::Agent {
                runner: Runner::Claude,
                role: "revisor".into(),
                prompt: "revise o diff".into(),
                dir: String::new(),
                orchestrate: false,
            },
        )
    }

    fn note(id: &str) -> Node {
        node(
            id,
            NodeKind::Note {
                markdown: "# plano".into(),
            },
        )
    }

    fn edge(id: &str, from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            kind,
        }
    }

    fn graph(nodes: Vec<Node>, edges: Vec<Edge>) -> Graph {
        Graph {
            id: "g-1".into(),
            name: "teste".into(),
            dir: "/tmp/repo".into(),
            nodes,
            edges,
            variables: BTreeMap::new(),
        }
    }

    fn integration(id: &str, trigger: Trigger) -> Integration {
        Integration {
            id: id.into(),
            name: "avisar".into(),
            script: "cat \"$TYBA_IN/log.txt\"".into(),
            trigger,
            output: OutputKind::Json,
            file: None,
        }
    }

    #[test]
    fn valid_graph_passes() {
        let g = graph(
            vec![note("n"), agent("a"), process("p", OutputKind::Log)],
            vec![
                edge("e1", "n", "a", EdgeKind::File),
                edge("e2", "a", "p", EdgeKind::Diff),
            ],
        );
        assert_eq!(validate_graph(&g, &[]), Ok(()));
    }

    #[test]
    fn cycle_is_refused() {
        let g = graph(
            vec![agent("a"), agent("b"), agent("c")],
            vec![
                edge("e1", "a", "b", EdgeKind::Context),
                edge("e2", "b", "c", EdgeKind::Context),
                edge("e3", "c", "a", EdgeKind::Context),
            ],
        );
        assert!(matches!(validate_graph(&g, &[]), Err(OrbitError::Cycle(_))));
    }

    #[test]
    fn self_loop_is_a_cycle() {
        let g = graph(
            vec![agent("a")],
            vec![edge("e1", "a", "a", EdgeKind::Context)],
        );
        assert_eq!(validate_graph(&g, &[]), Err(OrbitError::Cycle("e1".into())));
    }

    #[test]
    fn note_receiving_diff_is_refused() {
        let g = graph(
            vec![agent("a"), note("n")],
            vec![edge("e1", "a", "n", EdgeKind::Diff)],
        );
        assert!(matches!(
            validate_graph(&g, &[]),
            Err(OrbitError::NotAccepted { .. })
        ));
    }

    #[test]
    fn edge_kind_not_produced_is_refused() {
        // Processo de log não produz json.
        let g = graph(
            vec![process("p", OutputKind::Log), agent("a")],
            vec![edge("e1", "p", "a", EdgeKind::Json)],
        );
        assert!(matches!(
            validate_graph(&g, &[]),
            Err(OrbitError::NotProduced { .. })
        ));
    }

    #[test]
    fn integration_on_input_accepts_only_its_kind() {
        let integrations = [integration(
            "i-1",
            Trigger::OnInput {
                kind: EdgeKind::Log,
            },
        )];
        let int_node = node(
            "i",
            NodeKind::Integration {
                integration_id: "i-1".into(),
                dir: String::new(),
            },
        );
        let ok = graph(
            vec![process("p", OutputKind::Log), int_node.clone()],
            vec![edge("e1", "p", "i", EdgeKind::Log)],
        );
        assert_eq!(validate_graph(&ok, &integrations), Ok(()));

        let bad = graph(
            vec![agent("a"), int_node],
            vec![edge("e1", "a", "i", EdgeKind::Diff)],
        );
        assert!(matches!(
            validate_graph(&bad, &integrations),
            Err(OrbitError::NotAccepted { .. })
        ));
    }

    #[test]
    fn unknown_integration_is_refused() {
        let g = graph(
            vec![node(
                "i",
                NodeKind::Integration {
                    integration_id: "nao-existe".into(),
                    dir: String::new(),
                },
            )],
            vec![],
        );
        assert_eq!(
            validate_graph(&g, &[integration("i-1", Trigger::Manual)]),
            Err(OrbitError::UnknownIntegration {
                node: "i".into(),
                integration: "nao-existe".into(),
            })
        );
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let g = graph(vec![agent("a"), agent("a")], vec![]);
        assert_eq!(
            validate_graph(&g, &[]),
            Err(OrbitError::DuplicateNode("a".into()))
        );
        let g = graph(
            vec![agent("a"), agent("b"), agent("c")],
            vec![
                edge("e", "a", "b", EdgeKind::Context),
                edge("e", "a", "c", EdgeKind::Context),
            ],
        );
        assert_eq!(
            validate_graph(&g, &[]),
            Err(OrbitError::DuplicateEdge("e".into()))
        );
    }

    #[test]
    fn edge_to_missing_node_is_refused() {
        let g = graph(
            vec![agent("a")],
            vec![edge("e1", "a", "fantasma", EdgeKind::Context)],
        );
        assert!(matches!(
            validate_graph(&g, &[]),
            Err(OrbitError::UnknownNode { .. })
        ));
    }

    #[test]
    fn node_ceiling_is_enforced() {
        let nodes = (0..=MAX_NODES).map(|i| note(&format!("n{i}"))).collect();
        assert_eq!(
            validate_graph(&graph(nodes, vec![]), &[]),
            Err(OrbitError::TooManyNodes(MAX_NODES + 1))
        );
    }

    #[test]
    fn control_chars_are_refused() {
        let mut p = process("p", OutputKind::Log);
        p.kind = NodeKind::Process {
            command: "echo oi\rrm -rf ~".into(),
            output: OutputKind::Log,
            file: None,
            dir: String::new(),
        };
        assert!(matches!(
            validate_graph(&graph(vec![p], vec![]), &[]),
            Err(OrbitError::ControlChars(_))
        ));

        let mut a = agent("a");
        a.kind = NodeKind::Agent {
            runner: Runner::Codex,
            role: String::new(),
            prompt: "oi\u{1b}[2J".into(),
            dir: String::new(),
            orchestrate: false,
        };
        assert!(matches!(
            validate_graph(&graph(vec![a], vec![]), &[]),
            Err(OrbitError::ControlChars(_))
        ));

        let mut g = graph(vec![], vec![]);
        g.variables.insert("alvo".into(), "main\r".into());
        assert!(matches!(
            validate_graph(&g, &[]),
            Err(OrbitError::ControlChars(_))
        ));

        // `\n` e `\t` seguem permitidos, como no snippet.
        let mut n = note("n");
        n.kind = NodeKind::Note {
            markdown: "# a\n\tb".into(),
        };
        assert_eq!(validate_graph(&graph(vec![n], vec![]), &[]), Ok(()));
    }

    #[test]
    fn integration_script_is_validated() {
        let mut i = integration("i-1", Trigger::Manual);
        i.script = "echo oi\u{7}".into();
        assert!(matches!(
            validate_integration(&i),
            Err(OrbitError::ControlChars(_))
        ));
        i.script = "#".repeat(MAX_SCRIPT_BYTES + 1);
        assert_eq!(
            validate_integration(&i),
            Err(OrbitError::ScriptTooLarge("i-1".into()))
        );
    }

    #[test]
    fn output_file_outside_dir_is_refused() {
        let with_file = |file: &str| {
            graph(
                vec![node(
                    "p",
                    NodeKind::Process {
                        command: "cargo test".into(),
                        output: OutputKind::File,
                        file: Some(file.into()),
                        dir: String::new(),
                    },
                )],
                vec![],
            )
        };
        for ok in ["out/result.json", "relatorio.txt", "a/./b", "..x/arquivo"] {
            assert_eq!(validate_graph(&with_file(ok), &[]), Ok(()), "{ok}");
        }
        for bad in [
            "/etc/passwd",
            "../segredo",
            "out/../../segredo",
            "..",
            "\\servidor\\c",
            "..\\segredo",
            "C:\\Windows\\win.ini",
            "c:arquivo",
        ] {
            assert_eq!(
                validate_graph(&with_file(bad), &[]),
                Err(OrbitError::FileOutsideDir("nó p".into())),
                "{bad}"
            );
        }

        let mut i = integration("i-1", Trigger::Manual);
        i.file = Some("../../.ssh/id_ed25519".into());
        assert_eq!(
            validate_integration(&i),
            Err(OrbitError::FileOutsideDir("integração i-1".into()))
        );
        let g = graph(
            vec![node(
                "n",
                NodeKind::Integration {
                    integration_id: "i-1".into(),
                    dir: String::new(),
                },
            )],
            vec![],
        );
        assert!(matches!(
            validate_graph(&g, &[i]),
            Err(OrbitError::FileOutsideDir(_))
        ));
    }

    #[test]
    fn executable_node_needs_an_absolute_dir() {
        let with_dir = |dir: &str| {
            let mut a = agent("a");
            if let Some(d) = a.kind.dir_mut() {
                *d = dir.into();
            }
            a
        };
        // Sem pasta própria, vale a padrão do grafo.
        assert_eq!(
            validate_graph(&graph(vec![agent("a")], vec![]), &[]),
            Ok(())
        );
        assert_eq!(
            validate_graph(&graph(vec![with_dir("/srv/outra")], vec![]), &[]),
            Ok(())
        );

        let mut no_default = graph(vec![agent("a")], vec![]);
        no_default.dir.clear();
        assert_eq!(
            validate_graph(&no_default, &[]),
            Err(OrbitError::NodeDir("a".into()))
        );
        // A pasta do nó cobre a falta da padrão; nota não tem pasta.
        no_default.nodes = vec![with_dir("/srv/outra"), note("n")];
        assert_eq!(validate_graph(&no_default, &[]), Ok(()));

        for bad in ["relativa", "./x", "  "] {
            let mut g = graph(vec![with_dir(bad)], vec![]);
            g.dir.clear();
            assert_eq!(
                validate_graph(&g, &[]),
                Err(OrbitError::NodeDir("a".into())),
                "{bad}"
            );
        }
        assert_eq!(
            validate_graph(&graph(vec![with_dir("relativa")], vec![]), &[]),
            Err(OrbitError::NodeDir("a".into()))
        );
        assert!(matches!(
            validate_graph(&graph(vec![with_dir("/tmp/x\u{1b}")], vec![]), &[]),
            Err(OrbitError::ControlChars(_))
        ));
        // A estrutura não olha a pasta: é o que a importação usa.
        let mut g = graph(vec![with_dir("C:\\de\\outra\\maquina")], vec![]);
        g.dir.clear();
        assert_eq!(validate_structure(&g, &[]), Ok(()));
    }

    #[test]
    fn graph_saved_before_node_dir_still_loads() {
        let json = serde_json::json!({
            "id": "g", "name": "velho", "dir": "/tmp/repo",
            "nodes": [{
                "id": "p", "title": "p",
                "kind": {"type": "process", "command": "make", "output": "log"},
                "position": {"x": 0, "y": 0}, "size": {"w": 1, "h": 1}
            }],
            "edges": []
        });
        let g: Graph = serde_json::from_value(json).unwrap();
        assert_eq!(g.nodes[0].kind.dir(), Some(""));
        assert_eq!(g.node_dir(&g.nodes[0]), "/tmp/repo");
        assert_eq!(validate_graph(&g, &[]), Ok(()));
    }

    fn browser(id: &str, url: &str) -> Node {
        node(id, NodeKind::Browser { url: url.into() })
    }

    fn service(id: &str) -> Node {
        node(
            id,
            NodeKind::Service {
                source: ServiceSource::Port {
                    session_id: None,
                    port: 5173,
                },
            },
        )
    }

    #[test]
    fn browser_and_service_produce_and_accept() {
        let none = HashMap::new();
        assert!(produces(&browser("b", "").kind, &none).is_empty());
        assert_eq!(
            accepts(&browser("b", "").kind, &none),
            vec![EdgeKind::Url, EdgeKind::Control]
        );
        assert_eq!(
            produces(&service("s").kind, &none),
            vec![EdgeKind::Log, EdgeKind::Url]
        );
        assert!(accepts(&service("s").kind, &none).is_empty());
        // Processo de log também oferece url (vale quando a porta aparece).
        assert_eq!(
            produces(&process("p", OutputKind::Log).kind, &none),
            vec![EdgeKind::Log, EdgeKind::Url]
        );
        assert!(accepts(&agent("a").kind, &none).contains(&EdgeKind::Url));
        assert!(!accepts(&note("n").kind, &none).contains(&EdgeKind::Url));

        let ok = graph(
            vec![
                process("p", OutputKind::Log),
                service("s"),
                browser("b1", ""),
                browser("b2", ""),
                agent("a"),
            ],
            vec![
                edge("e1", "p", "b1", EdgeKind::Url),
                edge("e2", "s", "b2", EdgeKind::Url),
                edge("e3", "s", "a", EdgeKind::Log),
            ],
        );
        assert_eq!(validate_graph(&ok, &[]), Ok(()));

        // Navegador não produz; serviço não recebe; navegador só recebe url.
        for (from, to, kind) in [
            ("b1", "a", EdgeKind::Url),
            ("a", "s", EdgeKind::Context),
            ("s", "b1", EdgeKind::Log),
        ] {
            let g = graph(
                vec![service("s"), browser("b1", ""), agent("a")],
                vec![edge("e", from, to, kind)],
            );
            assert!(validate_graph(&g, &[]).is_err(), "{from} → {to} {kind:?}");
        }
    }

    #[test]
    fn browser_url_must_be_local() {
        for ok in [
            "",
            "http://localhost:5173",
            "http://127.0.0.1/x",
            "http://[::1]:8080",
        ] {
            assert_eq!(
                validate_graph(&graph(vec![browser("b", ok)], vec![]), &[]),
                Ok(()),
                "{ok}"
            );
        }
        for bad in [
            "http://192.168.0.2:5173",
            "https://example.com",
            "javascript:alert(1)",
            "file:///etc/passwd",
        ] {
            assert_eq!(
                validate_graph(&graph(vec![browser("b", bad)], vec![]), &[]),
                Err(OrbitError::NotLocalUrl("b".into())),
                "{bad}"
            );
        }
    }

    #[test]
    fn service_source_is_checked_but_may_be_incomplete() {
        let with = |source| graph(vec![node("s", NodeKind::Service { source })], vec![]);
        // Recém-criado: ainda sem escolha.
        assert_eq!(
            validate_graph(
                &with(ServiceSource::Container {
                    container_id: String::new(),
                    name: String::new()
                }),
                &[]
            ),
            Ok(())
        );
        assert_eq!(
            validate_graph(
                &with(ServiceSource::Container {
                    container_id: "abc123def456".into(),
                    name: "api".into()
                }),
                &[]
            ),
            Ok(())
        );
        assert_eq!(
            validate_graph(
                &with(ServiceSource::Container {
                    container_id: "--privileged".into(),
                    name: "api".into()
                }),
                &[]
            ),
            Err(OrbitError::InvalidContainer("s".into()))
        );
        assert!(matches!(
            validate_graph(
                &with(ServiceSource::Tunnel {
                    session_id: "x\u{1b}".into(),
                    tunnel_id: "t".into()
                }),
                &[]
            ),
            Err(OrbitError::ControlChars(_))
        ));
    }

    #[test]
    fn serde_shape_is_camel_case_with_type_tag() {
        let n = node(
            "i",
            NodeKind::Integration {
                integration_id: "i-1".into(),
                dir: String::new(),
            },
        );
        let json = serde_json::to_value(&n).unwrap();
        assert_eq!(json["kind"]["type"], "integration");
        assert_eq!(json["kind"]["integrationId"], "i-1");
        let t = serde_json::to_value(Trigger::OnInput {
            kind: EdgeKind::Diff,
        })
        .unwrap();
        assert_eq!(t, serde_json::json!({"type": "onInput", "kind": "diff"}));
    }

    #[test]
    fn plan_and_control_link_only_agent_to_note_and_browser() {
        let ok = graph(
            vec![agent("o"), note("n"), browser("b", ""), service("s")],
            vec![
                edge("e1", "o", "n", EdgeKind::Plan),
                edge("e2", "o", "b", EdgeKind::Control),
                edge("e3", "s", "b", EdgeKind::Url),
            ],
        );
        assert_eq!(validate_graph(&ok, &[]), Ok(()));
        let bad = [
            // Plan só para nota; Control só para navegador.
            ("o", "b", EdgeKind::Plan),
            ("o", "n", EdgeKind::Control),
            ("o", "a2", EdgeKind::Plan),
            ("o", "p", EdgeKind::Control),
            // Só agente produz.
            ("p", "n", EdgeKind::Plan),
            ("s", "b", EdgeKind::Control),
            ("n", "b", EdgeKind::Control),
        ];
        for (from, to, kind) in bad {
            let g = graph(
                vec![
                    agent("o"),
                    agent("a2"),
                    note("n"),
                    browser("b", ""),
                    service("s"),
                    process("p", OutputKind::Log),
                ],
                vec![edge("e1", from, to, kind)],
            );
            assert!(
                validate_graph(&g, &[]).is_err(),
                "{from} → {to} ({kind:?}) devia ser recusada"
            );
        }
        // Integração manual não recebe plan nem control.
        let i = integration("i-1", Trigger::Manual);
        let g = graph(
            vec![
                agent("o"),
                node(
                    "i",
                    NodeKind::Integration {
                        integration_id: "i-1".into(),
                        dir: String::new(),
                    },
                ),
            ],
            vec![edge("e1", "o", "i", EdgeKind::Control)],
        );
        assert!(validate_graph(&g, &[i]).is_err());
    }

    #[test]
    fn plan_and_control_serialize_and_carry_no_data() {
        assert_eq!(serde_json::to_value(EdgeKind::Plan).unwrap(), "plan");
        assert_eq!(serde_json::to_value(EdgeKind::Control).unwrap(), "control");
        assert!(!EdgeKind::Plan.carries_data() && !EdgeKind::Control.carries_data());
        assert_eq!(EdgeKind::data().len(), 6);
        assert!(EdgeKind::data().iter().all(|k| k.carries_data()));
    }
}
