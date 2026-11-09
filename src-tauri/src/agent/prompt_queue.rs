//! Fila de prompts por sessão (proposta 12 da IA v2).
//!
//! A pessoa enfileira N prompts; quando o agente da sessão fecha o turno — e
//! "fechou" aqui é só o que veio de hook (`Stop`, ou o último subagente do
//! turno terminando), nunca palpite de tela —, o core manda o próximo pelo
//! mesmo caminho do input rico ([`crate::inject_rich_input`]).
//!
//! A fila pausa sozinha quando o agente para com falha (`StopFailure`), quando
//! o turno é interrompido, quando há pedido de aprovação aberto e quando o
//! agente ou a sessão morrem; só a pessoa retoma. Nunca manda com o agente
//! trabalhando.
//!
//! Só memória: prompt é texto livre do dono e pode carregar segredo — nada
//! daqui vai para o SQLite.

use std::collections::HashMap;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::session::{AwaitingReason, SessionId, SessionKind, SessionStatus};

/// O estado inteiro da fila de uma sessão, a cada mudança.
pub const EVENT_CHANGED: &str = "prompt-queue://changed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    /// A pessoa pausou.
    User,
    /// O turno acabou por erro da API (`StopFailure`: limite, auth, rede…).
    Failed,
    /// A pessoa interrompeu o turno.
    Interrupted,
    /// O agente está esperando uma aprovação.
    Approval,
    /// O agente saiu (`SessionEnd`); o pane pode seguir vivo, com um shell.
    AgentEnded,
    /// A sessão morreu.
    SessionEnded,
    /// Escrever no terminal falhou; o prompt voltou para o topo.
    SendFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueuedPrompt {
    pub id: Uuid,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueueState {
    pub session_id: SessionId,
    pub prompts: Vec<QueuedPrompt>,
    pub paused: Option<PauseReason>,
}

/// O que a fila sabe do agente, só pelo que ela viu acontecer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Busy,
    Idle,
    /// O agente saiu: um `Idle` que venha depois não é ele esperando.
    Gone,
}

/// Os fatos que movem a fila.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// O turno fechou (hook). É o único sinal que manda o próximo.
    TurnEnded,
    /// O status entrou em `Idle` por outro motivo (falha, interrupção):
    /// o agente está parado, mas sem convite para o próximo.
    Settled,
    Working,
    Approval,
    Failed,
    Interrupted,
    AgentEnded,
    SessionEnded,
}

#[derive(Debug)]
struct Queue {
    prompts: Vec<QueuedPrompt>,
    paused: Option<PauseReason>,
    phase: Phase,
}

impl Queue {
    fn new(agent_idle: bool) -> Self {
        Queue {
            prompts: Vec::new(),
            paused: None,
            phase: if agent_idle { Phase::Idle } else { Phase::Busy },
        }
    }

    /// O próximo prompt, se dá para mandar agora. Tirá-lo marca o agente como
    /// ocupado: ninguém manda dois seguidos sem um fim de turno no meio.
    fn take_next(&mut self) -> Option<QueuedPrompt> {
        if self.paused.is_some() || self.phase != Phase::Idle || self.prompts.is_empty() {
            return None;
        }
        self.phase = Phase::Busy;
        Some(self.prompts.remove(0))
    }

    /// Pausa automática só vale com prompt esperando: fila vazia não guarda
    /// a falha de um turno que ela nem mandou.
    fn auto_pause(&mut self, reason: PauseReason) {
        if !self.prompts.is_empty() {
            self.paused = Some(reason);
        }
    }

    fn on_signal(&mut self, signal: Signal) -> Option<QueuedPrompt> {
        match signal {
            Signal::TurnEnded => {
                self.phase = Phase::Idle;
                return self.take_next();
            }
            Signal::Settled => {
                if self.phase != Phase::Gone {
                    self.phase = Phase::Idle;
                }
            }
            Signal::Working => self.phase = Phase::Busy,
            Signal::Approval => {
                self.phase = Phase::Busy;
                self.auto_pause(PauseReason::Approval);
            }
            Signal::Failed => self.auto_pause(PauseReason::Failed),
            Signal::Interrupted => self.auto_pause(PauseReason::Interrupted),
            Signal::AgentEnded => {
                self.phase = Phase::Gone;
                self.auto_pause(PauseReason::AgentEnded);
            }
            Signal::SessionEnded => {
                self.phase = Phase::Gone;
                self.auto_pause(PauseReason::SessionEnded);
            }
        }
        None
    }

