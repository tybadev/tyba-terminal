use std::borrow::Cow;
use std::sync::LazyLock;

use regex::{NoExpand, Regex};

pub const REDACTION_MARK: &str = "[REDACTED]";

/// Bloco PEM vem primeiro: fechado, some do BEGIN ao END (com quebra real ou
/// `\n` escapado dentro de JSON); sem END, some do BEGIN ao fim do texto —
/// texto cortado no meio da chave ainda esconde o corpo.
static SECRETS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        -----BEGIN\ [A-Z\ ]*PRIVATE\ KEY-----(?s:.*?-----END\ [A-Z\ ]*PRIVATE\ KEY-----|.*)
        | (AKIA|ASIA)[0-9A-Z]{16}
        | sk-[A-Za-z0-9_-]{20,}
        | gh[pousr]_[A-Za-z0-9]{20,}
        | github_pat_[A-Za-z0-9_]{20,}
        | glpat-[A-Za-z0-9_-]{20,}
        | xox[abposr]-[A-Za-z0-9-]{10,}
        | AIza[0-9A-Za-z_-]{35}
        | eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+
        ",
    )
    .unwrap()
});

/// `scheme://user:SENHA@host`: o grupo 1 fica, só a senha vira marca. A senha
/// vai até o ÚLTIMO `@` antes de `/`, `?`, `#`, aspas ou espaço — host nunca
/// tem `@`, senha crua às vezes tem.
static URL_PASSWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"([A-Za-z][A-Za-z0-9+.-]*://[^\s:/@]*:)[^\s/?#"'<>]+@"#).unwrap()
});

static PEM_BEGIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----").unwrap());
static PEM_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-----END [A-Z ]*PRIVATE KEY-----").unwrap());

pub fn redact(input: &str) -> Cow<'_, str> {
    let password = |c: &regex::Captures| format!("{}{REDACTION_MARK}@", &c[1]);
    match SECRETS.replace_all(input, NoExpand(REDACTION_MARK)) {
        Cow::Borrowed(s) => URL_PASSWORD.replace_all(s, password),
        Cow::Owned(s) => Cow::Owned(URL_PASSWORD.replace_all(&s, password).into_owned()),
    }
}

/// Mesmos padrões do [`redact`], como pergunta: o texto carrega um segredo?
pub fn contains_secret(input: &str) -> bool {
    SECRETS.is_match(input) || URL_PASSWORD.is_match(input)
}

/// [`redact`] linha a linha, lembrando do bloco PEM aberto.
///
/// Quem redige por linha lógica (blocos) recebe o BEGIN, o corpo base64 e o
/// END separados. O `redact` sozinho esconderia só a linha do BEGIN, e o corpo
/// — que É a chave — passaria. Aqui toda linha entre BEGIN e END vira marca.
/// Base64 solto fora de um bloco não é tocado: hash e dump dariam falso
/// positivo demais.
///
/// Como no `redact`, `Cow::Owned` quer dizer que algo foi redigido.
#[derive(Default)]
pub struct LineRedactor {
    in_pem: bool,
}

