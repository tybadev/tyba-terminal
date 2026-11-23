//! Quem espera o fim de um comando: o `tyba run --wait`.
//!
//! A CLI registra o pane ANTES de digitar o comando; a thread de leitura do
//! PTY, quando fecha um bloco (`133;D`), entrega o primeiro bloco daquele pane
//! e tira o registro. Pane que morre antes cancela: o canal cai e quem espera
//! fica sabendo. Sem ninguém esperando, `offer` não copia nada.

use std::collections::HashMap;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{LazyLock, Mutex};

/// O bloco que fechou, ainda cru: a redação acontece do lado da CLI, antes de
/// qualquer byte sair do app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub exit_code: Option<i32>,
    pub bytes: Vec<u8>,
    pub cols: u16,
    pub rows: u16,
}

static WAITERS: LazyLock<Mutex<HashMap<String, SyncSender<Done>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Passa a esperar o próximo bloco de `session`. Um registro por pane: o
/// novo substitui o antigo (cujo canal cai).
pub fn register(session: &str) -> Receiver<Done> {
    let (tx, rx) = sync_channel(1);
    if let Ok(mut map) = WAITERS.lock() {
        map.insert(session.to_string(), tx);
    }
    rx
}

/// O pane morreu ou a CLI desistiu: quem esperava recebe `Disconnected`.
pub fn cancel(session: &str) {
    if let Ok(mut map) = WAITERS.lock() {
        map.remove(session);
    }
}

/// Chamado a cada bloco fechado. `done` só roda (e copia os bytes) quando há
/// alguém esperando aquele pane.
pub fn offer(session: &str, done: impl FnOnce() -> Done) {
    let waiter = WAITERS.lock().ok().and_then(|mut map| map.remove(session));
    if let Some(tx) = waiter {
        let _ = tx.try_send(done());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(code: i32) -> Done {
        Done {
            exit_code: Some(code),
            bytes: b"oi".to_vec(),
            cols: 80,
            rows: 24,
        }
    }

    #[test]
    fn the_first_block_goes_to_the_waiter_and_the_next_one_to_nobody() {
        let rx = register("w-first");
        offer("w-first", || done(3));
        assert_eq!(rx.recv().unwrap().exit_code, Some(3));
        let mut copied = false;
        offer("w-first", || {
            copied = true;
            done(4)
        });
        assert!(!copied, "sem ninguém esperando, nada é copiado");
    }

    #[test]
    fn cancel_wakes_the_waiter_with_a_disconnect() {
        let rx = register("w-cancel");
        cancel("w-cancel");
        assert!(rx.recv().is_err());
    }

    #[test]
    fn another_pane_does_not_answer() {
        let rx = register("w-a");
        offer("w-b", || done(0));
        assert!(rx.try_recv().is_err());
        cancel("w-a");
    }
}
