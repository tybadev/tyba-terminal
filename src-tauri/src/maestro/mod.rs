//! Maestro: o Workspace como harness de agentes.
//!
//! Um grafo de nós (agente, processo, nota, integração) ligados por ligações
//! tipadas. Todo nó roda na pasta do workspace (`Graph::dir`) — não há
//! worktree. O estado vive aqui no core; o front desenha e chama comandos.
//!
//! A validação deste módulo é a barreira: o front só oferece ligações válidas,
//! mas quem decide é `validate_graph`, chamado antes de gravar.

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
    /// Pasta do workspace. Todo nó roda nela.
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
    },
    Process {
        command: String,
        #[serde(default)]
        output: OutputKind,
        #[serde(default)]
        file: Option<String>,
    },
    Note {
        markdown: String,
    },
    Integration {
        integration_id: String,
    },
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
}

impl EdgeKind {
    const ALL: [EdgeKind; 5] = [
        EdgeKind::Context,
        EdgeKind::Diff,
        EdgeKind::File,
        EdgeKind::Log,
        EdgeKind::Json,
    ];
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
    /// Script bash do dono, rodado como processo na pasta do workspace.
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
pub enum MaestroError {
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
}

/// Mesma regra de `snippet::validate`: `\n` e `\t` passam, o resto dos
/// caracteres de controle não. Texto daqui vira linha de comando ou prompt.
fn has_control_chars(text: &str) -> bool {
    text.chars()
        .any(|c| c != '\n' && c != '\t' && c.is_control())
}

fn check_text(what: impl FnOnce() -> String, text: &str) -> Result<(), MaestroError> {
    if has_control_chars(text) {
        return Err(MaestroError::ControlChars(what()));
    }
    Ok(())
}

pub fn validate_integration(integration: &Integration) -> Result<(), MaestroError> {
    if integration.id.trim().is_empty() {
        return Err(MaestroError::EmptyIntegrationId);
    }
    if integration.name.trim().is_empty() {
        return Err(MaestroError::EmptyIntegrationName);
    }
    if integration.script.len() > MAX_SCRIPT_BYTES {
        return Err(MaestroError::ScriptTooLarge(integration.id.clone()));
    }
    check_text(
        || format!("script da integração {}", integration.id),
        &integration.script,
    )
}

/// O que o nó produz (tabela de rules).
fn produces(kind: &NodeKind, integrations: &HashMap<&str, &Integration>) -> Vec<EdgeKind> {
    match kind {
        NodeKind::Agent { .. } => vec![EdgeKind::Context, EdgeKind::Diff],
        NodeKind::Process { output, .. } => vec![(*output).into()],
        NodeKind::Note { .. } => vec![EdgeKind::File],
        NodeKind::Integration { integration_id } => integrations
            .get(integration_id.as_str())
            .map(|i| vec![i.output.into()])
            .unwrap_or_default(),
    }
}

/// O que o nó aceita (tabela de rules).
fn accepts(kind: &NodeKind, integrations: &HashMap<&str, &Integration>) -> Vec<EdgeKind> {
    match kind {
        NodeKind::Agent { .. } | NodeKind::Process { .. } => EdgeKind::ALL.to_vec(),
        NodeKind::Note { .. } => Vec::new(),
        NodeKind::Integration { integration_id } => {
            match integrations.get(integration_id.as_str()).map(|i| i.trigger) {
                Some(Trigger::OnInput { kind }) => vec![kind],
                Some(Trigger::Manual) => EdgeKind::ALL.to_vec(),
                None => Vec::new(),
            }
        }
    }
}

/// Barreira do core antes de gravar um grafo: ids únicos, teto de nós,
/// integrações existentes, ligações com tipo aceito, DAG e texto sem caractere
/// de controle.
pub fn validate_graph(graph: &Graph, integrations: &[Integration]) -> Result<(), MaestroError> {
    if graph.id.trim().is_empty() {
        return Err(MaestroError::EmptyGraphId);
    }
    if graph.nodes.len() > MAX_NODES {
        return Err(MaestroError::TooManyNodes(graph.nodes.len()));
    }
    // Variável é substituída dentro de comando, script e prompt: o valor
    // passaria por fora da checagem do texto se não fosse checado aqui.
    for (name, value) in &graph.variables {
        check_text(|| format!("variável {name}"), name)?;
        check_text(|| format!("variável {name}"), value)?;
    }

    let by_id: HashMap<&str, &Integration> =
        integrations.iter().map(|i| (i.id.as_str(), i)).collect();

    let mut nodes: HashMap<&str, &Node> = HashMap::new();
    for node in &graph.nodes {
        if node.id.trim().is_empty() {
            return Err(MaestroError::EmptyNodeId);
        }
        if nodes.insert(node.id.as_str(), node).is_some() {
            return Err(MaestroError::DuplicateNode(node.id.clone()));
        }
        let what = || format!("nó {}", node.id);
        match &node.kind {
            NodeKind::Agent { prompt, .. } => check_text(what, prompt)?,
            NodeKind::Process { command, .. } => check_text(what, command)?,
            NodeKind::Note { markdown } => check_text(what, markdown)?,
            NodeKind::Integration { integration_id } => {
                let integration = by_id.get(integration_id.as_str()).ok_or_else(|| {
                    MaestroError::UnknownIntegration {
                        node: node.id.clone(),
                        integration: integration_id.clone(),
                    }
                })?;
                validate_integration(integration)?;
            }
        }
    }

    let mut edge_ids: HashSet<&str> = HashSet::new();
    for edge in &graph.edges {
        if !edge_ids.insert(edge.id.as_str()) {
            return Err(MaestroError::DuplicateEdge(edge.id.clone()));
        }
        let lookup = |id: &str| {
            nodes
                .get(id)
                .copied()
                .ok_or_else(|| MaestroError::UnknownNode {
                    edge: edge.id.clone(),
                    node: id.to_string(),
                })
        };
        let from = lookup(&edge.from)?;
        let to = lookup(&edge.to)?;
        if !produces(&from.kind, &by_id).contains(&edge.kind) {
            return Err(MaestroError::NotProduced {
                edge: edge.id.clone(),
                node: from.id.clone(),
                kind: edge.kind,
            });
        }
        if !accepts(&to.kind, &by_id).contains(&edge.kind) {
            return Err(MaestroError::NotAccepted {
                edge: edge.id.clone(),
                node: to.id.clone(),
                kind: edge.kind,
            });
        }
    }

    check_acyclic(graph)
}

/// Kahn: o que sobra com grau de entrada > 0 depois de esvaziar a fila está
/// num ciclo. Aponta uma ligação entre dois nós que sobraram.
fn check_acyclic(graph: &Graph) -> Result<(), MaestroError> {
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
        Some(edge) => Err(MaestroError::Cycle(edge.id.clone())),
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
        assert!(matches!(
            validate_graph(&g, &[]),
            Err(MaestroError::Cycle(_))
        ));
    }

