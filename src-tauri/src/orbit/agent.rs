//! Nó de agente do Orbit: o CLI (Claude, Codex) sobe **livre e observado**,
//! como o claude digitado, na pasta do nó — sem worktree, sem jaula.
//!
//! Aqui fica o que é do nó e não do spawn: o prompt com o bloco "Entradas",
//! o snapshot de diff na largada e o diff no fim, o `context` redigido e a
//! espera pelo primeiro turno. O spawn mora em
//! [`crate::agent::session::spawn_free_agent`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::talk::PeerState;
use super::{EdgeKind, Runner};
use crate::agent::session::{SharedTurnTail, TurnTail};
use crate::session::{AgentRunnerKind, AwaitingReason, SessionStatus};

use super::{Edge, Integration, Node, NodeKind};

/// Teto do texto do turno que vira `context.md`.
const CONTEXT_MAX_CHARS: usize = 32_000;
const TURN_POLL: Duration = Duration::from_millis(300);
/// O `Stop` do Claude pode chegar antes de o transcript ter a última fala
/// (mesmo cuidado do aviso de fim de turno).
const TURN_SETTLE: Duration = Duration::from_millis(1500);

pub fn runner_kind(runner: Runner) -> AgentRunnerKind {
    match runner {
        Runner::Claude => AgentRunnerKind::ClaudeCode,
        Runner::Codex => AgentRunnerKind::Codex,
    }
}

fn kind_label(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Context => "contexto",
        EdgeKind::Diff => "diff",
        EdgeKind::File => "arquivo",
        EdgeKind::Log => "log",
        EdgeKind::Json => "json",
        EdgeKind::Url => "url",
        EdgeKind::Plan => "plano",
        EdgeKind::Control => "controla",
        EdgeKind::Talk => "conversa",
        EdgeKind::Memory => "memória",
    }
}

/// Um arquivo de texto pequeno recebido (uma nota ligada pela saída
/// "arquivo"), redigido, para entrar no prompt como o contexto entra. Binário,
/// texto que não é UTF-8 ou acima do teto fica só listado em "Entradas": o
/// dono ligava a nota pela bolinha de arquivo e o agente ignorava a instrução.
pub fn read_text_input(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > (CONTEXT_MAX_CHARS * 4) as u64 {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    if text.chars().count() > CONTEXT_MAX_CHARS {
        return None;
    }
    let text = text.trim();
    if text.is_empty() || text.contains('\0') {
        return None;
    }
    Some(context_markdown(text))
}

/// O texto de um `context.md` recebido, até o teto, para entrar no prompt.
pub fn read_context(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(CONTEXT_MAX_CHARS).collect())
}

/// O prompt do nó (variáveis já substituídas), o **contexto** recebido por
/// inteiro e o bloco "Entradas": uma linha por arquivo de `$TYBA_IN`.
///
/// Contexto vai no corpo, e não só como caminho: uma nota ligada a um agente
/// é a instrução dele, e listada como arquivo o agente a ignorava (teste do
/// dono, 15/10: a nota pedia 6 subagentes e ele rodou 3).
/// As partes do prompt do agente, na ordem do quadro OrbitPapeis: papel,
/// tarefa, ligados a você, contexto e entradas.
pub struct PromptParts<'a> {
    /// Nome e instruções (já com as variáveis) do papel da biblioteca.
    pub role: Option<(&'a str, &'a str)>,
    pub task: &'a str,
    pub contexts: &'a [(String, String)],
    pub inputs: &'a [(EdgeKind, PathBuf)],
    /// Uma linha por nó ligado ([`linked_lines`]).
    pub links: &'a [String],
}

