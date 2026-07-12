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
    WriteFiles { paths: Vec<String> },
    ReadOnly,
    Network,
    Unknown,
}

impl ToolAction {
    pub fn classify(&self, worktree_root: &Path) -> RiskLevel {
        match self {
            ToolAction::RunCommand { command } => classify_command(command, worktree_root),
            ToolAction::WriteFile { path } => classify_write(path, worktree_root),
            ToolAction::WriteFiles { paths } => {
                if paths.is_empty() {
                    return RiskLevel::Yellow;
                }
                paths
                    .iter()
                    .map(|path| classify_write(path, worktree_root))
                    .max_by_key(|risk| match risk {
                        RiskLevel::Green => 0,
                        RiskLevel::Yellow => 1,
                        RiskLevel::Red => 2,
                    })
                    .unwrap_or(RiskLevel::Yellow)
            }
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
        AgentRunnerKind::Codex => normalize_codex(tool_name, tool_input),
        AgentRunnerKind::Custom(_) => NormalizedToolUse {
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

fn normalize_codex(tool_name: &str, tool_input: Option<&Value>) -> NormalizedToolUse {
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
    if tool_name == "apply_patch" {
        return match str_field(tool_input, "command") {
            Some(patch) => {
                let paths = apply_patch_paths(patch);
                let description = if paths.is_empty() {
                    "apply_patch (sem arquivos)".to_string()
                } else {
                    format!("apply_patch {}", paths.join(", "))
                };
                NormalizedToolUse {
                    action: ToolAction::WriteFiles { paths },
                    description,
                }
            }
            None => NormalizedToolUse {
                action: ToolAction::Unknown,
                description: "apply_patch (sem patch)".to_string(),
            },
        };
    }
    if tool_name == "web_search" {
        let description = match str_field(tool_input, "query") {
            Some(query) => format!("web_search {query}"),
            None => "web_search".to_string(),
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

const APPLY_PATCH_PATH_PREFIXES: &[&str] = &[
    "*** Add File: ",
    "*** Update File: ",
    "*** Delete File: ",
    "*** Move to: ",
];

fn apply_patch_paths(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            APPLY_PATCH_PATH_PREFIXES
                .iter()
                .find_map(|prefix| trimmed.strip_prefix(prefix))
        })
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect()
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

    fn codex(tool: &str, input: Option<Value>) -> NormalizedToolUse {
        normalize_tool_use(&AgentRunnerKind::Codex, tool, input.as_ref())
    }

    #[test]
    fn custom_runner_falls_back_to_unknown() {
        let n = normalize_tool_use(&AgentRunnerKind::Custom("x".into()), "tool", None);
        assert_eq!(n.action, ToolAction::Unknown);
    }

    #[test]
    fn codex_bash_becomes_run_command_like_claude() {
        let n = codex("Bash", Some(json!({"command": "git status"})));
        assert_eq!(
            n.action,
            ToolAction::RunCommand {
                command: "git status".into()
            }
        );
        assert_eq!(n.description, "git status");
    }

    #[test]
    fn codex_apply_patch_extracts_paths_from_patch_envelope() {
        let patch = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** Add File: docs/b.md\n+hello\n*** Delete File: old.txt\n*** End Patch";
        let n = codex("apply_patch", Some(json!({ "command": patch })));
        assert_eq!(
            n.action,
            ToolAction::WriteFiles {
                paths: vec!["src/a.rs".into(), "docs/b.md".into(), "old.txt".into()]
            }
        );
        assert_eq!(n.description, "apply_patch src/a.rs, docs/b.md, old.txt");
    }

    #[test]
    fn codex_apply_patch_move_to_counts_as_write_target() {
        let patch =
            "*** Begin Patch\n*** Update File: a.rs\n*** Move to: ../fora/b.rs\n*** End Patch";
        let n = codex("apply_patch", Some(json!({ "command": patch })));
        assert_eq!(
            n.action,
            ToolAction::WriteFiles {
                paths: vec!["a.rs".into(), "../fora/b.rs".into()]
            }
        );
    }

    #[test]
    fn codex_apply_patch_without_paths_is_yellow() {
        let n = codex(
            "apply_patch",
            Some(json!({"command": "*** Begin Patch\n*** End Patch"})),
        );
        assert_eq!(n.action, ToolAction::WriteFiles { paths: vec![] });
        let root = PathBuf::from("/wt");
        assert_eq!(n.action.classify(&root), RiskLevel::Yellow);
    }

    #[test]
    fn codex_web_search_is_network() {
        let n = codex("web_search", Some(json!({"query": "rust"})));
        assert_eq!(n.action, ToolAction::Network);
        assert_eq!(n.description, "web_search rust");
    }

    #[test]
    fn codex_mcp_and_unknown_tools_are_yellow() {
        let n = codex("mcp__server__tool", Some(json!({"x": 1})));
        assert_eq!(n.action, ToolAction::Unknown);
        assert_eq!(n.description, "mcp__server__tool");
        assert_eq!(codex("spawn_agent", None).action, ToolAction::Unknown);
    }

    #[test]
    fn write_files_classifies_worst_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let inside = ToolAction::WriteFiles {
            paths: vec!["a.rs".into(), "b/c.rs".into()],
        };
        assert_eq!(inside.classify(&root), RiskLevel::Green);
        let escaping = ToolAction::WriteFiles {
            paths: vec!["a.rs".into(), "../fora.rs".into()],
        };
        assert_eq!(escaping.classify(&root), RiskLevel::Red);
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
