pub mod policy;
#[cfg(target_os = "macos")]
pub mod seatbelt;
#[cfg(all(test, target_os = "macos"))]
mod seatbelt_exec_tests;

use std::path::PathBuf;

use portable_pty::CommandBuilder;

use policy::AgentAccess;

pub struct SandboxSpec {
    pub writable_root: PathBuf,
    pub readable_root: PathBuf,
    pub allow_network: bool,
    pub repo_git_dir: PathBuf,
    pub worktree_git_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub hook_socket: PathBuf,
    pub tyba_exe: PathBuf,
    pub home: PathBuf,
    pub tmpdir: Option<PathBuf>,
    pub exec_path_dirs: Vec<PathBuf>,
    pub agent: AgentAccess,
    pub read_allow_extra: Vec<PathBuf>,
}

pub trait Sandbox: Send + Sync {
    fn wrap(&self, cmd: CommandBuilder, spec: &SandboxSpec) -> Result<CommandBuilder, String>;
}

pub fn platform_sandbox() -> Result<Box<dyn Sandbox>, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(seatbelt::SeatbeltSandbox::new()?))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(
            "sandbox de agente indisponível nesta plataforma — sessão recusada (fail-closed)"
                .into(),
        )
    }
}
