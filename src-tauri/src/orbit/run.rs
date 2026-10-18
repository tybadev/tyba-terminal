//! Motor de execução do Orbit: processos, notas, agentes e integrações.
//!
//! ▶ Executar cria um [`Run`] e uma pasta `~/.tyba/orbit/runs/<run>/<nó>/
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
use super::browser::AgentAction;
use super::orchestrator::{self, Created, Plan, PlanStep, ToolCall, ToolHandler, MAX_CREATED};
use super::service::{self, ServiceReady, ServiceSource};
use super::talk::{self, Mailbox, PeerState};
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
    /// Assentado: nada acontece (todo agente com o turno fechado há
    /// [`SETTLE_MS`], sem pedido do dono, sem fila de conversa, nenhum
    /// processo nem nó esperando). O run segue `running` por baixo — sessões
    /// e ferramentas vivas —, mas a tela mostra "concluída · sessões
    /// abertas" e `finished_at` guarda, provisório, o momento em que
    /// assentou (o relógio para ali). Algo volta a acontecer: sai daqui e o
    /// tempo parado vai para `paused_ms`.
    #[serde(default)]
    pub settled: bool,
    /// Tempo parado (assentado) que já passou: o tempo ativo do run é
    /// `(finished_at ou agora) - started_at - paused_ms`.
    #[serde(default)]
    pub paused_ms: u64,
    /// Nós que um orquestrador criou neste run (efêmeros: não estão no grafo
    /// salvo, a menos que o dono os mantenha).
    #[serde(default)]
    pub created: Vec<Created>,
    /// Cópia da nota de plano de cada orquestrador (a nota vive no grafo).
    #[serde(default)]
    pub plans: Vec<Plan>,
    /// "#N" da execução no grafo, dado no ▶ e gravado: não muda quando a
    /// poda tira as velhas. 0 = gravada antes da numeração.
    #[serde(default)]
    pub number: u32,
}

/// Como fecha um run sem nada rodando: falhou se algum nó falhou.
fn closed_status(nodes: &HashMap<String, NodeRun>) -> RunStatus {
    if nodes.values().any(|n| n.status == NodeStatus::Failed) {
        RunStatus::Failed
    } else {
        RunStatus::Done
    }
}

/// Motivo do nó que rodava quando o app fechou.
pub const APP_CLOSED: &str = "o TYBA fechou durante a execução";

/// Motivo do nó que o dono parou antes de ele começar.
pub const STOPPED_BEFORE_START: &str = "parado antes de começar";

impl Run {
    /// Fecha o retrato de um run que o app deixou aberto ao fechar: o PTY de
    /// cada nó morreu junto (`kill_all` no quit). Rodando vira `stopped` com
    /// [`APP_CLOSED`], esperando vira `skipped`, e nenhuma sessão fica — a
    /// que estava aqui não existe mais. A versão sobe para a gravação valer.
    fn close_interrupted(&mut self, now: u64) {
        // Assentado: o trabalho já tinha acabado, só as sessões seguiam
        // abertas — fecha concluído, com o relógio onde parou.
        if self.settled {
            let at = self.finished_at.unwrap_or(now);
            for n in self.nodes.values_mut() {
                if n.status == NodeStatus::Running {
                    n.status = NodeStatus::Done;
                    n.finished_at = Some(at);
                }
                n.session_id = None;
                n.orchestrating = false;
            }
            self.status = closed_status(&self.nodes);
            self.settled = false;
            self.finished_at = Some(at);
            self.version += 1;
            return;
        }
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
    /// Navegador: a última página que um agente abriu nele
    /// (`orbit_browser_open`, `$BROWSER`). A tela navega e mostra o chip.
    #[serde(default)]
    pub opened: Option<BrowserOpened>,
    /// Agente: mensagens de outros agentes esperando ele fechar o turno.
    #[serde(default)]
    pub inbox: usize,
    /// Agente: a mesma fila, por id de quem mandou (a etiqueta da conversa
    /// mostra "N mensagens na fila" do par).
    #[serde(default)]
    pub inbox_from: HashMap<String, usize>,
    /// Agente: a última mensagem que ele mandou (`orbit_send`), redigida.
    #[serde(default)]
    pub sent: Option<SentMessage>,
    /// Nota: a última escrita de um agente ligado por `memory`, com o texto
    /// novo (o canvas o copia para o nó, como a url do navegador).
    #[serde(default)]
    pub written: Option<NoteWritten>,
    /// Agente: a última entrega da fila falhou, com o motivo. As mensagens
    /// seguem na fila e saem na próxima tentativa; some quando uma entrega
    /// dá certo.
    #[serde(default)]
    pub undelivered: Option<String>,
    /// Quantas vezes o nó foi reiniciado ou rodado de novo neste run. O
    /// aviso de saída da sessão (processo, agente) leva o número de quando
    /// subiu: o da sessão velha, morta pelo reinício, não fecha a nova.
    #[serde(default)]
    pub attempt: u32,
    /// "↻ para aplicar": o nó já começou e algo de que ele depende mudou
    /// depois (ver [`StaleReason`]). Não reinicia sozinho; some quando ele
    /// roda de novo.
    #[serde(default)]
    pub stale: bool,
    /// Por que o nó ficou "↻ para aplicar" (o último motivo), para a tela
    /// explicar a causa certa.
    #[serde(default)]
    pub stale_reason: Option<StaleReason>,
}

/// O motivo do "↻ para aplicar".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StaleReason {
    /// Uma origem rodou de novo (ou trocou a url) depois que o nó começou.
    Source,
    /// A config do nó mudou no grafo salvo (comando, prompt, pasta, alvo,
    /// script da integração).
    Config,
    /// As ligações que chegam no nó mudaram.
    Edges,
    /// Uma variável que o comando, o prompt ou o script usa mudou de valor.
    Variable,
}

impl NodeRun {
    /// Marca "↻ para aplicar" com o motivo. Devolve `true` se mudou algo.
    fn mark_stale(&mut self, reason: StaleReason) -> bool {
        let changed = !self.stale || self.stale_reason != Some(reason);
        self.stale = true;
        self.stale_reason = Some(reason);
        changed
    }
}

/// A última mensagem que um agente mandou: a tela mostra "→ para X: …".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentMessage {
    /// Título do nó de destino.
    pub to: String,
    /// O começo do texto ([`talk::preview`]), já redigido.
    pub text: String,
    pub at: u64,
}

/// Quem escreveu a nota por último e o texto que ficou (redigido).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteWritten {
    /// Título do nó do agente.
    pub by: String,
    pub at: u64,
    pub markdown: String,
}

