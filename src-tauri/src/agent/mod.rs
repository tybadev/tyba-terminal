//! Runners de agente (Fase 4).
//!
//! Princípio #2: spawn de agente SEMPRE atrás da trait Sandbox.
//! Princípio #4: ações vermelhas (push, sudo, rede, rm -rf, escrita
//! fora do worktree) nunca são auto-aprovadas — hard-coded no runner.

use std::collections::HashMap;
use std::path::Path;

use portable_pty::CommandBuilder;

use crate::session::AgentRunnerKind;

/// Um runner sabe montar o comando de um agente e interpretar seu output.
pub trait AgentRunner: Send + Sync {
    fn kind(&self) -> AgentRunnerKind;

    /// Monta o comando do agente para rodar dentro do worktree.
    /// `env` é a allowlist filtrada do `.tyba/config` — nunca o env
    /// completo do usuário (princípio #6).
    fn build_command(
        &self,
        worktree_path: &Path,
        prompt: &str,
        env: &HashMap<String, String>,
    ) -> CommandBuilder;
}

/// Claude Code via `--output-format stream-json --include-partial-messages`.
/// Eventos estruturados no stdout: zero scraping de ANSI para status.
pub struct ClaudeCodeRunner;

impl AgentRunner for ClaudeCodeRunner {
    fn kind(&self) -> AgentRunnerKind {
        AgentRunnerKind::ClaudeCode
    }

    fn build_command(
        &self,
        worktree_path: &Path,
        prompt: &str,
        env: &HashMap<String, String>,
    ) -> CommandBuilder {
        let mut cmd = CommandBuilder::new("claude");
        cmd.arg("-p");
        cmd.arg(prompt);
        cmd.arg("--output-format");
        cmd.arg("stream-json");
        cmd.arg("--include-partial-messages");
        cmd.cwd(worktree_path);
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd
    }
}

// TODO(fase 4): parser dos eventos stream-json -> SessionStatus
//   - tool_use pendente de aprovação => AwaitingInput { hint }
//   - result final => Idle
// TODO(fase 5): CodexRunner, CustomRunner