    fn push(&mut self, text: String) -> Option<QueuedPrompt> {
        self.prompts.push(QueuedPrompt {
            id: Uuid::new_v4(),
            text,
        });
        self.take_next()
    }

    fn resume(&mut self) -> Option<QueuedPrompt> {
        self.paused = None;
        self.take_next()
    }

    fn remove(&mut self, id: Uuid) -> bool {
        let before = self.prompts.len();
        self.prompts.retain(|p| p.id != id);
        self.prompts.len() != before
    }

    /// `order` tem de ser exatamente os ids da fila, em outra ordem: lista
    /// velha (um prompt saiu no meio) é recusada, não remendada.
    fn reorder(&mut self, order: &[Uuid]) -> Result<(), String> {
        let mut current: Vec<Uuid> = self.prompts.iter().map(|p| p.id).collect();
        let mut wanted = order.to_vec();
        current.sort();
        wanted.sort();
        if current != wanted {
            return Err("a fila mudou; recarregue a ordem".into());
        }
        let mut old = std::mem::take(&mut self.prompts);
        for id in order {
            let at = old.iter().position(|p| p.id == *id).expect("permutação");
            self.prompts.push(old.remove(at));
        }
        Ok(())
    }

    /// A escrita falhou: o prompt volta para o topo e a fila para.
    fn send_failed(&mut self, prompt: QueuedPrompt) {
        self.prompts.insert(0, prompt);
        self.paused = Some(PauseReason::SendFailed);
    }
}

#[derive(Default)]
pub struct QueueBook {
    queues: Mutex<HashMap<SessionId, Queue>>,
}

/// O que uma operação devolve: o estado para emitir (quando mudou) e o
/// prompt a mandar agora, se houver.
type Outcome = (Option<QueueState>, Option<QueuedPrompt>);

impl QueueBook {
    fn snapshot(session_id: SessionId, q: &Queue) -> QueueState {
        QueueState {
            session_id,
            prompts: q.prompts.clone(),
            paused: q.paused,
        }
    }

    pub fn state(&self, session_id: SessionId) -> QueueState {
        match self.queues.lock().get(&session_id) {
            Some(q) => Self::snapshot(session_id, q),
            None => QueueState {
                session_id,
                prompts: Vec::new(),
                paused: None,
            },
        }
    }

    /// Muda a fila (se existir) e devolve o estado só quando algo visível
    /// mudou.
    fn with_existing(
        &self,
        session_id: SessionId,
        f: impl FnOnce(&mut Queue) -> Option<QueuedPrompt>,
    ) -> Outcome {
        let mut queues = self.queues.lock();
        let Some(q) = queues.get_mut(&session_id) else {
            return (None, None);
        };
        let before = Self::snapshot(session_id, q);
        let send = f(q);
        let after = Self::snapshot(session_id, q);
        ((before != after).then_some(after), send)
    }

    /// A fila nasce no primeiro prompt, com o agente parado só se o status
    /// (vindo de hook) diz `Idle`.
    pub fn enqueue(&self, session_id: SessionId, agent_idle: bool, text: String) -> Outcome {
        let mut queues = self.queues.lock();
        let q = queues
            .entry(session_id)
            .or_insert_with(|| Queue::new(agent_idle));
        let send = q.push(text);
        (Some(Self::snapshot(session_id, q)), send)
    }

    pub fn signal(&self, session_id: SessionId, signal: Signal) -> Outcome {
        self.with_existing(session_id, |q| q.on_signal(signal))
    }

    pub fn pause(&self, session_id: SessionId) -> Outcome {
        self.with_existing(session_id, |q| {
            q.paused = Some(PauseReason::User);
            None
        })
    }

    pub fn resume(&self, session_id: SessionId) -> Outcome {
        self.with_existing(session_id, Queue::resume)
    }

    pub fn remove(&self, session_id: SessionId, id: Uuid) -> Outcome {
        self.with_existing(session_id, |q| {
            q.remove(id);
            None
        })
    }

    pub fn reorder(&self, session_id: SessionId, order: &[Uuid]) -> Result<Outcome, String> {
        let mut result = Ok(());
        let outcome = self.with_existing(session_id, |q| {
            result = q.reorder(order);
            None
        });
        result.map(|()| outcome)
    }

    pub fn clear(&self, session_id: SessionId) -> Outcome {
        self.with_existing(session_id, |q| {
            q.prompts.clear();
            q.paused = None;
            None
        })
    }

    pub fn send_failed(&self, session_id: SessionId, prompt: QueuedPrompt) -> Outcome {
        self.with_existing(session_id, |q| {
            q.send_failed(prompt);
            None
        })
    }