/// Monta o prompt. Sem papel nem ligação, é a tarefa seguida de contexto e
/// entradas, como sempre foi; com papel, abre com "Papel: <nome>" e as
/// instruções, e a tarefa ganha o cabeçalho "Tarefa:".
pub fn build_prompt(parts: &PromptParts) -> String {
    if parts.role.is_none()
        && parts.links.is_empty()
        && parts.inputs.is_empty()
        && parts.contexts.is_empty()
    {
        return parts.task.to_string();
    }
    let mut out = String::new();
    match parts.role {
        Some((name, instructions)) => {
            out.push_str(&format!("Papel: {name}\n{}", instructions.trim()));
            let task = parts.task.trim();
            if !task.is_empty() {
                out.push_str("\n\nTarefa:\n");
                out.push_str(task);
            }
        }
        None => out.push_str(parts.task.trim_end()),
    }
    if !parts.links.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str("Ligados a você:");
        for line in parts.links {
            out.push_str("\n- ");
            out.push_str(line);
        }
    }
    for (heading, body) in parts.contexts {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(heading);
        out.push_str(":\n");
        out.push_str(body);
    }
    if parts.inputs.is_empty() {
        return out;
    }
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str("Entradas (arquivos em $TYBA_IN):");
    for (kind, path) in parts.inputs {
        out.push_str(&format!("\n- {} ({})", path.display(), kind_label(*kind)));
    }
    out
}

/// "Ligados a você": uma linha por nó ligado a `me`, na ordem das ligações
/// — `<título> · <o que ele é> → <o que a ligação faz>`. Dado que chega:
/// "manda contexto para você"; que sai: "recebe diff de você"; conversa:
/// "conversa com você"; memória: "você lê e escreve" (nota com cadeado:
/// "você lê"); plano: "você escreve o plano nela"; controla: "você controla".
pub fn linked_lines(
    nodes: &[Node],
    edges: &[Edge],
    integrations: &std::collections::HashMap<String, Integration>,
    me: &str,
) -> Vec<String> {
    let mut order: Vec<&str> = Vec::new();
    let mut incoming: std::collections::HashMap<&str, Vec<EdgeKind>> = Default::default();
    let mut outgoing: std::collections::HashMap<&str, Vec<EdgeKind>> = Default::default();
    for e in edges {
        let (other, bucket) = if e.to == me && e.from != me {
            (e.from.as_str(), &mut incoming)
        } else if e.from == me && e.to != me {
            (e.to.as_str(), &mut outgoing)
        } else {
            continue;
        };
        if !order.contains(&other) {
            order.push(other);
        }
        let kinds = bucket.entry(other).or_default();
        if !kinds.contains(&e.kind) {
            kinds.push(e.kind);
        }
    }
    order
        .into_iter()
        .filter_map(|id| {
            let node = nodes.iter().find(|n| n.id == id)?;
            let inc = incoming.get(id).cloned().unwrap_or_default();
            let out = outgoing.get(id).cloned().unwrap_or_default();
            let mut phrases: Vec<String> = Vec::new();
            let data_in: Vec<&str> = inc
                .iter()
                .filter(|k| k.carries_data())
                .map(|k| kind_label(*k))
                .collect();
            if !data_in.is_empty() {
                phrases.push(format!("manda {} para você", join_and(&data_in)));
            }
            let data_out: Vec<&str> = out
                .iter()
                .filter(|k| k.carries_data())
                .map(|k| kind_label(*k))
                .collect();
            if !data_out.is_empty() {
                phrases.push(format!("recebe {} de você", join_and(&data_out)));
            }
            if inc.contains(&EdgeKind::Talk) || out.contains(&EdgeKind::Talk) {
                phrases.push("conversa com você".into());
            }
            if out.contains(&EdgeKind::Memory) {
                let locked = matches!(&node.kind, NodeKind::Note { locked: true, .. });
                phrases.push(
                    if locked {
                        "você lê"
                    } else {
                        "você lê e escreve"
                    }
                    .into(),
                );
            }
            if out.contains(&EdgeKind::Plan) {
                phrases.push("você escreve o plano nela".into());
            }
            if out.contains(&EdgeKind::Control) {
                phrases.push("você controla".into());
            }
            if phrases.is_empty() {
                return None;
            }
            Some(format!(
                "{} · {} → {}",
                super::talk::clean_title(&node.title).trim(),
                describe(&node.kind, integrations),
                phrases.join(" · ")
            ))
        })
        .collect()
}

