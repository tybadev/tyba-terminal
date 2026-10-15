//! A implementação padrão da trait `Sandbox` no agente livre (princípio 2):
//! não jaula nada. O spawn continua passando pela trait — é o mesmo ponto
//! único de spawn de agente — mas o comando sai como entrou, e quem pede
//! permissão é o próprio CLI do agente.

use portable_pty::CommandBuilder;

use super::{Sandbox, SandboxSpec};

pub struct PassthroughSandbox;

impl Sandbox for PassthroughSandbox {
    fn wrap(&self, cmd: CommandBuilder, _spec: &SandboxSpec) -> Result<CommandBuilder, String> {
        Ok(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn spec() -> SandboxSpec {
        SandboxSpec {
            writable_root: PathBuf::from("/w"),
            readable_root: PathBuf::from("/w"),
            allow_network: true,
            repo_git_dir: PathBuf::new(),
            worktree_git_dir: PathBuf::new(),
            runtime_dir: PathBuf::from("/r"),
            hook_socket: PathBuf::from("/r/hook.sock"),
            tyba_exe: PathBuf::from("/bin/tyba"),
            tyba_data_dir: PathBuf::new(),
            home: PathBuf::new(),
            tmpdir: None,
            exec_path_dirs: Vec::new(),
            agent: Default::default(),
            read_allow_extra: Vec::new(),
            data_dir_reads: Vec::new(),
        }
    }

    #[test]
    fn wrap_returns_the_command_untouched() {
        let mut cmd = CommandBuilder::new("claude");
        cmd.arg("-c");
        cmd.env("X", "1");
        let out = PassthroughSandbox.wrap(cmd, &spec()).unwrap();
        assert_eq!(out.get_argv(), &["claude", "-c"]);
        assert!(out
            .iter_extra_env_as_str()
            .all(|(k, _)| k != "TYBA_SANDBOX"));
    }
}