    /// A sessão foi descartada.
    pub fn forget(&self, session_id: SessionId) {
        self.queues.lock().remove(&session_id);
    }
}

/// O sinal que uma mudança de status dá à fila. `Idle` só conta quando o
/// status ENTRA em `Idle`: o `Idle → Idle` do resumo que assenta depois do
/// turno não é fim de turno nenhum.
pub fn signal_for_status(prev: &SessionStatus, next: &SessionStatus) -> Option<Signal> {
    match next {
        SessionStatus::Running => Some(Signal::Working),
        SessionStatus::AwaitingInput {
            reason: AwaitingReason::Approval | AwaitingReason::NativePrompt,
            ..
        } => Some(Signal::Approval),
        SessionStatus::AwaitingInput { .. } => None,
        SessionStatus::Idle { .. } if !matches!(prev, SessionStatus::Idle { .. }) => {
            Some(Signal::Settled)
        }
        SessionStatus::Idle { .. } => None,
        SessionStatus::Exited { .. } | SessionStatus::Failed { .. } => Some(Signal::SessionEnded),
    }
}

static BOOK: std::sync::OnceLock<QueueBook> = std::sync::OnceLock::new();

pub fn book() -> &'static QueueBook {
    BOOK.get_or_init(QueueBook::default)
}

/// Emite o estado (quando mudou) e manda o prompt (quando há). O envio roda
/// fora de quem chamou: este caminho sai de dentro do handler de hook, e o
/// input rico dorme entre a colagem e o Enter.
pub fn apply(app: &AppHandle, session_id: SessionId, (state, send): Outcome) {
    if let Some(state) = state {
        let _ = app.emit(EVENT_CHANGED, state);
    }
    let Some(prompt) = send else {
        return;
    };
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<crate::AppState>();
        if let Err(e) = deliver(&state, session_id, &prompt.text) {
            eprintln!("[tyba] fila de prompts de {session_id}: envio falhou: {e}");
            let outcome = book().send_failed(session_id, prompt);
            apply(&app, session_id, outcome);
        }
    });
}

/// Num pane de shell, o prompt só vai se o agente ainda está lá: sem ele,
/// o texto viraria comando do shell.
fn deliver(state: &crate::AppState, session_id: SessionId, text: &str) -> Result<(), String> {
    let session = state
        .sessions
        .get(session_id)
        .ok_or_else(|| "sessão sumiu".to_string())?;
    if matches!(session.kind, SessionKind::Shell)
        && state.agent_prober.detected(session_id).is_none()
    {
        return Err("nenhum agente no pane".into());
    }
    crate::inject_rich_input(state, session_id, text, true)
}

/// Um fato de hook sobre a sessão.
pub fn on_signal(app: &AppHandle, session_id: SessionId, signal: Signal) {
    apply(app, session_id, book().signal(session_id, signal));
}

