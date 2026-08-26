//! Completar caminho na linha de comando.
//!
//! Não é o `@arquivo` do prompt de agente (busca fuzzy no worktree): aqui é
//! completar o token que está sendo digitado contra o diretório de verdade,
//! como o Tab do shell faz. Diretório vem com `/` no fim para o próximo Tab
//! continuar descendo.

pub mod argument;
pub mod binary;

use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 40;

fn expand_home(raw: &str) -> Option<PathBuf> {
    let rest = raw.strip_prefix('~')?;
    let home = std::env::var_os("HOME")?;
    let home = PathBuf::from(home);
    Some(match rest.strip_prefix('/') {
        Some(tail) => home.join(tail),
        None if rest.is_empty() => home,
        None => return None,
    })
}

/// Quebra o token em (prefixo textual do diretório, base a filtrar).
///
/// O prefixo é devolvido **como o usuário digitou** — `../s` continua `../`, e
/// não vira caminho absoluto — para que a troca no texto não reescreva o que ele
/// escolheu escrever.
fn split_token(token: &str) -> (&str, &str) {
    match token.rfind('/') {
        Some(cut) => (&token[..=cut], &token[cut + 1..]),
        None => ("", token),
    }
}

fn resolve_dir(cwd: &Path, dir_part: &str) -> Option<PathBuf> {
    if dir_part.is_empty() {
        return Some(cwd.to_path_buf());
    }
    if dir_part.starts_with('~') {
        return expand_home(dir_part);
    }
    if dir_part.starts_with('/') {
        return Some(PathBuf::from(dir_part));
    }
    Some(cwd.join(dir_part))
}

