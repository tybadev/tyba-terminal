use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use super::{classify_risk, RiskLevel};

fn normalize_lexical(path: &Path) -> Option<PathBuf> {
    let mut stack: Vec<Component> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match stack.last() {
                Some(Component::Normal(_)) => {
                    stack.pop();
                }
                _ => return None,
            },
            other => stack.push(other),
        }
    }
    let mut result = PathBuf::new();
    for comp in stack {
        result.push(comp.as_os_str());
    }
    Some(result)
}

fn existing_prefix_canonical(path: &Path) -> Option<PathBuf> {
    let mut current = path;
    loop {
        if let Ok(canonical) = std::fs::canonicalize(current) {
            return Some(canonical);
        }
        current = current.parent()?;
    }
}

fn escapes_via_symlink(resolved: &Path, worktree_root: &Path) -> bool {
    let Ok(canonical_root) = std::fs::canonicalize(worktree_root) else {
        return true;
    };
    match existing_prefix_canonical(resolved) {
        Some(prefix) => !prefix.starts_with(&canonical_root),
        None => true,
    }
}

pub(super) fn classify_write(path: &str, worktree_root: &Path) -> RiskLevel {
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        worktree_root.join(path)
    };
    let Some(root) = normalize_lexical(worktree_root) else {
        return RiskLevel::Red;
    };
    let Some(resolved) = normalize_lexical(&candidate) else {
        return RiskLevel::Red;
    };
    if !resolved.starts_with(&root) {
        return RiskLevel::Red;
    }
    if escapes_via_symlink(&resolved, worktree_root) {
        return RiskLevel::Red;
    }
    RiskLevel::Green
}

const SYSTEM_READ_PREFIXES: &[&str] = &["/bin", "/sbin", "/usr/bin", "/usr/sbin", "/dev/null"];

fn path_candidates(command: &str) -> Vec<PathBuf> {
    command
        .split_whitespace()
        .map(|raw| {
            raw.trim_matches(|c| matches!(c, '"' | '\'' | '(' | ')' | ';' | '|' | '&'))
                .trim_start_matches(['>', '<'])
        })
        .filter(|token| token.starts_with('/') || token.starts_with("~/"))
        .map(|token| crate::session::expand_home(Path::new(token)))
        .collect()
}

fn bash_touches_outside(command: &str, worktree_root: &Path) -> bool {
    let Some(root) = normalize_lexical(worktree_root) else {
        return true;
    };
    path_candidates(command).into_iter().any(|candidate| {
        if SYSTEM_READ_PREFIXES
            .iter()
            .any(|p| candidate.starts_with(p))
        {
            return false;
        }
        match normalize_lexical(&candidate) {
            Some(resolved) => !resolved.starts_with(&root),
            None => true,
        }
    })
}

pub(super) fn classify_command(command: &str, worktree_root: &Path) -> RiskLevel {
    match classify_risk(command) {
        RiskLevel::Red => RiskLevel::Red,
        risk => {
            if bash_touches_outside(command, worktree_root) {
                RiskLevel::Red
            } else {
                risk
            }
        }
    }
}

pub fn classify_tool_use(
    tool_name: &str,
    tool_input: Option<&Value>,
    worktree_root: &Path,
) -> RiskLevel {
    super::tool_action::normalize_tool_use(
        &crate::session::AgentRunnerKind::ClaudeCode,
        tool_name,
        tool_input,
    )
    .action
    .classify(worktree_root)
}