/// Chamado a cada status novo (ver `SessionManager::set_status`).
pub fn on_status(
    app: &AppHandle,
    session_id: SessionId,
    prev: &SessionStatus,
    next: &SessionStatus,
) {
    if let Some(signal) = signal_for_status(prev, next) {
        on_signal(app, session_id, signal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionId {
        SessionId::new_v4()
    }

    fn texts(state: &QueueState) -> Vec<&str> {
        state.prompts.iter().map(|p| p.text.as_str()).collect()
    }

    fn sent(outcome: Outcome) -> Option<String> {
        outcome.1.map(|p| p.text)
    }

    #[test]
    fn enfileirar_com_agente_trabalhando_nao_envia() {
        let book = QueueBook::default();
        let s = session();
        assert_eq!(sent(book.enqueue(s, false, "a".into())), None);
        assert_eq!(sent(book.signal(s, Signal::Working)), None);
        assert_eq!(texts(&book.state(s)), vec!["a"]);
    }

    #[test]
    fn fim_de_turno_envia_um_por_vez_na_ordem() {
        let book = QueueBook::default();
        let s = session();
        book.enqueue(s, false, "a".into());
        book.enqueue(s, false, "b".into());
        book.enqueue(s, false, "c".into());
        assert_eq!(
            sent(book.signal(s, Signal::TurnEnded)).as_deref(),
            Some("a")
        );
        assert_eq!(texts(&book.state(s)), vec!["b", "c"]);
        // Sem fim de turno novo, nada sai — nem com outro prompt chegando.
        assert_eq!(sent(book.enqueue(s, false, "d".into())), None);
        assert_eq!(sent(book.signal(s, Signal::Working)), None);
        assert_eq!(
            sent(book.signal(s, Signal::TurnEnded)).as_deref(),
            Some("b")
        );
        assert_eq!(
            sent(book.signal(s, Signal::TurnEnded)).as_deref(),
            Some("c")
        );
        assert_eq!(
            sent(book.signal(s, Signal::TurnEnded)).as_deref(),
            Some("d")
        );
        assert_eq!(sent(book.signal(s, Signal::TurnEnded)), None);
    }

    #[test]
    fn agente_parado_recebe_o_primeiro_na_hora() {
        let book = QueueBook::default();
        let s = session();
        assert_eq!(
            sent(book.enqueue(s, true, "a".into())).as_deref(),
            Some("a")
        );
        // O segundo espera o turno do primeiro.
        assert_eq!(sent(book.enqueue(s, true, "b".into())), None);
    }

    #[test]
    fn idle_que_nao_e_fim_de_turno_nao_envia() {
        let book = QueueBook::default();
        let s = session();
        book.enqueue(s, false, "a".into());
        assert_eq!(sent(book.signal(s, Signal::Settled)), None);
        assert_eq!(texts(&book.state(s)), vec!["a"]);
    }

    #[test]
    fn enviar_marca_ocupado_ate_o_proximo_fim_de_turno() {
        let book = QueueBook::default();
        let s = session();
        book.enqueue(s, false, "a".into());
        book.enqueue(s, false, "b".into());
        assert_eq!(
            sent(book.signal(s, Signal::TurnEnded)).as_deref(),
            Some("a")
        );
        // Claude só com o plugin: não há `UserPromptSubmit`, o status fica
        // `Idle` e o resumo assenta (`Idle → Idle`, sem sinal). Retomar ou
        // enfileirar não pode furar.
        assert_eq!(sent(book.pause(s)), None);
        assert_eq!(sent(book.resume(s)), None);
        assert_eq!(sent(book.enqueue(s, true, "c".into())), None);
    }

    #[test]
    fn falha_pausa_e_retomar_envia() {
        let book = QueueBook::default();
        let s = session();
        book.enqueue(s, false, "a".into());
        book.signal(s, Signal::Failed);
        assert_eq!(sent(book.signal(s, Signal::Settled)), None);
        assert_eq!(book.state(s).paused, Some(PauseReason::Failed));
        assert_eq!(sent(book.signal(s, Signal::TurnEnded)), None);
        assert_eq!(sent(book.resume(s)).as_deref(), Some("a"));
        assert_eq!(book.state(s).paused, None);
    }

    #[test]
    fn interrupcao_pausa() {
        let book = QueueBook::default();
        let s = session();
        book.enqueue(s, false, "a".into());
        book.signal(s, Signal::Interrupted);
        book.signal(s, Signal::Settled);
        assert_eq!(book.state(s).paused, Some(PauseReason::Interrupted));
        assert_eq!(sent(book.signal(s, Signal::TurnEnded)), None);
    }

    #[test]
    fn aprovacao_pendente_pausa_e_segura_o_agente_ocupado() {
        let book = QueueBook::default();
        let s = session();
        book.enqueue(s, false, "a".into());
        book.signal(s, Signal::Approval);
        assert_eq!(book.state(s).paused, Some(PauseReason::Approval));
        // Retomar com o prompt de aprovação aberto não manda nada.
        assert_eq!(sent(book.resume(s)), None);
        assert_eq!(
            sent(book.signal(s, Signal::TurnEnded)).as_deref(),
            Some("a")
        );
    }

    #[test]
    fn agente_que_saiu_nao_recebe_nem_depois_de_idle() {
        let book = QueueBook::default();
        let s = session();
        book.enqueue(s, false, "a".into());
        book.signal(s, Signal::AgentEnded);
        // `SessionEnd` no prompt nativo desce o status a `Idle`.
        book.signal(s, Signal::Settled);
        assert_eq!(book.state(s).paused, Some(PauseReason::AgentEnded));
        assert_eq!(sent(book.resume(s)), None);
        // Um claude novo no pane fecha um turno: aí sim.
        assert_eq!(
            sent(book.signal(s, Signal::TurnEnded)).as_deref(),
            Some("a")
        );
    }

    #[test]
    fn sessao_morta_pausa() {
        let book = QueueBook::default();
        let s = session();
        book.enqueue(s, true, "a".into());
        book.enqueue(s, true, "b".into());
        book.signal(s, Signal::SessionEnded);
        assert_eq!(book.state(s).paused, Some(PauseReason::SessionEnded));
        assert_eq!(sent(book.resume(s)), None);
        assert_eq!(texts(&book.state(s)), vec!["b"]);
    }

    #[test]
    fn pausa_automatica_nao_vale_com_fila_vazia() {
        let book = QueueBook::default();
        let s = session();
        book.enqueue(s, true, "a".into());
        book.signal(s, Signal::Failed);
        assert_eq!(book.state(s).paused, None);
    }

    #[test]
    fn pausa_do_usuario_segura_o_fim_de_turno() {
        let book = QueueBook::default();
        let s = session();
        book.enqueue(s, false, "a".into());
        book.pause(s);
        assert_eq!(sent(book.signal(s, Signal::TurnEnded)), None);
        assert_eq!(book.state(s).paused, Some(PauseReason::User));
        assert_eq!(sent(book.resume(s)).as_deref(), Some("a"));
    }

    #[test]
    fn remover_e_reordenar() {
        let book = QueueBook::default();
        let s = session();
        for t in ["a", "b", "c"] {
            book.enqueue(s, false, t.into());
        }
        let ids: Vec<Uuid> = book.state(s).prompts.iter().map(|p| p.id).collect();
        let (changed, _) = book.remove(s, ids[1]);
        assert_eq!(texts(&changed.unwrap()), vec!["a", "c"]);
        assert_eq!(book.remove(s, ids[1]).0, None);

        let (changed, _) = book.reorder(s, &[ids[2], ids[0]]).unwrap();
        assert_eq!(texts(&changed.unwrap()), vec!["c", "a"]);
        // Lista velha (com o removido) ou incompleta é recusada.
        assert!(book.reorder(s, &ids).is_err());
        assert!(book.reorder(s, &[ids[0]]).is_err());
        assert_eq!(texts(&book.state(s)), vec!["c", "a"]);
        assert_eq!(
            sent(book.signal(s, Signal::TurnEnded)).as_deref(),
            Some("c")
        );
    }

    #[test]
    fn limpar_esvazia_e_tira_a_pausa() {
        let book = QueueBook::default();
        let s = session();
        book.enqueue(s, false, "a".into());
        book.signal(s, Signal::Failed);
        let (changed, _) = book.clear(s);
        let state = changed.unwrap();
        assert!(state.prompts.is_empty());
        assert_eq!(state.paused, None);
    }

    #[test]
    fn envio_que_falhou_volta_ao_topo_pausado() {
        let book = QueueBook::default();
        let s = session();
        book.enqueue(s, false, "a".into());
        book.enqueue(s, false, "b".into());
        let prompt = book.signal(s, Signal::TurnEnded).1.unwrap();
        book.send_failed(s, prompt);
        let state = book.state(s);
        assert_eq!(texts(&state), vec!["a", "b"]);
        assert_eq!(state.paused, Some(PauseReason::SendFailed));
    }

    #[test]
    fn sessao_sem_fila_ignora_sinais_e_esquecer_apaga() {
        let book = QueueBook::default();
        let s = session();
        assert_eq!(book.signal(s, Signal::TurnEnded), (None, None));
        assert_eq!(book.resume(s), (None, None));
        book.enqueue(s, false, "a".into());
        book.forget(s);
        assert!(book.state(s).prompts.is_empty());
    }

    #[test]
    fn status_vira_sinal_so_na_entrada_do_idle() {
        let idle = |s: &str| SessionStatus::Idle {
            summary: Some(s.into()),
        };
        let running = SessionStatus::Running;
        assert_eq!(
            signal_for_status(&running, &idle("x")),
            Some(Signal::Settled)
        );
        // Resumo que assenta depois do turno: não é sinal.
        assert_eq!(signal_for_status(&idle("x"), &idle("y")), None);
        assert_eq!(
            signal_for_status(&idle("x"), &running),
            Some(Signal::Working)
        );
        for reason in [AwaitingReason::Approval, AwaitingReason::NativePrompt] {
            let awaiting = SessionStatus::AwaitingInput { hint: None, reason };
            assert_eq!(
                signal_for_status(&running, &awaiting),
                Some(Signal::Approval)
            );
        }
        let reply = SessionStatus::AwaitingInput {
            hint: None,
            reason: AwaitingReason::Reply,
        };
        assert_eq!(signal_for_status(&idle("x"), &reply), None);
        assert_eq!(
            signal_for_status(&running, &SessionStatus::Exited { code: 0 }),
            Some(Signal::SessionEnded)
        );
        assert_eq!(
            signal_for_status(&running, &SessionStatus::Failed { reason: "x".into() }),
            Some(Signal::SessionEnded)
        );
    }
}
