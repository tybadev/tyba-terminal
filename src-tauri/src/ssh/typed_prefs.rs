//! Configurações › SSH › Integração: o que o dono decidiu sobre o `ssh`
//! digitado — detectar ou não, o que fazer ao detectar, e os hosts negados
//! ("Nunca para este host").
//!
//! Mora no core (chave [`SETTING_KEY`] da tabela `settings`, fora do `pref.*`
//! para o webview não gravar direto): a tela só lê e pede mudança pelos
//! commands, e quem aplica é [`super::typed::DetectedEvent::build`].

use serde::{Deserialize, Serialize};

use crate::session::store::Store;

/// Chave em `settings`. Sem o prefixo `pref.`: o `set_pref` genérico não a
/// alcança, então a lista só muda pelos commands que validam a entrada.
pub const SETTING_KEY: &str = "ssh.typed_prefs";
/// Teto da lista de negados: o arquivo é do dono, mas o aviso é por bloco.
pub const MAX_DENIED: usize = 200;
const MAX_HOST_LEN: usize = 253;
const MAX_USER_LEN: usize = 64;

/// "Quando detectar", na ordem do quadro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TypedMode {
    /// Sempre perguntar: o aviso no bloco.
    #[default]
    Ask,
    /// Sempre abrir integrado, sem aviso (host cadastrado).
    Open,
    /// Nunca: nada aparece.
    Never,
}

/// Para onde o `ssh` foi, como o chip mostra e a lista de negados guarda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedTarget {
    pub user: Option<String>,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeniedHost {
    #[serde(flatten)]
    pub target: TypedTarget,
    /// Quando entrou na lista ("desde dd/mm").
    pub since_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedPrefs {
    #[serde(default = "default_true")]
    pub detect: bool,
    #[serde(default)]
    pub mode: TypedMode,
    #[serde(default)]
    pub denied: Vec<DeniedHost>,
}

fn default_true() -> bool {
    true
}

impl Default for TypedPrefs {
    fn default() -> Self {
        Self {
            detect: true,
            mode: TypedMode::Ask,
            denied: Vec::new(),
        }
    }
}

/// O que fazer com um `ssh` digitado que logou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Nada: detecção desligada, modo Nunca ou host negado.
    Quiet,
    /// O aviso no bloco.
    Ask,
    /// Abrir integrado direto, sem aviso.
    Open,
}

impl TypedTarget {
    /// O destino efetivo: usuário e porta digitados, senão os do Host
    /// cadastrado; o host é o hostname do cadastro (um alias digitado vira o
    /// endereço que ele materializa), senão o digitado.
    pub fn of(dest: &super::typed::TypedDestination, registered: Option<&super::Host>) -> Self {
        Self {
            user: dest
                .user
                .clone()
                .or_else(|| registered.and_then(|h| h.username.clone())),
            host: registered.map_or_else(|| dest.host.clone(), |h| h.hostname.clone()),
            port: dest
                .port
                .or_else(|| registered.and_then(|h| h.port))
                .unwrap_or(22),
        }
    }

    /// `user@host:porta` — a forma do quadro e do campo de adicionar.
    pub fn label(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        match &self.user {
            Some(u) => format!("{u}@{host}:{}", self.port),
            None => format!("{host}:{}", self.port),
        }
    }

