//! Motor de execução do Maestro: processos, notas e agentes (integração
//! chega no bloco seguinte).
//!
//! ▶ Executar cria um [`Run`] e uma pasta `~/.tyba/maestro/runs/<run>/<nó>/
//! {in,out}` (0700). Um nó começa quando todas as ligações que chegam nele já
//! têm valor; independentes rodam em paralelo. Falha pula tudo que depende do
//! nó; Parar mata o process group de cada nó rodando.
//!
//! O agendador ([`Scheduler`]) é puro. Quem sobe processo, mata e avisa o front
//! é o [`RunHost`] — no app, [`TauriHost`], que passa pelo `SessionManager`.
//! Todo nó roda na pasta do workspace (`Graph::dir`), como o dono. Sem worktree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::agent::AgentTurn;
use super::{EdgeKind, Graph, Node, NodeKind, OutputKind, Runner, Stored};
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
    sources: HashMap<String, Vec<String>>,
    dependents: HashMap<String, Vec<String>>,
}

impl Scheduler {
    pub fn new(graph: &Graph, run_id: String, now: u64) -> Self {
        let mut sources: HashMap<String, Vec<String>> = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        for edge in &graph.edges {
            sources
                .entry(edge.to.clone())
                .or_default()
                .push(edge.from.clone());
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
        }
    }

    pub fn run(&self) -> &Run {
        &self.run
    }

    fn status(&self, id: &str) -> Option<NodeStatus> {
        self.run.nodes.get(id).map(|n| n.status)
    }

