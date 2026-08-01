//! Snippets de comando.
//!
//! Modelo lido em desenho do Warp (`crates/cloud_object_models/src/workflow.rs`)
//! — nome, comando, descrição, tags e argumentos com placeholder. Ver o ADR
//! "O Warp é referência de desenho, nunca de código" no cofre.
//!
//! Duas origens: os locais, escritos pelo dono; e os do repositório, declarados
//! em `[[snippet]]` no `.tyba/config.toml`. A segunda só aparece depois do
//! consentimento por hash que a config de agente já usa — clonar um repositório
//! não pode, sozinho, colocar comando na paleta do dono.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// De onde veio. O front marca o snippet de repo na UI — o usuário precisa
    /// saber que aquele comando não é dele.
    #[serde(default)]
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Local,
    Repo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placeholder {
    pub name: String,
    pub default: Option<String>,
}

const OPEN: &str = "{{";
const CLOSE: &str = "}}";
const MAX_NAME: usize = 64;

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .chars()
            .all(|c| c == '_' || c == '-' || c.is_ascii_alphanumeric())
}

/// Extrai os placeholders `{{nome}}` / `{{nome=padrão}}`, na ordem de aparição e
/// sem repetir. Nome inválido não vira placeholder: o texto fica literal, para
/// que `awk '{{print $1}}'` não seja lido como campo a preencher.
pub fn placeholders(command: &str) -> Vec<Placeholder> {
    let mut found: Vec<Placeholder> = Vec::new();
    let mut rest = command;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else {
            break;
        };
        let body = &after[..end];
        rest = &after[end + CLOSE.len()..];
        let (name, default) = match body.split_once('=') {
            Some((name, value)) => (name.trim(), Some(value.trim().to_string())),
            None => (body.trim(), None),
        };
        if !valid_name(name) {
            continue;
        }
        if found.iter().any(|p| p.name == name) {
            continue;
        }
        found.push(Placeholder {
            name: name.to_string(),
            default,
        });
    }
    found
}

/// Substitui os placeholders pelos valores informados. Placeholder sem valor cai
/// no padrão declarado; sem padrão, vira string vazia — nunca sobra `{{x}}` na
/// linha, que o shell interpretaria como brace expansion.
pub fn render(command: &str, values: &[(String, String)]) -> String {
    let mut out = String::with_capacity(command.len());
    let mut rest = command;
    loop {
        let Some(start) = rest.find(OPEN) else {
            out.push_str(rest);
            return out;
        };
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else {
            out.push_str(rest);
            return out;
        };
        let body = &after[..end];
        let (name, default) = match body.split_once('=') {
            Some((name, value)) => (name.trim(), Some(value.trim())),
            None => (body.trim(), None),
        };
        out.push_str(&rest[..start]);
        if valid_name(name) {
            let value = values
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
                .or(default)
                .unwrap_or("");
            out.push_str(value);
        } else {
            out.push_str(&rest[start..start + OPEN.len() + end + CLOSE.len()]);
        }
        rest = &after[end + CLOSE.len()..];
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SnippetError {
    #[error("snippet sem nome")]
    EmptyName,
    #[error("snippet sem comando")]
    EmptyCommand,
    #[error("comando de snippet não pode conter quebra de linha sem bracketed paste")]
    ControlChars,
}

/// Um snippet é injetado na linha de comando do usuário. Caractere de controle
/// ali é o que transforma "colar" em "executar" — o `\r` do fim de uma linha
/// basta. Multilinha (`\n`) segue permitido: a injeção passa pelo caminho de
/// paste, que exige bracketed paste e confirma com o usuário.
pub fn validate(name: &str, command: &str) -> Result<(), SnippetError> {
    if name.trim().is_empty() {
        return Err(SnippetError::EmptyName);
    }
    if command.trim().is_empty() {
        return Err(SnippetError::EmptyCommand);
    }
    if command
        .chars()
        .any(|c| c != '\n' && c != '\t' && c.is_control())
    {
        return Err(SnippetError::ControlChars);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(command: &str) -> Vec<String> {
        placeholders(command).into_iter().map(|p| p.name).collect()
    }

    #[test]
    fn finds_placeholders_in_order() {
        assert_eq!(
            names("git checkout -b {{tipo}}/{{slug}}"),
            vec!["tipo", "slug"]
        );
    }

    #[test]
    fn repeated_placeholder_is_asked_once() {
        assert_eq!(names("echo {{a}} && echo {{a}}"), vec!["a"]);
    }

    #[test]
    fn reads_the_declared_default() {
        let found = placeholders("git push origin {{branch=main}}");
        assert_eq!(
            found,
            vec![Placeholder {
                name: "branch".into(),
                default: Some("main".into()),
            }]
        );
    }

    #[test]
    fn awk_braces_are_not_placeholders() {
        assert!(names("awk '{{print $1}}'").is_empty());
        assert_eq!(render("awk '{{print $1}}'", &[]), "awk '{{print $1}}'");
    }

    #[test]
    fn unclosed_placeholder_is_left_alone() {
        assert!(names("echo {{oops").is_empty());
        assert_eq!(render("echo {{oops", &[]), "echo {{oops");
    }

    #[test]
    fn renders_values_defaults_and_blanks() {
        let command = "deploy {{env}} --tag {{tag=latest}} {{extra}}";
        let rendered = render(command, &[("env".to_string(), "prod".to_string())]);
        assert_eq!(rendered, "deploy prod --tag latest ");
    }

    #[test]
    fn render_never_leaves_a_placeholder_behind() {
        // `{{x}}` que sobrasse viraria brace expansion no shell.
        let rendered = render("echo {{x}}", &[]);
        assert!(!rendered.contains("{{"));
    }

    #[test]
    fn value_wins_over_default() {
        let rendered = render(
            "git push origin {{branch=main}}",
            &[("branch".to_string(), "fix/x".to_string())],
        );
        assert_eq!(rendered, "git push origin fix/x");
    }

    #[test]
    fn rejects_carriage_return_that_would_execute_the_line() {
        assert_eq!(
            validate("deploy", "deploy prod\r"),
            Err(SnippetError::ControlChars)
        );
        assert_eq!(
            validate("deploy", "deploy prod\x1b]133;C\x07"),
            Err(SnippetError::ControlChars)
        );
    }

    #[test]
    fn accepts_multiline_command() {
        assert_eq!(validate("build", "cd app\nbun run build"), Ok(()));
    }

    #[test]
    fn rejects_blank_name_or_command() {
        assert_eq!(validate("  ", "ls"), Err(SnippetError::EmptyName));
        assert_eq!(validate("listar", "  "), Err(SnippetError::EmptyCommand));
    }

    #[test]
    fn placeholder_name_is_bounded() {
        let huge = "x".repeat(MAX_NAME + 1);
        assert!(names(&format!("echo {{{{{huge}}}}}")).is_empty());
    }
}
