//! Limite de requisições da API do GitHub, por host e por recurso (`core` é a
//! REST, `graphql` é a GraphQL — baldes separados no GitHub).
//!
//! O estado vem do que a forja já devolve: cabeçalhos `X-RateLimit-*` das
//! chamadas REST condicionais, o campo `rateLimit` da consulta GraphQL do
//! detalhe e, quando nada disso é recente, `GET /rate_limit` (que não conta
//! contra o limite). Com o balde baixo, a revalidação em segundo plano para até
//! o reset; o pedido explícito do usuário ainda passa, a não ser que o balde
//! esteja zerado.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;

/// Abaixo de 10% do limite, a revalidação automática para.
const LOW_FRACTION: u64 = 10;

/// Sem leitura há mais que isso, o estado não serve para decidir nada.
pub const STALE_AFTER: Duration = Duration::from_secs(300);

/// Bloqueio sem `reset` conhecido (limite secundário, mensagem sem cabeçalho).
const UNKNOWN_RESET_SECS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimit {
    pub resource: String,
    pub limit: u64,
    pub remaining: u64,
    /// Epoch em segundos.
    pub reset: i64,
}

impl RateLimit {
    pub fn low(&self) -> bool {
        self.limit > 0 && self.remaining * LOW_FRACTION < self.limit
    }
}

/// O que a UI mostra: "limite da API baixo, volta às HH:MM".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ForgeRateLimit {
    pub resource: String,
    pub limit: u64,
    pub remaining: u64,
    /// ISO 8601 (UTC).
    pub reset_at: String,
}

type State = HashMap<(String, String), (Instant, RateLimit)>;

fn state() -> &'static Mutex<State> {
    static STATE: std::sync::OnceLock<Mutex<State>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_epoch() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn record(host: &str, rate: RateLimit) {
    if rate.resource.is_empty() {
        return;
    }
    state().lock().insert(
        (host.to_ascii_lowercase(), rate.resource.clone()),
        (Instant::now(), rate),
    );
}

/// Bloqueio vindo de erro (mensagem de "rate limit exceeded" sem cabeçalho).
pub fn record_exhausted(host: &str, resource: &str) {
    record(
        host,
        RateLimit {
            resource: resource.into(),
            limit: 1,
            remaining: 0,
            reset: now_epoch() + UNKNOWN_RESET_SECS,
        },
    );
}

fn current(host: &str, resource: &str) -> Option<(Instant, RateLimit)> {
    state()
        .lock()
        .get(&(host.to_ascii_lowercase(), resource.to_string()))
        .cloned()
}

/// Leitura vencida (reset passou) não conta: o balde já encheu de novo.
fn live(rate: &RateLimit, now: i64) -> bool {
    rate.reset > now
}

/// A revalidação automática deve esperar? Só com leitura viva e balde baixo.
pub fn back_off(host: &str, resource: &str) -> bool {
    decide_back_off(
        current(host, resource).map(|(_, r)| r).as_ref(),
        now_epoch(),
    )
}

pub fn decide_back_off(rate: Option<&RateLimit>, now: i64) -> bool {
    rate.is_some_and(|r| live(r, now) && r.low())
}

/// Nem o pedido explícito passa: o balde está zerado até o reset.
pub fn exhausted(host: &str, resource: &str) -> Option<RateLimit> {
    current(host, resource)
        .map(|(_, r)| r)
        .filter(|r| live(r, now_epoch()) && r.remaining == 0)
}

/// O estado de `resource` é velho demais (ou não existe) para decidir.
pub fn unknown(host: &str, resource: &str) -> bool {
    current(host, resource)
        .is_none_or(|(at, r)| at.elapsed() > STALE_AFTER || !live(&r, now_epoch()))
}

fn to_ui(rate: &RateLimit) -> ForgeRateLimit {
    ForgeRateLimit {
        resource: rate.resource.clone(),
        limit: rate.limit,
        remaining: rate.remaining,
        reset_at: chrono::DateTime::from_timestamp(rate.reset, 0)
            .unwrap_or_default()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }
}

/// O balde baixo de `host` que a UI deve mostrar (o mais apertado), ou `None`.
pub fn low_for_ui(host: &str) -> Option<ForgeRateLimit> {
    let host = host.to_ascii_lowercase();
    let now = now_epoch();
    state()
        .lock()
        .iter()
        .filter(|((h, _), (_, r))| *h == host && live(r, now) && r.low())
        .map(|(_, (_, r))| r)
        .min_by_key(|r| r.remaining)
        .map(to_ui)
}

/// Cabeçalhos de uma resposta HTTP (nomes em qualquer caixa).
pub fn from_headers(headers: &[(String, String)]) -> Option<RateLimit> {
    let get = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim())
    };
    Some(RateLimit {
        limit: get("x-ratelimit-limit")?.parse().ok()?,
        remaining: get("x-ratelimit-remaining")?.parse().ok()?,
        reset: get("x-ratelimit-reset")?.parse().ok()?,
        resource: get("x-ratelimit-resource").unwrap_or("core").to_string(),
    })
}

/// Corpo de `GET /rate_limit`: um balde por recurso.
pub fn parse_rate_limit_body(body: &[u8]) -> Result<Vec<RateLimit>, String> {
    #[derive(serde::Deserialize)]
    struct Bucket {
        limit: u64,
        remaining: u64,
        reset: i64,
    }
    #[derive(serde::Deserialize)]
    struct Body {
        resources: HashMap<String, Bucket>,
    }
    let body: Body = serde_json::from_slice(body)
        .map_err(|e| format!("resposta inesperada de `gh api rate_limit`: {e}"))?;
    let mut out: Vec<RateLimit> = body
        .resources
        .into_iter()
        .map(|(resource, b)| RateLimit {
            resource,
            limit: b.limit,
            remaining: b.remaining,
            reset: b.reset,
        })
        .collect();
    out.sort_by(|a, b| a.resource.cmp(&b.resource));
    Ok(out)
}

