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

use std::path::PathBuf;

use portable_pty::CommandBuilder;

use policy::AgentAccess;

pub(crate) const TOOLCHAIN_HOME_DIRS: [&str; 4] = [".cargo", ".npm", ".bun", ".rustup"];

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
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(
            "sandbox de agente indisponível nesta plataforma — sessão recusada (fail-closed)"
                .into(),
        )
    }
}
