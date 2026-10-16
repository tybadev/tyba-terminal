//! Ferramentas do Orbit: todo nó de agente cria nós no run ativo e escreve
//! o plano (o agente criado por outro, sem `orbit_create_*`).
//!
//! O pedido chega do `tyba _orbit-mcp` ([`super::mcp`]) pelo socket de
//! hooks da sessão dele e cai em [`super::run::Runs::orchestrator_call`]. Aqui
//! mora o que não depende do run: ler e validar os argumentos de cada
//! ferramenta (mesma régua do salvar — pasta absoluta que existe, URL só
//! local, sem caractere de controle), o nó que nasce deles, o retrato do log
//! para o `orbit_node_status` e a junção dos nós mantidos no grafo.
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
/// Linhas do log no `orbit_node_status`.
pub const STATUS_LOG_LINES: usize = 40;
/// Quanto do fim do `log.txt` se lê para achar as últimas linhas.
const STATUS_LOG_TAIL_BYTES: u64 = 64 * 1024;
/// Teto do texto do turno no `orbit_node_status`; o resto o agente lê
/// pelo caminho do `context.md`.
pub const STATUS_TEXT_MAX_CHARS: usize = 8_000;
/// Arquivos listados no resumo do diff (os totais contam todos).
const STATUS_DIFF_MAX_FILES: usize = 100;
/// Teto de passos do `orbit_plan`.
pub const MAX_PLAN_STEPS: usize = 40;
/// Teto, em caracteres, do texto de um passo e do `made`.
pub const MAX_PLAN_TEXT_CHARS: usize = 300;
/// Título da nota de plano.
pub const PLAN_TITLE: &str = "Plano";

/// Quem atende as ferramentas da sessão de um agente do Orbit:
/// `(ferramenta, argumentos) → resultado`. Montado pelo run para o nó,
/// levado até o handler do socket de hooks da sessão. `can_create` é falso
/// no agente criado por outro agente (sem cascata): o `tools/list` dele não
/// anuncia as `orbit_create_*`, e o core recusa a chamada mesmo assim.
#[derive(Clone)]
pub struct ToolHandler {
    pub call: Arc<ToolFn>,
    pub can_create: bool,
}

/// `(ferramenta, argumentos) → resultado`.
pub type ToolFn = dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync;

impl ToolHandler {
    pub fn call(&self, tool: &str, args: &Value) -> Result<Value, String> {
        (self.call)(tool, args)
    }
}

/// As `orbit_create_*`: o que o agente criado não tem.
pub fn is_create_tool(tool: &str) -> bool {
    tool.starts_with("orbit_create_")
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
    /// O plano inteiro, já validado e redigido.
    Plan {
        steps: Vec<PlanStep>,
    },
    /// Abrir `url` (já local) no navegador ligado por `controla`. Sem `node`
    /// (o `tyba _orbit-open` do `$BROWSER`): o ligado mais recente.
    BrowserOpen {
        node: Option<String>,
        url: String,
    },
    /// Os agentes ligados por `talk`.
    Peers,
    /// Texto (já pela régua de [`super::talk::message_text`]) para a fila de `to`.
    Send {
        to: String,
        text: String,
    },
    /// Ler a nota ligada por `memory`.
    NoteRead {
        node: String,
    },
    /// Trocar (ou completar, com `append`) o texto da nota ligada por `memory`.
    NoteWrite {
        node: String,
        text: String,
        append: bool,
    },
}

/// Estado de um passo do plano.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepState {
    Todo,
    Doing,
    Done,
    Failed,
}

/// Um passo do plano do orquestrador; `made` é o nó que ele criou no passo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanStep {
    pub text: String,
    pub state: StepState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made: Option<String>,
}

