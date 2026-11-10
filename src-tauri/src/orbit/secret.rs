//! Variável secreta do Orbit: o nome fica no grafo (`Graph::secrets`), o
//! valor fica no Keychain do macOS e só ali.
//!
//! Nunca no SQLite ([`strip`] roda dentro de `Store::save_orbit_graph`), nunca
//! no arquivo exportado (o `export_toml` também passa por [`strip`]) e nunca no
//! webview: o grafo que o front recebe é o do banco, e o valor só sai por
//! [`SecretStore::get`] no Revelar, sob demanda. O valor carregado vira padrão
//! literal da redação ([`crate::session::redact::set_literal`]), para não ir
//! a log nem scrollback se aparecer na saída.
//!
//! Só macOS por enquanto: no Linux e no Windows [`SUPPORTED`] é `false`, o
//! cadeado fica desligado com aviso e nenhum valor é guardado em outro lugar.

use super::Graph;

/// O Keychain existe neste sistema.
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// Onde o valor secreto vive. O de produção é o [`Keychain`]; os testes usam
/// um em memória.
pub trait SecretStore: Send + Sync {
    fn get(&self, graph_id: &str, name: &str) -> Result<Option<String>, String>;
    fn set(&self, graph_id: &str, name: &str, value: &str) -> Result<(), String>;
    /// Apagar o que não existe não é erro.
    fn delete(&self, graph_id: &str, name: &str) -> Result<(), String>;
}

/// Chave da redação e conta do Keychain: JSON de `[grafo, nome]`, sem
/// ambiguidade com `:` ou `/` dentro do id ou do nome.
fn account(graph_id: &str, name: &str) -> String {
    serde_json::to_string(&[graph_id, name]).expect("par de strings serializa")
}

/// Zera o valor das variáveis secretas: o que vai ao banco e ao arquivo.
pub fn strip(graph: &mut Graph) {
    for name in &graph.secrets {
        if let Some(value) = graph.variables.get_mut(name) {
            value.clear();
        }
    }
}

/// O valor de cada secreta do grafo, do cofre, para o run (só em memória).
/// Secreta sem valor no cofre fica vazia: o ▶ recusa como variável sem valor.
pub fn fill(graph: &mut Graph, store: &dyn SecretStore) -> Result<(), String> {
    let names: Vec<String> = graph.secrets.iter().cloned().collect();
    for name in names {
        let Some(slot) = graph.variables.get_mut(&name) else {
            continue;
        };
        let value = store.get(&graph.id, &name)?.unwrap_or_default();
        remember(&graph.id, &name, &value);
        *slot = value;
    }
    Ok(())
}

/// Guarda o valor e passa a redigi-lo.
pub fn set(store: &dyn SecretStore, graph_id: &str, name: &str, value: &str) -> Result<(), String> {
    store.set(graph_id, name, value)?;
    remember(graph_id, name, value);
    Ok(())
}

/// O valor para o Revelar. Não guarda nada além do que [`fill`] guardaria.
pub fn reveal(
    store: &dyn SecretStore,
    graph_id: &str,
    name: &str,
) -> Result<Option<String>, String> {
    let value = store.get(graph_id, name)?;
    if let Some(v) = &value {
        remember(graph_id, name, v);
    }
    Ok(value)
}

/// Apaga do cofre as secretas de `before` que não são mais secretas em
/// `after` (destrancadas, removidas ou o grafo inteiro apagado com `None`).
pub fn forget_removed(store: &dyn SecretStore, before: &Graph, after: Option<&Graph>) {
    for name in &before.secrets {
        let kept =
            after.is_some_and(|g| g.secrets.contains(name) && g.variables.contains_key(name));
        if !kept {
            if let Err(e) = store.delete(&before.id, name) {
                eprintln!("tyba: apagar a variável secreta {name} do Keychain: {e}");
            }
            crate::session::redact::forget_literal(&account(&before.id, name));
        }
    }
}

fn remember(graph_id: &str, name: &str, value: &str) {
    crate::session::redact::set_literal(&account(graph_id, name), value);
}

/// O Keychain de login do macOS (item genérico, serviço [`SERVICE`]).
pub struct Keychain;

pub const SERVICE: &str = "dev.tyba.app.orbit-variables";

#[cfg(target_os = "macos")]
impl SecretStore for Keychain {
    fn get(&self, graph_id: &str, name: &str) -> Result<Option<String>, String> {
        use security_framework::passwords::get_generic_password;
        match get_generic_password(SERVICE, &account(graph_id, name)) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| "valor do Keychain não é texto".to_string()),
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(format!("Keychain: {e}")),
        }
    }

    fn set(&self, graph_id: &str, name: &str, value: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(
            SERVICE,
            &account(graph_id, name),
            value.as_bytes(),
        )
        .map_err(|e| format!("Keychain: {e}"))
    }

    fn delete(&self, graph_id: &str, name: &str) -> Result<(), String> {
        match security_framework::passwords::delete_generic_password(
            SERVICE,
            &account(graph_id, name),
        ) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(e) => Err(format!("Keychain: {e}")),
        }
    }
}