fn join_and(items: &[&str]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].to_string(),
        n => format!("{} e {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

/// O que o nó é, para a linha de "Ligados a você": o papel do agente (ou
/// "agente") e o runner, a primeira linha do comando, o nome da integração.
fn describe(
    kind: &NodeKind,
    integrations: &std::collections::HashMap<String, Integration>,
) -> String {
    match kind {
        NodeKind::Agent { runner, role, .. } => {
            let runner = match runner {
                Runner::Claude => "Claude",
                Runner::Codex => "Codex",
            };
            let role = super::talk::clean_title(role.trim());
            if role.is_empty() {
                format!("agente ({runner})")
            } else {
                format!("{role} ({runner})")
            }
        }
        NodeKind::Process { command, .. } => format!("processo \"{}\"", short_line(command)),
        NodeKind::Note { .. } => "nota".into(),
        NodeKind::Integration { integration_id, .. } => format!(
            "integração \"{}\"",
            integrations
                .get(integration_id)
                .map_or(integration_id.as_str(), |i| i.name.as_str())
        ),
        NodeKind::Browser { .. } => "navegador".into(),
        NodeKind::Service { .. } => "serviço".into(),
        NodeKind::Terminal { .. } => "terminal".into(),
    }
}

/// A primeira linha do texto, até 60 caracteres.
fn short_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    let mut out: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        out.push('…');
    }
    out
}

/// O texto que sai como `context`, sempre pela redação.
pub fn context_markdown(text: &str) -> String {
    crate::session::redact::redact(text).into_owned()
}

/// A fala final do turno: a do payload (Codex) ou a última do transcript.
fn turn_text(tail: &TurnTail) -> Option<String> {
    if let Some(m) = &tail.last_message {
        return Some(m.clone());
    }
    tail.transcript_path
        .as_deref()
        .and_then(|p| crate::status::transcript::last_assistant_text(p, CONTEXT_MAX_CHARS))
}

/// Como o primeiro turno do agente acabou.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentTurn {
    /// Turno fechado (`Idle`), com a fala final quando houver.
    Done { text: Option<String> },
    /// Turno acabou por erro da API.
    Failed(String),
    /// O agente saiu antes de fechar o turno.
    Exited,
}

#[derive(Debug, PartialEq)]
enum TurnState {
    Pending,
    Ended,
    Gone,
}

/// `Idle` é o turno fechado — inclusive o fechamento segurado por
/// subagentes, que só desce a `Idle` quando o último termina. Prompt aberto
/// (`AwaitingInput`) segue esperando o dono responder no terminal do nó.
fn turn_state(status: Option<&SessionStatus>) -> TurnState {
    match status {
        Some(SessionStatus::Idle { .. }) => TurnState::Ended,
        Some(SessionStatus::Running | SessionStatus::AwaitingInput { .. }) => TurnState::Pending,
        Some(SessionStatus::Exited { .. } | SessionStatus::Failed { .. }) | None => TurnState::Gone,
    }
}

fn turn_from_tail(tail: &TurnTail) -> AgentTurn {
    match &tail.failure {
        Some(reason) => AgentTurn::Failed(reason.clone()),
        None => AgentTurn::Done {
            text: turn_text(tail),
        },
    }
}

