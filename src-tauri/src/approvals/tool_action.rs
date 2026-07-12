use std::path::Path;

use serde_json::Value;

use super::tool_risk::{classify_command, classify_write};
use super::RiskLevel;
use crate::session::AgentRunnerKind;

const DESCRIPTION_MAX_CHARS: usize = 500;

const CLAUDE_WRITE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit"];
const CLAUDE_READ_ONLY_TOOLS: &[&str] = &[
    "Read",
    "Glob",
    "Grep",
    "LS",
    "NotebookRead",
    "TodoRead",
    "TodoWrite",
    "AskUserQuestion",
];
const CLAUDE_NETWORK_TOOLS: &[&str] = &["WebFetch", "WebSearch"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolAction {
    RunCommand { command: String },
    WriteFile { path: String },
    ReadOnly,
    Network,
    Unknown,
}

impl ToolAction {
    pub fn classify(&self, worktree_root: &Path) -> RiskLevel {
        match self {
            ToolAction::RunCommand { command } => classify_command(command, worktree_root),
            ToolAction::WriteFile { path } => classify_write(path, worktree_root),
            ToolAction::ReadOnly => RiskLevel::Green,
            ToolAction::Network => RiskLevel::Red,
            ToolAction::Unknown => RiskLevel::Yellow,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedToolUse {
    pub action: ToolAction,
    pub description: String,
}

pub fn normalize_tool_use(
    runner: &AgentRunnerKind,
    tool_name: &str,
    tool_input: Option<&Value>,
) -> NormalizedToolUse {
    let normalized = match runner {
        AgentRunnerKind::ClaudeCode => normalize_claude(tool_name, tool_input),
        AgentRunnerKind::Codex | AgentRunnerKind::Custom(_) => NormalizedToolUse {
            action: ToolAction::Unknown,
            description: tool_name.to_string(),
        },
    };
    NormalizedToolUse {
        action: normalized.action,
        description: truncate_chars(normalized.description, DESCRIPTION_MAX_CHARS),
    }
}

fn normalize_claude(tool_name: &str, tool_input: Option<&Value>) -> NormalizedToolUse {
    if tool_name == "Bash" {
        return match str_field(tool_input, "command") {
            Some(command) => NormalizedToolUse {
                action: ToolAction::RunCommand {
                    command: command.to_string(),
                },
                description: command.to_string(),
            },
            None => NormalizedToolUse {
                action: ToolAction::Unknown,
                description: "Bash (sem comando)".to_string(),
            },
        };
    }
    if CLAUDE_WRITE_TOOLS.contains(&tool_name) {
        return match str_field(tool_input, claude_write_path_key(tool_name)) {
            Some(path) => NormalizedToolUse {
                action: ToolAction::WriteFile {
                    path: path.to_string(),
                },
                description: format!("{tool_name} {path}"),
            },
            None => NormalizedToolUse {
                action: ToolAction::Unknown,
                description: format!("{tool_name} (sem caminho)"),
            },
        };
    }
    if CLAUDE_READ_ONLY_TOOLS.contains(&tool_name) {
        return NormalizedToolUse {
            action: ToolAction::ReadOnly,
            description: tool_name.to_string(),
        };
    }
    if CLAUDE_NETWORK_TOOLS.contains(&tool_name) {
        let description = match tool_name {
            "WebFetch" => match str_field(tool_input, "url") {
                Some(url) => format!("WebFetch {url}"),
                None => "WebFetch".to_string(),
            },
            _ => match str_field(tool_input, "query") {
                Some(query) => format!("WebSearch {query}"),
                None => "WebSearch".to_string(),
            },
        };
        return NormalizedToolUse {
            action: ToolAction::Network,
            description,
        };
    }
    NormalizedToolUse {
        action: ToolAction::Unknown,
        description: tool_name.to_string(),
    }
}

fn claude_write_path_key(tool_name: &str) -> &'static str {
    if tool_name == "NotebookEdit" {
        "notebook_path"
    } else {
        "file_path"
    }
}

fn str_field<'a>(input: Option<&'a Value>, key: &str) -> Option<&'a str> {
    input?.get(key)?.as_str()
}

fn truncate_chars(text: String, max: usize) -> String {
    if text.chars().count() > max {
        text.chars().take(max).collect()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn claude(tool: &str, input: Option<Value>) -> NormalizedToolUse {
        normalize_tool_use(&AgentRunnerKind::ClaudeCode, tool, input.as_ref())
    }

    #[test]
    fn bash_becomes_run_command() {
        let n = claude("Bash", Some(json!({"command": "ls -la"})));
        assert_eq!(
            n.action,
            ToolAction::RunCommand {
                command: "ls -la".into()
            }
        );
        assert_eq!(n.description, "ls -la");
    }

    #[test]
    fn bash_without_command_is_unknown() {
        let n = claude("Bash", Some(json!({})));
        assert_eq!(n.action, ToolAction::Unknown);
        assert_eq!(n.description, "Bash (sem comando)");
    }

    #[test]
    fn write_becomes_write_file() {
        let n = claude("Write", Some(json!({"file_path": "/wt/a.txt"})));
        assert_eq!(
            n.action,
            ToolAction::WriteFile {
                path: "/wt/a.txt".into()
            }
        );
        assert_eq!(n.description, "Write /wt/a.txt");
    }

    #[test]
    fn notebook_edit_uses_notebook_path() {
        let n = claude(
            "NotebookEdit",
            Some(json!({"notebook_path": "/wt/n.ipynb"})),
        );
        assert_eq!(
            n.action,
            ToolAction::WriteFile {
                path: "/wt/n.ipynb".into()
            }
        );
    }

    #[test]
    fn write_without_path_is_unknown() {
        let n = claude("Edit", None);
        assert_eq!(n.action, ToolAction::Unknown);
        assert_eq!(n.description, "Edit (sem caminho)");
    }

    #[test]
    fn read_only_tools_are_read_only() {
        for tool in CLAUDE_READ_ONLY_TOOLS {
            assert_eq!(claude(tool, None).action, ToolAction::ReadOnly);
        }
    }

    #[test]
    fn network_tools_are_network() {
        let n = claude("WebFetch", Some(json!({"url": "https://x.dev"})));
        assert_eq!(n.action, ToolAction::Network);
        assert_eq!(n.description, "WebFetch https://x.dev");
        let n = claude("WebSearch", Some(json!({"query": "rust"})));
        assert_eq!(n.action, ToolAction::Network);
        assert_eq!(n.description, "WebSearch rust");
    }

    #[test]
    fn unknown_tool_is_unknown() {
        let n = claude("SomethingNew", None);
        assert_eq!(n.action, ToolAction::Unknown);
        assert_eq!(n.description, "SomethingNew");
    }

    #[test]
    fn codex_and_custom_fall_back_to_unknown() {
        let n = normalize_tool_use(&AgentRunnerKind::Codex, "shell", None);
        assert_eq!(n.action, ToolAction::Unknown);
        let n = normalize_tool_use(&AgentRunnerKind::Custom("x".into()), "tool", None);
        assert_eq!(n.action, ToolAction::Unknown);
    }

    #[test]
    fn description_truncates_at_500_chars() {
        let long = "x".repeat(600);
        let n = claude("Bash", Some(json!({ "command": long })));
        assert_eq!(n.description.chars().count(), 500);
    }

    #[test]
    fn classify_read_only_is_green_and_network_red_and_unknown_yellow() {
        let root = PathBuf::from("/wt");
        assert_eq!(ToolAction::ReadOnly.classify(&root), RiskLevel::Green);
        assert_eq!(ToolAction::Network.classify(&root), RiskLevel::Red);
        assert_eq!(ToolAction::Unknown.classify(&root), RiskLevel::Yellow);
    }
}