    #[test]
    fn self_loop_is_a_cycle() {
        let g = graph(
            vec![agent("a")],
            vec![edge("e1", "a", "a", EdgeKind::Context)],
        );
        assert_eq!(
            validate_graph(&g, &[]),
            Err(MaestroError::Cycle("e1".into()))
        );
    }

    #[test]
    fn note_receiving_diff_is_refused() {
        let g = graph(
            vec![agent("a"), note("n")],
            vec![edge("e1", "a", "n", EdgeKind::Diff)],
        );
        assert!(matches!(
            validate_graph(&g, &[]),
            Err(MaestroError::NotAccepted { .. })
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
            Err(MaestroError::NotProduced { .. })
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
            Err(MaestroError::NotAccepted { .. })
        ));
    }

    #[test]
    fn unknown_integration_is_refused() {
        let g = graph(
            vec![node(
                "i",
                NodeKind::Integration {
                    integration_id: "nao-existe".into(),
                },
            )],
            vec![],
        );
        assert_eq!(
            validate_graph(&g, &[integration("i-1", Trigger::Manual)]),
            Err(MaestroError::UnknownIntegration {
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
            Err(MaestroError::DuplicateNode("a".into()))
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
            Err(MaestroError::DuplicateEdge("e".into()))
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
            Err(MaestroError::UnknownNode { .. })
        ));
    }

    #[test]
    fn node_ceiling_is_enforced() {
        let nodes = (0..=MAX_NODES).map(|i| note(&format!("n{i}"))).collect();
        assert_eq!(
            validate_graph(&graph(nodes, vec![]), &[]),
            Err(MaestroError::TooManyNodes(MAX_NODES + 1))
        );
    }

    #[test]
    fn control_chars_are_refused() {
        let mut p = process("p", OutputKind::Log);
        p.kind = NodeKind::Process {
            command: "echo oi\rrm -rf ~".into(),
            output: OutputKind::Log,
            file: None,
        };
        assert!(matches!(
            validate_graph(&graph(vec![p], vec![]), &[]),
            Err(MaestroError::ControlChars(_))
        ));

        let mut a = agent("a");
        a.kind = NodeKind::Agent {
            runner: Runner::Codex,
            role: String::new(),
            prompt: "oi\u{1b}[2J".into(),
        };
        assert!(matches!(
            validate_graph(&graph(vec![a], vec![]), &[]),
            Err(MaestroError::ControlChars(_))
        ));

        let mut g = graph(vec![], vec![]);
        g.variables.insert("alvo".into(), "main\r".into());
        assert!(matches!(
            validate_graph(&g, &[]),
            Err(MaestroError::ControlChars(_))
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
            Err(MaestroError::ControlChars(_))
        ));
        i.script = "#".repeat(MAX_SCRIPT_BYTES + 1);
        assert_eq!(
            validate_integration(&i),
            Err(MaestroError::ScriptTooLarge("i-1".into()))
        );
    }

    #[test]
    fn serde_shape_is_camel_case_with_type_tag() {
        let n = node(
            "i",
            NodeKind::Integration {
                integration_id: "i-1".into(),
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
}
