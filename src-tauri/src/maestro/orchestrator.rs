//! Orquestrador: o nó de agente com `orchestrate` cria nós no run ativo.
//!
//! O pedido chega do `tyba _maestro-mcp` ([`super::mcp`]) pelo socket de
//! hooks da sessão dele e cai em [`super::run::Runs::orchestrator_call`]. Aqui
//! mora o que não depende do run: ler e validar os argumentos de cada
//! ferramenta (mesma régua do salvar — pasta absoluta que existe, URL só
//! local, sem caractere de controle), o nó que nasce deles, o retrato do log
//! para o `maestro_node_status` e a junção dos nós mantidos no grafo.
//!
//! Nó criado é efêmero: vive no [`super::run::Run`] (`created`), nunca no
//! grafo salvo, até o dono clicar "Manter no grafo".

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    has_control_chars, service, Edge, Graph, Node, NodeKind, OutputKind, Position, Runner, Size,
};

/// Teto de nós criados por execução.
pub const MAX_CREATED: usize = 30;
/// Linhas do log no `maestro_node_status`.
pub const STATUS_LOG_LINES: usize = 40;
/// Quanto do fim do `log.txt` se lê para achar as últimas linhas.
const STATUS_LOG_TAIL_BYTES: u64 = 64 * 1024;
/// Teto do texto do turno no `maestro_node_status`; o resto o agente lê
/// pelo caminho do `context.md`.
pub const STATUS_TEXT_MAX_CHARS: usize = 8_000;
/// Arquivos listados no resumo do diff (os totais contam todos).
const STATUS_DIFF_MAX_FILES: usize = 100;

/// Quem atende as ferramentas da sessão do orquestrador: `(ferramenta,
/// argumentos) → resultado`. Montado pelo run para o nó, levado até o
/// handler do socket de hooks da sessão.
#[derive(Clone)]
pub struct ToolHandler(pub Arc<ToolFn>);

/// `(ferramenta, argumentos) → resultado`.
pub type ToolFn = dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync;

impl ToolHandler {
    pub fn call(&self, tool: &str, args: &Value) -> Result<Value, String> {
        (self.0)(tool, args)
    }
}

impl std::fmt::Debug for ToolHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ToolHandler")
    }
}

/// Nó criado pelo orquestrador neste run. `edges` são as ligações que chegam
/// nele (a `url` do `from_node`); a ligação tracejada criador→criado é só
/// desenho, sai de `created_by` e não entra no agendamento.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    pub node: Node,
    pub created_by: String,
    pub created_at: u64,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// O dono clicou "Manter no grafo": o nó já está no grafo salvo.
    #[serde(default)]
    pub kept: bool,
}

