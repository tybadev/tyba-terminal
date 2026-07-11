use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use super::{classify_risk, RiskLevel};

const WRITE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit"];
const READ_ONLY_TOOLS: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "LS",
    "NotebookRead",
    "TodoRead",
    "TodoWrite",
    "AskUserQuestion",
];
const NETWORK_TOOLS: &[&str] = &["WebFetch", "WebSearch"];

fn write_path_key(tool_name: &str) -> &'static str {
    if tool_name == "NotebookEdit" {
        "notebook_path"
    } else {
        "file_path"
    }
}

fn str_field<'a>(input: Option<&'a Value>, key: &str) -> Option<&'a str> {
    input?.get(key)?.as_str()
}

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

fn classify_write(path: &str, worktree_root: &Path) -> RiskLevel {
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
    if resolved.starts_with(&root) {
        RiskLevel::Green
    } else {
        RiskLevel::Red
    }
}

pub fn classify_tool_use(
    tool_name: &str,
    tool_input: Option<&Value>,
    worktree_root: &Path,
) -> RiskLevel {
    if tool_name == "Bash" {
        return match str_field(tool_input, "command") {
            Some(command) => classify_risk(command),
            None => RiskLevel::Yellow,
        };
    }
    if WRITE_TOOLS.contains(&tool_name) {
        return match str_field(tool_input, write_path_key(tool_name)) {
            Some(path) => classify_write(path, worktree_root),
            None => RiskLevel::Yellow,
        };
    }
    if READ_ONLY_TOOLS.contains(&tool_name) {
        return RiskLevel::Green;
    }
    if NETWORK_TOOLS.contains(&tool_name) {
        return RiskLevel::Red;
    }
    RiskLevel::Yellow
}

fn truncate_500(text: String) -> String {
    if text.chars().count() > 500 {
        text.chars().take(500).collect()
    } else {
        text
    }
}

pub fn describe_tool_use(tool_name: &str, tool_input: Option<&Value>) -> String {
    let raw = if tool_name == "Bash" {
        match str_field(tool_input, "command") {
            Some(command) => command.to_string(),
            None => "Bash (sem comando)".to_string(),
        }
    } else if WRITE_TOOLS.contains(&tool_name) {
        match str_field(tool_input, write_path_key(tool_name)) {
            Some(path) => format!("{tool_name} {path}"),
            None => format!("{tool_name} (sem caminho)"),
        }
    } else if tool_name == "WebFetch" {
        match str_field(tool_input, "url") {
            Some(url) => format!("WebFetch {url}"),
            None => "WebFetch".to_string(),
        }
    } else if tool_name == "WebSearch" {
        match str_field(tool_input, "query") {
            Some(query) => format!("WebSearch {query}"),
            None => "WebSearch".to_string(),
        }
    } else {
        tool_name.to_string()
    };
    truncate_500(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn root() -> PathBuf {
        PathBuf::from("/home/user/wt")
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
    fn write_dentro_do_worktree_green() {
        assert_eq!(
            classify_tool_use(
                "Write",
                Some(&json!({ "file_path": "src/main.rs" })),
                &root()
            ),
            RiskLevel::Green
        );
        assert_eq!(
            classify_tool_use(
                "Write",
                Some(&json!({ "file_path": "/home/user/wt/src/main.rs" })),
                &root()
            ),
            RiskLevel::Green
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
        assert_eq!(
            classify_tool_use(
                "Edit",
                Some(&json!({ "file_path": "src/../lib/x.rs" })),
                &root()
            ),
            RiskLevel::Green
        );
    }

    #[test]
    fn notebook_edit_usa_notebook_path() {
        assert_eq!(
            classify_tool_use(
                "NotebookEdit",
                Some(&json!({ "notebook_path": "nb/a.ipynb" })),
                &root()
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

    #[test]
    fn symlink_dentro_do_worktree_e_green_limitacao_lexica_conhecida_da_f5() {
        assert_eq!(
            classify_tool_use(
                "Write",
                Some(&json!({ "file_path": "src/link-para-fora.rs" })),
                &root()
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