pub fn describe_tool_use(tool_name: &str, tool_input: Option<&Value>) -> String {
    super::tool_action::normalize_tool_use(
        &crate::session::AgentRunnerKind::ClaudeCode,
        tool_name,
        tool_input,
    )
    .description
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn root() -> PathBuf {
        PathBuf::from("/home/user/wt")
    }

    fn real_root() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn bash_delega_para_classify_risk_verde_amarelo_vermelho() {
        assert_eq!(
            classify_tool_use("Bash", Some(&json!({ "command": "git status" })), &root()),
            RiskLevel::Green
        );
        assert_eq!(
            classify_tool_use("Bash", Some(&json!({ "command": "cargo build" })), &root()),
            RiskLevel::Yellow
        );
        assert_eq!(
            classify_tool_use(
                "Bash",
                Some(&json!({ "command": "git push origin feat/x" })),
                &root()
            ),
            RiskLevel::Red
        );
    }

    #[test]
    fn bash_sem_command_ou_tipo_errado_amarelo() {
        assert_eq!(
            classify_tool_use("Bash", Some(&json!({ "foo": "bar" })), &root()),
            RiskLevel::Yellow
        );
        assert_eq!(
            classify_tool_use("Bash", Some(&json!({ "command": 42 })), &root()),
            RiskLevel::Yellow
        );
        assert_eq!(classify_tool_use("Bash", None, &root()), RiskLevel::Yellow);
    }

    #[test]
    fn bash_escrita_fora_do_worktree_e_red() {
        let wt = real_root();
        assert_eq!(
            classify_tool_use(
                "Bash",
                Some(&json!({ "command": "touch /tmp/e2e-negado.txt" })),
                wt.path()
            ),
            RiskLevel::Red
        );
        assert_eq!(
            classify_tool_use(
                "Bash",
                Some(&json!({ "command": "echo oi > /etc/hosts" })),
                wt.path()
            ),
            RiskLevel::Red
        );
    }

    #[test]
    fn bash_leitura_de_segredo_fora_do_worktree_nao_e_green() {
        let wt = real_root();
        assert_eq!(
            classify_tool_use(
                "Bash",
                Some(&json!({ "command": "cat ~/.ssh/id_rsa" })),
                wt.path()
            ),
            RiskLevel::Red
        );
        assert_eq!(
            classify_tool_use(
                "Bash",
                Some(&json!({ "command": "cat /Users/outro/.aws/credentials" })),
                wt.path()
            ),
            RiskLevel::Red
        );
    }

    #[test]
    fn bash_sem_path_absoluto_mantem_classificacao_de_comando() {
        let wt = real_root();
        assert_eq!(
            classify_tool_use("Bash", Some(&json!({ "command": "git status" })), wt.path()),
            RiskLevel::Green
        );
        assert_eq!(
            classify_tool_use(
                "Bash",
                Some(&json!({ "command": "cargo build" })),
                wt.path()
            ),
            RiskLevel::Yellow
        );
        assert_eq!(
            classify_tool_use(
                "Bash",
                Some(&json!({ "command": "cat src/main.rs" })),
                wt.path()
            ),
            RiskLevel::Green
        );
    }

    #[test]
    fn bash_com_path_absoluto_dentro_do_worktree_mantem_verde() {
        let wt = real_root();
        let inside = wt.path().join("src/main.rs");
        assert_eq!(
            classify_tool_use(
                "Bash",
                Some(&json!({ "command": format!("cat {}", inside.display()) })),
                wt.path()
            ),
            RiskLevel::Green
        );
    }

    #[test]
    fn bash_binario_de_sistema_nao_escala_para_red() {
        let wt = real_root();
        assert_eq!(
            classify_tool_use(
                "Bash",
                Some(&json!({ "command": "/usr/bin/env cargo build" })),
                wt.path()
            ),
            RiskLevel::Yellow
        );
        assert_eq!(
            classify_tool_use(
                "Bash",
                Some(&json!({ "command": "cargo build 2> /dev/null" })),
                wt.path()
            ),
            RiskLevel::Yellow
        );
    }

    #[test]
    fn write_dentro_do_worktree_green() {
        let wt = real_root();
        assert_eq!(
            classify_tool_use(
                "Write",
                Some(&json!({ "file_path": "src/main.rs" })),
                wt.path()
            ),
            RiskLevel::Green
        );
        let absolute = wt.path().join("src/main.rs");
        assert_eq!(
            classify_tool_use(
                "Write",
                Some(&json!({ "file_path": absolute.to_str().unwrap() })),
                wt.path()
            ),
            RiskLevel::Green
        );
    }

    #[test]
    fn write_com_root_inexistente_red() {
        assert_eq!(
            classify_tool_use(
                "Write",
                Some(&json!({ "file_path": "src/main.rs" })),
                &root()
            ),
            RiskLevel::Red
        );
    }

    #[test]
    fn write_fora_absoluto_red() {
        assert_eq!(
            classify_tool_use(
                "Write",
                Some(&json!({ "file_path": "/etc/passwd" })),
                &root()
            ),
            RiskLevel::Red
        );
    }

    #[test]
    fn edit_com_parent_relativo_escapando_red() {
        assert_eq!(
            classify_tool_use(
                "Edit",
                Some(&json!({ "file_path": "../../etc/passwd" })),
                &root()
            ),
            RiskLevel::Red
        );
    }

    #[test]
    fn parent_que_permanece_dentro_green() {
        let wt = real_root();
        assert_eq!(
            classify_tool_use(
                "Edit",
                Some(&json!({ "file_path": "src/../lib/x.rs" })),
                wt.path()
            ),
            RiskLevel::Green
        );
    }

    #[test]
    fn notebook_edit_usa_notebook_path() {
        let wt = real_root();
        assert_eq!(
            classify_tool_use(
                "NotebookEdit",
                Some(&json!({ "notebook_path": "nb/a.ipynb" })),
                wt.path()
            ),
            RiskLevel::Green
        );
        assert_eq!(
            classify_tool_use(
                "NotebookEdit",
                Some(&json!({ "notebook_path": "/tmp/a.ipynb" })),
                &root()
            ),
            RiskLevel::Red
        );
    }

    #[test]
    fn write_path_ausente_ou_nao_string_yellow() {
        assert_eq!(
            classify_tool_use("Write", Some(&json!({ "file_path": 7 })), &root()),
            RiskLevel::Yellow
        );
        assert_eq!(classify_tool_use("Edit", None, &root()), RiskLevel::Yellow);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_no_worktree_apontando_para_fora_red() {
        let wt = real_root();
        let outside = TempDir::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), wt.path().join("evil")).unwrap();
        assert_eq!(
            classify_tool_use(
                "Write",
                Some(&json!({ "file_path": "evil/passwd" })),
                wt.path()
            ),
            RiskLevel::Red
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_interno_ao_worktree_green() {
        let wt = real_root();
        std::fs::create_dir(wt.path().join("real")).unwrap();
        std::os::unix::fs::symlink(wt.path().join("real"), wt.path().join("alias")).unwrap();
        assert_eq!(
            classify_tool_use(
                "Write",
                Some(&json!({ "file_path": "alias/x.rs" })),
                wt.path()
            ),
            RiskLevel::Green
        );
    }

    #[test]
    fn read_only_green() {
        for tool in [
            "Read",
            "Glob",
            "Grep",
            "LS",
            "NotebookRead",
            "TodoRead",
            "TodoWrite",
            "AskUserQuestion",
        ] {
            assert_eq!(
                classify_tool_use(tool, None, &root()),
                RiskLevel::Green,
                "{tool}"
            );
        }
    }

    #[test]
    fn rede_red() {
        assert_eq!(
            classify_tool_use("WebFetch", Some(&json!({ "url": "https://x" })), &root()),
            RiskLevel::Red
        );
        assert_eq!(
            classify_tool_use("WebSearch", Some(&json!({ "query": "rust" })), &root()),
            RiskLevel::Red
        );
    }

    #[test]
    fn tool_desconhecida_yellow() {
        assert_eq!(
            classify_tool_use("SomethingElse", None, &root()),
            RiskLevel::Yellow
        );
    }

    #[test]
    fn describe_bash() {
        assert_eq!(
            describe_tool_use("Bash", Some(&json!({ "command": "ls -la" }))),
            "ls -la"
        );
        assert_eq!(describe_tool_use("Bash", None), "Bash (sem comando)");
    }

    #[test]
    fn describe_escrita() {
        assert_eq!(
            describe_tool_use("Write", Some(&json!({ "file_path": "/path/x.rs" }))),
            "Write /path/x.rs"
        );
        assert_eq!(
            describe_tool_use(
                "NotebookEdit",
                Some(&json!({ "notebook_path": "nb/a.ipynb" }))
            ),
            "NotebookEdit nb/a.ipynb"
        );
        assert_eq!(describe_tool_use("Edit", None), "Edit (sem caminho)");
    }

    #[test]
    fn describe_rede() {
        assert_eq!(
            describe_tool_use("WebFetch", Some(&json!({ "url": "https://x" }))),
            "WebFetch https://x"
        );
        assert_eq!(
            describe_tool_use("WebSearch", Some(&json!({ "query": "rust ownership" }))),
            "WebSearch rust ownership"
        );
    }

    #[test]
    fn describe_desconhecida() {
        assert_eq!(describe_tool_use("SomethingElse", None), "SomethingElse");
    }

    #[test]
    fn describe_trunca_em_500() {
        let big = "a".repeat(2000);
        let out = describe_tool_use("Bash", Some(&json!({ "command": big })));
        assert_eq!(out.chars().count(), 500);
    }
}
