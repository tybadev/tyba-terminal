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

fn config_files(repo: &Path) -> Vec<PathBuf> {
    let Some(git_dir) = cheap_git_dir(repo) else {
        return Vec::new();
    };
    let mut files = vec![git_dir.join("config"), git_dir.join("config.worktree")];
    if let Ok(common) = std::fs::read_to_string(git_dir.join("commondir")) {
        let common = git_dir.join(common.trim());
        files.push(common.join("config"));
    }
    files
}

fn section_name(line: &str) -> Option<String> {
    let l = line.trim();
    let rest = l.strip_prefix('[')?;
    let name: String = rest
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ']' && *c != '"')
        .collect();
    Some(name.to_ascii_lowercase())
}

/// A RCE do #42 só arma se o repo define um filtro de conteúdo (`clean`/`smudge`/
/// `process`) ou um `diff.*.textconv` — e um `[include]` pode esconder qualquer um.
/// `core.fsmonitor`/`core.pager`/`diff.external` já são neutralizados por `-c` no
/// `git_in`. Detectar por leitura de arquivo (microssegundos) evita pagar a jaula
/// nos ~99% dos repos que não têm filtro nenhum.
pub fn repo_uses_content_filter(repo: &Path) -> bool {
    let files = config_files(repo);
    if files.is_empty() {
        return true; // não consegui resolver o gitdir → fail-secure
    }
    for cfg in files {
        let Ok(content) = std::fs::read_to_string(&cfg) else {
            continue;
        };
        for line in content.lines() {
            if let Some(section) = section_name(line) {
                if matches!(
                    section.as_str(),
                    "filter" | "diff" | "include" | "includeif"
                ) {
                    return true;
                }
            }
        }
    }
    false
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
    // `push`/`fetch` não rodam filtro de conteúdo do worktree — não são o vetor
    // do #42 — e precisam de rede + credencial (SSH/creds), que uma jaula
    // arriscaria quebrar sem ganho no escopo desta defesa. O vetor próprio deles
    // (transporte `ext::`, credential helper) é item separado. Ficam fora.
    if profile == GitProfile::Network {
        return git;
    }
    // Sem filtro configurado no repo, não há RCE possível — roda direto, na
    // velocidade nativa. A jaula só custa nos repos que de fato armam o vetor.
    if !repo_uses_content_filter(repo) {
        return git;
    }
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

    #[test]
    fn detects_content_filter_only_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("r");
        std::fs::create_dir_all(repo.join(".git")).unwrap();

        std::fs::write(repo.join(".git/config"), "[core]\n\tbare = false\n").unwrap();
        assert!(!repo_uses_content_filter(&repo), "repo limpo → sem jaula");

        std::fs::write(
            repo.join(".git/config"),
            "[core]\n\tbare = false\n[filter \"pwn\"]\n\tclean = sh -c evil\n",
        )
        .unwrap();
        assert!(repo_uses_content_filter(&repo), "filtro → jaula");

        std::fs::write(
            repo.join(".git/config"),
            "[core]\n[include]\n\tpath = /tmp/hidden\n",
        )
        .unwrap();
        assert!(
            repo_uses_content_filter(&repo),
            "include pode esconder filtro → fail-secure"
        );
    }

    #[test]
    fn missing_gitdir_fails_secure() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(repo_uses_content_filter(&tmp.path().join("not-a-repo")));
    }

    #[test]
    fn network_profile_is_not_caged() {
        let mut git = Command::new("git");
        git.args(["-C", "/repo", "push"]);
        let out = wrap(git, GitProfile::Network, Path::new("/repo"), &[]);
        assert_eq!(
            out.get_program(),
            "git",
            "push/fetch ficam fora da jaula — precisam de rede+credencial e não são vetor de filtro"
        );
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
