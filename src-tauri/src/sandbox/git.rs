use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitProfile {
    ReadOnly,
    Write,
    Network,
}

fn push_unique(roots: &mut Vec<PathBuf>, p: PathBuf) {
    if !roots.contains(&p) {
        roots.push(p);
    }
}

/// Diretório do `.git` do worktree, resolvido por leitura de arquivo (sem
/// shell-out de git): o painel chama isto por op de escrita e não pode pagar
/// um subprocesso só pra montar a política.
fn cheap_git_dir(repo: &Path) -> Option<PathBuf> {
    let pointer = repo.join(".git");
    if pointer.is_dir() {
        return Some(pointer);
    }
    let content = std::fs::read_to_string(&pointer).ok()?;
    let rel = content
        .lines()
        .find_map(|l| l.strip_prefix("gitdir:"))?
        .trim();
    let dir = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        repo.join(rel)
    };
    Some(dir)
}

fn writable_roots(repo: &Path, extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = vec![crate::repo::canonicalize_or(repo)];
    for e in extra {
        push_unique(&mut roots, crate::repo::canonicalize_or(e));
    }
    if let Some(git_dir) = cheap_git_dir(repo) {
        let git_dir = crate::repo::canonicalize_or(&git_dir);
        // `.git/worktrees/<n>/commondir` aponta pro `.git` compartilhado do
        // repo principal, onde vivem objects/refs que o commit escreve.
        if let Ok(common) = std::fs::read_to_string(git_dir.join("commondir")) {
            let common = git_dir.join(common.trim());
            push_unique(&mut roots, crate::repo::canonicalize_or(&common));
        }
        push_unique(&mut roots, git_dir);
    }
    if let Ok(managed) = crate::worktree::managed_root() {
        push_unique(&mut roots, crate::repo::canonicalize_or(&managed));
    }
    roots
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use crate::sandbox::policy::{render_rule, Rule};

    pub const LAUNCHER: &str = "/usr/bin/sandbox-exec";

    fn base_policy() -> String {
        [
            "(version 1)",
            "(deny default)",
            "(allow process-fork)",
            "(allow process-exec)",
            "(allow signal (target same-sandbox))",
            "(allow process-info* (target same-sandbox))",
            "(allow file-read-metadata)",
            "(allow sysctl-read)",
            "(allow ipc-posix-shm*)",
            "(allow user-preference-read)",
            "(allow mach-lookup (global-name \"com.apple.cfprefsd.daemon\") (global-name \"com.apple.cfprefsd.agent\") (global-name \"com.apple.system.opendirectoryd.libinfo\") (global-name \"com.apple.system.notification_center\") (global-name \"com.apple.system.logger\"))",
            "(allow file-read* file-map-executable)",
            "(allow file-ioctl)",
            "(allow file-write* (literal \"/dev/null\") (literal \"/dev/zero\") (literal \"/dev/dtracehelper\") (literal \"/dev/tty\") (regex #\"^/dev/ttys[0-9]+$\"))",
        ]
        .join("\n")
    }

    pub fn policy(profile: GitProfile, repo: &Path, extra: &[PathBuf]) -> String {
        let mut policy = base_policy();
        if profile != GitProfile::ReadOnly {
            let roots = writable_roots(repo, extra)
                .iter()
                .map(|r| render_rule(&Rule::Subpath(r.clone())))
                .collect::<Vec<_>>()
                .join(" ");
            policy.push_str(&format!("\n(allow file-write* {roots})"));
        }
        if profile == GitProfile::Network {
            policy.push_str("\n(allow network-outbound)\n(allow system-socket)\n(allow mach-lookup (global-name \"com.apple.SecurityServer\") (global-name \"com.apple.trustd\") (global-name \"com.apple.trustd.agent\") (global-name \"com.apple.ocspd\") (global-name \"com.apple.networkd\") (global-name \"com.apple.dnssd.service\") (global-name \"com.apple.SystemConfiguration.configd\") (global-name \"com.apple.SystemConfiguration.DNSConfiguration\"))");
        }
        policy
    }

    pub fn wrap(git: Command, profile: GitProfile, repo: &Path, extra: &[PathBuf]) -> Command {
        let mut cmd = Command::new(LAUNCHER);
        cmd.arg("-p").arg(policy(profile, repo, extra)).arg("--");
        cmd.arg(git.get_program());
        cmd.args(git.get_args());
        cmd
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;

    pub const LAUNCHER: &str = "bwrap";

    pub fn wrap(git: Command, profile: GitProfile, repo: &Path, extra: &[PathBuf]) -> Command {
        let mut cmd = Command::new(LAUNCHER);
        cmd.args([
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--die-with-parent",
        ]);
        cmd.arg("--unshare-user");
        cmd.arg("--unshare-pid");
        cmd.arg("--unshare-ipc");
        cmd.arg("--unshare-uts");
        cmd.arg("--unshare-cgroup-try");
        if profile != GitProfile::Network {
            cmd.arg("--unshare-net");
        }
        if profile != GitProfile::ReadOnly {
            for root in writable_roots(repo, extra) {
                if root.exists() {
                    let s = root.to_string_lossy().into_owned();
                    cmd.args(["--bind", &s, &s]);
                }
            }
        }
        cmd.arg("--");
        cmd.arg(git.get_program());
        cmd.args(git.get_args());
        cmd
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    use super::*;
    pub fn wrap(git: Command, _profile: GitProfile, _repo: &Path, _extra: &[PathBuf]) -> Command {
        git
    }
}

pub fn wrap(git: Command, profile: GitProfile, repo: &Path, extra: &[PathBuf]) -> Command {
    imp::wrap(git, profile, repo, extra)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cheap_git_dir_reads_dir_and_pointer() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("main");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        assert_eq!(cheap_git_dir(&repo), Some(repo.join(".git")));

        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: /main/.git/worktrees/wt\n").unwrap();
        assert_eq!(
            cheap_git_dir(&wt),
            Some(PathBuf::from("/main/.git/worktrees/wt"))
        );
    }

    #[test]
    fn writable_roots_never_shell_out_and_include_extra() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("r");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let extra = tmp.path().join("managed");
        std::fs::create_dir_all(&extra).unwrap();
        let roots = writable_roots(&repo, std::slice::from_ref(&extra));
        assert!(roots.contains(&crate::repo::canonicalize_or(&repo)));
        assert!(roots.contains(&crate::repo::canonicalize_or(&extra)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn read_only_policy_denies_writes_and_network() {
        let repo = PathBuf::from("/private/tmp/x");
        let p = imp::policy(GitProfile::ReadOnly, &repo, &[]);
        assert!(p.contains("(deny default)"));
        assert!(!p.contains("(allow network-outbound)"));
        assert!(!p.contains("(allow file-write* (subpath"));
        assert!(p.contains(r#"(allow file-write* (literal "/dev/null")"#));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn write_policy_allows_repo_but_not_network() {
        let repo = PathBuf::from("/private/tmp/x");
        std::fs::create_dir_all(repo.join(".git")).ok();
        let p = imp::policy(GitProfile::Write, &repo, &[]);
        assert!(p.contains("(allow file-write* (subpath"));
        assert!(p.contains("/private/tmp/x"));
        assert!(
            !p.contains("(allow network-outbound)"),
            "op de escrita não pode ter rede — um filtro encaixotado exfiltraria"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn network_policy_allows_outbound_and_tls() {
        let repo = PathBuf::from("/private/tmp/x");
        let p = imp::policy(GitProfile::Network, &repo, &[]);
        assert!(p.contains("(allow network-outbound)"));
        assert!(p.contains("com.apple.SecurityServer"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn wrap_prepends_sandbox_exec_and_git_becomes_arg() {
        let mut git = Command::new("git");
        git.args(["-C", "/repo", "status"]);
        let wrapped = wrap(git, GitProfile::ReadOnly, Path::new("/repo"), &[]);
        assert_eq!(wrapped.get_program(), imp::LAUNCHER);
        let argv: Vec<String> = wrapped
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(argv[0], "-p");
        assert_eq!(argv[2], "--");
        assert_eq!(argv[3], "git");
        assert!(argv.contains(&"status".to_string()));
    }
}