/// Uma ferramenta pedida, já lida.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolCall {
    /// Nó novo (processo, agente ou navegador) e, no navegador, a origem da url.
    Create {
        title: String,
        kind: NodeKind,
        from_node: Option<String>,
    },
    Status {
        node_id: String,
    },
    Stop {
        node_id: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessArgs {
    title: String,
    command: String,
    dir: String,
    #[serde(default)]
    output: Option<OutputKind>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentArgs {
    title: String,
    runner: Runner,
    prompt: String,
    dir: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserArgs {
    title: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    from_node: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeArgs {
    node_id: String,
}

fn args<T: serde::de::DeserializeOwned>(tool: &str, value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|e| format!("{tool}: argumentos inválidos: {e}"))
}

fn text(what: &str, value: &str) -> Result<(), String> {
    if has_control_chars(value) {
        return Err(format!("{what} contém caractere de controle"));
    }
    Ok(())
}

fn required(what: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{what} vazio"));
    }
    text(what, value)
}

/// Pasta absoluta que existe nesta máquina (a régua do salvar e do importar).
fn existing_dir(dir: &str) -> Result<(), String> {
    text("dir", dir)?;
    let path = Path::new(dir);
    if !path.is_absolute() {
        return Err(format!("dir tem que ser caminho absoluto: {dir}"));
    }
    if !path.is_dir() {
        return Err(format!("a pasta não existe: {dir}"));
    }
    Ok(())
}

/// Lê e valida o pedido. O que depende do run (o `from_node` existir e dar
/// url, o teto, de quem é o nó parado) o run confere.
pub fn parse_call(tool: &str, value: &Value) -> Result<ToolCall, String> {
    match tool {
        "maestro_create_process" => {
            let a: ProcessArgs = args(tool, value)?;
            required("title", &a.title)?;
            required("command", &a.command)?;
            existing_dir(&a.dir)?;
            let output = a.output.unwrap_or_default();
            if output == OutputKind::File {
                return Err("output: use log, json ou diff (file pede o arquivo no nó)".into());
            }
            Ok(ToolCall::Create {
                title: a.title,
                kind: NodeKind::Process {
                    command: a.command,
                    output,
                    file: None,
                    dir: a.dir,
                },
                from_node: None,
            })
        }
        "maestro_create_agent" => {
            let a: AgentArgs = args(tool, value)?;
            required("title", &a.title)?;
            required("prompt", &a.prompt)?;
            existing_dir(&a.dir)?;
            Ok(ToolCall::Create {
                title: a.title,
                // Agente criado não orquestra: sem ferramentas, sem cascata.
                kind: NodeKind::Agent {
                    runner: a.runner,
                    role: String::new(),
                    prompt: a.prompt,
                    dir: a.dir,
                    orchestrate: false,
                },
                from_node: None,
            })
        }
        "maestro_create_browser" => {
            let a: BrowserArgs = args(tool, value)?;
            required("title", &a.title)?;
            let url = a.url.unwrap_or_default().trim().to_string();
            let from_node = a.from_node.filter(|f| !f.trim().is_empty());
            if url.is_empty() && from_node.is_none() {
                return Err("informe url (local) ou from_node".into());
            }
            text("url", &url)?;
            if !url.is_empty() && !service::is_local_url(&url) {
                return Err(format!(
                    "url recusada: só local (http/https em {})",
                    service::LOCAL_HOSTS
                ));
            }
            if let Some(f) = &from_node {
                text("from_node", f)?;
            }
            Ok(ToolCall::Create {
                title: a.title,
                kind: NodeKind::Browser { url },
                from_node,
            })
        }
        "maestro_node_status" => {
            let a: NodeArgs = args(tool, value)?;
            required("node_id", &a.node_id)?;
            Ok(ToolCall::Status { node_id: a.node_id })
        }
        "maestro_stop_node" => {
            let a: NodeArgs = args(tool, value)?;
            required("node_id", &a.node_id)?;
            Ok(ToolCall::Stop { node_id: a.node_id })
        }
        _ => Err(format!("ferramenta desconhecida: {tool}")),
    }
}

/// Tamanho do nó criado (o mesmo `DEFAULT_SIZE` do front).
fn default_size(kind: &NodeKind) -> Size {
    match kind {
        NodeKind::Agent { .. } => Size { w: 360.0, h: 300.0 },
        NodeKind::Browser { .. } => Size { w: 640.0, h: 420.0 },
        _ => Size { w: 360.0, h: 250.0 },
    }
}

/// O nó novo, à direita do orquestrador, empilhado abaixo dos que ele já
/// criou (quadro: uma coluna de nós tracejados).
pub fn new_node(
    id: String,
    title: String,
    kind: NodeKind,
    orchestrator: Option<&Node>,
    siblings: &[&Node],
) -> Node {
    const GAP_X: f64 = 140.0;
    const GAP_Y: f64 = 40.0;
    let size = default_size(&kind);
    let (x, top) = orchestrator
        .map(|o| (o.position.x + o.size.w + GAP_X, o.position.y))
        .unwrap_or((0.0, 0.0));
    let y = top + siblings.iter().map(|n| n.size.h + GAP_Y).sum::<f64>();
    Node {
        id,
        title,
        kind,
        position: Position { x, y },
        size,
    }
}

/// Tira as sequências de controle do terminal (CSI `ESC[…`, OSC `ESC]…BEL`)
/// e os `\r` do PTY: o modelo lê o texto, não as cores.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// As últimas `n` linhas do log do nó, sem cor e redigidas. Lê só o fim do
/// arquivo. Sem arquivo: `None`.
pub fn log_tail(path: &Path, n: usize) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(STATUS_LOG_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = strip_ansi(&String::from_utf8_lossy(&bytes));
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(n)..].join("\n");
    Some(crate::session::redact::redact(&tail).into_owned())
}

/// O texto do `context.md` (a fala final do agente, já redigida ao gravar),
/// até [`STATUS_TEXT_MAX_CHARS`]. `true` quando cortou. Vazio: `None`.
pub fn turn_text(path: &Path) -> Option<(String, bool)> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let cut: String = text.chars().take(STATUS_TEXT_MAX_CHARS).collect();
    let truncated = cut.len() < text.len();
    Some((cut, truncated))
}

/// Um arquivo do resumo do diff.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiffFile {
    pub path: String,
    pub added: usize,
    pub removed: usize,
}