/// Espera o primeiro turno numa thread e chama `on_turn` uma vez. O agente
/// continua vivo depois; isto só avisa o Run. Com `on_exit` (o orquestrador),
/// segue olhando a sessão pelo mesmo estado e chama `on_exit` quando ela sai.
/// `on_state` recebe o estado a cada poll (o turno fechado só depois do
/// `on_turn`): é o que entrega a fila da conversa.
pub fn watch_first_turn(
    status: impl Fn() -> Option<SessionStatus> + Send + 'static,
    tail: SharedTurnTail,
    on_turn: Box<dyn FnOnce(AgentTurn) + Send>,
    on_exit: Option<Box<dyn FnOnce() + Send>>,
    on_state: OnState,
) {
    std::thread::spawn(move || {
        loop {
            let status = status();
            match turn_state(status.as_ref()) {
                TurnState::Pending => {
                    if let Some(s) = peer_state(status.as_ref()) {
                        on_state(s, waiting_you(status.as_ref()));
                    }
                    std::thread::sleep(TURN_POLL)
                }
                TurnState::Gone => return on_turn(AgentTurn::Exited),
                TurnState::Ended => {
                    std::thread::sleep(TURN_SETTLE);
                    let tail = tail.lock().expect("turn tail lock").clone();
                    on_turn(turn_from_tail(&tail));
                    break;
                }
            }
        }
        let Some(on_exit) = on_exit else { return };
        loop {
            let status = status();
            if turn_state(status.as_ref()) == TurnState::Gone {
                break;
            }
            if let Some(s) = peer_state(status.as_ref()) {
                on_state(s, waiting_you(status.as_ref()));
            }
            std::thread::sleep(TURN_POLL);
        }
        on_exit();
    });
}

/// O que o vigia da sessão avisa a cada poll: o estado para a conversa e,
/// quando o agente espera o dono, o pedido (quadro OrbitEsperando).
pub type OnState = Box<dyn Fn(PeerState, Option<Attention>) + Send>;

/// Por que o agente espera o dono. Só a permissão: o turno fechado com
/// pergunta o core não distingue do turno que só terminou (o `idle_prompt`
/// do Claude chega nos dois e não muda mais o status), então não entra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WaitReason {
    Permission,
}

/// O agente parado esperando o dono, com o que ele pede (o resumo da
/// chamada que o status da sessão já traz, redigido no `set_status`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attention {
    pub reason: WaitReason,
    pub hint: Option<String>,
}

/// O pedido de permissão aberto na sessão: o prompt nativo do Claude
/// (`PermissionRequest`) ou a aprovação do gate (`Approval`). A resposta é
/// no terminal do agente; o TYBA não aprova nada daqui. Esperando resposta
/// (`Reply`), trabalhando, turno fechado ou sessão morta: nada.
pub fn waiting_you(status: Option<&SessionStatus>) -> Option<Attention> {
    match status? {
        SessionStatus::AwaitingInput {
            hint,
            reason: AwaitingReason::NativePrompt | AwaitingReason::Approval,
        } => Some(Attention {
            reason: WaitReason::Permission,
            hint: hint.clone().filter(|h| !h.trim().is_empty()),
        }),
        _ => None,
    }
}

/// O estado da sessão para a conversa: `Idle` é o turno fechado.
fn peer_state(status: Option<&SessionStatus>) -> Option<PeerState> {
    match status? {
        SessionStatus::Idle { .. } => Some(PeerState::Ready),
        SessionStatus::AwaitingInput { .. } => Some(PeerState::Waiting),
        SessionStatus::Running => Some(PeerState::Working),
        SessionStatus::Exited { .. } | SessionStatus::Failed { .. } => None,
    }
}

/// O estado da pasta na largada do nó. Sem repo git não há snapshot (e o nó
/// não produz diff).
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub(crate) root: PathBuf,
    pub(crate) base: String,
    pub(crate) untracked: HashSet<String>,
}

fn git_stdout(mut cmd: std::process::Command, ok_codes: &[i32]) -> Option<Vec<u8>> {
    let out = cmd.output().ok()?;
    ok_codes.contains(&out.status.code()?).then_some(out.stdout)
}

fn untracked(root: &Path) -> Option<HashSet<String>> {
    let mut cmd = crate::worktree::git_in(root);
    cmd.args(["ls-files", "-z", "--others", "--exclude-standard"]);
    let out = git_stdout(cmd, &[0])?;
    Some(
        out.split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect(),
    )
}

