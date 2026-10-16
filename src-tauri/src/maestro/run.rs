//! Motor de execução do Maestro: processos, notas, agentes e integrações.
//!
//! ▶ Executar cria um [`Run`] e uma pasta `~/.tyba/maestro/runs/<run>/<nó>/
//! {in,out}` (0700). Um nó começa quando todas as ligações que chegam nele já
//! têm valor; independentes rodam em paralelo. Falha pula tudo que depende do
//! nó; Parar mata o process group de cada nó rodando.
//!
//! O agendador ([`Scheduler`]) é puro. Quem sobe processo, mata e avisa o front
//! é o [`RunHost`] — no app, [`TauriHost`], que passa pelo `SessionManager`.
//! Cada nó roda na própria pasta (`Graph::node_dir`), como o dono. Sem worktree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::agent::{AgentTurn, Snapshot};
use super::orchestrator::{self, Created, ToolCall, ToolHandler, MAX_CREATED};
use super::service::{self, ServiceReady, ServiceSource};
use super::{Edge, EdgeKind, Graph, Integration, Node, NodeKind, OutputKind, Runner, Stored};
use crate::session::store::Store;

/// Execuções guardadas por grafo; a poda apaga a pasta das mais antigas.
pub const KEEP_RUNS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Running,
    Done,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeStatus {
    Waiting,
    Running,
    Done,
    Failed,
    Skipped,
    Stopped,
}

impl NodeStatus {
    fn is_terminal(self) -> bool {
        !matches!(self, NodeStatus::Waiting | NodeStatus::Running)
    }
}

/// Tempos em ms desde a época.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub graph_id: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub status: RunStatus,
    pub nodes: HashMap<String, NodeRun>,
    /// Cresce a cada retrato tirado do agendador. Retratos são publicados fora
    /// do lock e podem chegar fora de ordem: a gravação ignora o mais velho.
    #[serde(default)]
    pub version: u64,
    /// Run de "Testar com a última entrada": não ocupa vaga na poda dos runs.
    #[serde(default)]
    pub test: bool,
    /// Sessões de agente do run ainda vivas (o run pode já ter fechado).
    /// Calculado na leitura; o que fica gravado no banco não vale.
    #[serde(default)]
    pub live_agents: Vec<String>,
    /// Nós que um orquestrador criou neste run (efêmeros: não estão no grafo
    /// salvo, a menos que o dono os mantenha).
    #[serde(default)]
    pub created: Vec<Created>,
}

/// Motivo do nó que rodava quando o app fechou.
pub const APP_CLOSED: &str = "o TYBA fechou durante a execução";

impl Run {
    /// Fecha o retrato de um run que o app deixou aberto ao fechar: o PTY de
    /// cada nó morreu junto (`kill_all` no quit). Rodando vira `stopped` com
    /// [`APP_CLOSED`], esperando vira `skipped`, e nenhuma sessão fica — a
    /// que estava aqui não existe mais. A versão sobe para a gravação valer.
    fn close_interrupted(&mut self, now: u64) {
        for n in self.nodes.values_mut() {
            match n.status {
                NodeStatus::Running => {
                    n.status = NodeStatus::Stopped;
                    n.finished_at = Some(now);
                    n.reason = Some(APP_CLOSED.into());
                }
                NodeStatus::Waiting => n.status = NodeStatus::Skipped,
                _ => {}
            }
            n.session_id = None;
            n.orchestrating = false;
        }
        self.status = RunStatus::Stopped;
        self.finished_at = Some(now);
        self.version += 1;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeRun {
    pub status: NodeStatus,
    /// Sessão do terminal do nó, para o front desenhar. Fica fora da grade.
    pub session_id: Option<String>,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub exit_code: Option<i32>,
    /// Tipo da saída → caminho do arquivo com o valor.
    #[serde(default)]
    pub outputs: HashMap<EdgeKind, String>,
    /// Por que falhou, quando não é só o exit code.
    #[serde(default)]
    pub reason: Option<String>,
    /// URL local do nó: a que o navegador mostra, a do serviço, ou a porta
    /// que a sessão do processo abriu (aparece com o processo ainda rodando).
    #[serde(default)]
    pub url: Option<String>,
    /// Orquestrador que fechou o primeiro turno: as saídas já valem para quem
    /// depende dele, mas o nó segue rodando (e o run aberto) até a sessão sair.
    #[serde(default)]
    pub orchestrating: bool,
}

impl NodeRun {
    fn waiting() -> Self {
        Self {
            status: NodeStatus::Waiting,
            session_id: None,
            started_at: None,
            finished_at: None,
            exit_code: None,
            outputs: HashMap::new(),
            reason: None,
            url: None,
            orchestrating: false,
        }
    }
}

/// Como um nó terminou.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub outputs: HashMap<EdgeKind, String>,
    pub reason: Option<String>,
    pub url: Option<String>,
}

impl Outcome {
    fn failed(reason: impl Into<String>) -> Self {
        Self {
            ok: false,
            reason: Some(reason.into()),
            ..Self::default()
        }
    }
}

/// Agendador puro: só estado, sem processo nem disco.
#[derive(Debug, Clone)]
pub struct Scheduler {
    run: Run,
    /// Ordem dos nós no grafo: dá ordem estável ao `ready`.
    order: Vec<String>,
    /// Quem alimenta cada nó, e com que tipo.
    sources: HashMap<String, Vec<(String, EdgeKind)>>,
    dependents: HashMap<String, Vec<String>>,
    /// Integrações manuais: esperam o ▶ do dono mesmo com as entradas prontas.
    held: HashSet<String>,
}

impl Scheduler {
    pub fn new(graph: &Graph, run_id: String, now: u64) -> Self {
        let mut sources: HashMap<String, Vec<(String, EdgeKind)>> = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        for edge in &graph.edges {
            sources
                .entry(edge.to.clone())
                .or_default()
                .push((edge.from.clone(), edge.kind));
            dependents
                .entry(edge.from.clone())
                .or_default()
                .push(edge.to.clone());
        }
        let mut run = Run {
            id: run_id,
            graph_id: graph.id.clone(),
            started_at: now,
            finished_at: None,
            status: RunStatus::Running,
            nodes: graph
                .nodes
                .iter()
                .map(|n| (n.id.clone(), NodeRun::waiting()))
                .collect(),
            version: 0,
            test: false,
            live_agents: Vec::new(),
            created: Vec::new(),
        };
        if run.nodes.is_empty() {
            run.status = RunStatus::Done;
            run.finished_at = Some(now);
        }
        Self {
            run,
            order: graph.nodes.iter().map(|n| n.id.clone()).collect(),
            sources,
            dependents,
            held: HashSet::new(),
        }
    }

    /// Segura o nó em `waiting` até [`Scheduler::release`].
    pub fn hold(&mut self, id: &str) {
        if self.run.nodes.contains_key(id) {
            self.held.insert(id.to_string());
        }
    }

    /// O ▶ do dono num nó manual: começa assim que as entradas chegarem.
    pub fn release(&mut self, id: &str) -> Result<(), String> {
        if self.status(id) != Some(NodeStatus::Waiting) {
            return Err("o nó não está esperando".into());
        }
        if !self.held.remove(id) {
            return Err("o nó não é de integração manual".into());
        }
        Ok(())
    }

    pub fn run(&self) -> &Run {
        &self.run
    }

    /// Retrato para publicar, com versão nova. Tirado com o lock na mão, a
    /// ordem das versões é a ordem dos estados.
    pub fn snapshot(&mut self) -> Run {
        self.run.version += 1;
        self.run.clone()
    }

    fn status(&self, id: &str) -> Option<NodeStatus> {
        self.run.nodes.get(id).map(|n| n.status)
    }

    /// A ligação já tem valor: a origem terminou, é um orquestrador que já
    /// publicou o primeiro turno, ou é `url` e a origem (um processo ainda
    /// rodando) já publicou a url.
    fn has_value(&self, source: &str, kind: EdgeKind) -> bool {
        self.run.nodes.get(source).is_some_and(|n| {
            n.status == NodeStatus::Done
                || (n.status == NodeStatus::Running
                    && (n.orchestrating
                        || (kind == EdgeKind::Url && n.outputs.contains_key(&EdgeKind::Url))))
        })
    }

    /// Nós esperando cujas entradas todas já têm valor.
    pub fn ready(&self) -> Vec<String> {
        if self.run.status != RunStatus::Running {
            return Vec::new();
        }
        self.order
            .iter()
            .filter(|id| self.status(id) == Some(NodeStatus::Waiting))
            .filter(|id| !self.held.contains(*id))
            .filter(|id| {
                self.sources
                    .get(*id)
                    .into_iter()
                    .flatten()
                    .all(|(s, kind)| self.has_value(s, *kind))
            })
            .cloned()
            .collect()
    }

    pub fn mark_running(&mut self, id: &str, now: u64) {
        if let Some(n) = self.run.nodes.get_mut(id) {
            if n.status == NodeStatus::Waiting {
                n.status = NodeStatus::Running;
                n.started_at = Some(now);
            }
        }
    }

    pub fn set_session(&mut self, id: &str, session_id: String) {
        if let Some(n) = self.run.nodes.get_mut(id) {
            n.session_id = Some(session_id);
        }
    }

    /// Saída que sai antes do fim (a url do processo). Só em nó rodando, e
    /// a primeira vale. Devolve `true` se gravou.
    pub fn publish_url(&mut self, id: &str, path: String, url: String) -> bool {
        let Some(n) = self.run.nodes.get_mut(id) else {
            return false;
        };
        if n.status != NodeStatus::Running || n.outputs.contains_key(&EdgeKind::Url) {
            return false;
        }
        n.outputs.insert(EdgeKind::Url, path);
        n.url = Some(url);
        true
    }

    /// O orquestrador fechou o primeiro turno: publica as saídas para quem
    /// depende dele, mas o nó segue rodando até a sessão sair. Devolve `true`
    /// se gravou.
    pub fn orchestrate(&mut self, id: &str, outputs: HashMap<EdgeKind, String>) -> bool {
        let Some(n) = self.run.nodes.get_mut(id) else {
            return false;
        };
        if n.status != NodeStatus::Running || n.orchestrating {
            return false;
        }
        n.outputs.extend(outputs);
        n.orchestrating = true;
        true
    }

    fn orchestrating(&self, id: &str) -> bool {
        self.run
            .nodes
            .get(id)
            .is_some_and(|n| n.status == NodeStatus::Running && n.orchestrating)
    }

    /// Fecha um nó rodando. Falha pula, em cascata, tudo que depende dele.
    /// Devolve `false` quando o nó não estava rodando (ex.: já parado).
    pub fn finish(&mut self, id: &str, outcome: Outcome, now: u64) -> bool {
        let Some(n) = self.run.nodes.get_mut(id) else {
            return false;
        };
        if n.status != NodeStatus::Running {
            return false;
        }
        n.status = if outcome.ok {
            NodeStatus::Done
        } else {
            NodeStatus::Failed
        };
        n.orchestrating = false;
        n.finished_at = Some(now);
        n.exit_code = outcome.exit_code;
        // O que saiu antes do fim (url) fica.
        let early = std::mem::replace(&mut n.outputs, outcome.outputs);
        for (kind, path) in early {
            n.outputs.entry(kind).or_insert(path);
        }
        n.reason = outcome.reason;
        if outcome.url.is_some() {
            n.url = outcome.url;
        }
        if !outcome.ok {
            self.skip_dependents(id);
        }
        self.settle(now);
        true
    }

    /// Nó novo num run rodando (o que o orquestrador criou): espera as
    /// entradas como qualquer outro. Origem que já falhou ou foi pulada pula
    /// o nó novo na hora — senão ele esperaria para sempre.
    pub fn add_node(&mut self, id: &str, sources: &[(String, EdgeKind)]) -> Result<(), String> {
        if self.run.status != RunStatus::Running {
            return Err("a execução não está rodando".into());
        }
        if self.run.nodes.contains_key(id) {
            return Err(format!("nó repetido: {id}"));
        }
        self.run.nodes.insert(id.to_string(), NodeRun::waiting());
        self.order.push(id.to_string());
        for (from, kind) in sources {
            self.sources
                .entry(id.to_string())
                .or_default()
                .push((from.clone(), *kind));
            self.dependents
                .entry(from.clone())
                .or_default()
                .push(id.to_string());
        }
        let dead = sources.iter().any(|(from, _)| {
            matches!(
                self.status(from),
                Some(NodeStatus::Failed | NodeStatus::Skipped | NodeStatus::Stopped)
            )
        });
        if dead {
            if let Some(n) = self.run.nodes.get_mut(id) {
                n.status = NodeStatus::Skipped;
            }
        }
        Ok(())
    }

    /// Para um nó só (o `maestro_stop_node`): rodando vira `stopped`,
    /// esperando vira `skipped`, e quem depende dele é pulado. Devolve a
    /// sessão a matar quando ele rodava.
    pub fn stop_node(&mut self, id: &str, now: u64) -> Option<String> {
        let n = self.run.nodes.get_mut(id)?;
        let session = match n.status {
            NodeStatus::Running => {
                n.status = NodeStatus::Stopped;
                n.finished_at = Some(now);
                n.session_id.clone()
            }
            NodeStatus::Waiting => {
                n.status = NodeStatus::Skipped;
                None
            }
            _ => return None,
        };
        self.skip_dependents(id);
        self.settle(now);
        session
    }

    fn skip_dependents(&mut self, id: &str) {
        let mut stack = vec![id.to_string()];
        while let Some(current) = stack.pop() {
            for next in self.dependents.get(&current).cloned().unwrap_or_default() {
                if let Some(n) = self.run.nodes.get_mut(&next) {
                    if n.status == NodeStatus::Waiting {
                        n.status = NodeStatus::Skipped;
                        stack.push(next);
                    }
                }
            }
        }
    }

    /// Fecha o run quando não sobra nó esperando nem rodando.
    fn settle(&mut self, now: u64) {
        if self.run.status != RunStatus::Running
            || !self.run.nodes.values().all(|n| n.status.is_terminal())
        {
            return;
        }
        let failed = self
            .run
            .nodes
            .values()
            .any(|n| n.status == NodeStatus::Failed);
        self.run.status = if failed {
            RunStatus::Failed
        } else {
            RunStatus::Done
        };
        self.run.finished_at = Some(now);
    }

