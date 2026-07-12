pub mod codex_hooks;
pub mod hooks_settings;
pub mod session;

#[cfg(test)]
mod codex_e2e_tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use portable_pty::CommandBuilder;

use crate::session::AgentRunnerKind;

pub struct HookSetup {
    pub settings_path: PathBuf,
    pub hook_command: String,
}

pub trait AgentRunner: Send + Sync {
    fn kind(&self) -> AgentRunnerKind;

    fn build_command(
        &self,
        worktree_path: &Path,
        env: &HashMap<String, String>,
        hooks: &HookSetup,
    ) -> CommandBuilder;

    fn submit_delay(&self) -> Duration {
        Duration::from_millis(50)
    }

    fn supports_hooks(&self) -> bool {
        false
    }

    fn needs_network(&self) -> bool {
        false
    }
}

pub fn runner_binary(kind: &AgentRunnerKind) -> Option<&'static str> {
    match kind {
        AgentRunnerKind::ClaudeCode => Some("claude"),
        AgentRunnerKind::Codex => Some("codex"),
        AgentRunnerKind::Custom(_) => None,
    }
}

pub fn binary_available(kind: &AgentRunnerKind) -> bool {
    let Some(name) = runner_binary(kind) else {
        return false;
    };
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| is_executable(&dir.join(name)))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

pub struct ClaudeCodeRunner;

impl AgentRunner for ClaudeCodeRunner {
    fn kind(&self) -> AgentRunnerKind {
        AgentRunnerKind::ClaudeCode
    }

    fn build_command(
        &self,
        worktree_path: &Path,
        env: &HashMap<String, String>,
        hooks: &HookSetup,
    ) -> CommandBuilder {
        let mut cmd = CommandBuilder::new("claude");
        cmd.arg("--settings");
        cmd.arg(&hooks.settings_path);
        cmd.cwd(worktree_path);
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd
    }

    fn supports_hooks(&self) -> bool {
        true
    }

    fn needs_network(&self) -> bool {
        true
    }
}

pub struct CodexRunner;

impl AgentRunner for CodexRunner {
    fn kind(&self) -> AgentRunnerKind {
        AgentRunnerKind::Codex
    }

    fn build_command(
        &self,
        worktree_path: &Path,
        env: &HashMap<String, String>,
        hooks: &HookSetup,
    ) -> CommandBuilder {
        let mut cmd = CommandBuilder::new("codex");
        cmd.arg("--sandbox");
        cmd.arg("workspace-write");
        cmd.arg("--ask-for-approval");
        cmd.arg("on-request");
        for over in codex_hooks::codex_config_overrides(&hooks.hook_command) {
            cmd.arg("--config");
            cmd.arg(over);
        }
        cmd.cwd(worktree_path);
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd
    }

    fn supports_hooks(&self) -> bool {
        true
    }

