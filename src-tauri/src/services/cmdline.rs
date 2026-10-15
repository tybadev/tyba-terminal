//! A linha de comando que a tela mostra: redigida antes de sair do core.
//!
//! Token na linha de comando é comum (`--token=...`, `PGPASSWORD=...`,
//! `postgres://user:senha@host`). Além dos padrões de segredo do
//! `session::redact`, o valor de qualquer opção ou atribuição com nome de
//! segredo vira `[REDACTED]`. Redigir demais é o erro aceitável aqui.

use std::sync::LazyLock;

use regex::Regex;

use crate::session::redact::{redact, REDACTION_MARK};

/// Teto do que vai para a tela: o resto não ajuda a reconhecer o processo.
pub const MAX_COMMAND_CHARS: usize = 400;

/// `--api-key=x`, `--password x`, `--requirepass x`, `GITHUB_TOKEN=x`,
/// `-Dsecret=x`, `Authorization: Bearer x` (o esquema fica, o valor some).
static SECRET_OPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)((?:^|\s)-{0,2}[\w.-]*(?:token|secret|pass(?:w(?:or)?d)?|pwd|api[-_]?key|access[-_]?key|private[-_]?key|auth|credential|session[-_]?id|cookie)[\w.-]*)(=|:\s*|\s+)((?:bearer|basic|token)\s+)?(\S+)",
    )
    .unwrap()
});

/// `-phunter2` (mysql e afins): opção curta com o valor colado. Redige
/// também `-p8080`, e tudo bem.
static GLUED_PASSWORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"((?:^|\s)-p)(\S+)").unwrap());

/// `Bearer x` solto (num header passado como argumento).
static BEARER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b((?:bearer|basic)\s+)([^\s\[]\S*)").unwrap());

/// Caractere que não se vê mas mexe no que se vê: controle (Cc) e as marcas
/// de direção (bidi), que reordenam o texto na tela.
pub fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// `esquema://usuário:senha@` → a senha some, o usuário fica.
static URL_CREDENTIALS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z][A-Za-z0-9+.-]*://[^/\s:@]*):[^@\s/]+@").unwrap());

pub fn redact_command(args: &str) -> String {
    let cleaned: String = args
        .chars()
        .filter_map(|c| match c {
            c if c.is_control() => Some(' '),
            c if is_hidden_char(c) => None,
            c => Some(c),
        })
        .collect();
    let base = redact(&cleaned);
    let urls = URL_CREDENTIALS.replace_all(&base, format!("$1:{REDACTION_MARK}@"));
    let options = SECRET_OPTION.replace_all(&urls, format!("$1$2$3{REDACTION_MARK}"));
    let glued = GLUED_PASSWORD.replace_all(&options, format!("$1{REDACTION_MARK}"));
    let options = BEARER.replace_all(&glued, format!("$1{REDACTION_MARK}"));
    let mut out: String = options.chars().take(MAX_COMMAND_CHARS).collect();
    if options.chars().count() > MAX_COMMAND_CHARS {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_command_is_untouched() {
        assert_eq!(
            redact_command("node dist/api.js --port 8080"),
            "node dist/api.js --port 8080"
        );
    }

    #[test]
    fn option_values_with_secret_names_are_redacted() {
        assert_eq!(
            redact_command("server --token=abc123 --port 3000"),
            "server --token=[REDACTED] --port 3000"
        );
        assert_eq!(
            redact_command("cli --password hunter2 run"),
            "cli --password [REDACTED] run"
        );
        assert_eq!(
            redact_command("app --api-key=k1 --API_KEY k2"),
            "app --api-key=[REDACTED] --API_KEY [REDACTED]"
        );
        assert_eq!(
            redact_command("java -Ddb.secret=xyz -jar a.jar"),
            "java -Ddb.secret=[REDACTED] -jar a.jar"
        );
    }

    #[test]
    fn env_assignments_with_secret_names_are_redacted() {
        assert_eq!(
            redact_command("env PGPASSWORD=s3nha GITHUB_TOKEN=x psql"),
            "env PGPASSWORD=[REDACTED] GITHUB_TOKEN=[REDACTED] psql"
        );
    }

    #[test]
    fn dev_server_passwords_are_redacted() {
        assert_eq!(
            redact_command("redis-server --port 6379 --requirepass hunter2"),
            "redis-server --port 6379 --requirepass [REDACTED]"
        );
        assert_eq!(
            redact_command("mysql -uroot -phunter2"),
            "mysql -uroot -p[REDACTED]"
        );
        assert_eq!(
            redact_command("node server.js --pass hunter2"),
            "node server.js --pass [REDACTED]"
        );
        assert_eq!(
            redact_command("redis-server --masterauth s3"),
            "redis-server --masterauth [REDACTED]"
        );
    }

    #[test]
    fn authorization_headers_keep_the_scheme_only() {
        assert_eq!(
            redact_command("curl -H Authorization: Bearer 9f8a7c6b5a4d3e2f1a0b x"),
            "curl -H Authorization: Bearer [REDACTED] x"
        );
        assert_eq!(
            redact_command("curl -H Authorization:abc123"),
            "curl -H Authorization:[REDACTED]"
        );
        assert_eq!(
            redact_command("curl -H 'X: Bearer abc123'"),
            "curl -H 'X: Bearer [REDACTED]"
        );
    }

    #[test]
    fn bidi_marks_are_dropped() {
        assert_eq!(redact_command("node a\u{202e}b.js"), "node ab.js");
        assert!(is_hidden_char('\u{2066}'));
        assert!(!is_hidden_char('é'));
    }

    #[test]
    fn url_credentials_lose_the_password_only() {
        assert_eq!(
            redact_command("worker postgres://app:s3nha@db:5432/x"),
            "worker postgres://app:[REDACTED]@db:5432/x"
        );
        // Sem senha, nada a esconder.
        assert_eq!(
            redact_command("open https://example.com/a:b"),
            "open https://example.com/a:b"
        );
    }

    #[test]
    fn known_secret_shapes_are_redacted_anywhere() {
        let fake = format!("sk-{}", "a".repeat(24));
        let out = redact_command(&format!("bot {fake} run"));
        assert!(!out.contains(&fake));
        assert!(out.contains(REDACTION_MARK));
    }

    #[test]
    fn control_characters_and_length_are_bounded() {
        assert_eq!(redact_command("a\u{1b}[31mb\nc"), "a [31mb c");
        let long = "x".repeat(MAX_COMMAND_CHARS + 50);
        let out = redact_command(&long);
        assert_eq!(out.chars().count(), MAX_COMMAND_CHARS + 1);
        assert!(out.ends_with('…'));
    }
}
