pub mod hooks_settings;
pub mod session;

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use portable_pty::CommandBuilder;

use crate::session::AgentRunnerKind;

pub trait AgentRunner: Send + Sync {
    fn kind(&self) -> AgentRunnerKind;

    fn build_command(
        &self,
        worktree_path: &Path,
        env: &HashMap<String, String>,
        hook_settings_path: &Path,
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

pub struct ClaudeCodeRunner;

impl AgentRunner for ClaudeCodeRunner {
    fn kind(&self) -> AgentRunnerKind {
        AgentRunnerKind::ClaudeCode
    }

    fn build_command(
        &self,
        worktree_path: &Path,
        env: &HashMap<String, String>,
        hook_settings_path: &Path,
    ) -> CommandBuilder {
        let mut cmd = CommandBuilder::new("claude");
        cmd.arg("--settings");
        cmd.arg(hook_settings_path);
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
        let cmd =
            ClaudeCodeRunner.build_command(Path::new("/wt"), &env, Path::new("/tmp/hooks.json"));
        let argv = argv_strings(&cmd);
        assert_eq!(argv, vec!["claude", "--settings", "/tmp/hooks.json"]);
    }

    #[test]
    fn build_command_sets_cwd_to_worktree() {
        let env = HashMap::new();
        let cmd =
            ClaudeCodeRunner.build_command(Path::new("/wt"), &env, Path::new("/tmp/hooks.json"));
        assert_eq!(cmd.get_cwd().map(OsStr::new), Some(OsStr::new("/wt")));
    }

    #[test]
    fn build_command_applies_env_allowlist() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        let cmd =
            ClaudeCodeRunner.build_command(Path::new("/wt"), &env, Path::new("/tmp/hooks.json"));
        assert_eq!(cmd.get_env("PATH"), Some(OsStr::new("/usr/bin")));
    }

    #[test]
    fn build_command_never_bypasses_permissions() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        let cmd =
            ClaudeCodeRunner.build_command(Path::new("/wt"), &env, Path::new("/tmp/hooks.json"));
        let argv = argv_strings(&cmd);
        for arg in &argv {
            assert_ne!(arg, "--dangerously-skip-permissions");
            assert_ne!(arg, "--permission-mode");
            assert_ne!(arg, "-p");
        }
    }
}
