//! Um "Conectar" por Host de cada vez.
//!
//! Clique repetido no card (ou duplo clique, ou Enter segurado) chegava ao core
//! como N pedidos e virava N SSH Sessions. O portão guarda, por Host, o pedido
//! em voo: enquanto ele não terminou, um segundo pedido não abre nada.
//!
//! "Em voo" tem duas fases:
//! - `Starting`: entre a entrada do pedido e a SSH Session existir. Quem sai
//!   dessa fase é o [`Ticket`] — por [`Ticket::opened`] ou pelo `Drop` (erro,
//!   pânico, cancelamento), então não há como a marca ficar presa.
//! - `Opened(id)`: a sessão existe e o Cano ainda está em `connecting`. Essa
//!   fase não precisa ser limpa por ninguém: quem decide se ela ainda vale é o
//!   predicado `still_connecting`, consultado a cada pedido. Sessão que logou,
//!   falhou ou foi fechada deixa de contar sozinha.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::session::SessionId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Starting,
    Opened(SessionId),
}

/// Por que o pedido não seguiu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InFlight {
    /// A sessão do pedido anterior ainda não existe.
    Starting,
    /// A sessão do pedido anterior existe e ainda está conectando.
    Session(SessionId),
}

#[derive(Default)]
pub struct ConnectGate {
    slots: Mutex<HashMap<String, Slot>>,
}

pub type SharedConnectGate = Arc<ConnectGate>;

impl ConnectGate {
    /// Entra se não houver conexão em voo para `host_id`.
    pub fn begin(
        self: &Arc<Self>,
        host_id: &str,
        still_connecting: impl Fn(SessionId) -> bool,
    ) -> Result<Ticket, InFlight> {
        let mut slots = self.slots.lock();
        match slots.get(host_id) {
            Some(Slot::Starting) => return Err(InFlight::Starting),
            Some(Slot::Opened(id)) if still_connecting(*id) => return Err(InFlight::Session(*id)),
            _ => {}
        }
        slots.insert(host_id.to_string(), Slot::Starting);
        Ok(Ticket {
            gate: Arc::clone(self),
            host_id: host_id.to_string(),
            done: false,
        })
    }
}

/// A vez de um pedido. Solto sem [`Ticket::opened`], libera o Host.
pub struct Ticket {
    gate: Arc<ConnectGate>,
    host_id: String,
    done: bool,
}

impl Ticket {
    /// A sessão nasceu: o Host segue em voo enquanto ela estiver conectando.
    pub fn opened(mut self, id: SessionId) {
        self.gate
            .slots
            .lock()
            .insert(self.host_id.clone(), Slot::Opened(id));
        self.done = true;
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        if !self.done {
            self.gate.slots.lock().remove(&self.host_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn gate() -> Arc<ConnectGate> {
        Arc::new(ConnectGate::default())
    }

    #[test]
    fn second_request_while_starting_is_refused() {
        let g = gate();
        let _t = g.begin("h1", |_| true).unwrap();
        assert_eq!(g.begin("h1", |_| true).err(), Some(InFlight::Starting));
    }

    #[test]
    fn other_hosts_are_independent() {
        let g = gate();
        let _t = g.begin("h1", |_| true).unwrap();
        assert!(g.begin("h2", |_| true).is_ok());
    }

    #[test]
    fn second_request_while_connecting_returns_the_open_session() {
        let g = gate();
        let id = Uuid::new_v4();
        g.begin("h1", |_| true).unwrap().opened(id);
        assert_eq!(g.begin("h1", |_| true).err(), Some(InFlight::Session(id)));
    }

    #[test]
    fn session_that_left_connecting_frees_the_host() {
        // Logou, falhou ou foi fechada: o predicado diz não, o Host volta a aceitar.
        let g = gate();
        g.begin("h1", |_| true).unwrap().opened(Uuid::new_v4());
        assert!(g.begin("h1", |_| false).is_ok());
    }

    #[test]
    fn dropped_ticket_frees_the_host() {
        // Erro antes da sessão existir: o `?` solta o ticket e nada fica preso.
        let g = gate();
        drop(g.begin("h1", |_| true).unwrap());
        assert!(g.begin("h1", |_| true).is_ok());
    }

    #[test]
    fn many_concurrent_requests_admit_exactly_one() {
        let g = gate();
        let admitted = std::thread::scope(|s| {
            let handles: Vec<_> = (0..16)
                .map(|_| {
                    let g = Arc::clone(&g);
                    s.spawn(move || {
                        g.begin("h1", |_| true)
                            .map(|t| t.opened(Uuid::new_v4()))
                            .is_ok()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .filter(|ok| *ok)
                .count()
        });
        assert_eq!(admitted, 1);
    }
}
