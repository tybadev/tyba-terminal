use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;

pub const REDACTION_MARK: &str = "[REDACTED]";

/// Começo e fim de um bloco de chave privada: PEM (PKCS#1/#8, EC, OpenSSH,
/// cifrada), PGP armor e a parte privada do `.ppk` do PuTTY.
const KEY_START: &str =
    r"-----BEGIN (?:[A-Z ]*PRIVATE KEY|PGP PRIVATE KEY BLOCK)-----|Private-Lines: *[0-9]+";
const KEY_END: &str =
    r"-----END (?:[A-Z ]*PRIVATE KEY|PGP PRIVATE KEY BLOCK)-----|Private-MAC: *[0-9A-Fa-f]*";
/// Cabeçalho que pode estar DENTRO de um bloco de chave (PEM cifrado, armor
/// do PGP); o resto do corpo é base64.
const KEY_HEADER: &str = r"(?:Proc-Type|DEK-Info|Comment|Version|Hash):";
/// Quebra de linha real ou `\n` escapado (chave dentro de JSON/YAML).
const KEY_SEP: &str = r"(?:[\t ]*(?:\r?\n|\\n))+[\t ]*";

/// O bloco de chave vem primeiro. Ele engole a linha do começo, as linhas
/// seguintes enquanto tiverem cara de corpo e, se vier logo depois, a do fim.
/// Um BEGIN citado (`rg`, diff de fixture) leva só ele mesmo: a primeira linha
/// que não parece corpo encerra o bloco.
///
/// Linha real de corpo precisa ir até o fim da linha (`(?m:$)`), senão o
/// base64 comeria o prefixo de `src/a.rs:10: …`. No `\n` escapado não há fim
/// de linha a exigir; o pior caso é um prefixo a mais dentro da mesma string.
static SECRETS: LazyLock<Regex> = LazyLock::new(|| {
    let real_line =
        format!(r"(?:[\t ]*\r?\n)+[\t ]*(?:{KEY_HEADER}[^\r\n]*|[A-Za-z0-9+/=]+[\t ]*\r?(?m:$))");
    let escaped_line = format!(r#"(?:\\n)+(?:{KEY_HEADER}[^\\"\r\n]*|[A-Za-z0-9+/=]+)"#);
    let key_block =
        format!(r"(?:{KEY_START})(?:{real_line}|{escaped_line})*(?:{KEY_SEP}(?:{KEY_END}))?");
    Regex::new(
        &[
            key_block.as_str(),
            r"(AKIA|ASIA)[0-9A-Z]{16}",
            r"sk-[A-Za-z0-9_-]{20,}",
            r"gh[pousr]_[A-Za-z0-9]{20,}",
            r"github_pat_[A-Za-z0-9_]{20,}",
            r"glpat-[A-Za-z0-9_-]{20,}",
            r"xox[abposr]-[A-Za-z0-9-]{10,}",
            r"AIza[0-9A-Za-z_-]{35}",
            r"eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
        ]
        .join("|"),
    )
    .unwrap()
});

/// `scheme://user:SENHA@host`: o grupo 1 fica, só a senha vira marca. A senha
/// vai até o ÚLTIMO `@` antes de `/`, `?`, `#`, aspas ou espaço — host nunca
/// tem `@`, senha crua às vezes tem.
static URL_PASSWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"([A-Za-z][A-Za-z0-9+.-]*://[^\s:/@]*:)[^\s/?#"'<>]+@"#).unwrap()
});

static KEY_START_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(KEY_START).unwrap());
static KEY_END_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(KEY_END).unwrap());
static KEY_BODY_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^\s*(?:[A-Za-z0-9+/=]*|{KEY_HEADER}.*)\s*$")).unwrap());

