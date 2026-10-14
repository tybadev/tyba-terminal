//! GET condicional na REST do GitHub (`ETag` / `If-None-Match`) pelo `gh api -i`.
//!
//! Resposta `304 Not Modified` não conta contra o limite primário da API: o
//! poll da CI (15s com o painel aberto) só gasta quando algo mudou. O `gh`
//! sai com código 1 no 304 e imprime "gh: HTTP 304" no stderr, então a decisão
//! é tomada pela linha de status do stdout, não pelo código de saída.
//!
//! Escolhido no lugar de `gh api --cache`: o `--cache` é um TTL em disco, que
//! devolve o dado velho sem perguntar ao servidor, e não economiza nada quando
//! o TTL vence. O ETag pergunta sempre e só paga quando mudou.

use std::collections::HashMap;
use std::path::Path;

use parking_lot::Mutex;

use super::exec::{self, NETWORK_TIMEOUT};
use super::github::{self, cli_failed};
use super::rate;
use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Saída do `gh api -i`: linha de status, cabeçalhos, linha vazia, corpo.
/// Aceita `\r\n` ou `\n`. `None` se não começa com `HTTP/`.
pub fn parse_include(stdout: &[u8]) -> Option<HttpResponse> {
    if !stdout.starts_with(b"HTTP/") {
        return None;
    }
    let (head_end, body_start) = match find(stdout, b"\r\n\r\n") {
        Some(i) => (i, i + 4),
        None => match find(stdout, b"\n\n") {
            Some(i) => (i, i + 2),
            None => (stdout.len(), stdout.len()),
        },
    };
    let head = String::from_utf8_lossy(&stdout[..head_end]);
    let mut lines = head.lines().map(|l| l.trim_end_matches('\r'));
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let headers = lines
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect();
    Some(HttpResponse {
        status,
        headers,
        body: stdout[body_start..].to_vec(),
    })
}

/// O ETag vai para argv (`-H If-None-Match: …`): só ASCII visível, curto.
pub fn safe_etag(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value.chars().all(|c| c.is_ascii_graphic() || c == ' ')
}

type Store = HashMap<(String, String), (String, Vec<u8>)>;

fn store() -> &'static Mutex<Store> {
    static STORE: std::sync::OnceLock<Mutex<Store>> = std::sync::OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// argv do GET condicional. `endpoint` é montado por quem chama, com segmentos
/// já validados (`safe_segment`).
pub fn args(host: &str, endpoint: &str, etag: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = ["api", "-i", "--hostname", host].map(String::from).to_vec();
    if let Some(etag) = etag.filter(|e| safe_etag(e)) {
        args.extend(["-H".into(), format!("If-None-Match: {etag}")]);
    }
    args.push(endpoint.into());
    args
}

/// Decide o resultado de uma resposta: corpo novo (guarda o ETag), corpo
/// guardado (304) ou erro. Atualiza o estado de limite com os cabeçalhos.
pub fn settle(
    host: &str,
    endpoint: &str,
    response: Option<HttpResponse>,
    stderr: &str,
) -> Result<Vec<u8>, AppError> {
    let key = (host.to_ascii_lowercase(), endpoint.to_string());
    let Some(response) = response else {
        if rate::is_rate_limit_error(stderr) {
            rate::record_exhausted(host, "core");
            return Err(AppError::new("forge.rate_limited"));
        }
        return Err(cli_failed(stderr.to_string()));
    };
    if let Some(r) = rate::from_headers(&response.headers) {
        rate::record(host, r);
    }
    match response.status {
        304 => store()
            .lock()
            .get(&key)
            .map(|(_, body)| body.clone())
            // 304 sem corpo guardado não deveria acontecer (só mandamos ETag
            // que guardamos); se acontecer, é falha, não lista vazia.
            .ok_or_else(|| cli_failed("HTTP 304 sem resposta guardada")),
        200..=299 => {
            if let Some(etag) = response.header("etag").filter(|e| safe_etag(e)) {
                store()
                    .lock()
                    .insert(key, (etag.to_string(), response.body.clone()));
            }
            Ok(response.body)
        }
        403 | 429
            if response.header("x-ratelimit-remaining") == Some("0")
                || rate::is_rate_limit_error(&String::from_utf8_lossy(&response.body)) =>
        {
            if rate::from_headers(&response.headers).is_none() {
                rate::record_exhausted(host, "core");
            }
            Err(AppError::new("forge.rate_limited"))
        }
        _ => Err(cli_failed(if stderr.trim().is_empty() {
            format!("HTTP {}", response.status)
        } else {
            stderr.trim().to_string()
        })),
    }
}

/// GET condicional. Devolve o corpo (novo ou o guardado, num 304).
pub fn get(repo_root: &Path, host: &str, endpoint: &str) -> Result<Vec<u8>, AppError> {
    let etag = store()
        .lock()
        .get(&(host.to_ascii_lowercase(), endpoint.to_string()))
        .map(|(etag, _)| etag.clone());
    let args = args(host, endpoint, etag.as_deref());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out =
        exec::run(github::CLI, &args, repo_root, None, NETWORK_TIMEOUT).map_err(cli_failed)?;
    let stderr = out.stderr_message();
    settle(host, endpoint, parse_include(&out.stdout), &stderr)
}

