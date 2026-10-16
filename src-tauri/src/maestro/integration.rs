//! Nó de integração: o script bash do dono, rodado como processo na pasta do
//! nó, com o mesmo wrapper e o mesmo env do nó de processo.
//!
//! Gatilho `manual` segura o nó em `waiting` até o ▶ do dono
//! (`maestro_run_node`); `on_input` roda quando a entrada chega, como qualquer
//! nó. "Testar com a última entrada" roda o script sozinho com o `in/` da
//! última execução daquele nó.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::run::Run;
use super::{Graph, Integration, NodeKind, Trigger};

/// Nós de integração com gatilho manual: esperam o ▶ do dono.
pub fn manual_nodes(graph: &Graph, integrations: &HashMap<String, Integration>) -> Vec<String> {
    graph
        .nodes
        .iter()
        .filter(|n| match &n.kind {
            NodeKind::Integration { integration_id, .. } => integrations
                .get(integration_id)
                .is_some_and(|i| i.trigger == Trigger::Manual),
            _ => false,
        })
        .map(|n| n.id.clone())
        .collect()
}

/// `in/` da execução mais nova (a lista vem da mais nova para a mais velha) em
/// que o nó chegou a começar — só aí as entradas foram materializadas.
pub fn last_inputs(runs: &[Run], root: &Path, node_id: &str) -> Option<PathBuf> {
    runs.iter()
        .filter(|r| super::run::safe_component(&r.id))
        .filter(|r| r.nodes.get(node_id).is_some_and(|n| n.started_at.is_some()))
        .map(|r| root.join(&r.id).join(node_id).join("in"))
        .find(|p| p.is_dir())
}

/// Copia os arquivos de `from` para `to`. Só arquivo comum: link simbólico e
/// pasta ficam de fora (o `in/` só tem arquivo).
pub fn copy_inputs(from: &Path, to: &Path) -> std::io::Result<usize> {
    let mut copied = 0;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            std::fs::copy(entry.path(), to.join(entry.file_name()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// Grafo de um nó só para o teste: o nó com a integração pedida, sem ligação.
/// Variáveis são as do grafo; a pasta é a do nó.
pub fn test_graph(graph: &Graph, node_id: &str, integration_id: &str) -> Result<Graph, String> {
    let node = graph
        .nodes
        .iter()
        .find(|n| n.id == node_id)
        .ok_or("nó não encontrado no grafo")?;
    let dir = graph.node_dir(node).to_string();
    let mut node = node.clone();
    node.kind = NodeKind::Integration {
        integration_id: integration_id.to_string(),
        dir,
    };
    Ok(Graph {
        nodes: vec![node],
        edges: Vec::new(),
        ..graph.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maestro::run::{NodeRun, NodeStatus, RunStatus};
    use crate::maestro::{EdgeKind, Node, OutputKind, Position, Size};
    use std::collections::BTreeMap;

    fn integration(id: &str, trigger: Trigger) -> Integration {
        Integration {
            id: id.into(),
            name: id.into(),
            script: "echo oi".into(),
            trigger,
            output: OutputKind::Log,
            file: None,
        }
    }

    fn node(id: &str, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            title: id.into(),
            kind,
            position: Position { x: 0.0, y: 0.0 },
            size: Size { w: 1.0, h: 1.0 },
        }
    }

    fn graph() -> Graph {
        Graph {
            id: "g".into(),
            name: "g".into(),
            dir: "/tmp".into(),
            nodes: vec![
                node(
                    "m",
                    NodeKind::Integration {
                        integration_id: "manual".into(),
                        dir: String::new(),
                    },
                ),
                node(
                    "a",
                    NodeKind::Integration {
                        integration_id: "auto".into(),
                        dir: String::new(),
                    },
                ),
                node(
                    "p",
                    NodeKind::Process {
                        command: "true".into(),
                        output: OutputKind::Log,
                        file: None,
                        dir: String::new(),
                    },
                ),
            ],
            edges: Vec::new(),
            variables: BTreeMap::new(),
        }
    }

    #[test]
    fn only_manual_integrations_are_held() {
        let integrations = HashMap::from([
            ("manual".to_string(), integration("manual", Trigger::Manual)),
            (
                "auto".to_string(),
                integration(
                    "auto",
                    Trigger::OnInput {
                        kind: EdgeKind::Diff,
                    },
                ),
            ),
        ]);
        assert_eq!(manual_nodes(&graph(), &integrations), vec!["m".to_string()]);
    }

    fn run(id: &str, started: bool) -> Run {
        let mut n = NodeRun {
            status: NodeStatus::Skipped,
            session_id: None,
            started_at: None,
            finished_at: None,
            exit_code: None,
            outputs: HashMap::new(),
            reason: None,
        };
        if started {
            n.status = NodeStatus::Done;
            n.started_at = Some(1);
        }
        Run {
            id: id.into(),
            graph_id: "g".into(),
            started_at: 0,
            finished_at: None,
            status: RunStatus::Done,
            nodes: HashMap::from([("m".to_string(), n)]),
            version: 1,
            test: false,
            live_agents: Vec::new(),
        }
    }

    #[test]
    fn last_inputs_skips_runs_where_the_node_never_started() {
        let root = tempfile::tempdir().unwrap();
        for id in ["novo", "velho"] {
            std::fs::create_dir_all(root.path().join(id).join("m").join("in")).unwrap();
        }
        let runs = vec![run("novo", false), run("velho", true)];
        assert_eq!(
            last_inputs(&runs, root.path(), "m"),
            Some(root.path().join("velho").join("m").join("in"))
        );
        assert_eq!(last_inputs(&runs[..1], root.path(), "m"), None);
        assert_eq!(last_inputs(&runs, root.path(), "outro"), None);
    }

    #[test]
    fn test_graph_keeps_only_the_node() {
        let g = test_graph(&graph(), "p", "manual").unwrap();
        assert_eq!(g.nodes.len(), 1);
        assert_eq!(
            g.nodes[0].kind,
            NodeKind::Integration {
                integration_id: "manual".into(),
                // A pasta do nó (aqui, a padrão do grafo) vai junto.
                dir: "/tmp".into(),
            }
        );
        assert!(test_graph(&graph(), "x", "manual").is_err());
    }
}