impl LineRedactor {
    pub fn line<'a>(&mut self, line: &'a str) -> Cow<'a, str> {
        if self.in_pem {
            let Some(end) = PEM_END.find(line) else {
                return Cow::Owned(REDACTION_MARK.to_string());
            };
            self.in_pem = false;
            let rest = self.line(&line[end.end()..]);
            return Cow::Owned(format!("{REDACTION_MARK}{rest}"));
        }
        let out = redact(line);
        // BEGIN sempre redige, então `Borrowed` dispensa a busca.
        if matches!(out, Cow::Owned(_)) {
            self.in_pem = PEM_BEGIN
                .find_iter(line)
                .last()
                .is_some_and(|b| !PEM_END.is_match(&line[b.end()..]));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_forge_tokens() {
        for token in [
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "gho_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
            "glpat-abcdefghij1234567890xy",
        ] {
            let line = format!("remote https://x:{token}@host/r");
            let out = redact(&line);
            assert!(!out.contains(token), "{out}");
        }
    }

    #[test]
    fn passes_clean_text_through_without_allocating() {
        let input = "just some normal terminal output, nothing secret here";
        let out = redact(input);
        assert_eq!(out, input);
        assert!(matches!(out, Cow::Borrowed(_)));
    }

    #[test]
    fn redacts_aws_access_key_id() {
        let out = redact("export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE next");
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(out.contains(REDACTION_MARK));
    }

    #[test]
    fn redacts_openai_style_key() {
        let key = "sk-abcdef1234567890ABCDEFghijkl";
        let line = format!("token: {key} loaded");
        let out = redact(&line);
        assert!(!out.contains(key));
        assert!(out.contains(REDACTION_MARK));
    }

    #[test]
    fn redacts_jwt() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        let line = format!("Authorization: Bearer {jwt}");
        let out = redact(&line);
        assert!(!out.contains(jwt));
        assert!(out.contains(REDACTION_MARK));
    }

    #[test]
    fn redacts_every_secret_on_a_line() {
        let line = "k1=AKIAIOSFODNN7EXAMPLE k2=sk-abcdef1234567890ABCDEFghijkl end";
        let out = redact(line);
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!out.contains("sk-abcdef1234567890ABCDEFghijkl"));
        assert_eq!(out.matches(REDACTION_MARK).count(), 2);
    }

    /// Segredo sintético montado em runtime: o literal inteiro nunca aparece no
    /// fonte (repo público, scanner de segredo do GitHub).
    fn fake(prefix: &str, body: &str, n: usize) -> String {
        format!("{prefix}{}", body.repeat(n))
    }

    #[test]
    fn redacts_only_the_password_of_a_url_credential() {
        for (url, want) in [
            (
                "postgres://app:senha@db:5432/x",
                "postgres://app:[REDACTED]@db:5432/x",
            ),
            (
                "mysql://root:pw123@127.0.0.1/db",
                "mysql://root:[REDACTED]@127.0.0.1/db",
            ),
            (
                "redis://:senha@cache:6379",
                "redis://:[REDACTED]@cache:6379",
            ),
            (
                "mongodb+srv://u:p@cluster0.example.net/app",
                "mongodb+srv://u:[REDACTED]@cluster0.example.net/app",
            ),
            (
                "https://user:token@host/r",
                "https://user:[REDACTED]@host/r",
            ),
            ("postgres://u:p@ss@db/x", "postgres://u:[REDACTED]@db/x"),
        ] {
            let line = format!("DATABASE_URL={url} ok");
            assert_eq!(redact(&line), format!("DATABASE_URL={want} ok"));
        }
    }

    #[test]
    fn leaves_urls_without_password_alone() {
        for line in [
            "postgres://db:5432/x",
            "https://user@host/r",
            "http://localhost:8080/a?next=b@c",
            "git@github.com:org/repo.git",
            "see https://host:443/x and mail me@host",
        ] {
            assert!(matches!(redact(line), Cow::Borrowed(_)), "{line}");
            assert!(!contains_secret(line), "{line}");
        }
    }

    #[test]
    fn redacts_slack_token() {
        let token = fake("xoxb-", "0", 24);
        let line = format!("SLACK={token} ok");
        let out = redact(&line);
        assert_eq!(out, "SLACK=[REDACTED] ok");
        // Vizinhos: prefixo fora da família e corpo curto demais.
        assert!(!contains_secret(&fake("xoxz-", "0", 24)));
        assert!(!contains_secret("xoxb-curto"));
    }

    #[test]
    fn redacts_github_fine_grained_token() {
        let token = fake("github_pat_", "A1_", 10);
        let line = format!("GH={token} ok");
        let out = redact(&line);
        assert_eq!(out, "GH=[REDACTED] ok");
        assert!(!contains_secret("github_pat_curto"));
    }

    #[test]
    fn redacts_google_api_key() {
        let key = fake("AIza", "a", 35);
        let line = format!("KEY={key} ok");
        let out = redact(&line);
        assert_eq!(out, "KEY=[REDACTED] ok");
        // 34 caracteres não são uma chave do Google.
        assert!(!contains_secret(&format!("{} ok", fake("AIza", "a", 34))));
    }

    fn pem_lines() -> Vec<String> {
        vec![
            format!("-----BEGIN {} KEY-----", "RSA PRIVATE"),
            fake("", "Zm9vYmFy", 8),
            fake("", "YmF6cXV4", 8),
            "Zm9vYmFyYmF6".to_string(),
            format!("-----END {} KEY-----", "RSA PRIVATE"),
        ]
    }

    #[test]
    fn redacts_a_whole_pem_block_in_one_text() {
        let text = format!("antes\n{}\ndepois", pem_lines().join("\n"));
        let out = redact(&text);
        assert_eq!(out, "antes\n[REDACTED]\ndepois");
    }

    #[test]
    fn redacts_a_pem_block_escaped_inside_json() {
        let text = format!(
            r#"{{"private_key": "{}\n", "x": 1}}"#,
            pem_lines().join(r"\n")
        );
        let out = redact(&text);
        assert_eq!(out, r#"{"private_key": "[REDACTED]\n", "x": 1}"#);
    }

    #[test]
    fn redacts_to_the_end_when_the_pem_never_closes() {
        let lines = pem_lines();
        let text = format!("antes\n{}", lines[..3].join("\n"));
        assert_eq!(redact(&text), "antes\n[REDACTED]");
    }

    #[test]
    fn leaves_public_keys_and_certificates_alone() {
        for kind in ["PUBLIC KEY", "CERTIFICATE"] {
            let text = format!(
                "-----BEGIN {kind}-----\n{}\n-----END {kind}-----",
                fake("", "Zm9v", 16)
            );
            assert!(matches!(redact(&text), Cow::Borrowed(_)), "{kind}");
        }
    }

    #[test]
    fn redacts_the_pem_body_when_it_arrives_line_by_line() {
        let mut r = LineRedactor::default();
        let mut out: Vec<String> = vec![r.line("$ cat id_rsa").into_owned()];
        out.extend(pem_lines().iter().map(|l| r.line(l).into_owned()));
        out.push(r.line("$ ls").into_owned());
        assert_eq!(
            out,
            [
                "$ cat id_rsa",
                "[REDACTED]",
                "[REDACTED]",
                "[REDACTED]",
                "[REDACTED]",
                "[REDACTED]",
                "$ ls"
            ]
        );
    }

    #[test]
    fn line_redactor_leaves_loose_base64_alone() {
        // Base64 solto (hash, dump) fora de um bloco PEM não é segredo.
        let mut r = LineRedactor::default();
        let line = fake("", "Zm9vYmFy", 8);
        assert!(matches!(r.line(&line), Cow::Borrowed(_)));
        assert!(matches!(r.line("ok"), Cow::Borrowed(_)));
    }
}