    /// Nós esperando cujas entradas todas já têm valor (origem `done`).
    pub fn ready(&self) -> Vec<String> {
        if self.run.status != RunStatus::Running {
            return Vec::new();
        }
        self.order
            .iter()
            .filter(|id| self.status(id) == Some(NodeStatus::Waiting))
            .filter(|id| {
                self.sources
                    .get(*id)
                    .into_iter()
                    .flatten()
                    .all(|s| self.status(s) == Some(NodeStatus::Done))
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
        n.finished_at = Some(now);
        n.exit_code = outcome.exit_code;
        n.outputs = outcome.outputs;
        n.reason = outcome.reason;
        if !outcome.ok {
            self.skip_dependents(id);
        }
        self.settle(now);
        true
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
fn safe_component(id: &str) -> bool {
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

/// `bash -lc` com o wrapper de rules. `TYBA_IN`/`TYBA_OUT` chegam como
/// argumentos posicionais e são exportados antes do comando — assim o caminho
/// não passa por aspas montadas à mão. O `}` vai em linha própria para um
/// comentário no fim do comando não engolir o fechamento.
pub fn process_argv(command: &str, in_dir: &Path, out_dir: &Path) -> (PathBuf, Vec<String>) {
    let script = format!(
        "export TYBA_IN=\"$1\" TYBA_OUT=\"$2\"; shift 2\n\
         set -o pipefail; {{ {command}\n}} 2>&1 | tee \"$TYBA_OUT/log.txt\"; \
         echo $? > \"$TYBA_OUT/.exit\""
    );
    (
        bash_program(),
        vec![
            "-lc".into(),
            script,
            "tyba-maestro".into(),
            in_dir.to_string_lossy().into_owned(),
            out_dir.to_string_lossy().into_owned(),
        ],
    )
}

/// Exit gravado pelo wrapper. Sem `.exit` o processo morreu antes do fim.
fn read_exit(out_dir: &Path) -> Option<i32> {
    std::fs::read_to_string(out_dir.join(".exit"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Onde mora a saída declarada de um processo.
fn process_outputs(
    output: OutputKind,
    file: Option<&str>,
    out_dir: &Path,
    graph_dir: &Path,
) -> HashMap<EdgeKind, String> {
    let path = match output {
        OutputKind::Log => Some(out_dir.join("log.txt")),
        OutputKind::Json => Some(out_dir.join("out.json")),
        OutputKind::File => file.map(|f| graph_dir.join(f)),
        // O snapshot de diff chega com o nó de agente (B4); até lá o destino
        // recebe `diff.patch` vazio, como num diretório sem git.
        OutputKind::Diff => None,
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
}

/// Efeitos fora do agendador: subir processo, matar, avisar o front.
pub trait RunHost: Send + Sync + 'static {
    /// Sobe o agente livre e devolve o id da sessão. `on_turn` roda uma vez,
    /// quando o primeiro turno fecha (ou o agente sai antes disso). O agente
    /// segue vivo depois.
    fn spawn_agent(
        &self,
        launch: AgentLaunch,
        on_turn: Box<dyn FnOnce(AgentTurn) + Send>,
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
    /// `maestro://run/<run_id>` com o run inteiro.
    fn emit(&self, run: &Run);
}

struct Active {
    graph: Graph,
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
    graph_dir: PathBuf,
}

enum Launched {
    Session(String),
    Finished(Outcome),
}

pub struct Runs<H: RunHost> {
    host: H,
    store: Arc<Store>,
    root: Option<PathBuf>,
    active: Mutex<HashMap<String, Active>>,
    /// Sessões de agente de cada run: continuam vivas depois do nó pronto, e
    /// o Parar as mata mesmo com o run já fechado.
    agents: Mutex<HashMap<String, Vec<String>>>,
    me: Weak<Runs<H>>,
}

fn now() -> u64 {
    crate::approvals::now_ms()
}

impl<H: RunHost> Runs<H> {
    pub fn new(host: H, store: Arc<Store>, root: Option<PathBuf>) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            host,
            store,
            root,
            active: Mutex::new(HashMap::new()),
            agents: Mutex::new(HashMap::new()),
            me: me.clone(),
        })
    }

    /// Começa o run. O grafo já passou por consentimento e validação.
    pub fn start(&self, graph: Graph) -> Result<Run, String> {
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

        let sched = Scheduler::new(&graph, run_id.clone(), now());
        let graph_id = graph.id.clone();
        let snapshot = sched.run().clone();
        if snapshot.status == RunStatus::Running {
            self.active
                .lock()
                .insert(run_id.clone(), Active { graph, sched, dir });
        }
        self.publish(&snapshot);
        self.prune(&graph_id);
        self.advance(&run_id);
        Ok(self.get(&run_id).unwrap_or(snapshot))
    }

    /// Run em memória (rodando) ou do banco. Run que ficou `running` no banco
    /// sem estar ativo é de um app que fechou no meio: aparece parado.
    pub fn get(&self, run_id: &str) -> Option<Run> {
        if let Some(a) = self.active.lock().get(run_id) {
            return Some(a.sched.run().clone());
        }
        let mut run = self.store.get_maestro_run(run_id).ok().flatten()?;
        if run.status == RunStatus::Running {
            run.status = RunStatus::Stopped;
        }
        Some(run)
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
        let stopped = self.active.lock().remove(run_id).map(|mut a| {
            let kill = a.sched.stop(now());
            (a.sched.run().clone(), kill)
        });
        let agents = self.agents.lock().remove(run_id).unwrap_or_default();
        if stopped.is_none() && agents.is_empty() {
            return Err("execução não está rodando".into());
        }
        let mut kill: Vec<String> = Vec::new();
        if let Some((snapshot, sessions)) = stopped {
            self.publish(&snapshot);
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

    /// Grava, avisa o front e, se o run fechou, tira da memória.
    fn publish(&self, run: &Run) {
        if let Err(e) = self.store.save_maestro_run(run) {
            eprintln!("[maestro] gravar execução {}: {e}", run.id);
        }
        self.host.emit(run);
    }

    fn snapshot_and_retire(&self, run_id: &str) -> Option<Run> {
        let mut active = self.active.lock();
        let run = active.get(run_id)?.sched.run().clone();
        if run.status != RunStatus::Running {
            active.remove(run_id);
        }
        Some(run)
    }

    fn prune(&self, graph_id: &str) {
        let Some(root) = &self.root else { return };
        let removed = match self.store.prune_maestro_runs(graph_id, KEEP_RUNS) {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!("[maestro] podar execuções: {e}");
                return;
            }
        };
        for id in removed {
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
            if let Some(run) = self.snapshot_and_retire(run_id) {
                self.publish(&run);
            }
            for start in starts {
                let node_id = start.node.id.clone();
                let launched = self.launch(run_id, start);
                {
                    let mut active = self.active.lock();
                    let Some(a) = active.get_mut(run_id) else {
                        return;
                    };
                    match launched {
                        Launched::Session(session) => a.sched.set_session(&node_id, session),
                        Launched::Finished(outcome) => {
                            a.sched.finish(&node_id, outcome, now());
                        }
                    }
                }
                if let Some(run) = self.snapshot_and_retire(run_id) {
                    self.publish(&run);
                }
            }
        }
    }

    fn launch(&self, run_id: &str, start: Start) -> Launched {
        match &start.node.kind {
            NodeKind::Note { markdown } => {
                let path = start.out_dir.join(note_file_name(&start.node.title));
                match std::fs::write(&path, markdown) {
                    Ok(()) => Launched::Finished(Outcome {
                        ok: true,
                        outputs: HashMap::from([(
                            EdgeKind::File,
                            path.to_string_lossy().into_owned(),
                        )]),
                        ..Outcome::default()
                    }),
                    Err(e) => Launched::Finished(Outcome::failed(format!("gravar a nota: {e}"))),
                }
            }
            NodeKind::Integration { .. } => {
                Launched::Finished(Outcome::failed("ainda não suportado"))
            }
            NodeKind::Agent { runner, prompt, .. } => {
                self.launch_agent(run_id, &start, *runner, prompt)
            }
            NodeKind::Process { command, .. } => {
                if let Err(e) = materialize(&start.in_dir, &start.inputs) {
                    return Launched::Finished(Outcome::failed(format!("preparar entradas: {e}")));
                }
                let command = crate::snippet::render(command, &start.variables);
                let (program, args) = process_argv(&command, &start.in_dir, &start.out_dir);
                let me = self.me.clone();
                let (run, node) = (run_id.to_string(), start.node.id.clone());
                let on_exit = Box::new(move || {
                    if let Some(runs) = me.upgrade() {
                        runs.process_exited(&run, &node);
                    }
                });
                let launch = ProcessLaunch {
                    title: start.node.title.clone(),
                    cwd: start.graph_dir.clone(),
                    program,
                    args,
                };
                match self.host.spawn_process(launch, on_exit) {
                    Ok(session) => Launched::Session(session),
                    Err(e) => Launched::Finished(Outcome::failed(format!("subir o processo: {e}"))),
                }
            }
        }
    }

    /// Entradas em `in/`, snapshot de diff na largada e o agente livre na
    /// pasta do workspace.
    fn launch_agent(&self, run_id: &str, start: &Start, runner: Runner, prompt: &str) -> Launched {
        let written = match materialize(&start.in_dir, &start.inputs) {
            Ok(w) => w,
            Err(e) => {
                return Launched::Finished(Outcome::failed(format!("preparar entradas: {e}")))
            }
        };
        let inputs: Vec<(EdgeKind, PathBuf)> =
            start.inputs.iter().map(|i| i.kind).zip(written).collect();
        let prompt = crate::snippet::render(prompt, &start.variables);
        let prompt = super::agent::build_prompt(&prompt, &inputs);
        let snapshot = super::agent::snapshot(&start.graph_dir);
        let me = self.me.clone();
        let (run, node, out_dir) = (
            run_id.to_string(),
            start.node.id.clone(),
            start.out_dir.clone(),
        );
        let on_turn = Box::new(move |turn: AgentTurn| {
            if let Some(runs) = me.upgrade() {
                runs.agent_turn_ended(&run, &node, &out_dir, snapshot.as_ref(), turn);
            }
        });
        let launch = AgentLaunch {
            title: start.node.title.clone(),
            cwd: start.graph_dir.clone(),
            runner,
            prompt,
            in_dir: start.in_dir.clone(),
            out_dir: start.out_dir.clone(),
        };
        match self.host.spawn_agent(launch, on_turn) {
            Ok(session) => {
                self.agents
                    .lock()
                    .entry(run_id.to_string())
                    .or_default()
                    .push(session.clone());
                Launched::Session(session)
            }
            Err(e) => Launched::Finished(Outcome::failed(format!("subir o agente: {e}"))),
        }
    }

    /// O primeiro turno fechou: grava `context.md` (redigido) e `diff.patch`
    /// e segue o grafo. O agente continua vivo.
    fn agent_turn_ended(
        &self,
        run_id: &str,
        node_id: &str,
        out_dir: &Path,
        snapshot: Option<&super::agent::Snapshot>,
        turn: AgentTurn,
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
            if !a.sched.finish(node_id, outcome, now()) {
                return;
            }
        }
        if let Some(run) = self.snapshot_and_retire(run_id) {
            self.publish(&run);
        }
        self.advance(run_id);
    }

    /// O PTY do processo fechou: lê o `.exit` e segue o grafo.
    fn process_exited(&self, run_id: &str, node_id: &str) {
        {
            let mut active = self.active.lock();
            let Some(a) = active.get_mut(run_id) else {
                return;
            };
            let Some(node) = a.graph.nodes.iter().find(|n| n.id == node_id) else {
                return;
            };
            let out_dir = a.dir.join(node_id).join("out");
            let exit = read_exit(&out_dir);
            let outputs = match &node.kind {
                NodeKind::Process { output, file, .. } => {
                    process_outputs(*output, file.as_deref(), &out_dir, Path::new(&a.graph.dir))
                }
                _ => HashMap::new(),
            };
            let outcome = Outcome {
                ok: exit == Some(0),
                exit_code: exit,
                outputs,
                reason: exit
                    .is_none()
                    .then(|| "o processo terminou sem exit code".into()),
            };
            if !a.sched.finish(node_id, outcome, now()) {
                return;
            }
        }
        if let Some(run) = self.snapshot_and_retire(run_id) {
            self.publish(&run);
        }
        self.advance(run_id);
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
    let node_dir = a.dir.join(id);
    Some(Start {
        node,
        in_dir: node_dir.join("in"),
        out_dir: node_dir.join("out"),
        inputs,
        variables: a
            .graph
            .variables
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        graph_dir: PathBuf::from(&a.graph.dir),
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
                move |id| {
                    let state = handle.state::<crate::AppState>();
                    state.sessions.set_status(
                        &handle,
                        id,
                        crate::session::SessionStatus::Exited { code: -1 },
                    );
                    // A linha no banco faria o boot reabrir este nó como shell.
                    let _ = state.store.remove_session(id);
                    on_exit();
                },
            )
            .map_err(|e| e.to_string())?;
        // Sessão de nó não volta no boot: é do run, não da grade.
        let _ = state.store.remove_session(session.id);
        Ok(session.id.to_string())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn spawn_agent(
        &self,
        launch: AgentLaunch,
        on_turn: Box<dyn FnOnce(AgentTurn) + Send>,
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
        };
        let handle = self.app.clone();
        let (session, tail) = crate::agent::session::spawn_free_agent(&ctx, spec, move |id| {
            let state = handle.state::<crate::AppState>();
            state.sessions.set_status(
                &handle,
                id,
                crate::session::SessionStatus::Exited { code: -1 },
            );
            let _ = state.store.remove_session(id);
        })?;
        // Sessão de nó não volta no boot: é do run, não da grade.
        let _ = state.store.remove_session(session.id);
        let sessions = Arc::clone(&state.sessions);
        let id = session.id;
        super::agent::watch_first_turn(move || sessions.get(id).map(|s| s.status), tail, on_turn);
        Ok(session.id.to_string())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn spawn_agent(
        &self,
        _launch: AgentLaunch,
        _on_turn: Box<dyn FnOnce(AgentTurn) + Send>,
    ) -> Result<String, String> {
        Err("o nó de agente ainda não roda neste sistema".into())
    }

    fn kill(&self, session_id: &str) {
        use tauri::Manager;
        let Ok(id) = session_id.parse() else { return };
        let state = self.app.state::<crate::AppState>();
        state.sessions.dispose(&state.pty_pool, id);
    }

    fn emit(&self, run: &Run) {
        use tauri::Emitter;
        let _ = self.app.emit(&format!("maestro://run/{}", run.id), run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maestro::{Edge, Position, Runner, Size};
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
        ) -> Result<String, String> {
            self.agents.lock().push(launch.clone());
            std::thread::spawn(move || {
                std::fs::write(launch.cwd.join(format!("{}.txt", launch.title)), "feito\n")
                    .unwrap();
                on_turn(AgentTurn::Done {
                    text: Some(format!("pronto, usei {}", fake_key())),
                });
            });
            Ok(format!("agente-{}", uuid::Uuid::new_v4()))
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
        let app = tauri::test::mock_app();
        let host = Arc::new(PtyHost {
            app: app.handle().clone(),
            pool: crate::pty::PtyPool::new(),
            runs: Mutex::new(Vec::new()),
            agents: Mutex::new(Vec::new()),
            killed: Mutex::new(Vec::new()),
        });
        let store = Arc::new(Store::open_in_memory().unwrap());
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
        let run = runs.start(g).unwrap();
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

    #[cfg(unix)]
    #[test]
    fn failing_process_skips_dependent_and_integration_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let integration = node(
            "a",
            NodeKind::Integration {
                integration_id: "i".into(),
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
        let run = runs.start(g).unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.nodes["p"].exit_code, Some(3));
        assert_eq!(run.nodes["q"].status, NodeStatus::Skipped);
        assert_eq!(run.nodes["n"].status, NodeStatus::Done);
        assert!(run.nodes["n"].outputs[&EdgeKind::File].ends_with("n.md"));
        assert_eq!(run.nodes["a"].status, NodeStatus::Failed);
        assert_eq!(
            run.nodes["a"].reason.as_deref(),
            Some("ainda não suportado")
        );
    }

    #[cfg(unix)]
    #[test]
    fn stop_kills_running_process() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let run = runs
            .start(graph(vec![process("p", "sleep 30")], vec![]))
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
        let run = runs.start(g).unwrap();
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
        assert!(
            lb.prompt.starts_with("Revise.\n\nEntradas"),
            "{}",
            lb.prompt
        );
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

    #[test]
    fn unsafe_node_id_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        assert!(runs.start(graph(vec![note("../fora")], vec![])).is_err());
    }
}