/// Resumo de um patch unificado (`git diff`, com os arquivos novos de
/// `--no-index`): arquivos com linhas `+`/`−` e os totais. Cabeçalhos
/// (`---`/`+++`) só contam como cabeçalho antes do primeiro `@@` do arquivo:
/// dentro do hunk, `+++x` é uma linha `++x` adicionada.
pub fn diff_summary(patch: &str) -> Value {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut in_hunk = false;
    for line in patch.lines() {
        if let Some(header) = line.strip_prefix("diff --git ") {
            let path = header.rsplit_once(" b/").map_or(header, |(_, p)| p);
            files.push(DiffFile {
                path: path.to_string(),
                added: 0,
                removed: 0,
            });
            in_hunk = false;
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if !in_hunk {
            if let Some(path) = line.strip_prefix("+++ b/") {
                file.path = path.to_string();
            } else if line.starts_with("@@") {
                in_hunk = true;
            }
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => file.added += 1,
            Some(b'-') => file.removed += 1,
            _ => {}
        }
    }
    let added = files.iter().map(|f| f.added).sum::<usize>();
    let removed = files.iter().map(|f| f.removed).sum::<usize>();
    let changed = files.len();
    files.truncate(STATUS_DIFF_MAX_FILES);
    serde_json::json!({
        "files_changed": changed,
        "added": added,
        "removed": removed,
        "files": files,
    })
}

/// "Manter no grafo": os nós criados entram no grafo como nós normais (sem
/// `created_by`, que só existe no run), com as ligações que chegam neles. Id
/// que já está no grafo não entra de novo.
pub fn merge_kept(graph: &mut Graph, created: &[Created]) {
    for c in created {
        if !graph.nodes.iter().any(|n| n.id == c.node.id) {
            graph.nodes.push(c.node.clone());
        }
    }
    for edge in created.iter().flat_map(|c| &c.edges) {
        if !graph.edges.iter().any(|e| e.id == edge.id) {
            graph.edges.push(edge.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dir() -> String {
        std::env::temp_dir().to_string_lossy().into_owned()
    }

    #[test]
    fn create_process_needs_an_existing_absolute_dir() {
        let ok = parse_call(
            "maestro_create_process",
            &json!({"title": "api", "command": "bun run dev", "dir": dir()}),
        )
        .unwrap();
        assert!(matches!(
            ok,
            ToolCall::Create {
                kind: NodeKind::Process {
                    output: OutputKind::Log,
                    ..
                },
                ..
            }
        ));
        for bad in ["relativa/x", "/nao/existe/mesmo/tyba-orq", ""] {
            let err = parse_call(
                "maestro_create_process",
                &json!({"title": "api", "command": "x", "dir": bad}),
            )
            .unwrap_err();
            assert!(
                err.contains("absoluto") || err.contains("não existe"),
                "{bad}: {err}"
            );
        }
    }

    #[test]
    fn control_chars_and_empty_fields_are_refused() {
        let err = parse_call(
            "maestro_create_process",
            &json!({"title": "api", "command": "echo oi\rrm -rf ~", "dir": dir()}),
        )
        .unwrap_err();
        assert!(err.contains("controle"), "{err}");
        let err = parse_call(
            "maestro_create_agent",
            &json!({"title": "rev", "runner": "claude", "prompt": "oi\u{1b}[2J", "dir": dir()}),
        )
        .unwrap_err();
        assert!(err.contains("controle"), "{err}");
        assert!(parse_call(
            "maestro_create_process",
            &json!({"title": "  ", "command": "x", "dir": dir()}),
        )
        .is_err());
        // Campo a mais, tipo errado, runner desconhecido, saída file.
        for (tool, args) in [
            (
                "maestro_create_process",
                json!({"title": "a", "command": "x", "dir": dir(), "sudo": true}),
            ),
            ("maestro_create_process", json!({"title": 1})),
            (
                "maestro_create_agent",
                json!({"title": "a", "runner": "gpt", "prompt": "x", "dir": dir()}),
            ),
            (
                "maestro_create_process",
                json!({"title": "a", "command": "x", "dir": dir(), "output": "file"}),
            ),
        ] {
            assert!(parse_call(tool, &args).is_err(), "{tool} {args}");
        }
    }

    #[test]
    fn created_agent_never_orchestrates() {
        let call = parse_call(
            "maestro_create_agent",
            &json!({"title": "rev", "runner": "codex", "prompt": "revise", "dir": dir()}),
        )
        .unwrap();
        let ToolCall::Create { kind, .. } = call else {
            panic!("{call:?}")
        };
        assert!(matches!(
            kind,
            NodeKind::Agent {
                runner: Runner::Codex,
                orchestrate: false,
                ..
            }
        ));
    }

    #[test]
    fn browser_url_must_be_local_or_come_from_a_node() {
        let ok = |args: Value| parse_call("maestro_create_browser", &args);
        assert!(ok(json!({"title": "code", "url": "http://localhost:5180"})).is_ok());
        assert!(ok(json!({"title": "code", "from_node": "orq-1"})).is_ok());
        for bad in [
            "https://example.com",
            "http://192.168.0.2:5173",
            "javascript:alert(1)",
            "http://user@localhost:1",
        ] {
            let err = ok(json!({"title": "code", "url": bad})).unwrap_err();
            assert!(err.contains("só local"), "{bad}: {err}");
            // A recusa ensina a regra inteira, domínios de dev inclusive.
            assert!(
                err.contains("*.test") && err.contains("*.localhost"),
                "{err}"
            );
        }
        assert!(ok(json!({"title": "code"})).is_err());
    }

    #[test]
    fn unknown_tool_is_refused() {
        assert!(parse_call("maestro_delete_graph", &json!({})).is_err());
    }

    #[test]
    fn strip_ansi_keeps_the_text() {
        assert_eq!(
            strip_ansi("\u{1b}[32mVITE\u{1b}[0m pronto\r\n\u{1b}]0;titulo\u{7}ok"),
            "VITE pronto\nok"
        );
    }

    #[test]
    fn log_tail_is_the_last_lines_redacted() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("log.txt");
        let key = format!("sk-{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6");
        let body: String = (0..100).map(|i| format!("linha {i}\n")).collect();
        std::fs::write(&path, format!("{body}chave {key}\n")).unwrap();
        let tail = log_tail(&path, 3).unwrap();
        assert!(tail.starts_with("linha 98\nlinha 99\nchave "), "{tail}");
        assert!(!tail.contains(&key), "{tail}");
        assert_eq!(log_tail(&tmp.path().join("nao"), 3), None);
    }

    #[test]
    fn diff_summary_counts_lines_per_file_and_new_files() {
        // Fixture sintética: um arquivo mudado, um novo (`--no-index`), um
        // apagado e um binário; no hunk, `+++`/`---` são linhas de conteúdo.
        let patch = "\
diff --git a/src/app.ts b/src/app.ts
index 1111111..2222222 100644
--- a/src/app.ts
+++ b/src/app.ts
@@ -1,3 +1,4 @@
 const a = 1;
-const b = 2;
+const b = 3;
+++contador;
@@ -10,2 +11,1 @@ function f() {
---x;
 fim();
\\ No newline at end of file
diff --git a/novo arquivo.md b/novo arquivo.md
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/novo arquivo.md
@@ -0,0 +1,2 @@
+# título
+texto
diff --git a/velho.txt b/velho.txt
deleted file mode 100644
index 4444444..0000000
--- a/velho.txt
+++ /dev/null
@@ -1 +0,0 @@
-adeus
diff --git a/logo.png b/logo.png
index 5555555..6666666 100644
Binary files a/logo.png and b/logo.png differ
";
        let s = diff_summary(patch);
        assert_eq!(s["files_changed"], 4);
        assert_eq!(s["added"], 4);
        assert_eq!(s["removed"], 3);
        assert_eq!(
            s["files"],
            json!([
                {"path": "src/app.ts", "added": 2, "removed": 2},
                {"path": "novo arquivo.md", "added": 2, "removed": 0},
                {"path": "velho.txt", "added": 0, "removed": 1},
                {"path": "logo.png", "added": 0, "removed": 0},
            ])
        );
        assert_eq!(diff_summary("")["files_changed"], 0);
    }

    #[test]
    fn turn_text_is_capped_and_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("context.md");
        std::fs::write(&path, "  feito: revisei o X.\n").unwrap();
        assert_eq!(
            turn_text(&path),
            Some(("feito: revisei o X.".into(), false))
        );
        let long = "é".repeat(STATUS_TEXT_MAX_CHARS + 10);
        std::fs::write(&path, &long).unwrap();
        let (text, truncated) = turn_text(&path).unwrap();
        assert_eq!(text.chars().count(), STATUS_TEXT_MAX_CHARS);
        assert!(truncated);
        std::fs::write(&path, " \n").unwrap();
        assert_eq!(turn_text(&path), None);
        assert_eq!(turn_text(&tmp.path().join("nao")), None);
    }

    #[test]
    fn new_nodes_stack_to_the_right_of_the_orchestrator() {
        let orch = Node {
            id: "o".into(),
            title: "Orquestrador".into(),
            kind: NodeKind::Note {
                markdown: String::new(),
            },
            position: Position { x: 30.0, y: 40.0 },
            size: Size { w: 370.0, h: 520.0 },
        };
        let first = new_node(
            "a".into(),
            "api".into(),
            NodeKind::Browser { url: String::new() },
            Some(&orch),
            &[],
        );
        assert_eq!(first.position, Position { x: 540.0, y: 40.0 });
        let second = new_node(
            "b".into(),
            "b".into(),
            NodeKind::Browser { url: String::new() },
            Some(&orch),
            &[&first],
        );
        assert_eq!(second.position.y, 40.0 + 420.0 + 40.0);
    }
}
