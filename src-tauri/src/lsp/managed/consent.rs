use std::sync::Arc;

use crate::session::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Accept,
    UseMine,
    Refuse,
}

pub struct Consent {
    store: Arc<Store>,
}

impl Consent {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    pub fn is_granted(&self, server_id: &str) -> bool {
        self.store.lsp_managed_consent(server_id).unwrap_or(false)
    }

    pub fn record(
        &self,
        server_id: &str,
        version: &str,
        decision: Decision,
    ) -> Result<(), String> {
        match decision {
            Decision::Accept => self
                .store
                .set_lsp_managed_consent(server_id, version)
                .map_err(|e| e.to_string()),
            Decision::UseMine | Decision::Refuse => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn consent() -> Consent {
        Consent::new(Arc::new(Store::open_in_memory().unwrap()))
    }

    #[test]
    fn accept_persists() {
        let c = consent();
        assert!(!c.is_granted("pyright"));
        c.record("pyright", "1.1.411", Decision::Accept).unwrap();
        assert!(c.is_granted("pyright"));
    }

    #[test]
    fn refuse_is_never_persisted_as_final() {
        let c = consent();
        c.record("pyright", "1.1.411", Decision::Refuse).unwrap();
        assert!(
            !c.is_granted("pyright"),
            "recusa não vira decisão final: o card volta a ser oferecido"
        );
    }

    #[test]
    fn use_mine_does_not_persist_consent() {
        let c = consent();
        c.record("pyright", "1.1.411", Decision::UseMine).unwrap();
        assert!(!c.is_granted("pyright"));
    }
}
