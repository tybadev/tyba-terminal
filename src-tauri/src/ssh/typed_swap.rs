//! A troca no mesmo pane (quadros 1A′–1D de `SshTrocaNoPane`): o `ssh`
//! digitado num shell local vira a sessão integrada NO LUGAR do shell — mesmo
//! pane, mesma posição no split — e o shell é encerrado com o process group
//! inteiro, o `ssh` junto. Um terminal só.
//!
//! Quem decide é o core: o front só diz "troque esta sessão", e a troca só
//! acontece se a sessão é um shell e foi ela que detectou o `ssh` para o Host
//! que vai abrir.

use std::collections::HashMap;

use parking_lot::Mutex;
use serde::Serialize;

use super::typed::{resolve, TypedDestination, TypedMatch};
use crate::error::AppError;
use crate::layout::{LayoutError, LayoutManager, PaneId};
use crate::session::{SessionId, SessionKind};

/// O `ssh` digitado que a sessão de shell avisou por último.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub destination: TypedDestination,
    /// Veio do "Sempre abrir integrado" (troca direta, com toast).
    pub open: bool,
}

/// O último aviso de cada shell: é o que autoriza a troca.
#[derive(Default)]
pub struct SeenBook(Mutex<HashMap<SessionId, Seen>>);

impl SeenBook {
    pub fn record(&self, id: SessionId, seen: Seen) {
        self.0.lock().insert(id, seen);
    }

    pub fn get(&self, id: SessionId) -> Option<Seen> {
        self.0.lock().get(&id).cloned()
    }

    pub fn forget(&self, id: SessionId) {
        self.0.lock().remove(&id);
    }
}

/// A marca da troca no topo do pane integrado: "⇄ substituiu o shell local ·
/// ssh encerrado · HH:MM" (ou "aberto integrado", com `auto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Replaced {
    pub at_ms: i64,
    pub auto: bool,
}

/// Pode trocar `kind` (a sessão a substituir; `None` = id desconhecido) pelo
/// Host `host`? Devolve se a troca é a automática.
pub fn check(
    kind: Option<&SessionKind>,
    seen: Option<&Seen>,
    host: &super::Host,
) -> Result<bool, AppError> {
    match kind {
        None => return Err(AppError::new("ssh.replace_unknown_session")),
        Some(SessionKind::Shell) => {}
        Some(_) => return Err(AppError::new("ssh.replace_not_shell")),
    }
    let Some(seen) = seen else {
        return Err(AppError::new("ssh.replace_not_detected"));
    };
    match resolve(seen.destination.clone(), std::slice::from_ref(host)) {
        Some(TypedMatch::Registered(id)) if id == host.id => Ok(seen.open),
        _ => Err(AppError::new("ssh.replace_not_detected")),
    }
}

/// `new` assume o pane de `old` e só então `old` é encerrada (`dispose`, o
/// caminho do `killpg`). Se o layout recusa, nada é encerrado.
pub fn swap(
    layout: &LayoutManager,
    old: SessionId,
    new: SessionId,
    dispose: impl FnOnce(SessionId),
) -> Result<PaneId, LayoutError> {
    let pane = layout.replace_session(old, new)?;
    dispose(old);
    Ok(pane)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::SplitKind;
    use crate::session::store::Store;
    use std::sync::Arc;

    fn host(id: &str, alias: &str, hostname: &str, user: Option<&str>) -> super::super::Host {
        super::super::Host {
            id: id.into(),
            alias: alias.into(),
            hostname: hostname.into(),
            port: None,
            username: user.map(Into::into),
            identity_file: None,
            proxy_jump: None,
            group_id: None,
            color: None,
            notes: None,
            position: 0,
            tunnels: Vec::new(),
            auth_method: super::super::AuthMethod::Auto,
            agent_key: None,
            integration_enabled: true,
            created_at: chrono::Utc::now(),
            last_connected_at: None,
        }
    }

    fn seen(line: &str, open: bool) -> Seen {
        Seen {
            destination: super::super::typed::parse(line).expect("linha interativa"),
            open,
        }
    }

    #[test]
    fn only_the_shell_that_saw_the_ssh_can_be_replaced() {
        let srv = host("h1", "srv-01", "203.0.113.10", Some("root"));
        let s = seen("ssh root@203.0.113.10", false);

        assert!(!check(Some(&SessionKind::Shell), Some(&s), &srv).unwrap());
        assert!(
            check(
                Some(&SessionKind::Shell),
                Some(&seen("ssh srv-01", true)),
                &srv
            )
            .unwrap(),
            "o modo automático vai junto"
        );

        let code = |r: Result<bool, AppError>| r.unwrap_err().code;
        assert_eq!(
            code(check(None, Some(&s), &srv)),
            "ssh.replace_unknown_session"
        );
        assert_eq!(
            code(check(
                Some(&SessionKind::Ssh {
                    host_id: "h1".into()
                }),
                Some(&s),
                &srv
            )),
            "ssh.replace_not_shell"
        );
        assert_eq!(
            code(check(Some(&SessionKind::Shell), None, &srv)),
            "ssh.replace_not_detected"
        );
        assert_eq!(
            code(check(
                Some(&SessionKind::Shell),
                Some(&seen("ssh root@198.51.100.7", false)),
                &srv
            )),
            "ssh.replace_not_detected",
            "o ssh desta sessão é para outro host"
        );
    }

    /// "Cadastrar e abrir integrado": o Host nasce do destino detectado, e
    /// a troca tem de casar com ele.
    #[test]
    fn a_host_registered_from_the_detection_matches_it() {
        let s = seen("ssh -p 2222 deploy@203.0.113.10", false);
        let mut fresh = host("h2", "203.0.113.10", "203.0.113.10", Some("deploy"));
        fresh.port = Some(2222);
        assert!(!check(Some(&SessionKind::Shell), Some(&s), &fresh).unwrap());
    }

    #[test]
    fn swap_disposes_the_old_session_only_after_the_pane_changed_hands() {
        let layout = LayoutManager::new(Arc::new(Store::open_in_memory().unwrap()));
        let left = uuid::Uuid::new_v4();
        layout.create_workspace("dev", None, left).unwrap();
        let left_pane = layout.state().workspaces[0].tabs[0].active_pane.unwrap();
        let shell = uuid::Uuid::new_v4();
        let shell_pane = layout.split_pane(left_pane, SplitKind::V, shell).unwrap();
        let integrated = uuid::Uuid::new_v4();

        let mut disposed = Vec::new();
        let pane = swap(&layout, shell, integrated, |old| {
            assert_eq!(
                layout.pane_of_session(old),
                None,
                "o shell já saiu do pane quando é encerrado"
            );
            disposed.push(old);
        })
        .unwrap();

        assert_eq!(pane, shell_pane);
        assert_eq!(disposed, vec![shell]);
        assert_eq!(layout.pane_of_session(integrated), Some(shell_pane));

        let mut none = Vec::new();
        assert!(
            swap(&layout, uuid::Uuid::new_v4(), uuid::Uuid::new_v4(), |old| {
                none.push(old)
            })
            .is_err()
        );
        assert!(none.is_empty(), "troca recusada não encerra nada");
    }
}