/// Candidatos para o token, já prontos para substituí-lo no texto.
pub fn complete_path(cwd: &Path, token: &str) -> Vec<String> {
    let (dir_part, base) = split_token(token);
    let Some(dir) = resolve_dir(cwd, dir_part) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    // Oculto só aparece quando o usuário já escreveu o ponto: senão todo `cd `
    // num home despeja `.config`, `.cache`, `.ssh` na cara dele.
    let wants_hidden = base.starts_with('.');
    let lower = base.to_lowercase();

    let mut exact: Vec<(bool, String)> = Vec::new();
    let mut loose: Vec<(bool, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && !wants_hidden {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let suffix = if is_dir { "/" } else { "" };
        let completed = format!("{dir_part}{name}{suffix}");
        if name.starts_with(base) {
            exact.push((is_dir, completed));
        } else if !base.is_empty() && name.to_lowercase().starts_with(&lower) {
            loose.push((is_dir, completed));
        }
    }

    // Case-insensitive só entra quando o casamento exato não achou nada: com
    // `RE` num diretório que tem `README` e `retry/`, quem digitou maiúscula
    // quis o README.
    let mut found = if exact.is_empty() { loose } else { exact };
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.truncate(MAX_ENTRIES);
    found.into_iter().map(|(_, name)| name).collect()
}

/// Subcomandos e flags aprendidos do próprio histórico.
///
/// A base de specs de comando do Warp é um crate inteiro e é AGPL — fora de
/// alcance. Mas o TYBA já guarda tudo que o dono digitou: para `git co` basta
/// olhar os comandos que começaram com `git ` e oferecer o token seguinte. Sai
/// personalizado (só aparece o que ele de fato usa), não envelhece e não custa
/// manutenção.
///
/// `commands` chega ordenado por recência; a ordem é preservada.
pub fn next_tokens(commands: &[String], prefix: &str, token: &str) -> Vec<String> {
    if prefix.trim().is_empty() {
        return Vec::new();
    }
    let mut found: Vec<String> = Vec::new();
    for command in commands {
        let Some(rest) = command.strip_prefix(prefix) else {
            continue;
        };
        let Some(next) = rest.split_whitespace().next() else {
            continue;
        };
        if !next.starts_with(token) || next == token {
            continue;
        }
        if found.iter().any(|seen| seen == next) {
            continue;
        }
        found.push(next.to_string());
        if found.len() >= 8 {
            break;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> Vec<String> {
        vec![
            "git commit -m wip".to_string(),
            "git checkout main".to_string(),
            "git commit --no-verify".to_string(),
            "cargo test --lib".to_string(),
            "cargo test --nocapture".to_string(),
            "ls -la".to_string(),
        ]
    }

    #[test]
    fn learns_subcommands_from_what_was_actually_used() {
        assert_eq!(
            next_tokens(&history(), "git ", "c"),
            vec!["commit", "checkout"]
        );
    }

    #[test]
    fn learns_flags_deeper_in_the_line() {
        assert_eq!(
            next_tokens(&history(), "cargo test ", "--"),
            vec!["--lib", "--nocapture"]
        );
    }

    #[test]
    fn keeps_recency_order_and_does_not_repeat() {
        // `git commit` aparece duas vezes no histórico; a lista mostra uma.
        let found = next_tokens(&history(), "git ", "");
        assert_eq!(found, vec!["commit", "checkout"]);
    }

    #[test]
    fn ignores_commands_that_do_not_share_the_prefix() {
        assert!(next_tokens(&history(), "docker ", "").is_empty());
    }

    #[test]
    fn never_offers_what_is_already_written_whole() {
        assert!(!next_tokens(&history(), "git ", "commit").contains(&"commit".to_string()));
    }

    #[test]
    fn empty_prefix_completes_nothing() {
        // Sem prefixo isto viraria "sugira qualquer primeira palavra", que é
        // trabalho do histórico, não da completação de argumento.
        assert!(next_tokens(&history(), "", "g").is_empty());
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::create_dir(dir.path().join("scripts")).unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join("README.md"), "").unwrap();
        std::fs::write(dir.path().join("setup.sh"), "").unwrap();
        std::fs::write(dir.path().join(".env"), "").unwrap();
        std::fs::create_dir(dir.path().join("src/lib")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "").unwrap();
        dir
    }

    #[test]
    fn completes_by_prefix_with_directories_first() {
        let dir = fixture();
        let found = complete_path(dir.path(), "s");
        assert_eq!(found, vec!["scripts/", "src/", "setup.sh"]);
    }

    #[test]
    fn directory_gets_a_trailing_slash_so_the_next_tab_descends() {
        let dir = fixture();
        assert!(complete_path(dir.path(), "sr").contains(&"src/".to_string()));
    }

    #[test]
    fn descends_into_the_typed_directory_keeping_what_was_written() {
        let dir = fixture();
        let found = complete_path(dir.path(), "src/");
        assert_eq!(found, vec!["src/lib/", "src/main.rs"]);
    }

    #[test]
    fn hidden_entries_only_show_after_the_dot() {
        // `cd ` no home não pode despejar `.ssh` e `.config` na cara do usuário.
        let dir = fixture();
        let visible = complete_path(dir.path(), "");
        assert!(!visible.iter().any(|name| name.starts_with('.')));

        let hidden = complete_path(dir.path(), ".");
        assert!(hidden.contains(&".git/".to_string()));
        assert!(hidden.contains(&".env".to_string()));
    }

    #[test]
    fn falls_back_to_case_insensitive_only_when_exact_finds_nothing() {
        let dir = fixture();
        assert_eq!(complete_path(dir.path(), "RE"), vec!["README.md"]);
        // `s` casa exato com três: o insensitive não entra e não traz mais nada.
        assert_eq!(complete_path(dir.path(), "s").len(), 3);
    }

    #[test]
    fn unknown_directory_completes_to_nothing_instead_of_erroring() {
        let dir = fixture();
        assert!(complete_path(dir.path(), "nao/existe/a").is_empty());
    }

    #[test]
    fn splits_the_token_preserving_what_the_user_wrote() {
        assert_eq!(split_token("src/lib/ip"), ("src/lib/", "ip"));
        assert_eq!(split_token("tyba"), ("", "tyba"));
        assert_eq!(split_token("../ou"), ("../", "ou"));
        assert_eq!(split_token("/usr/"), ("/usr/", ""));
    }

    #[test]
    fn relative_prefix_is_not_rewritten_as_absolute() {
        let dir = fixture();
        let nested = dir.path().join("src");
        let found = complete_path(&nested, "../scr");
        assert_eq!(found, vec!["../scripts/"]);
    }
}
