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

/// Por que um snippet chamado pelo nome não pôde virar comando.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AliasError {
    #[error("{snippet}: falta o argumento {{{{{arg}}}}}")]
    MissingArg { snippet: String, arg: String },
    #[error("{snippet}: recebe {expected} argumento(s), vieram {got}")]
    TooManyArgs {
        snippet: String,
        expected: usize,
        got: usize,
    },
    #[error("{0}: snippet de várias linhas sai pela paleta (⌘⇧R), não pelo nome")]
    Multiline(String),
}

/// As palavras de `rest`, cada uma com as aspas como foram digitadas: o valor
/// entra no comando do jeito que o dono escreveu, e `"a b"` continua um
/// argumento só para o shell.
fn words(rest: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None;
    let mut quote: Option<char> = None;
    for (i, c) in rest.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => {
                quote = Some(c);
                start.get_or_insert(i);
            }
            (None, c) if c.is_whitespace() => {
                if let Some(s) = start.take() {
                    out.push(&rest[s..i]);
                }
            }
            (None, _) => {
                start.get_or_insert(i);
            }
        }
    }
    if let Some(s) = start {
        out.push(&rest[s..]);
    }
    out
}

/// A linha começa pelo nome de um snippet? Então vira o comando dele.
///
/// O snippet como alias (S5): `vpn-x` + Enter roda o comando do `vpn-x`, e
/// `vpn-x prod` preenche os `{{arg}}` por posição. Sem placeholder, o que vem
/// depois do nome segue colado no fim, como num `alias` do shell.
///
/// - Snippet LOCAL ganha de qualquer comando com o mesmo nome: foi o dono que
///   o criou com aquele nome.
/// - Snippet de REPOSITÓRIO só vale pelo nome quando nada no shell atende por
///   ele (`shadows`): um `.tyba/config.toml` consentido não pode, sozinho,
///   sequestrar o `ls` ou o `git` de quem digita.
/// - Linha que começa com espaço não expande: é o jeito de pedir o comando de
///   verdade.
pub fn expand_alias<'a>(
    line: &str,
    snippets: &'a [Snippet],
    shadows: &dyn Fn(&str) -> bool,
) -> Result<Option<(&'a Snippet, String)>, AliasError> {
    if line.starts_with(char::is_whitespace) {
        return Ok(None);
    }
    let (name, rest) = match line.split_once(char::is_whitespace) {
        Some((name, rest)) => (name, rest.trim()),
        None => (line, ""),
    };
    let found = snippets
        .iter()
        .filter(|s| s.name == name)
        .find(|s| s.source == Source::Local)
        .or_else(|| {
            snippets
                .iter()
                .find(|s| s.name == name && s.source == Source::Repo && !shadows(name))
        });
    let Some(snippet) = found else {
        return Ok(None);
    };
    let fields = placeholders(&snippet.command);
    let command = if fields.is_empty() {
        if rest.is_empty() {
            snippet.command.clone()
        } else {
            format!("{} {rest}", snippet.command)
        }
    } else {
        let args = words(rest);
        if args.len() > fields.len() {
            return Err(AliasError::TooManyArgs {
                snippet: name.to_string(),
                expected: fields.len(),
                got: args.len(),
            });
        }
        let mut values = Vec::with_capacity(fields.len());
        for (i, field) in fields.iter().enumerate() {
            let value = match (args.get(i), &field.default) {
                (Some(arg), _) => (*arg).to_string(),
                (None, Some(default)) => default.clone(),
                (None, None) => {
                    return Err(AliasError::MissingArg {
                        snippet: name.to_string(),
                        arg: field.name.clone(),
                    })
                }
            };
            values.push((field.name.clone(), value));
        }
        render(&snippet.command, &values)
    };
    if command.contains('\n') {
        return Err(AliasError::Multiline(name.to_string()));
    }
    Ok(Some((snippet, command)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snip(name: &str, command: &str, source: Source) -> Snippet {
        Snippet {
            id: name.into(),
            name: name.into(),
            command: command.into(),
            description: None,
            tags: Vec::new(),
            source,
        }
    }

    fn expand(
        line: &str,
        all: &[Snippet],
        shadowed: &[&str],
    ) -> Result<Option<String>, AliasError> {
        expand_alias(line, all, &|n| shadowed.contains(&n)).map(|hit| hit.map(|(_, c)| c))
    }

    #[test]
    fn alias_runs_the_snippet_by_name() {
        let all = [snip("vpn-x", "sudo openfortivpn -c /etc/x", Source::Local)];
        assert_eq!(
            expand("vpn-x", &all, &[]).unwrap().as_deref(),
            Some("sudo openfortivpn -c /etc/x")
        );
        assert_eq!(expand("vpn-xy", &all, &[]).unwrap(), None);
        assert_eq!(expand("echo vpn-x", &all, &[]).unwrap(), None);
        assert_eq!(
            expand(" vpn-x", &all, &[]).unwrap(),
            None,
            "espaço na frente pede o comando real"
        );
    }

    #[test]
    fn alias_without_placeholders_appends_the_rest() {
        let all = [snip("ll", "ls -la", Source::Local)];
        assert_eq!(
            expand("ll /tmp", &all, &[]).unwrap().as_deref(),
            Some("ls -la /tmp")
        );
    }

    #[test]
    fn alias_fills_placeholders_by_position_keeping_quotes() {
        let all = [snip(
            "dep",
            "deploy --env {{env}} --msg {{msg=ok}}",
            Source::Local,
        )];
        assert_eq!(
            expand("dep prod \"primeira vez\"", &all, &[])
                .unwrap()
                .as_deref(),
            Some("deploy --env prod --msg \"primeira vez\"")
        );
        assert_eq!(
            expand("dep prod", &all, &[]).unwrap().as_deref(),
            Some("deploy --env prod --msg ok")
        );
        assert_eq!(
            expand("dep", &all, &[]),
            Err(AliasError::MissingArg {
                snippet: "dep".into(),
                arg: "env".into()
            })
        );
        assert!(matches!(
            expand("dep a b c", &all, &[]),
            Err(AliasError::TooManyArgs { .. })
        ));
    }

    #[test]
    fn repo_snippet_never_shadows_a_real_command() {
        let all = [
            snip("ls", "rm -rf ~", Source::Repo),
            snip("build", "make all", Source::Repo),
        ];
        assert_eq!(
            expand("ls", &all, &["ls"]).unwrap(),
            None,
            "o ls de verdade ganha"
        );
        assert_eq!(
            expand("build", &all, &["ls"]).unwrap().as_deref(),
            Some("make all")
        );
    }

    #[test]
    fn local_snippet_wins_over_command_and_repo() {
        let all = [
            snip("gs", "repo", Source::Repo),
            snip("gs", "git status", Source::Local),
        ];
        assert_eq!(
            expand("gs", &all, &["gs"]).unwrap().as_deref(),
            Some("git status")
        );
    }

    #[test]
    fn multiline_snippet_stays_in_the_palette() {
        let all = [snip("two", "echo a\necho b", Source::Local)];
        assert_eq!(
            expand("two", &all, &[]),
            Err(AliasError::Multiline("two".into()))
        );
    }

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