/// A nota de plano de um orquestrador, como ficou no grafo: cópia no run
/// para o histórico e para o front pôr a nota e a ligação no canvas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub orchestrator: String,
    pub note: Node,
    pub edge: Edge,
    pub steps: Vec<PlanStep>,
    pub updated_at: u64,
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
struct BrowserOpenArgs {
    #[serde(default)]
    node: Option<String>,
    url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendArgs {
    to: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteReadArgs {
    node: String,
}

#[derive(Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum WriteMode {
    #[default]
    Replace,
    Append,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteWriteArgs {
    node: String,
    text: String,
    #[serde(default)]
    mode: WriteMode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanArgs {
    steps: Vec<PlanStep>,
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
        "orbit_create_process" => {
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
        "orbit_create_agent" => {
            let a: AgentArgs = args(tool, value)?;
            required("title", &a.title)?;
            required("prompt", &a.prompt)?;
            existing_dir(&a.dir)?;
            Ok(ToolCall::Create {
                title: a.title,
                // O agente criado tem as ferramentas menos as de criar
                // (sem cascata): o run sabe que ele é criado.
                kind: NodeKind::Agent {
                    runner: a.runner,
                    role: String::new(),
                    prompt: a.prompt,
                    dir: a.dir,
                },
                from_node: None,
            })
        }
        "orbit_create_browser" => {
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
        "orbit_node_status" => {
            let a: NodeArgs = args(tool, value)?;
            required("node_id", &a.node_id)?;
            Ok(ToolCall::Status { node_id: a.node_id })
        }
        "orbit_stop_node" => {
            let a: NodeArgs = args(tool, value)?;
            required("node_id", &a.node_id)?;
            Ok(ToolCall::Stop { node_id: a.node_id })
        }
        "orbit_browser_open" => {
            let a: BrowserOpenArgs = args(tool, value)?;
            let url = a.url.trim().to_string();
            required("url", &url)?;
            Ok(ToolCall::BrowserOpen {
                node: a.node.filter(|n| !n.trim().is_empty()),
                url: browser_url(&url)?,
            })
        }
        "orbit_peers" => {
            let _: EmptyArgs = args(tool, value)?;
            Ok(ToolCall::Peers)
        }
        "orbit_send" => {
            let a: SendArgs = args(tool, value)?;
            required("to", &a.to)?;
            Ok(ToolCall::Send {
                to: a.to,
                text: super::talk::message_text(&a.text)?,
            })
        }
        "orbit_note_read" => {
            let a: NoteReadArgs = args(tool, value)?;
            required("node", &a.node)?;
            Ok(ToolCall::NoteRead { node: a.node })
        }
        "orbit_note_write" => {
            let a: NoteWriteArgs = args(tool, value)?;
            required("node", &a.node)?;
            // A régua do texto (controle, teto, redação) depende do texto
            // atual da nota no `append`: o run aplica.
            Ok(ToolCall::NoteWrite {
                node: a.node,
                text: a.text,
                append: a.mode == WriteMode::Append,
            })
        }
        "orbit_plan" => {
            let a: PlanArgs = args(tool, value)?;
            Ok(ToolCall::Plan {
                steps: plan_steps(a.steps)?,
            })
        }
        _ => Err(format!("ferramenta desconhecida: {tool}")),
    }
}

/// Recusa de `file://`: o iframe não abre arquivo (a Fase 5 abre).
pub const FILE_URL_REFUSED: &str = "arquivo local: sirva por localhost (ex.: python3 -m \
http.server na pasta) e abra o http://localhost:<porta>; abrir arquivo direto ainda não existe";

/// A url que o agente manda abrir: `file://` ganha a explicação própria; o
/// resto, a régua do salvar ([`service::is_local_url`]).
fn browser_url(url: &str) -> Result<String, String> {
    if url
        .get(..5)
        .is_some_and(|s| s.eq_ignore_ascii_case("file:"))
    {
        return Err(FILE_URL_REFUSED.into());
    }
    if !service::is_local_url(url) {
        return Err(format!(
            "url recusada: só local (http/https em {})",
            service::LOCAL_HOSTS
        ));
    }
    Ok(url.to_string())
}

/// Texto de uma linha do plano: não vazio, sem nenhum caractere de controle
/// nem separador de linha (cada passo é uma linha do checklist), até
/// [`MAX_PLAN_TEXT_CHARS`], e redigido.
fn plan_line(what: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{what} vazio"));
    }
    // Separador de linha/parágrafo (U+2028/U+2029) também quebra a linha.
    if value
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err(format!("{what} contém caractere de controle"));
    }
    if value.chars().count() > MAX_PLAN_TEXT_CHARS {
        return Err(format!(
            "{what} passa do teto de {MAX_PLAN_TEXT_CHARS} caracteres"
        ));
    }
    Ok(crate::session::redact::redact(value).into_owned())
}

/// Os passos do `orbit_plan`: de 1 a [`MAX_PLAN_STEPS`], cada texto pela
/// régua de [`plan_line`]. `made` vazio conta como ausente.
fn plan_steps(steps: Vec<PlanStep>) -> Result<Vec<PlanStep>, String> {
    if steps.is_empty() {
        return Err("steps vazio: mande a lista inteira de passos".into());
    }
    if steps.len() > MAX_PLAN_STEPS {
        return Err(format!(
            "plano com {} passos passa do teto de {MAX_PLAN_STEPS}",
            steps.len()
        ));
    }
    steps
        .into_iter()
        .enumerate()
        .map(|(i, step)| {
            let n = i + 1;
            let made = match step.made.as_deref().map(str::trim) {
                None | Some("") => None,
                Some(m) => Some(plan_line(&format!("steps[{n}].made"), m)?),
            };
            Ok(PlanStep {
                text: plan_line(&format!("steps[{n}].text"), &step.text)?,
                state: step.state,
                made,
            })
        })
        .collect()
}

/// O texto da nota de plano: um checklist em markdown (`- [ ]` falta,
/// `- [~]` fazendo, `- [x]` pronto, `- [!]` falhou) e, sob o passo, a linha
/// recuada `  criou <made>`.
pub fn plan_markdown(steps: &[PlanStep]) -> String {
    let mut out = String::new();
    for step in steps {
        let mark = match step.state {
            StepState::Todo => ' ',
            StepState::Doing => '~',
            StepState::Done => 'x',
            StepState::Failed => '!',
        };
        out.push_str(&format!("- [{mark}] {}\n", step.text));
        if let Some(made) = &step.made {
            out.push_str(&format!("  criou {made}\n"));
        }
    }
    out
}

/// O texto pode virar plano sem apagar nada do dono: vazio, ou só o
/// checklist que [`plan_markdown`] escreve (o mesmo que o `parseChecklist`
/// da tela lê).
pub fn is_plan_text(markdown: &str) -> bool {
    let mut step = false;
    for line in markdown.lines().map(str::trim_end) {
        if line.is_empty() {
            continue;
        }
        let checked = line
            .strip_prefix("- [")
            .and_then(|r| r.split_at_checked(1))
            .filter(|(mark, _)| matches!(*mark, " " | "~" | "x" | "X" | "!"))
            .and_then(|(_, r)| r.strip_prefix("] "))
            .is_some_and(|text| !text.trim().is_empty());
        if checked {
            step = true;
            continue;
        }
        let made = line.len() - line.trim_start_matches(' ').len() >= 2
            && line
                .trim_start_matches(' ')
                .strip_prefix("criou ")
                .is_some_and(|m| !m.trim().is_empty());
        if !(made && step) {
            return false;
        }
    }
    true
}

/// A nota de plano nova: logo abaixo do orquestrador, num espaço livre.
pub fn plan_note(id: String, anchor: Option<&Node>, occupied: &[&Node], markdown: String) -> Node {
    const GAP_Y: f64 = 60.0;
    let kind = NodeKind::Note {
        markdown,
        locked: false,
    };
    let size = default_size(&kind);
    let want = anchor
        .map(|o| Position {
            x: o.position.x,
            y: o.position.y + o.size.h + GAP_Y,
        })
        .unwrap_or(Position { x: 0.0, y: 0.0 });
    Node {
        id,
        title: PLAN_TITLE.to_string(),
        kind,
        position: free_spot(want, &size, occupied),
        size,
    }
}

/// Tamanho do nó criado (o mesmo `DEFAULT_SIZE` do front). Agente e processo
/// nascem com tamanho de trabalho: o TUI do claude cabe com conversa à vista.
fn default_size(kind: &NodeKind) -> Size {
    match kind {
        NodeKind::Agent { .. } | NodeKind::Process { .. } => Size { w: 560.0, h: 440.0 },
        NodeKind::Browser { .. } => Size { w: 640.0, h: 420.0 },
        _ => Size { w: 360.0, h: 250.0 },
    }
}

/// Folga mínima entre dois nós (a mesma `LAYOUT_GAP` do front).
const LAYOUT_GAP: f64 = 40.0;

fn overlaps(x: f64, y: f64, size: &Size, other: &Node) -> bool {
    x < other.position.x + other.size.w + LAYOUT_GAP
        && other.position.x < x + size.w + LAYOUT_GAP
        && y < other.position.y + other.size.h + LAYOUT_GAP
        && other.position.y < y + size.h + LAYOUT_GAP
}

/// A primeira posição livre a partir de `want` (achados canvas-6/nos-8): desce
/// passando por baixo de quem estiver no caminho e, sem espaço na coluna,
/// abre outra à direita. Espelha `freeSpot` (`src/lib/graphLayout.ts`).
pub fn free_spot(want: Position, size: &Size, occupied: &[&Node]) -> Position {
    const MAX_LANES: usize = 12;
    const MAX_HOPS: usize = 60;
    let mut last = want;
    for lane in 0..MAX_LANES {
        let x = want.x + lane as f64 * (size.w + LAYOUT_GAP);
        let mut y = want.y;
        for _ in 0..MAX_HOPS {
            let Some(hit) = occupied.iter().find(|n| overlaps(x, y, size, n)) else {
                return Position { x, y };
            };
            last = Position { x, y };
            y = hit.position.y + hit.size.h + LAYOUT_GAP;
            if y - want.y > 4000.0 {
                break;
            }
        }
    }
    last
}

/// O nó novo, à direita de `anchor` (o orquestrador, ou o `from_node` do
/// navegador) e num espaço livre: nunca cai em cima de um nó do grafo nem de
/// outro criado (`occupied`).
pub fn new_node(
    id: String,
    title: String,
    kind: NodeKind,
    anchor: Option<&Node>,
    occupied: &[&Node],
) -> Node {
    const GAP_X: f64 = 140.0;
    let size = default_size(&kind);
    let want = anchor
        .map(|o| Position {
            x: o.position.x + o.size.w + GAP_X,
            y: o.position.y,
        })
        .unwrap_or(Position { x: 0.0, y: 0.0 });
    let position = free_spot(want, &size, occupied);
    Node {
        id,
        title,
        kind,
        position,
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

/// Os criados que o "Manter" leva: o `node_id` (o "Manter" do nó) ou, sem
/// ele, os do `orchestrator` (o "Manter tudo" do ⋯ dele) — nunca os que outro
/// orquestrador do mesmo run criou.
pub fn pick_to_keep(
    created: Vec<Created>,
    node_id: Option<&str>,
    orchestrator: Option<&str>,
) -> Result<Vec<Created>, String> {
    let picked: Vec<Created> = match (node_id, orchestrator) {
        (Some(id), _) => created.into_iter().filter(|c| c.node.id == id).collect(),
        (None, Some(o)) => created.into_iter().filter(|c| c.created_by == o).collect(),
        (None, None) => return Err("informe node_id ou orchestrator".into()),
    };
    if node_id.is_some() && picked.is_empty() {
        return Err("nó criado não encontrado ou já mantido".into());
    }
    Ok(picked)
}

/// "Manter no grafo": os nós de `keep` entram no grafo como nós normais (sem
/// `created_by`, que só existe no run). Id que já está no grafo não entra de
/// novo. Das ligações dos criados do run (`all`), entram as que tocam um nó
/// de `keep` e têm as duas pontas no grafo: o "Manter" de um nó só deixa de
/// fora a url de um criado que ainda não foi mantido, e ela volta quando ele
/// for.
pub fn merge_kept(graph: &mut Graph, keep: &[Created], all: &[Created]) {
    for c in keep {
        if !graph.nodes.iter().any(|n| n.id == c.node.id) {
            graph.nodes.push(c.node.clone());
        }
    }
    let kept = |id: &str| keep.iter().any(|c| c.node.id == id);
    for edge in all.iter().chain(keep).flat_map(|c| &c.edges) {
        let ends = [&edge.from, &edge.to]
            .into_iter()
            .all(|id| graph.nodes.iter().any(|n| &n.id == id));
        if (kept(&edge.from) || kept(&edge.to))
            && ends
            && !graph.edges.iter().any(|e| e.id == edge.id)
        {
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
            "orbit_create_process",
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
                "orbit_create_process",
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
            "orbit_create_process",
            &json!({"title": "api", "command": "echo oi\rrm -rf ~", "dir": dir()}),
        )
        .unwrap_err();
        assert!(err.contains("controle"), "{err}");
        let err = parse_call(
            "orbit_create_agent",
            &json!({"title": "rev", "runner": "claude", "prompt": "oi\u{1b}[2J", "dir": dir()}),
        )
        .unwrap_err();
        assert!(err.contains("controle"), "{err}");
        assert!(parse_call(
            "orbit_create_process",
            &json!({"title": "  ", "command": "x", "dir": dir()}),
        )
        .is_err());
        // Campo a mais, tipo errado, runner desconhecido, saída file.
        for (tool, args) in [
            (
                "orbit_create_process",
                json!({"title": "a", "command": "x", "dir": dir(), "sudo": true}),
            ),
            ("orbit_create_process", json!({"title": 1})),
            (
                "orbit_create_agent",
                json!({"title": "a", "runner": "gpt", "prompt": "x", "dir": dir()}),
            ),
            (
                "orbit_create_process",
                json!({"title": "a", "command": "x", "dir": dir(), "output": "file"}),
            ),
        ] {
            assert!(parse_call(tool, &args).is_err(), "{tool} {args}");
        }
    }

    #[test]
    fn only_the_create_tools_are_create_tools() {
        let create: Vec<&str> = super::super::mcp::TOOLS
            .iter()
            .copied()
            .filter(|t| is_create_tool(t))
            .collect();
        assert_eq!(
            create,
            [
                "orbit_create_process",
                "orbit_create_agent",
                "orbit_create_browser"
            ]
        );
    }

    #[test]
    fn create_agent_builds_an_agent_node() {
        let call = parse_call(
            "orbit_create_agent",
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
                ..
            }
        ));
    }

    #[test]
    fn browser_url_must_be_local_or_come_from_a_node() {
        let ok = |args: Value| parse_call("orbit_create_browser", &args);
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
    fn browser_open_takes_only_a_local_url_and_explains_file() {
        let open = |args: Value| parse_call("orbit_browser_open", &args);
        assert_eq!(
            open(json!({"node": "b", "url": " http://localhost:5173/login "})).unwrap(),
            ToolCall::BrowserOpen {
                node: Some("b".into()),
                url: "http://localhost:5173/login".into()
            }
        );
        // Sem node (o `$BROWSER`): o core escolhe o ligado mais recente.
        assert_eq!(
            open(json!({"url": "https://code.muriki.test"})).unwrap(),
            ToolCall::BrowserOpen {
                node: None,
                url: "https://code.muriki.test".into()
            }
        );
        for file in ["file:///Users/x/index.html", "FILE:///tmp/a.html"] {
            let err = open(json!({"url": file})).unwrap_err();
            assert_eq!(err, FILE_URL_REFUSED, "{file}");
            assert!(err.contains("sirva por localhost"));
        }
        for bad in [
            "https://example.com",
            "http://user@localhost:1",
            "javascript:alert(1)",
        ] {
            let err = open(json!({"node": "b", "url": bad})).unwrap_err();
            assert!(err.contains("só local"), "{bad}: {err}");
        }
        assert!(open(json!({"node": "b"})).is_err());
        assert!(open(json!({"url": "  "})).is_err());
        assert!(open(json!({"url": "http://localhost:1", "x": 1})).is_err());
        assert!(open(json!({"url": "http://localhost:1/\u{1b}[2J"})).is_err());
    }

    #[test]
    fn talk_and_memory_calls_parse_and_validate() {
        use serde_json::json;
        assert_eq!(parse_call("orbit_peers", &json!({})), Ok(ToolCall::Peers));
        assert!(parse_call("orbit_peers", &json!({"x": 1})).is_err());
        assert_eq!(
            parse_call("orbit_send", &json!({"to": "b", "text": " oi\n "})),
            Ok(ToolCall::Send {
                to: "b".into(),
                text: "oi".into()
            })
        );
        assert!(parse_call("orbit_send", &json!({"to": "", "text": "oi"})).is_err());
        assert!(parse_call("orbit_send", &json!({"to": "b", "text": "\u{7}"})).is_err());
        let long = "x".repeat(super::super::talk::MAX_MESSAGE_CHARS + 1);
        assert!(parse_call("orbit_send", &json!({"to": "b", "text": long})).is_err());
        assert_eq!(
            parse_call("orbit_note_read", &json!({"node": "n"})),
            Ok(ToolCall::NoteRead { node: "n".into() })
        );
        assert_eq!(
            parse_call("orbit_note_write", &json!({"node": "n", "text": "a"})),
            Ok(ToolCall::NoteWrite {
                node: "n".into(),
                text: "a".into(),
                append: false
            })
        );
        assert_eq!(
            parse_call(
                "orbit_note_write",
                &json!({"node": "n", "text": "a", "mode": "append"})
            ),
            Ok(ToolCall::NoteWrite {
                node: "n".into(),
                text: "a".into(),
                append: true
            })
        );
        assert!(parse_call(
            "orbit_note_write",
            &json!({"node": "n", "text": "a", "mode": "prepend"})
        )
        .is_err());
    }

    #[test]
    fn unknown_tool_is_refused() {
        assert!(parse_call("orbit_delete_graph", &json!({})).is_err());
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

    fn note(id: &str, x: f64, y: f64, w: f64, h: f64) -> Node {
        Node {
            id: id.into(),
            title: id.into(),
            kind: NodeKind::Note {
                markdown: String::new(),
                locked: false,
            },
            position: Position { x, y },
            size: Size { w, h },
        }
    }

    #[test]
    fn new_nodes_stack_to_the_right_of_the_orchestrator() {
        let orch = note("o", 30.0, 40.0, 370.0, 520.0);
        let first = new_node(
            "a".into(),
            "api".into(),
            NodeKind::Browser { url: String::new() },
            Some(&orch),
            &[&orch],
        );
        assert_eq!(first.position, Position { x: 540.0, y: 40.0 });
        let second = new_node(
            "b".into(),
            "b".into(),
            NodeKind::Browser { url: String::new() },
            Some(&orch),
            &[&orch, &first],
        );
        assert_eq!(
            second.position,
            Position {
                x: 540.0,
                y: 40.0 + 420.0 + 40.0
            }
        );
    }

    #[test]
    fn new_node_never_lands_on_the_owners_nodes() {
        let orch = note("o", 0.0, 0.0, 360.0, 250.0);
        // O dono deixou um nó bem onde o criado cairia.
        let mine = note("m", 480.0, 0.0, 400.0, 300.0);
        let node = new_node(
            "c".into(),
            "c".into(),
            NodeKind::Browser { url: String::new() },
            Some(&orch),
            &[&orch, &mine],
        );
        assert_eq!(node.position, Position { x: 500.0, y: 340.0 });
        let size = node.size;
        for other in [&orch, &mine] {
            assert!(!overlaps(node.position.x, node.position.y, &size, other));
        }
    }

    #[test]
    fn free_spot_opens_a_new_column_when_this_one_is_full() {
        let wall = note("w", 0.0, -100.0, 200.0, 10_000.0);
        let size = Size { w: 100.0, h: 100.0 };
        let at = free_spot(Position { x: 0.0, y: 0.0 }, &size, &[&wall]);
        assert_eq!(at, Position { x: 280.0, y: 0.0 });
    }

    #[test]
    fn agent_and_process_start_with_working_size() {
        for kind in [
            NodeKind::Agent {
                runner: crate::orbit::Runner::Claude,
                role: String::new(),
                prompt: "subir o ambiente".into(),
                dir: String::new(),
            },
            NodeKind::Process {
                command: "bun dev".into(),
                output: Default::default(),
                file: None,
                dir: String::new(),
            },
        ] {
            let node = new_node("n".into(), "n".into(), kind, None, &[]);
            assert_eq!(node.size, Size { w: 560.0, h: 440.0 });
        }
    }

    #[test]
    fn plan_parses_valid_steps_and_redacts() {
        let key = format!("sk-{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6");
        let call = parse_call(
            "orbit_plan",
            &json!({"steps": [
                {"text": "Subir a api", "state": "done", "made": "muriki-api (:3000)"},
                {"text": format!("Testar com {key}"), "state": "doing"},
                {"text": "Escrever o que achou", "state": "todo", "made": "  "},
                {"text": "Caddy", "state": "failed"}
            ]}),
        )
        .unwrap();
        let ToolCall::Plan { steps } = call else {
            panic!("não é plano: {call:?}")
        };
        assert_eq!(steps.len(), 4);
        assert_eq!(steps[0].state, StepState::Done);
        assert_eq!(steps[0].made.as_deref(), Some("muriki-api (:3000)"));
        assert!(!steps[1].text.contains(&key), "{}", steps[1].text);
        // made só de espaço conta como ausente.
        assert_eq!(steps[2].made, None);
        assert_eq!(steps[3].state, StepState::Failed);
    }

    #[test]
    fn plan_refuses_bad_state_control_chars_and_ceilings() {
        let step = |text: &str, state: &str| json!({"text": text, "state": state});
        let bad = [
            json!({"steps": [step("a", "pronto")]}),
            json!({"steps": [step("linha\nnova", "todo")]}),
            json!({"steps": [step("tab\tno meio", "todo")]}),
            json!({"steps": [step("esc \u{1b}[31m", "todo")]}),
            json!({"steps": [{"text": "a", "state": "todo", "made": "x\u{7}"}]}),
            // Separadores de linha e parágrafo, e o NEL, também quebram a linha.
            json!({"steps": [step("linha\u{2028}nova", "todo")]}),
            json!({"steps": [step("parágrafo\u{2029}novo", "todo")]}),
            json!({"steps": [{"text": "a", "state": "todo", "made": "x\u{2028}y"}]}),
            json!({"steps": [step("nel\u{85}aqui", "todo")]}),
            json!({"steps": [step("", "todo")]}),
            json!({"steps": []}),
            json!({"steps": [{"text": "a", "state": "todo", "extra": 1}]}),
            json!({"steps": [step(&"a".repeat(MAX_PLAN_TEXT_CHARS + 1), "todo")]}),
            json!({"steps": (0..=MAX_PLAN_STEPS).map(|i| step(&format!("p{i}"), "todo")).collect::<Vec<_>>()}),
        ];
        for args in bad {
            assert!(parse_call("orbit_plan", &args).is_err(), "aceitou {args}");
        }
        // No teto passa.
        let full: Vec<Value> = (0..MAX_PLAN_STEPS)
            .map(|i| step(&format!("p{i}"), "todo"))
            .collect();
        assert!(parse_call("orbit_plan", &json!({"steps": full})).is_ok());
        assert!(parse_call(
            "orbit_plan",
            &json!({"steps": [step(&"a".repeat(MAX_PLAN_TEXT_CHARS), "todo")]})
        )
        .is_ok());
    }

    #[test]
    fn merge_kept_of_one_node_leaves_out_the_url_of_an_unkept_one() {
        let edge = |id: &str, from: &str, to: &str, kind| Edge {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            kind,
        };
        let made = |node: Node, edges: Vec<Edge>| Created {
            node,
            created_by: "o".into(),
            created_at: 0,
            edges,
            kept: false,
        };
        let web = made(note("web", 0.0, 0.0, 10.0, 10.0), vec![]);
        let page = made(
            note("page", 0.0, 0.0, 10.0, 10.0),
            vec![
                edge("u", "web", "page", crate::orbit::EdgeKind::Url),
                edge("c", "o", "page", crate::orbit::EdgeKind::Control),
            ],
        );
        let all = vec![web.clone(), page.clone()];
        let mut graph = Graph {
            id: "g".into(),
            name: "g".into(),
            dir: String::new(),
            nodes: vec![note("o", 0.0, 0.0, 10.0, 10.0)],
            edges: vec![],
            variables: Default::default(),
        };
        // Só a página: o controle do orquestrador vai; a url do web, ainda não.
        merge_kept(&mut graph, std::slice::from_ref(&page), &all);
        let ids = |g: &Graph| g.edges.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&graph), vec!["c"]);
        // Depois o web: a url dele para a página, já mantida, entra junto.
        merge_kept(&mut graph, std::slice::from_ref(&web), &all);
        assert_eq!(ids(&graph), vec!["c", "u"]);
        assert_eq!(graph.nodes.len(), 3);
    }

    #[test]
    fn plan_text_is_empty_or_the_checklist_plan_markdown_writes() {
        let steps = [
            PlanStep {
                text: "Subir a api".into(),
                state: StepState::Done,
                made: Some("api (:3000)".into()),
            },
            PlanStep {
                text: "Abrir".into(),
                state: StepState::Todo,
                made: None,
            },
        ];
        assert!(is_plan_text(&plan_markdown(&steps)));
        assert!(is_plan_text(""));
        assert!(is_plan_text("  \n\n"));
        assert!(is_plan_text("- [~] fazendo\n- [!] falhou\n- [X] pronto"));
        // Texto do dono não é plano.
        assert!(!is_plan_text("lembrar de testar o login"));
        assert!(!is_plan_text("- [ ] passo\nnota solta"));
        assert!(!is_plan_text("  criou api"));
        assert!(!is_plan_text("- [?] passo"));
        assert!(!is_plan_text("- [ ]  "));
    }

    #[test]
    fn keep_all_of_one_orchestrator_leaves_the_other_ones_out() {
        let made = |id: &str, by: &str| Created {
            node: note(id, 0.0, 0.0, 10.0, 10.0),
            created_by: by.into(),
            created_at: 0,
            edges: vec![],
            kept: false,
        };
        let all = vec![made("a1", "a"), made("b1", "b"), made("a2", "a")];
        let ids = |v: Vec<Created>| v.into_iter().map(|c| c.node.id).collect::<Vec<_>>();
        assert_eq!(
            ids(pick_to_keep(all.clone(), None, Some("a")).unwrap()),
            vec!["a1", "a2"]
        );
        assert_eq!(
            ids(pick_to_keep(all.clone(), None, Some("b")).unwrap()),
            vec!["b1"]
        );
        assert_eq!(
            ids(pick_to_keep(all.clone(), Some("b1"), None).unwrap()),
            vec!["b1"]
        );
        assert!(pick_to_keep(all.clone(), Some("x"), None).is_err());
        // Sem nó nem orquestrador não leva tudo.
        assert!(pick_to_keep(all, None, None).is_err());
    }

    #[test]
    fn plan_markdown_is_a_checklist_with_made_under_the_step() {
        let step = |text: &str, state, made: Option<&str>| PlanStep {
            text: text.into(),
            state,
            made: made.map(Into::into),
        };
        let md = plan_markdown(&[
            step("Subir a api", StepState::Done, Some("muriki-api (:3000)")),
            step("Abrir o login", StepState::Doing, None),
            step("Escrever", StepState::Todo, None),
            step("Caddy", StepState::Failed, Some("caddy")),
        ]);
        assert_eq!(
            md,
            "- [x] Subir a api\n  criou muriki-api (:3000)\n- [~] Abrir o login\n\
             - [ ] Escrever\n- [!] Caddy\n  criou caddy\n"
        );
    }

    #[test]
    fn plan_note_goes_below_the_orchestrator_in_a_free_spot() {
        let orq = Node {
            id: "o".into(),
            title: "o".into(),
            kind: NodeKind::Note {
                markdown: String::new(),
                locked: false,
            },
            position: Position { x: 100.0, y: 100.0 },
            size: Size { w: 400.0, h: 300.0 },
        };
        let note = plan_note("n".into(), Some(&orq), &[&orq], "- [ ] a\n".into());
        assert_eq!(note.title, PLAN_TITLE);
        assert_eq!(note.position.x, 100.0);
        assert!(note.position.y >= 100.0 + 300.0, "{:?}", note.position);
        assert!(!overlaps(
            note.position.x,
            note.position.y,
            &note.size,
            &orq
        ));
        assert_eq!(
            note.kind,
            NodeKind::Note {
                markdown: "- [ ] a\n".into(),
                locked: false,
            }
        );
    }
}
