//! O que o laço emissor faz com o palpite de tela, sem o laço.
//!
//! O laço do PTY não é alcançável por teste unitário — ele precisa de um
//! `AppHandle` e de um processo vivo. Então tudo o que ele **decide** mora aqui:
//! quando recortar, quando o recorte fica devendo, e o que acontece quando o
//! PTY morre. Lá ficam cinco chamadas e nenhuma regra.
//!
//! O ritmo tem duas batidas, e a segunda é a que quase não se vê:
//!
//! - **flush** (16 ms): o recorte sai junto com o quadro que vai para a tela. É
//!   o orçamento acordado — 1 ms por sessão por flush —, e recortar por chunk
//!   pagaria o mesmo trabalho dezenas de vezes por quadro.
//! - **assentamento**: a última tela de uma rajada não é a de um flush. O chunk
//!   que desenha "Do you want to proceed?" chega no meio da janela, e depois
//!   dele o agente **para de escrever** — que é o que esperar significa. Sem
//!   uma segunda batida no fim da rajada, esse estado só seria visto quando o
//!   agente voltasse a escrever, e ele não vai.

use crate::status::observer::ScreenObserver;
use crate::status::screen::{self, ScreenSnapshot, MAX_REGION_LINES};

use super::ScreenState;

pub(super) struct ScreenPipe {
    observer: ScreenObserver,
    /// Houve chunk depois do último recorte: o assentamento está armado.
    pending: bool,
}

impl ScreenPipe {
    pub(super) fn new(observer: ScreenObserver) -> Self {
        Self {
            observer,
            pending: false,
        }
    }

    /// O recorte de um chunk. Chamado **sob** o lock de tela.
    ///
    /// `None` não é "nada aconteceu": quando não é hora do flush, arma o
    /// assentamento. Registro vazio (a v1, antes dos manifestos) não arma nada —
    /// senão toda sessão trocaria o `recv` bloqueante do laço por um despertar a
    /// cada 16 ms para não fazer nada.
    pub(super) fn cut(&mut self, state: &ScreenState, due: bool) -> Option<ScreenSnapshot> {
        if !self.observer.wants_snapshot() {
            return None;
        }
        if !due {
            self.pending = true;
            return None;
        }
        self.pending = false;
        Some(cut_screen(state))
    }

    /// O recorte que ficou devendo, no fim da rajada. Também sob o lock.
    pub(super) fn cut_pending(&mut self, state: &ScreenState) -> Option<ScreenSnapshot> {
        if !self.pending {
            return None;
        }
        self.pending = false;
        Some(cut_screen(state))
    }

    /// O laço precisa acordar para assentar?
    pub(super) fn wants_settle(&self) -> bool {
        self.pending
    }

    /// A avaliação do manifesto. Chamada **fora** do lock — ver o laço.
    pub(super) fn feed(&mut self, snapshot: &ScreenSnapshot) {
        self.observer.observe(snapshot);
    }

    /// O PTY morreu: o que estava na tela virou passado, e um palpite
    /// sobrevivente afirmaria um agente que não existe mais.
    pub(super) fn finish(&mut self) {
        self.observer.clear();
    }
}

/// A única coisa que o palpite faz dentro do lock.
///
/// `MAX_REGION_LINES` e não o que o manifesto pede: um recorte só serve todas as
/// regras da sessão. A região de cada regra é aparada de novo na avaliação, e
/// pedir aqui mais linhas do que alguém olha só custa hash.
fn cut_screen(state: &ScreenState) -> ScreenSnapshot {
    screen::snapshot(state.parser.screen(), MAX_REGION_LINES)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;

    use super::*;
    use crate::session::SessionKind;
    use crate::status::registry::ManifestRegistry;

    const CODEX: &str = r#"
id = "codex"
match = { title = ["Codex"] }

[[rules]]
id = "working"
state = "running"
region = { bottom_lines = 3 }
contains = ["esc to interrupt"]
"#;

    type Visto = Arc<Mutex<Vec<String>>>;

    fn pipe(sources: &[&str]) -> (ScreenPipe, Visto) {
        let visto: Visto = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&visto);
        let observer = ScreenObserver::for_session(
            &SessionKind::Shell,
            Arc::new(ManifestRegistry::from_sources(sources)),
            Box::new(|| None),
            Box::new(move |observed| {
                sink.lock().push(match observed {
                    Some(agent) => format!("{}:{:?}", agent.agent, agent.state),
                    None => "limpo".into(),
                })
            }),
        )
        .expect("shell recebe observador");
        (ScreenPipe::new(observer), visto)
    }

    fn tela() -> ScreenState {
        let mut state = ScreenState::new(24, 80);
        state.parser.process(
            b"\x1b]0;Codex\x07\xe2\x80\xa2 Working (2s \xe2\x80\xa2 esc to interrupt)\r\n",
        );
        state
    }

    /// A prova do assentamento: chunk fora da hora do flush não some, fica
    /// devendo. Sem isto, a última tela da rajada — a que pede aprovação — só
    /// seria vista se o agente voltasse a escrever.
    #[test]
    fn chunk_fora_do_flush_arma_o_assentamento() {
        let (mut pipe, _) = pipe(&[CODEX]);
        let state = tela();

        assert!(pipe.cut(&state, false).is_none());
        assert!(pipe.wants_settle(), "a rajada não deixou nada pendente");

        assert!(pipe.cut_pending(&state).is_some());
        assert!(!pipe.wants_settle());
        assert!(
            pipe.cut_pending(&state).is_none(),
            "o assentamento recortou duas vezes"
        );
    }

    #[test]
    fn o_recorte_do_flush_desarma_o_assentamento() {
        let (mut pipe, _) = pipe(&[CODEX]);
        let state = tela();

        pipe.cut(&state, false);
        assert!(pipe.cut(&state, true).is_some());

        assert!(
            !pipe.wants_settle(),
            "o laço vai acordar de graça depois de já ter observado"
        );
    }

    /// Registro vazio não custa nem o recorte nem o despertar.
    #[test]
    fn sem_manifesto_nao_recorta_nem_arma() {
        let (mut pipe, _) = pipe(&[]);
        let state = tela();

        assert!(pipe.cut(&state, true).is_none());
        assert!(pipe.cut(&state, false).is_none());
        assert!(!pipe.wants_settle());
    }

    #[test]
    fn o_recorte_alimenta_a_avaliacao() {
        let (mut pipe, visto) = pipe(&[CODEX]);
        let state = tela();

        let snapshot = pipe.cut(&state, true).unwrap();
        pipe.feed(&snapshot);

        assert_eq!(visto.lock().as_slice(), ["codex:Some(Running)"]);
    }

    #[test]
    fn o_fim_do_pty_limpa_o_palpite() {
        let (mut pipe, visto) = pipe(&[CODEX]);
        let state = tela();
        let snapshot = pipe.cut(&state, true).unwrap();
        pipe.feed(&snapshot);

        pipe.finish();

        assert_eq!(visto.lock().last().unwrap(), "limpo");
    }
}