/// Quem abriu a página do navegador e quando (ms).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOpened {
    /// Título do nó do agente, como o canvas o mostra.
    pub by: String,
    pub at: u64,
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
            opened: None,
            inbox: 0,
            inbox_from: HashMap::new(),
            sent: None,
            written: None,
            undelivered: None,
            attempt: 0,
            stale: false,
            stale_reason: None,
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
        // Plano e controle não levam valor: não seguram o destino.
        for edge in graph.edges.iter().filter(|e| e.kind.carries_data()) {
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
            settled: false,
            paused_ms: 0,
            created: Vec::new(),
            plans: Vec::new(),
            number: 0,
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

    /// A url impressa chegou depois da porta da varredura: ela vale (o
    /// arquivo em `outputs` já foi reescrito por quem chama). Quem já começou
    /// com a de antes fica "↻ para aplicar". Devolve `true` se mudou.
    pub fn replace_url(&mut self, id: &str, url: String) -> bool {
        let Some(n) = self.run.nodes.get_mut(id) else {
            return false;
        };
        if n.status != NodeStatus::Running
            || !n.outputs.contains_key(&EdgeKind::Url)
            || n.url.as_deref() == Some(url.as_str())
        {
            return false;
        }
        n.url = Some(url);
        for d in self.dependents.get(id).cloned().unwrap_or_default() {
            if let Some(dn) = self.run.nodes.get_mut(&d) {
                if dn.started_at.is_some() {
                    dn.mark_stale(StaleReason::Source);
                }
            }
        }
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
        let settled_at = self.settled_at();
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
        // Assentado, a sessão que sai agora (o agente parado) fecha no
        // momento em que o run assentou: o relógio não anda no tempo parado.
        n.finished_at = Some(settled_at.unwrap_or(now));
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
        let sources: Vec<&(String, EdgeKind)> =
            sources.iter().filter(|(_, k)| k.carries_data()).collect();
        for (from, kind) in sources.iter().copied() {
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

    /// Para um nó só (o `orbit_stop_node`): rodando vira `stopped`,
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
                // Parado pelo dono, não pulado por uma origem: rodar a origem
                // de novo não o traz de volta.
                n.reason = Some(STOPPED_BEFORE_START.into());
                None
            }
            _ => return None,
        };
        self.skip_dependents(id);
        self.settle(now);
        session
    }

    /// Quantas vezes o nó já foi reiniciado (ver [`NodeRun::attempt`]).
    pub fn attempt(&self, id: &str) -> Option<u32> {
        self.run.nodes.get(id).map(|n| n.attempt)
    }

    /// Reiniciar (nó rodando) ou rodar de novo (nó que terminou, falhou ou
    /// foi parado), com a mesma config: o nó volta a esperar as entradas,
    /// com a tentativa seguinte, e fica segurado até [`Scheduler::unhold`]
    /// (quem reinicia limpa a `out/` antes). Quem depende dele e já começou
    /// com o valor antigo fica "↻ para aplicar" ([`NodeRun::stale`]), sem
    /// reiniciar sozinho; quem foi pulado por causa dele volta a esperar.
    /// Devolve a sessão de antes, para matar (o agente vivo depois do turno
    /// inclusive).
    pub fn rerun(&mut self, id: &str) -> Result<Option<String>, String> {
        if self.run.status != RunStatus::Running {
            return Err("a execução já fechou: ▶ Executar de novo".into());
        }
        let n = self
            .run
            .nodes
            .get_mut(id)
            .ok_or_else(|| format!("nó não existe neste run: {id}"))?;
        match n.status {
            NodeStatus::Waiting => return Err("o nó ainda não começou".into()),
            NodeStatus::Skipped => {
                return Err(
                    "o nó foi pulado porque uma origem dele não terminou: rode de novo a origem"
                        .into(),
                )
            }
            NodeStatus::Running | NodeStatus::Done | NodeStatus::Failed | NodeStatus::Stopped => {}
        }
        let session = n.session_id.take();
        let mut fresh = NodeRun::waiting();
        fresh.attempt = n.attempt + 1;
        // A fila da conversa segue no core: a sessão nova a recebe.
        fresh.inbox = n.inbox;
        fresh.inbox_from = std::mem::take(&mut n.inbox_from);
        *n = fresh;
        self.held.insert(id.to_string());
        let dependents = self.dependents.get(id).cloned().unwrap_or_default();
        for d in &dependents {
            if let Some(dn) = self.run.nodes.get_mut(d) {
                if dn.started_at.is_some() {
                    dn.mark_stale(StaleReason::Source);
                }
            }
        }
        self.unskip_dependents(id);
        Ok(session)
    }

    /// Solta o nó segurado pelo [`Scheduler::rerun`].
    pub fn unhold(&mut self, id: &str) {
        self.held.remove(id);
    }

    /// A config do nó (ou uma variável que ele usa) mudou no grafo salvo com
    /// o run aberto: quem já começou fica "↻ para aplicar" com o motivo (a
    /// config nova vale quando o dono o reinicia); quem ainda espera começa
    /// com ela, sem marca. Devolve `true` se marcou ou trocou o motivo.
    pub fn mark_changed(&mut self, id: &str, reason: StaleReason) -> bool {
        match self.run.nodes.get_mut(id) {
            Some(n) if n.started_at.is_some() => n.mark_stale(reason),
            _ => false,
        }
    }

    /// As ligações de valor do grafo salvo valem na hora (o dono edita com o
    /// run aberto). Ligação com ponta fora do run não conta (nó novo só entra
    /// no próximo ▶). Quem ainda espera passa a esperar as origens novas;
    /// quem já começou e teve as entradas mudadas fica "↻ para aplicar".
    /// Esperando com origem morta é pulado; pulado em cascata cuja origem
    /// morta saiu volta a esperar. Devolve `true` se algo mudou.
    pub fn rewire(&mut self, edges: &[Edge]) -> bool {
        let mut sources: HashMap<String, Vec<(String, EdgeKind)>> = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        for e in edges.iter().filter(|e| e.kind.carries_data()) {
            if !self.run.nodes.contains_key(&e.from) || !self.run.nodes.contains_key(&e.to) {
                continue;
            }
            sources
                .entry(e.to.clone())
                .or_default()
                .push((e.from.clone(), e.kind));
            dependents
                .entry(e.from.clone())
                .or_default()
                .push(e.to.clone());
        }
        let same = |a: Option<&Vec<(String, EdgeKind)>>, b: Option<&Vec<(String, EdgeKind)>>| {
            let (a, b) = (
                a.map_or(&[][..], Vec::as_slice),
                b.map_or(&[][..], Vec::as_slice),
            );
            a.len() == b.len() && a.iter().all(|x| b.contains(x))
        };
        let mut changed = false;
        for (id, n) in self.run.nodes.iter_mut() {
            if same(self.sources.get(id), sources.get(id)) {
                continue;
            }
            changed = true;
            if n.started_at.is_some() {
                n.mark_stale(StaleReason::Edges);
            }
        }
        self.sources = sources;
        self.dependents = dependents;
        let dead = |s: &Self, id: &str| {
            s.sources.get(id).into_iter().flatten().any(|(from, _)| {
                matches!(
                    s.status(from),
                    Some(NodeStatus::Failed | NodeStatus::Skipped | NodeStatus::Stopped)
                )
            })
        };
        // Até assentar: pular um pode matar a origem de outro, e voltar um
        // pode ressuscitar a de outro (cada volta move ao menos um nível).
        for _ in 0..=self.order.len() {
            let mut moved = false;
            for id in self.order.clone() {
                let Some(status) = self.status(&id) else {
                    continue;
                };
                let skipped_by_source = status == NodeStatus::Skipped
                    && self.run.nodes[&id].reason.is_none()
                    && self.run.nodes[&id].started_at.is_none();
                let next = match status {
                    NodeStatus::Waiting if dead(self, &id) => NodeStatus::Skipped,
                    _ if skipped_by_source && !dead(self, &id) => NodeStatus::Waiting,
                    _ => continue,
                };
                if let Some(n) = self.run.nodes.get_mut(&id) {
                    n.status = next;
                }
                moved = true;
            }
            if !moved {
                break;
            }
            changed = true;
        }
        changed
    }

    /// Volta a esperar quem foi pulado em cascata por `id` e não tem mais
    /// origem morta (falhou, parou ou foi pulada). O pulado com motivo (o
    /// dono o parou antes de começar) fica.
    fn unskip_dependents(&mut self, id: &str) {
        let mut stack = vec![id.to_string()];
        while let Some(current) = stack.pop() {
            for next in self.dependents.get(&current).cloned().unwrap_or_default() {
                let skipped = self
                    .run
                    .nodes
                    .get(&next)
                    .is_some_and(|n| n.status == NodeStatus::Skipped && n.reason.is_none());
                let alive = self.sources.get(&next).into_iter().flatten().all(|(s, _)| {
                    !matches!(
                        self.status(s),
                        Some(NodeStatus::Failed | NodeStatus::Skipped | NodeStatus::Stopped)
                    )
                });
                if skipped && alive {
                    if let Some(n) = self.run.nodes.get_mut(&next) {
                        n.status = NodeStatus::Waiting;
                    }
                    stack.push(next);
                }
            }
        }
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

    /// Fecha o run quando não sobra nó esperando nem rodando. Assentado, o
    /// relógio fica onde parou.
    fn settle(&mut self, now: u64) {
        if self.run.status != RunStatus::Running
            || !self.run.nodes.values().all(|n| n.status.is_terminal())
        {
            return;
        }
        self.run.status = closed_status(&self.run.nodes);
        self.run.finished_at = Some(self.settled_at().unwrap_or(now));
        self.run.settled = false;
    }

    /// O momento em que o run assentou, se está assentado.
    fn settled_at(&self) -> Option<u64> {
        self.run.settled.then_some(self.run.finished_at).flatten()
    }

    /// Nada roda nem espera no grafo além de agentes que já fecharam o
    /// primeiro turno (os que seguram o run), e há ao menos um deles. O
    /// resto do "assentado" (turno fechado há um tempo, fila vazia) é do
    /// [`Runs`], que vê as sessões.
    fn only_live_agents(&self) -> bool {
        self.run.status == RunStatus::Running
            && self.run.nodes.values().all(|n| {
                n.status.is_terminal() || (n.status == NodeStatus::Running && n.orchestrating)
            })
            && self
                .run
                .nodes
                .values()
                .any(|n| n.status == NodeStatus::Running)
    }

    /// O run assentou (`true`) ou voltou a andar. Assentado, o relógio para
    /// em `now` (`finished_at` provisório); ao voltar, o tempo parado soma em
    /// `paused_ms` e o relógio retoma do tempo ativo. Devolve `true` se mudou.
    pub fn set_settled(&mut self, settled: bool, now: u64) -> bool {
        if self.run.status != RunStatus::Running || self.run.settled == settled {
            return false;
        }
        if settled {
            self.run.finished_at = Some(now);
        } else {
            let since = self.run.finished_at.take().unwrap_or(now);
            self.run.paused_ms += now.saturating_sub(since);
        }
        self.run.settled = settled;
        true
    }

    /// Parar: rodando vira `stopped`, esperando vira `skipped`. Devolve as
    /// sessões a matar.
    ///
    /// Assentado, o trabalho já acabou: o Parar só fecha as sessões, os
    /// agentes ficam prontos e o run, concluído, com o relógio onde parou.
    pub fn stop(&mut self, now: u64) -> Vec<String> {
        if self.run.status != RunStatus::Running {
            return Vec::new();
        }
        let settled_at = self.settled_at();
        let mut kill = Vec::new();
        for n in self.run.nodes.values_mut() {
            match n.status {
                NodeStatus::Running => {
                    if settled_at.is_some() {
                        n.status = NodeStatus::Done;
                        n.orchestrating = false;
                    } else {
                        n.status = NodeStatus::Stopped;
                    }
                    n.finished_at = Some(settled_at.unwrap_or(now));
                    if let Some(s) = &n.session_id {
                        kill.push(s.clone());
                    }
                }
                NodeStatus::Waiting => n.status = NodeStatus::Skipped,
                _ => {}
            }
        }
        self.run.status = match settled_at {
            Some(_) => closed_status(&self.run.nodes),
            None => RunStatus::Stopped,
        };
        self.run.finished_at = Some(settled_at.unwrap_or(now));
        self.run.settled = false;
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

/// [`node_label`] de um nó do run ativo (o criado por agente inclusive).
fn active_label(a: &Active, id: &str) -> String {
    a.graph
        .nodes
        .iter()
        .find(|n| n.id == id)
        .map_or_else(|| format!("nó {id}"), node_label)
}

/// O nó numa mensagem de erro: o título entre aspas (o id quando não há).
pub(super) fn node_label(node: &Node) -> String {
    let title = node.title.trim();
    if title.is_empty() {
        format!("nó {}", node.id)
    } else {
        format!("\"{title}\"")
    }
}

/// Variável usada (`{{nome}}`) em comando, script ou prompt sem valor no
/// grafo nem padrão no texto: o ▶ recusa, citando o nó — sem isto ela virava
/// texto vazio e o comando rodava pela metade.
pub fn missing_variable(graph: &Graph, integrations: &[&Integration]) -> Result<(), String> {
    for node in &graph.nodes {
        let integration = match &node.kind {
            NodeKind::Integration { integration_id, .. } => integrations
                .iter()
                .find(|i| &i.id == integration_id)
                .copied(),
            _ => None,
        };
        node_missing_variable(node, integration, &graph.variables)?;
    }
    Ok(())
}

/// O texto que o nó roda e onde entram as variáveis: o comando, o prompt ou
/// o script da integração (vazio sem ela). Os outros tipos não têm.
fn run_text<'a>(node: &'a Node, integration: Option<&'a Integration>) -> Option<&'a str> {
    match &node.kind {
        NodeKind::Process { command, .. } => Some(command),
        NodeKind::Agent { prompt, .. } => Some(prompt),
        NodeKind::Integration { .. } => Some(integration.map_or("", |i| i.script.as_str())),
        _ => None,
    }
}

/// [`missing_variable`] de um nó só, com as variáveis de agora: o ▶, o
/// reinício e o começo de cada nó usam a mesma régua.
fn node_missing_variable(
    node: &Node,
    integration: Option<&Integration>,
    variables: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    let Some(text) = run_text(node, integration) else {
        return Ok(());
    };
    // Processo sem comando viraria `( \n)` no wrapper, erro de sintaxe do
    // bash que o dono lê como falha do Orbit.
    if matches!(node.kind, NodeKind::Process { .. }) && text.trim().is_empty() {
        return Err(format!(
            "{}: falta o comando — escreva o que ele roda antes do ▶",
            node_label(node)
        ));
    }
    let empty = crate::snippet::placeholders(text).into_iter().find(|p| {
        p.default.as_deref().is_none_or(str::is_empty)
            && variables.get(&p.name).is_none_or(|v| v.trim().is_empty())
    });
    match empty {
        Some(p) => Err(format!(
            "{}: a variável {{{{{}}}}} está sem valor — preencha em Variáveis antes do ▶",
            node_label(node),
            p.name
        )),
        None => Ok(()),
    }
}

/// [`node_missing_variable`] de um nó do run ativo.
fn active_missing_variable(a: &Active, id: &str) -> Result<(), String> {
    let Some(node) = a.graph.nodes.iter().find(|n| n.id == id) else {
        return Ok(());
    };
    let integration = match &node.kind {
        NodeKind::Integration { integration_id, .. } => a.integrations.get(integration_id),
        _ => None,
    };
    node_missing_variable(node, integration, &a.graph.variables)
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
        // Não levam valor: nunca viram entrada (filtradas antes).
        EdgeKind::Plan => "plano.txt".into(),
        EdgeKind::Control => "controle.txt".into(),
        EdgeKind::Talk => "conversa.txt".into(),
        EdgeKind::Memory => "memoria.txt".into(),
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
/// assim o caminho não passa por aspas montadas à mão. O comando roda num
/// subshell — o `exit 3` dele não pula a gravação do `.exit` — direto no PTY
/// (sem `| tee`: cor e saída sem buffer de pipe; o `log.txt` o core grava do
/// fluxo). O `)` vai em linha própria para um comentário no fim do comando
/// não engolir o fechamento.
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
         set -o pipefail; ( {command}\n); echo $? > \"$TYBA_OUT/.exit\""
    );
    let mut args = vec![
        "-lc".into(),
        script,
        "tyba-orbit".into(),
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

/// O log inteiro de um nó, para a tela.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WholeLog {
    pub text: String,
    /// O começo ficou de fora (passou do teto da leitura).
    pub cut: bool,
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
    /// Todo nó de agente: as ferramentas do Orbit que o
    /// servidor MCP da sessão chama pelo socket de hooks.
    pub tools: Option<ToolHandler>,
}

/// Efeitos fora do agendador: subir processo, matar, avisar o front.
pub trait RunHost: Send + Sync + 'static {
    /// Sobe o agente livre e devolve o id da sessão. `on_turn` roda uma vez,
    /// quando o primeiro turno fecha (ou o agente sai antes disso). O agente
    /// segue vivo depois; `on_exit`, quando presente, roda quando a sessão
    /// sai depois do primeiro turno fechado.
    ///
    /// `on_state` roda a cada estado visto da sessão (trabalhando, esperando
    /// o dono, turno fechado), depois do `on_turn` quando o primeiro fecha:
    /// é por ele que a fila da conversa sai.
    fn spawn_agent(
        &self,
        launch: AgentLaunch,
        on_turn: Box<dyn FnOnce(AgentTurn) + Send>,
        on_exit: Option<Box<dyn FnOnce() + Send>>,
        on_state: Box<dyn Fn(PeerState) + Send>,
    ) -> Result<String, String>;
    /// Escreve `text` no terminal do agente e envia, pela estratégia do
    /// runner (colagem e Enter), como o input rico do dono.
    fn send_input(&self, session_id: &str, text: &str) -> Result<(), String>;
    /// A colagem de várias linhas chega inteira (bracketed paste ligado e
    /// usado pelo runner). Sem ela, a conversa vai numa linha só.
    fn multiline(&self, session_id: &str) -> bool;
    /// O que o dono anda digitando no terminal da sessão.
    fn owner_keys(&self, session_id: &str) -> talk::OwnerKeys;
    /// Sobe o processo num PTY e devolve o id da sessão. `tap` recebe o
    /// fluxo do pane (o `log.txt`) e é solto antes do `on_exit`, que roda
    /// quando o PTY fecha.
    fn spawn_process(
        &self,
        launch: ProcessLaunch,
        tap: crate::pty::OutputTap,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Result<String, String>;
    /// Mata o process group da sessão.
    fn kill(&self, session_id: &str);
    /// Portas em LISTEN do process group da sessão, agora (varredura curta,
    /// com teto de tempo; bloqueia — fora da main thread).
    fn session_ports(&self, session_id: &str) -> Vec<u16>;
    /// Acha o serviço e espera ficar pronto (bloqueia). `Err` diz por que não
    /// achou o alvo.
    fn resolve_service(&self, source: &ServiceSource) -> Result<ServiceReady, String>;
    /// A sessão ainda roda (não saiu nem foi morta).
    fn alive(&self, session_id: &str) -> bool;
    /// `orbit://run/<run_id>` com o run inteiro.
    fn emit(&self, run: &Run);
    /// Uma ferramenta de agente na página do navegador `node_id` do grafo
    /// `graph_id` (a permissão já conferida); `by` é o título do agente,
    /// para o card. Bloqueia até a página responder.
    fn browser(
        &self,
        graph_id: &str,
        node_id: &str,
        by: &str,
        action: AgentAction,
    ) -> Result<serde_json::Value, String>;
}

struct Active {
    graph: Graph,
    /// Integrações do grafo como estavam no ▶ Executar.
    integrations: HashMap<String, Integration>,
    sched: Scheduler,
    dir: PathBuf,
    /// Filas da conversa entre agentes deste run.
    talk: Mailbox,
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
    /// Nó criado por outro agente neste run (`run.created`).
    created: bool,
    /// A tentativa do nó ao subir ([`NodeRun::attempt`]).
    attempt: u32,
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
    /// Quanto tempo de tudo quieto assenta o run ([`SETTLE_MS`]; o teste
    /// encurta).
    settle_ms: std::sync::atomic::AtomicU64,
    me: Weak<Runs<H>>,
}

/// Quanto todo agente fica com o turno fechado, sem mais nada acontecendo,
/// para o run assentar ([`Run::settled`]).
pub const SETTLE_MS: u64 = 3_000;

/// Assenta o run quando nada acontece, ou o tira do assentado quando algo
/// volta a acontecer (ver [`Run::settled`]): só agentes que seguram o run,
/// cada sessão de agente viva com o turno fechado há `after` ms, e nenhuma
/// mensagem na fila. `live`: as sessões de agente do run que ainda rodam.
/// Devolve `true` se mudou.
fn refresh_settled(a: &mut Active, live: &[String], now: u64, after: u64) -> bool {
    let quiet = a.sched.only_live_agents()
        && !a.talk.any_waiting()
        && a.sched.run.nodes.iter().all(|(id, n)| {
            let live_agent = n.session_id.as_ref().is_some_and(|s| live.contains(s));
            let needs_idle = n.status == NodeStatus::Running || live_agent;
            !needs_idle || a.talk.idle_for(id, now).is_some_and(|t| t >= after)
        });
    a.sched.set_settled(quiet, now)
}

/// A thread que sobe os nós de um run que acabou de começar.
type Launching = std::thread::JoinHandle<()>;

struct RunAgents {
    graph_id: String,
    sessions: Vec<String>,
}

fn now() -> u64 {
    crate::approvals::now_ms()
}

/// A varredura do boot (ver [`Runs::new`]).
fn close_interrupted_runs(store: &Store) {
    let open = match store.list_running_orbit_runs() {
        Ok(runs) => runs,
        Err(e) => {
            eprintln!("[orbit] ler execuções abertas: {e}");
            return;
        }
    };
    let t = now();
    for mut run in open {
        run.close_interrupted(t);
        if let Err(e) = store.save_orbit_run(&run) {
            eprintln!("[orbit] fechar execução {}: {e}", run.id);
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
            settle_ms: std::sync::atomic::AtomicU64::new(SETTLE_MS),
            me: me.clone(),
        })
    }

    /// Começa o run. O grafo já passou por consentimento e validação. Antes,
    /// para os runs anteriores do mesmo grafo que ainda têm agente: um ▶ novo
    /// não deixa o claude do run de antes vivo e fora de alcance.
    ///
    /// Volta com o run já publicado; os nós sobem em outra thread.
    pub fn start(&self, graph: Graph, integrations: Vec<Integration>) -> Result<Run, String> {
        self.start_launching(graph, integrations)
            .map(|(run, _)| run)
    }

    /// [`Runs::start`] com a thread que sobe os nós.
    fn start_launching(
        &self,
        graph: Graph,
        integrations: Vec<Integration>,
    ) -> Result<(Run, Launching), String> {
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
        self.test_integration_launching(graph, integration, node_id)
            .map(|(run, _)| run)
    }

    fn test_integration_launching(
        &self,
        graph: Graph,
        integration: Integration,
        node_id: &str,
    ) -> Result<(Run, Launching), String> {
        let root = self
            .root
            .clone()
            .ok_or("sem pasta home para guardar as execuções")?;
        let previous = self
            .store
            .list_orbit_runs(&graph.id, KEEP_RUNS)
            .map_err(|e| e.to_string())?;
        let seed = super::integration::last_inputs(&previous, &root, node_id).ok_or(
            "este nó ainda não rodou: execute o grafo uma vez antes de testar com a última entrada",
        )?;
        let single = super::integration::test_graph(&graph, node_id, &integration.id)?;
        self.begin(single, vec![integration], Some(seed))
    }

    /// O ▶ num nó de integração manual de um run rodando.
    pub fn run_node(&self, run_id: &str, node_id: &str) -> Result<(), String> {
        {
            let mut active = self.active.lock();
            let a = active.get_mut(run_id).ok_or("execução não está rodando")?;
            a.sched
                .release(node_id)
                .map_err(|e| format!("{}: {e}", active_label(a, node_id)))?;
        }
        self.advance(run_id);
        Ok(())
    }

    /// `seed`: `in/` a copiar para o único nó (teste de integração). No teste
    /// o nó manual não espera ▶ — o clique em Testar já é o ▶.
    fn begin(
        &self,
        mut graph: Graph,
        integrations: Vec<Integration>,
        seed: Option<PathBuf>,
    ) -> Result<(Run, Launching), String> {
        if let Some(bad) = graph.nodes.iter().find(|n| !safe_component(&n.id)) {
            return Err(format!(
                "{}: id de nó inválido para execução ({:?}; use letras, números, - e _)",
                node_label(bad),
                bad.id
            ));
        }
        let integration_list: Vec<&Integration> = integrations.iter().collect();
        missing_variable(&graph, &integration_list)?;
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
        sched.run.number = self.next_number(&graph.id);
        if seed.is_none() {
            for id in super::integration::manual_nodes(&graph, &integrations) {
                sched.hold(&id);
            }
            sched.run.plans = self.reset_plans(&mut graph);
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
                    talk: Mailbox::default(),
                },
            );
        }
        self.publish(snapshot.clone());
        // O clique volta com o run já rodando; subir os nós (git, PTY, agente)
        // e podar o histórico vão para outra thread.
        let me = self.me.clone();
        let launching = std::thread::spawn(move || {
            if let Some(runs) = me.upgrade() {
                runs.prune(&graph_id, KEEP_RUNS);
                runs.advance(&run_id);
            }
        });
        Ok((snapshot, launching))
    }

    /// O "#N" da execução nova: um a mais que a mais nova gravada. As de
    /// antes da numeração (0) contam pela posição, como a tela as mostrava.
    fn next_number(&self, graph_id: &str) -> u32 {
        let previous = self
            .store
            .list_orbit_runs(graph_id, KEEP_RUNS)
            .unwrap_or_default();
        let top = previous.iter().map(|r| r.number).max().unwrap_or(0);
        top.max(previous.len() as u32) + 1
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
                let mut run = self.store.get_orbit_run(run_id).ok().flatten()?;
                if run.status == RunStatus::Running {
                    run.status = if run.settled {
                        RunStatus::Done
                    } else {
                        RunStatus::Stopped
                    };
                    run.settled = false;
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
            .list_orbit_runs(graph_id, KEEP_RUNS)
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
        if let Err(e) = self.store.save_orbit_run(&run) {
            eprintln!("[orbit] gravar execução {}: {e}", run.id);
        }
        self.host.emit(&run);
    }

    /// Retrato novo, gravado e avisado. Run fechado só sai da memória depois
    /// de gravado: antes disso o `get` leria do banco um retrato mais velho.
    fn publish_current(&self, run_id: &str) {
        let live = self.live_agents(run_id);
        let after = self.settle_ms();
        let Some(run) = self.active.lock().get_mut(run_id).map(|a| {
            refresh_settled(a, &live, now(), after);
            a.sched.snapshot()
        }) else {
            return;
        };
        let closed = run.status != RunStatus::Running;
        self.publish(run);
        if closed {
            self.active.lock().remove(run_id);
        }
    }

    fn settle_ms(&self) -> u64 {
        self.settle_ms.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// O dono apagou o grafo: as execuções dele saem junto (linha e pasta).
    /// Com execução aberta ou agente vivo, recusa: pare antes.
    pub fn forget_graph(&self, graph_id: &str) -> Result<(), String> {
        let busy = self.active.lock().values().any(|a| a.graph.id == graph_id) || {
            let with_agents: Vec<String> = self
                .agents
                .lock()
                .iter()
                .filter(|(_, a)| a.graph_id == graph_id)
                .map(|(id, _)| id.clone())
                .collect();
            with_agents
                .iter()
                .any(|id| !self.live_agents(id).is_empty())
        };
        if busy {
            return Err("pare a execução do grafo antes de apagá-lo".into());
        }
        self.prune(graph_id, 0);
        Ok(())
    }

    /// Poda o histórico do grafo às `keep` mais recentes. Run rodando ou com
    /// agente vivo fica, mesmo fora da janela: a pasta dele está em uso.
    fn prune(&self, graph_id: &str, keep: usize) {
        let Some(root) = &self.root else { return };
        let mut protect: HashSet<String> = self.active.lock().keys().cloned().collect();
        let with_agents: Vec<String> = self.agents.lock().keys().cloned().collect();
        for id in with_agents {
            if !self.live_agents(&id).is_empty() {
                protect.insert(id);
            }
        }
        let removed = match self.store.prune_orbit_runs(graph_id, keep, &protect) {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!("[orbit] podar execuções: {e}");
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
            let (starts, refused) = {
                let mut active = self.active.lock();
                let Some(a) = active.get_mut(run_id) else {
                    return;
                };
                let ready = a.sched.ready();
                let t = now();
                let mut refused = false;
                let starts = ready
                    .iter()
                    .filter_map(|id| {
                        a.sched.mark_running(id, t);
                        // Variável que ficou sem valor depois do ▶ (o dono
                        // editou com o run aberto): falha com o motivo, em vez
                        // de rodar o comando com o buraco.
                        if let Err(e) = active_missing_variable(a, id) {
                            a.sched.finish(id, Outcome::failed(e), t);
                            refused = true;
                            return None;
                        }
                        prepare_start(a, id)
                    })
                    .collect::<Vec<_>>();
                (starts, refused)
            };
            if starts.is_empty() {
                if refused {
                    self.publish_current(run_id);
                }
                return;
            }
            self.publish_current(run_id);
            // Em paralelo: cada nó custa git (snapshot do diff), PTY e o
            // agente; em fila, o último esperava a soma de todos.
            let go_on = std::thread::scope(|s| {
                let launches: Vec<_> = starts
                    .into_iter()
                    .map(|start| s.spawn(move || self.launch_and_record(run_id, start)))
                    .collect();
                // Espera todos (sem parar no primeiro `false`).
                let results: Vec<bool> = launches
                    .into_iter()
                    .map(|l| l.join().unwrap_or(false))
                    .collect();
                results.into_iter().all(|go_on| go_on)
            });
            if !go_on {
                return;
            }
        }
    }

    /// Sobe um nó e registra o que voltou. `false`: o run já não está ativo
    /// e não há mais o que subir.
    fn launch_and_record(&self, run_id: &str, start: Start) -> bool {
        let node_id = start.node.id.clone();
        let graph_id = start.graph_id.clone();
        let is_agent = matches!(start.node.kind, NodeKind::Agent { .. });
        let attempt = start.attempt;
        let launched = self.launch(run_id, start);
        // `active` na mão para decidir: o Parar marca `stopped` sob o
        // mesmo lock, então ou ele já viu esta sessão no run, ou ela
        // morre aqui.
        let orphan = {
            let mut active = self.active.lock();
            match (active.get_mut(run_id), launched) {
                // Reiniciado enquanto subia: a sessão desta tentativa sobra.
                (Some(a), Launched::Session(session))
                    if a.sched.attempt(&node_id) != Some(attempt) =>
                {
                    Some(session)
                }
                (Some(a), Launched::Finished(_)) if a.sched.attempt(&node_id) != Some(attempt) => {
                    None
                }
                (Some(a), Launched::Session(session)) => {
                    a.sched.set_session(&node_id, session.clone());
                    if is_agent {
                        self.register_agent(run_id, &graph_id, session);
                    }
                    None
                }
                (Some(_), Launched::Pending) => None,
                (None, Launched::Pending) => return false,
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
                        return false;
                    }
                }
                (None, Launched::Finished(_)) => return false,
            }
        };
        if let Some(session) = orphan {
            self.host.kill(&session);
            // Morta pelo reinício, o run segue; pelo Parar, acabou.
            return self.active.lock().contains_key(run_id);
        }
        self.publish_current(run_id);
        true
    }

    fn launch(&self, run_id: &str, start: Start) -> Launched {
        match &start.node.kind {
            NodeKind::Note { markdown, .. } => {
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
                Some(i) => self.launch_script(run_id, &start, &i.script, i.output, false),
                None => Launched::Finished(Outcome::failed("integração não encontrada")),
            },
            NodeKind::Agent { runner, prompt, .. } => {
                self.launch_agent(run_id, &start, *runner, prompt)
            }
            NodeKind::Process {
                command, output, ..
            } => self.launch_script(run_id, &start, command, *output, true),
            NodeKind::Browser { url } => Launched::Finished(browser_outcome(&start, url)),
            NodeKind::Service { source } => self.launch_service(run_id, &start, source.clone()),
        }
    }

    /// O serviço em outra thread: achar o contêiner e ler o primeiro lote de
    /// log pode levar segundos, e o resto do grafo não espera.
    fn launch_service(&self, run_id: &str, start: &Start, source: ServiceSource) -> Launched {
        let me = self.me.clone();
        let (run, node, out_dir, attempt) = (
            run_id.to_string(),
            start.node.id.clone(),
            start.out_dir.clone(),
            start.attempt,
        );
        std::thread::spawn(move || {
            let Some(runs) = me.upgrade() else { return };
            let outcome = match runs.host.resolve_service(&source) {
                Ok(ready) => service_outcome(&out_dir, ready),
                Err(reason) => Outcome::failed(reason),
            };
            runs.finish_later(&run, &node, attempt, outcome);
        });
        Launched::Pending
    }

    /// Fecha um nó que terminou fora do `advance` e segue o grafo.
    fn finish_later(&self, run_id: &str, node_id: &str, attempt: u32, outcome: Outcome) {
        {
            let mut active = self.active.lock();
            let Some(a) = active.get_mut(run_id) else {
                return;
            };
            if a.sched.attempt(node_id) != Some(attempt) || !a.sched.finish(node_id, outcome, now())
            {
                return;
            }
        }
        self.publish_current(run_id);
        self.advance(run_id);
    }

    /// A url do processo rodando: primeiro a que ele imprime
    /// ([`super::log::printed_url`], chega por `printed`), depois a porta da
    /// varredura do process group a cada [`PORT_POLL`]
    /// ([`super::log::PortPick`]). Achou: grava `out/url.txt`, publica a url
    /// com o processo ainda rodando e segue o grafo (quem espera a url
    /// começa). Publicada pela varredura, a impressa que vier depois ainda a
    /// troca. Para quando o nó deixa de rodar ou o PTY fecha.
    fn watch_ports(
        &self,
        run_id: &str,
        node_id: &str,
        session: &str,
        out_dir: PathBuf,
        attempt: u32,
        printed: std::sync::mpsc::Receiver<String>,
    ) {
        use std::sync::mpsc::RecvTimeoutError;
        let me = self.me.clone();
        let (run, node, session) = (run_id.to_string(), node_id.to_string(), session.to_string());
        std::thread::spawn(move || {
            let mut pick = super::log::PortPick::default();
            let mut from_scan = false;
            loop {
                let url = match printed.recv_timeout(PORT_POLL) {
                    Ok(url) => Some((url, true)),
                    Err(RecvTimeoutError::Timeout) => None,
                    // O PTY fechou: o fim do processo segue por `process_exited`.
                    Err(RecvTimeoutError::Disconnected) => return,
                };
                let Some(runs) = me.upgrade() else { return };
                let running = runs.active.lock().get(&run).is_some_and(|a| {
                    a.sched.status(&node) == Some(NodeStatus::Running)
                        && a.sched.attempt(&node) == Some(attempt)
                });
                if !running {
                    return;
                }
                let url = match url {
                    Some(found) => Some(found),
                    None if from_scan => None,
                    None => pick
                        .observe(&runs.host.session_ports(&session))
                        .map(|port| (service::local_url(port), false)),
                };
                let Some((url, is_printed)) = url else {
                    continue;
                };
                let path = out_dir.join("url.txt");
                if std::fs::write(&path, &url).is_err() {
                    return;
                }
                let changed = runs.active.lock().get_mut(&run).is_some_and(|a| {
                    if a.sched.attempt(&node) != Some(attempt) {
                        return false;
                    }
                    if from_scan {
                        a.sched.replace_url(&node, url)
                    } else {
                        a.sched
                            .publish_url(&node, path.to_string_lossy().into_owned(), url)
                    }
                });
                if changed {
                    runs.publish_current(&run);
                    runs.advance(&run);
                }
                if is_printed {
                    return;
                }
                from_scan = true;
            }
        });
    }

    /// Processo e integração: o texto (com variáveis) no wrapper de rules,
    /// rodado em `bash -lc` com o PATH de login do dono, num PTY, na pasta do nó.
    ///
    /// O comando roda no PTY de verdade (cor, sem buffer de pipe); o
    /// `log.txt` sai do fluxo ([`super::log::LogWriter`]). Processo (`watch`)
    /// também publica a url com ele rodando ([`Runs::watch_ports`]).
    fn launch_script(
        &self,
        run_id: &str,
        start: &Start,
        command: &str,
        output: OutputKind,
        watch: bool,
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
        // A config com que esta tentativa subiu: o dono pode editar o nó
        // com o processo rodando, e o fim dele lê o que ele rodou de fato.
        let config = ExitConfig {
            out_dir: start.out_dir.clone(),
            node_dir: start.node_dir.clone(),
            declared: match &start.node.kind {
                NodeKind::Process { output, file, .. } => Some((*output, file.clone())),
                NodeKind::Integration { .. } => start
                    .integration
                    .as_ref()
                    .map(|i| (i.output, i.file.clone())),
                _ => None,
            },
            command: match &start.node.kind {
                NodeKind::Process { command, .. } => command.clone(),
                _ => String::new(),
            },
            title: start.node.title.clone(),
            snapshot,
        };
        let me = self.me.clone();
        let (run, node, attempt) = (run_id.to_string(), start.node.id.clone(), start.attempt);
        let on_exit = Box::new(move || {
            if let Some(runs) = me.upgrade() {
                runs.process_exited(&run, &node, attempt, &config);
            }
        });
        let launch = ProcessLaunch {
            title: start.node.title.clone(),
            cwd: start.node_dir.clone(),
            program,
            args,
        };
        let log = match super::log::LogWriter::create(
            &start.out_dir.join("log.txt"),
            super::log::LOG_MAX_BYTES,
        ) {
            Ok(log) => log,
            Err(e) => return Launched::Finished(Outcome::failed(format!("criar o log: {e}"))),
        };
        // A primeira url local impressa vai ao vigia; depois disso, nada.
        let (tx, printed) = std::sync::mpsc::channel::<String>();
        let mut tx = watch.then_some(tx);
        let mut log = log.on_line(move |line| {
            if let Some(url) = tx.as_ref().and_then(|_| super::log::printed_url(line)) {
                if let Some(tx) = tx.take() {
                    let _ = tx.send(url);
                }
            }
        });
        let tap: crate::pty::OutputTap = Box::new(move |bytes| log.feed(bytes));
        match self.host.spawn_process(launch, tap, on_exit) {
            Ok(session) => {
                if watch {
                    self.watch_ports(
                        run_id,
                        &start.node.id,
                        &session,
                        start.out_dir.clone(),
                        start.attempt,
                        printed,
                    );
                }
                Launched::Session(session)
            }
            Err(e) => Launched::Finished(Outcome::failed(format!("subir o processo: {e}"))),
        }
    }

    /// Entradas em `in/`, snapshot de diff na largada e o agente livre na
    /// pasta do nó, com as ferramentas do Orbit. O agente do grafo do dono
    /// segura o nó (e o run) até a sessão sair, como o antigo orquestrador:
    /// publica as saídas no fim do primeiro turno e segue podendo criar nós
    /// e reescrever o plano. O criado por outro agente fecha no fim do turno
    /// e não tem as `orbit_create_*` (sem cascata).
    fn launch_agent(&self, run_id: &str, start: &Start, runner: Runner, prompt: &str) -> Launched {
        let holds = !start.created;
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
        let (run, node, out_dir, attempt) = (
            run_id.to_string(),
            start.node.id.clone(),
            start.out_dir.clone(),
            start.attempt,
        );
        let on_turn = {
            let (me, run, node) = (me.clone(), run.clone(), node.clone());
            Box::new(move |turn: AgentTurn| {
                if let Some(runs) = me.upgrade() {
                    let end = TurnEnd {
                        out_dir: &out_dir,
                        snapshot: snapshot.as_ref(),
                        holds,
                        attempt,
                    };
                    runs.agent_turn_ended(&run, &node, end, turn);
                }
            })
        };
        let on_state: Box<dyn Fn(PeerState) + Send> = {
            let (me, run, node) = (me.clone(), run.clone(), node.clone());
            Box::new(move |state| {
                if let Some(runs) = me.upgrade() {
                    runs.agent_state(&run, &node, attempt, state);
                }
            })
        };
        // Toda sessão de agente avisa a saída: o orquestrador segura o nó (e o
        // run) até ela; a de qualquer agente sai de `live_agents`.
        let on_exit: Box<dyn FnOnce() + Send> = Box::new(move || {
            if let Some(runs) = me.upgrade() {
                runs.agent_exited(&run, &node, holds, attempt);
            }
        });
        let no_create = matches!(
            start.node.kind,
            NodeKind::Agent {
                no_create: true,
                ..
            }
        );
        let tools = Some(self.tool_handler(run_id, &start.node.id, !start.created && !no_create));
        let launch = AgentLaunch {
            title: start.node.title.clone(),
            cwd: start.node_dir.clone(),
            runner,
            prompt,
            in_dir: start.in_dir.clone(),
            out_dir: start.out_dir.clone(),
            tools,
        };
        match self
            .host
            .spawn_agent(launch, on_turn, Some(on_exit), on_state)
        {
            Ok(session) => Launched::Session(session),
            Err(e) => Launched::Finished(Outcome::failed(format!("subir o agente: {e}"))),
        }
    }

    /// As ferramentas do agente `node_id` neste run. Fraco: a sessão que
    /// sobrevive ao app não segura o motor.
    fn tool_handler(&self, run_id: &str, node_id: &str, can_create: bool) -> ToolHandler {
        let me = self.me.clone();
        let (run, node) = (run_id.to_string(), node_id.to_string());
        ToolHandler {
            call: Arc::new(move |tool, args| {
                let runs = me.upgrade().ok_or("o TYBA está fechando")?;
                runs.orchestrator_call(&run, &node, tool, args)
            }),
            can_create,
        }
    }

    /// Uma ferramenta do Orbit pedida pela sessão do orquestrador
    /// `orchestrator` do run `run_id`.
    pub fn orchestrator_call(
        &self,
        run_id: &str,
        orchestrator: &str,
        tool: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        // Sem cascata: o agente criado por outro não cria nós, nem o que o
        // dono marcou "Não criar nós" — anuncie o `tools/list` dele o que
        // anunciar.
        if orchestrator::is_create_tool(tool) {
            let refused = self.active.lock().get(run_id).and_then(|a| {
                if is_created(a, orchestrator) {
                    Some("agente criado por outro agente não cria nós (sem cascata)")
                } else if no_create(a, orchestrator) {
                    Some("o dono marcou \"Não criar nós\" neste agente")
                } else {
                    None
                }
            });
            if let Some(why) = refused {
                return Err(format!("{tool}: {why}"));
            }
        }
        let call = orchestrator::parse_call(tool, args)?;
        eprintln!("[orbit] agente {orchestrator}: {tool}");
        match call {
            ToolCall::Create {
                title,
                kind,
                from_node,
            } => {
                let process = matches!(kind, NodeKind::Process { .. });
                let id = self.create_node(run_id, orchestrator, title, kind, from_node)?;
                if process {
                    return Ok(self.settle_process(run_id, &id));
                }
                Ok(serde_json::json!({ "node_id": id }))
            }
            ToolCall::Status { node_id } => self.node_status(run_id, orchestrator, &node_id),
            ToolCall::Stop { node_id } => {
                self.stop_created(run_id, orchestrator, &node_id)?;
                Ok(serde_json::json!({ "node_id": node_id, "stopped": true }))
            }
            ToolCall::Restart { node } => self.restart_by_agent(run_id, orchestrator, &node),
            ToolCall::Plan { steps } => {
                let id = self.write_plan(run_id, orchestrator, steps)?;
                Ok(serde_json::json!({ "note_id": id }))
            }
            ToolCall::BrowserOpen { node, url } => {
                let id = self.open_browser(run_id, orchestrator, node.as_deref(), &url)?;
                Ok(serde_json::json!({ "node_id": id, "url": url }))
            }
            ToolCall::Browser { node, action } => {
                self.browser_action(run_id, orchestrator, node.as_deref(), action)
            }
            ToolCall::Peers => self.peers(run_id, orchestrator),
            ToolCall::Finish => self.finish_agent(run_id, orchestrator),
            ToolCall::Send { to, text } => {
                let waiting = self.send_message(run_id, orchestrator, &to, &text)?;
                Ok(serde_json::json!({ "to": to, "queued": waiting }))
            }
            ToolCall::NoteRead { node } => self.note_read(run_id, orchestrator, &node),
            ToolCall::NoteWrite { node, text, append } => {
                let chars = self.note_write(run_id, orchestrator, &node, &text, append)?;
                Ok(serde_json::json!({ "node": node, "chars": chars }))
            }
        }
    }

    /// O processo recém-criado nos primeiros segundos. Caiu antes de ficar
    /// pronto: o motivo (o mesmo do rodapé do card) e o fim do log, redigido,
    /// para o agente reagir — trocar de porta, parar o nó quebrado. Abriu
    /// porta: a url. Senão, o estado de agora. Espera fora de qualquer lock.
    fn settle_process(&self, run_id: &str, id: &str) -> serde_json::Value {
        let deadline = std::time::Instant::now() + PROCESS_SETTLE;
        loop {
            let Some(n) = self.get(run_id).and_then(|r| r.nodes.get(id).cloned()) else {
                return serde_json::json!({ "node_id": id });
            };
            if n.status == NodeStatus::Failed {
                let log = self.root.as_ref().and_then(|root| {
                    let path = root.join(run_id).join(id).join("out").join("log.txt");
                    orchestrator::log_tail(&path, SETTLE_LOG_LINES)
                });
                return serde_json::json!({
                    "node_id": id,
                    "status": n.status,
                    "failed": true,
                    "exit_code": n.exit_code,
                    "reason": n.reason,
                    "log": log,
                    "next": "o processo caiu antes de ficar pronto: pare este nó com \
                             orbit_stop_node, corrija (outra porta, por exemplo) e crie \
                             um novo; navegador ligado a este nó, religue ao novo",
                });
            }
            let settled = n.url.is_some() || n.status.is_terminal();
            if settled || std::time::Instant::now() >= deadline {
                return serde_json::json!({
                    "node_id": id,
                    "status": n.status,
                    "url": n.url,
                    "exit_code": n.exit_code,
                });
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// As últimas linhas do `log.txt` de um nó do run (sem cor, redigidas):
    /// o card do processo que já terminou não tem mais sessão de onde ler.
    pub fn node_log(
        &self,
        run_id: &str,
        node_id: &str,
        lines: usize,
    ) -> Result<Vec<String>, String> {
        let run = self.get(run_id).ok_or("execução não encontrada")?;
        if !run.nodes.contains_key(node_id) {
            return Err(format!("nó não existe neste run: {node_id}"));
        }
        let root = self.root.as_ref().ok_or("sem pasta para as execuções")?;
        let path = root.join(run_id).join(node_id).join("out").join("log.txt");
        Ok(orchestrator::log_tail(&path, lines.min(NODE_LOG_MAX_LINES))
            .map(|t| t.lines().map(String::from).collect())
            .unwrap_or_default())
    }

    /// "Ver log inteiro": o `log.txt` do nó (com o rotacionado antes), sem
    /// cor e redigido, até [`super::log::LOG_VIEW_MAX_BYTES`] do fim. `cut`
    /// diz que o começo ficou de fora.
    pub fn node_log_whole(&self, run_id: &str, node_id: &str) -> Result<WholeLog, String> {
        let run = self.get(run_id).ok_or("execução não encontrada")?;
        if !run.nodes.contains_key(node_id) {
            return Err(format!("nó não existe neste run: {node_id}"));
        }
        let root = self.root.as_ref().ok_or("sem pasta para as execuções")?;
        let path = root.join(run_id).join(node_id).join("out").join("log.txt");
        let (text, cut) =
            super::log::read_whole(&path, super::log::LOG_VIEW_MAX_BYTES).unwrap_or_default();
        Ok(WholeLog { text, cut })
    }

    /// Os agentes ligados a `agent` por `talk` (nos dois sentidos), com o
    /// estado de cada um e quantas mensagens esperam por ele.
    pub fn peers(&self, run_id: &str, agent: &str) -> Result<serde_json::Value, String> {
        let active = self.active.lock();
        let a = active
            .get(run_id)
            .ok_or("esta execução já terminou: ▶ Executar de novo para conversar")?;
        let saved = self.saved_graph(&a.graph.id)?;
        let edges = current_edges(saved.as_ref(), &a.sched.run.created);
        let peers: Vec<serde_json::Value> = talk_peers(&edges, agent)
            .into_iter()
            .filter_map(|id| a.graph.nodes.iter().find(|n| n.id == id))
            .map(|n| {
                let run = a.sched.run.nodes.get(&n.id);
                let state = match (run.map(|r| r.status), a.talk.state(&n.id)) {
                    (Some(NodeStatus::Running), Some(s)) => serde_json::to_value(s).ok(),
                    (Some(NodeStatus::Running), None) => Some("working".into()),
                    (Some(NodeStatus::Waiting), _) => Some("notStarted".into()),
                    _ => Some("gone".into()),
                };
                serde_json::json!({
                    "id": n.id,
                    "title": n.title,
                    "state": state,
                    "queued": a.talk.waiting(&n.id),
                })
            })
            .collect();
        Ok(serde_json::json!({ "peers": peers }))
    }

    /// `orbit_send`: só para um agente ligado por `talk`. O texto (já pela
    /// régua de [`talk::message_text`]) vai para a fila do destino e sai
    /// quando ele fechar o turno ([`Self::agent_state`]). Devolve quantas
    /// esperam por ele.
    pub fn send_message(
        &self,
        run_id: &str,
        agent: &str,
        to: &str,
        text: &str,
    ) -> Result<usize, String> {
        let mut active = self.active.lock();
        let a = active
            .get_mut(run_id)
            .ok_or("esta execução já terminou: ▶ Executar de novo para conversar")?;
        let saved = self.saved_graph(&a.graph.id)?;
        let edges = current_edges(saved.as_ref(), &a.sched.run.created);
        if !talk_peers(&edges, agent).iter().any(|p| p == to) {
            return Err(format!(
                "{to} não é um agente ligado a você por conversa: só fala com quem o dono \
                 ligou a você no canvas (veja orbit_peers)"
            ));
        }
        let gone = a
            .sched
            .run
            .nodes
            .get(to)
            .is_none_or(|n| n.status.is_terminal());
        if gone {
            return Err(format!(
                "{to} não está mais rodando: a mensagem não teria quem leia"
            ));
        }
        let title = |id: &str| {
            a.graph
                .nodes
                .iter()
                .find(|n| n.id == id)
                .map_or_else(|| id.to_string(), |n| n.title.clone())
        };
        let (from, to_title) = (title(agent), title(to));
        let waiting = a.talk.push(
            to,
            talk::Message {
                sender: agent.to_string(),
                from,
                text: text.to_string(),
            },
        )?;
        let at = now();
        let from_each = a.talk.waiting_from(to);
        if let Some(n) = a.sched.run.nodes.get_mut(to) {
            n.inbox = waiting;
            n.inbox_from = from_each;
        }
        if let Some(n) = a.sched.run.nodes.get_mut(agent) {
            n.sent = Some(SentMessage {
                to: to_title,
                text: talk::preview(text),
                at,
            });
        }
        drop(active);
        self.publish_current(run_id);
        Ok(waiting)
    }

    /// O estado visto da sessão do agente `node_id`. Turno fechado com fila e
    /// o dono sem nada por enviar no terminal dele: o bloco marcado vai para
    /// o terminal, fora do lock (numa linha só se a sessão não cola várias).
    /// A fila só esvazia com o envio Ok; com erro, as mensagens voltam a
    /// esperar e o retrato leva o motivo (`undelivered`).
    ///
    /// Sem entrega, é também o relógio do assentado: o watcher avisa o
    /// estado a cada poll, e o run assenta (ou volta) por aqui.
    fn agent_state(&self, run_id: &str, node_id: &str, attempt: u32, state: PeerState) {
        let live = self.live_agents(run_id);
        let (session, delivery) = {
            let mut active = self.active.lock();
            let Some(a) = active.get_mut(run_id) else {
                return;
            };
            // A sessão velha de um nó reiniciado não fala pela nova.
            if a.sched.attempt(node_id) != Some(attempt) {
                return;
            }
            let session = a
                .sched
                .run
                .nodes
                .get(node_id)
                .and_then(|n| n.session_id.clone());
            let at = now();
            let quiet = state != PeerState::Ready
                || session
                    .as_deref()
                    .is_none_or(|s| self.host.owner_keys(s).quiet(at));
            let Some(delivery) = a.talk.on_state(node_id, state, at, quiet) else {
                let changed = refresh_settled(a, &live, at, self.settle_ms());
                drop(active);
                if changed {
                    self.publish_current(run_id);
                }
                return;
            };
            (session, delivery)
        };
        let sent = match &session {
            Some(session) => {
                let text = if self.host.multiline(session) {
                    delivery.block.clone()
                } else {
                    talk::one_line(&delivery.block)
                };
                self.host.send_input(session, &text)
            }
            None => Err("sem sessão".to_string()),
        };
        {
            let mut active = self.active.lock();
            let Some(a) = active.get_mut(run_id) else {
                return;
            };
            let undelivered = match sent {
                Ok(()) => {
                    a.talk.delivered(node_id, delivery.count);
                    None
                }
                Err(e) => {
                    eprintln!("[orbit] entregar a conversa para {node_id}: {e}");
                    Some(format!("não entregue: {e}"))
                }
            };
            let (waiting, from_each) = (a.talk.waiting(node_id), a.talk.waiting_from(node_id));
            if let Some(n) = a.sched.run.nodes.get_mut(node_id) {
                n.inbox = waiting;
                n.inbox_from = from_each;
                n.undelivered = undelivered;
            }
        }
        self.publish_current(run_id);
    }

    /// `orbit_finish`: fecha a sessão do próprio agente, como sair do CLI —
    /// só a dele (a ferramenta não tem argumento e o core usa o nó de quem
    /// chama). A resposta ainda precisa chegar ao agente, então a saída
    /// espera o turno fechar ([`FINISH_TURN_WAIT`] no máximo), escreve o
    /// comando de sair do runner e, se a sessão não sair em
    /// [`FINISH_GRACE`], mata. Daí segue o caminho de sempre: o nó fica
    /// pronto e o run fecha quando não sobra sessão.
    pub fn finish_agent(&self, run_id: &str, agent: &str) -> Result<serde_json::Value, String> {
        let run = self.get(run_id).ok_or("execução não encontrada")?;
        let session = run
            .nodes
            .get(agent)
            .and_then(|n| n.session_id.clone())
            .filter(|s| run.live_agents.contains(s))
            .ok_or("a sua sessão já não está aberta")?;
        let exit = match self.agent_runner(run_id, agent) {
            Some(Runner::Codex) => "/quit",
            _ => "/exit",
        };
        let called_at = now();
        let me = self.me.clone();
        let (run, node) = (run_id.to_string(), agent.to_string());
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + FINISH_TURN_WAIT;
            loop {
                let Some(runs) = me.upgrade() else { return };
                if !runs.host.alive(&session) {
                    return;
                }
                // Turno fechado depois da chamada (o `Ready` de antes não
                // vale: a resposta ainda não tinha chegado).
                let t = now();
                let closed = runs.active.lock().get(&run).is_none_or(|a| {
                    a.talk
                        .idle_for(&node, t)
                        .is_some_and(|idle| t - idle >= called_at)
                });
                if closed || std::time::Instant::now() >= deadline {
                    break;
                }
                drop(runs);
                std::thread::sleep(FINISH_POLL);
            }
            let Some(runs) = me.upgrade() else { return };
            if let Err(e) = runs.host.send_input(&session, exit) {
                eprintln!("[orbit] orbit_finish de {node}: {e}");
            }
            let grace = std::time::Instant::now() + FINISH_GRACE;
            while std::time::Instant::now() < grace {
                if !runs.host.alive(&session) {
                    return;
                }
                std::thread::sleep(FINISH_POLL);
            }
            runs.host.kill(&session);
        });
        Ok(serde_json::json!({
            "finishing": true,
            "note": "a sua sessão fecha quando este turno terminar; encerre o turno sem \
                     mais ferramentas",
        }))
    }

    /// O runner do agente `node`: do grafo do run (o do dono e os criados).
    fn agent_runner(&self, run_id: &str, node: &str) -> Option<Runner> {
        let kind = |n: &Node| match n.kind {
            NodeKind::Agent { runner, .. } => Some(runner),
            _ => None,
        };
        if let Some(a) = self.active.lock().get(run_id) {
            return a.graph.nodes.iter().find(|n| n.id == node).and_then(kind);
        }
        let run = self.get(run_id)?;
        if let Some(c) = run.created.iter().find(|c| c.node.id == node) {
            return kind(&c.node);
        }
        self.saved_graph(&run.graph_id)
            .ok()
            .flatten()?
            .nodes
            .iter()
            .find(|n| n.id == node)
            .and_then(kind)
    }

    /// O grafo salvo agora (o dono edita no canvas durante o run).
    fn saved_graph(&self, graph_id: &str) -> Result<Option<Graph>, String> {
        Ok(self
            .store
            .get_orbit_graph(graph_id)
            .map_err(|e| e.to_string())?
            .map(|s| s.value))
    }

    /// A nota `node`, só se ligada a `agent` por `memory` nas ligações de
    /// agora ([`current_edges`]). O texto é o do grafo salvo — o que o dono
    /// escreveu depois do ▶ conta —; a nota criada no run (fora do salvo)
    /// vem do run.
    fn memory_note(
        a: &Active,
        saved: Option<&Graph>,
        agent: &str,
        node: &str,
    ) -> Result<Node, String> {
        let linked = current_edges(saved, &a.sched.run.created)
            .iter()
            .any(|e| e.kind == EdgeKind::Memory && e.from == agent && e.to == node);
        let in_saved = saved.and_then(|g| g.nodes.iter().find(|n| n.id == node));
        let note = in_saved
            .or_else(|| {
                a.graph
                    .nodes
                    .iter()
                    .find(|n| n.id == node && is_created(a, node))
            })
            .filter(|n| matches!(n.kind, NodeKind::Note { .. }));
        match (linked, note) {
            (true, Some(n)) => Ok(n.clone()),
            _ => Err(format!(
                "{node} não é uma nota ligada a você por memória: só lê e escreve a nota \
                 que o dono ligou a você no canvas"
            )),
        }
    }

    /// `orbit_note_read`: o texto (redigido) da nota ligada por `memory`.
    pub fn note_read(
        &self,
        run_id: &str,
        agent: &str,
        node: &str,
    ) -> Result<serde_json::Value, String> {
        let active = self.active.lock();
        let a = active
            .get(run_id)
            .ok_or("esta execução já terminou: ▶ Executar de novo para ler a nota")?;
        let saved = self.saved_graph(&a.graph.id)?;
        let note = Self::memory_note(a, saved.as_ref(), agent, node)?;
        let NodeKind::Note { markdown, locked } = &note.kind else {
            unreachable!("memory_note só devolve nota");
        };
        Ok(serde_json::json!({
            "node": note.id,
            "title": note.title,
            "markdown": crate::session::redact::redact(markdown),
            "locked": locked,
        }))
    }

    /// `orbit_note_write`: troca (ou, com `append`, completa) o texto da nota
    /// ligada por `memory`, sempre sobre o texto do grafo salvo. Nota com
    /// cadeado (o do salvo: o dono tranca e destranca pela cápsula, também
    /// depois do ▶) recusa. Grava no grafo salvo (mesma validação do salvar)
    /// e no do run; a nota criada no run só no run. O retrato leva quem
    /// escreveu e o texto novo à tela. Devolve o tamanho do texto, em
    /// caracteres.
    pub fn note_write(
        &self,
        run_id: &str,
        agent: &str,
        node: &str,
        text: &str,
        append: bool,
    ) -> Result<usize, String> {
        let mut active = self.active.lock();
        let a = active
            .get_mut(run_id)
            .ok_or("esta execução já terminou: ▶ Executar de novo para escrever na nota")?;
        let stored = self
            .store
            .get_orbit_graph(&a.graph.id)
            .map_err(|e| e.to_string())?;
        let saved = stored.as_ref().map(|s| &s.value);
        let note = Self::memory_note(a, saved, agent, node)?;
        let NodeKind::Note { markdown, locked } = &note.kind else {
            unreachable!("memory_note só devolve nota");
        };
        if *locked {
            return Err(format!(
                "a nota {node} está com cadeado (só leitura): o dono não deixa agente \
                 escrever nela"
            ));
        }
        let by = a
            .graph
            .nodes
            .iter()
            .find(|n| n.id == agent)
            .map_or_else(|| agent.to_string(), |n| n.title.clone());
        let stamp = talk::author_stamp(&by, &chrono::Local::now().format("%H:%M").to_string());
        let markdown = talk::note_text(markdown, text, append, &stamp)?;
        let kind = NodeKind::Note {
            markdown: markdown.clone(),
            locked: false,
        };
        let in_saved = stored
            .as_ref()
            .is_some_and(|s| s.value.nodes.iter().any(|n| n.id == node));
        match stored {
            Some(stored) if in_saved => {
                let mut graph = stored.value;
                if let Some(n) = graph.nodes.iter_mut().find(|n| n.id == node) {
                    n.kind = kind.clone();
                }
                let integrations = self
                    .store
                    .list_orbit_integrations()
                    .map_err(|e| e.to_string())?;
                super::validate_graph(&graph, &integrations).map_err(|e| e.to_string())?;
                self.store
                    .save_orbit_graph(&graph, stored.consented, None)
                    .map_err(|e| e.to_string())?;
            }
            _ => {
                if let Some(c) = a.sched.run.created.iter_mut().find(|c| c.node.id == node) {
                    c.node.kind = kind.clone();
                }
            }
        }
        if let Some(n) = a.graph.nodes.iter_mut().find(|n| n.id == node) {
            n.kind = kind;
        }
        let chars = markdown.chars().count();
        if let Some(n) = a.sched.run.nodes.get_mut(node) {
            n.written = Some(NoteWritten {
                by,
                at: now(),
                markdown,
            });
        }
        drop(active);
        self.publish_current(run_id);
        Ok(chars)
    }

    /// `orbit_browser_open` e o `$BROWSER` da sessão: só no navegador ligado
    /// por `controla` a este agente (sem `node`, o ligado mais recente: o
    /// último a abrir página ou a ganhar a ligação). A url (já local) vira a do
    /// nó no run e, se o nó é do grafo do dono, no grafo salvo; o retrato do
    /// run leva `opened` à tela, que navega o frame. Devolve o id do nó.
    pub fn open_browser(
        &self,
        run_id: &str,
        agent: &str,
        node: Option<&str>,
        url: &str,
    ) -> Result<String, String> {
        let mut active = self.active.lock();
        let a = active
            .get_mut(run_id)
            .ok_or("esta execução já terminou: ▶ Executar de novo para o agente abrir páginas")?;
        let target = self.controlled_browser(a, agent, node)?;
        let by = a
            .graph
            .nodes
            .iter()
            .find(|n| n.id == agent)
            .map_or_else(|| agent.to_string(), |n| n.title.clone());
        let kind = NodeKind::Browser {
            url: url.to_string(),
        };
        if let Some(n) = a.graph.nodes.iter_mut().find(|n| n.id == target) {
            n.kind = kind.clone();
        }
        match a.sched.run.created.iter_mut().find(|c| c.node.id == target) {
            Some(c) => c.node.kind = kind.clone(),
            None => self.save_browser_url(&a.graph.id, &target, &kind),
        }
        if let Some(n) = a.sched.run.nodes.get_mut(&target) {
            n.url = Some(url.to_string());
            n.opened = Some(BrowserOpened { by, at: now() });
        }
        drop(active);
        self.publish_current(run_id);
        Ok(target)
    }

    /// O navegador ligado por `controla` a `agent` agora (as ligações do
    /// grafo salvo, mais as dos nós criados no run): `node` se ele é um
    /// desses; sem `node`, o ligado mais recente — o último a abrir página ou
    /// a ganhar a ligação.
    fn controlled_browser(
        &self,
        a: &Active,
        agent: &str,
        node: Option<&str>,
    ) -> Result<String, String> {
        // As ligações de agora: o dono liga e desliga no canvas durante o run.
        let saved = self.saved_graph(&a.graph.id)?;
        let linked: Vec<String> = current_edges(saved.as_ref(), &a.sched.run.created)
            .into_iter()
            .filter(|e| e.kind == EdgeKind::Control && e.from == agent)
            .filter(|e| {
                saved
                    .iter()
                    .flat_map(|g| g.nodes.iter())
                    .chain(a.graph.nodes.iter())
                    .any(|n| n.id == e.to && matches!(n.kind, NodeKind::Browser { .. }))
            })
            .map(|e| e.to)
            .collect();
        match node {
            Some(n) if linked.iter().any(|l| l == n) => Ok(n.to_string()),
            Some(n) => Err(format!(
                "{n} não é um navegador ligado a você por controla: só o navegador que o \
                 dono ligou a você no canvas"
            )),
            None => {
                // A última vez que o navegador mudou para este agente: a
                // página aberta, ou quando a ligação nasceu (a do grafo, no
                // ▶; a do criado, ao criar).
                let touched = |id: &str| {
                    let linked_at = a
                        .sched
                        .run
                        .created
                        .iter()
                        .find(|c| c.node.id == id)
                        .map_or(a.sched.run.started_at, |c| c.created_at);
                    let opened_at = a
                        .sched
                        .run
                        .nodes
                        .get(id)
                        .and_then(|n| n.opened.as_ref())
                        .map_or(0, |o| o.at);
                    linked_at.max(opened_at)
                };
                linked
                    .iter()
                    .enumerate()
                    .max_by_key(|(i, id)| (touched(id), *i))
                    .map(|(_, id)| id.clone())
                    .ok_or_else(|| {
                        let hint = if is_created(a, agent) {
                            "quem te criou pode criar um"
                        } else {
                            "o dono liga um no canvas, ou crie com orbit_create_browser"
                        };
                        format!(
                            "nenhum navegador ligado a você por controla ({hint}); o \
                             navegador do sistema não abre por aqui"
                        )
                    })
            }
        }
    }

    /// `orbit_browser_snapshot` e as outras: ler ou usar a página do
    /// navegador ligado por `controla` a `agent` (a mesma régua do
    /// `orbit_browser_open`, relida a cada chamada). Quem lê e age é o host
    /// (a view nativa); o lock do run sai antes, porque a ação espera a
    /// página.
    pub fn browser_action(
        &self,
        run_id: &str,
        agent: &str,
        node: Option<&str>,
        action: AgentAction,
    ) -> Result<serde_json::Value, String> {
        let mut action = action;
        let (graph_id, target, by) = {
            let active = self.active.lock();
            let a = active.get(run_id).ok_or(
                "esta execução já terminou: ▶ Executar de novo para o agente usar o navegador",
            )?;
            let target = self.controlled_browser(a, agent, node)?;
            // O print fica na pasta do run, na `out/` do navegador (0700),
            // e é podado junto com o run.
            if let AgentAction::Screenshot { dir } = &mut action {
                *dir = a.dir.join(&target).join("out");
                create_private_dir(dir).map_err(|e| format!("pasta do print: {e}"))?;
            }
            let by = a
                .graph
                .nodes
                .iter()
                .find(|n| n.id == agent)
                .map_or_else(|| agent.to_string(), |n| n.title.clone());
            (a.graph.id.clone(), target, by)
        };
        let mut value = self.host.browser(&graph_id, &target, &by, action)?;
        if let serde_json::Value::Object(map) = &mut value {
            map.entry("node_id")
                .or_insert_with(|| target.clone().into());
        }
        Ok(value)
    }

    /// A url nova do navegador do grafo do dono, no grafo salvo. Falha aqui
    /// não desfaz a navegação: fica no log.
    fn save_browser_url(&self, graph_id: &str, node_id: &str, kind: &NodeKind) {
        let save = || -> Result<(), String> {
            let Some(stored) = self
                .store
                .get_orbit_graph(graph_id)
                .map_err(|e| e.to_string())?
            else {
                return Ok(());
            };
            let mut graph = stored.value;
            let Some(n) = graph.nodes.iter_mut().find(|n| n.id == node_id) else {
                return Ok(());
            };
            if n.kind == *kind {
                return Ok(());
            }
            n.kind = kind.clone();
            let integrations = self
                .store
                .list_orbit_integrations()
                .map_err(|e| e.to_string())?;
            super::validate_graph(&graph, &integrations).map_err(|e| e.to_string())?;
            self.store
                .save_orbit_graph(&graph, stored.consented, None)
                .map_err(|e| e.to_string())
        };
        if let Err(e) = save() {
            eprintln!("[orbit] gravar a url do navegador {node_id}: {e}");
        }
    }

    /// `orbit_plan`: a nota de plano do orquestrador, no grafo salvo (não é
    /// nó criado do run). A primeira chamada cria a nota "Plano" logo abaixo
    /// dele e a ligação `plan`; as seguintes só reescrevem o texto da mesma
    /// nota (a do id guardado em `run.plans`: se um salvar da tela em voo a
    /// tirou do grafo, ela volta com o mesmo id). Nota já ligada por `plan`
    /// (um run anterior, ou o dono ligou à mão) só é reaproveitada se o texto
    /// dela está vazio ou é checklist; senão o texto é do dono e nasce outra.
    /// Devolve o id da nota. O grafo passa pela validação do salvar; o run guarda uma cópia e o evento do run leva a
    /// nota ao front.
    pub fn write_plan(
        &self,
        run_id: &str,
        orchestrator: &str,
        steps: Vec<PlanStep>,
    ) -> Result<String, String> {
        let markdown = orchestrator::plan_markdown(&steps);
        if let Some(id) = self.write_created_plan(run_id, orchestrator, &markdown)? {
            self.publish_current(run_id);
            return Ok(id);
        }
        let note_markdown = |n: &Node| match &n.kind {
            NodeKind::Note { markdown, .. } => Some(markdown.clone()),
            _ => None,
        };
        let note_id = {
            let mut active = self.active.lock();
            let a = active.get_mut(run_id).ok_or(
                "esta execução já terminou: ▶ Executar de novo para o orquestrador escrever o plano",
            )?;
            let stored = self
                .store
                .get_orbit_graph(&a.graph.id)
                .map_err(|e| e.to_string())?
                .ok_or("grafo não encontrado")?;
            let mut graph = stored.value;
            let own = a
                .sched
                .run
                .plans
                .iter()
                .find(|p| p.orchestrator == orchestrator)
                .cloned();
            let existing = match &own {
                Some(p) => {
                    // Um salvar da tela em voo a tirou: volta com o mesmo id.
                    if !graph.nodes.iter().any(|n| n.id == p.note.id) {
                        graph.nodes.push(p.note.clone());
                    }
                    if !graph.edges.iter().any(|e| e.id == p.edge.id) {
                        graph.edges.push(p.edge.clone());
                    }
                    Some((p.note.id.clone(), p.edge.clone()))
                }
                None => graph
                    .edges
                    .iter()
                    .filter(|e| e.kind == EdgeKind::Plan && e.from == orchestrator)
                    .find_map(|e| {
                        graph
                            .nodes
                            .iter()
                            .find(|n| n.id == e.to)
                            .and_then(note_markdown)
                            .filter(|m| orchestrator::is_plan_text(m))
                            .map(|_| (e.to.clone(), e.clone()))
                    }),
            };
            let (note, edge) = match existing {
                Some((id, edge)) => {
                    let note = graph
                        .nodes
                        .iter_mut()
                        .find(|n| n.id == id)
                        .ok_or("nota de plano sumiu")?;
                    let locked = matches!(note.kind, NodeKind::Note { locked: true, .. });
                    note.kind = NodeKind::Note {
                        markdown: markdown.clone(),
                        locked,
                    };
                    (note.clone(), edge)
                }
                None => {
                    let id = format!("plano-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
                    let anchor = graph
                        .nodes
                        .iter()
                        .chain(a.graph.nodes.iter())
                        .find(|n| n.id == orchestrator)
                        .cloned();
                    // O grafo salvo e o que o run criou ocupam o canvas.
                    let occupied: Vec<&Node> = graph
                        .nodes
                        .iter()
                        .chain(
                            a.graph
                                .nodes
                                .iter()
                                .filter(|n| !graph.nodes.iter().any(|g| g.id == n.id)),
                        )
                        .collect();
                    let note = orchestrator::plan_note(
                        id.clone(),
                        anchor.as_ref(),
                        &occupied,
                        markdown.clone(),
                    );
                    let edge = Edge {
                        id: format!("{id}-plan"),
                        from: orchestrator.to_string(),
                        to: id,
                        kind: EdgeKind::Plan,
                    };
                    graph.nodes.push(note.clone());
                    graph.edges.push(edge.clone());
                    (note, edge)
                }
            };
            let integrations = self
                .store
                .list_orbit_integrations()
                .map_err(|e| e.to_string())?;
            super::validate_graph(&graph, &integrations).map_err(|e| e.to_string())?;
            self.store
                .save_orbit_graph(&graph, stored.consented, None)
                .map_err(|e| e.to_string())?;
            // O grafo do run também a vê: os nós criados depois desviam dela.
            match a.graph.nodes.iter_mut().find(|n| n.id == note.id) {
                Some(n) => n.kind = note.kind.clone(),
                None => a.graph.nodes.push(note.clone()),
            }
            if !a.graph.edges.iter().any(|e| e.id == edge.id) {
                a.graph.edges.push(edge.clone());
            }
            let plan = Plan {
                orchestrator: orchestrator.to_string(),
                note: note.clone(),
                edge,
                steps,
                updated_at: now(),
            };
            let plans = &mut a.sched.run.plans;
            match plans.iter_mut().find(|p| p.orchestrator == orchestrator) {
                Some(p) => *p = plan,
                None => plans.push(plan),
            }
            note.id
        };
        self.publish_current(run_id);
        Ok(note_id)
    }

    /// No ▶: a nota de plano de cada agente do run fica vazia na hora, antes
    /// de ele escrever — o checklist da execução anterior não aparece como
    /// se fosse desta. Vale a nota que o [`Runs::write_plan`] reaproveitaria (a
    /// primeira ligada por `plan` com texto de plano); a do dono, com texto
    /// próprio, fica como está. Zera no grafo do run e no salvo, e volta os
    /// planos pendentes (sem passos) que o run guarda: a tela mostra
    /// "esperando o agente escrever o plano…" e o `orbit_plan` reescreve a
    /// mesma nota.
    fn reset_plans(&self, graph: &mut Graph) -> Vec<Plan> {
        let mut pending: Vec<Plan> = Vec::new();
        for edge in graph.edges.iter().filter(|e| e.kind == EdgeKind::Plan) {
            let agent = graph
                .nodes
                .iter()
                .any(|n| n.id == edge.from && matches!(n.kind, NodeKind::Agent { .. }));
            if !agent || pending.iter().any(|p| p.orchestrator == edge.from) {
                continue;
            }
            let Some(note) = graph.nodes.iter().find(|n| n.id == edge.to) else {
                continue;
            };
            let NodeKind::Note { markdown, locked } = &note.kind else {
                continue;
            };
            if !orchestrator::is_plan_text(markdown) {
                continue;
            }
            let mut note = note.clone();
            note.kind = NodeKind::Note {
                markdown: String::new(),
                locked: *locked,
            };
            pending.push(Plan {
                orchestrator: edge.from.clone(),
                note,
                edge: edge.clone(),
                steps: Vec::new(),
                updated_at: now(),
            });
        }
        if pending.is_empty() {
            return pending;
        }
        let blank = |nodes: &mut Vec<Node>| {
            for p in &pending {
                if let Some(n) = nodes.iter_mut().find(|n| {
                    n.id == p.note.id
                        && matches!(&n.kind, NodeKind::Note { markdown, .. } if orchestrator::is_plan_text(markdown))
                }) {
                    if let NodeKind::Note { markdown, .. } = &mut n.kind {
                        markdown.clear();
                    }
                }
            }
        };
        blank(&mut graph.nodes);
        let save = || -> Result<(), String> {
            let stored = self
                .store
                .get_orbit_graph(&graph.id)
                .map_err(|e| e.to_string())?
                .ok_or("grafo não encontrado")?;
            let mut saved = stored.value;
            blank(&mut saved.nodes);
            self.store
                .save_orbit_graph(&saved, stored.consented, None)
                .map_err(|e| e.to_string())
        };
        if let Err(e) = save() {
            eprintln!("[orbit] zerar a nota de plano no ▶: {e}");
        }
        pending
    }

    /// O plano do agente criado por outro: ele não está no grafo salvo, então
    /// a nota também não vai para lá — entra no run como nó criado por ele
    /// (efêmera como o autor, com "Manter"), ligada por `plan`. As chamadas
    /// seguintes só reescrevem o texto dela. `None`: quem pede é do grafo do
    /// dono (segue o caminho do grafo salvo).
    fn write_created_plan(
        &self,
        run_id: &str,
        agent: &str,
        markdown: &str,
    ) -> Result<Option<String>, String> {
        let mut active = self.active.lock();
        let a = active.get_mut(run_id).ok_or(
            "esta execução já terminou: ▶ Executar de novo para o agente escrever o plano",
        )?;
        if !is_created(a, agent) {
            return Ok(None);
        }
        let kind = NodeKind::Note {
            markdown: markdown.to_string(),
            locked: false,
        };
        let own = a.sched.run.created.iter_mut().find(|c| {
            c.created_by == agent
                && c.edges
                    .iter()
                    .any(|e| e.kind == EdgeKind::Plan && e.from == agent)
        });
        if let Some(c) = own {
            c.node.kind = kind.clone();
            let id = c.node.id.clone();
            if let Some(n) = a.graph.nodes.iter_mut().find(|n| n.id == id) {
                n.kind = kind;
            }
            return Ok(Some(id));
        }
        if a.sched.run.created.len() >= MAX_CREATED {
            return Err(format!(
                "teto de {MAX_CREATED} nós criados por execução atingido"
            ));
        }
        let id = format!("plano-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let note = {
            let anchor = a.graph.nodes.iter().find(|n| n.id == agent);
            let occupied: Vec<&Node> = a.graph.nodes.iter().collect();
            orchestrator::plan_note(id.clone(), anchor, &occupied, markdown.to_string())
        };
        let edge = Edge {
            id: format!("{id}-plan"),
            from: agent.to_string(),
            to: id.clone(),
            kind: EdgeKind::Plan,
        };
        if let Some(root) = &self.root {
            for sub in ["in", "out"] {
                create_private_dir(&root.join(run_id).join(&id).join(sub))
                    .map_err(|e| e.to_string())?;
            }
        }
        // `plan` não segura o alvo: a nota grava o texto e fica pronta.
        a.sched.add_node(&id, &[])?;
        a.graph.nodes.push(note.clone());
        a.graph.edges.push(edge.clone());
        a.sched.run.created.push(Created {
            node: note,
            created_by: agent.to_string(),
            created_at: now(),
            edges: vec![edge],
            kept: false,
        });
        drop(active);
        self.advance(run_id);
        Ok(Some(id))
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
            // O navegador que o orquestrador cria é controlado por ele.
            if matches!(kind, NodeKind::Browser { .. }) {
                edges.push(Edge {
                    id: format!("{id}-control"),
                    from: orchestrator.to_string(),
                    to: id.clone(),
                    kind: EdgeKind::Control,
                });
            }
            // Ao lado de quem dá a url (o navegador com `from_node`) ou do
            // orquestrador, num espaço livre: o grafo do run já tem os nós do
            // dono e os criados antes deste.
            let node = {
                let anchor_id = from_node.as_deref().unwrap_or(orchestrator);
                let anchor = a.graph.nodes.iter().find(|n| n.id == anchor_id);
                let occupied: Vec<&Node> = a.graph.nodes.iter().collect();
                orchestrator::new_node(id.clone(), title, kind, anchor, &occupied)
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
    /// mudou. Só sai o que já é redigido: nada de caminho para a saída crua
    /// (log, patch, nota, json) — o log inteiro vem por ferramenta que redige.
    ///
    /// Só de nó ligado a quem pergunta (qualquer ligação, em qualquer
    /// sentido, nas de agora) ou que ele criou.
    pub fn node_status(
        &self,
        run_id: &str,
        caller: &str,
        node_id: &str,
    ) -> Result<serde_json::Value, String> {
        let run = self.get(run_id).ok_or("execução não encontrada")?;
        let n = run
            .nodes
            .get(node_id)
            .ok_or_else(|| format!("nó não existe neste run: {node_id}"))?;
        if !self.reaches(&run, caller, node_id)? {
            return Err(format!(
                "{node_id} não está ligado a você nem foi criado por você: só vê o estado \
                 de nó ligado a você no canvas ou que você criou"
            ));
        }
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
        }))
    }

    /// `caller` alcança `node_id`: criou o nó neste run, ou há ligação entre
    /// os dois (qualquer tipo, qualquer sentido) nas ligações de agora.
    fn reaches(&self, run: &Run, caller: &str, node_id: &str) -> Result<bool, String> {
        let made = run
            .created
            .iter()
            .any(|c| c.node.id == node_id && c.created_by == caller);
        if made {
            return Ok(true);
        }
        let saved = self.saved_graph(&run.graph_id)?;
        Ok(current_edges(saved.as_ref(), &run.created).iter().any(|e| {
            (e.from == caller && e.to == node_id) || (e.from == node_id && e.to == caller)
        }))
    }

    /// `orbit_restart_node`: a régua do `orbit_node_status` (nó ligado a
    /// quem chama ou criado por ele), nunca o próprio nó. Processo: responde
    /// como o `orbit_create_process` (caiu de novo, a url, ou o estado).
    fn restart_by_agent(
        &self,
        run_id: &str,
        caller: &str,
        node_id: &str,
    ) -> Result<serde_json::Value, String> {
        if node_id == caller {
            return Err(
                "orbit_restart_node não reinicia o seu próprio nó: para fechar a sua sessão, \
                 orbit_finish"
                    .into(),
            );
        }
        let run = self.get(run_id).ok_or("execução não encontrada")?;
        let process = match run.nodes.get(node_id) {
            None => return Err(format!("nó não existe neste run: {node_id}")),
            Some(_) => self.node_kind(run_id, node_id).is_some_and(|k| {
                matches!(k, NodeKind::Process { .. } | NodeKind::Integration { .. })
            }),
        };
        if !self.reaches(&run, caller, node_id)? {
            return Err(format!(
                "{node_id} não está ligado a você nem foi criado por você: só reinicia nó \
                 ligado a você no canvas ou que você criou"
            ));
        }
        // Integração manual roda só pelo ▶ do dono: o agente não a dispara
        // reiniciando.
        if self.manual_integration(run_id, node_id) {
            return Err(format!(
                "integração manual: só o dono roda — {node_id} espera o ▶ do dono no nó"
            ));
        }
        self.restart_node(run_id, node_id)?;
        if process {
            return Ok(self.settle_process(run_id, node_id));
        }
        Ok(serde_json::json!({ "node_id": node_id, "restarted": true }))
    }

    /// O nó é de integração com gatilho manual, no run ativo.
    fn manual_integration(&self, run_id: &str, node_id: &str) -> bool {
        let active = self.active.lock();
        let Some(a) = active.get(run_id) else {
            return false;
        };
        a.graph.nodes.iter().any(|n| {
            n.id == node_id
                && matches!(&n.kind, NodeKind::Integration { integration_id, .. }
                    if a.integrations.get(integration_id)
                        .is_some_and(|i| i.trigger == super::Trigger::Manual))
        })
    }

    /// O tipo do nó no grafo do run ativo (o do dono e os criados).
    fn node_kind(&self, run_id: &str, node_id: &str) -> Option<NodeKind> {
        let active = self.active.lock();
        let a = active.get(run_id)?;
        a.graph
            .nodes
            .iter()
            .chain(a.sched.run.created.iter().map(|c| &c.node))
            .find(|n| n.id == node_id)
            .map(|n| n.kind.clone())
    }

    /// `orbit_stop_node`: só nó que este orquestrador criou. Mata o process
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

    /// Apagar um nó rodando na tela: para só ele (rodando vira `stopped`,
    /// esperando vira `skipped`, como o `orbit_stop_node`) e mata a sessão
    /// dele — também a do agente que segue vivo depois do turno, com o run já
    /// fechado. O front só tira o nó do grafo depois disto. Nó sem nada vivo:
    /// não faz nada.
    pub fn stop_one(&self, run_id: &str, node_id: &str) -> Result<(), String> {
        let (was_active, mut kill) = {
            let mut active = self.active.lock();
            match active.get_mut(run_id) {
                Some(a) => (true, a.sched.stop_node(node_id, now())),
                None => (false, None),
            }
        };
        let session = match self.get(run_id) {
            Some(run) => run.nodes.get(node_id).and_then(|n| n.session_id.clone()),
            None if was_active => None,
            None => return Err("execução não encontrada".into()),
        };
        // O agente vivo depois do turno sai da lista do run: o Parar e o
        // `live_agents` não o veem mais.
        if let Some(session) = session {
            let mut agents = self.agents.lock();
            if let Some(a) = agents.get_mut(run_id) {
                let before = a.sessions.len();
                a.sessions.retain(|s| *s != session);
                if a.sessions.len() != before {
                    kill = Some(session);
                }
            }
        }
        if let Some(session) = &kill {
            self.host.kill(session);
        }
        if was_active {
            self.publish_current(run_id);
        } else if kill.is_some() {
            if let Some(run) = self.get(run_id) {
                self.publish(run);
            }
        }
        Ok(())
    }

    /// ↻ do nó, com a execução aberta (rodando ou assentada — que volta a
    /// "ao vivo"): reiniciar o que roda (mata o process group e sobe de novo
    /// com a mesma config) ou rodar de novo o que terminou, falhou ou parou.
    /// A `out/` da tentativa de antes é limpa antes de subir; quem depende
    /// do nó e já consumiu o valor antigo fica "↻ para aplicar".
    pub fn restart_node(&self, run_id: &str, node_id: &str) -> Result<(), String> {
        let (session, node_dir) = {
            let mut active = self.active.lock();
            let a = active
                .get_mut(run_id)
                .ok_or("a execução já fechou: ▶ Executar de novo")?;
            // O reinício usa as variáveis de agora: a que ficou sem valor
            // recusa antes de matar a sessão velha.
            active_missing_variable(a, node_id)?;
            let session = a
                .sched
                .rerun(node_id)
                .map_err(|e| format!("{}: {e}", active_label(a, node_id)))?;
            a.talk.reset_state(node_id);
            (session, a.dir.join(node_id))
        };
        // O nó segue segurado (`rerun`): nada o sobe antes de a sessão velha
        // morrer e a `out/` dela sumir.
        if let Some(session) = &session {
            if let Some(a) = self.agents.lock().get_mut(run_id) {
                a.sessions.retain(|s| s != session);
            }
            self.host.kill(session);
        }
        // `in/` e `out/` da tentativa de antes saem: a nova materializa as
        // entradas de agora, sem sobra de uma ligação que já não existe.
        let cleared = ["in", "out"].iter().try_for_each(|sub| {
            let dir = node_dir.join(sub);
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            }
            .and_then(|()| create_private_dir(&dir))
        });
        {
            let mut active = self.active.lock();
            if let Some(a) = active.get_mut(run_id) {
                a.sched.unhold(node_id);
                if let Err(e) = &cleared {
                    a.sched.mark_running(node_id, now());
                    a.sched.finish(
                        node_id,
                        Outcome::failed(format!("limpar a tentativa anterior: {e}")),
                        now(),
                    );
                }
            }
        }
        eprintln!("[orbit] nó {node_id} reiniciado");
        self.publish_current(run_id);
        self.advance(run_id);
        Ok(())
    }

    /// O serviço subiu ("Subir"): o nó dele que caiu ou parou no run aberto
    /// do grafo roda de novo, e quem esperava por ele segue.
    pub fn rerun_service(&self, graph_id: &str, node_id: &str) {
        let runs: Vec<String> = self
            .active
            .lock()
            .iter()
            .filter(|(_, a)| {
                a.graph.id == graph_id
                    && !a.sched.run.test
                    && matches!(
                        a.sched.status(node_id),
                        Some(NodeStatus::Failed | NodeStatus::Stopped)
                    )
            })
            .map(|(id, _)| id.clone())
            .collect();
        for run_id in runs {
            if let Err(e) = self.restart_node(&run_id, node_id) {
                eprintln!("[orbit] rodar de novo o serviço {node_id}: {e}");
            }
        }
    }

    /// O dono salvou o grafo com execução aberta: o core reconcilia cada run
    /// aberto dele (não o de teste).
    /// - Nó que saiu do grafo e ainda roda ou espera: para (o `stop_one`,
    ///   que mata o process group). O criado no run não está no salvo e fica.
    /// - Nó cuja config mudou (comando, prompt, pasta, url, alvo…): a nova
    ///   vale no próximo começo; quem já começou fica "↻ para aplicar"
    ///   ([`Scheduler::mark_changed`]) e só muda quando o dono reinicia.
    /// - Ligações de valor valem na hora ([`Scheduler::rewire`]); as de
    ///   permissão já são lidas do salvo a cada chamada.
    pub fn graph_saved(&self, graph: &Graph) {
        let mut removed: Vec<(String, String)> = Vec::new();
        let mut touched: Vec<String> = Vec::new();
        {
            let mut active = self.active.lock();
            for (run_id, a) in active
                .iter_mut()
                .filter(|(_, a)| a.graph.id == graph.id && !a.sched.run.test)
            {
                let created: HashSet<String> = a
                    .sched
                    .run
                    .created
                    .iter()
                    .map(|c| c.node.id.clone())
                    .collect();
                let mut changed = false;
                for node in a.graph.nodes.iter_mut() {
                    match graph.nodes.iter().find(|n| n.id == node.id) {
                        Some(new) if node.kind != new.kind => {
                            // A nota é lida do salvo por quem a usa; o
                            // navegador troca de página sozinho (a url que
                            // chega e a que o agente abre vão para o nó).
                            // O título não muda o que roda.
                            // O serviço com o mesmo alvo e id novo (o card
                            // copia o do contêiner recriado) também não.
                            let marks = match (&node.kind, &new.kind) {
                                (NodeKind::Note { .. } | NodeKind::Browser { .. }, _) => false,
                                (
                                    NodeKind::Service { source: old },
                                    NodeKind::Service { source: next },
                                ) => !old.same_target(next),
                                _ => true,
                            };
                            node.kind = new.kind.clone();
                            if marks {
                                changed |= a.sched.mark_changed(&node.id, StaleReason::Config);
                            }
                        }
                        Some(_) => {}
                        None if created.contains(&node.id) => {}
                        None => {
                            if matches!(
                                a.sched.status(&node.id),
                                Some(NodeStatus::Running | NodeStatus::Waiting)
                            ) {
                                removed.push((run_id.clone(), node.id.clone()));
                            }
                        }
                    }
                }
                // As variáveis do salvo valem no próximo começo (o reinício
                // inclusive); quem já começou e usa uma que mudou fica "↻
                // para aplicar".
                if a.graph.variables != graph.variables {
                    let old = std::mem::replace(&mut a.graph.variables, graph.variables.clone());
                    for node in &a.graph.nodes {
                        let integration = match &node.kind {
                            NodeKind::Integration { integration_id, .. } => {
                                a.integrations.get(integration_id)
                            }
                            _ => None,
                        };
                        let uses_changed = run_text(node, integration).is_some_and(|text| {
                            crate::snippet::placeholders(text)
                                .iter()
                                .any(|p| old.get(&p.name) != graph.variables.get(&p.name))
                        });
                        if uses_changed {
                            changed |= a.sched.mark_changed(&node.id, StaleReason::Variable);
                        }
                    }
                }
                let ids: HashSet<&str> = a.graph.nodes.iter().map(|n| n.id.as_str()).collect();
                let edges: Vec<Edge> = current_edges(Some(graph), &a.sched.run.created)
                    .into_iter()
                    .filter(|e| ids.contains(e.from.as_str()) && ids.contains(e.to.as_str()))
                    .collect();
                if a.graph.edges != edges {
                    changed |= a.sched.rewire(&edges);
                    a.graph.edges = edges;
                    a.sched.settle(now());
                }
                if changed {
                    touched.push(run_id.clone());
                }
            }
        }
        for (run_id, node_id) in &removed {
            eprintln!("[orbit] nó {node_id} saiu do grafo com o run aberto: parado");
            let _ = self.stop_one(run_id, node_id);
        }
        for run_id in touched {
            self.publish_current(&run_id);
            self.advance(&run_id);
        }
    }

    /// A integração mudou na biblioteca com execução aberta: o nó que a usa
    /// e ainda espera roda o script novo; o que já começou fica "↻ para
    /// aplicar".
    pub fn integration_saved(&self, integration: &Integration) {
        let mut touched = Vec::new();
        {
            let mut active = self.active.lock();
            for (run_id, a) in active.iter_mut() {
                if a.integrations.get(&integration.id) == Some(integration) {
                    continue;
                }
                let users: Vec<String> = a
                    .graph
                    .nodes
                    .iter()
                    .filter(|n| matches!(&n.kind, NodeKind::Integration { integration_id, .. } if *integration_id == integration.id))
                    .map(|n| n.id.clone())
                    .collect();
                if users.is_empty() {
                    continue;
                }
                a.integrations
                    .insert(integration.id.clone(), integration.clone());
                let mut changed = false;
                for id in &users {
                    changed |= a.sched.mark_changed(id, StaleReason::Config);
                }
                if changed {
                    touched.push(run_id.clone());
                }
            }
        }
        for run_id in touched {
            self.publish_current(&run_id);
        }
    }

    /// Os nós criados ainda não mantidos do run (o "Manter no grafo"): do
    /// ativo ou do retrato guardado — o run terminado, parado ou do histórico
    /// também os deixa manter.
    pub fn created_to_keep(&self, run_id: &str) -> Result<Vec<Created>, String> {
        let run = self.get(run_id).ok_or("execução não encontrada")?;
        Ok(run.created.into_iter().filter(|c| !c.kept).collect())
    }

    /// Marca como mantidos (já gravados no grafo) e avisa o front. Run fechado:
    /// a marca vai para o retrato guardado.
    pub fn mark_kept(&self, run_id: &str, ids: &[String]) {
        let mark = |created: &mut Vec<Created>| {
            for c in created.iter_mut().filter(|c| ids.contains(&c.node.id)) {
                c.kept = true;
            }
        };
        {
            let mut active = self.active.lock();
            let Some(a) = active.get_mut(run_id) else {
                if let Some(mut run) = self.stored_closed(run_id) {
                    mark(&mut run.created);
                    self.publish(run);
                }
                return;
            };
            mark(&mut a.sched.run.created);
        }
        self.publish_current(run_id);
    }

    /// O retrato guardado de um run fechado. Chamado com `active` na mão: o
    /// run que fecha só sai de `active` depois de gravado, e dois retoques do
    /// mesmo run não se atropelam. `running` no banco sem estar ativo é o
    /// ■ Parar no meio, que grava o retrato dele logo em seguida.
    fn stored_closed(&self, run_id: &str) -> Option<Run> {
        self.store
            .get_orbit_run(run_id)
            .ok()
            .flatten()
            .filter(|r| r.status != RunStatus::Running)
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
    fn agent_turn_ended(&self, run_id: &str, node_id: &str, end: TurnEnd<'_>, turn: AgentTurn) {
        let current = self
            .active
            .lock()
            .get(run_id)
            .is_some_and(|a| a.sched.attempt(node_id) == Some(end.attempt));
        if !current {
            return;
        }
        let outcome = match turn {
            AgentTurn::Exited => Outcome::failed("o agente saiu antes de fechar o turno"),
            AgentTurn::Failed(reason) => Outcome::failed(reason),
            AgentTurn::Done { text } => agent_outputs(end.out_dir, end.snapshot, text.as_deref()),
        };
        let holds = end.holds;
        {
            let mut active = self.active.lock();
            let Some(a) = active.get_mut(run_id) else {
                return;
            };
            if a.sched.attempt(node_id) != Some(end.attempt) {
                return;
            }
            let changed = if holds && outcome.ok {
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
    /// A sessão de agente saiu depois do primeiro turno. Run ativo: o
    /// orquestrador fecha o nó, e o retrato novo leva `live_agents` sem ela.
    /// Run já fechado: o retrato guardado é republicado só por isso.
    fn agent_exited(&self, run_id: &str, node_id: &str, holds: bool, attempt: u32) {
        {
            let active = self.active.lock();
            if !active.contains_key(run_id) {
                if let Some(run) = self.stored_closed(run_id) {
                    self.publish(run);
                }
                return;
            }
        }
        if holds {
            self.orchestrator_exited(run_id, node_id, attempt);
        } else {
            self.publish_current(run_id);
        }
    }

    fn orchestrator_exited(&self, run_id: &str, node_id: &str, attempt: u32) {
        {
            let mut active = self.active.lock();
            let Some(a) = active.get_mut(run_id) else {
                return;
            };
            if a.sched.attempt(node_id) != Some(attempt) || !a.sched.orchestrating(node_id) {
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
    /// sai fora do lock. A saída declarada, a pasta e o comando são os da
    /// largada desta tentativa ([`ExitConfig`]), não os do grafo editado.
    fn process_exited(&self, run_id: &str, node_id: &str, attempt: u32, config: &ExitConfig) {
        {
            let active = self.active.lock();
            let Some(a) = active.get(run_id) else {
                return;
            };
            // O PTY velho de um nó reiniciado: a `out/` já é da tentativa nova.
            if a.sched.attempt(node_id) != Some(attempt) {
                return;
            }
        }
        let ExitConfig {
            out_dir,
            node_dir,
            declared,
            command,
            title,
            snapshot,
        } = config;
        let exit = read_exit(out_dir);
        // Caiu: o porquê numa linha (porta ocupada e por quem, ou a última
        // linha do log) para o rodapé do card e para o agente que criou. O
        // `lsof` roda aqui, fora do lock e da main thread.
        let reason = match exit {
            None => Some("o processo terminou sem exit code".to_string()),
            Some(0) => None,
            Some(_) => super::failure::failure_reason(&out_dir.join("log.txt"), command, title),
        };
        let outputs = match declared {
            Some((output, file)) => process_outputs(
                *output,
                file.as_deref(),
                out_dir,
                node_dir,
                snapshot.as_ref(),
            ),
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
                reason,
                url: None,
            };
            if a.sched.attempt(node_id) != Some(attempt) || !a.sched.finish(node_id, outcome, now())
            {
                return;
            }
        }
        self.publish_current(run_id);
        self.advance(run_id);
    }
}

/// Quanto o `orbit_create_process` espera o processo cair ou abrir porta
/// antes de responder: erro de subida (porta ocupada, módulo faltando) sai
/// nisso.
const PROCESS_SETTLE: std::time::Duration = std::time::Duration::from_secs(3);
/// Quanto o `orbit_finish` espera o turno fechar antes de sair mesmo assim.
const FINISH_TURN_WAIT: std::time::Duration = std::time::Duration::from_secs(60);
/// Quanto a sessão tem para sair depois do comando de sair, antes de morrer.
const FINISH_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
const FINISH_POLL: std::time::Duration = std::time::Duration::from_millis(200);
/// Linhas do log na resposta do processo que caiu.
const SETTLE_LOG_LINES: usize = 20;
/// Teto de linhas que o card pede do log.
const NODE_LOG_MAX_LINES: usize = 200;

/// De quanto em quanto o processo olha as portas em LISTEN do process group
/// da sessão (a varredura própria, não o retrato de 10 s do chip).
const PORT_POLL: std::time::Duration = std::time::Duration::from_secs(1);

/// Navegador: a url dele, quando tem uma; sem ela, a que chegou pela
/// ligação. A ligação ainda serve para esperar o nó ficar pronto: o Caddy
/// abre várias portas e a menor não é o host virtual que ele serve. Só local
/// passa — de novo aqui, porque a que chega por ligação nunca passou pelo
/// salvar.
fn browser_outcome(start: &Start, own: &str) -> Outcome {
    let own = own.trim();
    let url = if own.is_empty() {
        start
            .inputs
            .iter()
            .filter(|i| i.kind == EdgeKind::Url)
            .filter_map(|i| i.source.as_deref())
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .map(|t| t.trim().to_string())
            .find(|t| !t.is_empty())
            .unwrap_or_default()
    } else {
        own.to_string()
    };
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

/// O que o fim do processo precisa da largada da tentativa: o grafo do run
/// pode ter sido editado depois, e a config nova só vale no reinício.
struct ExitConfig {
    out_dir: PathBuf,
    node_dir: PathBuf,
    /// A saída declarada (processo ou integração) e o arquivo dela.
    declared: Option<(OutputKind, Option<String>)>,
    /// O comando do processo (sem as variáveis), para o motivo da falha.
    command: String,
    title: String,
    snapshot: Option<Snapshot>,
}

/// O que o fim do primeiro turno precisa da largada do agente.
struct TurnEnd<'a> {
    out_dir: &'a Path,
    snapshot: Option<&'a Snapshot>,
    /// Agente do grafo do dono: segura o nó até a sessão sair.
    holds: bool,
    attempt: u32,
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
        "[orbit] agente pronto: contexto {} bytes, diff {}",
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
        .filter(|e| e.to == id && e.kind.carries_data())
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
        created: is_created(a, id),
        attempt: a.sched.attempt(id).unwrap_or(0),
    })
}

/// O nó foi criado por um agente neste run (não é do grafo do dono).
/// Os agentes ligados a `agent` por `talk`, nos dois sentidos, sem repetir.
fn talk_peers(edges: &[Edge], agent: &str) -> Vec<String> {
    let mut peers: Vec<String> = Vec::new();
    for e in edges.iter().filter(|e| e.kind == EdgeKind::Talk) {
        let other = if e.from == agent {
            &e.to
        } else if e.to == agent {
            &e.from
        } else {
            continue;
        };
        if other != agent && !peers.contains(other) {
            peers.push(other.clone());
        }
    }
    peers
}

fn is_created(a: &Active, id: &str) -> bool {
    a.sched.run.created.iter().any(|c| c.node.id == id)
}

/// O dono marcou "Não criar nós" no agente.
fn no_create(a: &Active, id: &str) -> bool {
    a.graph.nodes.iter().any(|n| {
        n.id == id
            && matches!(
                n.kind,
                NodeKind::Agent {
                    no_create: true,
                    ..
                }
            )
    })
}

/// As ligações que dão permissão agora (conversa, memória, controla, o
/// estado de outro nó): as do grafo salvo — o dono liga e desliga no canvas
/// durante o run, e vale na hora — mais as dos nós criados no run, que só
/// entram no salvo com o "Manter".
fn current_edges(saved: Option<&Graph>, created: &[Created]) -> Vec<Edge> {
    let mut edges: Vec<Edge> = saved.map(|g| g.edges.clone()).unwrap_or_default();
    for e in created.iter().flat_map(|c| c.edges.iter()) {
        if !edges.iter().any(|x| x.id == e.id) {
            edges.push(e.clone());
        }
    }
    edges
}

/// Pasta das execuções no perfil: `~/.tyba/orbit/runs` (dev: `~/.tyba-dev`).
/// A de quando o recurso se chamava Maestro (`~/.tyba/maestro/runs`) muda
/// para cá na primeira chamada.
pub fn default_root() -> Option<PathBuf> {
    crate::session::cwd::home().map(|h| {
        let profile = h.join(crate::profile::HOME_DIR);
        if let Err(e) = move_legacy_runs(&profile) {
            eprintln!("[orbit] mover a pasta de execuções antiga: {e}");
        }
        profile.join("orbit").join("runs")
    })
}

/// `<perfil>/maestro/runs` → `<perfil>/orbit/runs`, só quando a nova ainda não
/// existe (nunca mistura as duas). A `maestro/` vazia sai junto. Os caminhos
/// gravados nos runs já foram trocados na migração do banco (degrau 15).
fn move_legacy_runs(profile: &Path) -> std::io::Result<()> {
    let legacy = profile.join("maestro");
    let old = legacy.join("runs");
    let new = profile.join("orbit").join("runs");
    if !old.is_dir() || new.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(profile.join("orbit"))?;
    std::fs::rename(&old, &new)?;
    // Só se ficou vazia: o que mais houver lá não é desta migração.
    let _ = std::fs::remove_dir(&legacy);
    Ok(())
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
        tap: crate::pty::OutputTap,
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
                crate::session::SessionOwner::Orbit,
                Some(tap),
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
        on_state: Box<dyn Fn(PeerState) + Send>,
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
            orbit_tools: launch.tools,
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
            on_state,
        );
        Ok(session.id.to_string())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn spawn_agent(
        &self,
        _launch: AgentLaunch,
        _on_turn: Box<dyn FnOnce(AgentTurn) + Send>,
        _on_exit: Option<Box<dyn FnOnce() + Send>>,
        _on_state: Box<dyn Fn(PeerState) + Send>,
    ) -> Result<String, String> {
        Err("o nó de agente ainda não roda neste sistema".into())
    }

    fn send_input(&self, session_id: &str, text: &str) -> Result<(), String> {
        use tauri::Manager;
        let id = session_id
            .parse()
            .map_err(|_| format!("sessão inválida: {session_id}"))?;
        let state = self.app.state::<crate::AppState>();
        crate::inject_rich_input(&state, id, text, true)
    }

    fn multiline(&self, session_id: &str) -> bool {
        use tauri::Manager;
        let Ok(id) = session_id.parse() else {
            return false;
        };
        let state = self.app.state::<crate::AppState>();
        crate::rich_input_pastes(&state, id).unwrap_or(false)
    }

    fn owner_keys(&self, session_id: &str) -> talk::OwnerKeys {
        use tauri::Manager;
        self.app
            .state::<crate::AppState>()
            .owner_input
            .get(session_id)
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
        state
            .pty_pool
            .leader_pid(id)
            .map(crate::services::listen::group_ports)
            .unwrap_or_default()
    }

    fn resolve_service(&self, source: &ServiceSource) -> Result<ServiceReady, String> {
        service::resolve(&self.app, source)
    }

    fn emit(&self, run: &Run) {
        use tauri::Emitter;
        let _ = self.app.emit(&format!("orbit://run/{}", run.id), run);
    }

    fn browser(
        &self,
        graph_id: &str,
        node_id: &str,
        by: &str,
        action: AgentAction,
    ) -> Result<serde_json::Value, String> {
        use tauri::Manager;
        let key = crate::orbit::browser::ViewKey {
            graph: graph_id.to_string(),
            node: node_id.to_string(),
        };
        self.app
            .state::<crate::orbit::browser::BrowserViews>()
            .agent(&self.app, &key, by, action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_runs_folder_moves_to_orbit() {
        let profile = tempfile::tempdir().unwrap();
        let old = profile.path().join("maestro").join("runs").join("r1");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("log.txt"), "ok").unwrap();

        move_legacy_runs(profile.path()).unwrap();

        let moved = profile.path().join("orbit").join("runs").join("r1");
        assert_eq!(
            std::fs::read_to_string(moved.join("log.txt")).unwrap(),
            "ok"
        );
        assert!(!profile.path().join("maestro").exists());
        // Segunda chamada: nada a mover, nada quebra.
        move_legacy_runs(profile.path()).unwrap();
    }

    #[test]
    fn legacy_runs_folder_never_merges_into_an_existing_one() {
        let profile = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(profile.path().join("maestro").join("runs").join("velho")).unwrap();
        std::fs::create_dir_all(profile.path().join("orbit").join("runs").join("novo")).unwrap();

        move_legacy_runs(profile.path()).unwrap();

        assert!(profile.path().join("maestro/runs/velho").is_dir());
        assert!(!profile.path().join("orbit/runs/velho").exists());
    }
    use crate::orbit::{Edge, Position, Runner, Size, Trigger};
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
                locked: false,
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
    fn browser_takes_its_own_url_or_the_received_and_only_local() {
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
                    created: false,
                    attempt: 0,
                },
                own,
            )
        };
        std::fs::write(&received, "http://localhost:5173/\n").unwrap();
        // Sem url própria, vale a que chegou pela ligação.
        let o = start("", Some(received.clone()));
        assert!(o.ok);
        assert_eq!(o.url.as_deref(), Some("http://localhost:5173/"));
        // Com url própria, ela vale: a ligação só fez esperar.
        let o = start("https://code.muriki.test", Some(received.clone()));
        assert_eq!(o.url.as_deref(), Some("https://code.muriki.test"));
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
    fn empty_variable_refuses_the_run_citing_the_node_title() {
        let mut build = process("p", "cargo build --bin {{alvo}}");
        build.title = "Build do app".into();
        let mut g = graph(vec![build], vec![]);
        let err = missing_variable(&g, &[]).unwrap_err();
        assert!(
            err.contains("\"Build do app\"") && err.contains("{{alvo}}"),
            "{err}"
        );
        // Valor em branco também conta como sem valor.
        g.variables.insert("alvo".into(), "  ".into());
        assert!(missing_variable(&g, &[]).is_err());
        g.variables.insert("alvo".into(), "tyba".into());
        assert_eq!(missing_variable(&g, &[]), Ok(()));
        // Padrão no próprio texto basta; `awk '{{print $1}}'` não é variável.
        let g = graph(
            vec![process("p", "echo {{modo=dev}}; awk '{{print $1}}' x")],
            vec![],
        );
        assert_eq!(missing_variable(&g, &[]), Ok(()));
        // Script de integração e prompt de agente também.
        let i = Integration {
            id: "i-1".into(),
            name: "avisar".into(),
            script: "curl {{hook}}".into(),
            trigger: Trigger::Manual,
            output: OutputKind::Json,
            file: None,
        };
        let mut n = node(
            "n",
            NodeKind::Integration {
                integration_id: "i-1".into(),
                dir: String::new(),
            },
        );
        n.title = "Avisar".into();
        let err = missing_variable(&graph(vec![n], vec![]), &[&i]).unwrap_err();
        assert!(err.starts_with("\"Avisar\""), "{err}");
    }

    #[test]
    fn process_without_command_refuses_the_run_citing_the_node_title() {
        for command in ["", "  \n "] {
            let mut p = process("p", command);
            p.title = "Processo".into();
            let err = missing_variable(&graph(vec![p], vec![]), &[]).unwrap_err();
            assert!(
                err.starts_with("\"Processo\"") && err.contains("falta o comando"),
                "{err}"
            );
        }
    }

    #[test]
    fn begin_refuses_empty_variable_and_numbers_runs_stably() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(vec![process("p", "echo {{quem}}")], vec![]);
        let err = runs.start_now(g.clone(), vec![]).unwrap_err();
        assert!(err.contains("\"p\"") && err.contains("{{quem}}"), "{err}");

        let mut g = g;
        g.variables.insert("quem".into(), "dono".into());
        let mut numbers = Vec::new();
        for _ in 0..(KEEP_RUNS + 3) {
            let run = runs.start_now(g.clone(), vec![]).unwrap();
            let run = wait_finished(&runs, &run.id);
            numbers.push(run.number);
        }
        let want: Vec<u32> = (1..=(KEEP_RUNS as u32 + 3)).collect();
        assert_eq!(numbers, want, "a poda não renumera");
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
        /// O `on_exit` de cada sessão de agente.
        exits: Mutex<HashMap<String, Box<dyn FnOnce() + Send>>>,
        /// Sessões de agente que saíram sozinhas.
        gone: Mutex<Vec<String>>,
        /// O `on_state` de cada sessão de agente, com o título do nó.
        states: Mutex<HashMap<String, (String, Box<dyn Fn(PeerState) + Send>)>>,
        /// O que o core escreveu no terminal de cada agente (`send_input`).
        inputs: Mutex<Vec<(String, String)>>,
        /// Quando presente, o `send_input` falha com este motivo.
        send_error: Mutex<Option<String>>,
        /// A sessão cola várias linhas (bracketed paste).
        multiline: Mutex<bool>,
        /// O teclado do dono em cada sessão.
        owner: talk::OwnerInput,
        /// As ferramentas de navegador que chegaram ao host: (nó, quem, ação).
        browser: Mutex<Vec<(String, String, AgentAction)>>,
    }

    impl PtyHost {
        /// A sessão do agente de título `title` passa a `state` (o que o
        /// watcher real avisa a cada poll).
        fn set_state(&self, title: &str, state: PeerState) {
            let states = self.states.lock();
            let (_, on_state) = states
                .values()
                .find(|(t, _)| t == title)
                .expect("agente subiu");
            on_state(state);
        }

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
            on_state: Box<dyn Fn(PeerState) + Send>,
        ) -> Result<String, String> {
            self.agents.lock().push(launch.clone());
            let title = launch.title.clone();
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
            self.states
                .lock()
                .insert(session.clone(), (title, on_state));
            self.spawned.lock().push(session.clone());
            Ok(session)
        }

        fn spawn_process(
            &self,
            launch: ProcessLaunch,
            tap: crate::pty::OutputTap,
            on_exit: Box<dyn FnOnce() + Send>,
        ) -> Result<String, String> {
            let mut cmd = portable_pty::CommandBuilder::new(&launch.program);
            cmd.args(&launch.args);
            cmd.cwd(&launch.cwd);
            let id = uuid::Uuid::new_v4();
            self.pool.set_tap(id, tap);
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

        fn send_input(&self, session_id: &str, text: &str) -> Result<(), String> {
            if let Some(e) = self.send_error.lock().clone() {
                return Err(e);
            }
            self.inputs
                .lock()
                .push((session_id.to_string(), text.to_string()));
            // Como o CLI de verdade: o comando de sair fecha a sessão.
            if text == "/exit" {
                self.exit_agent(session_id);
            }
            Ok(())
        }

        fn multiline(&self, _session_id: &str) -> bool {
            *self.multiline.lock()
        }

        fn owner_keys(&self, session_id: &str) -> talk::OwnerKeys {
            self.owner.get(session_id)
        }

        fn browser(
            &self,
            _graph_id: &str,
            node_id: &str,
            by: &str,
            action: AgentAction,
        ) -> Result<serde_json::Value, String> {
            self.browser
                .lock()
                .push((node_id.to_string(), by.to_string(), action));
            Ok(serde_json::json!({ "ok": true }))
        }
    }

    /// O run começado e com a primeira leva de nós já subida (o que o
    /// `start` fazia antes de soltar o clique).
    impl<H: RunHost> Runs<H> {
        fn start_now(&self, graph: Graph, integrations: Vec<Integration>) -> Result<Run, String> {
            let (run, launching) = self.start_launching(graph, integrations)?;
            launching.join().unwrap();
            Ok(self.get(&run.id).unwrap_or(run))
        }

        fn test_integration_now(
            &self,
            graph: Graph,
            integration: Integration,
            node_id: &str,
        ) -> Result<Run, String> {
            let (run, launching) = self.test_integration_launching(graph, integration, node_id)?;
            launching.join().unwrap();
            Ok(self.get(&run.id).unwrap_or(run))
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
            states: Mutex::new(HashMap::new()),
            inputs: Mutex::new(Vec::new()),
            send_error: Mutex::new(None),
            multiline: Mutex::new(true),
            owner: talk::OwnerInput::default(),
            browser: Mutex::new(Vec::new()),
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
        let run = runs.start_now(g, vec![]).unwrap();
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

    /// Sem `| tee`: o comando vê um terminal (cor, sem buffer de pipe) e o
    /// core grava o `log.txt` do fluxo — sem ANSI e redigido. A url que ele
    /// imprime vira a url do nó com ele rodando, sem esperar a varredura.
    #[cfg(unix)]
    #[test]
    fn process_runs_on_a_real_tty_and_its_printed_url_wins() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        // A varredura acharia outra porta: a impressa chega antes e vale.
        *host.ports.lock() = vec![2019];
        let command = format!(
            "[ -t 1 ] && echo terminal; printf '\\033[32mchave %s\\033[0m\\r\\n' {key}; \
             printf '  Local:   \\033[36mhttp://0.0.0.0:\\033[1m5999\\033[22m/\\033[39m\\n'; sleep 30",
            key = fake_key()
        );
        let g = graph(vec![process("p", &command)], vec![]);
        let run = runs.start_now(g, vec![]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let url = loop {
            let r = runs.get(&run.id).unwrap();
            if let Some(url) = r.nodes["p"].url.clone() {
                break url;
            }
            assert!(Instant::now() < deadline, "sem url: {r:?}");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(url, "http://localhost:5999/");
        let out = dir.path().join("runs").join(&run.id).join("p").join("out");
        assert_eq!(
            std::fs::read_to_string(out.join("url.txt")).unwrap(),
            "http://localhost:5999/"
        );
        let log = std::fs::read_to_string(out.join("log.txt")).unwrap();
        assert!(log.starts_with("terminal\n"), "{log:?}");
        assert!(!log.contains('\x1b'), "{log:?}");
        assert!(!log.contains(&fake_key()), "{log:?}");
        assert!(log.contains("Local:   http://0.0.0.0:5999/"), "{log:?}");
        runs.stop(&run.id).unwrap();
    }

    /// Sem url impressa: a porta da varredura do process group, depois de
    /// aparecer em duas varreduras seguidas — a primeira que abriu, não a
    /// menor.
    #[test]
    fn process_without_printed_url_takes_the_first_scanned_port() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        *host.ports.lock() = vec![8080];
        let g = graph(vec![process("p", "echo subiu; sleep 30")], vec![]);
        let run = runs.start_now(g, vec![]).unwrap();
        std::thread::sleep(Duration::from_millis(1300));
        *host.ports.lock() = vec![3000, 8080];
        let deadline = Instant::now() + Duration::from_secs(10);
        let url = loop {
            let r = runs.get(&run.id).unwrap();
            if let Some(url) = r.nodes["p"].url.clone() {
                break url;
            }
            assert!(Instant::now() < deadline, "sem url: {r:?}");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(url, "http://localhost:8080");
        runs.stop(&run.id).unwrap();
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
        let out = std::process::Command::new(&program)
            .args(&args)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("HOME", dir.path())
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(read_exit(&out_dir), Some(0));
        // Sem `| tee`: a saída vai direto para o terminal (o `log.txt` o core
        // grava do PTY).
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "achei");
        assert!(!out_dir.join("log.txt").exists());
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
            .start_now(graph(vec![process("p", command)], vec![]), vec![])
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
        let run = runs.start_now(g, vec![]).unwrap();
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
            .start_now(graph(vec![process("p", "sleep 30")], vec![]), vec![])
            .unwrap();
        let session = run.nodes["p"].session_id.clone().unwrap();
        runs.stop(&run.id).unwrap();
        let run = runs.get(&run.id).unwrap();
        assert_eq!(run.status, RunStatus::Stopped);
        assert_eq!(run.nodes["p"].status, NodeStatus::Stopped);
        assert!(!host.pool.is_alive(session.parse().unwrap()));
    }

    /// Apagar um nó rodando: para só ele; o resto do run segue.
    #[cfg(unix)]
    #[test]
    fn stop_one_kills_only_that_node() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let run = runs
            .start_now(
                graph(
                    vec![process("p", "sleep 30"), process("q", "sleep 30")],
                    vec![],
                ),
                vec![],
            )
            .unwrap();
        let p = run.nodes["p"].session_id.clone().unwrap();
        let q = run.nodes["q"].session_id.clone().unwrap();
        runs.stop_one(&run.id, "p").unwrap();
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.nodes["p"].status, NodeStatus::Stopped);
        assert_eq!(r.nodes["q"].status, NodeStatus::Running);
        assert_eq!(r.status, RunStatus::Running);
        assert!(!host.pool.is_alive(p.parse().unwrap()));
        assert!(host.pool.is_alive(q.parse().unwrap()));
        // De novo: nada vivo, nada a fazer.
        runs.stop_one(&run.id, "p").unwrap();
        runs.stop(&run.id).unwrap();
        assert!(runs.stop_one("nao-existe", "p").is_err());
    }

    /// Run fechado, agente vivo depois do turno: apagar o nó mata a sessão.
    #[cfg(unix)]
    #[test]
    fn stop_one_kills_the_live_agent_of_a_closed_run() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let run = runs
            .start_now(graph(vec![agent("a", "x")], vec![]), vec![])
            .unwrap();
        let run = wait_finished(&runs, &run.id);
        let session = host.spawned.lock()[0].clone();
        assert_eq!(run.live_agents, vec![session.clone()]);
        runs.stop_one(&run.id, "a").unwrap();
        assert!(host.killed.lock().contains(&session));
        assert!(runs.get(&run.id).unwrap().live_agents.is_empty());
    }

    fn agent(id: &str, prompt: &str) -> Node {
        node(
            id,
            NodeKind::Agent {
                runner: Runner::Claude,
                role: String::new(),
                prompt: prompt.into(),
                dir: String::new(),
                no_create: false,
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
        let run = runs.start_now(g, vec![]).unwrap();
        // Os agentes publicam no fim do turno e seguram o run vivos.
        wait_status(&runs, &run.id, "q", NodeStatus::Done);
        wait_orchestrating(&runs, &run.id, "a");
        let run = wait_orchestrating(&runs, &run.id, "b");
        assert_eq!(run.status, RunStatus::Running, "{run:?}");
        assert_eq!(run.nodes["q"].status, NodeStatus::Done, "{run:?}");

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

        // Agentes vivos: o Parar os mata.
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
        let run = runs.start_now(g, vec![]).unwrap();
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
        let run = runs.start_now(g, vec![]).unwrap();
        let run = wait_orchestrating(&runs, &run.id, "a");
        assert!(run.nodes["a"].orchestrating, "{run:?}");

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
        let run = runs.start_now(g, integrations).unwrap();

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
            .test_integration_now(g.clone(), first.clone(), "m")
            .unwrap_err();
        assert!(err.contains("ainda não rodou"), "{err}");

        let run = runs.start_now(g.clone(), vec![first.clone()]).unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Done, "{run:?}");

        // O script editado (manual, inclusive) roda na hora com o in/ de antes,
        // sem o processo de origem rodar de novo.
        let edited = Integration {
            script: "echo testado; cat \"$TYBA_IN/log.txt\"".into(),
            trigger: Trigger::Manual,
            ..first
        };
        let test = runs.test_integration_now(g, edited, "m").unwrap();
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
        let run = runs.start_now(g, vec![i]).unwrap();
        runs.run_node(&run.id, "m").unwrap();
        let run = wait_finished(&runs, &run.id);
        assert_eq!(run.status, RunStatus::Done, "{run:?}");
        let diff = std::fs::read_to_string(&run.nodes["m"].outputs[&EdgeKind::Diff]).unwrap();
        assert!(diff.contains("gerado.txt"), "{diff}");
    }

    /// O ▶ volta com o run rodando antes de qualquer nó subir, e os nós sobem
    /// juntos: um agente preso no spawn não segura o outro.
    #[cfg(unix)]
    #[test]
    fn start_returns_before_the_nodes_launch_and_they_launch_in_parallel() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let (entered_tx, entered) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        *host.hold_spawn.lock() = Some((entered_tx, release_rx));

        let (started_tx, started) = std::sync::mpsc::channel();
        {
            let runs = Arc::clone(&runs);
            std::thread::spawn(move || {
                let g = graph(vec![agent("a", "x"), agent("b", "x")], vec![]);
                started_tx.send(runs.start(g, vec![])).unwrap();
            });
        }
        let run = started
            .recv_timeout(Duration::from_secs(20))
            .expect("o start esperou os nós subirem")
            .unwrap();
        assert_eq!(run.status, RunStatus::Running);
        entered.recv_timeout(Duration::from_secs(20)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while host.spawned.lock().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            host.spawned.lock().len(),
            1,
            "o segundo agente esperou o primeiro"
        );
        release.send(()).unwrap();
        while host.spawned.lock().len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(host.spawned.lock().len(), 2);
        runs.stop(&run.id).unwrap();
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
            std::thread::spawn(move || runs.start_now(graph(vec![agent("a", "x")], vec![]), vec![]))
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

        let first = runs.start_now(g.clone(), vec![]).unwrap();
        let first = wait_orchestrating(&runs, &first.id, "a");
        assert_eq!(first.status, RunStatus::Running, "{first:?}");
        let session = host.spawned.lock()[0].clone();
        // Agente vivo depois do turno: a tela sabe e o Parar segue valendo.
        assert_eq!(first.live_agents, vec![session.clone()]);
        assert_eq!(runs.list(&g.id)[0].live_agents, vec![session.clone()]);

        // Novo ▶ do mesmo grafo: o claude do run anterior morre antes.
        let second = runs.start_now(g.clone(), vec![]).unwrap();
        assert!(host.killed.lock().contains(&session));
        assert!(runs.get(&first.id).unwrap().live_agents.is_empty());
        let second = wait_orchestrating(&runs, &second.id, "a");
        let other = host.spawned.lock()[1].clone();
        assert_eq!(second.live_agents, vec![other.clone()]);
        assert!(!host.killed.lock().contains(&other));

        // Outro grafo não mexe nos agentes deste.
        let mut h = graph(vec![note("n")], vec![]);
        h.id = "g-2".into();
        runs.start_now(h, vec![]).unwrap();
        assert!(!host.killed.lock().contains(&other));
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_versions_grow_and_store_keeps_the_last() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let run = runs
            .start_now(graph(vec![process("p", "true"), note("n")], vec![]), vec![])
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
        let stored = runs.store.get_orbit_run(&run.id).unwrap().unwrap();
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
            .start_now(
                graph(vec![integration_node("m", "i")], vec![]),
                vec![integration("i", "true", Trigger::Manual)],
            )
            .unwrap();
        for _ in 0..KEEP_RUNS + 2 {
            runs.start_now(graph(vec![note("n")], vec![]), vec![])
                .unwrap();
        }
        let got = runs.get(&waiting.id).unwrap();
        assert_eq!(got.status, RunStatus::Running);
        assert!(root.join(&waiting.id).join("m").is_dir(), "pasta podada");
        assert!(
            runs.store.get_orbit_run(&waiting.id).unwrap().is_some(),
            "linha podada"
        );
        // A poda segue valendo para os outros.
        assert_eq!(runs.list("g-1").len(), KEEP_RUNS);
    }

    #[cfg(unix)]
    #[test]
    fn deleting_a_graph_refuses_an_open_run_then_drops_its_runs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("runs");
        let (runs, _host, _app) = pty_runs(root.clone());
        // Integração manual: o run fica rodando, esperando o ▶ do nó.
        let waiting = runs
            .start_now(
                graph(vec![integration_node("m", "i")], vec![]),
                vec![integration("i", "true", Trigger::Manual)],
            )
            .unwrap();
        assert!(runs.forget_graph("g-1").is_err(), "execução aberta");
        assert!(root.join(&waiting.id).is_dir());
        runs.stop(&waiting.id).unwrap();
        let done = runs
            .start_now(graph(vec![note("n")], vec![]), vec![])
            .unwrap();
        wait_finished(&runs, &done.id);
        assert_eq!(runs.forget_graph("outro"), Ok(()), "outro grafo não mexe");
        assert_eq!(runs.list("g-1").len(), 2);
        assert_eq!(runs.forget_graph("g-1"), Ok(()));
        assert!(runs.list("g-1").is_empty());
        assert!(!root.join(&waiting.id).exists(), "pasta ficou");
        assert!(!root.join(&done.id).exists(), "pasta ficou");
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
        let run = runs.start_now(g, vec![]).unwrap();
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
        let run = wait_finished(&runs, &runs.start_now(g, vec![]).unwrap().id);
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
        let run = wait_finished(&runs, &runs.start_now(g, vec![]).unwrap().id);
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
                no_create: false,
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
        let run = runs.start_now(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        // Quem depende dele recebe o contexto do turno e fecha o dele.
        let r = wait_orchestrating(&runs, &run.id, "a");
        assert!(
            r.nodes["a"].outputs.contains_key(&EdgeKind::Context),
            "{r:?}"
        );
        assert_eq!(r.nodes["o"].status, NodeStatus::Running, "{r:?}");
        assert!(r.nodes["o"].outputs.contains_key(&EdgeKind::Context));
        assert_eq!(r.status, RunStatus::Running, "{r:?}");

        // Run aberto: o orquestrador ainda cria nós.
        let tools = tools_of(&host, "o");
        let created = tools
            .call(
                "orbit_create_browser",
                &json!({"title": "b", "url": "http://localhost:1"}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        let r = wait_status(&runs, &run.id, &created, NodeStatus::Done);
        assert_eq!(r.nodes[&created].status, NodeStatus::Done, "{r:?}");
        assert_eq!(r.status, RunStatus::Running);

        // As sessões saem: os nós ficam prontos e o run fecha.
        host.exit_agent(&r.nodes["a"].session_id.clone().unwrap());
        let r = wait_status(&runs, &run.id, "a", NodeStatus::Done);
        assert_eq!(r.status, RunStatus::Running, "{r:?}");
        let session = r.nodes["o"].session_id.clone().unwrap();
        host.exit_agent(&session);
        let r = wait_finished(&runs, &run.id);
        assert_eq!(r.status, RunStatus::Done, "{r:?}");
        assert_eq!(r.nodes["o"].status, NodeStatus::Done);
        assert!(!r.nodes["o"].orchestrating);
        assert!(r.nodes["o"].outputs.contains_key(&EdgeKind::Context));
        // Fechado: nada mais se cria, mas o criado ainda pode ser mantido.
        let keep = runs.created_to_keep(&run.id).unwrap();
        assert_eq!(
            keep.iter().map(|c| c.node.id.as_str()).collect::<Vec<_>>(),
            vec![created.as_str()]
        );
        let err = tools
            .call(
                "orbit_create_browser",
                &json!({"title": "tarde", "url": "http://localhost:1"}),
            )
            .unwrap_err();
        assert!(err.contains("já terminou"), "{err}");
    }

    /// Todo agente do grafo do dono segura o run depois do turno (pode
    /// criar nós e reescrever o plano), e a saída da sessão o fecha.
    #[cfg(unix)]
    #[test]
    fn agent_holds_the_run_until_its_session_exits() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let run = runs
            .start_now(graph(vec![agent("a", "x")], vec![]), vec![])
            .unwrap();
        let r = wait_orchestrating(&runs, &run.id, "a");
        assert_eq!(r.status, RunStatus::Running, "{r:?}");
        assert_eq!(r.nodes["a"].status, NodeStatus::Running);
        assert!(r.nodes["a"].outputs.contains_key(&EdgeKind::Context));
        let session = host.spawned.lock()[0].clone();
        assert_eq!(r.live_agents, vec![session.clone()]);

        // A sessão sai: o nó fica pronto, o run fecha sem ela e o banco
        // também fica sabendo.
        host.exit_agent(&session);
        let last = wait_finished(&runs, &run.id);
        assert_eq!(last.status, RunStatus::Done, "{last:?}");
        assert_eq!(last.nodes["a"].status, NodeStatus::Done);
        assert!(last.live_agents.is_empty(), "{last:?}");
        let stored = runs.store.get_orbit_run(&run.id).unwrap().unwrap();
        assert!(stored.live_agents.is_empty(), "{stored:?}");
    }

    /// Assentar e voltar no agendador: o relógio para no momento em que
    /// assentou, o tempo parado vai para `paused_ms`, e a sessão que sai
    /// assentada fecha o run concluído com o tempo ativo.
    #[test]
    fn settled_run_stops_the_clock_resumes_and_closes_done() {
        let g = graph(vec![orchestrator("a"), process("p", "x")], vec![]);
        let mut s = Scheduler::new(&g, "r".into(), 0);
        s.mark_running("a", 0);
        s.set_session("a", "sess-a".into());
        s.mark_running("p", 0);
        // Primeiro turno ainda aberto, e um processo rodando: não assenta.
        assert!(!s.only_live_agents());
        assert!(s.orchestrate("a", HashMap::new()));
        assert!(!s.only_live_agents());
        assert!(s.finish("p", ok(), 1_000));
        assert!(s.only_live_agents());

        assert!(s.set_settled(true, 10_000));
        assert_eq!(s.run().status, RunStatus::Running);
        assert_eq!(s.run().finished_at, Some(10_000));
        assert!(!s.set_settled(true, 11_000), "já assentado");
        assert!(s.set_settled(false, 15_000));
        assert_eq!(s.run().finished_at, None);
        assert_eq!(s.run().paused_ms, 5_000);
        assert!(s.set_settled(true, 20_000));

        // A sessão sai com o run assentado: pronto no momento em que assentou.
        assert!(s.finish("a", ok(), 30_000));
        let r = s.run();
        assert_eq!(r.status, RunStatus::Done);
        assert!(!r.settled);
        assert_eq!(r.finished_at, Some(20_000));
        assert_eq!(r.nodes["a"].finished_at, Some(20_000));
        // Tempo ativo: 20 s de relógio menos 5 s parado.
        assert_eq!(r.finished_at.unwrap() - r.started_at - r.paused_ms, 15_000);
        assert!(!s.set_settled(false, 31_000), "run fechado não muda");
    }

    /// Parar (ou o app fechar) com o run assentado: o trabalho já acabou —
    /// concluído, agentes prontos, sessões a matar e o relógio onde parou.
    #[test]
    fn stop_and_boot_close_a_settled_run_as_done() {
        let g = graph(vec![orchestrator("a"), note("n")], vec![]);
        let mut s = Scheduler::new(&g, "r".into(), 0);
        s.mark_running("a", 0);
        s.set_session("a", "sess-a".into());
        s.mark_running("n", 0);
        assert!(s.finish("n", ok(), 1));
        assert!(s.orchestrate("a", HashMap::new()));
        assert!(s.set_settled(true, 5_000));
        let mut booted = s.snapshot();

        assert_eq!(s.stop(9_000), vec!["sess-a".to_string()]);
        let r = s.run();
        assert_eq!(r.status, RunStatus::Done);
        assert_eq!(r.finished_at, Some(5_000));
        assert_eq!(r.nodes["a"].status, NodeStatus::Done);
        assert!(!r.nodes["a"].orchestrating);
        assert!(!r.settled);

        booted.close_interrupted(9_000);
        assert_eq!(booted.status, RunStatus::Done);
        assert_eq!(booted.finished_at, Some(5_000));
        assert_eq!(booted.nodes["a"].status, NodeStatus::Done);
        assert!(booted.nodes["a"].reason.is_none());
        assert!(!booted.settled);
    }

    /// Espera o run assentar (`true`) ou sair do assentado.
    fn wait_settled(runs: &Runs<Arc<PtyHost>>, run: &str, want: bool) -> Run {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let r = runs.get(run).unwrap();
            if r.settled == want || Instant::now() > deadline {
                return r;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// O run assenta quando todo agente fecha o turno há um tempo e nada mais
    /// acontece, e volta a "ao vivo" na mesma execução por turno, mensagem
    /// ou processo; o relógio para e retoma com o tempo ativo. Parar
    /// assentado fecha concluído.
    #[cfg(unix)]
    #[test]
    fn run_settles_when_all_rests_and_wakes_by_turn_message_and_process() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        runs.settle_ms
            .store(150, std::sync::atomic::Ordering::Relaxed);
        let mut a = orchestrator("a");
        a.title = "Front".into();
        let mut b = orchestrator("b");
        b.title = "Revisor".into();
        let g = graph(vec![a, b], vec![edge("t1", "a", "b", EdgeKind::Talk)]);
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g, vec![]).unwrap();
        for id in ["a", "b"] {
            wait_orchestrating(&runs, &run.id, id);
        }
        let rest = || {
            std::thread::sleep(Duration::from_millis(200));
            // O poll do watcher, que repete o estado.
            host.set_state("Front", PeerState::Ready);
            host.set_state("Revisor", PeerState::Ready);
        };

        // Turno fechado há pouco: ainda ao vivo.
        host.set_state("Front", PeerState::Ready);
        host.set_state("Revisor", PeerState::Ready);
        assert!(!runs.get(&run.id).unwrap().settled);
        rest();
        let r = runs.get(&run.id).unwrap();
        assert!(r.settled, "{r:?}");
        assert_eq!(r.status, RunStatus::Running, "sessões abertas por baixo");
        assert_eq!(r.live_agents.len(), 2);
        let frozen = r.finished_at.expect("relógio parado");
        assert!(runs.store.get_orbit_run(&run.id).unwrap().unwrap().settled);
        rest();
        assert_eq!(runs.get(&run.id).unwrap().finished_at, Some(frozen));

        // Por turno: o dono digita e o Front trabalha.
        host.set_state("Front", PeerState::Working);
        let r = runs.get(&run.id).unwrap();
        assert!(!r.settled, "{r:?}");
        assert_eq!(r.finished_at, None);
        let paused = r.paused_ms;
        assert!(paused >= 200, "{paused}");
        host.set_state("Front", PeerState::Ready);
        rest();
        assert!(runs.get(&run.id).unwrap().settled);
        std::thread::sleep(Duration::from_millis(60));

        // Por mensagem: na fila, ao vivo na hora; entregue, o turno fechado
        // conta de novo.
        tools_of(&host, "Front")
            .call("orbit_send", &json!({"to": "b", "text": "revise"}))
            .unwrap();
        assert!(!runs.get(&run.id).unwrap().settled);
        host.set_state("Revisor", PeerState::Ready);
        assert!(host
            .inputs
            .lock()
            .iter()
            .any(|(_, t)| t.contains("> revise")));
        assert!(!runs.get(&run.id).unwrap().settled);
        rest();
        let r = runs.get(&run.id).unwrap();
        assert!(r.settled, "{r:?}");
        assert!(r.paused_ms >= paused + 60, "{} {paused}", r.paused_ms);

        // Por processo: um nó novo rodando tira do assentado; parado, volta.
        let work = dir.path().join("api");
        std::fs::create_dir(&work).unwrap();
        let got = tools_of(&host, "Front")
            .call(
                "orbit_create_process",
                &json!({"title": "api", "command": "sleep 30", "dir": work}),
            )
            .unwrap();
        let created = got["node_id"].as_str().unwrap().to_string();
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.nodes[&created].status, NodeStatus::Running);
        assert!(!r.settled, "{r:?}");
        runs.stop_one(&run.id, &created).unwrap();
        let r = wait_settled(&runs, &run.id, true);
        assert!(r.settled, "{r:?}");
        let frozen = r.finished_at.unwrap();

        // Parar assentado: concluída, agentes prontos, sessões fechadas.
        runs.stop(&run.id).unwrap();
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.status, RunStatus::Done, "{r:?}");
        assert_eq!(r.finished_at, Some(frozen));
        assert_eq!(r.nodes["a"].status, NodeStatus::Done);
        assert_eq!(r.nodes["b"].status, NodeStatus::Done);
        assert!(r.live_agents.is_empty(), "{r:?}");
        let stored = runs.store.get_orbit_run(&run.id).unwrap().unwrap();
        assert_eq!(stored.status, RunStatus::Done);
        assert!(!stored.settled);
    }

    /// `orbit_finish` fecha só a sessão de quem chama, depois do turno
    /// fechar; quando a última sai, o run fica concluído.
    #[cfg(unix)]
    #[test]
    fn orbit_finish_closes_only_the_callers_session() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut a = orchestrator("a");
        a.title = "Front".into();
        let mut b = orchestrator("b");
        b.title = "Revisor".into();
        let run = runs.start_now(graph(vec![a, b], vec![]), vec![]).unwrap();
        for id in ["a", "b"] {
            wait_orchestrating(&runs, &run.id, id);
        }
        let r = runs.get(&run.id).unwrap();
        let (sa, sb) = (
            r.nodes["a"].session_id.clone().unwrap(),
            r.nodes["b"].session_id.clone().unwrap(),
        );

        // O dono disse que acabou: o Front trabalha e chama a ferramenta.
        host.set_state("Front", PeerState::Ready);
        host.set_state("Front", PeerState::Working);
        let got = tools_of(&host, "Front")
            .call("orbit_finish", &json!({}))
            .unwrap();
        assert_eq!(got["finishing"], true);
        // A resposta ainda tem de chegar: nada sai no meio do turno.
        std::thread::sleep(Duration::from_millis(400));
        assert!(host.inputs.lock().is_empty());
        assert!(host.alive(&sa));

        // Turno fechado: sai como o /exit do claude, só ele.
        host.set_state("Front", PeerState::Ready);
        let r = wait_status(&runs, &run.id, "a", NodeStatus::Done);
        assert_eq!(r.nodes["a"].status, NodeStatus::Done, "{r:?}");
        assert_eq!(*host.inputs.lock(), vec![(sa.clone(), "/exit".to_string())]);
        assert_eq!(r.nodes["b"].status, NodeStatus::Running);
        assert!(host.alive(&sb));
        assert!(!host.killed.lock().contains(&sb));
        assert_eq!(r.status, RunStatus::Running);
        assert_eq!(r.live_agents, vec![sb.clone()]);

        // A última sessão fecha: o run fica concluído.
        host.set_state("Revisor", PeerState::Working);
        tools_of(&host, "Revisor")
            .call("orbit_finish", &json!({}))
            .unwrap();
        host.set_state("Revisor", PeerState::Ready);
        let r = wait_finished(&runs, &run.id);
        assert_eq!(r.status, RunStatus::Done, "{r:?}");
        assert!(r.live_agents.is_empty());
        // A sessão fechada não fecha de novo.
        let err = tools_of(&host, "Front")
            .call("orbit_finish", &json!({}))
            .unwrap_err();
        assert!(err.contains("não está aberta"), "{err}");
    }

    /// As ferramentas que o run entregou à sessão do orquestrador.
    fn tools_of(host: &PtyHost, title: &str) -> ToolHandler {
        let launches = host.agents.lock().clone();
        let launch = launches.iter().find(|l| l.title == title).unwrap();
        launch.tools.clone().expect("orquestrador sem ferramentas")
    }

    #[cfg(unix)]
    #[test]
    fn created_process_that_dies_returns_the_reason_and_the_log_tail() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("api");
        std::fs::create_dir(&work).unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(vec![orchestrator("o"), process("p", "sleep 30")], vec![]);
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        let tools = tools_of(&_host, "o");
        // A porta fica ocupada por este teste; o processo imita o Python que
        // não consegue o bind (mensagem escrita à mão, sem a porta).
        let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = busy.local_addr().unwrap().port();
        let command = format!(
            "echo 'Traceback (most recent call last):' >&2; \
             echo 'OSError: [Errno 48] Address already in use' >&2; exit 1 # http.server {port}"
        );
        let out = tools
            .call(
                "orbit_create_process",
                &json!({"title": format!("resumo (:{port})"), "command": command, "dir": work}),
            )
            .unwrap();
        assert_eq!(out["failed"], true, "{out}");
        assert_eq!(out["status"], "failed", "{out}");
        assert_eq!(out["exit_code"], 1, "{out}");
        let reason = out["reason"].as_str().unwrap();
        assert!(
            reason.starts_with(&format!("porta {port} em uso")),
            "{reason}"
        );
        #[cfg(target_os = "macos")]
        assert!(
            reason.ends_with(&format!("(pid {})", std::process::id())),
            "{reason}"
        );
        assert!(
            out["log"]
                .as_str()
                .unwrap()
                .contains("Address already in use"),
            "{out}"
        );
        assert!(out["next"].as_str().unwrap().contains("orbit_stop_node"));
        // O card lê o mesmo fim do log com a sessão já morta.
        let id = out["node_id"].as_str().unwrap();
        let lines = runs.node_log(&run.id, id, 14).unwrap();
        assert_eq!(
            lines.last().unwrap(),
            "OSError: [Errno 48] Address already in use"
        );
        assert!(runs.node_log(&run.id, "nada", 14).is_err());
        // O nó quebrado se para sem erro.
        tools
            .call("orbit_stop_node", &json!({"node_id": id}))
            .unwrap();
        drop(busy);
        runs.stop(&run.id).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn orchestrator_creates_nodes_in_the_active_run_and_stop_leaves_them_out() {
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
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        // Todo agente do grafo do dono recebe as ferramentas, as de criar
        // inclusive.
        assert!(tools_of(&host, "a").can_create);
        let tools = tools_of(&host, "o");
        assert!(tools.can_create);

        let created = tools
            .call(
                "orbit_create_process",
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
                .call("orbit_node_status", &json!({"node_id": api}))
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
                "orbit_create_process",
                &json!({"title": "x", "command": "true", "dir": "/nao/existe/tyba-orq"}),
            )
            .unwrap_err();
        assert!(err.contains("não existe"), "{err}");
        let err = tools
            .call(
                "orbit_create_browser",
                &json!({"title": "x", "url": "https://example.com"}),
            )
            .unwrap_err();
        assert!(err.contains("só local"), "{err}");
        let err = tools
            .call(
                "orbit_create_browser",
                &json!({"title": "x", "from_node": "a"}),
            )
            .unwrap_err();
        assert!(err.contains("não produz url"), "{err}");
        let err = tools
            .call("orbit_stop_node", &json!({"node_id": "p"}))
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
                "orbit_create_browser",
                &json!({"title": "code", "from_node": api}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        let r = wait_status(&runs, &run.id, &code, NodeStatus::Done);
        assert_eq!(r.nodes[&code].url.as_deref(), Some("http://localhost:5180"));
        let edges = &r.created.iter().find(|c| c.node.id == code).unwrap().edges;
        assert_eq!(edges.len(), 2);
        assert_eq!(
            (edges[0].from.as_str(), edges[0].kind),
            (api.as_str(), EdgeKind::Url)
        );
        // E o orquestrador ganha o controle do navegador que criou.
        assert_eq!(
            (edges[1].from.as_str(), edges[1].to.as_str(), edges[1].kind),
            ("o", code.as_str(), EdgeKind::Control)
        );
        // Caddy: várias portas, e a url que sai do nó (a menor) não é o host
        // virtual. Com url própria, o navegador espera o nó (ligação url) e
        // abre a dele.
        let caddy = tools
            .call(
                "orbit_create_browser",
                &json!({
                    "title": "muriki",
                    "from_node": api,
                    "url": "https://code.muriki.test"
                }),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        let r = wait_status(&runs, &run.id, &caddy, NodeStatus::Done);
        assert_eq!(
            r.nodes[&caddy].url.as_deref(),
            Some("https://code.muriki.test")
        );
        let edges = &r.created.iter().find(|c| c.node.id == caddy).unwrap().edges;
        assert_eq!(
            (edges[0].from.as_str(), edges[0].kind),
            (api.as_str(), EdgeKind::Url)
        );
        let typed = tools
            .call(
                "orbit_create_browser",
                &json!({"title": "digitado", "url": "http://localhost:1"}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        let r = runs.get(&run.id).unwrap();
        let edges = &r.created.iter().find(|c| c.node.id == typed).unwrap().edges;
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::Control);
        // O controle não segura o navegador: ele roda sem esperar o orquestrador.
        wait_status(&runs, &run.id, &typed, NodeStatus::Done);

        // Teto de 30 por execução (já são 4).
        for i in 4..MAX_CREATED {
            tools
                .call(
                    "orbit_create_browser",
                    &json!({"title": format!("b{i}"), "url": "http://localhost:1"}),
                )
                .unwrap();
        }
        let err = tools
            .call(
                "orbit_create_browser",
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
            .call("orbit_stop_node", &json!({"node_id": api}))
            .unwrap();
        assert!(host.killed.lock().contains(&session));
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.nodes[&api].status, NodeStatus::Stopped);
        assert_eq!(r.status, RunStatus::Running);

        // ■ Parar: o run fecha e os criados ficam fora do grafo (até o dono
        // mandar manter).
        runs.stop(&run.id).unwrap();
        assert!(runs
            .created_to_keep(&run.id)
            .unwrap()
            .iter()
            .any(|c| c.node.id == api));
        let err = tools
            .call(
                "orbit_create_process",
                &json!({"title": "tarde", "command": "true", "dir": work}),
            )
            .unwrap_err();
        assert!(err.contains("já terminou"), "{err}");
        // O grafo salvo nunca recebeu os criados.
        let saved = runs.store.get_orbit_graph("g-1").unwrap().unwrap().value;
        assert_eq!(saved.nodes.len(), 3);
        assert_eq!(runs.get(&run.id).unwrap().status, RunStatus::Stopped);
    }

    /// O orquestrador delega a um agente criado e lê o resultado: a fala
    /// final (redigida) e o resumo do que mudou na pasta — sem caminho cru.
    #[cfg(unix)]
    #[test]
    fn node_status_of_a_created_agent_brings_its_answer_and_diff() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(vec![orchestrator("o"), process("p", "sleep 30")], vec![]);
        let run = runs.start_now(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        let tools = tools_of(&host, "o");
        let rev = tools
            .call(
                "orbit_create_agent",
                &json!({"title": "revisor", "runner": "claude", "prompt": "revise", "dir": repo}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        let r = wait_status(&runs, &run.id, &rev, NodeStatus::Done);
        assert_eq!(r.nodes[&rev].status, NodeStatus::Done, "{r:?}");

        let st = tools
            .call("orbit_node_status", &json!({"node_id": rev}))
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
        // Nenhum caminho para saída crua (patch, log, nota) sai da ferramenta.
        assert!(st.get("paths").is_none(), "{st}");
        let out = dir.path().join("runs");
        assert!(!st.to_string().contains(&*out.to_string_lossy()), "{st}");
        runs.stop(&run.id).unwrap();
    }

    /// Agente criado por outro: tem as ferramentas menos as de criar (o core
    /// recusa a chamada), e o plano dele é uma nota criada no run — ele não
    /// está no grafo salvo, então a nota também não vai para lá.
    #[cfg(unix)]
    #[test]
    fn created_agent_plans_in_the_run_and_never_creates() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo);
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(vec![orchestrator("o"), process("p", "sleep 30")], vec![]);
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g.clone(), vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        let rev = tools_of(&host, "o")
            .call(
                "orbit_create_agent",
                &json!({"title": "revisor", "runner": "claude", "prompt": "revise", "dir": repo}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        // Agente criado fecha no fim do turno (não segura o run).
        wait_status(&runs, &run.id, &rev, NodeStatus::Done);
        let tools = tools_of(&host, "revisor");
        assert!(!tools.can_create);
        for tool in [
            "orbit_create_process",
            "orbit_create_agent",
            "orbit_create_browser",
        ] {
            let err = tools
                .call(tool, &json!({"title": "x", "command": "true", "dir": repo}))
                .unwrap_err();
            assert!(err.contains("sem cascata"), "{tool}: {err}");
        }
        let created = runs.get(&run.id).unwrap().created.len();
        assert_eq!(created, 1);

        let step = |state: &str| json!({"steps": [{"text": "ler o diff", "state": state}]});
        let note = tools.call("orbit_plan", &step("doing")).unwrap()["note_id"]
            .as_str()
            .unwrap()
            .to_string();
        let r = runs.get(&run.id).unwrap();
        let c = r.created.iter().find(|c| c.node.id == note).unwrap();
        assert_eq!(c.created_by, rev);
        assert_eq!(
            (c.edges[0].from.as_str(), c.edges[0].kind),
            (rev.as_str(), EdgeKind::Plan)
        );
        assert!(r.plans.is_empty(), "{r:?}");
        // A segunda chamada reescreve a mesma nota.
        let again = tools.call("orbit_plan", &step("done")).unwrap()["note_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(again, note);
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.created.len(), 2);
        let NodeKind::Note { markdown, .. } = &r.created[1].node.kind else {
            panic!("{r:?}")
        };
        assert!(markdown.contains("[x] ler o diff"), "{markdown}");
        // O grafo salvo não recebeu a nota.
        let saved = runs.store.get_orbit_graph(&g.id).unwrap().unwrap().value;
        assert_eq!(saved.nodes.len(), 2);
        runs.stop(&run.id).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn keep_writes_created_nodes_into_the_graph_as_normal_nodes() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(vec![orchestrator("o"), process("p", "sleep 30")], vec![]);
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g.clone(), vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        let tools = tools_of(&host, "o");
        let web = tools
            .call(
                "orbit_create_process",
                &json!({"title": "web", "command": "sleep 30", "dir": dir.path()}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        let page = tools
            .call(
                "orbit_create_browser",
                &json!({"title": "page", "from_node": web}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();

        // O que o comando "Manter no grafo" faz.
        let keep = runs.created_to_keep(&run.id).unwrap();
        assert_eq!(keep.len(), 2);
        let mut graph = runs.store.get_orbit_graph(&g.id).unwrap().unwrap().value;
        orchestrator::merge_kept(&mut graph, &keep, &keep);
        crate::orbit::validate_graph(&graph, &[]).unwrap();
        runs.store.save_orbit_graph(&graph, true, None).unwrap();
        runs.mark_kept(
            &run.id,
            &keep.iter().map(|c| c.node.id.clone()).collect::<Vec<_>>(),
        );

        let saved = runs.store.get_orbit_graph(&g.id).unwrap().unwrap().value;
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

    /// "Manter no grafo" depois do ■ Parar: o retrato guardado ainda tem os
    /// criados, e a marca de mantido vai para ele.
    #[cfg(unix)]
    #[test]
    fn keep_works_after_the_run_stopped() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(vec![orchestrator("o"), process("p", "sleep 30")], vec![]);
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g.clone(), vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        let page = tools_of(&host, "o")
            .call(
                "orbit_create_browser",
                &json!({"title": "page", "url": "http://localhost:1"}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        runs.stop(&run.id).unwrap();
        assert_eq!(runs.get(&run.id).unwrap().status, RunStatus::Stopped);

        let keep = runs.created_to_keep(&run.id).unwrap();
        assert_eq!(keep.len(), 1);
        let mut graph = runs.store.get_orbit_graph(&g.id).unwrap().unwrap().value;
        orchestrator::merge_kept(&mut graph, &keep, &keep);
        runs.store.save_orbit_graph(&graph, true, None).unwrap();
        runs.mark_kept(&run.id, std::slice::from_ref(&page));

        let stored = runs.store.get_orbit_run(&run.id).unwrap().unwrap();
        assert_eq!(stored.status, RunStatus::Stopped);
        assert!(stored.created.iter().all(|c| c.kept), "{stored:?}");
        let last = host.runs.lock().last().cloned().unwrap();
        assert!(last.created.iter().all(|c| c.kept), "{last:?}");
        assert!(runs.created_to_keep(&run.id).unwrap().is_empty());
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
        store.save_orbit_run(&open).unwrap();
        let mut done = open.clone();
        done.id = "r-pronto".into();
        done.status = RunStatus::Done;
        store.save_orbit_run(&done).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs_on(dir.path().join("runs"), Arc::clone(&store));

        let r = store.get_orbit_run("r-aberto").unwrap().unwrap();
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
        assert_eq!(store.get_orbit_run("r-pronto").unwrap(), Some(done));
    }

    #[test]
    fn unsafe_node_id_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        assert!(runs
            .start_now(graph(vec![note("../fora")], vec![]), vec![])
            .is_err());
    }

    #[test]
    fn plan_and_control_edges_never_hold_the_target() {
        let g = graph(
            vec![
                node(
                    "o",
                    NodeKind::Agent {
                        runner: Runner::Claude,
                        role: String::new(),
                        prompt: "x".into(),
                        dir: String::new(),
                        no_create: false,
                    },
                ),
                note("n"),
                browser("b", "http://localhost:1"),
            ],
            vec![
                edge("e1", "o", "n", EdgeKind::Plan),
                edge("e2", "o", "b", EdgeKind::Control),
            ],
        );
        let mut s = Scheduler::new(&g, "r".into(), 0);
        assert_eq!(s.ready(), vec!["o", "n", "b"]);
        // Nó novo com controle também não espera.
        s.add_node("b2", &[("o".into(), EdgeKind::Control)])
            .unwrap();
        assert!(s.ready().contains(&"b2".to_string()));
    }

    /// `orbit_plan`: uma nota por orquestrador, no grafo salvo, com a ligação
    /// plan; a segunda chamada só reescreve o texto; o run guarda a cópia.
    /// `orbit_browser_open`: só no navegador ligado por `controla` ao agente;
    /// a url vira a do nó no run e no grafo salvo, com quem abriu. Sem `node`
    /// (o `$BROWSER`), o ligado mais recente.
    #[cfg(unix)]
    #[test]
    fn agent_opens_a_page_only_in_a_browser_linked_by_control() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut o = orchestrator("o");
        o.title = "Claude".into();
        let g = graph(
            vec![
                o,
                agent("x", "outro"),
                browser("b", "http://localhost:1"),
                browser("solto", "http://localhost:2"),
            ],
            vec![edge("c1", "o", "b", EdgeKind::Control)],
        );
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        wait_orchestrating(&runs, &run.id, "x");
        let tools = tools_of(&host, "Claude");

        let got = tools
            .call(
                "orbit_browser_open",
                &json!({"node": "b", "url": "http://localhost:5173/login"}),
            )
            .unwrap();
        assert_eq!(got["node_id"], "b");
        let r = runs.get(&run.id).unwrap();
        assert_eq!(
            r.nodes["b"].url.as_deref(),
            Some("http://localhost:5173/login")
        );
        let opened = r.nodes["b"].opened.clone().unwrap();
        assert_eq!(opened.by, "Claude");
        let saved = runs.store.get_orbit_graph("g-1").unwrap().unwrap();
        let b = saved.value.nodes.iter().find(|n| n.id == "b").unwrap();
        assert_eq!(
            b.kind,
            NodeKind::Browser {
                url: "http://localhost:5173/login".into()
            }
        );

        // Navegador sem ligação dele, url não local, file://: recusa.
        let err = tools
            .call(
                "orbit_browser_open",
                &json!({"node": "solto", "url": "http://localhost:5173"}),
            )
            .unwrap_err();
        assert!(err.contains("controla"), "{err}");
        let err = tools
            .call(
                "orbit_browser_open",
                &json!({"node": "b", "url": "https://example.com"}),
            )
            .unwrap_err();
        assert!(err.contains("só local"), "{err}");
        let err = tools
            .call(
                "orbit_browser_open",
                &json!({"node": "b", "url": "file:///tmp/index.html"}),
            )
            .unwrap_err();
        assert!(err.contains("sirva por localhost"), "{err}");
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.nodes["solto"].opened, None);
        assert_eq!(
            r.nodes["b"].url.as_deref(),
            Some("http://localhost:5173/login")
        );

        // Agente sem ligação: o `$BROWSER` dele recusa, não abre nada.
        let err = tools_of(&host, "x")
            .call("orbit_browser_open", &json!({"url": "http://localhost:3"}))
            .unwrap_err();
        assert!(err.contains("nenhum navegador ligado"), "{err}");

        // Sem node: o mais recente — o navegador que ele acabou de criar.
        let made = tools
            .call(
                "orbit_create_browser",
                &json!({"title": "novo", "url": "http://localhost:4"}),
            )
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        let got = tools
            .call(
                "orbit_browser_open",
                &json!({"url": "http://localhost:4/a"}),
            )
            .unwrap();
        assert_eq!(got["node_id"], made.as_str());
        // E depois o que abriu por último, mesmo sendo a ligação mais velha.
        std::thread::sleep(Duration::from_millis(5));
        tools
            .call(
                "orbit_browser_open",
                &json!({"node": "b", "url": "http://localhost:5173/x"}),
            )
            .unwrap();
        let got = tools
            .call(
                "orbit_browser_open",
                &json!({"url": "http://localhost:5173/y"}),
            )
            .unwrap();
        assert_eq!(got["node_id"], "b");
        // O criado muda só no run, não no grafo salvo.
        let r = runs.get(&run.id).unwrap();
        let c = r.created.iter().find(|c| c.node.id == made).unwrap();
        assert_eq!(
            c.node.kind,
            NodeKind::Browser {
                url: "http://localhost:4/a".into()
            }
        );
        let saved = runs.store.get_orbit_graph("g-1").unwrap().unwrap();
        assert!(!saved.value.nodes.iter().any(|n| n.id == made));
    }

    /// `orbit_browser_snapshot` (e as outras do navegador): a régua do
    /// `orbit_browser_open` — só o navegador ligado por `controla` agora,
    /// sem `node` o ligado mais recente; tirar a ligação revoga na hora.
    #[cfg(unix)]
    #[test]
    fn agent_reads_only_a_browser_linked_by_control_now() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut o = orchestrator("o");
        o.title = "Claude".into();
        let mut g = graph(
            vec![
                o,
                browser("b", "http://localhost:1"),
                browser("solto", "http://localhost:2"),
            ],
            vec![edge("c1", "o", "b", EdgeKind::Control)],
        );
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g.clone(), vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        let tools = tools_of(&host, "Claude");

        let got = tools.call("orbit_browser_snapshot", &json!({})).unwrap();
        assert_eq!(got["node_id"], "b");
        let got = tools
            .call("orbit_browser_snapshot", &json!({"node": "b"}))
            .unwrap();
        assert_eq!(got["node_id"], "b");
        let err = tools
            .call("orbit_browser_snapshot", &json!({"node": "solto"}))
            .unwrap_err();
        assert!(err.contains("controla"), "{err}");
        tools
            .call(
                "orbit_browser_type",
                &json!({"ref": "e2", "text": "eu@x.test", "submit": true}),
            )
            .unwrap();
        let err = tools
            .call(
                "orbit_browser_click",
                &json!({"node": "solto", "ref": "e1"}),
            )
            .unwrap_err();
        assert!(err.contains("controla"), "{err}");
        assert_eq!(
            host.browser.lock().clone(),
            vec![
                ("b".to_string(), "Claude".to_string(), AgentAction::Snapshot),
                ("b".to_string(), "Claude".to_string(), AgentAction::Snapshot),
                (
                    "b".to_string(),
                    "Claude".to_string(),
                    AgentAction::Type {
                        reference: "e2".into(),
                        text: "eu@x.test".into(),
                        submit: true
                    }
                ),
            ]
        );

        // O dono tira a ligação no canvas: recusa na hora, sem ▶ de novo.
        g.edges.clear();
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let err = tools
            .call("orbit_browser_snapshot", &json!({"node": "b"}))
            .unwrap_err();
        assert!(err.contains("controla"), "{err}");
        let err = tools
            .call("orbit_browser_snapshot", &json!({}))
            .unwrap_err();
        assert!(err.contains("nenhum navegador ligado"), "{err}");
        let err = tools
            .call("orbit_browser_click", &json!({"ref": "e1"}))
            .unwrap_err();
        assert!(err.contains("nenhum navegador ligado"), "{err}");
        assert_eq!(host.browser.lock().len(), 3);

        // O print vai para a `out/` do navegador na pasta do run, 0700.
        g.edges = vec![edge("c1", "o", "b", EdgeKind::Control)];
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        tools.call("orbit_browser_screenshot", &json!({})).unwrap();
        let (node, _, action) = host.browser.lock().last().cloned().unwrap();
        assert_eq!(node, "b");
        let AgentAction::Screenshot { dir: out } = action else {
            panic!("{action:?}")
        };
        assert_eq!(
            out,
            dir.path().join("runs").join(&run.id).join("b").join("out")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&out).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[cfg(unix)]
    #[test]
    fn orchestrator_plan_is_one_note_in_the_graph_rewritten_on_each_call() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut o = orchestrator("o");
        o.position = Position { x: 100.0, y: 100.0 };
        let g = graph(vec![o, process("p", "sleep 30")], vec![]);
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        let tools = tools_of(&host, "o");

        let first = tools
            .call(
                "orbit_plan",
                &json!({"steps": [
                    {"text": "Subir a api", "state": "doing"},
                    {"text": "Abrir o login", "state": "todo"}
                ]}),
            )
            .unwrap();
        let note_id = first["note_id"].as_str().unwrap().to_string();
        let saved = runs.store.get_orbit_graph("g-1").unwrap().unwrap();
        assert!(saved.consented);
        let note = saved.value.nodes.iter().find(|n| n.id == note_id).unwrap();
        assert_eq!(note.title, "Plano");
        assert_eq!(
            note.kind,
            NodeKind::Note {
                markdown: "- [~] Subir a api\n- [ ] Abrir o login\n".into(),
                locked: false,
            }
        );
        // Logo abaixo do orquestrador.
        assert_eq!(note.position.x, 100.0);
        assert!(note.position.y >= 100.0 + 200.0, "{:?}", note.position);
        let plan_edges: Vec<&Edge> = saved
            .value
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Plan)
            .collect();
        assert_eq!(plan_edges.len(), 1);
        assert_eq!(
            (plan_edges[0].from.as_str(), plan_edges[0].to.as_str()),
            ("o", note_id.as_str())
        );
        // Não é nó criado do run (sem tracejado nem Manter).
        let r = runs.get(&run.id).unwrap();
        assert!(r.created.iter().all(|c| c.node.id != note_id));
        assert_eq!(r.plans.len(), 1);
        assert_eq!(r.plans[0].note.id, note_id);
        // O evento do run leva a nota ao front.
        assert!(host
            .runs
            .lock()
            .iter()
            .any(|r| r.plans.iter().any(|p| p.note.id == note_id)));

        let second = tools
            .call(
                "orbit_plan",
                &json!({"steps": [
                    {"text": "Subir a api", "state": "done", "made": "api (:3000)"},
                    {"text": "Abrir o login", "state": "failed"}
                ]}),
            )
            .unwrap();
        assert_eq!(second["note_id"], note_id.as_str());
        let saved = runs.store.get_orbit_graph("g-1").unwrap().unwrap().value;
        assert_eq!(saved.nodes.len(), 3);
        assert_eq!(saved.edges.len(), 1);
        let note = saved.nodes.iter().find(|n| n.id == note_id).unwrap();
        assert_eq!(
            note.kind,
            NodeKind::Note {
                markdown: "- [x] Subir a api\n  criou api (:3000)\n- [!] Abrir o login\n".into(),
                locked: false,
            }
        );
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.plans.len(), 1);
        assert_eq!(r.plans[0].steps[1].state, orchestrator::StepState::Failed);

        // Um salvar da tela em voo tirou a nota (e a ligação) do grafo: ela
        // volta com o mesmo id, sem nascer outra.
        let mut lost = runs.store.get_orbit_graph("g-1").unwrap().unwrap().value;
        lost.nodes.retain(|n| n.id != note_id);
        lost.edges.clear();
        runs.store.save_orbit_graph(&lost, true, None).unwrap();
        let third = tools
            .call(
                "orbit_plan",
                &json!({"steps": [{"text": "Abrir o login", "state": "done"}]}),
            )
            .unwrap();
        assert_eq!(third["note_id"], note_id.as_str());
        let saved = runs.store.get_orbit_graph("g-1").unwrap().unwrap().value;
        assert_eq!(saved.nodes.len(), 3);
        assert_eq!(saved.edges.len(), 1);
        assert_eq!(saved.edges[0].kind, EdgeKind::Plan);
        assert_eq!(
            saved.nodes.iter().find(|n| n.id == note_id).unwrap().kind,
            NodeKind::Note {
                markdown: "- [x] Abrir o login\n".into(),
                locked: false,
            }
        );

        // Plano inválido não mexe no grafo.
        assert!(tools
            .call(
                "orbit_plan",
                &json!({"steps": [{"text": "x", "state": "?"}]})
            )
            .is_err());
        runs.stop(&run.id).unwrap();
        // O histórico guarda a cópia; o grafo segue com a nota.
        let stored = runs.store.get_orbit_run(&run.id).unwrap().unwrap();
        assert_eq!(stored.plans.len(), 1);
        let err = tools
            .call(
                "orbit_plan",
                &json!({"steps": [{"text": "x", "state": "todo"}]}),
            )
            .unwrap_err();
        assert!(err.contains("já terminou"), "{err}");
    }

    /// O ▶ zera na hora a nota de plano de cada agente (no grafo salvo e no
    /// run, com plano pendente sem passos); a do dono fica. O run anterior
    /// guarda o plano dele, e o `orbit_plan` do novo reescreve a mesma nota.
    #[cfg(unix)]
    #[test]
    fn run_starts_with_each_agents_plan_cleared_and_keeps_the_old_one() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(
            vec![orchestrator("o"), orchestrator("o2"), note("n")],
            vec![edge("p2", "o2", "n", EdgeKind::Plan)],
        );
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let first = runs.start_now(g.clone(), vec![]).unwrap();
        // A nota do dono ("# plano") não vira plano pendente.
        assert!(first.plans.is_empty(), "{:?}", first.plans);
        wait_orchestrating(&runs, &first.id, "o");
        let step = json!({"steps": [{"text": "Subir a api", "state": "done"}]});
        let note_id = tools_of(&host, "o").call("orbit_plan", &step).unwrap()["note_id"]
            .as_str()
            .unwrap()
            .to_string();
        runs.stop(&first.id).unwrap();

        let saved = runs.store.get_orbit_graph("g-1").unwrap().unwrap().value;
        let second = runs.start_now(saved, vec![]).unwrap();
        assert_eq!(second.number, first.number + 1);
        // O retrato do ▶ já leva o plano pendente, vazio.
        assert_eq!(second.plans.len(), 1, "{:?}", second.plans);
        assert_eq!(second.plans[0].orchestrator, "o");
        assert_eq!(second.plans[0].note.id, note_id);
        assert!(second.plans[0].steps.is_empty());
        let saved = runs.store.get_orbit_graph("g-1").unwrap().unwrap().value;
        let kind = |id: &str| {
            saved
                .nodes
                .iter()
                .find(|n| n.id == id)
                .unwrap()
                .kind
                .clone()
        };
        assert_eq!(
            kind(&note_id),
            NodeKind::Note {
                markdown: String::new(),
                locked: false,
            }
        );
        assert_eq!(
            kind("n"),
            NodeKind::Note {
                markdown: "# plano".into(),
                locked: false,
            }
        );
        // A execução anterior segue com o plano dela.
        let old = runs.store.get_orbit_run(&first.id).unwrap().unwrap();
        assert_eq!(old.plans.len(), 1);
        assert_eq!(old.plans[0].steps[0].text, "Subir a api");

        wait_orchestrating(&runs, &second.id, "o");
        let again = json!({"steps": [{"text": "Abrir o login", "state": "doing"}]});
        // As ferramentas da sessão nova (a última que subiu com o título).
        let tools = {
            let launches = host.agents.lock().clone();
            let launch = launches.iter().rev().find(|l| l.title == "o").unwrap();
            launch.tools.clone().unwrap()
        };
        let id = tools.call("orbit_plan", &again).unwrap()["note_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(id, note_id);
        let r = runs.get(&second.id).unwrap();
        assert_eq!(r.plans.len(), 1);
        assert_eq!(r.plans[0].steps[0].text, "Abrir o login");
        runs.stop(&second.id).unwrap();
    }

    /// Ligar o orquestrador a uma nota do dono não faz dela o plano: com texto
    /// próprio nasce uma nota "Plano" nova; vazia (ou checklist) é reaproveitada.
    #[cfg(unix)]
    #[test]
    fn plan_never_overwrites_an_owners_note_linked_by_plan() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let empty = node(
            "e",
            NodeKind::Note {
                markdown: String::new(),
                locked: false,
            },
        );
        let g = graph(
            vec![orchestrator("o"), orchestrator("o2"), note("n"), empty],
            vec![
                edge("p1", "o", "n", EdgeKind::Plan),
                edge("p2", "o2", "e", EdgeKind::Plan),
            ],
        );
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "o");
        wait_orchestrating(&runs, &run.id, "o2");
        let step = json!({"steps": [{"text": "Subir a api", "state": "todo"}]});

        let id = tools_of(&host, "o").call("orbit_plan", &step).unwrap()["note_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(id, "n");
        let saved = runs.store.get_orbit_graph("g-1").unwrap().unwrap().value;
        // O texto do dono fica.
        assert_eq!(
            saved.nodes.iter().find(|n| n.id == "n").unwrap().kind,
            NodeKind::Note {
                markdown: "# plano".into(),
                locked: false,
            }
        );
        let fresh = saved.nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!(fresh.title, "Plano");
        assert!(saved
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Plan && e.from == "o" && e.to == id));

        // A vazia vira o plano do o2.
        let id2 = tools_of(&host, "o2").call("orbit_plan", &step).unwrap()["note_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(id2, "e");
        let saved = runs.store.get_orbit_graph("g-1").unwrap().unwrap().value;
        assert_eq!(
            saved.nodes.iter().find(|n| n.id == "e").unwrap().kind,
            NodeKind::Note {
                markdown: "- [ ] Subir a api\n".into(),
                locked: false,
            }
        );
        assert_eq!(saved.nodes.len(), 5);
        runs.stop(&run.id).unwrap();
    }

    /// Conversa: só entre agentes ligados por `talk` (nos dois sentidos); a
    /// mensagem espera na fila do core e sai, marcada, quando o destino fecha
    /// o turno.
    #[cfg(unix)]
    #[test]
    fn agents_talk_only_through_a_talk_link_and_messages_wait_for_idle() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut a = orchestrator("a");
        a.title = "Front".into();
        let mut b = orchestrator("b");
        b.title = "Revisor".into();
        let mut c = orchestrator("c");
        c.title = "Solto".into();
        let g = graph(vec![a, b, c], vec![edge("t1", "a", "b", EdgeKind::Talk)]);
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g, vec![]).unwrap();
        for id in ["a", "b", "c"] {
            wait_orchestrating(&runs, &run.id, id);
        }
        let (front, revisor, solto) = (
            tools_of(&host, "Front"),
            tools_of(&host, "Revisor"),
            tools_of(&host, "Solto"),
        );

        // Peers nos dois sentidos; quem não tem ligação não tem par.
        let peers = front.call("orbit_peers", &json!({})).unwrap();
        assert_eq!(peers["peers"][0]["id"], "b");
        assert_eq!(peers["peers"][0]["title"], "Revisor");
        assert_eq!(peers["peers"].as_array().unwrap().len(), 1);
        let peers = revisor.call("orbit_peers", &json!({})).unwrap();
        assert_eq!(peers["peers"][0]["id"], "a");
        let peers = solto.call("orbit_peers", &json!({})).unwrap();
        assert!(peers["peers"].as_array().unwrap().is_empty());

        // Sem ligação: recusa e não enfileira.
        let err = solto
            .call("orbit_send", &json!({"to": "a", "text": "oi"}))
            .unwrap_err();
        assert!(err.contains("conversa"), "{err}");
        let err = front
            .call("orbit_send", &json!({"to": "c", "text": "oi"}))
            .unwrap_err();
        assert!(err.contains("conversa"), "{err}");
        let err = front
            .call("orbit_send", &json!({"to": "b", "text": "a\u{1b}[201~b"}))
            .unwrap_err();
        assert!(err.contains("controle"), "{err}");

        // Enfileira com o destino trabalhando: nada vai ao terminal.
        host.set_state("Revisor", PeerState::Working);
        let text = format!("revise o login; a chave é {}", fake_key());
        let got = front
            .call("orbit_send", &json!({"to": "b", "text": text}))
            .unwrap();
        assert_eq!(got["queued"], 1);
        assert!(host.inputs.lock().is_empty());
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.nodes["b"].inbox, 1);
        assert_eq!(
            r.nodes["b"].inbox_from,
            HashMap::from([("a".to_string(), 1)])
        );
        let sent = r.nodes["a"].sent.clone().unwrap();
        assert_eq!(sent.to, "Revisor");
        assert!(sent.text.starts_with("revise o login"));
        assert!(!sent.text.contains(&fake_key()), "{}", sent.text);
        let peers = front.call("orbit_peers", &json!({})).unwrap();
        assert_eq!(peers["peers"][0]["state"], "working");
        assert_eq!(peers["peers"][0]["queued"], 1);

        // Turno fechado: o bloco marcado vai para o terminal do Revisor.
        host.set_state("Revisor", PeerState::Ready);
        let inputs = host.inputs.lock().clone();
        assert_eq!(inputs.len(), 1);
        let b_session = runs.get(&run.id).unwrap().nodes["b"].session_id.clone();
        assert_eq!(Some(inputs[0].0.clone()), b_session);
        assert!(
            inputs[0].1.starts_with("[mensagem de Front pelo Orbit"),
            "{}",
            inputs[0].1
        );
        assert!(inputs[0].1.contains("> revise o login"));
        assert!(!inputs[0].1.contains(&fake_key()));
        assert_eq!(runs.get(&run.id).unwrap().nodes["b"].inbox, 0);
        assert!(runs.get(&run.id).unwrap().nodes["b"].inbox_from.is_empty());

        // A volta pela mesma ligação.
        revisor
            .call("orbit_send", &json!({"to": "a", "text": "feito"}))
            .unwrap();
        host.set_state("Front", PeerState::Ready);
        let inputs = host.inputs.lock().clone();
        assert!(inputs[1].1.contains("mensagem de Revisor") && inputs[1].1.contains("> feito"));

        // Teto da fila do destino.
        host.set_state("Revisor", PeerState::Working);
        for _ in 0..talk::MAX_QUEUE {
            front
                .call("orbit_send", &json!({"to": "b", "text": "mais"}))
                .unwrap();
        }
        let err = front
            .call("orbit_send", &json!({"to": "b", "text": "mais"}))
            .unwrap_err();
        assert!(err.contains("cheia"), "{err}");
        assert_eq!(runs.get(&run.id).unwrap().nodes["b"].inbox, talk::MAX_QUEUE);
        runs.stop(&run.id).unwrap();
    }

    /// Memória: só a nota ligada por `memory`; cadeado recusa a escrita; o
    /// texto é redigido e vai para o grafo salvo, com quem escreveu.
    #[cfg(unix)]
    #[test]
    fn agent_reads_and_writes_only_a_note_linked_by_memory() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut a = orchestrator("a");
        a.title = "Front".into();
        let mut x = orchestrator("x");
        x.title = "Outro".into();
        let mut trancada = note("t");
        trancada.kind = NodeKind::Note {
            markdown: "regras do dono".into(),
            locked: true,
        };
        let g = graph(
            vec![a, x, note("m"), note("solta"), trancada],
            vec![
                edge("m1", "a", "m", EdgeKind::Memory),
                edge("m2", "a", "t", EdgeKind::Memory),
            ],
        );
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "a");
        wait_orchestrating(&runs, &run.id, "x");
        let front = tools_of(&host, "Front");

        let got = front
            .call("orbit_note_read", &json!({"node": "m"}))
            .unwrap();
        assert_eq!(got["markdown"], "# plano");
        assert_eq!(got["locked"], false);
        let got = front
            .call("orbit_note_read", &json!({"node": "t"}))
            .unwrap();
        assert_eq!(got["locked"], true);

        // Sem ligação (a nota solta; o outro agente): recusa.
        for (tools, node) in [(&front, "solta"), (&tools_of(&host, "Outro"), "m")] {
            let err = tools
                .call("orbit_note_read", &json!({"node": node}))
                .unwrap_err();
            assert!(err.contains("memória"), "{err}");
            let err = tools
                .call("orbit_note_write", &json!({"node": node, "text": "x"}))
                .unwrap_err();
            assert!(err.contains("memória"), "{err}");
        }

        // Cadeado: só leitura.
        let err = front
            .call("orbit_note_write", &json!({"node": "t", "text": "x"}))
            .unwrap_err();
        assert!(err.contains("cadeado"), "{err}");

        // Escrita: troca, depois completa; redigida; no grafo salvo.
        front
            .call(
                "orbit_note_write",
                &json!({"node": "m", "text": format!("porta 5173, chave {}", fake_key())}),
            )
            .unwrap();
        front
            .call(
                "orbit_note_write",
                &json!({"node": "m", "text": "login ok", "mode": "append"}),
            )
            .unwrap();
        let saved = runs.store.get_orbit_graph("g-1").unwrap().unwrap().value;
        let m = saved.nodes.iter().find(|n| n.id == "m").unwrap();
        let NodeKind::Note { markdown, locked } = &m.kind else {
            panic!("nota");
        };
        assert!(markdown.starts_with("porta 5173, chave "), "{markdown}");
        // O append marca a linha com quem escreveu e a hora local.
        let last = markdown.rsplit('\n').next().unwrap();
        let hhmm = last
            .strip_prefix("login ok (Front, ")
            .and_then(|r| r.strip_suffix(')'))
            .unwrap_or_else(|| panic!("{markdown}"));
        assert!(hhmm.len() == 5 && hhmm.as_bytes()[2] == b':', "{markdown}");
        assert!(!markdown.contains(&fake_key()), "{markdown}");
        assert!(!locked);
        let t = saved.nodes.iter().find(|n| n.id == "t").unwrap();
        assert_eq!(
            t.kind,
            NodeKind::Note {
                markdown: "regras do dono".into(),
                locked: true
            }
        );
        let written = runs.get(&run.id).unwrap().nodes["m"]
            .written
            .clone()
            .unwrap();
        assert_eq!(written.by, "Front");
        assert_eq!(&written.markdown, markdown);
        // A leitura seguinte vê a escrita.
        let got = front
            .call("orbit_note_read", &json!({"node": "m"}))
            .unwrap();
        assert_eq!(got["markdown"], markdown.as_str());

        // O dono trancou depois do ▶: vale o cadeado do grafo salvo.
        let mut g = saved.clone();
        if let Some(n) = g.nodes.iter_mut().find(|n| n.id == "m") {
            n.kind = NodeKind::Note {
                markdown: markdown.clone(),
                locked: true,
            };
        }
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let err = front
            .call("orbit_note_write", &json!({"node": "m", "text": "x"}))
            .unwrap_err();
        assert!(err.contains("cadeado"), "{err}");

        // E destrancou a que veio trancada no ▶: o agente já escreve nela.
        if let Some(n) = g.nodes.iter_mut().find(|n| n.id == "t") {
            n.kind = NodeKind::Note {
                markdown: "regras do dono".into(),
                locked: false,
            };
        }
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        front
            .call(
                "orbit_note_write",
                &json!({"node": "t", "text": "liberada"}),
            )
            .unwrap();
        runs.stop(&run.id).unwrap();
    }

    /// Memória sobre o texto do grafo salvo: o que o dono escreveu depois do
    /// ▶ é o que o agente lê, e o `append` soma a ele (nada volta ao texto
    /// do ▶).
    #[cfg(unix)]
    #[test]
    fn memory_reads_and_appends_over_the_owners_text_saved_after_start() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut a = orchestrator("a");
        a.title = "Front".into();
        let g = graph(
            vec![a, note("m")],
            vec![edge("m1", "a", "m", EdgeKind::Memory)],
        );
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g.clone(), vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "a");
        let front = tools_of(&host, "Front");

        // O dono escreve na nota depois do ▶.
        let mut edited = g.clone();
        edited.nodes[1].kind = NodeKind::Note {
            markdown: "do dono".into(),
            locked: false,
        };
        runs.store.save_orbit_graph(&edited, true, None).unwrap();
        let got = front
            .call("orbit_note_read", &json!({"node": "m"}))
            .unwrap();
        assert_eq!(got["markdown"], "do dono");
        front
            .call(
                "orbit_note_write",
                &json!({"node": "m", "text": "do agente", "mode": "append"}),
            )
            .unwrap();
        let saved = runs.store.get_orbit_graph("g-1").unwrap().unwrap().value;
        let NodeKind::Note { markdown, locked } = &saved.nodes[1].kind else {
            panic!("nota");
        };
        assert!(markdown.starts_with("do dono\ndo agente ("), "{markdown}");
        assert!(!locked);
        runs.stop(&run.id).unwrap();
    }

    /// As permissões olham as ligações de agora do grafo salvo: tirar a
    /// ligação no canvas durante o run revoga na hora, pôr libera na hora —
    /// conversa, memória e controla.
    #[cfg(unix)]
    #[test]
    fn links_removed_or_added_during_the_run_count_at_once() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut a = orchestrator("a");
        a.title = "Front".into();
        let mut b = orchestrator("b");
        b.title = "Revisor".into();
        let with = graph(
            vec![a, b, note("m"), browser("w", "http://localhost:1")],
            vec![
                edge("t1", "a", "b", EdgeKind::Talk),
                edge("m1", "a", "m", EdgeKind::Memory),
                edge("c1", "a", "w", EdgeKind::Control),
            ],
        );
        let mut without = with.clone();
        without.edges.clear();
        runs.store.save_orbit_graph(&with, true, None).unwrap();
        let run = runs.start_now(with.clone(), vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "a");
        wait_orchestrating(&runs, &run.id, "b");
        let front = tools_of(&host, "Front");
        let calls = [
            ("orbit_send", json!({"to": "b", "text": "oi"}), "conversa"),
            ("orbit_note_read", json!({"node": "m"}), "memória"),
            (
                "orbit_note_write",
                json!({"node": "m", "text": "x"}),
                "memória",
            ),
            (
                "orbit_browser_open",
                json!({"node": "w", "url": "http://localhost:5173"}),
                "controla",
            ),
            (
                "orbit_browser_open",
                json!({"url": "http://localhost:5173"}),
                "nenhum navegador ligado",
            ),
        ];
        host.set_state("Revisor", PeerState::Working);
        for (tool, args, _) in &calls {
            front.call(tool, args).unwrap();
        }
        let peers = front.call("orbit_peers", &json!({})).unwrap();
        assert_eq!(peers["peers"].as_array().unwrap().len(), 1);

        // O dono tira as ligações no canvas: recusa na hora.
        runs.store.save_orbit_graph(&without, true, None).unwrap();
        for (tool, args, why) in &calls {
            let err = front.call(tool, args).unwrap_err();
            assert!(err.contains(why), "{tool}: {err}");
        }
        let peers = front.call("orbit_peers", &json!({})).unwrap();
        assert!(peers["peers"].as_array().unwrap().is_empty());

        // E põe de volta: aceita na hora.
        runs.store.save_orbit_graph(&with, true, None).unwrap();
        for (tool, args, _) in &calls {
            front.call(tool, args).unwrap();
        }
        runs.stop(&run.id).unwrap();
    }

    /// A fila só esvazia com o envio Ok: com erro, as mensagens voltam a
    /// esperar e o retrato leva "não entregue" com o motivo. Sessão sem
    /// bracketed paste recebe o bloco numa linha só. Dono digitando no
    /// terminal do destino: a entrega espera.
    #[cfg(unix)]
    #[test]
    fn delivery_keeps_the_queue_on_error_goes_one_line_and_waits_for_the_owner() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut a = orchestrator("a");
        a.title = "Front".into();
        let mut b = orchestrator("b");
        b.title = "Revisor".into();
        let g = graph(vec![a, b], vec![edge("t1", "a", "b", EdgeKind::Talk)]);
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "a");
        wait_orchestrating(&runs, &run.id, "b");
        let front = tools_of(&host, "Front");
        let session = runs.get(&run.id).unwrap().nodes["b"]
            .session_id
            .clone()
            .unwrap();

        host.set_state("Revisor", PeerState::Working);
        front
            .call("orbit_send", &json!({"to": "b", "text": "um\ndois"}))
            .unwrap();
        *host.send_error.lock() = Some("sessão fechou".into());
        host.set_state("Revisor", PeerState::Ready);
        assert!(host.inputs.lock().is_empty());
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.nodes["b"].inbox, 1);
        let why = r.nodes["b"].undelivered.clone().unwrap();
        assert!(
            why.contains("não entregue") && why.contains("sessão fechou"),
            "{why}"
        );

        // Volta a dar certo no turno seguinte, sem bracketed paste: uma linha.
        *host.send_error.lock() = None;
        *host.multiline.lock() = false;
        host.set_state("Revisor", PeerState::Working);
        host.set_state("Revisor", PeerState::Ready);
        let inputs = host.inputs.lock().clone();
        assert_eq!(inputs.len(), 1);
        assert!(!inputs[0].1.contains('\n'), "{}", inputs[0].1);
        assert!(inputs[0].1.contains("> um ⏎ > dois"), "{}", inputs[0].1);
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.nodes["b"].inbox, 0);
        assert_eq!(r.nodes["b"].undelivered, None);

        // O dono deixou texto por enviar no terminal do Revisor: espera.
        host.set_state("Revisor", PeerState::Working);
        front
            .call("orbit_send", &json!({"to": "b", "text": "três"}))
            .unwrap();
        host.owner.record(&session, b"git sta", now());
        host.set_state("Revisor", PeerState::Ready);
        assert_eq!(host.inputs.lock().len(), 1);
        // Enviou, mas teclou agora há pouco: ainda espera.
        host.owner.record(&session, b"\r", now());
        host.set_state("Revisor", PeerState::Ready);
        assert_eq!(host.inputs.lock().len(), 1);
        // Teclado parado há mais de 2 s: sai no mesmo Idle.
        host.owner
            .record(&session, b"\r", now() - talk::OWNER_QUIET_MS - 1);
        host.set_state("Revisor", PeerState::Ready);
        let inputs = host.inputs.lock().clone();
        assert_eq!(inputs.len(), 2);
        assert!(inputs[1].1.contains("> três"));
        runs.stop(&run.id).unwrap();
    }

    /// `orbit_node_status`: só de nó ligado a quem pergunta (qualquer
    /// ligação, em qualquer sentido) ou que ele criou.
    #[cfg(unix)]
    #[test]
    fn node_status_needs_a_link_or_authorship() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut a = orchestrator("a");
        a.title = "Front".into();
        let g = graph(
            vec![
                a,
                note("m"),
                process("p", "sleep 30"),
                process("solto", "sleep 30"),
            ],
            vec![
                edge("m1", "a", "m", EdgeKind::Memory),
                edge("d1", "a", "p", EdgeKind::Diff),
            ],
        );
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g.clone(), vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "a");
        let front = tools_of(&host, "Front");
        front
            .call("orbit_node_status", &json!({"node_id": "m"}))
            .unwrap();
        front
            .call("orbit_node_status", &json!({"node_id": "p"}))
            .unwrap();
        let err = front
            .call("orbit_node_status", &json!({"node_id": "solto"}))
            .unwrap_err();
        assert!(err.contains("não está ligado"), "{err}");
        // A ligação nova no sentido contrário vale na hora.
        let mut linked = g.clone();
        linked.edges.push(edge("l1", "solto", "a", EdgeKind::Log));
        runs.store.save_orbit_graph(&linked, true, None).unwrap();
        front
            .call("orbit_node_status", &json!({"node_id": "solto"}))
            .unwrap();
        runs.stop(&run.id).unwrap();
    }

    /// "Não criar nós": o agente tem as ferramentas sem as `orbit_create_*`
    /// (o `tools/list` não as anuncia e o core recusa a chamada).
    #[cfg(unix)]
    #[test]
    fn agent_marked_no_create_gets_the_tools_without_create() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("api");
        std::fs::create_dir(&work).unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut a = orchestrator("a");
        a.title = "Quieto".into();
        if let NodeKind::Agent { no_create, .. } = &mut a.kind {
            *no_create = true;
        }
        let g = graph(vec![a, orchestrator("o")], vec![]);
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "a");
        wait_orchestrating(&runs, &run.id, "o");
        assert!(tools_of(&host, "o").can_create);
        let tools = tools_of(&host, "Quieto");
        assert!(!tools.can_create);
        let err = tools
            .call(
                "orbit_create_process",
                &json!({"title": "api", "command": "echo", "dir": work}),
            )
            .unwrap_err();
        assert!(err.contains("Não criar nós"), "{err}");
        // O resto das ferramentas segue.
        tools
            .call(
                "orbit_plan",
                &json!({"steps": [{"text": "um", "state": "doing"}]}),
            )
            .unwrap();
        runs.stop(&run.id).unwrap();
    }

    // ---- Parar / reiniciar / rodar de novo um nó (Fase 3) ----

    /// Rodar de novo a origem que falhou: ela volta a esperar segurada (até
    /// o `unhold`), com a tentativa seguinte, e quem foi pulado por ela volta
    /// a esperar.
    #[test]
    fn rerun_brings_back_the_skipped_and_holds_until_unhold() {
        let mut s = Scheduler::new(&diamond(), "r".into(), 0);
        s.mark_running("a", 1);
        s.mark_running("b", 1);
        s.set_session("a", "sess-a".into());
        s.finish("a", Outcome::failed("exit 1"), 2);
        assert_eq!(status(&s, "c"), NodeStatus::Skipped);
        assert_eq!(status(&s, "d"), NodeStatus::Skipped);

        assert_eq!(s.rerun("a"), Ok(Some("sess-a".to_string())));
        let a = &s.run().nodes["a"];
        assert_eq!(a.status, NodeStatus::Waiting);
        assert_eq!(a.attempt, 1);
        assert!(a.reason.is_none() && a.exit_code.is_none() && a.session_id.is_none());
        assert_eq!(status(&s, "c"), NodeStatus::Waiting);
        assert_eq!(status(&s, "d"), NodeStatus::Waiting);
        assert!(!s.run().nodes["c"].stale, "c nunca começou");
        assert!(s.ready().is_empty(), "segurado até a out/ velha sumir");
        s.unhold("a");
        assert_eq!(s.ready(), vec!["a".to_string()]);
    }

    /// Quem já começou com o valor antigo fica "↻ para aplicar" e não
    /// reinicia sozinho; rodá-lo de novo tira a marca.
    #[test]
    fn rerun_marks_the_consumers_that_already_started() {
        let mut s = Scheduler::new(&diamond(), "r".into(), 0);
        for id in ["a", "b"] {
            s.mark_running(id, 1);
            s.finish(id, ok(), 2);
        }
        s.mark_running("c", 3);
        assert_eq!(s.rerun("a"), Ok(None));
        let c = &s.run().nodes["c"];
        assert!(c.stale);
        assert_eq!(c.stale_reason, Some(StaleReason::Source));
        assert_eq!(c.status, NodeStatus::Running, "não reinicia sozinho");
        assert!(!s.run().nodes["b"].stale, "b não depende de a");
        assert!(!s.run().nodes["d"].stale, "d ainda não começou");
        s.unhold("a");
        s.mark_running("a", 4);
        s.finish("a", ok(), 5);
        assert!(s.run().nodes["c"].stale, "a marca fica até c rodar de novo");
        assert_eq!(s.rerun("c"), Ok(None));
        assert!(!s.run().nodes["c"].stale);
        assert_eq!(s.run().nodes["c"].stale_reason, None);
        assert_eq!(s.run().nodes["c"].attempt, 1);
    }

    #[test]
    fn rerun_refuses_waiting_skipped_and_closed() {
        let mut s = Scheduler::new(&diamond(), "r".into(), 0);
        s.mark_running("a", 1);
        s.mark_running("b", 1);
        assert!(s.rerun("c").unwrap_err().contains("ainda não começou"));
        assert!(s.rerun("x").unwrap_err().contains("não existe"));
        // O dono parou c antes de começar: rodar a de novo não o traz.
        s.stop_node("c", 2);
        assert_eq!(
            s.run().nodes["c"].reason.as_deref(),
            Some(STOPPED_BEFORE_START)
        );
        s.finish("a", Outcome::failed("exit 1"), 3);
        assert!(s.rerun("d").unwrap_err().contains("pulado"));
        s.rerun("a").unwrap();
        assert_eq!(status(&s, "c"), NodeStatus::Skipped);
        assert_eq!(status(&s, "d"), NodeStatus::Skipped);
        s.stop(4);
        assert!(s.rerun("b").unwrap_err().contains("já fechou"));
    }

    fn wait_attempt(
        runs: &Runs<Arc<PtyHost>>,
        run: &str,
        node: &str,
        attempt: u32,
        want: NodeStatus,
    ) -> Run {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let r = runs.get(run).unwrap();
            let n = &r.nodes[node];
            if (n.attempt == attempt && n.status == want) || Instant::now() > deadline {
                return r;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Reiniciar um processo rodando: o process group velho morre, sobe
    /// outro com a mesma config, e o fim do PTY velho não fecha o novo.
    #[cfg(unix)]
    #[test]
    fn restart_kills_the_running_process_and_starts_it_again() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let run = runs
            .start_now(
                graph(vec![process("p", "echo oi; sleep 30")], vec![]),
                vec![],
            )
            .unwrap();
        let old = run.nodes["p"].session_id.clone().unwrap();
        runs.restart_node(&run.id, "p").unwrap();
        assert!(!host.pool.is_alive(old.parse().unwrap()));
        let r = wait_attempt(&runs, &run.id, "p", 1, NodeStatus::Running);
        let new = r.nodes["p"].session_id.clone().expect("sessão nova");
        assert_ne!(new, old);
        std::thread::sleep(Duration::from_millis(400));
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.nodes["p"].status, NodeStatus::Running, "{r:?}");
        assert_eq!(r.status, RunStatus::Running);
        assert!(host.pool.is_alive(new.parse().unwrap()));
        runs.stop(&run.id).unwrap();
    }

    /// Rodar de novo o processo que falhou, com a execução aberta: ele roda
    /// de novo na `out/` limpa (o `.exit` velho não vale) e o dependente que
    /// tinha sido pulado roda com o valor novo.
    #[cfg(unix)]
    #[test]
    fn rerun_failed_process_runs_the_skipped_dependent() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let mark = dir.path().join("mark");
        let command = format!(
            "if [ -f '{m}' ]; then echo segunda; else touch '{m}'; echo primeira; exit 3; fi",
            m = mark.display()
        );
        let g = graph(
            vec![
                process("p", &command),
                process("q", "cat \"$TYBA_IN/log.txt\""),
                process("keep", "sleep 30"),
            ],
            vec![edge("e1", "p", "q", EdgeKind::Log)],
        );
        let run = runs.start_now(g, vec![]).unwrap();
        let r = wait_status(&runs, &run.id, "p", NodeStatus::Failed);
        assert_eq!(r.nodes["q"].status, NodeStatus::Skipped);
        assert_eq!(r.nodes["p"].exit_code, Some(3));

        runs.restart_node(&run.id, "p").unwrap();
        let r = wait_status(&runs, &run.id, "q", NodeStatus::Done);
        let p = &r.nodes["p"];
        assert_eq!(
            (p.status, p.attempt, p.exit_code),
            (NodeStatus::Done, 1, Some(0)),
            "{r:?}"
        );
        assert!(p.reason.is_none());
        let log = std::fs::read_to_string(&r.nodes["q"].outputs[&EdgeKind::Log]).unwrap();
        assert!(
            log.contains("segunda") && !log.contains("primeira"),
            "{log:?}"
        );
        assert!(!r.nodes["q"].stale);
        runs.stop(&run.id).unwrap();
    }

    /// O dependente que já consumiu fica "↻ para aplicar", sem reiniciar
    /// sozinho; reiniciá-lo tira a marca e ele lê o valor novo.
    #[cfg(unix)]
    #[test]
    fn consumer_of_a_rerun_node_is_marked_and_restarts_only_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let count = dir.path().join("count");
        let g = graph(
            vec![
                process(
                    "p",
                    &format!("echo x >> '{c}'; wc -l < '{c}'", c = count.display()),
                ),
                process("q", "cat \"$TYBA_IN/log.txt\"; sleep 30"),
            ],
            vec![edge("e1", "p", "q", EdgeKind::Log)],
        );
        let run = runs.start_now(g, vec![]).unwrap();
        wait_status(&runs, &run.id, "q", NodeStatus::Running);
        // `running` vem antes da sessão gravada (ela sai do spawn).
        let deadline = Instant::now() + Duration::from_secs(20);
        let q_session = loop {
            let r = runs.get(&run.id).unwrap();
            match r.nodes["q"].session_id.clone() {
                Some(s) => break s,
                None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
                None => panic!("q sem sessão: {r:?}"),
            }
        };
        runs.restart_node(&run.id, "p").unwrap();
        let r = wait_attempt(&runs, &run.id, "p", 1, NodeStatus::Done);
        let q = &r.nodes["q"];
        assert!(q.stale, "{r:?}");
        assert_eq!(q.status, NodeStatus::Running);
        assert_eq!(
            q.session_id.as_deref(),
            Some(q_session.as_str()),
            "não reiniciou sozinho"
        );
        assert!(host.pool.is_alive(q_session.parse().unwrap()));

        runs.restart_node(&run.id, "q").unwrap();
        let r = wait_attempt(&runs, &run.id, "q", 1, NodeStatus::Running);
        assert!(!r.nodes["q"].stale);
        let input = dir
            .path()
            .join("runs")
            .join(&run.id)
            .join("q")
            .join("in")
            .join("log.txt");
        assert_eq!(std::fs::read_to_string(input).unwrap().trim(), "2");
        runs.stop(&run.id).unwrap();
    }

    /// Ligação nova com o run aberto: quem espera passa a esperar a origem
    /// nova; quem já começou e teve as entradas mudadas fica marcado.
    #[test]
    fn rewire_applies_new_edges_now() {
        let mut s = Scheduler::new(&diamond(), "r".into(), 0);
        s.mark_running("a", 1);
        s.mark_running("b", 1);
        s.finish("a", ok(), 2);
        s.finish("b", ok(), 2);
        s.mark_running("c", 3);
        // c perde a ligação de b; d ganha uma de a (d ainda espera c).
        let mut g = diamond();
        g.edges.retain(|e| e.id != "e2");
        g.edges.push(edge("e4", "a", "d", EdgeKind::Log));
        assert!(s.rewire(&g.edges));
        assert!(s.run().nodes["c"].stale, "c já tinha começado");
        assert_eq!(s.run().nodes["c"].stale_reason, Some(StaleReason::Edges));
        assert!(!s.run().nodes["d"].stale, "d ainda não começou");
        assert!(s.ready().is_empty(), "d segue esperando c");
        s.finish("c", ok(), 4);
        assert_eq!(s.ready(), vec!["d".to_string()]);
        // As mesmas ligações de novo não mudam nada.
        assert!(!s.rewire(&g.edges));
    }

    /// Ligação para uma origem morta pula quem espera; tirar a ligação da
    /// origem morta traz de volta o pulado em cascata. Ponta fora do run
    /// não conta.
    #[test]
    fn rewire_skips_and_unskips_by_the_dead_source() {
        let mut s = Scheduler::new(&diamond(), "r".into(), 0);
        s.mark_running("a", 1);
        s.finish("a", Outcome::failed("exit 1"), 2);
        assert_eq!(status(&s, "c"), NodeStatus::Skipped);
        assert_eq!(status(&s, "d"), NodeStatus::Skipped);
        let mut g = diamond();
        g.edges.retain(|e| e.id != "e1");
        g.edges.push(edge("e9", "novo", "b", EdgeKind::Log));
        assert!(s.rewire(&g.edges));
        assert_eq!(status(&s, "c"), NodeStatus::Waiting);
        assert_eq!(status(&s, "d"), NodeStatus::Waiting);
        assert_eq!(
            s.ready(),
            vec!["b".to_string()],
            "a ligação de fora não segura b"
        );
        g.edges.push(edge("e5", "a", "b", EdgeKind::Log));
        assert!(s.rewire(&g.edges));
        assert_eq!(status(&s, "b"), NodeStatus::Skipped);
        assert_eq!(status(&s, "c"), NodeStatus::Skipped);
    }

    #[test]
    fn mark_changed_only_marks_who_already_started() {
        let mut s = Scheduler::new(&diamond(), "r".into(), 0);
        s.mark_running("a", 1);
        assert!(s.mark_changed("a", StaleReason::Config));
        assert!(!s.mark_changed("a", StaleReason::Config), "já marcado");
        assert!(s.mark_changed("a", StaleReason::Variable), "motivo novo");
        assert_eq!(s.run().nodes["a"].stale_reason, Some(StaleReason::Variable));
        assert!(
            !s.mark_changed("c", StaleReason::Config),
            "espera: começa com a config nova"
        );
        assert!(!s.run().nodes["c"].stale);
    }

    /// Editar o grafo com o run aberto: o comando novo não reinicia o nó
    /// sozinho (fica "↻ para aplicar") e vale no reinício; o nó apagado
    /// que rodava morre (process group inteiro); a ligação nova vale na hora.
    #[cfg(unix)]
    #[test]
    fn saving_the_graph_reconciles_the_open_run() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(
            vec![
                process("p", "echo velho; sleep 30"),
                process("gone", "sleep 30"),
                process("q", "cat \"$TYBA_IN/log.txt\""),
                process("w", "cat \"$TYBA_IN/log.txt\"; sleep 30"),
            ],
            vec![
                edge("e1", "gone", "q", EdgeKind::Log),
                edge("e2", "p", "w", EdgeKind::Log),
            ],
        );
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g.clone(), vec![]).unwrap();
        let r = wait_status(&runs, &run.id, "gone", NodeStatus::Running);
        let gone_session = r.nodes["gone"].session_id.clone().unwrap();
        let p_session = r.nodes["p"].session_id.clone().unwrap();
        assert_eq!(r.nodes["q"].status, NodeStatus::Waiting);

        // O dono apaga "gone", liga p → q no lugar e troca o comando de p.
        let mut edited = g.clone();
        edited.nodes.retain(|n| n.id != "gone");
        edited.edges.retain(|e| e.id != "e1");
        edited.edges.push(edge("e3", "p", "q", EdgeKind::Log));
        if let NodeKind::Process { command, .. } = &mut edited.nodes[0].kind {
            *command = "echo novo; sleep 30".into();
        }
        runs.store.save_orbit_graph(&edited, true, None).unwrap();
        runs.graph_saved(&edited);

        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.nodes["gone"].status, NodeStatus::Stopped, "{r:?}");
        assert!(!host.pool.is_alive(gone_session.parse().unwrap()));
        let p = &r.nodes["p"];
        assert!(p.stale, "config nova espera o reinício");
        assert_eq!(p.stale_reason, Some(StaleReason::Config));
        assert_eq!(p.session_id.as_deref(), Some(p_session.as_str()));
        assert!(host.pool.is_alive(p_session.parse().unwrap()));
        assert_eq!(r.nodes["q"].status, NodeStatus::Waiting, "agora espera p");
        assert!(!r.nodes["w"].stale, "a ligação de w não mudou");

        runs.restart_node(&run.id, "p").unwrap();
        let r = wait_attempt(&runs, &run.id, "p", 1, NodeStatus::Running);
        assert!(!r.nodes["p"].stale);
        let deadline = Instant::now() + Duration::from_secs(20);
        let log = dir
            .path()
            .join("runs")
            .join(&run.id)
            .join("p")
            .join("out")
            .join("log.txt");
        loop {
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            if text.contains("novo") {
                assert!(!text.contains("velho"), "{text:?}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "o reinício não usou o comando novo: {text:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        runs.stop(&run.id).unwrap();
    }

    /// Execução fechada não reinicia nó: o ▶ Executar é o caminho.
    #[cfg(unix)]
    #[test]
    fn restart_needs_an_open_run() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let run = runs
            .start_now(graph(vec![process("p", "exit 1")], vec![]), vec![])
            .unwrap();
        wait_finished(&runs, &run.id);
        let err = runs.restart_node(&run.id, "p").unwrap_err();
        assert!(err.contains("já fechou"), "{err}");
    }

    /// Reiniciar o agente do grafo: a sessão velha morre (e a saída dela
    /// não fecha o nó), sobe outra com o mesmo prompt e ela segura o run.
    #[cfg(unix)]
    #[test]
    fn restart_agent_kills_the_old_session_and_starts_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let run = runs
            .start_now(graph(vec![agent("a", "faça x")], vec![]), vec![])
            .unwrap();
        wait_orchestrating(&runs, &run.id, "a");
        let old = host.spawned.lock()[0].clone();
        runs.restart_node(&run.id, "a").unwrap();
        assert!(host.killed.lock().contains(&old));
        let deadline = Instant::now() + Duration::from_secs(20);
        let r = loop {
            let r = runs.get(&run.id).unwrap();
            if (r.nodes["a"].attempt == 1 && r.nodes["a"].orchestrating)
                || Instant::now() > deadline
            {
                break r;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(r.nodes["a"].status, NodeStatus::Running, "{r:?}");
        assert_eq!(r.status, RunStatus::Running);
        let new = host.spawned.lock()[1].clone();
        assert_eq!(r.live_agents, vec![new.clone()]);
        let prompts: Vec<String> = host
            .agents
            .lock()
            .iter()
            .map(|l| l.prompt.clone())
            .collect();
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0], prompts[1], "mesma config");
        runs.stop(&run.id).unwrap();
        assert!(host.killed.lock().contains(&new));
    }

    /// Rodar de novo um nó de uma execução assentada a volta a "ao vivo".
    #[cfg(unix)]
    #[test]
    fn rerun_in_a_settled_run_wakes_it() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        runs.settle_ms
            .store(150, std::sync::atomic::Ordering::Relaxed);
        let mark = dir.path().join("mark");
        let mut a = orchestrator("a");
        a.title = "Front".into();
        let command = format!(
            "if [ -f '{m}' ]; then sleep 30; else touch '{m}'; fi",
            m = mark.display()
        );
        let g = graph(vec![a, process("p", &command)], vec![]);
        let run = runs.start_now(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "a");
        wait_status(&runs, &run.id, "p", NodeStatus::Done);
        host.set_state("Front", PeerState::Ready);
        std::thread::sleep(Duration::from_millis(200));
        host.set_state("Front", PeerState::Ready);
        assert!(wait_settled(&runs, &run.id, true).settled);

        runs.restart_node(&run.id, "p").unwrap();
        let r = wait_attempt(&runs, &run.id, "p", 1, NodeStatus::Running);
        assert!(!r.settled, "{r:?}");
        assert_eq!(r.status, RunStatus::Running);
        runs.stop(&run.id).unwrap();
    }

    /// `orbit_restart_node`: só nó ligado a quem chama ou criado por ele,
    /// nunca o próprio; o processo responde com o estado novo.
    #[cfg(unix)]
    #[test]
    fn agent_restarts_only_a_linked_or_created_node() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut a = orchestrator("a");
        a.title = "Front".into();
        let g = graph(
            vec![a, process("p", "sleep 30"), process("solto", "sleep 30")],
            vec![edge("d1", "a", "p", EdgeKind::Diff)],
        );
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g, vec![]).unwrap();
        wait_orchestrating(&runs, &run.id, "a");
        wait_status(&runs, &run.id, "p", NodeStatus::Running);
        let front = tools_of(&host, "Front");
        let err = front
            .call("orbit_restart_node", &json!({"node": "solto"}))
            .unwrap_err();
        assert!(err.contains("não está ligado"), "{err}");
        let err = front
            .call("orbit_restart_node", &json!({"node": "a"}))
            .unwrap_err();
        assert!(err.contains("orbit_finish"), "{err}");
        assert!(front.call("orbit_restart_node", &json!({})).is_err());
        let answer = front
            .call("orbit_restart_node", &json!({"node": "p"}))
            .unwrap();
        assert_eq!(answer["node_id"], "p");
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.nodes["p"].attempt, 1);
        assert_eq!(r.nodes["solto"].attempt, 0);
        runs.stop(&run.id).unwrap();
    }

    fn wait_until(
        runs: &Runs<Arc<PtyHost>>,
        run: &str,
        what: &str,
        ok: impl Fn(&Run) -> bool,
    ) -> Run {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let r = runs.get(run).unwrap();
            if ok(&r) {
                return r;
            }
            assert!(Instant::now() < deadline, "{what}: {r:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn set_command(g: &mut Graph, id: &str, text: &str) {
        let n = g.nodes.iter_mut().find(|n| n.id == id).unwrap();
        if let NodeKind::Process { command, .. } = &mut n.kind {
            *command = text.into();
        }
    }

    /// Variável salva com o run aberto chega ao reinício: quem a usa e já
    /// começou fica "↻ para aplicar" (motivo: variável), e o reinício roda
    /// o valor novo. Variável sem valor nunca vira texto vazio no comando
    /// (`rm -rf {{out}}/cache` → `rm -rf /cache`): o reinício recusa com o
    /// título, e o nó que começa depois falha sem rodar.
    #[cfg(unix)]
    #[test]
    fn saved_variables_reach_the_restart_and_an_empty_one_never_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mark = dir.path().join("mark");
        let mut g = graph(
            vec![
                process("p", "echo rm -rf {{out}}/cache; sleep 30"),
                process("w", "sleep 30"),
                process(
                    "gate",
                    &format!(
                        "while [ ! -f '{m}' ]; do sleep 0.05; done",
                        m = mark.display()
                    ),
                ),
                process("q", "true"),
            ],
            vec![edge("e1", "gate", "q", EdgeKind::Log)],
        );
        g.variables.insert("out".into(), "/tmp/velho".into());
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g.clone(), vec![]).unwrap();
        let r = wait_until(&runs, &run.id, "p e w sobem", |r| {
            r.nodes["p"].session_id.is_some() && r.nodes["w"].session_id.is_some()
        });
        let w_session = r.nodes["w"].session_id.clone().unwrap();

        let mut edited = g.clone();
        edited.variables.insert("out".into(), "/tmp/novo".into());
        set_command(&mut edited, "w", "echo rm -rf {{vazia}}/cache");
        set_command(&mut edited, "q", "echo rm -rf {{vazia}}/cache");
        runs.store.save_orbit_graph(&edited, true, None).unwrap();
        runs.graph_saved(&edited);

        let r = runs.get(&run.id).unwrap();
        assert_eq!(
            r.nodes["p"].stale_reason,
            Some(StaleReason::Variable),
            "{r:?}"
        );
        assert_eq!(r.nodes["w"].stale_reason, Some(StaleReason::Config));
        assert!(!r.nodes["gate"].stale, "gate não usa variável");

        let err = runs.restart_node(&run.id, "w").unwrap_err();
        assert!(err.contains("\"w\"") && err.contains("{{vazia}}"), "{err}");
        let r = runs.get(&run.id).unwrap();
        assert_eq!(r.nodes["w"].attempt, 0, "recusa antes de matar");
        assert!(host.pool.is_alive(w_session.parse().unwrap()));

        runs.restart_node(&run.id, "p").unwrap();
        let log = dir
            .path()
            .join("runs")
            .join(&run.id)
            .join("p")
            .join("out")
            .join("log.txt");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            if text.contains("rm -rf /tmp/novo/cache") {
                break;
            }
            assert!(Instant::now() < deadline, "sem o valor novo: {text:?}");
            std::thread::sleep(Duration::from_millis(20));
        }

        std::fs::write(&mark, "").unwrap();
        let r = wait_status(&runs, &run.id, "q", NodeStatus::Failed);
        let q = &r.nodes["q"];
        assert!(
            q.reason
                .as_deref()
                .is_some_and(|m| m.contains("\"q\"") && m.contains("{{vazia}}")),
            "{q:?}"
        );
        assert!(q.session_id.is_none(), "não subiu o comando com o buraco");
        runs.stop(&run.id).unwrap();
    }

    /// `orbit_restart_node` não dispara integração manual: esperando o ▶ ou
    /// já rodando, o agente recebe a recusa; o dono reinicia normalmente.
    #[cfg(unix)]
    #[test]
    fn agent_never_runs_a_manual_integration() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let (runs, host, _app) = pty_runs(dir.path().join("runs"));
        let mut a = orchestrator("a");
        a.title = "Front".into();
        let g = graph(
            vec![a, integration_node("m", "manual")],
            vec![edge("d1", "a", "m", EdgeKind::Diff)],
        );
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs
            .start_now(g, vec![integration("manual", "sleep 30", Trigger::Manual)])
            .unwrap();
        wait_orchestrating(&runs, &run.id, "a");
        let front = tools_of(&host, "Front");
        let refused = |front: &ToolHandler| {
            let err = front
                .call("orbit_restart_node", &json!({"node": "m"}))
                .unwrap_err();
            assert!(err.contains("integração manual: só o dono roda"), "{err}");
        };
        refused(&front);
        assert_eq!(
            runs.get(&run.id).unwrap().nodes["m"].status,
            NodeStatus::Waiting
        );

        runs.run_node(&run.id, "m").unwrap();
        wait_status(&runs, &run.id, "m", NodeStatus::Running);
        refused(&front);
        assert_eq!(runs.get(&run.id).unwrap().nodes["m"].attempt, 0);

        runs.restart_node(&run.id, "m").unwrap();
        wait_attempt(&runs, &run.id, "m", 1, NodeStatus::Running);
        runs.stop(&run.id).unwrap();
    }

    /// A url impressa com segredo no caminho vira só `scheme://host:porta`
    /// no nó, no `url.txt` e no banco.
    #[cfg(unix)]
    #[test]
    fn printed_url_with_a_secret_never_reaches_the_node() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let command = format!(
            "echo 'pronto: http://localhost:5998/callback/{key}?token=x'; sleep 30",
            key = fake_key()
        );
        let run = runs
            .start_now(graph(vec![process("p", &command)], vec![]), vec![])
            .unwrap();
        let r = wait_until(&runs, &run.id, "sem url", |r| r.nodes["p"].url.is_some());
        assert_eq!(r.nodes["p"].url.as_deref(), Some("http://localhost:5998"));
        let out = dir.path().join("runs").join(&run.id).join("p").join("out");
        assert_eq!(
            std::fs::read_to_string(out.join("url.txt")).unwrap(),
            "http://localhost:5998"
        );
        let stored = runs.store.get_orbit_run(&run.id).unwrap().unwrap();
        assert_eq!(
            stored.nodes["p"].url.as_deref(),
            Some("http://localhost:5998")
        );
        runs.stop(&run.id).unwrap();
    }

    /// O fim do processo lê a saída declarada com que a tentativa subiu: o
    /// dono trocar a saída (e o comando) com ele rodando só vale no reinício.
    #[cfg(unix)]
    #[test]
    fn process_end_uses_the_config_it_started_with() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let mark = dir.path().join("mark");
        let g = graph(
            vec![
                process(
                    "p",
                    &format!(
                        "while [ ! -f '{m}' ]; do sleep 0.05; done; echo fim",
                        m = mark.display()
                    ),
                ),
                process("keep", "sleep 30"),
            ],
            vec![],
        );
        runs.store.save_orbit_graph(&g, true, None).unwrap();
        let run = runs.start_now(g.clone(), vec![]).unwrap();
        wait_until(&runs, &run.id, "p sobe", |r| {
            r.nodes["p"].session_id.is_some()
        });

        let mut edited = g.clone();
        let other = tempfile::tempdir().unwrap();
        edited.nodes[0].kind = NodeKind::Process {
            command: "exit 7".into(),
            output: OutputKind::Json,
            file: None,
            dir: other.path().to_string_lossy().into_owned(),
        };
        runs.store.save_orbit_graph(&edited, true, None).unwrap();
        runs.graph_saved(&edited);
        std::fs::write(&mark, "").unwrap();

        let r = wait_status(&runs, &run.id, "p", NodeStatus::Done);
        let p = &r.nodes["p"];
        assert_eq!(p.exit_code, Some(0), "{r:?}");
        assert!(p.outputs.contains_key(&EdgeKind::Log), "{p:?}");
        assert!(!p.outputs.contains_key(&EdgeKind::Json), "{p:?}");
        assert!(p.stale, "a config nova espera o reinício");
        runs.stop(&run.id).unwrap();
    }

    /// O navegador que ficou com a url velha (a origem rodou de novo e
    /// imprimiu outra) aplica a nova no reinício.
    #[cfg(unix)]
    #[test]
    fn restarting_the_browser_applies_the_new_url() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let mark = dir.path().join("mark");
        let command = format!(
            "if [ -f '{m}' ]; then echo http://localhost:4102/; else touch '{m}'; \
             echo http://localhost:4101/; fi; sleep 30",
            m = mark.display()
        );
        let g = graph(
            vec![process("p", &command), browser("b", "")],
            vec![edge("u", "p", "b", EdgeKind::Url)],
        );
        let run = runs.start_now(g, vec![]).unwrap();
        let r = wait_status(&runs, &run.id, "b", NodeStatus::Done);
        assert_eq!(r.nodes["b"].url.as_deref(), Some("http://localhost:4101/"));

        runs.restart_node(&run.id, "p").unwrap();
        let r = wait_until(&runs, &run.id, "p com a url nova", |r| {
            r.nodes["p"].url.as_deref() == Some("http://localhost:4102/")
        });
        let b = &r.nodes["b"];
        assert_eq!(b.stale_reason, Some(StaleReason::Source), "{b:?}");
        assert_eq!(b.url.as_deref(), Some("http://localhost:4101/"));

        runs.restart_node(&run.id, "b").unwrap();
        let r = wait_attempt(&runs, &run.id, "b", 1, NodeStatus::Done);
        let b = &r.nodes["b"];
        assert_eq!(b.url.as_deref(), Some("http://localhost:4102/"), "{r:?}");
        assert!(!b.stale);
        runs.stop(&run.id).unwrap();
    }

    /// O reinício limpa `in/` além da `out/`: a entrada de antes não sobra
    /// para a tentativa nova, que materializa as de agora.
    #[cfg(unix)]
    #[test]
    fn restart_clears_the_inputs_too() {
        let dir = tempfile::tempdir().unwrap();
        let (runs, _host, _app) = pty_runs(dir.path().join("runs"));
        let g = graph(
            vec![
                process("p", "echo oi"),
                process("q", "cat \"$TYBA_IN/log.txt\"; sleep 30"),
            ],
            vec![edge("e1", "p", "q", EdgeKind::Log)],
        );
        let run = runs.start_now(g, vec![]).unwrap();
        wait_until(&runs, &run.id, "q sobe", |r| {
            r.nodes["q"].session_id.is_some()
        });
        let in_dir = dir.path().join("runs").join(&run.id).join("q").join("in");
        std::fs::write(in_dir.join("sobra.txt"), "velho").unwrap();
        runs.restart_node(&run.id, "q").unwrap();
        assert!(!in_dir.join("sobra.txt").exists());
        wait_until(&runs, &run.id, "q de novo", |r| {
            r.nodes["q"].attempt == 1 && r.nodes["q"].session_id.is_some()
        });
        assert!(in_dir.join("log.txt").is_file());
        runs.stop(&run.id).unwrap();
    }
}