pub fn redact(input: &str) -> Cow<'_, str> {
    // O bloco de chave vira UMA marca seguida das quebras que ele tinha: o
    // texto mantém o número de linhas (`error_line` do CI, export de logs).
    let mark = |c: &regex::Captures| {
        let m = &c[0];
        let nl = if m.contains("\r\n") { "\r\n" } else { "\n" };
        format!("{REDACTION_MARK}{}", nl.repeat(m.matches('\n').count()))
    };
    let password = |c: &regex::Captures| format!("{}{REDACTION_MARK}@", &c[1]);
    match SECRETS.replace_all(input, mark) {
        Cow::Borrowed(s) => URL_PASSWORD.replace_all(s, password),
        Cow::Owned(s) => Cow::Owned(URL_PASSWORD.replace_all(&s, password).into_owned()),
    }
}

/// Mesmos padrões do [`redact`], como pergunta: o texto carrega um segredo?
pub fn contains_secret(input: &str) -> bool {
    SECRETS.is_match(input) || URL_PASSWORD.is_match(input)
}

/// A linha poderia ser corpo de uma chave: base64, vazia ou cabeçalho de PEM
/// cifrado / armor do PGP.
pub fn looks_like_key_body(line: &str) -> bool {
    KEY_BODY_LINE.is_match(line)
}

/// [`redact`] linha a linha, lembrando do bloco de chave aberto.
///
/// Quem redige por linha lógica (blocos) recebe o BEGIN, o corpo base64 e o
/// END separados. O `redact` sozinho esconderia só a linha do BEGIN, e o corpo
/// — que É a chave — passaria. Aqui, depois de um começo de chave, as linhas
/// com cara de corpo viram marca até o fim do bloco ou até a primeira que não
/// parece corpo — essa fecha o estado e é redigida normalmente, para um BEGIN
/// citado não apagar o resto da saída. Base64 solto fora de um bloco não é
/// tocado: hash e dump dariam falso positivo demais.
///
/// Como no `redact`, `Cow::Owned` quer dizer que algo foi redigido.
#[derive(Default)]
pub struct LineRedactor {
    in_key: bool,
}