    /// Lê `user@host:porta` (usuário e porta opcionais; IPv6 entre
    /// colchetes). Erro em pt-BR para a tela.
    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        let bad = || format!("destino inválido: use user@host:porta ({input})");
        if input.is_empty() || input.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(bad());
        }
        let (user, rest) = match input.rsplit_once('@') {
            Some((u, r)) => (Some(u), r),
            None => (None, input),
        };
        if let Some(u) = user {
            if u.is_empty() || u.len() > MAX_USER_LEN || u.contains('@') || u.contains('/') {
                return Err(bad());
            }
        }
        let (host, port) = if let Some(inner) = rest.strip_prefix('[') {
            let (addr, after) = inner.split_once(']').ok_or_else(bad)?;
            let port = match after {
                "" => None,
                p => Some(p.strip_prefix(':').ok_or_else(bad)?),
            };
            (addr, port)
        } else {
            match rest.matches(':').count() {
                0 => (rest, None),
                1 => {
                    let (h, p) = rest.split_once(':').ok_or_else(bad)?;
                    (h, Some(p))
                }
                // IPv6 sem colchetes: sem porta.
                _ => (rest, None),
            }
        };
        if host.is_empty()
            || host.len() > MAX_HOST_LEN
            || host.contains('/')
            || host.contains('[')
            || host.contains(']')
        {
            return Err(bad());
        }
        let port = match port {
            None => 22,
            Some(p) => match p.parse::<u16>() {
                Ok(n) if n > 0 => n,
                _ => return Err(bad()),
            },
        };
        Ok(Self {
            user: user.map(str::to_string),
            host: host.to_string(),
            port,
        })
    }

    /// A entrada negada cobre este destino: mesmo host (sem caixa) e porta, e
    /// o mesmo usuário — ou qualquer um, quando a entrada não diz.
    fn covered_by(&self, entry: &TypedTarget) -> bool {
        entry.host.eq_ignore_ascii_case(&self.host)
            && entry.port == self.port
            && entry
                .user
                .as_ref()
                .is_none_or(|u| Some(u) == self.user.as_ref())
    }
}

impl TypedPrefs {
    pub fn is_denied(&self, target: &TypedTarget) -> bool {
        self.denied.iter().any(|d| target.covered_by(&d.target))
    }

    /// A decisão do core. "Sempre abrir integrado" só abre direto o host
    /// cadastrado: o não cadastrado precisa do cadastro, que é do dono, e cai
    /// no aviso.
    pub fn decide(&self, target: &TypedTarget, registered: bool) -> Decision {
        if !self.detect || self.mode == TypedMode::Never || self.is_denied(target) {
            return Decision::Quiet;
        }
        match self.mode {
            TypedMode::Open if registered => Decision::Open,
            _ => Decision::Ask,
        }
    }

    /// Põe na lista (sem duplicar o mesmo destino). `false` quando já estava.
    pub fn deny(&mut self, target: TypedTarget, now_ms: i64) -> Result<bool, String> {
        if self.denied.iter().any(|d| {
            d.target.user == target.user
                && d.target.port == target.port
                && d.target.host.eq_ignore_ascii_case(&target.host)
        }) {
            return Ok(false);
        }
        if self.denied.len() >= MAX_DENIED {
            return Err(format!(
                "a lista de hosts negados tem no máximo {MAX_DENIED} hosts"
            ));
        }
        self.denied.push(DeniedHost {
            target,
            since_ms: now_ms,
        });
        Ok(true)
    }

    pub fn allow(&mut self, target: &TypedTarget) {
        self.denied.retain(|d| {
            !(d.target.user == target.user
                && d.target.port == target.port
                && d.target.host.eq_ignore_ascii_case(&target.host))
        });
    }
}

