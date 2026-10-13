use std::path::Path;

use serde_json::Value;

use super::tool_risk::{classify_command, classify_write};
use super::{red, Classification, RedReason, RiskLevel};
use crate::session::AgentRunnerKind;

/// Corte da chave do "sempre permitir" da sessão (Codex) e do resumo de uma
/// linha (dica de status, notificação do SO). NÃO é o corte do texto exibido
/// no pedido — ver [`DISPLAY_MAX_BYTES`].
const DESCRIPTION_MAX_CHARS: usize = 500;

/// Limite de transporte do texto exibido num pedido (invariante 6 do fluxo
/// híbrido): vermelho se decide lendo tudo, e 64 KiB cobre qualquer comando
/// que alguém leia. Acima disso o texto vai cortado com marcador visível.
pub const DISPLAY_MAX_BYTES: usize = 64 * 1024;

const SUBAGENT_SPAWN_RISK_TOOLS_GATED_BY_PRETOOLUSE: RiskLevel = RiskLevel::Green;

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
    // Claude Code 2.1.x: leitura/consulta pura, sem fs/rede/exec.
    // BashOutput só lê o buffer de um shell em background; ToolSearch só
    // consulta schemas de tools. KillShell/KillBash matam processo — ficam de fora.
    "ToolSearch",
    "BashOutput",
];
const CLAUDE_NETWORK_TOOLS: &[&str] = &["WebFetch", "WebSearch"];
const CLAUDE_SUBAGENT_TOOLS: &[&str] = &["Agent", "Task"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolAction {
    RunCommand {
        command: String,
    },
    WriteFile {
        path: String,
    },
    WriteFiles {
        paths: Vec<String>,
    },
    ReadOnly,
    Network,
    Subagent {
        agent_type: Option<String>,
        description: Option<String>,
    },
    Unknown,
}

impl ToolAction {
    /// Nível de risco e, no vermelho, o porquê.
    pub fn classify(&self, worktree_root: &Path) -> Classification {
        match self {
            ToolAction::RunCommand { command } => classify_command(command, worktree_root),
            ToolAction::WriteFile { path } => classify_write(path, worktree_root),
            ToolAction::WriteFiles { paths } => {
                if paths.is_empty() {
                    return (RiskLevel::Yellow, None);
                }
                // O pior caminho decide; no empate fica o primeiro (o motivo
                // aponta para o primeiro arquivo que fez o patch ser vermelho).
                let mut worst: Option<Classification> = None;
                for classification in paths.iter().map(|path| classify_write(path, worktree_root)) {
                    let rank = |risk: RiskLevel| match risk {
                        RiskLevel::Green => 0,
                        RiskLevel::Yellow => 1,
                        RiskLevel::Red => 2,
                    };
                    if worst
                        .as_ref()
                        .is_none_or(|(risk, _)| rank(classification.0) > rank(*risk))
                    {
                        worst = Some(classification);
                    }
                }
                worst.unwrap_or((RiskLevel::Yellow, None))
            }
            ToolAction::ReadOnly => (RiskLevel::Green, None),
            ToolAction::Network => red(RedReason::WebAccess),
            ToolAction::Subagent { .. } => (SUBAGENT_SPAWN_RISK_TOOLS_GATED_BY_PRETOOLUSE, None),
            ToolAction::Unknown => (RiskLevel::Yellow, None),
        }
    }
}

/// Uma chamada de ferramenta normalizada.
///
/// `description` é o texto **inteiro**, sem corte nenhum — é o que a recusa do
/// core ([`super::is_refused_by_core`]) precisa ler: um push para main
/// escondido depois do corte não pode escapar. Os textos cortados saem dele:
/// [`NormalizedToolUse::display_text`] (pedido e histórico) e
/// [`NormalizedToolUse::allowlist_key`] / [`NormalizedToolUse::summary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedToolUse {
    pub action: ToolAction,
    pub description: String,
}

impl NormalizedToolUse {
    /// Texto exibido no pedido e gravado no histórico: inteiro até
    /// [`DISPLAY_MAX_BYTES`], com marcador de corte acima disso.
    pub fn display_text(&self) -> String {
        display_text(&self.description)
    }

    /// Chave do "sempre permitir" da sessão (Codex). Mesma de antes do fluxo
    /// híbrido: os primeiros 500 caracteres.
    pub fn allowlist_key(&self) -> String {
        allowlist_key(&self.description)
    }

    /// Resumo de uma linha para dica de status e notificação do SO — onde o
    /// texto inteiro não cabe nem é lido. Mesmo corte da chave.
    pub fn summary(&self) -> String {
        truncate_chars(self.description.clone(), DESCRIPTION_MAX_CHARS)
    }
}

/// Ver [`NormalizedToolUse::allowlist_key`].
pub fn allowlist_key(text: &str) -> String {
    truncate_chars(text.to_string(), DESCRIPTION_MAX_CHARS)
}