impl LineRedactor {
    pub fn line<'a>(&mut self, line: &'a str) -> Cow<'a, str> {
        if self.in_key {
            if let Some(end) = KEY_END_RE.find(line) {
                self.in_key = false;
                let rest = self.line(&line[end.end()..]);
                return Cow::Owned(format!("{REDACTION_MARK}{rest}"));
            }
            if looks_like_key_body(line) {
                return Cow::Owned(REDACTION_MARK.to_string());
            }
            self.in_key = false;
        }
        let out = redact(line);
        // Começo de chave sempre redige, então `Borrowed` dispensa a busca.
        if matches!(out, Cow::Owned(_)) {
            self.in_key = KEY_START_RE
                .find_iter(line)
                .last()
                .is_some_and(|b| !KEY_END_RE.is_match(&line[b.end()..]));
        }
        out
    }

    /// A linha fecha uma chave cujo começo este redator não viu (o teto de
    /// linhas cortou a cabeça do bloco). Quem tem as linhas anteriores redige
    /// para trás as que parecem corpo.
    pub fn closes_unseen(&self, line: &str) -> bool {
        !self.in_key
            && KEY_END_RE
                .find(line)
                .is_some_and(|e| !KEY_START_RE.is_match(&line[..e.start()]))
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
        // Mesmo número de linhas: as 5 linhas da chave viram marca + 4 quebras.
        assert_eq!(out, "antes\n[REDACTED]\n\n\n\n\ndepois");
        assert_eq!(out.lines().count(), text.lines().count());
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
    fn redacts_the_body_when_the_pem_never_closes() {
        let lines = pem_lines();
        let text = format!("antes\n{}", lines[..3].join("\n"));
        assert_eq!(redact(&text), "antes\n[REDACTED]\n\n");
    }

    #[test]
    fn keeps_crlf_line_count() {
        let text = format!("a\r\n{}\r\nb", pem_lines().join("\r\n"));
        assert_eq!(redact(&text), "a\r\n[REDACTED]\r\n\r\n\r\n\r\n\r\nb");
    }

    /// `rg 'PRIVATE KEY-----'` numa fixture: o BEGIN citado não pode apagar o
    /// resto da saída.
    fn quoted_begin() -> Vec<String> {
        vec![
            format!(
                "tests/k.rs:3:    let k = \"-----BEGIN {} KEY-----\";",
                "RSA PRIVATE"
            ),
            "tests/k.rs:4:    assert!(k.len() > 0);".to_string(),
            "src/a.rs:10:fn main() {}".to_string(),
            "README.md:2:texto".to_string(),
        ]
    }

    #[test]
    fn a_quoted_begin_keeps_the_rest_of_the_text() {
        let lines = quoted_begin();
        let out = redact(&lines.join("\n")).into_owned();
        let out: Vec<&str> = out.lines().collect();
        assert_eq!(out[1..], lines[1..]);
    }

    #[test]
    fn a_quoted_begin_keeps_the_next_lines_line_by_line() {
        let lines = quoted_begin();
        let mut r = LineRedactor::default();
        let out: Vec<String> = lines.iter().map(|l| r.line(l).into_owned()).collect();
        assert!(out[0].contains(REDACTION_MARK));
        assert_eq!(out[1..], lines[1..]);
    }

    fn pgp_lines() -> Vec<String> {
        vec![
            format!("-----BEGIN PGP {} BLOCK-----", "PRIVATE KEY"),
            "Version: sintetica".to_string(),
            String::new(),
            fake("", "cXV4", 16),
            fake("", "Zm9v", 16),
            "=YWJj".to_string(),
            format!("-----END PGP {} BLOCK-----", "PRIVATE KEY"),
        ]
    }

    fn putty_lines() -> Vec<String> {
        vec![
            "PuTTY-User-Key-File-3: ssh-ed25519".to_string(),
            "Encryption: none".to_string(),
            "Comment: sintetica".to_string(),
            "Public-Lines: 1".to_string(),
            fake("", "UFVC", 16),
            "Private-Lines: 1".to_string(),
            fake("", "UFJJ", 16),
            format!("Private-MAC: {}", "ab".repeat(32)),
        ]
    }

    #[test]
    fn redacts_pgp_and_putty_private_keys_in_one_text() {
        let pgp = format!("a\n{}\nb", pgp_lines().join("\n"));
        assert_eq!(
            redact(&pgp),
            format!("a\n{REDACTION_MARK}{}\nb", "\n".repeat(6))
        );

        let ppk = putty_lines().join("\n");
        let out = redact(&ppk);
        assert!(!out.contains(&fake("", "UFJJ", 16)), "{out}");
        assert!(!out.contains("Private-MAC"), "{out}");
        // A parte pública fica.
        assert!(out.contains(&fake("", "UFVC", 16)), "{out}");
        assert_eq!(out.split('\n').count(), ppk.split('\n').count());
    }

    #[test]
    fn redacts_pgp_and_putty_private_keys_line_by_line() {
        for (lines, secret) in [
            (pgp_lines(), fake("", "cXV4", 16)),
            (putty_lines(), fake("", "UFJJ", 16)),
        ] {
            let mut r = LineRedactor::default();
            let mut out: Vec<String> = lines.iter().map(|l| r.line(l).into_owned()).collect();
            out.push(r.line("$ ls").into_owned());
            assert!(out.iter().all(|l| !l.contains(&secret)), "{out:?}");
            assert_eq!(out.last().unwrap(), "$ ls");
        }
    }

    #[test]
    fn spots_an_end_whose_begin_was_cut() {
        let r = LineRedactor::default();
        let end = format!("-----END {} KEY-----", "RSA PRIVATE");
        assert!(r.closes_unseen(&end));
        assert!(r.closes_unseen(&format!("Private-MAC: {}", "ab".repeat(4))));
        assert!(!r.closes_unseen(&pem_lines().join(r"\n")));
        assert!(!r.closes_unseen("-----END CERTIFICATE-----"));
        assert!(looks_like_key_body(&fake("", "Zm9v", 16)));
        assert!(looks_like_key_body(""));
        assert!(!looks_like_key_body("src/a.rs:1: x"));
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
