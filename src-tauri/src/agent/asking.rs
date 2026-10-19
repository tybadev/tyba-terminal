//! Pedidos de permissão em aberto (agente livre, fatia 5).
//!
//! O agente pediu permissão no prompt dele; o app mostra um card "o Claude
//! está pedindo algo · projeto · branch" com "Ir" para a sessão que pede e
//! não está na tela. Quem guarda os pendentes é o core — um por sessão, na
//! ordem de chegada —, e o front só pinta a lista que recebe. Nada aqui
//! aprova coisa nenhuma: a resposta é no prompt do próprio agente.

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::session::{SessionId, SessionStatus};

/// A lista inteira, mais antigo primeiro, a cada mudança.
pub const EVENT_ASKS: &str = "agent://asks";

/// Os campos do card; o texto é montado no front, com i18n. Nada do payload
/// do hook: o agente, e projeto e branch lidos do cwd do processo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ask {
    pub session_id: SessionId,
    pub agent: String,
    pub project: Option<String>,
    pub branch: Option<String>,
}

#[derive(Default)]
pub struct AskBook {
    asks: Mutex<Vec<Ask>>,
}

impl AskBook {
    /// Um pendente por sessão: pedido novo da mesma sessão só troca o texto e
    /// mantém o lugar na fila. Devolve se a lista mudou.
    pub fn record(&self, ask: Ask) -> bool {
        let mut asks = self.asks.lock();
        match asks.iter_mut().find(|a| a.session_id == ask.session_id) {
            Some(old) if *old == ask => false,
            Some(old) => {
                *old = ask;
                true
            }
            None => {
                asks.push(ask);
                true
            }
        }
    }

    pub fn clear(&self, session_id: SessionId) -> bool {
        let mut asks = self.asks.lock();
        let before = asks.len();
        asks.retain(|a| a.session_id != session_id);
        asks.len() != before
    }

    pub fn list(&self) -> Vec<Ask> {
        self.asks.lock().clone()
    }
}

static BOOK: std::sync::OnceLock<AskBook> = std::sync::OnceLock::new();

pub fn book() -> &'static AskBook {
    BOOK.get_or_init(AskBook::default)
}

fn emit(app: &AppHandle) {
    let _ = app.emit(EVENT_ASKS, book().list());
}

/// O agente abriu um pedido de permissão.
pub fn record(app: &AppHandle, ask: Ask) {
    if book().record(ask) {
        emit(app);
    }
}

/// A sessão saiu do prompt nativo (ou o dono dispensou o card): o pedido
/// fecha.
pub fn clear(app: &AppHandle, session_id: SessionId) {
    if book().clear(session_id) {
        emit(app);
    }
}

/// Chamado a cada status novo: só o prompt nativo mantém o pedido aberto.
pub fn on_status(app: &AppHandle, session_id: SessionId, status: &SessionStatus) {
    if !crate::status::agent_events::is_native_prompt(status) {
        clear(app, session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(session_id: SessionId, branch: &str) -> Ask {
        Ask {
            session_id,
            agent: "Claude".into(),
            project: Some("api".into()),
            branch: Some(branch.into()),
        }
    }

    #[test]
    fn one_pending_per_session_in_arrival_order() {
        let book = AskBook::default();
        let a = SessionId::new_v4();
        let b = SessionId::new_v4();
        assert!(book.record(ask(a, "x")));
        assert!(book.record(ask(b, "y")));
        assert!(book.record(ask(a, "z")));
        assert!(!book.record(ask(a, "z")));
        assert_eq!(book.list(), vec![ask(a, "z"), ask(b, "y")]);
    }

    #[test]
    fn clearing_one_session_keeps_the_others() {
        let book = AskBook::default();
        let a = SessionId::new_v4();
        let b = SessionId::new_v4();
        book.record(ask(a, "x"));
        book.record(ask(b, "y"));
        assert!(book.clear(a));
        assert!(!book.clear(a));
        assert_eq!(book.list().len(), 1);
        assert_eq!(book.list()[0].session_id, b);
    }
}