/// Lê do banco. Valor ausente ou quebrado vira o padrão (detectar, perguntar,
/// sem negados) — nunca derruba a detecção.
pub fn load(store: &Store) -> TypedPrefs {
    store
        .get_setting(SETTING_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save(store: &Store, prefs: &TypedPrefs) -> Result<(), String> {
    let raw = serde_json::to_string(prefs).map_err(|e| e.to_string())?;
    store
        .set_setting(SETTING_KEY, &raw)
        .map_err(|e| e.to_string())
}

/// Ler, mudar e gravar sob um lock: dois cliques seguidos não perdem um ao
/// outro.
pub fn update<T>(
    store: &Store,
    change: impl FnOnce(&mut TypedPrefs) -> Result<T, String>,
) -> Result<TypedPrefs, String> {
    static LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    let _guard = LOCK.lock();
    let mut prefs = load(store);
    change(&mut prefs)?;
    save(store, &prefs)?;
    Ok(prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(user: Option<&str>, host: &str, port: u16) -> TypedTarget {
        TypedTarget {
            user: user.map(str::to_string),
            host: host.into(),
            port,
        }
    }

    #[test]
    fn parses_the_user_host_port_form() {
        assert_eq!(
            TypedTarget::parse("root@203.0.113.10:22"),
            Ok(target(Some("root"), "203.0.113.10", 22))
        );
        assert_eq!(
            TypedTarget::parse(" deploy@10.0.4.12:2222 "),
            Ok(target(Some("deploy"), "10.0.4.12", 2222))
        );
        assert_eq!(
            TypedTarget::parse("pi@raspberrypi.local"),
            Ok(target(Some("pi"), "raspberrypi.local", 22))
        );
        assert_eq!(
            TypedTarget::parse("web.example.test"),
            Ok(target(None, "web.example.test", 22))
        );
        assert_eq!(
            TypedTarget::parse("me@[2001:db8::1]:2200"),
            Ok(target(Some("me"), "2001:db8::1", 2200))
        );
        assert_eq!(
            TypedTarget::parse("2001:db8::1"),
            Ok(target(None, "2001:db8::1", 22))
        );
        for bad in [
            "",
            "   ",
            "@host",
            "user@",
            "user@host:",
            "user@host:0",
            "user@host:99999",
            "user@host:abc",
            "user name@host",
            "user@ho st",
            "a/b@host",
            "user@host/x",
            "[::1",
            "[::1]x",
        ] {
            assert!(TypedTarget::parse(bad).is_err(), "aceitou {bad:?}");
        }
    }

    #[test]
    fn the_label_round_trips() {
        for t in [
            target(Some("root"), "203.0.113.10", 22),
            target(None, "web.example.test", 2222),
            target(Some("me"), "2001:db8::1", 2200),
        ] {
            assert_eq!(TypedTarget::parse(&t.label()), Ok(t.clone()));
        }
        assert_eq!(target(Some("root"), "h", 22).label(), "root@h:22");
    }

    #[test]
    fn the_target_takes_what_was_typed_then_the_registered_host() {
        let dest = |line: &str| crate::ssh::typed::parse(line).expect("interativo");
        let mut prod = crate::ssh::Host {
            id: "h".into(),
            alias: "prod".into(),
            hostname: "web.example.test".into(),
            port: Some(2222),
            username: Some("deploy".into()),
            identity_file: None,
            proxy_jump: None,
            group_id: None,
            color: None,
            notes: None,
            position: 0,
            tunnels: Vec::new(),
            auth_method: crate::ssh::AuthMethod::Auto,
            agent_key: None,
            integration_enabled: true,
            created_at: chrono::Utc::now(),
            last_connected_at: None,
        };
        assert_eq!(
            TypedTarget::of(&dest("ssh root@203.0.113.10"), None),
            target(Some("root"), "203.0.113.10", 22)
        );
        assert_eq!(
            TypedTarget::of(&dest("ssh -p 2 a@b"), None),
            target(Some("a"), "b", 2)
        );
        assert_eq!(
            TypedTarget::of(&dest("ssh prod"), Some(&prod)),
            target(Some("deploy"), "web.example.test", 2222)
        );
        prod.port = None;
        assert_eq!(
            TypedTarget::of(&dest("ssh other@prod"), Some(&prod)),
            target(Some("other"), "web.example.test", 22)
        );
    }

    #[test]
    fn the_decision_follows_the_settings() {
        let t = target(Some("root"), "203.0.113.10", 22);
        let mut prefs = TypedPrefs::default();
        assert_eq!(prefs.decide(&t, true), Decision::Ask, "padrão: perguntar");
        assert_eq!(prefs.decide(&t, false), Decision::Ask);

        prefs.mode = TypedMode::Open;
        assert_eq!(prefs.decide(&t, true), Decision::Open);
        assert_eq!(
            prefs.decide(&t, false),
            Decision::Ask,
            "não cadastrado precisa do cadastro"
        );

        prefs.mode = TypedMode::Never;
        assert_eq!(prefs.decide(&t, true), Decision::Quiet);
        assert_eq!(prefs.decide(&t, false), Decision::Quiet);

        prefs.mode = TypedMode::Ask;
        prefs.detect = false;
        assert_eq!(
            prefs.decide(&t, true),
            Decision::Quiet,
            "detecção desligada"
        );
        assert_eq!(prefs.decide(&t, false), Decision::Quiet);
    }

    #[test]
    fn a_denied_host_stays_quiet() {
        let mut prefs = TypedPrefs::default();
        prefs.mode = TypedMode::Open;
        assert_eq!(
            prefs.deny(target(Some("root"), "203.0.113.10", 22), 1),
            Ok(true)
        );
        assert_eq!(
            prefs.decide(&target(Some("root"), "203.0.113.10", 22), true),
            Decision::Quiet
        );
        assert_eq!(
            prefs.decide(&target(Some("root"), "203.0.113.10", 2222), false),
            Decision::Ask,
            "outra porta"
        );
        assert_eq!(
            prefs.decide(&target(Some("deploy"), "203.0.113.10", 22), false),
            Decision::Ask,
            "outro usuário"
        );
        assert_eq!(
            prefs.decide(&target(None, "203.0.113.10", 22), false),
            Decision::Ask,
            "usuário desconhecido não é o negado"
        );

        prefs.deny(target(None, "Web.Example.Test", 22), 2).unwrap();
        assert_eq!(
            prefs.decide(&target(Some("any"), "web.example.test", 22), false),
            Decision::Quiet,
            "entrada sem usuário cobre todos, sem caixa"
        );
    }

    #[test]
    fn deny_does_not_duplicate_and_allow_removes() {
        let mut prefs = TypedPrefs::default();
        let t = target(Some("root"), "h", 22);
        assert_eq!(prefs.deny(t.clone(), 1), Ok(true));
        assert_eq!(prefs.deny(target(Some("root"), "H", 22), 5), Ok(false));
        assert_eq!(prefs.denied.len(), 1);
        assert_eq!(prefs.denied[0].since_ms, 1);
        prefs.allow(&target(Some("root"), "H", 22));
        assert!(prefs.denied.is_empty());
        assert_eq!(prefs.decide(&t, false), Decision::Ask);
    }

    #[test]
    fn the_list_has_a_ceiling() {
        let mut prefs = TypedPrefs::default();
        for i in 0..MAX_DENIED {
            prefs.deny(target(None, &format!("h{i}"), 22), 0).unwrap();
        }
        assert!(prefs.deny(target(None, "one-more", 22), 0).is_err());
    }

    #[test]
    fn persists_in_the_store_and_survives_garbage() {
        let store = Store::open_in_memory().expect("store");
        assert_eq!(load(&store), TypedPrefs::default());

        let saved = update(&store, |p| {
            p.mode = TypedMode::Never;
            p.deny(target(Some("pi"), "raspberrypi.local", 22), 7)
        })
        .expect("grava");
        assert_eq!(load(&store), saved);
        assert_eq!(load(&store).mode, TypedMode::Never);

        store.set_setting(SETTING_KEY, "{quebrado").unwrap();
        assert_eq!(load(&store), TypedPrefs::default());
    }

    #[test]
    fn the_wire_format_is_what_the_screen_reads() {
        let mut prefs = TypedPrefs::default();
        prefs.deny(target(Some("root"), "h", 22), 9).unwrap();
        assert_eq!(
            serde_json::to_value(&prefs).unwrap(),
            serde_json::json!({
                "detect": true,
                "mode": "ask",
                "denied": [{ "user": "root", "host": "h", "port": 22, "since_ms": 9 }]
            })
        );
    }
}
