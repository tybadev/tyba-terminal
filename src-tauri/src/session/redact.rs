use std::borrow::Cow;
use std::sync::LazyLock;

use regex::{NoExpand, Regex};

pub const REDACTION_MARK: &str = "[REDACTED]";

static SECRETS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (AKIA|ASIA)[0-9A-Z]{16}
        | sk-[A-Za-z0-9_-]{20,}
        | eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+
        ",
    )
    .unwrap()
});

pub fn redact(input: &str) -> Cow<'_, str> {
    SECRETS.replace_all(input, NoExpand(REDACTION_MARK))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