/// Ver [`NormalizedToolUse::display_text`]. O corte cai numa fronteira de
/// caractere, e o marcador diz quantos bytes ficaram de fora — sem texto de
/// idioma, porque quem traduz é a interface.
pub fn display_text(text: &str) -> String {
    if text.len() <= DISPLAY_MAX_BYTES {
        return text.to_string();
    }
    let mut cut = DISPLAY_MAX_BYTES;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let omitted = text.len() - cut;
    format!("{}{}", &text[..cut], truncation_marker(omitted))
}

/// Marcador visível do corte do texto exibido.
pub fn truncation_marker(omitted_bytes: usize) -> String {
    format!("\n…[+{omitted_bytes} bytes]")
}

pub fn normalize_tool_use(
    runner: &AgentRunnerKind,
    tool_name: &str,
    tool_input: Option<&Value>,
) -> NormalizedToolUse {
    match runner {
        AgentRunnerKind::ClaudeCode => normalize_claude(tool_name, tool_input),
        AgentRunnerKind::Codex => normalize_codex(tool_name, tool_input),
        AgentRunnerKind::Custom(_) => NormalizedToolUse {
            action: ToolAction::Unknown,
            description: tool_name.to_string(),
        },
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
    if CLAUDE_SUBAGENT_TOOLS.contains(&tool_name) {
        let agent_type = str_field(tool_input, "subagent_type").map(str::to_string);
        let description = str_field(tool_input, "description")
            .map(|d| truncate_chars(d.to_string(), DESCRIPTION_MAX_CHARS));
        let label = subagent_label(agent_type.as_deref(), description.as_deref());
        return NormalizedToolUse {
            action: ToolAction::Subagent {
                agent_type,
                description,
            },
            description: label,
        };
    }
    NormalizedToolUse {
        action: ToolAction::Unknown,
        description: tool_name.to_string(),
    }
}

fn subagent_label(agent_type: Option<&str>, description: Option<&str>) -> String {
    let kind = agent_type
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("desconhecido");
    match description.map(str::trim).filter(|s| !s.is_empty()) {
        Some(what) => format!("Subagente {kind}: {what}"),
        None => format!("Subagente {kind}"),
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
    if CODEX_READ_ONLY_TOOLS.contains(&tool_name) {
        return NormalizedToolUse {
            action: ToolAction::ReadOnly,
            description: tool_name.to_string(),
        };
    }
    NormalizedToolUse {
        action: ToolAction::Unknown,
        description: tool_name.to_string(),
    }
}

const CODEX_READ_ONLY_TOOLS: &[&str] = &["update_plan", "view_image"];

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
    fn tool_search_and_bash_output_are_read_only() {
        assert_eq!(claude("ToolSearch", None).action, ToolAction::ReadOnly);
        assert_eq!(claude("BashOutput", None).action, ToolAction::ReadOnly);
    }

    #[test]
    fn kill_shell_is_not_read_only() {
        assert_eq!(claude("KillShell", None).action, ToolAction::Unknown);
        assert_eq!(claude("KillBash", None).action, ToolAction::Unknown);
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
    fn agent_tool_becomes_subagent_with_fields() {
        let n = claude(
            "Agent",
            Some(json!({"subagent_type": "code-reviewer", "description": "revisar diff"})),
        );
        assert_eq!(
            n.action,
            ToolAction::Subagent {
                agent_type: Some("code-reviewer".into()),
                description: Some("revisar diff".into()),
            }
        );
        assert_eq!(n.description, "Subagente code-reviewer: revisar diff");
    }

    #[test]
    fn task_tool_is_also_a_subagent() {
        let n = claude(
            "Task",
            Some(json!({"subagent_type": "explorer", "description": "mapear repo"})),
        );
        assert_eq!(
            n.action,
            ToolAction::Subagent {
                agent_type: Some("explorer".into()),
                description: Some("mapear repo".into()),
            }
        );
        assert_eq!(n.description, "Subagente explorer: mapear repo");
    }

    #[test]
    fn subagent_without_description_falls_back_to_type_only() {
        let n = claude("Agent", Some(json!({"subagent_type": "explorer"})));
        assert_eq!(
            n.action,
            ToolAction::Subagent {
                agent_type: Some("explorer".into()),
                description: None,
            }
        );
        assert_eq!(n.description, "Subagente explorer");
    }

    #[test]
    fn subagent_without_type_uses_unknown_placeholder() {
        let n = claude("Agent", Some(json!({"description": "fazer algo"})));
        assert_eq!(
            n.action,
            ToolAction::Subagent {
                agent_type: None,
                description: Some("fazer algo".into()),
            }
        );
        assert_eq!(n.description, "Subagente desconhecido: fazer algo");
    }

    #[test]
    fn subagent_without_any_input_is_still_subagent() {
        let n = claude("Agent", None);
        assert_eq!(
            n.action,
            ToolAction::Subagent {
                agent_type: None,
                description: None,
            }
        );
        assert_eq!(n.description, "Subagente desconhecido");
    }

    #[test]
    fn subagent_spawn_is_green_with_tools_gated_by_pretooluse() {
        let root = PathBuf::from("/wt");
        let action = ToolAction::Subagent {
            agent_type: Some("x".into()),
            description: None,
        };
        assert_eq!(action.classify(&root).0, RiskLevel::Green);
    }

    /// A descrição do subagente é cortada em 500 antes do rótulo; a chave da
    /// allowlist corta o rótulo inteiro em 500, como antes.
    #[test]
    fn subagent_description_truncates_at_500_chars() {
        let long = "z".repeat(600);
        let n = claude(
            "Agent",
            Some(json!({"subagent_type": "explorer", "description": long})),
        );
        assert_eq!(
            n.description,
            format!("Subagente explorer: {}", "z".repeat(500))
        );
        assert_eq!(n.allowlist_key().chars().count(), 500);
    }

    #[test]
    fn subagent_action_description_caps_at_500_chars() {
        let long = "z".repeat(600);
        let n = claude(
            "Agent",
            Some(json!({"subagent_type": "explorer", "description": long})),
        );
        match n.action {
            ToolAction::Subagent { description, .. } => {
                assert_eq!(description.unwrap().chars().count(), 500);
            }
            other => panic!("esperava Subagent, veio {other:?}"),
        }
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
        assert_eq!(n.action.classify(&root).0, RiskLevel::Yellow);
    }

    #[test]
    fn codex_web_search_is_network() {
        let n = codex("web_search", Some(json!({"query": "rust"})));
        assert_eq!(n.action, ToolAction::Network);
        assert_eq!(n.description, "web_search rust");
    }

    #[test]
    fn codex_plan_and_image_tools_are_read_only() {
        let root = PathBuf::from("/wt");
        for tool in CODEX_READ_ONLY_TOOLS {
            let n = codex(tool, Some(json!({"plan": []})));
            assert_eq!(n.action, ToolAction::ReadOnly, "{tool}");
            assert_eq!(n.action.classify(&root).0, RiskLevel::Green, "{tool}");
        }
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
        assert_eq!(inside.classify(&root).0, RiskLevel::Green);
        let escaping = ToolAction::WriteFiles {
            paths: vec!["a.rs".into(), "../fora.rs".into()],
        };
        assert_eq!(escaping.classify(&root).0, RiskLevel::Red);
    }

    /// A chave do "sempre permitir" continua a de antes: 500 caracteres.
    #[test]
    fn allowlist_key_keeps_the_500_char_cut() {
        let long = "x".repeat(600);
        let n = claude("Bash", Some(json!({ "command": long })));
        assert_eq!(n.allowlist_key().chars().count(), 500);
        assert_eq!(n.summary().chars().count(), 500);
    }

    /// CA-21: o texto exibido não tem mais o corte de 500 — 10 KiB chega
    /// inteiro, e a descrição guarda o texto inteiro.
    #[test]
    fn display_text_of_10_kib_arrives_whole() {
        let command = format!("echo {}", "a".repeat(10 * 1024));
        let n = claude("Bash", Some(json!({ "command": command.clone() })));
        assert_eq!(n.description, command);
        assert_eq!(n.display_text(), command);
    }

    /// CA-21: acima de 64 KiB, corte com marcador visível dizendo quanto
    /// ficou de fora.
    #[test]
    fn display_text_of_100_kib_is_cut_with_marker() {
        let command = "b".repeat(100 * 1024);
        let n = claude("Bash", Some(json!({ "command": command.clone() })));
        assert_eq!(n.description, command, "descrição fica inteira");
        let shown = n.display_text();
        let marker = truncation_marker(100 * 1024 - DISPLAY_MAX_BYTES);
        assert!(shown.ends_with(&marker), "marcador no fim");
        assert_eq!(shown.len(), DISPLAY_MAX_BYTES + marker.len());
        assert!(command.starts_with(&shown[..DISPLAY_MAX_BYTES]));
    }

    #[test]
    fn display_text_exactly_at_the_limit_has_no_marker() {
        let text = "c".repeat(DISPLAY_MAX_BYTES);
        assert_eq!(display_text(&text), text);
    }

    /// O corte nunca parte um caractere de vários bytes.
    #[test]
    fn display_text_cuts_on_a_char_boundary() {
        let text = format!("x{}", "é".repeat(DISPLAY_MAX_BYTES));
        let shown = display_text(&text);
        let body = shown.split('\n').next().unwrap();
        assert!(body.len() <= DISPLAY_MAX_BYTES);
        assert!(text.starts_with(body));
        assert!(shown.contains("…[+"));
    }

    #[test]
    fn classify_read_only_is_green_and_network_red_and_unknown_yellow() {
        let root = PathBuf::from("/wt");
        assert_eq!(ToolAction::ReadOnly.classify(&root).0, RiskLevel::Green);
        assert_eq!(ToolAction::Network.classify(&root).0, RiskLevel::Red);
        assert_eq!(ToolAction::Unknown.classify(&root).0, RiskLevel::Yellow);
    }
}