/// `git stash create` (a árvore suja vira um commit solto, sem mexer em nada)
/// ou `HEAD` com a árvore limpa. A identidade fixa só assina esse commit
/// solto, que nunca vai para branch nenhuma.
pub fn snapshot(dir: &Path) -> Option<Snapshot> {
    let root = crate::repo::canonicalize_or(&crate::repo::toplevel(dir)?);
    let mut stash = crate::worktree::git_in_rw(&root);
    stash.args([
        "-c",
        "user.name=TYBA",
        "-c",
        "user.email=orbit@tyba.invalid",
        "stash",
        "create",
    ]);
    let created = git_stdout(stash, &[0])
        .map(|o| String::from_utf8_lossy(&o).trim().to_string())
        .filter(|s| !s.is_empty());
    let base = match created {
        Some(sha) => sha,
        None => {
            let mut head = crate::worktree::git_in(&root);
            head.args(["rev-parse", "--verify", "HEAD"]);
            let out = git_stdout(head, &[0])?;
            String::from_utf8_lossy(&out).trim().to_string()
        }
    };
    let untracked = untracked(&root)?;
    Some(Snapshot {
        root,
        base,
        untracked,
    })
}

/// `git diff <snapshot>` mais cada arquivo não rastreado que surgiu depois da
/// largada, como arquivo novo. Arquivo não rastreado que já existia fica de
/// fora (não é do nó).
pub fn diff_since(snap: &Snapshot) -> String {
    let mut out = Vec::new();
    let mut diff = crate::worktree::git_in(&snap.root);
    diff.args(["diff", "--no-color", "--no-ext-diff", &snap.base, "--"]);
    if let Some(bytes) = git_stdout(diff, &[0]) {
        out.extend(bytes);
    }
    let mut fresh: Vec<String> = untracked(&snap.root)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| !snap.untracked.contains(f))
        .collect();
    fresh.sort();
    for file in fresh {
        let mut cmd = crate::worktree::git_in(&snap.root);
        cmd.args([
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-index",
            "--",
            "/dev/null",
            &file,
        ]);
        // `--no-index` sai 1 quando há diferença.
        if let Some(bytes) = git_stdout(cmd, &[0, 1]) {
            out.extend(bytes);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Só o pedido de permissão (prompt nativo do Claude, aprovação do gate)
    /// é "esperando você"; resposta, trabalho, turno fechado e sessão morta,
    /// não.
    #[test]
    fn waiting_you_is_only_the_permission_request() {
        use crate::session::AwaitingReason as R;
        let ask = |reason, hint: Option<&str>| SessionStatus::AwaitingInput {
            hint: hint.map(String::from),
            reason,
        };
        for reason in [R::NativePrompt, R::Approval] {
            assert_eq!(
                waiting_you(Some(&ask(reason, Some("bun test src/api")))),
                Some(Attention {
                    reason: WaitReason::Permission,
                    hint: Some("bun test src/api".into()),
                })
            );
        }
        assert_eq!(
            waiting_you(Some(&ask(R::NativePrompt, Some("  "))))
                .unwrap()
                .hint,
            None
        );
        for other in [
            ask(R::Reply, Some("e agora?")),
            SessionStatus::Running,
            SessionStatus::Idle { summary: None },
            SessionStatus::Exited { code: 0 },
            SessionStatus::Failed { reason: "x".into() },
        ] {
            assert_eq!(waiting_you(Some(&other)), None, "{other:?}");
        }
        assert_eq!(waiting_you(None), None);
    }

    fn prompt(task: &str, contexts: &[(String, String)], inputs: &[(EdgeKind, PathBuf)]) -> String {
        build_prompt(&PromptParts {
            role: None,
            task,
            contexts,
            inputs,
            links: &[],
        })
    }

    #[test]
    fn prompt_lists_each_input_in_one_line() {
        let p = prompt(
            "Revise o diff.\n",
            &[],
            &[
                (EdgeKind::Diff, PathBuf::from("/r/n/in/diff.patch")),
                (EdgeKind::Context, PathBuf::from("/r/n/in/context.md")),
            ],
        );
        assert_eq!(
            p,
            "Revise o diff.\n\nEntradas (arquivos em $TYBA_IN):\n\
             - /r/n/in/diff.patch (diff)\n\
             - /r/n/in/context.md (contexto)"
        );
        assert_eq!(prompt("só isto", &[], &[]), "só isto");
        assert_eq!(
            prompt("", &[], &[(EdgeKind::Log, PathBuf::from("/i/log.txt"))]),
            "Entradas (arquivos em $TYBA_IN):\n- /i/log.txt (log)"
        );
    }

    #[test]
    fn context_goes_into_the_prompt_body() {
        let p = prompt(
            "",
            &[("Contexto".into(), "rode 6 subagentes com sleep 10".into())],
            &[(EdgeKind::Context, PathBuf::from("/r/n/in/context.md"))],
        );
        assert_eq!(
            p,
            "Contexto:\nrode 6 subagentes com sleep 10\n\n\
             Entradas (arquivos em $TYBA_IN):\n\
             - /r/n/in/context.md (contexto)"
        );
    }

    /// Quadro OrbitPapeis C: o papel abre o prompt, a tarefa vem depois com
    /// cabeçalho, os ligados antes do contexto; sem papel e sem ligação, o
    /// prompt é o de sempre (sem "Tarefa:").
    #[test]
    fn role_opens_the_prompt_and_links_come_before_the_context() {
        let links = vec!["dev · Desenvolvedor (Claude) → manda contexto para você".to_string()];
        let p = build_prompt(&PromptParts {
            role: Some(("Revisor", "Você revisa o diff de outro agente.\n")),
            task: "Revise o diff contra main.\n",
            contexts: &[("Contexto".into(), "pronto".into())],
            inputs: &[(EdgeKind::Context, PathBuf::from("/r/in/context.md"))],
            links: &links,
        });
        assert_eq!(
            p,
            "Papel: Revisor\nVocê revisa o diff de outro agente.\n\n\
             Tarefa:\nRevise o diff contra main.\n\n\
             Ligados a você:\n- dev · Desenvolvedor (Claude) → manda contexto para você\n\n\
             Contexto:\npronto\n\n\
             Entradas (arquivos em $TYBA_IN):\n- /r/in/context.md (contexto)"
        );
        // Papel sem tarefa: só o papel.
        assert_eq!(
            build_prompt(&PromptParts {
                role: Some(("QA", "Reproduza o bug.")),
                task: "  ",
                contexts: &[],
                inputs: &[],
                links: &[],
            }),
            "Papel: QA\nReproduza o bug."
        );
        // Ligação sem papel: a tarefa segue abrindo, sem "Tarefa:".
        assert_eq!(
            build_prompt(&PromptParts {
                role: None,
                task: "Suba a api.",
                contexts: &[],
                inputs: &[],
                links: &links,
            }),
            "Suba a api.\n\nLigados a você:\n- dev · Desenvolvedor (Claude) → manda contexto para você"
        );
    }

    fn node(id: &str, title: &str, kind: NodeKind) -> Node {
        Node {
            id: id.into(),
            title: title.into(),
            kind,
            position: super::super::Position { x: 0.0, y: 0.0 },
            size: super::super::Size { w: 1.0, h: 1.0 },
        }
    }

    fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge {
            id: format!("{from}-{to}-{kind:?}"),
            from: from.into(),
            to: to.into(),
            kind,
        }
    }

    /// Quadro OrbitPapeis C: cada nó ligado numa linha, com nome e papel; os
    /// dados agrupados por sentido; nota com cadeado é só leitura; nó ligado
    /// a outros, não a mim, não aparece; título é saneado.
    #[test]
    fn linked_lines_name_each_neighbour_with_its_role_and_direction() {
        let agent = |role: &str, runner| NodeKind::Agent {
            runner,
            role: role.into(),
            prompt: "x".into(),
            dir: String::new(),
            no_create: false,
        };
        let nodes = vec![
            node("rev", "rev", agent("Revisor", Runner::Claude)),
            node("dev", "dev", agent("Desenvolvedor", Runner::Claude)),
            node(
                "testes",
                "testes",
                NodeKind::Process {
                    command: "cargo test\n# segunda linha".into(),
                    output: Default::default(),
                    file: None,
                    dir: String::new(),
                },
            ),
            node(
                "notas",
                "no[tas]\u{2028}",
                NodeKind::Note {
                    markdown: String::new(),
                    locked: false,
                },
            ),
            node(
                "regras",
                "regras",
                NodeKind::Note {
                    markdown: String::new(),
                    locked: true,
                },
            ),
            node("qa", "qa", agent("", Runner::Codex)),
            node(
                "lint",
                "lint",
                NodeKind::Integration {
                    integration_id: "i-1".into(),
                    dir: String::new(),
                },
            ),
            node("web", "web", NodeKind::Browser { url: String::new() }),
        ];
        let edges = vec![
            edge("dev", "rev", EdgeKind::Context),
            edge("dev", "rev", EdgeKind::Diff),
            edge("testes", "rev", EdgeKind::Log),
            edge("rev", "notas", EdgeKind::Memory),
            edge("rev", "regras", EdgeKind::Memory),
            edge("rev", "qa", EdgeKind::Talk),
            edge("rev", "lint", EdgeKind::Diff),
            edge("rev", "web", EdgeKind::Control),
            edge("dev", "qa", EdgeKind::Context),
        ];
        let integrations = std::collections::HashMap::from([(
            "i-1".to_string(),
            Integration {
                id: "i-1".into(),
                name: "Lint".into(),
                script: String::new(),
                trigger: super::super::Trigger::Manual,
                output: Default::default(),
                file: None,
            },
        )]);
        assert_eq!(
            linked_lines(&nodes, &edges, &integrations, "rev"),
            vec![
                "dev · Desenvolvedor (Claude) → manda contexto e diff para você",
                "testes · processo \"cargo test\" → manda log para você",
                "notas · nota → você lê e escreve",
                "regras · nota → você lê",
                "qa · agente (Codex) → conversa com você",
                "lint · integração \"Lint\" → recebe diff de você",
                "web · navegador → você controla",
            ]
        );
        // Do outro lado: quem manda vê "recebe … de você", com o papel de quem recebe.
        assert_eq!(
            linked_lines(&nodes, &edges, &integrations, "dev"),
            vec![
                "rev · Revisor (Claude) → recebe contexto e diff de você",
                "qa · agente (Codex) → recebe contexto de você",
            ]
        );
        assert!(linked_lines(&nodes, &[], &integrations, "rev").is_empty());
    }

    #[test]
    fn small_text_file_input_is_read_and_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let note = dir.path().join("instrucao.md");
        let key = format!("sk-{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6");
        std::fs::write(&note, format!("suba a api\nchave {key}\n")).unwrap();
        let text = read_text_input(&note).unwrap();
        assert!(text.starts_with("suba a api"));
        assert!(!text.contains(&key));

        let bin = dir.path().join("x.bin");
        std::fs::write(&bin, [0u8, 159, 146, 150]).unwrap();
        assert_eq!(read_text_input(&bin), None);
        let big = dir.path().join("big.txt");
        std::fs::write(&big, "a".repeat(CONTEXT_MAX_CHARS + 1)).unwrap();
        assert_eq!(read_text_input(&big), None);
    }

    #[test]
    fn context_goes_through_redaction() {
        // Fixture sintética.
        let key = format!("sk-{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6");
        let out = context_markdown(&format!("usei a chave {key} no teste"));
        assert!(!out.contains(&key), "{out}");
        assert!(out.contains("usei a chave"));

        let tail = TurnTail {
            last_message: Some(format!("fim {key}")),
            ..TurnTail::default()
        };
        let AgentTurn::Done { text: Some(text) } = turn_from_tail(&tail) else {
            panic!("turno fechado com texto");
        };
        assert!(!context_markdown(&text).contains(&key));
    }

    #[test]
    fn turn_state_waits_for_idle_and_fails_on_exit() {
        assert_eq!(
            turn_state(Some(&SessionStatus::Running)),
            TurnState::Pending
        );
        assert_eq!(
            turn_state(Some(&SessionStatus::AwaitingInput {
                hint: None,
                reason: Default::default()
            })),
            TurnState::Pending
        );
        assert_eq!(
            turn_state(Some(&SessionStatus::Idle { summary: None })),
            TurnState::Ended
        );
        assert_eq!(
            turn_state(Some(&SessionStatus::Exited { code: 0 })),
            TurnState::Gone
        );
        assert_eq!(turn_state(None), TurnState::Gone);
        let failed = TurnTail {
            failure: Some("API sobrecarregada".into()),
            ..TurnTail::default()
        };
        assert_eq!(
            turn_from_tail(&failed),
            AgentTurn::Failed("API sobrecarregada".into())
        );
    }

    #[test]
    fn watcher_reports_the_exit_after_the_first_turn_when_asked() {
        use std::sync::{mpsc, Arc, Mutex};
        // Turno fecha, outro turno roda, a sessão some.
        let states = Arc::new(Mutex::new(vec![
            None,
            Some(SessionStatus::Idle { summary: None }),
            Some(SessionStatus::Running),
            Some(SessionStatus::Idle { summary: None }),
        ]));
        let status = {
            let states = Arc::clone(&states);
            move || {
                let mut s = states.lock().unwrap();
                if s.len() > 1 {
                    s.pop().unwrap()
                } else {
                    s[0].clone()
                }
            }
        };
        let (tx, rx) = mpsc::channel();
        let turn_tx = tx.clone();
        watch_first_turn(
            status,
            Arc::new(Mutex::new(TurnTail::default())),
            Box::new(move |turn| turn_tx.send(format!("{turn:?}")).unwrap()),
            Some(Box::new(move || tx.send("saiu".into()).unwrap())),
            Box::new(|_, _| {}),
        );
        let wait = Duration::from_secs(10);
        assert!(rx.recv_timeout(wait).unwrap().starts_with("Done"));
        assert_eq!(rx.recv_timeout(wait).unwrap(), "saiu");
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "t"]);
        std::fs::write(dir.path().join("a.txt"), "um\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn diff_shows_changed_and_new_files() {
        let dir = repo();
        let snap = snapshot(dir.path()).expect("repo git tem snapshot");
        std::fs::write(dir.path().join("a.txt"), "um\ndois\n").unwrap();
        std::fs::write(dir.path().join("novo.txt"), "oi\n").unwrap();
        let patch = diff_since(&snap);
        assert!(patch.contains("a/a.txt"), "{patch}");
        assert!(patch.contains("+dois"), "{patch}");
        assert!(patch.contains("novo.txt"), "{patch}");
        assert!(patch.contains("new file"), "{patch}");
        assert!(patch.contains("+oi"), "{patch}");
    }

    #[test]
    fn clean_repo_without_changes_gives_empty_patch() {
        let dir = repo();
        let snap = snapshot(dir.path()).unwrap();
        assert_eq!(diff_since(&snap), "");
    }

    #[test]
    fn changes_from_before_the_node_stay_out() {
        let dir = repo();
        std::fs::write(dir.path().join("a.txt"), "um\nantes\n").unwrap();
        std::fs::write(dir.path().join("velho.txt"), "x\n").unwrap();
        let snap = snapshot(dir.path()).unwrap();
        assert_eq!(diff_since(&snap), "");
        std::fs::write(dir.path().join("a.txt"), "um\nantes\ndepois\n").unwrap();
        let patch = diff_since(&snap);
        assert!(patch.contains("+depois"), "{patch}");
        assert!(!patch.contains("+antes"), "{patch}");
        assert!(!patch.contains("velho.txt"), "{patch}");
    }

    #[test]
    fn no_git_no_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        assert!(snapshot(dir.path()).is_none());
    }
}
