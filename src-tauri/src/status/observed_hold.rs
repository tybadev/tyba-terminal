//! Subagente trabalhando segura o palpite de tela em "trabalhando".
//!
//! Num shell (claude cru ou hospedado) o estado do agente vem da tela
//! ([`crate::status::observer`]). Com subagente em background, o TUI do Claude
//! volta ao prompt assim que o orquestrador encerra o turno — a tela diz
//! "ocioso" com trabalho em andamento (sintoma 3 do bug
//! `subagentes-background-async-quebram-captura`). O `SubagentTracker` sabe
//! que há subagente ativo: enquanto houver, o `Idle` da tela vira `Running`.
//!
//! Só o `Idle` é segurado. `AwaitingInput` passa como veio: um prompt nativo
//! aberto (inclusive o de uma ferramenta do próprio subagente) precisa de
//! alguém, e isso vence "trabalhando".
//!
//! Guarda o último palpite cru por sessão porque o fim do subagente não mexe
//! na tela: sem reavaliar no fim, o `Running` segurado ficaria preso até a
//! próxima mudança de tela.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::agent::subagents::SharedSubagents;
use crate::session::{ObservedAgent, ObservedState, SessionId};

/// O palpite foi segurado: a tela diz ocioso, mas há subagente trabalhando.
/// É o que vira `Session::subagents_working` num shell.
pub fn is_held(screen: &Option<ObservedAgent>, subagents_active: bool) -> bool {
    subagents_active
        && screen
            .as_ref()
            .is_some_and(|agent| agent.state == Some(ObservedState::Idle))
}

/// O que a sessão mostra, dado o palpite da tela e se há subagente ativo.
pub fn held(screen: Option<ObservedAgent>, subagents_active: bool) -> Option<ObservedAgent> {
    match screen {
        Some(agent) if subagents_active && agent.state == Some(ObservedState::Idle) => {
            Some(ObservedAgent {
                state: Some(ObservedState::Running),
                kind: None,
                ..agent
            })
        }
        other => other,
    }
}

pub struct ObservedHold {
    subagents: SharedSubagents,
    screen: Mutex<HashMap<SessionId, Option<ObservedAgent>>>,
}

impl ObservedHold {
    pub fn new(subagents: SharedSubagents) -> Self {
        Self {
            subagents,
            screen: Mutex::new(HashMap::new()),
        }
    }

    /// Palpite novo da tela. `apply` recebe o que a sessão deve mostrar (e se
    /// foi segurado por subagente) e roda
    /// sob o lock daqui: palpite e reavaliação não se cruzam (um `refresh` com o
    /// palpite velho não sobrescreve o novo).
    pub fn screen(
        &self,
        id: SessionId,
        screen: Option<ObservedAgent>,
        apply: impl FnOnce(Option<ObservedAgent>, bool),
    ) {
        let subagents_active = self.subagents.has_active(id);
        let mut map = self.screen.lock().expect("observed hold lock");
        map.insert(id, screen.clone());
        let was_held = is_held(&screen, subagents_active);
        apply(held(screen, subagents_active), was_held);
    }

    /// Reavalia com o último palpite — quando os subagentes começam ou acabam
    /// sem a tela mudar. Sem palpite ainda, não faz nada.
    pub fn refresh(&self, id: SessionId, apply: impl FnOnce(Option<ObservedAgent>, bool)) {
        let subagents_active = self.subagents.has_active(id);
        let map = self.screen.lock().expect("observed hold lock");
        if let Some(screen) = map.get(&id) {
            apply(
                held(screen.clone(), subagents_active),
                is_held(screen, subagents_active),
            );
        }
    }

    /// Esquece sessões que não estão mais vivas.
    pub fn retain(&self, live: &HashSet<SessionId>) {
        self.screen
            .lock()
            .expect("observed hold lock")
            .retain(|id, _| live.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::subagents::{RunEnd, SubagentTracker};
    use crate::session::ObservedWait;

    fn claude(state: Option<ObservedState>) -> Option<ObservedAgent> {
        Some(ObservedAgent {
            agent: "claude-code".into(),
            state,
            kind: None,
        })
    }

    #[test]
    fn idle_screen_with_active_subagents_is_working() {
        assert_eq!(
            held(claude(Some(ObservedState::Idle)), true),
            claude(Some(ObservedState::Running))
        );
        assert_eq!(
            held(claude(Some(ObservedState::Idle)), false),
            claude(Some(ObservedState::Idle))
        );
    }

    #[test]
    fn held_only_when_the_idle_screen_was_turned_into_working() {
        assert!(is_held(&claude(Some(ObservedState::Idle)), true));
        assert!(!is_held(&claude(Some(ObservedState::Idle)), false));
        assert!(!is_held(&claude(Some(ObservedState::Running)), true));
        assert!(!is_held(&claude(Some(ObservedState::AwaitingInput)), true));
        assert!(!is_held(&None, true));
    }

    #[test]
    fn waiting_for_someone_beats_working() {
        let prompt = Some(ObservedAgent {
            agent: "claude-code".into(),
            state: Some(ObservedState::AwaitingInput),
            kind: Some(ObservedWait::Permission),
        });
        assert_eq!(held(prompt.clone(), true), prompt);
    }

    #[test]
    fn no_agent_or_no_signal_is_never_invented() {
        assert_eq!(held(None, true), None);
        assert_eq!(held(claude(None), true), claude(None));
    }

    #[test]
    fn refresh_releases_the_hold_when_the_last_subagent_ends() {
        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let tracker: SharedSubagents = std::sync::Arc::new(SubagentTracker::new());
        let hold = ObservedHold::new(std::sync::Arc::clone(&tracker));
        let id = SessionId::new_v4();
        tracker.register_session(id);
        let agent = "a1b2c3d4e5f60718";
        tracker.on_subagent_started(&handle, id, agent.into(), "explorer".into(), None, None);

        let mut shown = Vec::new();
        hold.refresh(id, |o, h| shown.push((o, h)));
        assert!(shown.is_empty(), "sem palpite, nada a mostrar");

        hold.screen(id, claude(Some(ObservedState::Idle)), |o, h| {
            shown.push((o, h))
        });
        tracker.on_subagent_completed(&handle, id, agent, None, RunEnd::Completed);
        hold.refresh(id, |o, h| shown.push((o, h)));
        assert_eq!(
            shown,
            vec![
                (claude(Some(ObservedState::Running)), true),
                (claude(Some(ObservedState::Idle)), false)
            ]
        );

        hold.retain(&HashSet::new());
        hold.refresh(id, |_, _| panic!("sessão esquecida"));
    }
}