    fn needs_network(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn argv_strings(cmd: &CommandBuilder) -> Vec<String> {
        cmd.get_argv()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn hooks() -> HookSetup {
        HookSetup {
            settings_path: PathBuf::from("/tmp/hooks.json"),
            hook_command: "'/usr/bin/tyba' _hook".to_string(),
        }
    }

    #[test]
    fn runner_binary_maps_kinds() {
        assert_eq!(runner_binary(&AgentRunnerKind::ClaudeCode), Some("claude"));
        assert_eq!(runner_binary(&AgentRunnerKind::Codex), Some("codex"));
        assert_eq!(runner_binary(&AgentRunnerKind::Custom("x".into())), None);
    }

    #[test]
    fn custom_runner_binary_is_never_available() {
        assert!(!binary_available(&AgentRunnerKind::Custom("x".into())));
    }

    #[test]
    fn claude_kind_is_claude_code() {
        assert!(matches!(
            ClaudeCodeRunner.kind(),
            AgentRunnerKind::ClaudeCode
        ));
    }

    #[test]
    fn claude_supports_hooks() {
        assert!(ClaudeCodeRunner.supports_hooks());
    }

    #[test]
    fn claude_needs_network() {
        assert!(ClaudeCodeRunner.needs_network());
    }

    #[test]
    fn default_submit_delay_is_50ms() {
        assert_eq!(ClaudeCodeRunner.submit_delay(), Duration::from_millis(50));
    }

    #[test]
    fn build_command_uses_settings_flag_with_hook_path() {
        let env = HashMap::new();
        let cmd = ClaudeCodeRunner.build_command(Path::new("/wt"), &env, &hooks());
        let argv = argv_strings(&cmd);
        assert_eq!(argv, vec!["claude", "--settings", "/tmp/hooks.json"]);
    }

    #[test]
    fn build_command_sets_cwd_to_worktree() {
        let env = HashMap::new();
        let cmd = ClaudeCodeRunner.build_command(Path::new("/wt"), &env, &hooks());
        assert_eq!(cmd.get_cwd().map(OsStr::new), Some(OsStr::new("/wt")));
    }

    #[test]
    fn build_command_applies_env_allowlist() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        let cmd = ClaudeCodeRunner.build_command(Path::new("/wt"), &env, &hooks());
        assert_eq!(cmd.get_env("PATH"), Some(OsStr::new("/usr/bin")));
    }

    #[test]
    fn build_command_never_bypasses_permissions() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        let cmd = ClaudeCodeRunner.build_command(Path::new("/wt"), &env, &hooks());
        let argv = argv_strings(&cmd);
        for arg in &argv {
            assert_ne!(arg, "--dangerously-skip-permissions");
            assert_ne!(arg, "--permission-mode");
            assert_ne!(arg, "-p");
        }
    }

    #[test]
    fn codex_kind_supports_hooks_and_network() {
        assert!(matches!(CodexRunner.kind(), AgentRunnerKind::Codex));
        assert!(CodexRunner.supports_hooks());
        assert!(CodexRunner.needs_network());
    }

    #[test]
    fn codex_command_keeps_sandbox_on_and_tui_silent() {
        let env = HashMap::new();
        let cmd = CodexRunner.build_command(Path::new("/wt"), &env, &hooks());
        let argv = argv_strings(&cmd);
        assert_eq!(argv[0], "codex");
        let sandbox = argv.iter().position(|a| a == "--sandbox").unwrap();
        assert_eq!(argv[sandbox + 1], "workspace-write");
        let approval = argv.iter().position(|a| a == "--ask-for-approval").unwrap();
        assert_eq!(argv[approval + 1], "on-request");
        assert_eq!(cmd.get_cwd().map(OsStr::new), Some(OsStr::new("/wt")));
    }

    #[test]
    fn codex_command_injects_hooks_and_trust_via_config_overrides() {
        let env = HashMap::new();
        let cmd = CodexRunner.build_command(Path::new("/wt"), &env, &hooks());
        let argv = argv_strings(&cmd);
        let overrides: Vec<&String> = argv
            .iter()
            .enumerate()
            .filter(|(i, _)| *i > 0 && argv[i - 1] == "--config")
            .map(|(_, a)| a)
            .collect();
        assert_eq!(overrides.len(), 5);
        for key in [
            "hooks.PreToolUse=",
            "hooks.PermissionRequest=",
            "hooks.SessionStart=",
            "hooks.Stop=",
            "hooks.state=",
        ] {
            assert!(
                overrides.iter().any(|o| o.starts_with(key)),
                "faltou override {key}"
            );
        }
        assert!(overrides
            .iter()
            .all(|o| !o.starts_with("hooks.state=") || o.contains("trusted_hash")));
    }

    #[test]
    fn codex_command_never_bypasses_sandbox_or_hook_trust() {
        let env = HashMap::new();
        let cmd = CodexRunner.build_command(Path::new("/wt"), &env, &hooks());
        for arg in argv_strings(&cmd) {
            assert_ne!(arg, "--dangerously-bypass-approvals-and-sandbox");
            assert_ne!(arg, "--dangerously-bypass-hook-trust");
            assert_ne!(arg, "danger-full-access");
        }
    }
}
