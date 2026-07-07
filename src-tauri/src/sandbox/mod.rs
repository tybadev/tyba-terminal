//! Sandbox de execução de agentes.
//!
//! Princípio #2: a trait existe desde o dia 1 mesmo com implementação
//! passthrough — retrofitar isolamento depois é cirurgia.
//! Fase 5: Seatbelt/sandbox-exec (macOS), Landlock/bubblewrap (Linux):
//! write apenas em worktree + temp, read no repo.

use std::path::PathBuf;

use portable_pty::CommandBuilder;

pub struct SandboxSpec {
    /// Único diretório com permissão de escrita (o worktree da sessão).
    pub writable_root: PathBuf,
    /// Raiz de leitura (repo original).
    pub readable_root: PathBuf,
    /// Rede permitida? (vermelho por default para agentes)
    pub allow_network: bool,
}

pub trait Sandbox: Send + Sync {
    /// Envolve o comando do agente na política de isolamento.
    fn wrap(&self, cmd: CommandBuilder, spec: &SandboxSpec) -> CommandBuilder;
}

/// MVP: sem isolamento real, mas todo spawn de agente já passa por aqui.
pub struct PassthroughSandbox;

impl Sandbox for PassthroughSandbox {
    fn wrap(&self, cmd: CommandBuilder, _spec: &SandboxSpec) -> CommandBuilder {
        cmd
    }
}
