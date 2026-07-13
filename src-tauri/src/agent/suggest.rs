//! Sugestão de mensagem de commit via agente headless (spec funil-entregar).
//!
//! Roda a CLI do agente em modo print (`claude -p` / `codex exec`) com o
//! diff staged no prompt — rede acontece, mas só por clique explícito no
//! painel (mesmo precedente do fetch). O diff vai capado: mensagem de
//! commit não precisa de lockfile inteiro.

use std::path::Path;
use std::time::Duration;

use crate::error::AppError;
use crate::forge::exec;
use crate::worktree::{git_in, run_git};

const MAX_DIFF_BYTES: usize = 48 * 1024;
const SUGGEST_TIMEOUT: Duration = Duration::from_secs(60);

pub fn suggest_commit_message(repo: &Path, agent: &str) -> Result<String, AppError> {
    if !matches!(agent, "claude" | "codex") {
        return Err(AppError::new("suggest.unsupported_agent").with("agent", agent));
    }
    let diff = staged_diff(repo)?;
    if diff.trim().is_empty() {
        return Err(AppError::new("suggest.no_staged"));
    }
    let prompt = build_prompt(&diff);

    let out = if agent == "claude" {
        exec::run(
            "claude",
            &["-p", "--output-format", "text"],
            repo,
            Some(prompt.as_bytes()),
            SUGGEST_TIMEOUT,
        )
    } else {
        exec::run("codex", &["exec", &prompt], repo, None, SUGGEST_TIMEOUT)
    }
    .map_err(|detail| AppError::new("suggest.failed").with("detail", detail))?;

    if !out.ok {
        return Err(AppError::new("suggest.failed").with("detail", out.stderr_message()));
    }
    let suggestion = clean_suggestion(&String::from_utf8_lossy(&out.stdout));
    if suggestion.is_empty() {
        return Err(AppError::new("suggest.failed").with("detail", "resposta vazia do agente"));
    }
    Ok(suggestion)
}

fn staged_diff(repo: &Path) -> Result<String, AppError> {
    let raw = run_git(
        {
            let mut c = git_in(repo);
            c.args(["diff", "--cached", "--no-ext-diff", "--no-color"]);
            c
        },
        "git diff --cached",
    )
    .map_err(|detail| AppError::new("suggest.failed").with("detail", detail))?;
    Ok(cap_diff(&String::from_utf8_lossy(&raw)))
}

fn cap_diff(diff: &str) -> String {
    if diff.len() <= MAX_DIFF_BYTES {
        return diff.to_string();
    }
    let mut end = MAX_DIFF_BYTES;
    while !diff.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[diff truncado — {} bytes no total]",
        &diff[..end],
        diff.len()
    )
}

fn build_prompt(diff: &str) -> String {
    [
        "Escreva a mensagem de commit pro diff staged abaixo, no padrão conventional commits (`tipo(escopo): resumo`), em português.",
        "Responda com UMA linha apenas: a mensagem crua, sem aspas, sem crases, sem explicação antes ou depois.",
        "",
        diff,
    ]
    .join("\n")
}

fn clean_suggestion(raw: &str) -> String {
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    line.trim_matches(|c| c == '"' || c == '`' || c == '\'')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_carries_the_diff_and_the_one_line_rule() {
        let prompt = build_prompt("diff --git a/x b/x\n+linha\n");
        assert!(prompt.contains("conventional commits"));
        assert!(prompt.contains("UMA linha"));
        assert!(prompt.ends_with("+linha\n"));
    }

    #[test]
    fn cap_truncates_big_diffs_at_a_char_boundary() {
        let big = "é".repeat(MAX_DIFF_BYTES);
        let capped = cap_diff(&big);
        assert!(capped.len() < big.len() + 64);
        assert!(capped.contains("[diff truncado"));
        assert_eq!(cap_diff("pequeno"), "pequeno");
    }

    #[test]
    fn suggestion_takes_the_first_line_and_strips_wrapping() {
        assert_eq!(
            clean_suggestion("\n  \"feat(x): faz coisa\"  \nexplicação\n"),
            "feat(x): faz coisa"
        );
        assert_eq!(clean_suggestion("`fix: corrige`"), "fix: corrige");
        assert_eq!(clean_suggestion("   \n\n"), "");
    }

    #[test]
    fn unsupported_agent_is_refused_before_any_exec() {
        let err = suggest_commit_message(Path::new("/tmp"), "meu-script").unwrap_err();
        assert_eq!(err.code, "suggest.unsupported_agent", "{err}");
    }
}