    /// Parar: rodando vira `stopped`, esperando vira `skipped`. Devolve as
    /// sessões a matar.
    pub fn stop(&mut self, now: u64) -> Vec<String> {
        if self.run.status != RunStatus::Running {
            return Vec::new();
        }
        let mut kill = Vec::new();
        for n in self.run.nodes.values_mut() {
            match n.status {
                NodeStatus::Running => {
                    n.status = NodeStatus::Stopped;
                    n.finished_at = Some(now);
                    if let Some(s) = &n.session_id {
                        kill.push(s.clone());
                    }
                }
                NodeStatus::Waiting => n.status = NodeStatus::Skipped,
                _ => {}
            }
        }
        self.run.status = RunStatus::Stopped;
        self.run.finished_at = Some(now);
        kill
    }
}

/// Grafo importado e não confirmado não roda — no core, não só no botão.
pub fn startable(stored: Stored<Graph>) -> Result<Graph, String> {
    if !stored.consented {
        return Err(
            "grafo importado ainda não confirmado: revise e confirme antes de executar".into(),
        );
    }
    Ok(stored.value)
}

/// Id de nó vira nome de pasta: só o que não escapa de `runs/<run>/`.
pub(super) fn safe_component(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Nome do arquivo de entrada por tipo (rules: Execução, passo 3).
fn base_input_name(kind: EdgeKind, file_name: Option<&str>) -> String {
    match kind {
        EdgeKind::Context => "context.md".into(),
        EdgeKind::Diff => "diff.patch".into(),
        EdgeKind::File => file_name.unwrap_or("arquivo").to_string(),
        EdgeKind::Log => "log.txt".into(),
        EdgeKind::Json => "input.json".into(),
        EdgeKind::Url => "url.txt".into(),
    }
}

/// Nomes em `in/`: o segundo do mesmo nome ganha `-2` antes da extensão.
pub fn input_file_names(inputs: &[(EdgeKind, Option<String>)]) -> Vec<String> {
    let mut used: HashSet<String> = HashSet::new();
    inputs
        .iter()
        .map(|(kind, file_name)| {
            let base = base_input_name(*kind, file_name.as_deref());
            let (stem, ext) = match base.rfind('.') {
                Some(i) if i > 0 => (&base[..i], &base[i..]),
                _ => (base.as_str(), ""),
            };
            let mut name = base.clone();
            let mut n = 2;
            while !used.insert(name.clone()) {
                name = format!("{stem}-{n}{ext}");
                n += 1;
            }
            name
        })
        .collect()
}

/// Uma entrada: o tipo e o arquivo da origem (ausente = valor vazio).
#[derive(Debug, Clone)]
pub struct Input {
    pub kind: EdgeKind,
    pub source: Option<PathBuf>,
}

/// Copia as entradas para `in/`. Origem sem valor vira arquivo vazio.
pub fn materialize(in_dir: &Path, inputs: &[Input]) -> std::io::Result<Vec<PathBuf>> {
    let names = input_file_names(
        &inputs
            .iter()
            .map(|i| {
                let name = i
                    .source
                    .as_deref()
                    .and_then(Path::file_name)
                    .map(|n| n.to_string_lossy().into_owned());
                (i.kind, name)
            })
            .collect::<Vec<_>>(),
    );
    let mut written = Vec::new();
    for (input, name) in inputs.iter().zip(names) {
        let dest = in_dir.join(name);
        match input.source.as_deref().filter(|p| p.is_file()) {
            Some(src) => {
                std::fs::copy(src, &dest)?;
            }
            None => std::fs::write(&dest, b"")?,
        }
        written.push(dest);
    }
    Ok(written)
}

/// `<titulo>.md` sem separador de caminho nem ponto na frente.
fn note_file_name(title: &str) -> String {
    let clean: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let clean = clean.trim().trim_start_matches('.').trim();
    if clean.is_empty() {
        "nota.md".into()
    } else {
        format!("{clean}.md")
    }
}

fn bash_program() -> PathBuf {
    let fixed = Path::new("/bin/bash");
    if cfg!(unix) && fixed.is_file() {
        fixed.to_path_buf()
    } else {
        PathBuf::from("bash")
    }
}

/// `bash -lc` com o wrapper de rules — o contrato do comando salvo, qualquer
/// que seja o `$SHELL` do dono (glob sem match e divisão de palavras do bash). `TYBA_IN`/`TYBA_OUT` (e o PATH)
/// chegam como argumentos posicionais e são exportados antes do comando —
/// assim o caminho não passa por aspas montadas à mão. O `}` vai em linha
/// própria para um comentário no fim do comando não engolir o fechamento.
///
/// `path` é o PATH do shell de login do dono ([`crate::shell_path::login_env`]):
/// o app aberto pelo Dock herda o do launchd, e o `-l` não lê o `.zshrc`
/// onde moram o Homebrew, o bun e o nvm. Exportado depois do profile, vale o
/// mesmo PATH de um terminal digitado.
pub fn process_argv(
    path: Option<&str>,
    command: &str,
    in_dir: &Path,
    out_dir: &Path,
) -> (PathBuf, Vec<String>) {
    let path = path.filter(|p| !p.is_empty());
    let (export_path, shift) = match path {
        Some(_) => (" PATH=\"$3\"", 3),
        None => ("", 2),
    };
    let script = format!(
        "export TYBA_IN=\"$1\" TYBA_OUT=\"$2\"{export_path}; shift {shift}\n\
         set -o pipefail; {{ {command}\n}} 2>&1 | tee \"$TYBA_OUT/log.txt\"; \
         echo $? > \"$TYBA_OUT/.exit\""
    );
    let mut args = vec![
        "-lc".into(),
        script,
        "tyba-maestro".into(),
        in_dir.to_string_lossy().into_owned(),
        out_dir.to_string_lossy().into_owned(),
    ];
    args.extend(path.map(str::to_string));
    (bash_program(), args)
}

/// Exit gravado pelo wrapper. Sem `.exit` o processo morreu antes do fim.
fn read_exit(out_dir: &Path) -> Option<i32> {
    std::fs::read_to_string(out_dir.join(".exit"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Onde mora a saída declarada de um processo ou integração. `diff` vem do
/// snapshot da largada; sem git não há snapshot e o destino recebe
/// `diff.patch` vazio.
fn process_outputs(
    output: OutputKind,
    file: Option<&str>,
    out_dir: &Path,
    node_dir: &Path,
    snapshot: Option<&Snapshot>,
) -> HashMap<EdgeKind, String> {
    let path = match output {
        OutputKind::Log => Some(out_dir.join("log.txt")),
        OutputKind::Json => Some(out_dir.join("out.json")),
        OutputKind::File => file.map(|f| node_dir.join(f)),
        OutputKind::Diff => snapshot.and_then(|snap| {
            let patch = out_dir.join("diff.patch");
            std::fs::write(&patch, super::agent::diff_since(snap))
                .ok()
                .map(|()| patch)
        }),
    };
    path.filter(|p| p.is_file())
        .map(|p| HashMap::from([(output.into(), p.to_string_lossy().into_owned())]))
        .unwrap_or_default()
}

/// Cria a pasta (e as que faltam no caminho) com modo 0700.
pub fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(0o700);
        builder.create(path)?;
        return std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    #[allow(unreachable_code)]
    builder.create(path)
}

/// O que o host precisa para subir o processo de um nó.
#[derive(Debug, Clone)]
pub struct ProcessLaunch {
    pub title: String,
    pub cwd: PathBuf,
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// O que o host precisa para subir o agente de um nó.
#[derive(Debug, Clone)]
pub struct AgentLaunch {
    pub title: String,
    pub cwd: PathBuf,
    pub runner: Runner,
    pub prompt: String,
    pub in_dir: PathBuf,
    pub out_dir: PathBuf,
    /// Só no nó com orquestrar ligado: as ferramentas do Maestro que o
    /// servidor MCP da sessão chama pelo socket de hooks.
    pub tools: Option<ToolHandler>,
}

/// Efeitos fora do agendador: subir processo, matar, avisar o front.
pub trait RunHost: Send + Sync + 'static {
    /// Sobe o agente livre e devolve o id da sessão. `on_turn` roda uma vez,
    /// quando o primeiro turno fecha (ou o agente sai antes disso). O agente
    /// segue vivo depois; `on_exit`, quando presente, roda quando a sessão
    /// sai depois do primeiro turno fechado.
    fn spawn_agent(
        &self,
        launch: AgentLaunch,
        on_turn: Box<dyn FnOnce(AgentTurn) + Send>,
        on_exit: Option<Box<dyn FnOnce() + Send>>,
    ) -> Result<String, String>;
    /// Sobe o processo num PTY e devolve o id da sessão. `on_exit` roda quando
    /// o PTY fecha.
    fn spawn_process(
        &self,
        launch: ProcessLaunch,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<String, String>;
    /// Mata o process group da sessão.
    fn kill(&self, session_id: &str);
    /// Portas em que a sessão escuta, do último retrato do poll de serviços
    /// (`services::CHIP_POLL`).
    fn session_ports(&self, session_id: &str) -> Vec<u16>;
    /// Acha o serviço e espera ficar pronto (bloqueia). `Err` diz por que não
    /// achou o alvo.
    fn resolve_service(&self, source: &ServiceSource) -> Result<ServiceReady, String>;
    /// A sessão ainda roda (não saiu nem foi morta).
    fn alive(&self, session_id: &str) -> bool;
    /// `maestro://run/<run_id>` com o run inteiro.
    fn emit(&self, run: &Run);
}

struct Active {
    graph: Graph,
    /// Integrações do grafo como estavam no ▶ Executar.
    integrations: HashMap<String, Integration>,
    sched: Scheduler,
    dir: PathBuf,
}

/// O que sai do lock para ser executado.
struct Start {
    node: Node,
    in_dir: PathBuf,
    out_dir: PathBuf,
    inputs: Vec<Input>,
    variables: Vec<(String, String)>,
    /// Pasta onde o nó roda (a dele ou a padrão do grafo).
    node_dir: PathBuf,
    /// A integração do nó, quando é de integração e ela existe.
    integration: Option<Integration>,
    graph_id: String,
}

enum Launched {
    Session(String),
    Finished(Outcome),
    /// Termina sozinho mais tarde, sem sessão (o serviço).
    Pending,
}

pub struct Runs<H: RunHost> {
    host: H,
    store: Arc<Store>,
    root: Option<PathBuf>,
    active: Mutex<HashMap<String, Active>>,
    /// Sessões de agente de cada run: continuam vivas depois do nó pronto, e
    /// o Parar as mata mesmo com o run já fechado. Lock sempre depois de
    /// `active` quando os dois são pegos juntos.
    agents: Mutex<HashMap<String, RunAgents>>,
    /// Runs que o dono parou: sessão que termina de subir depois disso morre
    /// na hora. Lock sempre depois de `active`.
    stopped: Mutex<HashSet<String>>,
    me: Weak<Runs<H>>,
}

struct RunAgents {
    graph_id: String,
    sessions: Vec<String>,
}

fn now() -> u64 {
    crate::approvals::now_ms()
}

/// A varredura do boot (ver [`Runs::new`]).
fn close_interrupted_runs(store: &Store) {
    let open = match store.list_running_maestro_runs() {
        Ok(runs) => runs,
        Err(e) => {
            eprintln!("[maestro] ler execuções abertas: {e}");
            return;
        }
    };
    let t = now();
    for mut run in open {
        run.close_interrupted(t);
        if let Err(e) = store.save_maestro_run(&run) {
            eprintln!("[maestro] fechar execução {}: {e}", run.id);
        }
    }
}

impl<H: RunHost> Runs<H> {
    /// Uma por app, no boot. Nenhum run nasce ativo, então todo run gravado
    /// como `running` é de um app que fechou no meio e é fechado aqui, uma vez.
    pub fn new(host: H, store: Arc<Store>, root: Option<PathBuf>) -> Arc<Self> {
        close_interrupted_runs(&store);
        Arc::new_cyclic(|me| Self {
            host,
            store,
            root,
            active: Mutex::new(HashMap::new()),
            agents: Mutex::new(HashMap::new()),
            stopped: Mutex::new(HashSet::new()),
            me: me.clone(),
        })
    }

    /// Começa o run. O grafo já passou por consentimento e validação. Antes,
    /// para os runs anteriores do mesmo grafo que ainda têm agente: um ▶ novo
    /// não deixa o claude do run de antes vivo e fora de alcance.
    pub fn start(&self, graph: Graph, integrations: Vec<Integration>) -> Result<Run, String> {
        let previous: Vec<String> = self
            .agents
            .lock()
            .iter()
            .filter(|(_, a)| a.graph_id == graph.id)
            .map(|(id, _)| id.clone())
            .collect();
        for run_id in previous {
            // Erro aqui é só "já não havia o que parar".
            let _ = self.stop(&run_id);
        }
        self.begin(graph, integrations, None)
    }

    /// "Testar com a última entrada": roda só o script da integração no lugar
    /// do nó, com uma cópia do `in/` da última execução em que ele começou.
    pub fn test_integration(
        &self,
        graph: Graph,
        integration: Integration,
        node_id: &str,
    ) -> Result<Run, String> {
        let root = self
            .root
            .clone()
            .ok_or("sem pasta home para guardar as execuções")?;
        let previous = self
            .store
            .list_maestro_runs(&graph.id, KEEP_RUNS)
            .map_err(|e| e.to_string())?;
        let seed = super::integration::last_inputs(&previous, &root, node_id).ok_or(
            "este nó ainda não rodou: execute o grafo uma vez antes de testar com a última entrada",
        )?;
        let single = super::integration::test_graph(&graph, node_id, &integration.id)?;
        self.begin(single, vec![integration], Some(seed))
    }

    /// O ▶ num nó de integração manual de um run rodando.
    pub fn run_node(&self, run_id: &str, node_id: &str) -> Result<(), String> {
        self.active
            .lock()
            .get_mut(run_id)
            .ok_or("execução não está rodando")?
            .sched
            .release(node_id)?;
        self.advance(run_id);
        Ok(())
    }

    /// `seed`: `in/` a copiar para o único nó (teste de integração). No teste
    /// o nó manual não espera ▶ — o clique em Testar já é o ▶.
    fn begin(
        &self,
        graph: Graph,
        integrations: Vec<Integration>,
        seed: Option<PathBuf>,
    ) -> Result<Run, String> {
        if let Some(bad) = graph.nodes.iter().find(|n| !safe_component(&n.id)) {
            return Err(format!(
                "id de nó inválido para execução: {:?} (use letras, números, - e _)",
                bad.id
            ));
        }
        let root = self
            .root
            .clone()
            .ok_or("sem pasta home para guardar as execuções")?;
        create_private_dir(&root).map_err(|e| e.to_string())?;
        let run_id = uuid::Uuid::new_v4().to_string();
        let dir = root.join(&run_id);
        for node in &graph.nodes {
            for sub in ["in", "out"] {
                create_private_dir(&dir.join(&node.id).join(sub)).map_err(|e| e.to_string())?;
            }
        }
        create_private_dir(&dir).map_err(|e| e.to_string())?;
        if let (Some(seed), Some(node)) = (&seed, graph.nodes.first()) {
            super::integration::copy_inputs(seed, &dir.join(&node.id).join("in"))
                .map_err(|e| format!("copiar a última entrada: {e}"))?;
        }

        let integrations: HashMap<String, Integration> = integrations
            .into_iter()
            .map(|i| (i.id.clone(), i))
            .collect();
        let mut sched = Scheduler::new(&graph, run_id.clone(), now());
        sched.run.test = seed.is_some();
        if seed.is_none() {
            for id in super::integration::manual_nodes(&graph, &integrations) {
                sched.hold(&id);
            }
        }
        let graph_id = graph.id.clone();
        let snapshot = sched.snapshot();
        if snapshot.status == RunStatus::Running {
            self.active.lock().insert(
                run_id.clone(),
                Active {
                    graph,
                    integrations,
                    sched,
                    dir,
                },
            );
        }
        self.publish(snapshot.clone());
        self.prune(&graph_id);
        self.advance(&run_id);
        Ok(self.get(&run_id).unwrap_or(snapshot))
    }

    /// Run em memória (rodando) ou do banco. Run `running` no banco sem estar
    /// ativo é o que o ■ Parar acabou de tirar da memória e cujo retrato
    /// parado ainda está sendo gravado: aparece parado. (O que o app fechado
    /// deixou aberto a varredura do boot já fechou.)
    pub fn get(&self, run_id: &str) -> Option<Run> {
        let active = self
            .active
            .lock()
            .get(run_id)
            .map(|a| a.sched.run().clone());
        let mut run = match active {
            Some(run) => run,
            None => {
                let mut run = self.store.get_maestro_run(run_id).ok().flatten()?;
                if run.status == RunStatus::Running {
                    run.status = RunStatus::Stopped;
                }
                run
            }
        };
        run.live_agents = self.live_agents(run_id);
        Some(run)
    }

    /// Sessões de agente do run que ainda rodam.
    fn live_agents(&self, run_id: &str) -> Vec<String> {
        let sessions = self
            .agents
            .lock()
            .get(run_id)
            .map(|a| a.sessions.clone())
            .unwrap_or_default();
        sessions
            .into_iter()
            .filter(|s| self.host.alive(s))
            .collect()
    }

    pub fn list(&self, graph_id: &str) -> Vec<Run> {
        let runs = self
            .store
            .list_maestro_runs(graph_id, KEEP_RUNS)
            .unwrap_or_default();
        runs.into_iter()
            .map(|r| self.get(&r.id).unwrap_or(r))
            .collect()
    }

    pub fn stop(&self, run_id: &str) -> Result<(), String> {
        // Tudo com `active` na mão: quem termina de subir uma sessão olha
        // `active` e `stopped` sob o mesmo lock (ver `advance`).
        let (stopped, agents) = {
            let mut active = self.active.lock();
            let stopped = active.remove(run_id).map(|mut a| {
                let kill = a.sched.stop(now());
                (a.sched.snapshot(), kill)
            });
            let agents = self
                .agents
                .lock()
                .remove(run_id)
                .map(|a| a.sessions)
                .unwrap_or_default();
            if stopped.is_none() && agents.is_empty() {
                return Err("execução não está rodando".into());
            }
            self.stopped.lock().insert(run_id.to_string());
            (stopped, agents)
        };
        let mut kill: Vec<String> = Vec::new();
        if let Some((snapshot, sessions)) = stopped {
            self.publish(snapshot);
            kill.extend(sessions);
        }
        kill.extend(agents);
        let mut seen = HashSet::new();
        for session in kill {
            if seen.insert(session.clone()) {
                self.host.kill(&session);
            }
        }
        Ok(())
    }

    /// Grava e avisa o front, com os agentes vivos do run.
    fn publish(&self, mut run: Run) {
        run.live_agents = self.live_agents(&run.id);
        if let Err(e) = self.store.save_maestro_run(&run) {
            eprintln!("[maestro] gravar execução {}: {e}", run.id);
        }
        self.host.emit(&run);
    }

    /// Retrato novo, gravado e avisado. Run fechado só sai da memória depois
    /// de gravado: antes disso o `get` leria do banco um retrato mais velho.
    fn publish_current(&self, run_id: &str) {
        let Some(run) = self
            .active
            .lock()
            .get_mut(run_id)
            .map(|a| a.sched.snapshot())
        else {
            return;
        };
        let closed = run.status != RunStatus::Running;
        self.publish(run);
        if closed {
            self.active.lock().remove(run_id);
        }
    }

    /// Poda o histórico do grafo. Run rodando ou com agente vivo fica, mesmo
    /// fora da janela: a pasta dele está em uso.
    fn prune(&self, graph_id: &str) {
        let Some(root) = &self.root else { return };
        let mut protect: HashSet<String> = self.active.lock().keys().cloned().collect();
        let with_agents: Vec<String> = self.agents.lock().keys().cloned().collect();
        for id in with_agents {
            if !self.live_agents(&id).is_empty() {
                protect.insert(id);
            }
        }
        let removed = match self.store.prune_maestro_runs(graph_id, KEEP_RUNS, &protect) {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!("[maestro] podar execuções: {e}");
                return;
            }
        };
        for id in removed {
            self.agents.lock().remove(&id);
            self.stopped.lock().remove(&id);
            if safe_component(&id) {
                let _ = std::fs::remove_dir_all(root.join(id));
            }
        }
    }

    /// Começa tudo que ficou pronto, até não sobrar nada pronto.
    fn advance(&self, run_id: &str) {
        loop {
            let starts = {
                let mut active = self.active.lock();
                let Some(a) = active.get_mut(run_id) else {
                    return;
                };
                let ready = a.sched.ready();
                let t = now();
                ready
                    .iter()
                    .filter_map(|id| {
                        a.sched.mark_running(id, t);
                        prepare_start(a, id)
                    })
                    .collect::<Vec<_>>()
            };
            if starts.is_empty() {
                return;
            }
            self.publish_current(run_id);
            for start in starts {
                let node_id = start.node.id.clone();
                let graph_id = start.graph_id.clone();
                let is_agent = matches!(start.node.kind, NodeKind::Agent { .. });
                let is_process = matches!(start.node.kind, NodeKind::Process { .. });
                let out_dir = start.out_dir.clone();
                let launched = self.launch(run_id, start);
                // `active` na mão para decidir: o Parar marca `stopped` sob o
                // mesmo lock, então ou ele já viu esta sessão no run, ou ela
                // morre aqui.
                let orphan = {
                    let mut active = self.active.lock();
                    match (active.get_mut(run_id), launched) {
                        (Some(a), Launched::Session(session)) => {
                            a.sched.set_session(&node_id, session.clone());
                            if is_process {
                                self.watch_ports(run_id, &node_id, &session, out_dir);
                            }
                            if is_agent {
                                self.register_agent(run_id, &graph_id, session);
                            }
                            None
                        }
                        (Some(_), Launched::Pending) => None,
                        (None, Launched::Pending) => return,
                        (Some(a), Launched::Finished(outcome)) => {
                            a.sched.finish(&node_id, outcome, now());
                            None
                        }
                        (None, Launched::Session(session)) => {
                            if self.stopped.lock().contains(run_id) {
                                Some(session)
                            } else {
                                // O run fechou sozinho enquanto subia (o agente
                                // fechou o turno antes): segue vivo e parável.
                                if is_agent {
                                    self.register_agent(run_id, &graph_id, session);
                                }
                                return;
                            }
                        }
                        (None, Launched::Finished(_)) => return,
                    }
                };
                if let Some(session) = orphan {
                    self.host.kill(&session);
                    return;
                }
                self.publish_current(run_id);
            }
        }
    }

    fn launch(&self, run_id: &str, start: Start) -> Launched {
        match &start.node.kind {
            NodeKind::Note { markdown } => {
                let path = start.out_dir.join(note_file_name(&start.node.title));
                let context = start.out_dir.join("context.md");
                let written = std::fs::write(&path, markdown).and_then(|()| {
                    std::fs::write(&context, super::agent::context_markdown(markdown))
                });
                match written {
                    Ok(()) => Launched::Finished(Outcome {
                        ok: true,
                        outputs: HashMap::from([
                            (EdgeKind::File, path.to_string_lossy().into_owned()),
                            (EdgeKind::Context, context.to_string_lossy().into_owned()),
                        ]),
                        ..Outcome::default()
                    }),
                    Err(e) => Launched::Finished(Outcome::failed(format!("gravar a nota: {e}"))),
                }
            }
            NodeKind::Integration { .. } => match &start.integration {
                Some(i) => self.launch_script(run_id, &start, &i.script, i.output),
                None => Launched::Finished(Outcome::failed("integração não encontrada")),
            },
            NodeKind::Agent {
                runner,
                prompt,
                orchestrate,
                ..
            } => self.launch_agent(run_id, &start, *runner, prompt, *orchestrate),
            NodeKind::Process {
                command, output, ..
            } => self.launch_script(run_id, &start, command, *output),
            NodeKind::Browser { url } => Launched::Finished(browser_outcome(&start, url)),
            NodeKind::Service { source } => self.launch_service(run_id, &start, source.clone()),
        }
    }

    /// O serviço em outra thread: achar o contêiner e ler o primeiro lote de
    /// log pode levar segundos, e o resto do grafo não espera.
    fn launch_service(&self, run_id: &str, start: &Start, source: ServiceSource) -> Launched {
        let me = self.me.clone();
        let (run, node, out_dir) = (
            run_id.to_string(),
            start.node.id.clone(),
            start.out_dir.clone(),
        );
        std::thread::spawn(move || {
            let Some(runs) = me.upgrade() else { return };
            let outcome = match runs.host.resolve_service(&source) {
                Ok(ready) => service_outcome(&out_dir, ready),
                Err(reason) => Outcome::failed(reason),
            };
            runs.finish_later(&run, &node, outcome);
        });
        Launched::Pending
    }

    /// Fecha um nó que terminou fora do `advance` e segue o grafo.
    fn finish_later(&self, run_id: &str, node_id: &str, outcome: Outcome) {
        {
            let mut active = self.active.lock();
            let Some(a) = active.get_mut(run_id) else {
                return;
            };
            if !a.sched.finish(node_id, outcome, now()) {
                return;
            }
        }
        self.publish_current(run_id);
        self.advance(run_id);
    }

    /// Olha as portas da sessão do processo até a primeira aparecer: aí grava
    /// `out/url.txt`, publica a url com o processo ainda rodando e segue o
    /// grafo (quem espera a url começa). Para quando o nó deixa de rodar.
    fn watch_ports(&self, run_id: &str, node_id: &str, session: &str, out_dir: PathBuf) {
        let me = self.me.clone();
        let (run, node, session) = (run_id.to_string(), node_id.to_string(), session.to_string());
        std::thread::spawn(move || loop {
            std::thread::sleep(PORT_POLL);
            let Some(runs) = me.upgrade() else { return };
            let running = runs
                .active
                .lock()
                .get(&run)
                .is_some_and(|a| a.sched.status(&node) == Some(NodeStatus::Running));
            if !running {
                return;
            }
            let Some(url) = service::first_port_url(&runs.host.session_ports(&session)) else {
                continue;
            };
            let path = out_dir.join("url.txt");
            if std::fs::write(&path, &url).is_err() {
                return;
            }
            let published = runs.active.lock().get_mut(&run).is_some_and(|a| {
                a.sched
                    .publish_url(&node, path.to_string_lossy().into_owned(), url)
            });
            if published {
                runs.publish_current(&run);
                runs.advance(&run);
            }
            return;
        });
    }

    /// Processo e integração: o texto (com variáveis) no wrapper de rules,
    /// rodado em `bash -lc` com o PATH de login do dono, num PTY, na pasta do nó.
    fn launch_script(
        &self,
        run_id: &str,
        start: &Start,
        command: &str,
        output: OutputKind,
    ) -> Launched {
        if let Err(e) = materialize(&start.in_dir, &start.inputs) {
            return Launched::Finished(Outcome::failed(format!("preparar entradas: {e}")));
        }
        let command = crate::snippet::render(command, &start.variables);
        let login_path = crate::shell_path::login_env().path;
        let (program, args) = process_argv(
            login_path.as_deref(),
            &command,
            &start.in_dir,
            &start.out_dir,
        );
        let snapshot = (output == OutputKind::Diff)
            .then(|| super::agent::snapshot(&start.node_dir))
            .flatten();
        let me = self.me.clone();
        let (run, node) = (run_id.to_string(), start.node.id.clone());
        let on_exit = Box::new(move || {
            if let Some(runs) = me.upgrade() {
                runs.process_exited(&run, &node, snapshot.as_ref());
            }
        });
        let launch = ProcessLaunch {
            title: start.node.title.clone(),
            cwd: start.node_dir.clone(),
            program,
            args,
        };
        match self.host.spawn_process(launch, on_exit) {
            Ok(session) => Launched::Session(session),
            Err(e) => Launched::Finished(Outcome::failed(format!("subir o processo: {e}"))),
        }
    }

    /// Entradas em `in/`, snapshot de diff na largada e o agente livre na
    /// pasta do nó.
    fn launch_agent(
        &self,
        run_id: &str,
        start: &Start,
        runner: Runner,
        prompt: &str,
        orchestrate: bool,
    ) -> Launched {
        let written = match materialize(&start.in_dir, &start.inputs) {
            Ok(w) => w,
            Err(e) => {
                return Launched::Finished(Outcome::failed(format!("preparar entradas: {e}")))
            }
        };
        let inputs: Vec<(EdgeKind, PathBuf)> =
            start.inputs.iter().map(|i| i.kind).zip(written).collect();
        let prompt = crate::snippet::render(prompt, &start.variables);
        // Contexto entra inteiro; arquivo de texto pequeno também (uma nota
        // ligada pela saída "arquivo" é instrução do mesmo jeito).
        let contexts: Vec<(String, String)> = inputs
            .iter()
            .filter_map(|(kind, path)| match kind {
                EdgeKind::Context => {
                    super::agent::read_context(path).map(|t| ("Contexto".to_string(), t))
                }
                EdgeKind::File => super::agent::read_text_input(path).map(|t| {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    (format!("Arquivo {name}"), t)
                }),
                _ => None,
            })
            .collect();
        let prompt = super::agent::build_prompt(&prompt, &contexts, &inputs);
        let snapshot = super::agent::snapshot(&start.node_dir);
        let me = self.me.clone();
        let (run, node, out_dir) = (
            run_id.to_string(),
            start.node.id.clone(),
            start.out_dir.clone(),
        );
        let on_turn = {
            let (me, run, node) = (me.clone(), run.clone(), node.clone());
            Box::new(move |turn: AgentTurn| {
                if let Some(runs) = me.upgrade() {
                    runs.agent_turn_ended(
                        &run,
                        &node,
                        &out_dir,
                        snapshot.as_ref(),
                        turn,
                        orchestrate,
                    );
                }
            })
        };
        // O orquestrador segura o nó (e o run) até a sessão sair.
        let on_exit = orchestrate.then(|| {
            Box::new(move || {
                if let Some(runs) = me.upgrade() {
                    runs.orchestrator_exited(&run, &node);
                }
            }) as Box<dyn FnOnce() + Send>
        });
        let tools = orchestrate.then(|| self.tool_handler(run_id, &start.node.id));
        let launch = AgentLaunch {
            title: start.node.title.clone(),
            cwd: start.node_dir.clone(),
            runner,
            prompt,
            in_dir: start.in_dir.clone(),
            out_dir: start.out_dir.clone(),
            tools,
        };
        match self.host.spawn_agent(launch, on_turn, on_exit) {
            Ok(session) => Launched::Session(session),
            Err(e) => Launched::Finished(Outcome::failed(format!("subir o agente: {e}"))),
        }
    }

    /// As ferramentas do orquestrador `node_id` neste run. Fraco: a sessão
    /// que sobrevive ao app não segura o motor.
    fn tool_handler(&self, run_id: &str, node_id: &str) -> ToolHandler {
        let me = self.me.clone();
        let (run, node) = (run_id.to_string(), node_id.to_string());
        ToolHandler(Arc::new(move |tool, args| {
            let runs = me.upgrade().ok_or("o TYBA está fechando")?;
            runs.orchestrator_call(&run, &node, tool, args)
        }))
    }

    /// Uma ferramenta do Maestro pedida pela sessão do orquestrador
    /// `orchestrator` do run `run_id`.
    pub fn orchestrator_call(
        &self,
        run_id: &str,
        orchestrator: &str,
        tool: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let call = orchestrator::parse_call(tool, args)?;
        eprintln!("[maestro] orquestrador {orchestrator}: {tool}");
        match call {
            ToolCall::Create {
                title,
                kind,
                from_node,
            } => {
                let id = self.create_node(run_id, orchestrator, title, kind, from_node)?;
                Ok(serde_json::json!({ "node_id": id }))
            }
            ToolCall::Status { node_id } => self.node_status(run_id, &node_id),
            ToolCall::Stop { node_id } => {
                self.stop_created(run_id, orchestrator, &node_id)?;
                Ok(serde_json::json!({ "node_id": node_id, "stopped": true }))
            }
        }
    }

    /// Nó novo no run ativo, começando na hora (ou assim que a url do
    /// `from_node` sair). Teto de [`MAX_CREATED`] por run.
    pub fn create_node(
        &self,
        run_id: &str,
        orchestrator: &str,
        title: String,
        kind: NodeKind,
        from_node: Option<String>,
    ) -> Result<String, String> {
        let root = self.root.clone().ok_or("sem pasta para as execuções")?;
        let id = format!("orq-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        {
            let mut active = self.active.lock();
            let a = active.get_mut(run_id).ok_or(
                "esta execução já terminou: ▶ Executar de novo para o orquestrador criar nós",
            )?;
            if a.sched.run.created.len() >= MAX_CREATED {
                return Err(format!(
                    "teto de {MAX_CREATED} nós criados por execução atingido"
                ));
            }
            let mut edges = Vec::new();
            if let Some(from) = &from_node {
                let source = a
                    .graph
                    .nodes
                    .iter()
                    .find(|n| &n.id == from)
                    .ok_or_else(|| format!("from_node não existe neste run: {from}"))?;
                let by_id: HashMap<&str, &Integration> = a
                    .integrations
                    .iter()
                    .map(|(k, v)| (k.as_str(), v))
                    .collect();
                if !super::produces(&source.kind, &by_id).contains(&EdgeKind::Url) {
                    return Err(format!(
                        "from_node {from} não produz url (use um nó de processo ou serviço)"
                    ));
                }
                edges.push(Edge {
                    id: format!("{id}-url"),
                    from: from.clone(),
                    to: id.clone(),
                    kind: EdgeKind::Url,
                });
            }
            let node = {
                let siblings: Vec<&Node> = a
                    .sched
                    .run
                    .created
                    .iter()
                    .filter(|c| c.created_by == orchestrator)
                    .map(|c| &c.node)
                    .collect();
                let parent = a.graph.nodes.iter().find(|n| n.id == orchestrator);
                orchestrator::new_node(id.clone(), title, kind, parent, &siblings)
            };
            for sub in ["in", "out"] {
                create_private_dir(&root.join(run_id).join(&id).join(sub))
                    .map_err(|e| e.to_string())?;
            }
            let sources: Vec<(String, EdgeKind)> =
                edges.iter().map(|e| (e.from.clone(), e.kind)).collect();
            a.sched.add_node(&id, &sources)?;
            a.graph.nodes.push(node.clone());
            a.graph.edges.extend(edges.iter().cloned());
            a.sched.run.created.push(Created {
                node,
                created_by: orchestrator.to_string(),
                created_at: now(),
                edges,
                kept: false,
            });
        }
        self.publish_current(run_id);
        self.advance(run_id);
        Ok(id)
    }

    /// Estado de um nó do run, url/porta, as últimas linhas do log redigidas
    /// e, quando o nó tem, o resultado: o texto do turno (`context.md`, já
    /// redigido) e o resumo do diff — o agente criado não grava `log.txt`, e
    /// sem isto o orquestrador via `done` sem saber o que ele disse nem o que
    /// mudou. `paths` leva os caminhos absolutos para ler o resto.
    pub fn node_status(&self, run_id: &str, node_id: &str) -> Result<serde_json::Value, String> {
        let run = self.get(run_id).ok_or("execução não encontrada")?;
        let n = run
            .nodes
            .get(node_id)
            .ok_or_else(|| format!("nó não existe neste run: {node_id}"))?;
        let log_path = self
            .root
            .as_ref()
            .map(|root| root.join(run_id).join(node_id).join("out").join("log.txt"));
        let log = log_path
            .as_deref()
            .and_then(|p| orchestrator::log_tail(p, orchestrator::STATUS_LOG_LINES));
        let turn = n
            .outputs
            .get(&EdgeKind::Context)
            .and_then(|p| orchestrator::turn_text(Path::new(p)));
        let diff = n
            .outputs
            .get(&EdgeKind::Diff)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|patch| orchestrator::diff_summary(&patch));
        let mut paths: serde_json::Map<String, serde_json::Value> = n
            .outputs
            .iter()
            .filter_map(|(kind, path)| {
                let key = serde_json::to_value(kind).ok()?.as_str()?.to_string();
                Some((key, path.clone().into()))
            })
            .collect();
        if let Some(p) = log_path.filter(|p| p.is_file()) {
            paths
                .entry("log")
                .or_insert_with(|| p.to_string_lossy().into_owned().into());
        }
        let port = n.url.as_deref().and_then(|u| {
            let authority = u.split("://").nth(1)?.split(['/', '?', '#']).next()?;
            authority.rsplit_once(':')?.1.parse::<u16>().ok()
        });
        Ok(serde_json::json!({
            "node_id": node_id,
            "status": n.status,
            "url": n.url,
            "port": port,
            "exit_code": n.exit_code,
            "reason": n.reason,
            "log": log,
            "text": turn.as_ref().map(|(t, _)| t),
            "text_truncated": turn.as_ref().is_some_and(|(_, cut)| *cut),
            "diff": diff,
            "paths": paths,
        }))
    }

    /// `maestro_stop_node`: só nó que este orquestrador criou. Mata o process
    /// group dele (e o agente criado que segue vivo depois do turno).
    pub fn stop_created(
        &self,
        run_id: &str,
        orchestrator: &str,
        node_id: &str,
    ) -> Result<(), String> {
        let kill = {
            let mut active = self.active.lock();
            let a = active.get_mut(run_id).ok_or("execução não está rodando")?;
            if !a
                .sched
                .run
                .created
                .iter()
                .any(|c| c.node.id == node_id && c.created_by == orchestrator)
            {
                return Err(format!(
                    "só para nó criado por este orquestrador: {node_id} não é um deles"
                ));
            }
            let alive_agent = a
                .sched
                .run
                .nodes
                .get(node_id)
                .and_then(|n| n.session_id.clone());
            a.sched.stop_node(node_id, now()).or(alive_agent)
        };
        self.publish_current(run_id);
        if let Some(session) = kill {
            self.host.kill(&session);
        }
        Ok(())
    }

    /// Os nós criados ainda não mantidos do run ATIVO (o "Manter no grafo").
    /// Depois do ■ Parar ou do fim do run eles já foram descartados.
    pub fn created_to_keep(&self, run_id: &str) -> Result<Vec<Created>, String> {
        let active = self.active.lock();
        let a = active
            .get(run_id)
            .ok_or("a execução já terminou: os nós criados foram descartados")?;
        Ok(a.sched
            .run
            .created
            .iter()
            .filter(|c| !c.kept)
            .cloned()
            .collect())
    }

    /// Marca como mantidos (já gravados no grafo) e avisa o front.
    pub fn mark_kept(&self, run_id: &str, ids: &[String]) {
        {
            let mut active = self.active.lock();
            let Some(a) = active.get_mut(run_id) else {
                return;
            };
            for c in &mut a.sched.run.created {
                if ids.contains(&c.node.id) {
                    c.kept = true;
                }
            }
        }
        self.publish_current(run_id);
    }

    /// Guarda a sessão de agente para o Parar. Chamado com `active` na mão.
    fn register_agent(&self, run_id: &str, graph_id: &str, session: String) {
        self.agents
            .lock()
            .entry(run_id.to_string())
            .or_insert_with(|| RunAgents {
                graph_id: graph_id.to_string(),
                sessions: Vec::new(),
            })
            .sessions
            .push(session);
    }

    /// O primeiro turno fechou: grava `context.md` (redigido) e `diff.patch`
    /// e segue o grafo. O agente continua vivo. O orquestrador publica as
    /// saídas mas segue rodando até a sessão sair.
    fn agent_turn_ended(
        &self,
        run_id: &str,
        node_id: &str,
        out_dir: &Path,
        snapshot: Option<&super::agent::Snapshot>,
        turn: AgentTurn,
        orchestrate: bool,
    ) {
        if !self.active.lock().contains_key(run_id) {
            return;
        }
        let outcome = match turn {
            AgentTurn::Exited => Outcome::failed("o agente saiu antes de fechar o turno"),
            AgentTurn::Failed(reason) => Outcome::failed(reason),
            AgentTurn::Done { text } => agent_outputs(out_dir, snapshot, text.as_deref()),
        };
        {
            let mut active = self.active.lock();
            let Some(a) = active.get_mut(run_id) else {
                return;
            };
            let changed = if orchestrate && outcome.ok {
                a.sched.orchestrate(node_id, outcome.outputs)
            } else {
                a.sched.finish(node_id, outcome, now())
            };
            if !changed {
                return;
            }
        }
        self.publish_current(run_id);
        self.advance(run_id);
    }

    /// A sessão do orquestrador saiu depois do primeiro turno: o nó fica
    /// pronto (com as saídas do turno) e o run fecha se nada mais roda.
    fn orchestrator_exited(&self, run_id: &str, node_id: &str) {
        {
            let mut active = self.active.lock();
            let Some(a) = active.get_mut(run_id) else {
                return;
            };
            if !a.sched.orchestrating(node_id) {
                return;
            }
            let ok = Outcome {
                ok: true,
                ..Outcome::default()
            };
            a.sched.finish(node_id, ok, now());
        }
        self.publish_current(run_id);
        self.advance(run_id);
    }

    /// O PTY do processo fechou: lê o `.exit` e segue o grafo. O diff (git)
    /// sai fora do lock.
    fn process_exited(&self, run_id: &str, node_id: &str, snapshot: Option<&Snapshot>) {
        let (out_dir, node_dir, declared) = {
            let active = self.active.lock();
            let Some(a) = active.get(run_id) else {
                return;
            };
            let Some(node) = a.graph.nodes.iter().find(|n| n.id == node_id) else {
                return;
            };
            let declared = match &node.kind {
                NodeKind::Process { output, file, .. } => Some((*output, file.clone())),
                NodeKind::Integration { integration_id, .. } => a
                    .integrations
                    .get(integration_id)
                    .map(|i| (i.output, i.file.clone())),
                _ => None,
            };
            (
                a.dir.join(node_id).join("out"),
                PathBuf::from(a.graph.node_dir(node)),
                declared,
            )
        };
        let exit = read_exit(&out_dir);
        let outputs = match declared {
            Some((output, file)) => {
                process_outputs(output, file.as_deref(), &out_dir, &node_dir, snapshot)
            }
            None => HashMap::new(),
        };
        {
            let mut active = self.active.lock();
            let Some(a) = active.get_mut(run_id) else {
                return;
            };
            let outcome = Outcome {
                ok: exit == Some(0),
                exit_code: exit,
                outputs,
                reason: exit
                    .is_none()
                    .then(|| "o processo terminou sem exit code".into()),
                url: None,
            };
            if !a.sched.finish(node_id, outcome, now()) {
                return;
            }
        }
        self.publish_current(run_id);
        self.advance(run_id);
    }
}

/// De quanto em quanto o processo olha se a sessão abriu porta. O retrato das
/// portas é o do poll de serviços (`services::CHIP_POLL`); isto só lê dele.
const PORT_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// Navegador: a url que chegou pela ligação, ou a dele. Só local passa — de
/// novo aqui, porque a que chega por ligação nunca passou pelo salvar.
fn browser_outcome(start: &Start, own: &str) -> Outcome {
    let received = start
        .inputs
        .iter()
        .filter(|i| i.kind == EdgeKind::Url)
        .filter_map(|i| i.source.as_deref())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .map(|t| t.trim().to_string())
        .find(|t| !t.is_empty());
    let url = received.unwrap_or_else(|| own.trim().to_string());
    if url.is_empty() {
        return Outcome::failed("sem endereço: ligue uma url ou digite uma no nó");
    }
    if !service::is_local_url(&url) {
        return Outcome::failed(format!(
            "endereço recusado: só local (http/https em {})",
            service::LOCAL_HOSTS
        ));
    }
    Outcome {
        ok: true,
        url: Some(url),
        ..Outcome::default()
    }
}

/// Serviço pronto: `url.txt` quando há url, `log.txt` quando há log.
fn service_outcome(out_dir: &Path, ready: ServiceReady) -> Outcome {
    let mut outputs = HashMap::new();
    if let Some(url) = &ready.url {
        if !service::is_local_url(url) {
            return Outcome::failed("endereço do serviço não é local");
        }
        let path = out_dir.join("url.txt");
        if let Err(e) = std::fs::write(&path, url) {
            return Outcome::failed(format!("gravar a url: {e}"));
        }
        outputs.insert(EdgeKind::Url, path.to_string_lossy().into_owned());
    }
    if let Some(log) = &ready.log {
        let path = out_dir.join("log.txt");
        if let Err(e) = std::fs::write(&path, log) {
            return Outcome::failed(format!("gravar o log: {e}"));
        }
        outputs.insert(EdgeKind::Log, path.to_string_lossy().into_owned());
    }
    Outcome {
        ok: true,
        outputs,
        url: ready.url,
        ..Outcome::default()
    }
}

/// Saídas do agente pronto: `context.md` sempre (vazio sem fala) e
/// `diff.patch` só com snapshot (sem git, o destino recebe diff vazio).
fn agent_outputs(
    out_dir: &Path,
    snapshot: Option<&super::agent::Snapshot>,
    text: Option<&str>,
) -> Outcome {
    let mut outputs = HashMap::new();
    let context = out_dir.join("context.md");
    let body = super::agent::context_markdown(text.unwrap_or_default());
    if let Err(e) = std::fs::write(&context, &body) {
        return Outcome::failed(format!("gravar o contexto: {e}"));
    }
    outputs.insert(EdgeKind::Context, context.to_string_lossy().into_owned());
    if let Some(snap) = snapshot {
        let patch = out_dir.join("diff.patch");
        let diff = super::agent::diff_since(snap);
        if let Err(e) = std::fs::write(&patch, &diff) {
            return Outcome::failed(format!("gravar o diff: {e}"));
        }
        outputs.insert(EdgeKind::Diff, patch.to_string_lossy().into_owned());
    }
    eprintln!(
        "[maestro] agente pronto: contexto {} bytes, diff {}",
        body.len(),
        if snapshot.is_some() {
            "gravado"
        } else {
            "sem git"
        }
    );
    Outcome {
        ok: true,
        outputs,
        ..Outcome::default()
    }
}

/// Monta o que o nó precisa (entradas, pastas, variáveis) com o lock na mão.
fn prepare_start(a: &Active, id: &str) -> Option<Start> {
    let node = a.graph.nodes.iter().find(|n| n.id == id)?.clone();
    let integration = match &node.kind {
        NodeKind::Integration { integration_id, .. } => a.integrations.get(integration_id).cloned(),
        _ => None,
    };
    let inputs = a
        .graph
        .edges
        .iter()
        .filter(|e| e.to == id)
        .map(|e| Input {
            kind: e.kind,
            source: a
                .sched
                .run()
                .nodes
                .get(&e.from)
                .and_then(|n| n.outputs.get(&e.kind))
                .map(PathBuf::from),
        })
        .collect();
    let run_dir = a.dir.join(id);
    let node_dir = PathBuf::from(a.graph.node_dir(&node));
    Some(Start {
        node,
        in_dir: run_dir.join("in"),
        out_dir: run_dir.join("out"),
        inputs,
        variables: a
            .graph
            .variables
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        node_dir,
        integration,
        graph_id: a.graph.id.clone(),
    })
}

/// Pasta das execuções no perfil: `~/.tyba/maestro/runs` (dev: `~/.tyba-dev`).
pub fn default_root() -> Option<PathBuf> {
    crate::session::cwd::home().map(|h| {
        h.join(crate::profile::HOME_DIR)
            .join("maestro")
            .join("runs")
    })
}

/// Host do app: sessão de comando pelo `SessionManager`, fora de qualquer
/// layout (não aparece na barra lateral).
pub struct TauriHost {
    app: tauri::AppHandle,
}

impl TauriHost {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

pub type SharedRuns = Arc<Runs<TauriHost>>;

impl RunHost for TauriHost {
    fn spawn_process(
        &self,
        launch: ProcessLaunch,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<String, String> {
        use tauri::Manager;
        let state = self.app.state::<crate::AppState>();
        let handle = self.app.clone();
        let session = state
            .sessions
            .create_command_session(
                self.app.clone(),
                &state.pty_pool,
                &launch.program,
                &launch.args,
                launch.title,
                Some(&launch.cwd),
                100,
                30,
                crate::session::SessionKind::Shell,
                // Sessão de nó é do run, não da grade: nunca vai ao banco.
                crate::session::SessionOwner::Maestro,
                move |id| {
                    let state = handle.state::<crate::AppState>();
                    state.sessions.set_status(
                        &handle,
                        id,
                        crate::session::SessionStatus::Exited { code: -1 },
                    );
                    on_exit();
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(session.id.to_string())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn spawn_agent(
        &self,
        launch: AgentLaunch,
        on_turn: Box<dyn FnOnce(AgentTurn) + Send>,
        on_exit: Option<Box<dyn FnOnce() + Send>>,
    ) -> Result<String, String> {
        use tauri::Manager;
        let state = self.app.state::<crate::AppState>();
        let ctx = crate::agent::session::AgentSessionCtx {
            app: self.app.clone(),
            sessions: Arc::clone(&state.sessions),
            pty_pool: Arc::clone(&state.pty_pool),
            approvals: Arc::clone(&state.approvals),
            store: Arc::clone(&state.store),
            servers: Arc::clone(&state.hook_servers),
            subagents: Arc::clone(&state.subagents),
        };
        let spec = crate::agent::session::FreeAgentSpec {
            runner: super::agent::runner_kind(launch.runner),
            cwd: launch.cwd,
            title: launch.title,
            prompt: launch.prompt,
            extra_env: vec![
                (
                    "TYBA_IN".into(),
                    launch.in_dir.to_string_lossy().into_owned(),
                ),
                (
                    "TYBA_OUT".into(),
                    launch.out_dir.to_string_lossy().into_owned(),
                ),
            ],
            cols: 100,
            rows: 30,
            maestro_tools: launch.tools,
        };
        let handle = self.app.clone();
        let (session, tail) = crate::agent::session::spawn_free_agent(&ctx, spec, move |id| {
            let state = handle.state::<crate::AppState>();
            state.sessions.set_status(
                &handle,
                id,
                crate::session::SessionStatus::Exited { code: -1 },
            );
        })?;
        let sessions = Arc::clone(&state.sessions);
        let id = session.id;
        super::agent::watch_first_turn(
            move || sessions.get(id).map(|s| s.status),
            tail,
            on_turn,
            on_exit,
        );
        Ok(session.id.to_string())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn spawn_agent(
        &self,
        _launch: AgentLaunch,
        _on_turn: Box<dyn FnOnce(AgentTurn) + Send>,
        _on_exit: Option<Box<dyn FnOnce() + Send>>,
    ) -> Result<String, String> {
        Err("o nó de agente ainda não roda neste sistema".into())
    }

    fn kill(&self, session_id: &str) {
        use tauri::Manager;
        let Ok(id) = session_id.parse() else { return };
        let state = self.app.state::<crate::AppState>();
        state.sessions.dispose(&state.pty_pool, id);
    }

    fn alive(&self, session_id: &str) -> bool {
        use tauri::Manager;
        let Ok(id) = session_id.parse() else {
            return false;
        };
        let state = self.app.state::<crate::AppState>();
        state.sessions.get(id).is_some_and(|s| {
            !matches!(
                s.status,
                crate::session::SessionStatus::Exited { .. }
                    | crate::session::SessionStatus::Failed { .. }
            )
        })
    }

    fn session_ports(&self, session_id: &str) -> Vec<u16> {
        use tauri::Manager;
        let Ok(id) = session_id.parse() else {
            return Vec::new();
        };
        let state = self.app.state::<crate::AppState>();
        state.services.chip().remove(&id).unwrap_or_default()
    }

    fn resolve_service(&self, source: &ServiceSource) -> Result<ServiceReady, String> {
        service::resolve(&self.app, source)
    }

    fn emit(&self, run: &Run) {
        use tauri::Emitter;
        let _ = self.app.emit(&format!("maestro://run/{}", run.id), run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maestro::{Edge, Position, Runner, Size, Trigger};
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    fn node(id: &str, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            title: id.into(),
            kind,
            position: Position { x: 0.0, y: 0.0 },
            size: Size { w: 320.0, h: 200.0 },
        }
    }

    fn process(id: &str, command: &str) -> Node {
        node(
            id,
            NodeKind::Process {
                command: command.into(),
                output: OutputKind::Log,
                file: None,
                dir: String::new(),
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
            dir: std::env::temp_dir().to_string_lossy().into_owned(),
            nodes,
            edges,
            variables: BTreeMap::new(),
        }
    }

    fn ok() -> Outcome {
        Outcome {
            ok: true,
            exit_code: Some(0),
            ..Outcome::default()
        }
    }

    fn status(s: &Scheduler, id: &str) -> NodeStatus {
        s.run().nodes[id].status
    }

    // a ─┐
    //    ├─ c ── d
    // b ─┘
    fn diamond() -> Graph {
        graph(
            vec![
                process("a", "x"),
                process("b", "x"),
                process("c", "x"),
                process("d", "x"),
            ],
            vec![
                edge("e1", "a", "c", EdgeKind::Log),
                edge("e2", "b", "c", EdgeKind::Log),
                edge("e3", "c", "d", EdgeKind::Log),
            ],
        )
    }

    fn browser(id: &str, url: &str) -> Node {
        node(id, NodeKind::Browser { url: url.into() })
    }

    #[test]
    fn url_edge_has_value_while_the_process_still_runs() {
        let g = graph(
            vec![
                process("p", "bun run dev"),
                browser("b", ""),
                process("q", "x"),
            ],
            vec![
                edge("e1", "p", "b", EdgeKind::Url),
                edge("e2", "p", "q", EdgeKind::Log),
            ],
        );
        let mut s = Scheduler::new(&g, "r".into(), 0);
        assert_eq!(s.ready(), vec!["p".to_string()]);
        s.mark_running("p", 1);
        assert!(s.ready().is_empty(), "sem porta ainda, a url não tem valor");
        assert!(s.publish_url("p", "/out/url.txt".into(), "http://localhost:5173".into()));
        // A url vale com o processo rodando; o log só no fim.
        assert_eq!(s.ready(), vec!["b".to_string()]);
        assert_eq!(
            s.run().nodes["p"].url.as_deref(),
            Some("http://localhost:5173")
        );
        // A primeira url fica.
        assert!(!s.publish_url("p", "/x".into(), "http://localhost:1".into()));
        s.mark_running("b", 2);
        s.finish("p", ok(), 3);
        // O fim do processo não apaga a url publicada antes.
        assert_eq!(s.run().nodes["p"].outputs[&EdgeKind::Url], "/out/url.txt");
        assert_eq!(s.ready(), vec!["q".to_string()]);
    }

    #[test]
    fn browser_takes_the_received_url_or_its_own_and_only_local() {
        let dir = tempfile::tempdir().unwrap();
        let received = dir.path().join("url.txt");
        let start = |own: &str, source: Option<PathBuf>| {
            let node = browser("b", own);
            browser_outcome(
                &Start {
                    node,
                    in_dir: dir.path().join("in"),
                    out_dir: dir.path().join("out"),
                    inputs: vec![Input {
                        kind: EdgeKind::Url,
                        source,
                    }],
                    variables: Vec::new(),
                    node_dir: dir.path().to_path_buf(),
                    integration: None,
                    graph_id: "g".into(),
                },
                own,
            )
        };
        std::fs::write(&received, "http://localhost:5173/\n").unwrap();
        let o = start("http://127.0.0.1:9", Some(received.clone()));
        assert!(o.ok);
        assert_eq!(o.url.as_deref(), Some("http://localhost:5173/"));
        // Sem valor na ligação, vale a do nó.
        let o = start("http://127.0.0.1:9", None);
        assert_eq!(o.url.as_deref(), Some("http://127.0.0.1:9"));
        // Nada em lugar nenhum: falha com motivo.
        assert!(!start("", None).ok);
        // Recebida não local é recusada na execução.
        std::fs::write(&received, "https://example.com").unwrap();
        let o = start("", Some(received));
        assert!(!o.ok);
        assert!(o.reason.unwrap().contains("só local"));
    }

    #[test]
    fn scheduler_runs_in_topological_order_and_parallel() {
        let mut s = Scheduler::new(&diamond(), "r".into(), 0);
        // Independentes começam juntos.
        assert_eq!(s.ready(), vec!["a".to_string(), "b".to_string()]);
        s.mark_running("a", 1);
        s.mark_running("b", 1);
        assert!(s.ready().is_empty());

        // c espera TODAS as entradas.
        s.finish("a", ok(), 2);
        assert!(s.ready().is_empty(), "c não pode começar só com a");
        s.finish("b", ok(), 3);
        assert_eq!(s.ready(), vec!["c".to_string()]);
        s.mark_running("c", 3);
        s.finish("c", ok(), 4);
        assert_eq!(s.ready(), vec!["d".to_string()]);
        s.mark_running("d", 4);
        s.finish("d", ok(), 5);
        assert_eq!(s.run().status, RunStatus::Done);
        assert_eq!(s.run().finished_at, Some(5));
    }

    #[test]
    fn failure_skips_dependents_and_the_rest_continues() {
        let mut s = Scheduler::new(&diamond(), "r".into(), 0);
        s.mark_running("a", 1);
        s.mark_running("b", 1);
        s.finish("a", Outcome::failed("exit 1"), 2);
        assert_eq!(status(&s, "c"), NodeStatus::Skipped);
        assert_eq!(status(&s, "d"), NodeStatus::Skipped);
        // b segue rodando; o run só fecha quando ele termina.
        assert_eq!(s.run().status, RunStatus::Running);
        s.finish("b", ok(), 3);
        assert_eq!(status(&s, "b"), NodeStatus::Done);
        assert_eq!(s.run().status, RunStatus::Failed);
    }

    #[test]
    fn stop_marks_running_stopped_and_returns_sessions() {
        let mut s = Scheduler::new(&diamond(), "r".into(), 0);
        s.mark_running("a", 1);
        s.mark_running("b", 1);
        s.set_session("a", "sess-a".into());
        s.finish("b", ok(), 2);
        let kill = s.stop(3);
        assert_eq!(kill, vec!["sess-a".to_string()]);
        assert_eq!(status(&s, "a"), NodeStatus::Stopped);
        assert_eq!(status(&s, "b"), NodeStatus::Done);
        assert_eq!(status(&s, "c"), NodeStatus::Skipped);
        assert_eq!(s.run().status, RunStatus::Stopped);
        // O exit do processo morto chega depois e não muda nada.
        assert!(!s.finish("a", ok(), 4));
        assert!(s.ready().is_empty());
    }

    #[test]
    fn two_diff_inputs_get_suffix() {
        let names = input_file_names(&[
            (EdgeKind::Diff, None),
            (EdgeKind::Context, None),
            (EdgeKind::Diff, None),
            (EdgeKind::File, Some("plano.md".into())),
            (EdgeKind::File, Some("plano.md".into())),
        ]);
        assert_eq!(
            names,
            vec![
                "diff.patch",
                "context.md",
                "diff-2.patch",
                "plano.md",
                "plano-2.md"
            ]
        );
    }

    #[test]
    fn materialize_writes_files_and_empty_for_missing() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.patch");
        std::fs::write(&src, "diff --git a/x b/x\n").unwrap();
        let in_dir = dir.path().join("in");
        std::fs::create_dir(&in_dir).unwrap();
        materialize(
            &in_dir,
            &[
                Input {
                    kind: EdgeKind::Diff,
                    source: Some(src),
                },
                Input {
                    kind: EdgeKind::Diff,
                    source: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(in_dir.join("diff.patch")).unwrap(),
            "diff --git a/x b/x\n"
        );
        assert_eq!(
            std::fs::read_to_string(in_dir.join("diff-2.patch")).unwrap(),
            ""
        );
    }

    #[test]
    fn unconsented_graph_is_refused() {
        let g = graph(vec![note("n")], vec![]);
        let stored = Stored {
            value: g.clone(),
            consented: false,
            imported_hash: Some("h".into()),
        };
        assert!(startable(stored).is_err());
        let stored = Stored {
            value: g.clone(),
            consented: true,
            imported_hash: None,
        };
        assert_eq!(startable(stored), Ok(g));
    }

    #[test]
    fn note_title_becomes_safe_file_name() {
        assert_eq!(note_file_name("Plano"), "Plano.md");
        assert_eq!(note_file_name("../../etc/passwd"), "_.._etc_passwd.md");
        assert_eq!(note_file_name("  "), "nota.md");
    }

    /// Host de teste: sobe o processo num PTY de verdade pelo `PtyPool`, sem
    /// `SessionManager` (que é tipado no runtime do app, não no mock).
    struct PtyHost {
        app: tauri::AppHandle<tauri::test::MockRuntime>,
        pool: crate::pty::PtyPool,
        runs: Mutex<Vec<Run>>,
        /// Agentes de mentira: o que cada um recebeu e quem foi morto.
        agents: Mutex<Vec<AgentLaunch>>,
        killed: Mutex<Vec<String>>,
        /// Sessões que o `spawn_agent` devolveu.
        spawned: Mutex<Vec<String>>,
        /// Quando presente, o próximo `spawn_agent` avisa que entrou e espera
        /// a liberação antes de devolver — simula o spawn lento.
        hold_spawn: Mutex<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>>,
        /// Portas que toda sessão "abriu" (o poll de serviços de mentira).
        ports: Mutex<Vec<u16>>,
        /// O que o `resolve_service` devolve.
        service: Mutex<Result<ServiceReady, String>>,
        /// O `on_exit` de cada sessão de agente que o pediu (orquestrador).
        exits: Mutex<HashMap<String, Box<dyn FnOnce() + Send>>>,
        /// Sessões de agente que saíram sozinhas.
        gone: Mutex<Vec<String>>,
    }

    impl PtyHost {
        /// A sessão do agente sai (o dono fechou o claude): o mesmo aviso
        /// que a sessão real dá ao cair.
        fn exit_agent(&self, session: &str) {
            self.gone.lock().push(session.to_string());
            let exit = self.exits.lock().remove(session);
            if let Some(exit) = exit {
                exit();
            }
        }
    }

    /// Fixture sintética: chave no formato `sk-` que a redação pega.
    fn fake_key() -> String {
        format!("sk-{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6")
    }

    impl RunHost for Arc<PtyHost> {
        /// Sem CLI de verdade: o "agente" cria um arquivo na pasta e fecha o
        /// turno com uma fala que carrega um segredo.
        fn spawn_agent(
            &self,
            launch: AgentLaunch,
            on_turn: Box<dyn FnOnce(AgentTurn) + Send>,
            on_exit: Option<Box<dyn FnOnce() + Send>>,
        ) -> Result<String, String> {
            self.agents.lock().push(launch.clone());
            let hold = self.hold_spawn.lock().take();
            if let Some((entered, release)) = hold {
                entered.send(()).unwrap();
                release.recv().unwrap();
            }
            std::thread::spawn(move || {
                std::fs::write(launch.cwd.join(format!("{}.txt", launch.title)), "feito\n")
                    .unwrap();
                on_turn(AgentTurn::Done {
                    text: Some(format!("pronto, usei {}", fake_key())),
                });
            });
            let session = format!("agente-{}", uuid::Uuid::new_v4());
            if let Some(exit) = on_exit {
                self.exits.lock().insert(session.clone(), exit);
            }
            self.spawned.lock().push(session.clone());
            Ok(session)
        }

        fn spawn_process(
            &self,
            launch: ProcessLaunch,
            on_exit: Box<dyn FnOnce() + Send>,
        ) -> Result<String, String> {
            let mut cmd = portable_pty::CommandBuilder::new(&launch.program);
            cmd.args(&launch.args);
            cmd.cwd(&launch.cwd);
            let id = uuid::Uuid::new_v4();
            self.pool
                .spawn(
                    self.app.clone(),
                    id,
                    cmd,
                    None,
                    None,
                    80,
                    24,
                    &crate::session::SessionKind::Shell,
                    on_exit,
                )
                .map_err(|e| e.to_string())?;
            Ok(id.to_string())
        }

        fn kill(&self, session_id: &str) {
            self.killed.lock().push(session_id.to_string());
            if let Ok(id) = session_id.parse() {
                let _ = self.pool.kill(id);
            }
            // A sessão morta sai como a real: o orquestrador avisa.
            let exit = self.exits.lock().remove(session_id);
            if let Some(exit) = exit {
                exit();
            }
        }

        fn alive(&self, session_id: &str) -> bool {
            !self.killed.lock().iter().any(|k| k == session_id)
                && !self.gone.lock().iter().any(|k| k == session_id)
        }

        fn session_ports(&self, _session_id: &str) -> Vec<u16> {
            self.ports.lock().clone()
        }

        fn resolve_service(&self, _source: &ServiceSource) -> Result<ServiceReady, String> {
            self.service.lock().clone()
        }

        fn emit(&self, run: &Run) {
            self.runs.lock().push(run.clone());
        }
    }

    type TestRuns = (
        Arc<Runs<Arc<PtyHost>>>,
        Arc<PtyHost>,
        tauri::App<tauri::test::MockRuntime>,
    );

    /// O `App` volta junto para viver até o fim do teste.
    fn pty_runs(root: PathBuf) -> TestRuns {
        pty_runs_on(root, Arc::new(Store::open_in_memory().unwrap()))
    }

    /// Como [`pty_runs`], sobre um banco que já tem dados (o boot).
    fn pty_runs_on(root: PathBuf, store: Arc<Store>) -> TestRuns {
        let app = tauri::test::mock_app();
        let host = Arc::new(PtyHost {
            app: app.handle().clone(),
            pool: crate::pty::PtyPool::new(),
            runs: Mutex::new(Vec::new()),
            agents: Mutex::new(Vec::new()),
            killed: Mutex::new(Vec::new()),
            spawned: Mutex::new(Vec::new()),
            hold_spawn: Mutex::new(None),
            ports: Mutex::new(Vec::new()),
            service: Mutex::new(Err("sem serviço".into())),
            exits: Mutex::new(HashMap::new()),
            gone: Mutex::new(Vec::new()),
        });
        (Runs::new(Arc::clone(&host), store, Some(root)), host, app)
    }

    fn wait_finished(runs: &Runs<Arc<PtyHost>>, id: &str) -> Run {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let run = runs.get(id).unwrap();
            if run.status != RunStatus::Running || Instant::now() > deadline {
                return run;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn process_with_real_pty_writes_log_and_exit() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let mut g = graph(
            vec![
                process("p", "echo {{palavra}}"),
                process("q", "cat \"$TYBA_IN/log.txt\""),
            ],
            vec![edge("e1", "p", "q", EdgeKind::Log)],
        );
        g.variables.insert("palavra".into(), "oi".into());
        let run = runs.start(g, vec![]).unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Done, "{run:?}");
        let p = &run.nodes["p"];
        assert_eq!(p.exit_code, Some(0));
        assert!(p.session_id.is_some());
        let log = std::fs::read_to_string(&p.outputs[&EdgeKind::Log]).unwrap();
        assert!(log.contains("oi"), "log: {log:?}");
        // A entrada chegou no `in/` do dependente, e ele a leu por `TYBA_IN`.
        let q_log = std::fs::read_to_string(&run.nodes["q"].outputs[&EdgeKind::Log]).unwrap();
        assert!(q_log.contains("oi"), "log do dependente: {q_log:?}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("runs").join(&run.id))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    /// App aberto pelo Dock: o processo herda o PATH do launchd e o `-l` não
    /// lê o `.zshrc`. O comando que mora num bin do dono (fixture sintética)
    /// só é achado com o PATH de login exportado pelo wrapper.
    #[cfg(unix)]
    #[test]
    fn process_runs_with_the_owner_login_path_not_the_app_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin do dono");
        std::fs::create_dir(&bin).unwrap();
        let tool = bin.join("tyba-fixture-tool");
        std::fs::write(&tool, "#!/bin/sh\necho achei\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let login_path = format!("{}:/usr/bin:/bin", bin.display());

        let (in_dir, out_dir) = (dir.path().join("in"), dir.path().join("out"));
        create_private_dir(&in_dir).unwrap();
        create_private_dir(&out_dir).unwrap();
        let (program, args) =
            process_argv(Some(&login_path), "tyba-fixture-tool", &in_dir, &out_dir);
        assert_eq!(program, bash_program());
        let status = std::process::Command::new(&program)
            .args(&args)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", dir.path())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(read_exit(&out_dir), Some(0));
        let log = std::fs::read_to_string(out_dir.join("log.txt")).unwrap();
        assert_eq!(log.trim(), "achei");
    }

    /// O comando salvo roda em `bash -lc`, qualquer que seja o `$SHELL` do
    /// dono: glob sem match fica literal (no zsh, NOMATCH derrubava o `rm`)
    /// e variável sem aspas se divide em palavras.
    #[cfg(unix)]
    #[test]
    fn process_command_keeps_bash_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let command = "rm -f dist/*.map; true\nf='a b'; for x in $f; do echo \"[$x]\"; done";
        let run = runs
            .start(graph(vec![process("p", command)], vec![]), vec![])
            .unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Done, "{run:?}");
        let p = &run.nodes["p"];
        assert_eq!(p.exit_code, Some(0), "{run:?}");
        let log = std::fs::read_to_string(&p.outputs[&EdgeKind::Log]).unwrap();
        assert!(!log.contains("no matches"), "log: {log:?}");
        assert!(log.contains("[a]") && log.contains("[b]"), "log: {log:?}");
    }

    #[cfg(unix)]
    #[test]
    fn failing_process_skips_dependent_and_missing_integration_fails() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let integration = node(
            "a",
            NodeKind::Integration {
                integration_id: "i".into(),
                dir: String::new(),
            },
        );
        let g = graph(
            vec![
                process("p", "exit 3"),
                process("q", "true"),
                note("n"),
                integration,
            ],
            vec![edge("e1", "p", "q", EdgeKind::Log)],
        );
        let run = runs.start(g, vec![]).unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.nodes["p"].exit_code, Some(3));
        assert_eq!(run.nodes["q"].status, NodeStatus::Skipped);
        assert_eq!(run.nodes["n"].status, NodeStatus::Done);
        assert!(run.nodes["n"].outputs[&EdgeKind::File].ends_with("n.md"));
        assert_eq!(run.nodes["a"].status, NodeStatus::Failed);
        assert_eq!(
            run.nodes["a"].reason.as_deref(),
            Some("integração não encontrada")
        );
    }

    #[cfg(unix)]
    #[test]
    fn stop_kills_running_process() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let run = runs
            .start(graph(vec![process("p", "sleep 30")], vec![]), vec![])
            .unwrap();
        let session = run.nodes["p"].session_id.clone().unwrap();
        runs.stop(&run.id).unwrap();
        let run = runs.get(&run.id).unwrap();
        assert_eq!(run.status, RunStatus::Stopped);
        assert_eq!(run.nodes["p"].status, NodeStatus::Stopped);
        assert!(!host.pool.is_alive(session.parse().unwrap()));
    }

    fn agent(id: &str, prompt: &str) -> Node {
        node(
            id,
            NodeKind::Agent {
                runner: Runner::Claude,
                role: String::new(),
                prompt: prompt.into(),
                dir: String::new(),
                orchestrate: false,
            },
        )
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    #[cfg(unix)]
    #[test]
    fn agent_node_produces_redacted_context_and_diff_and_stop_kills_it() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("README.md"), "x\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);

        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut g = graph(
            vec![
                agent("a", "Faça {{coisa}}."),
                agent("b", "Revise."),
                process("q", "cat \"$TYBA_IN/context.md\" \"$TYBA_IN/diff.patch\""),
            ],
            vec![
                edge("e1", "a", "b", EdgeKind::Context),
                edge("e2", "a", "q", EdgeKind::Context),
                edge("e3", "a", "q", EdgeKind::Diff),
            ],
        );
        g.dir = repo.to_string_lossy().into_owned();
        g.variables.insert("coisa".into(), "o plano".into());
        let run = runs.start(g, vec![]).unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Done, "{run:?}");

        let a = &run.nodes["a"];
        let context = std::fs::read_to_string(&a.outputs[&EdgeKind::Context]).unwrap();
        assert!(context.contains("pronto"), "{context}");
        assert!(
            !context.contains(&fake_key()),
            "contexto sem redação: {context}"
        );
        let diff = std::fs::read_to_string(&a.outputs[&EdgeKind::Diff]).unwrap();
        assert!(diff.contains("a.txt"), "{diff}");

        let launches = host.agents.lock().clone();
        let la = launches.iter().find(|l| l.title == "a").unwrap();
        assert_eq!(la.prompt, "Faça o plano.");
        assert_eq!(la.cwd, repo);
        let lb = launches.iter().find(|l| l.title == "b").unwrap();
        // O contexto entra no corpo do prompt, já redigido.
        assert!(
            lb.prompt.starts_with("Revise.\n\nContexto:\npronto"),
            "{}",
            lb.prompt
        );
        assert!(!lb.prompt.contains(&fake_key()), "{}", lb.prompt);
        assert!(lb.prompt.contains("context.md (contexto)"), "{}", lb.prompt);

        let q_log = std::fs::read_to_string(&run.nodes["q"].outputs[&EdgeKind::Log]).unwrap();
        assert!(q_log.contains("pronto"), "{q_log}");
        assert!(!q_log.contains(&fake_key()), "{q_log}");

        // Run fechado, agentes vivos: o Parar ainda os mata.
        runs.stop(&run.id).unwrap();
        let killed = host.killed.lock().clone();
        for id in ["a", "b"] {
            let session = run.nodes[id].session_id.clone().unwrap();
            assert!(killed.contains(&session), "{id}: {killed:?}");
        }
        assert!(runs.stop(&run.id).is_err());
    }

    fn in_dir(mut n: Node, dir: &Path) -> Node {
        *n.kind.dir_mut().expect("nó executável") = dir.to_string_lossy().into_owned();
        n
    }

    fn init_repo(repo: &Path) {
        std::fs::create_dir(repo).unwrap();
        git(repo, &["init", "-q"]);
        git(repo, &["config", "user.email", "t@example.com"]);
        git(repo, &["config", "user.name", "t"]);
        git(repo, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("README.md"), "x\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", "init"]);
    }

    #[cfg(unix)]
    #[test]
    fn nodes_of_one_graph_run_in_their_own_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let (one, two, default) = (
            dir.path().join("um"),
            dir.path().join("dois"),
            dir.path().join("padrao"),
        );
        for d in [&one, &two, &default] {
            std::fs::create_dir(d).unwrap();
        }
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let mut file_out = in_dir(process("q", "echo segundo > saida.txt"), &two);
        if let NodeKind::Process { output, file, .. } = &mut file_out.kind {
            *output = OutputKind::File;
            *file = Some("saida.txt".into());
        }
        let mut g = graph(
            vec![
                in_dir(process("p", "echo primeiro > aqui.txt"), &one),
                file_out,
                process("r", "echo padrao > aqui.txt"),
            ],
            vec![],
        );
        g.dir = default.to_string_lossy().into_owned();
        let run = runs.start(g, vec![]).unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Done, "{run:?}");

        assert_eq!(
            std::fs::read_to_string(one.join("aqui.txt")).unwrap(),
            "primeiro\n"
        );
        // O `file` relativo é lido da pasta do nó, não da padrão.
        assert_eq!(
            PathBuf::from(&run.nodes["q"].outputs[&EdgeKind::File]),
            two.join("saida.txt")
        );
        assert!(!two.join("aqui.txt").exists());
        // Nó sem pasta própria (grafo salvo antes) cai na padrão do grafo.
        assert_eq!(
            std::fs::read_to_string(default.join("aqui.txt")).unwrap(),
            "padrao\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn agent_snapshot_and_diff_use_the_node_dir() {
        let dir = tempfile::tempdir().unwrap();
        let (mine, other) = (dir.path().join("meu"), dir.path().join("outro"));
        init_repo(&mine);
        init_repo(&other);
        // A pasta padrão do grafo tem uma mudança que não é do agente.
        std::fs::write(other.join("alheio.txt"), "nao\n").unwrap();

        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut g = graph(vec![in_dir(agent("a", "faça"), &mine)], vec![]);
        g.dir = other.to_string_lossy().into_owned();
        let run = runs.start(g, vec![]).unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Done, "{run:?}");

        let launch = host.agents.lock()[0].clone();
        assert_eq!(launch.cwd, mine);
        let diff = std::fs::read_to_string(&run.nodes["a"].outputs[&EdgeKind::Diff]).unwrap();
        assert!(diff.contains("a.txt"), "{diff}");
        assert!(!diff.contains("alheio.txt"), "{diff}");
        assert!(!other.join("a.txt").exists());
        runs.stop(&run.id).unwrap();
    }

    fn integration(id: &str, script: &str, trigger: Trigger) -> Integration {
        Integration {
            id: id.into(),
            name: id.into(),
            script: script.into(),
            trigger,
            output: OutputKind::Log,
            file: None,
        }
    }

    fn integration_node(id: &str, integration_id: &str) -> Node {
        node(
            id,
            NodeKind::Integration {
                integration_id: integration_id.into(),
                dir: String::new(),
            },
        )
    }

    #[test]
    fn held_node_waits_until_released() {
        let g = graph(
            vec![process("p", "x"), integration_node("m", "i")],
            vec![edge("e1", "p", "m", EdgeKind::Log)],
        );
        let mut s = Scheduler::new(&g, "r".into(), 0);
        s.hold("m");
        assert!(s.release("p").is_err(), "p não é manual");
        s.mark_running("p", 1);
        s.finish("p", ok(), 2);
        assert!(s.ready().is_empty(), "manual não roda sozinho");
        assert_eq!(s.run().status, RunStatus::Running);
        s.release("m").unwrap();
        assert_eq!(s.ready(), vec!["m".to_string()]);
        assert!(s.release("m").is_err(), "▶ duas vezes");
    }

    fn wait_status(runs: &Runs<Arc<PtyHost>>, run: &str, node: &str, want: NodeStatus) -> Run {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let r = runs.get(run).unwrap();
            if r.nodes[node].status == want || Instant::now() > deadline {
                return r;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn manual_integration_runs_only_after_run_node() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let mut g = graph(
            vec![
                process("p", "echo entrada"),
                integration_node("m", "manual"),
                integration_node("o", "auto"),
            ],
            vec![
                edge("e1", "p", "m", EdgeKind::Log),
                edge("e2", "p", "o", EdgeKind::Log),
            ],
        );
        g.variables.insert("quem".into(), "dono".into());
        let integrations = vec![
            integration(
                "manual",
                "echo {{quem}}; cat \"$TYBA_IN/log.txt\"",
                Trigger::Manual,
            ),
            integration(
                "auto",
                "echo automatico",
                Trigger::OnInput {
                    kind: EdgeKind::Log,
                },
            ),
        ];
        let run = runs.start(g, integrations).unwrap();

        // on_input roda quando a entrada chega; o manual fica esperando.
        let snapshot = wait_status(&runs, &run.id, "o", NodeStatus::Done);
        assert_eq!(snapshot.nodes["o"].status, NodeStatus::Done, "{snapshot:?}");
        std::thread::sleep(Duration::from_millis(200));
        let snapshot = runs.get(&run.id).unwrap();
        assert_eq!(snapshot.nodes["m"].status, NodeStatus::Waiting);
        assert_eq!(snapshot.status, RunStatus::Running);

        assert!(
            runs.run_node(&run.id, "p").is_err(),
            "processo não é manual"
        );
        runs.run_node(&run.id, "m").unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Done, "{run:?}");
        let log = std::fs::read_to_string(&run.nodes["m"].outputs[&EdgeKind::Log]).unwrap();
        assert!(log.contains("dono"), "variável: {log:?}");
        assert!(log.contains("entrada"), "entrada: {log:?}");
        assert!(runs.run_node(&run.id, "m").is_err(), "run já fechado");
    }

    #[cfg(unix)]
    #[test]
    fn integration_test_reuses_last_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(
            vec![process("p", "echo primeira"), integration_node("m", "i")],
            vec![edge("e1", "p", "m", EdgeKind::Log)],
        );
        let first = integration(
            "i",
            "cat \"$TYBA_IN/log.txt\"",
            Trigger::OnInput {
                kind: EdgeKind::Log,
            },
        );

        // Sem execução anterior do nó: erro claro.
        let err = runs
            .test_integration(g.clone(), first.clone(), "m")
            .unwrap_err();
        assert!(err.contains("ainda não rodou"), "{err}");

        let run = runs.start(g.clone(), vec![first.clone()]).unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Done, "{run:?}");

        // O script editado (manual, inclusive) roda na hora com o in/ de antes,
        // sem o processo de origem rodar de novo.
        let edited = Integration {
            script: "echo testado; cat \"$TYBA_IN/log.txt\"".into(),
            trigger: Trigger::Manual,
            ..first
        };
        let test = runs.test_integration(g, edited, "m").unwrap();
        assert_ne!(test.id, run.id);
        assert!(test.test, "run de teste marcado para a poda");
        assert!(!run.test);
        assert_eq!(test.nodes.len(), 1);
        let test = wait_finished(&runs, &test.id);
        assert_eq!(test.status, RunStatus::Done, "{test:?}");
        let log = std::fs::read_to_string(&test.nodes["m"].outputs[&EdgeKind::Log]).unwrap();
        assert!(log.contains("testado"), "{log:?}");
        assert!(log.contains("primeira"), "in/ anterior: {log:?}");
    }

    #[cfg(unix)]
    #[test]
    fn integration_diff_output_comes_from_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("README.md"), "x\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);

        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let mut g = graph(vec![integration_node("m", "i")], vec![]);
        g.dir = repo.to_string_lossy().into_owned();
        let i = Integration {
            output: OutputKind::Diff,
            ..integration("i", "echo novo > gerado.txt", Trigger::Manual)
        };
        let run = runs.start(g, vec![i]).unwrap();
        runs.run_node(&run.id, "m").unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Done, "{run:?}");
        let diff = std::fs::read_to_string(&run.nodes["m"].outputs[&EdgeKind::Diff]).unwrap();
        assert!(diff.contains("gerado.txt"), "{diff}");
    }

    #[cfg(unix)]
    #[test]
    fn stop_during_agent_spawn_kills_the_new_session() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let (entered_tx, entered) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        *host.hold_spawn.lock() = Some((entered_tx, release_rx));

        let starter = {
            let runs = Arc::clone(&runs);
            std::thread::spawn(move || runs.start(graph(vec![agent("a", "x")], vec![]), vec![]))
        };
        entered.recv_timeout(Duration::from_secs(20)).unwrap();
        // O spawn está no meio: o run já foi publicado, a sessão ainda não.
        let run_id = host.runs.lock()[0].id.clone();
        runs.stop(&run_id).unwrap();
        release.send(()).unwrap();
        starter.join().unwrap().unwrap();

        let session = host.spawned.lock()[0].clone();
        assert!(
            host.killed.lock().contains(&session),
            "sessão que subiu depois do Parar ficou viva"
        );
        let run = runs.get(&run_id).unwrap();
        assert_eq!(run.status, RunStatus::Stopped);
        assert!(run.live_agents.is_empty(), "{run:?}");
        assert!(
            runs.stop(&run_id).is_err(),
            "agente órfão registrado no run"
        );
    }