/// Campo `rateLimit` da GraphQL (`resetAt` em ISO).
pub fn from_graphql(limit: u64, remaining: u64, reset_at: &str) -> Option<RateLimit> {
    let reset = chrono::DateTime::parse_from_rfc3339(reset_at)
        .ok()?
        .timestamp();
    Some(RateLimit {
        resource: "graphql".into(),
        limit,
        remaining,
        reset,
    })
}

/// A mensagem do `gh` diz que bateu no limite (primário ou secundário).
pub fn is_rate_limit_error(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    detail.contains("rate limit") || detail.contains("submitted too quickly")
}

#[cfg(test)]
pub(crate) fn reset_for_tests(host: &str) {
    let host = host.to_ascii_lowercase();
    state().lock().retain(|(h, _), _| *h != host);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn reads_rate_limit_headers_in_any_case() {
        let h = headers(&[
            ("X-Ratelimit-Limit", "5000"),
            ("x-ratelimit-remaining", "4321"),
            ("X-RATELIMIT-RESET", "1900000000"),
            ("X-Ratelimit-Resource", "core"),
        ]);
        assert_eq!(
            from_headers(&h),
            Some(RateLimit {
                resource: "core".into(),
                limit: 5000,
                remaining: 4321,
                reset: 1_900_000_000,
            })
        );
    }

    #[test]
    fn missing_or_garbage_headers_mean_no_reading() {
        assert_eq!(
            from_headers(&headers(&[("X-Ratelimit-Limit", "5000")])),
            None
        );
        let bad = headers(&[
            ("X-Ratelimit-Limit", "cinco mil"),
            ("X-Ratelimit-Remaining", "1"),
            ("X-Ratelimit-Reset", "1"),
        ]);
        assert_eq!(from_headers(&bad), None);
    }

    #[test]
    fn low_is_under_ten_percent() {
        let r = |remaining| RateLimit {
            resource: "core".into(),
            limit: 5000,
            remaining,
            reset: 0,
        };
        assert!(!r(500).low());
        assert!(r(499).low());
        assert!(r(0).low());
    }

    #[test]
    fn back_off_only_while_the_window_is_live() {
        let low = RateLimit {
            resource: "graphql".into(),
            limit: 5000,
            remaining: 10,
            reset: 2000,
        };
        assert!(decide_back_off(Some(&low), 1000));
        assert!(
            !decide_back_off(Some(&low), 2000),
            "o reset passou: balde cheio"
        );
        assert!(!decide_back_off(None, 1000));
        let fine = RateLimit {
            remaining: 4000,
            ..low
        };
        assert!(!decide_back_off(Some(&fine), 1000));
    }

    #[test]
    fn parses_the_rate_limit_endpoint() {
        let body =
            br#"{"resources":{"core":{"limit":5000,"used":10,"remaining":4990,"reset":1900000000},
            "graphql":{"limit":5000,"used":4800,"remaining":200,"reset":1900000100}},
            "rate":{"limit":5000,"used":10,"remaining":4990,"reset":1900000000}}"#;
        let buckets = parse_rate_limit_body(body).expect("parse");
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[1].resource, "graphql");
        assert_eq!(buckets[1].remaining, 200);
        assert!(buckets[1].low());
        assert!(parse_rate_limit_body(b"nada").is_err());
    }

    #[test]
    fn graphql_reset_at_is_iso() {
        let r = from_graphql(5000, 4999, "2030-03-17T17:46:40Z").expect("iso");
        assert_eq!(r.reset, 1_900_000_000);
        assert_eq!(r.resource, "graphql");
        assert_eq!(from_graphql(1, 1, "amanhã"), None);
    }

    #[test]
    fn recognizes_rate_limit_messages() {
        assert!(is_rate_limit_error(
            "HTTP 403: API rate limit exceeded for user ID 1."
        ));
        assert!(is_rate_limit_error(
            "You have exceeded a secondary rate limit. Please wait a few minutes"
        ));
        assert!(is_rate_limit_error("was submitted too quickly"));
        assert!(!is_rate_limit_error("HTTP 404: Not Found"));
    }

    #[test]
    fn ui_shows_the_tightest_low_bucket_of_the_host() {
        let host = "rate-test.example";
        reset_for_tests(host);
        let future = now_epoch() + 600;
        record(
            host,
            RateLimit {
                resource: "core".into(),
                limit: 5000,
                remaining: 4000,
                reset: future,
            },
        );
        assert_eq!(low_for_ui(host), None);
        record(
            host,
            RateLimit {
                resource: "graphql".into(),
                limit: 5000,
                remaining: 42,
                reset: future,
            },
        );
        let ui = low_for_ui(host).expect("baixo");
        assert_eq!(ui.resource, "graphql");
        assert_eq!(ui.remaining, 42);
        assert!(ui.reset_at.ends_with('Z'));
        assert!(back_off(host, "graphql"));
        assert!(!back_off(host, "core"));
        assert!(exhausted(host, "graphql").is_none());
        record_exhausted(host, "graphql");
        assert!(exhausted(host, "graphql").is_some());
        reset_for_tests(host);
    }
}
