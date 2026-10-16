//! Navegador nativo do nó (Fase 5, `fase5-navegador`).
//!
//! Cada nó de navegador ganha um webview nativo próprio, preso à janela
//! principal e sem IPC do Tauri dentro dele. O estado (url, título, foco,
//! último print) vive aqui; o front manda só a intenção de geometria e de
//! navegação, e desenha o que o core emite.

pub mod policy;

use std::path::PathBuf;

use super::{EdgeKind, Graph};

/// As pastas de onde o `orbit-file` deste navegador pode servir: a pasta do
/// workspace (`Graph::dir`) e a de cada nó ligado a ele (quem manda a url,
/// quem o controla). Sem duplicata, na ordem: o índice vira o host `r<i>`.
pub fn file_roots(graph: &Graph, node_id: &str) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut push = |dir: &str| {
        let p = PathBuf::from(dir);
        if !dir.is_empty() && p.is_absolute() && !roots.contains(&p) {
            roots.push(p);
        }
    };
    push(&graph.dir);
    for e in graph
        .edges
        .iter()
        .filter(|e| e.to == node_id && matches!(e.kind, EdgeKind::Url | EdgeKind::Control))
    {
        if let Some(n) = graph.nodes.iter().find(|n| n.id == e.from) {
            push(graph.node_dir(n));
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orbit::{Edge, Node, NodeKind, Position, Size};

    fn node(id: &str, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            title: id.into(),
            kind,
            position: Position { x: 0.0, y: 0.0 },
            size: Size { w: 10.0, h: 10.0 },
        }
    }

    #[test]
    fn roots_are_the_workspace_and_the_linked_nodes() {
        let process = |dir: &str| NodeKind::Process {
            command: "x".into(),
            output: Default::default(),
            file: None,
            dir: dir.into(),
        };
        let graph = Graph {
            id: "g".into(),
            name: "g".into(),
            dir: "/ws".into(),
            nodes: vec![
                node("b", NodeKind::Browser { url: String::new() }),
                node("p", process("/ws/front")),
                node("q", process("")),
                node("solto", process("/outro")),
                node("rel", process("relativo")),
            ],
            edges: vec![
                Edge {
                    id: "1".into(),
                    from: "p".into(),
                    to: "b".into(),
                    kind: EdgeKind::Url,
                },
                Edge {
                    id: "2".into(),
                    from: "q".into(),
                    to: "b".into(),
                    kind: EdgeKind::Url,
                },
                Edge {
                    id: "3".into(),
                    from: "rel".into(),
                    to: "b".into(),
                    kind: EdgeKind::Url,
                },
            ],
            variables: Default::default(),
        };
        assert_eq!(
            file_roots(&graph, "b"),
            vec![PathBuf::from("/ws"), PathBuf::from("/ws/front")]
        );
        assert_eq!(file_roots(&graph, "outro"), vec![PathBuf::from("/ws")]);
    }
}