    #[cfg(unix)]
    #[test]
    fn live_agent_keeps_run_stoppable_and_new_start_kills_it() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(vec![agent("a", "x")], vec![]);

        let first = runs.start(g.clone(), vec![]).unwrap();
        let first = wait_finished(&runs, &first.id);
        assert_eq!(first.status, RunStatus::Done, "{first:?}");
        let session = host.spawned.lock()[0].clone();
        // Run fechado, agente vivo: a tela sabe e o Parar segue valendo.
        assert_eq!(first.live_agents, vec![session.clone()]);
        assert_eq!(runs.list(&g.id)[0].live_agents, vec![session.clone()]);

        // Novo ▶ do mesmo grafo: o claude do run anterior morre antes.
        let second = runs.start(g.clone(), vec![]).unwrap();
        assert!(host.killed.lock().contains(&session));
        assert!(runs.get(&first.id).unwrap().live_agents.is_empty());
        let second = wait_finished(&runs, &second.id);
        let other = host.spawned.lock()[1].clone();
        assert_eq!(second.live_agents, vec![other.clone()]);
        assert!(!host.killed.lock().contains(&other));

        // Outro grafo não mexe nos agentes deste.
        let mut h = graph(vec![note("n")], vec![]);
        h.id = "g-2".into();
        runs.start(h, vec![]).unwrap();
        assert!(!host.killed.lock().contains(&other));
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_versions_grow_and_store_keeps_the_last() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let run = runs
            .start(graph(vec![process("p", "true"), note("n")], vec![]), vec![])
            .unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Done, "{run:?}");
        let versions: Vec<u64> = host.runs.lock().iter().map(|r| r.version).collect();
        let mut sorted = versions.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            versions.len(),
            "versão repetida: {versions:?}"
        );
        let stored = runs.store.get_maestro_run(&run.id).unwrap().unwrap();
        assert_eq!(stored.version, *sorted.last().unwrap());
        assert_eq!(stored.status, RunStatus::Done);
    }

    #[cfg(unix)]
    #[test]
    fn prune_keeps_a_run_that_is_still_active() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("runs");
        let (runs, _host, _app) = pty_runs(root.clone());
        // Integração manual: o run fica rodando, esperando o ▶ do nó.
        let waiting = runs
            .start(
                graph(vec![integration_node("m", "i")], vec![]),
                vec![integration("i", "true", Trigger::Manual)],
            )
            .unwrap();
        for _ in 0..KEEP_RUNS + 2 {
            runs.start(graph(vec![note("n")], vec![]), vec![]).unwrap();
        }
        let got = runs.get(&waiting.id).unwrap();
        assert_eq!(got.status, RunStatus::Running);
        assert!(root.join(&waiting.id).join("m").is_dir(), "pasta podada");
        assert!(
            runs.store.get_maestro_run(&waiting.id).unwrap().is_some(),
            "linha podada"
        );
        // A poda segue valendo para os outros.
        assert_eq!(runs.list("g-1").len(), KEEP_RUNS);
    }

    #[cfg(unix)]
    #[test]
    fn process_port_feeds_browser_before_the_process_ends() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        *host.ports.lock() = vec![24678, 5173];
        let g = graph(
            vec![process("p", "sleep 3"), browser("b", "")],
            vec![edge("e1", "p", "b", EdgeKind::Url)],
        );
        let run = runs.start(g, vec![]).unwrap();
        let mid = wait_status(&runs, &run.id, "b", NodeStatus::Done);
        assert_eq!(mid.nodes["p"].status, NodeStatus::Running, "{mid:?}");
        assert_eq!(mid.nodes["p"].url.as_deref(), Some("http://localhost:5173"));
        assert_eq!(mid.nodes["b"].url.as_deref(), Some("http://localhost:5173"));
        let url = std::fs::read_to_string(&mid.nodes["p"].outputs[&EdgeKind::Url]).unwrap();
        assert_eq!(url, "http://localhost:5173");
        let done = wait_finished(&runs, &run.id);
        assert_eq!(done.status, RunStatus::Done, "{done:?}");
    }

    #[cfg(unix)]
    #[test]
    fn service_writes_log_and_url_or_fails_with_reason() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let service = node(
            "s",
            NodeKind::Service {
                source: ServiceSource::Port {
                    session_id: None,
                    port: 3000,
                },
            },
        );
        *host.service.lock() = Ok(ServiceReady {
            url: Some("http://localhost:3000".into()),
            log: Some("GET / 200\n".into()),
        });
        let g = graph(
            vec![
                service.clone(),
                browser("b", ""),
                process("q", "cat \"$TYBA_IN/log.txt\""),
            ],
            vec![
                edge("e1", "s", "b", EdgeKind::Url),
                edge("e2", "s", "q", EdgeKind::Log),
            ],
        );
        let run = wait_finished(&runs, &runs.start(g, vec![]).unwrap().id);
        assert_eq!(run.status, RunStatus::Done, "{run:?}");
        assert_eq!(run.nodes["b"].url.as_deref(), Some("http://localhost:3000"));
        let q = std::fs::read_to_string(&run.nodes["q"].outputs[&EdgeKind::Log]).unwrap();
        assert!(q.contains("GET / 200"), "{q:?}");

        // Alvo que não existe: falha com o motivo e pula quem depende.
        *host.service.lock() = Err("nada escuta na porta :3000".into());
        let g = graph(
            vec![service, browser("b", "")],
            vec![edge("e1", "s", "b", EdgeKind::Url)],
        );
        let run = wait_finished(&runs, &runs.start(g, vec![]).unwrap().id);
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(
            run.nodes["s"].reason.as_deref(),
            Some("nada escuta na porta :3000")
        );
        assert_eq!(run.nodes["b"].status, NodeStatus::Skipped);
    }

    #[test]
    fn scheduler_accepts_a_new_node_while_running() {
        let g = graph(vec![process("p", "x"), process("q", "x")], vec![]);
        let mut s = Scheduler::new(&g, "r".into(), 0);
        s.mark_running("p", 1);
        s.mark_running("q", 1);
        // Sem entrada: pronto na hora.
        s.add_node("n", &[]).unwrap();
        assert_eq!(s.ready(), vec!["n".to_string()]);
        assert!(s.add_node("n", &[]).is_err(), "id repetido");
        // Esperando a url de p: começa quando ela sai.
        s.add_node("b", &[("p".into(), EdgeKind::Url)]).unwrap();
        assert_eq!(s.ready(), vec!["n".to_string()]);
        s.publish_url("p", "/u".into(), "http://localhost:1".into());
        assert_eq!(s.ready(), vec!["n".to_string(), "b".to_string()]);
        // Origem que já falhou pula o novo na hora.
        s.finish("q", Outcome::failed("x"), 2);
        s.add_node("c", &[("q".into(), EdgeKind::Log)]).unwrap();
        assert_eq!(status(&s, "c"), NodeStatus::Skipped);
        // Parar um nó só: quem depende dele é pulado; o run segue.
        s.mark_running("n", 3);
        s.set_session("n", "sess-n".into());
        assert_eq!(s.stop_node("n", 4), Some("sess-n".into()));
        assert_eq!(status(&s, "n"), NodeStatus::Stopped);
        assert_eq!(s.stop_node("b", 4), None);
        assert_eq!(status(&s, "b"), NodeStatus::Skipped);
        assert_eq!(s.run().status, RunStatus::Running);
        // Run fechado não aceita nó novo.
        s.stop(5);
        assert!(s.add_node("d", &[]).is_err());
    }

    fn orchestrator(id: &str) -> Node {
        node(
            id,
            NodeKind::Agent {
                runner: Runner::Claude,
                role: String::new(),
                prompt: "suba o ambiente".into(),
                dir: String::new(),
                orchestrate: true,
            },
        )
    }

    /// Espera o orquestrador fechar o primeiro turno (segue rodando).
    fn wait_orchestrating(runs: &Runs<Arc<PtyHost>>, run: &str, node: &str) -> Run {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let r = runs.get(run).unwrap();
            if r.nodes[node].orchestrating || Instant::now() > deadline {
                return r;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn orchestrator_keeps_the_run_open_until_its_session_exits() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        // Sem processo longo: só o orquestrador segura o run.
        let g = graph(
            vec![orchestrator("o"), agent("a", "x")],
            vec![edge("e1", "o", "a", EdgeKind::Context)],
        );
        let run = runs.start(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        // Quem depende dele recebe o contexto do turno e termina.
        let r = wait_status(&runs, &run.id, "a", NodeStatus::Done);
        assert_eq!(r.nodes["a"].status, NodeStatus::Done, "{r:?}");
        assert_eq!(r.nodes["o"].status, NodeStatus::Running, "{r:?}");
        assert!(r.nodes["o"].outputs.contains_key(&EdgeKind::Context));
        assert_eq!(r.status, RunStatus::Running, "{r:?}");

        // Run aberto: o orquestrador ainda cria nós.
        let tools = tools_of(&host, "o");
        let created = tools
            .call(
                "maestro_create_browser",
                &json!({"title": "b", "url": "http://localhost:1"}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        let r = wait_status(&runs, &run.id, &created, NodeStatus::Done);
        assert_eq!(r.nodes[&created].status, NodeStatus::Done, "{r:?}");
        assert_eq!(r.status, RunStatus::Running);

        // A sessão do orquestrador sai: o nó fica pronto e o run fecha.
        let session = r.nodes["o"].session_id.clone().unwrap();
        host.exit_agent(&session);
        let r = wait_finished(&runs, &run.id);
        assert_eq!(r.status, RunStatus::Done, "{r:?}");
        assert_eq!(r.nodes["o"].status, NodeStatus::Done);
        assert!(!r.nodes["o"].orchestrating);
        assert!(r.nodes["o"].outputs.contains_key(&EdgeKind::Context));
        // Fechado: criados descartados e nada mais se cria.
        assert!(runs.created_to_keep(&run.id).is_err());
        let err = tools
            .call(
                "maestro_create_browser",
                &json!({"title": "tarde", "url": "http://localhost:1"}),
            )
            .unwrap_err();
        assert!(err.contains("já terminou"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn agent_without_orchestrate_closes_the_run_at_the_end_of_the_turn() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let run = runs
            .start(graph(vec![agent("a", "x")], vec![]), vec![])
            .unwrap();
        let r = wait_finished(&runs, &run.id);
        assert_eq!(r.status, RunStatus::Done, "{r:?}");
        assert_eq!(r.nodes["a"].status, NodeStatus::Done);
        assert!(!r.nodes["a"].orchestrating);
        // Sessão viva depois do turno, como antes; sem aviso de saída.
        assert_eq!(r.live_agents, vec![host.spawned.lock()[0].clone()]);
        assert!(host.exits.lock().is_empty());
    }

    /// As ferramentas que o run entregou à sessão do orquestrador.
    fn tools_of(host: &PtyHost, title: &str) -> ToolHandler {
        let launches = host.agents.lock().clone();
        let launch = launches.iter().find(|l| l.title == title).unwrap();
        launch.tools.clone().expect("orquestrador sem ferramentas")
    }

    #[cfg(unix)]
    #[test]
    fn orchestrator_creates_nodes_in_the_active_run_and_stop_discards_them() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("api");
        std::fs::create_dir(&work).unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        // O processo longo mantém o run vivo depois do turno do orquestrador.
        let g = graph(
            vec![orchestrator("o"), agent("a", "x"), process("p", "sleep 30")],
            vec![],
        );
        runs.store.save_maestro_graph(&g, true, None).unwrap();
        let run = runs.start(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        // Só o nó com orquestrar ligado recebe as ferramentas.
        assert!(host
            .agents
            .lock()
            .iter()
            .find(|l| l.title == "a")
            .unwrap()
            .tools
            .is_none());
        let tools = tools_of(&host, "o");

        let created = tools
            .call(
                "maestro_create_process",
                &json!({"title": "api", "command": "echo subiu; sleep 30", "dir": work}),
            )
            .unwrap();
        let api = created["node_id"].as_str().unwrap().to_string();
        let mid = wait_status(&runs, &run.id, &api, NodeStatus::Running);
        assert_eq!(mid.nodes[&api].status, NodeStatus::Running, "{mid:?}");
        assert!(mid.nodes[&api].session_id.is_some());
        assert_eq!(mid.created.len(), 1);
        let c = &mid.created[0];
        assert_eq!(c.created_by, "o");
        assert_eq!(c.node.id, api);
        assert!(!c.kept);
        // O evento leva o nó criado para o front.
        assert!(host
            .runs
            .lock()
            .iter()
            .any(|r| r.created.iter().any(|c| c.node.id == api)));
        // O processo roda na pasta pedida.
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            let st = tools
                .call("maestro_node_status", &json!({"node_id": api}))
                .unwrap();
            if st["log"].as_str().is_some_and(|l| l.contains("subiu")) || Instant::now() > deadline
            {
                break st;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(status["status"], "running", "{status}");
        assert!(
            status["log"].as_str().unwrap().contains("subiu"),
            "{status}"
        );

        // Validação do core: pasta inexistente, url não local, nó de outro.
        let err = tools
            .call(
                "maestro_create_process",
                &json!({"title": "x", "command": "true", "dir": "/nao/existe/tyba-orq"}),
            )
            .unwrap_err();
        assert!(err.contains("não existe"), "{err}");
        let err = tools
            .call(
                "maestro_create_browser",
                &json!({"title": "x", "url": "https://example.com"}),
            )
            .unwrap_err();
        assert!(err.contains("só local"), "{err}");
        let err = tools
            .call(
                "maestro_create_browser",
                &json!({"title": "x", "from_node": "a"}),
            )
            .unwrap_err();
        assert!(err.contains("não produz url"), "{err}");
        let err = tools
            .call("maestro_stop_node", &json!({"node_id": "p"}))
            .unwrap_err();
        assert!(err.contains("só para nó criado"), "{err}");
        assert_eq!(
            runs.get(&run.id).unwrap().nodes["p"].status,
            NodeStatus::Running
        );

        // Navegador ligado na url do processo criado: ligação url de verdade.
        *host.ports.lock() = vec![5180];
        let code = tools
            .call(
                "maestro_create_browser",
                &json!({"title": "code", "from_node": api}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        let r = wait_status(&runs, &run.id, &code, NodeStatus::Done);
        assert_eq!(r.nodes[&code].url.as_deref(), Some("http://localhost:5180"));
        let edges = &r.created.iter().find(|c| c.node.id == code).unwrap().edges;
        assert_eq!(edges.len(), 1);
        assert_eq!(
            (edges[0].from.as_str(), edges[0].kind),
            (api.as_str(), EdgeKind::Url)
        );

        // Teto de 30 por execução (já são 2).
        for i in 2..MAX_CREATED {
            tools
                .call(
                    "maestro_create_browser",
                    &json!({"title": format!("b{i}"), "url": "http://localhost:1"}),
                )
                .unwrap();
        }
        let err = tools
            .call(
                "maestro_create_browser",
                &json!({"title": "demais", "url": "http://localhost:1"}),
            )
            .unwrap_err();
        assert!(err.contains("teto"), "{err}");

        // Parar o nó criado: só ele morre.
        let session = runs.get(&run.id).unwrap().nodes[&api]
            .session_id
            .clone()
            .unwrap();
        tools
            .call("maestro_stop_node", &json!({"node_id": api}))
            .unwrap();
        assert!(host.killed.lock().contains(&session));
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.nodes[&api].status, NodeStatus::Stopped);
        assert_eq!(r.status, RunStatus::Running);

        // ■ Parar: o run fecha e os criados ficam fora do grafo.
        runs.stop(&run.id).unwrap();
        assert!(runs.created_to_keep(&run.id).is_err());
        let err = tools
            .call(
                "maestro_create_process",
                &json!({"title": "tarde", "command": "true", "dir": work}),
            )
            .unwrap_err();
        assert!(err.contains("já terminou"), "{err}");
        // O grafo salvo nunca recebeu os criados.
        let saved = runs.store.get_maestro_graph("g-1").unwrap().unwrap().value;
        assert_eq!(saved.nodes.len(), 3);
        assert_eq!(runs.get(&run.id).unwrap().status, RunStatus::Stopped);
    }

    /// O orquestrador delega a um agente criado e lê o resultado: a fala
    /// final (redigida), o que mudou na pasta e onde ler o resto.
    #[cfg(unix)]
    #[test]
    fn node_status_of_a_created_agent_brings_its_answer_and_diff() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(vec![orchestrator("o"), process("p", "sleep 30")], vec![]);
        let run = runs.start(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        let tools = tools_of(&host, "o");
        let rev = tools
            .call(
                "maestro_create_agent",
                &json!({"title": "revisor", "runner": "claude", "prompt": "revise", "dir": repo}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        let r = wait_status(&runs, &run.id, &rev, NodeStatus::Done);
        assert_eq!(r.nodes[&rev].status, NodeStatus::Done, "{r:?}");

        let st = tools
            .call("maestro_node_status", &json!({"node_id": rev}))
            .unwrap();
        let text = st["text"].as_str().expect("sem o texto do turno");
        assert!(text.starts_with("pronto, usei "), "{st}");
        assert!(!text.contains(&fake_key()), "segredo no resultado: {st}");
        assert_eq!(st["text_truncated"], false);
        assert_eq!(st["diff"]["files_changed"], 1, "{st}");
        assert_eq!(
            st["diff"]["files"],
            json!([{"path": "revisor.txt", "added": 1, "removed": 0}])
        );
        for kind in ["context", "diff"] {
            let path = st["paths"][kind].as_str().unwrap();
            assert!(
                Path::new(path).is_absolute() && Path::new(path).is_file(),
                "{st}"
            );
        }
        assert!(st["paths"].get("log").is_none(), "agente não grava log.txt");
        runs.stop(&run.id).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn keep_writes_created_nodes_into_the_graph_as_normal_nodes() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(vec![orchestrator("o"), process("p", "sleep 30")], vec![]);
        runs.store.save_maestro_graph(&g, true, None).unwrap();
        let run = runs.start(g.clone(), vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        let tools = tools_of(&host, "o");
        let web = tools
            .call(
                "maestro_create_process",
                &json!({"title": "web", "command": "sleep 30", "dir": dir.path()}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        let page = tools
            .call(
                "maestro_create_browser",
                &json!({"title": "page", "from_node": web}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();

        // O que o comando "Manter no grafo" faz.
        let keep = runs.created_to_keep(&run.id).unwrap();
        assert_eq!(keep.len(), 2);
        let mut graph = runs.store.get_maestro_graph(&g.id).unwrap().unwrap().value;
        orchestrator::merge_kept(&mut graph, &keep);
        crate::maestro::validate_graph(&graph, &[]).unwrap();
        runs.store.save_maestro_graph(&graph, true, None).unwrap();
        runs.mark_kept(
            &run.id,
            &keep.iter().map(|c| c.node.id.clone()).collect::<Vec<_>>(),
        );

        let saved = runs.store.get_maestro_graph(&g.id).unwrap().unwrap().value;
        let ids: Vec<&str> = saved.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["o", "p", web.as_str(), page.as_str()]);
        // Nó normal: sem marca de criado no grafo; a ligação url vai junto.
        let body = serde_json::to_value(&saved).unwrap().to_string();
        assert!(!body.contains("createdBy"), "{body}");
        assert!(saved
            .edges
            .iter()
            .any(|e| e.from == web && e.to == page && e.kind == EdgeKind::Url));
        let r = runs.get(&run.id).unwrap();
        assert!(r.created.iter().all(|c| c.kept));
        // Já mantidos: nada mais a manter.
        assert!(runs.created_to_keep(&run.id).unwrap().is_empty());
        runs.stop(&run.id).unwrap();
    }

    /// O app fechou no meio: o run gravado `running` tem nó rodando (com a
    /// sessão que morreu no quit), nó esperando e nó pronto. O boot fecha.
    #[test]
    fn boot_closes_the_runs_the_app_left_open() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let g = graph(
            vec![process("p", "sleep 30"), note("n"), process("d", "true")],
            vec![edge("e1", "p", "d", EdgeKind::Log)],
        );
        let mut s = Scheduler::new(&g, "r-aberto".into(), 1);
        s.mark_running("p", 2);
        s.set_session("p", "sessao-morta".into());
        s.mark_running("n", 2);
        assert!(s.finish("n", ok(), 3));
        let open = s.snapshot();
        assert_eq!(open.status, RunStatus::Running);
        store.save_maestro_run(&open).unwrap();
        let mut done = open.clone();
        done.id = "r-pronto".into();
        done.status = RunStatus::Done;
        store.save_maestro_run(&done).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs_on(dir.path().join("runs"), Arc::clone(&store));

        let r = store.get_maestro_run("r-aberto").unwrap().unwrap();
        assert_eq!(r.status, RunStatus::Stopped);
        assert!(r.finished_at.is_some());
        assert_eq!(r.nodes["p"].status, NodeStatus::Stopped);
        assert_eq!(r.nodes["p"].reason.as_deref(), Some(APP_CLOSED));
        assert!(r.nodes["p"].finished_at.is_some());
        assert_eq!(r.nodes["d"].status, NodeStatus::Skipped);
        assert_eq!(r.nodes["n"].status, NodeStatus::Done);
        assert!(r.nodes.values().all(|n| n.session_id.is_none()), "{r:?}");
        // O que o front lê é o mesmo retrato.
        assert_eq!(runs.get("r-aberto").unwrap().nodes["p"].session_id, None);
        // Run já fechado não muda.
        assert_eq!(store.get_maestro_run("r-pronto").unwrap(), Some(done));
    }

    #[test]
    fn unsafe_node_id_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        assert!(runs
            .start(graph(vec![note("../fora")], vec![]), vec![])
            .is_err());
    }
}
