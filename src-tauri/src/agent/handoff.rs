//! Handoff entre agentes (proposta 11 da ia-v2): o texto que abre o agente
//! seguinte na mesma pasta — o pedido, o que mudou no último turno e onde o
//! agente anterior parou. Tudo passa pela redação antes de sair do core.

use crate::agent::turn::{TurnDiff, TurnDigest};

/// Teto do pedido e da fala final lidos do transcript.
pub const TEXT_MAX_CHARS: usize = 4000;
/// Quantos arquivos entram na lista; o resto vira "e mais N".
const FILES_MAX: usize = 30;

/// O prompt do handoff. `None` quando não há nada para passar.
pub fn prompt(
    digest: Option<&TurnDigest>,
    task: Option<&str>,
    last_reply: Option<&str>,
) -> Option<String> {
    let task = task
        .or_else(|| digest.and_then(|d| d.prompt.as_deref()))
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let last_reply = last_reply.map(str::trim).filter(|t| !t.is_empty());
    let changes = digest.and_then(changes);
    if task.is_none() && last_reply.is_none() && changes.is_none() {
        return None;
    }

    let mut out = String::from(
        "Você está assumindo um trabalho que outro agente começou nesta mesma pasta. \
         Confira o estado com `git status` e `git diff` antes de mudar qualquer coisa.\n",
    );
    if let Some(task) = task {
        out.push_str("\n## Pedido\n");
        out.push_str(task);
        out.push('\n');
    }
    if let Some(changes) = changes {
        out.push_str("\n## O que mudou no último turno\n");
        out.push_str(&changes);
    }
    if let Some(reply) = last_reply {
        out.push_str("\n## Onde o agente anterior parou\n");
        out.push_str(reply);
        out.push('\n');
    }
    Some(crate::session::redact::redact(&out).into_owned())
}

fn changes(d: &TurnDigest) -> Option<String> {
    if d.diff != TurnDiff::Ok || d.files_total == 0 {
        return None;
    }
    let mut out = format!(
        "{} arquivo(s), +{} −{}{}\n",
        d.files_total,
        d.insertions,
        d.deletions,
        match d.commits {
            0 => String::new(),
            n => format!(", {n} commit(s)"),
        }
    );
    for file in d.files.iter().take(FILES_MAX) {
        out.push_str(&format!("- {}\n", file.path));
    }
    let shown = d.files.len().min(FILES_MAX) as u32;
    if d.files_total > shown {
        out.push_str(&format!("- e mais {}\n", d.files_total - shown));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::turn::TurnOutcome;
    use crate::worktree::diff::{FileDiff, FileStatus};

    fn digest(files: &[&str]) -> TurnDigest {
        TurnDigest {
            seq: 1,
            outcome: TurnOutcome::Completed,
            failure: None,
            started_at_ms: 0,
            ended_at_ms: 1,
            duration_ms: 1,
            prompt: Some("pedido curto".into()),
            diff: TurnDiff::Ok,
            files: files
                .iter()
                .map(|p| FileDiff {
                    path: p.to_string(),
                    old_path: None,
                    status: FileStatus::Modified,
                    insertions: Some(1),
                    deletions: Some(0),
                })
                .collect(),
            files_total: files.len() as u32,
            insertions: files.len() as u32,
            deletions: 0,
            commits: 0,
            cost_usd: None,
            tokens: None,
        }
    }

    #[test]
    fn nothing_to_pass_is_none() {
        assert_eq!(prompt(None, None, None), None);
        assert_eq!(prompt(None, Some("  "), Some("\n")), None);
    }

    #[test]
    fn carries_task_files_and_last_reply() {
        let d = digest(&["src/a.rs", "src/b.rs"]);
        let out = prompt(Some(&d), Some("arruma o login"), Some("faltou o teste")).unwrap();
        assert!(out.contains("## Pedido\narruma o login"));
        assert!(out.contains("2 arquivo(s), +2 −0"));
        assert!(out.contains("- src/a.rs\n- src/b.rs"));
        assert!(out.contains("## Onde o agente anterior parou\nfaltou o teste"));
    }

    #[test]
    fn falls_back_to_the_digest_prompt() {
        let d = digest(&[]);
        let out = prompt(Some(&d), None, Some("feito")).unwrap();
        assert!(out.contains("## Pedido\npedido curto"));
        assert!(!out.contains("## O que mudou"));
    }

    #[test]
    fn long_file_list_is_cut() {
        let names: Vec<String> = (0..40).map(|i| format!("f{i}.rs")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let out = prompt(Some(&digest(&refs)), None, None).unwrap();
        assert!(out.contains("- f29.rs\n- e mais 10"));
        assert!(!out.contains("f30.rs"));
    }

    #[test]
    fn secrets_are_redacted() {
        // Sintético: formato de chave, valor inventado.
        let key = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let out = prompt(None, Some(&format!("usa {key}")), Some(key)).unwrap();
        assert!(!out.contains(key), "{out}");
    }
}
