pub mod bwrap;
#[cfg(all(test, target_os = "linux"))]
mod bwrap_exec_tests;
pub mod git;
pub mod policy;
#[cfg(target_os = "macos")]
pub mod seatbelt;
#[cfg(all(test, target_os = "macos"))]
mod seatbelt_exec_tests;
#[cfg(target_os = "linux")]
pub mod seccomp;
#[cfg(target_os = "windows")]
pub mod windows;

use std::path::PathBuf;

use portable_pty::CommandBuilder;

use policy::AgentAccess;

pub(crate) const TOOLCHAIN_HOME_DIRS: [&str; 4] = [".cargo", ".npm", ".bun", ".rustup"];

/// Branches que nenhuma sessão de agente pode mover — nem por push (o core já
/// recusa) nem localmente. Importa quando o agente é anexado ao checkout
/// principal (review/conflitos num repo comum): ali o gitdir do worktree é o
/// `.git` do repo, e sem regra explícita o `refs/heads/main` cairia dentro da
/// área gravável.
pub(crate) const PROTECTED_BRANCHES: [&str; 2] = ["main", "master"];

pub struct SandboxSpec {
    pub writable_root: PathBuf,
    pub readable_root: PathBuf,
    pub allow_network: bool,
    pub repo_git_dir: PathBuf,
    pub worktree_git_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub hook_socket: PathBuf,
    pub tyba_exe: PathBuf,
    pub tyba_data_dir: PathBuf,
    pub home: PathBuf,
    pub tmpdir: Option<PathBuf>,
    pub exec_path_dirs: Vec<PathBuf>,
    pub agent: AgentAccess,
    pub read_allow_extra: Vec<PathBuf>,
}

pub trait Sandbox: Send + Sync {
    fn wrap(&self, cmd: CommandBuilder, spec: &SandboxSpec) -> Result<CommandBuilder, String>;

    /// Camada A do Windows (decisão de integração Opção B): a jaula se aplica no
    /// spawn (token restrito + ConPTY), não por reescrita de argv. Quando devolve
    /// `Some`, a camada de sessão sobe o agente por esse spawner e NÃO chama `wrap`.
    /// Default `None`: mac/linux aplicam a jaula via `wrap`.
    fn jailed_spawner(
        &self,
        _spec: &SandboxSpec,
    ) -> Result<Option<Box<dyn crate::pty::JailedSpawner>>, String> {
        Ok(None)
    }
}

pub fn platform_sandbox() -> Result<Box<dyn Sandbox>, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(seatbelt::SeatbeltSandbox::new()?))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(bwrap::BwrapSandbox::new()?))
    }
    #[cfg(target_os = "windows")]
    {
        // Camada A do Windows: a jaula se aplica no spawn (token restrito + ConPTY)
        // via `Sandbox::jailed_spawner`, não por `wrap`. Ver `sandbox/windows.rs`.
        Ok(Box::new(windows::WindowsSandbox::new()?))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err(
            "sandbox de agente indisponível nesta plataforma — sessão recusada (fail-closed)"
                .into(),
        )
    }
}