#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

/// Sem Keychain: recusa guardar e não acha nada. Nenhum valor vai a outro
/// lugar.
#[cfg(not(target_os = "macos"))]
impl SecretStore for Keychain {
    fn get(&self, _graph_id: &str, _name: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn set(&self, _graph_id: &str, _name: &str, _value: &str) -> Result<(), String> {
        Err("variável secreta só no macOS por enquanto (sem Keychain neste sistema)".into())
    }

    fn delete(&self, _graph_id: &str, _name: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    /// Cofre em memória dos testes: o Keychain de verdade nunca é tocado.
    #[derive(Default)]
    pub(crate) struct MemoryStore(pub Mutex<HashMap<(String, String), String>>);

    impl SecretStore for MemoryStore {
        fn get(&self, graph_id: &str, name: &str) -> Result<Option<String>, String> {
            Ok(self
                .0
                .lock()
                .get(&(graph_id.to_string(), name.to_string()))
                .cloned())
        }
        fn set(&self, graph_id: &str, name: &str, value: &str) -> Result<(), String> {
            self.0
                .lock()
                .insert((graph_id.to_string(), name.to_string()), value.to_string());
            Ok(())
        }
        fn delete(&self, graph_id: &str, name: &str) -> Result<(), String> {
            self.0
                .lock()
                .remove(&(graph_id.to_string(), name.to_string()));
            Ok(())
        }
    }

    fn graph() -> Graph {
        let mut g: Graph = serde_json::from_value(serde_json::json!({
            "id": "g-secret", "name": "g", "dir": "/tmp"
        }))
        .unwrap();
        g.variables
            .insert("BASE_URL".into(), "http://localhost:3000".into());
        g.variables.insert("TOKEN".into(), String::new());
        g.secrets.insert("TOKEN".into());
        g
    }

    // Fixture sintética, não é token de ninguém.
    const FAKE: &str = "fixture-secret-7Qk2Lm9Xw4";

    #[test]
    fn fill_brings_the_value_only_in_memory_and_strip_clears_it() {
        let store = MemoryStore::default();
        let mut g = graph();
        set(&store, &g.id, "TOKEN", FAKE).unwrap();
        fill(&mut g, &store).unwrap();
        assert_eq!(g.variables["TOKEN"], FAKE);
        strip(&mut g);
        assert_eq!(g.variables["TOKEN"], "");
        assert_eq!(g.variables["BASE_URL"], "http://localhost:3000");
    }

    #[test]
    fn unlocked_or_removed_secret_leaves_the_store() {
        let store = MemoryStore::default();
        let before = graph();
        set(&store, &before.id, "TOKEN", FAKE).unwrap();
        let mut after = before.clone();
        forget_removed(&store, &before, Some(&after));
        assert!(
            store.get(&before.id, "TOKEN").unwrap().is_some(),
            "segue secreta"
        );
        after.secrets.clear();
        forget_removed(&store, &before, Some(&after));
        assert!(store.get(&before.id, "TOKEN").unwrap().is_none());

        set(&store, &before.id, "TOKEN", FAKE).unwrap();
        forget_removed(&store, &before, None);
        assert!(
            store.get(&before.id, "TOKEN").unwrap().is_none(),
            "grafo apagado"
        );
    }

    /// Revelar devolve o valor e não grava nada: nem no cofre, nem no banco.
    #[test]
    fn reveal_returns_the_value_without_persisting_it() {
        let store = MemoryStore::default();
        let db = crate::session::store::Store::open_in_memory().unwrap();
        let g = graph();
        set(&store, &g.id, "TOKEN", FAKE).unwrap();
        db.save_orbit_graph(&g, true, None).unwrap();
        let vault = store.0.lock().clone();
        assert_eq!(
            reveal(&store, &g.id, "TOKEN").unwrap().as_deref(),
            Some(FAKE)
        );
        assert_eq!(*store.0.lock(), vault);
        let saved = db.get_orbit_graph(&g.id).unwrap().unwrap().value;
        assert_eq!(saved, g, "o banco segue sem valor");
        assert_eq!(reveal(&store, &g.id, "OUTRA").unwrap(), None);
    }

    #[test]
    fn account_is_unambiguous() {
        assert_ne!(account("a:b", "c"), account("a", "b:c"));
    }
}