/// Atualiza o estado de limite por `GET /rate_limit`, que não conta contra o
/// limite. Erro aqui é silencioso: sem leitura, nada muda.
pub fn refresh_rate_limit(repo_root: &Path, host: &str) {
    let Ok(out) = exec::run(
        github::CLI,
        &["api", "--hostname", host, "rate_limit"],
        repo_root,
        None,
        NETWORK_TIMEOUT,
    ) else {
        return;
    };
    if !out.ok {
        return;
    }
    if let Ok(buckets) = rate::parse_rate_limit_body(&out.stdout) {
        for b in buckets {
            rate::record(host, b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Respostas sintéticas no formato do `gh api -i` (status, cabeçalhos com
    // `\r\n`, linha vazia, corpo).
    const OK: &[u8] = b"HTTP/2.0 200 OK\r\nContent-Type: application/json\r\nEtag: W/\"abc123\"\r\nX-Ratelimit-Limit: 5000\r\nX-Ratelimit-Remaining: 4990\r\nX-Ratelimit-Reset: 4102444800\r\nX-Ratelimit-Resource: core\r\n\r\n{\"total_count\":0,\"workflow_runs\":[]}";
    const NOT_MODIFIED: &[u8] = b"HTTP/2.0 304 Not Modified\r\nEtag: \"abc123\"\r\nX-Ratelimit-Limit: 5000\r\nX-Ratelimit-Remaining: 4990\r\nX-Ratelimit-Reset: 4102444800\r\nX-Ratelimit-Resource: core\r\n\r\n";
    const LIMITED: &[u8] = b"HTTP/2.0 403 Forbidden\r\nX-Ratelimit-Limit: 5000\r\nX-Ratelimit-Remaining: 0\r\nX-Ratelimit-Reset: 4102444800\r\nX-Ratelimit-Resource: core\r\n\r\n{\"message\":\"API rate limit exceeded for user ID 1.\"}";

    #[test]
    fn parses_status_headers_and_body() {
        let r = parse_include(OK).expect("http");
        assert_eq!(r.status, 200);
        assert_eq!(r.header("ETag"), Some("W/\"abc123\""));
        assert_eq!(r.header("x-ratelimit-remaining"), Some("4990"));
        assert_eq!(r.body, b"{\"total_count\":0,\"workflow_runs\":[]}");
    }

    #[test]
    fn parses_a_304_with_no_body_and_bare_newlines() {
        let r = parse_include(NOT_MODIFIED).expect("http");
        assert_eq!(r.status, 304);
        assert!(r.body.is_empty());
        let lf = parse_include(b"HTTP/1.1 200 OK\nEtag: \"x\"\n\n[]").expect("lf");
        assert_eq!(lf.header("etag"), Some("\"x\""));
        assert_eq!(lf.body, b"[]");
    }

    #[test]
    fn non_http_output_is_not_a_response() {
        assert_eq!(parse_include(b"gh: Not Found (HTTP 404)"), None);
        assert_eq!(parse_include(b""), None);
        assert_eq!(parse_include(b"HTTP/2.0 abc\r\n\r\n"), None);
    }

    #[test]
    fn etag_goes_to_argv_only_when_safe() {
        let a = args("github.com", "repos/o/r/actions/runs", Some("W/\"abc\""));
        assert_eq!(
            a,
            [
                "api",
                "-i",
                "--hostname",
                "github.com",
                "-H",
                "If-None-Match: W/\"abc\"",
                "repos/o/r/actions/runs"
            ]
        );
        let evil = args("github.com", "x", Some("\"a\"\r\nAuthorization: x"));
        assert!(
            !evil.iter().any(|a| a.contains("If-None-Match")),
            "{evil:?}"
        );
        assert!(!args("h", "x", None).contains(&"-H".to_string()));
    }

    #[test]
    fn a_304_serves_the_stored_body_and_records_the_rate() {
        let host = "cond-test.example";
        rate::reset_for_tests(host);
        let endpoint = "repos/o/r/actions/runs?per_page=10";
        let first = settle(host, endpoint, parse_include(OK), "").expect("200");
        assert_eq!(first, b"{\"total_count\":0,\"workflow_runs\":[]}");
        let second =
            settle(host, endpoint, parse_include(NOT_MODIFIED), "gh: HTTP 304").expect("304");
        assert_eq!(second, first);
        assert!(!rate::unknown(host, "core"));
        rate::reset_for_tests(host);
    }

    #[test]
    fn a_304_without_a_stored_body_is_an_error() {
        let err = settle(
            "cond-empty.example",
            "nunca",
            parse_include(NOT_MODIFIED),
            "",
        )
        .expect_err("sem corpo");
        assert_eq!(err.code, "forge.cli_failed");
    }

    #[test]
    fn rate_limit_response_is_a_typed_error() {
        let host = "cond-limited.example";
        rate::reset_for_tests(host);
        let err = settle(host, "x", parse_include(LIMITED), "gh: HTTP 403").expect_err("403");
        assert_eq!(err.code, "forge.rate_limited");
        assert!(rate::exhausted(host, "core").is_some());
        rate::reset_for_tests(host);
    }

    #[test]
    fn other_errors_keep_the_cli_message() {
        let err = settle("h", "x", None, "gh: Not Found (HTTP 404)").expect_err("404");
        assert_eq!(err.code, "forge.cli_failed");
    }
}
