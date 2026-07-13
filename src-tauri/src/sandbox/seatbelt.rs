use std::path::{Path, PathBuf};

use portable_pty::CommandBuilder;

use super::policy::{render_rule, render_rules, render_ruleset, Rule, RuleSet};
use super::{Sandbox, SandboxSpec};

pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

const READ_OPS: &str = "file-read* file-map-executable";

const XCODE_SELECT_LINK: &str = "/private/var/db/xcode_select_link";

const BASE_MACH_LOOKUPS: [&str; 8] = [
    "com.apple.cfprefsd.daemon",
    "com.apple.cfprefsd.agent",
    "com.apple.system.opendirectoryd.libinfo",
    "com.apple.system.notification_center",
    "com.apple.system.logger",
    "com.apple.logd",
    "com.apple.diagnosticd",
    "com.apple.FSEvents",
];

const TLS_MACH_LOOKUPS: [&str; 11] = [
    "com.apple.SecurityServer",
    "com.apple.trustd",
    "com.apple.trustd.agent",
    "com.apple.ocspd",
    "com.apple.networkd",
    "com.apple.dnssd.service",
    "com.apple.SystemConfiguration.configd",
    "com.apple.SystemConfiguration.DNSConfiguration",
    "com.apple.SystemConfiguration.NetworkInformation",
    "com.apple.nehelper",
    "com.apple.nesessionmanager.content-filter",
];

const SYSTEM_READ_ROOTS: [&str; 10] = [
    "/usr",
    "/System",
    "/bin",
    "/sbin",
    "/opt/homebrew",
    "/usr/local",
    "/Library/Frameworks",
    "/Library/Preferences",
    "/private/etc",
    "/dev",
];

use super::TOOLCHAIN_HOME_DIRS;

fn mach_lookup_line(names: &[&str]) -> String {
    let globals = names
        .iter()
        .map(|n| format!("(global-name \"{n}\")"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("(allow mach-lookup {globals})")
}

fn app_bundle(path: &Path) -> Option<&Path> {
    path.ancestors()
        .find(|a| a.extension().map(|e| e == "app").unwrap_or(false))
}

fn developer_dir_rules() -> Vec<Rule> {
    let mut rules = vec![Rule::Subpath(PathBuf::from("/Library/Developer"))];
    let Ok(target) = std::fs::read_link(XCODE_SELECT_LINK) else {
        return rules;
    };
    let bundle = app_bundle(&target).unwrap_or(&target);
    rules.push(Rule::Subpath(bundle.to_path_buf()));
    rules
}

fn darwin_cache_dir(tmpdir: &Path) -> Option<PathBuf> {
    if !tmpdir.starts_with("/private/var/folders") || tmpdir.file_name()? != "T" {
        return None;
    }
    Some(tmpdir.parent()?.join("C"))
}

fn container_socket_dirs(home: &Path) -> Vec<Rule> {
    let system = [
        "/var/run/docker.sock",
        "/private/var/run/docker.sock",
        "/var/run/podman/podman.sock",
        "/private/var/run/podman/podman.sock",
        "/var/run/containerd/containerd.sock",
        "/run/docker.sock",
        "/run/podman/podman.sock",
    ]
    .iter()
    .map(|p| Rule::Literal(PathBuf::from(p)));
    let user = [
        ".docker/run",
        ".colima",
        ".orbstack/run",
        ".rd",
        ".lima",
        ".local/share/containers",
        ".local/share/nerdctl",
        ".config/containers",
    ]
    .iter()
    .map(|p| Rule::Subpath(home.join(p)));
    system.chain(user).collect()
}

fn tyba_exe_read_rules(exe: &Path) -> Vec<Rule> {
    let mut rules = vec![Rule::Literal(exe.to_path_buf())];
    if let Some(bundle) = app_bundle(exe) {
        rules.push(Rule::Subpath(bundle.to_path_buf()));
    }
    rules
}

fn canonical_dir(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn exec_path_read_rules(spec: &SandboxSpec) -> Vec<Rule> {
    let home = canonical_dir(&spec.home);
    let mut seen = std::collections::HashSet::new();
    let mut rules = Vec::new();
    for dir in &spec.exec_path_dirs {
        if !dir.is_absolute() {
            continue;
        }
        let canon = canonical_dir(dir);
        if home.starts_with(&canon) {
            continue;
        }
        if seen.insert(canon.clone()) {
            rules.push(Rule::Subpath(canon));
        }
    }
    rules
}

pub fn build_policy(spec: &SandboxSpec) -> String {
    let mut lines: Vec<String> = vec![
        "(version 1)".into(),
        "(deny default)".into(),
        "(allow process-fork)".into(),
        "(allow process-exec)".into(),
        "(allow signal (target same-sandbox))".into(),
        "(allow process-info* (target same-sandbox))".into(),
        "(allow file-read-metadata)".into(),
        "(allow sysctl-read)".into(),
        "(allow ipc-posix-shm*)".into(),
        "(allow user-preference-read)".into(),
        mach_lookup_line(&BASE_MACH_LOOKUPS),
        format!(
            "(allow file-ioctl (literal \"/dev/tty\") (literal \"/dev/ptmx\") (literal \"/dev/dtracehelper\") (regex #\"^/dev/ttys[0-9]+$\"))"
        ),
        format!(
            "(allow file-write* (literal \"/dev/null\") (literal \"/dev/zero\") (literal \"/dev/dtracehelper\") (literal \"/dev/tty\") (regex #\"^/dev/ttys[0-9]+$\"))"
        ),
    ];

    let mut system_read: Vec<Rule> = SYSTEM_READ_ROOTS
        .iter()
        .map(|p| Rule::Subpath(PathBuf::from(p)))
        .collect();
    system_read.push(Rule::Literal(PathBuf::from("/")));
    system_read.push(Rule::Subpath(PathBuf::from("/private/var/db/timezone")));
    system_read.push(Rule::Literal(PathBuf::from(XCODE_SELECT_LINK)));
    system_read.extend(developer_dir_rules());
    lines.extend(render_ruleset(READ_OPS, &RuleSet::allow(system_read)));

    let mut home_read = vec![
        Rule::Literal(spec.home.join(".gitconfig")),
        Rule::Subpath(spec.home.join(".config/git")),
    ];
    home_read.extend(
        TOOLCHAIN_HOME_DIRS
            .iter()
            .map(|d| Rule::Subpath(spec.home.join(d))),
    );
    home_read.extend(
        spec.read_allow_extra
            .iter()
            .map(|p| Rule::Subpath(p.clone())),
    );
    lines.extend(render_ruleset(
        READ_OPS,
        &RuleSet {
            allow: home_read,
            except: vec![
                Rule::Prefix(spec.home.join(".cargo/credentials")),
                Rule::Literal(spec.home.join(".git-credentials")),
                Rule::Literal(spec.home.join(".netrc")),
            ],
        },
    ));

    lines.extend(render_ruleset(
        READ_OPS,
        &RuleSet::allow(exec_path_read_rules(spec)),
    ));
    lines.extend(render_ruleset(
        READ_OPS,
        &RuleSet::allow(tyba_exe_read_rules(&spec.tyba_exe)),
    ));

    lines.extend(render_ruleset(
        READ_OPS,
        &RuleSet::allow(vec![
            Rule::Subpath(spec.readable_root.clone()),
            Rule::Subpath(spec.writable_root.clone()),
            Rule::Subpath(spec.runtime_dir.clone()),
        ]),
    ));

    let private_tmp = RuleSet {
        allow: vec![Rule::Subpath(PathBuf::from("/private/tmp"))],
        except: vec![Rule::Prefix(PathBuf::from("/private/tmp/tyba-"))],
    };
    lines.extend(render_ruleset(READ_OPS, &private_tmp));

    let tmpdir_rules = spec.tmpdir.as_deref().map(|raw| {
        let tmpdir = canonical_dir(raw);
        let mut allow = vec![Rule::Subpath(tmpdir.clone())];
        let mut except = vec![Rule::Prefix(tmpdir.join("tyba-"))];
        if let Some(cache) = darwin_cache_dir(&tmpdir) {
            except.push(Rule::Prefix(cache.join("tyba-")));
            allow.push(Rule::Subpath(cache));
        }
        RuleSet { allow, except }
    });
    if let Some(tmp_rules) = &tmpdir_rules {
        lines.extend(render_ruleset(READ_OPS, tmp_rules));
    }

    lines.extend(render_ruleset(
        "file-write*",
        &RuleSet {
            allow: vec![Rule::Subpath(spec.writable_root.clone())],
            except: vec![Rule::Node(spec.writable_root.join(".git"))],
        },
    ));
    lines.extend(render_ruleset(
        "file-write*",
        &RuleSet {
            allow: vec![Rule::Subpath(spec.worktree_git_dir.clone())],
            except: vec![
                Rule::Node(spec.worktree_git_dir.join("hooks")),
                Rule::Node(spec.worktree_git_dir.join("config")),
                Rule::Node(spec.worktree_git_dir.join("config.worktree")),
                Rule::Node(spec.worktree_git_dir.join("info")),
            ],
        },
    ));
    lines.extend(render_ruleset(
        "file-write*",
        &RuleSet::allow(vec![
            Rule::Subpath(spec.repo_git_dir.join("objects")),
            Rule::Node(spec.repo_git_dir.join("refs/heads/tyba")),
            Rule::Subpath(spec.repo_git_dir.join("logs/refs/heads/tyba")),
        ]),
    ));
    lines.extend(render_ruleset("file-write*", &private_tmp));
    if let Some(tmp_rules) = &tmpdir_rules {
        lines.extend(render_ruleset("file-write*", tmp_rules));
    }

    for set in &spec.agent.read {
        lines.extend(render_ruleset(READ_OPS, set));
    }
    for set in &spec.agent.write {
        lines.extend(render_ruleset("file-write*", set));
    }

    let hook_socket_allow = format!(
        "(allow network-outbound {})",
        render_rule(&Rule::Literal(spec.hook_socket.clone()))
    );
    if spec.allow_network {
        lines.push("(allow network-outbound)".into());
        lines.push("(allow system-socket)".into());
        lines.push("(allow network-bind (local ip \"localhost:*\"))".into());
        lines.push("(allow network-inbound (local ip \"localhost:*\"))".into());
        lines.push(mach_lookup_line(&TLS_MACH_LOOKUPS));
        let containers = render_rules(&container_socket_dirs(&spec.home));
        lines.push(format!("(deny network-outbound {containers})"));
    }
    lines.push(hook_socket_allow);

    let secrets = render_rules(&secret_denies(spec));
    lines.push(format!("(deny file-read* file-write* {secrets})"));
    let immutable = render_rules(&immutable_denies(spec));
    lines.push(format!("(deny file-write* {immutable})"));

    lines.join("\n")
}

fn secret_denies(spec: &SandboxSpec) -> Vec<Rule> {
    let home = &spec.home;
    let mut rules = vec![
        Rule::Node(home.join(".ssh")),
        Rule::Node(home.join(".aws")),
        Rule::Node(home.join(".gnupg")),
        Rule::Node(home.join(".kube")),
        Rule::Node(home.join(".config/gcloud")),
        Rule::Node(home.join(".config/gh")),
        Rule::Node(home.join("Library/Keychains")),
        Rule::Node(spec.tyba_data_dir.clone()),
        Rule::Node(home.join("Library/Application Support/dev.tyba.app")),
        Rule::Prefix(home.join(".git-credentials")),
        Rule::Prefix(home.join(".netrc")),
        Rule::Prefix(home.join(".npmrc")),
        Rule::Prefix(home.join(".pypirc")),
        Rule::Prefix(home.join(".docker/config.json")),
        Rule::Prefix(home.join(".cargo/credentials")),
    ];
    rules.extend(container_socket_dirs(home));
    rules
}

fn immutable_denies(spec: &SandboxSpec) -> Vec<Rule> {
    let mut rules = vec![
        Rule::Node(spec.home.join(".tyba/config.toml")),
        Rule::Node(spec.readable_root.join(".tyba/config.toml")),
        Rule::Node(spec.writable_root.join(".tyba/config.toml")),
        Rule::Node(spec.runtime_dir.clone()),
    ];
    // Sessão anexada ao checkout principal (review/conflitos num repo comum): ali
    // o gitdir do worktree É o `.git` do repo, e o allow de escrita do gitdir
    // alcançaria `refs/heads/main`. O agente não daria push — o core recusa —, mas
    // moveria a main LOCAL, que é a mesma regra vista de outro ângulo. O deny é
    // final: vence qualquer allow anterior.
    for name in super::PROTECTED_BRANCHES {
        rules.push(Rule::Node(spec.repo_git_dir.join("refs/heads").join(name)));
        rules.push(Rule::Node(
            spec.repo_git_dir.join("logs/refs/heads").join(name),
        ));
    }
    // `git pack-refs` move as refs pra dentro deste arquivo: gravável, ele é um
    // segundo caminho pra reescrever a main.
    rules.push(Rule::Node(spec.repo_git_dir.join("packed-refs")));
    rules.extend(tyba_exe_read_rules(&spec.tyba_exe));
    rules
}

pub struct SeatbeltSandbox;

impl SeatbeltSandbox {
    pub fn new() -> Result<Self, String> {
        if !Path::new(SANDBOX_EXEC).is_file() {
            return Err(format!("{SANDBOX_EXEC} não existe nesta máquina"));
        }
        Ok(SeatbeltSandbox)
    }
}

impl Sandbox for SeatbeltSandbox {
    fn wrap(&self, mut cmd: CommandBuilder, spec: &SandboxSpec) -> Result<CommandBuilder, String> {
        let policy = build_policy(spec);
        let argv = cmd.get_argv_mut();
        if argv.is_empty() {
            return Err("comando de agente vazio".into());
        }
        let mut wrapped: Vec<std::ffi::OsString> =
            vec![SANDBOX_EXEC.into(), "-p".into(), policy.into(), "--".into()];
        wrapped.append(argv);
        *argv = wrapped;
        cmd.env("TYBA_SANDBOX", "seatbelt");
        Ok(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::policy::AgentAccess;

    fn spec() -> SandboxSpec {
        SandboxSpec {
            writable_root: PathBuf::from("/private/wt/session-a"),
            readable_root: PathBuf::from("/private/repo"),
            allow_network: true,
            repo_git_dir: PathBuf::from("/private/repo/.git"),
            worktree_git_dir: PathBuf::from("/private/repo/.git/worktrees/session-a"),
            runtime_dir: PathBuf::from("/private/tmp/tyba-abc"),
            hook_socket: PathBuf::from("/private/tmp/tyba-abc/hook.sock"),
            tyba_exe: PathBuf::from("/Apps/Tyba.app/Contents/MacOS/tyba"),
            tyba_data_dir: PathBuf::from("/Users/nobody/Library/Application Support/dev.tyba.app"),
            home: PathBuf::from("/Users/nobody"),
            tmpdir: Some(PathBuf::from("/private/var/folders/xx/T")),
            exec_path_dirs: vec![PathBuf::from("/Users/nobody/.local/bin")],
            agent: AgentAccess::default(),
            read_allow_extra: vec![],
        }
    }

    #[test]
    fn policy_starts_with_deny_default() {
        let policy = build_policy(&spec());
        assert!(policy.starts_with("(version 1)\n(deny default)"));
    }

    #[test]
    fn policy_has_unconditional_pty_ioctl() {
        let policy = build_policy(&spec());
        assert!(policy.contains(r##"(allow file-ioctl (literal "/dev/tty") (literal "/dev/ptmx") (literal "/dev/dtracehelper") (regex #"^/dev/ttys[0-9]+$"))"##));
    }

    #[test]
    fn policy_protects_worktree_git_pointer() {
        let policy = build_policy(&spec());
        assert!(policy.contains(r#"(regex #"^/private/wt/session-a/\.git(/.*)?$")"#));
    }

    #[test]
    fn policy_blocks_gitdir_hooks_config_info() {
        let policy = build_policy(&spec());
        for frag in ["hooks", "config", "config.worktree", "info"] {
            assert!(
                policy.contains(&format!(
                    "(subpath \"/private/repo/.git/worktrees/session-a/{frag}\")"
                )),
                "faltou except pra {frag}"
            );
        }
    }

    #[test]
    fn policy_allows_objects_and_only_the_tyba_ref_namespace() {
        let policy = build_policy(&spec());
        assert!(policy.contains(r#"(subpath "/private/repo/.git/objects")"#));
        assert!(policy.contains(r#"(subpath "/private/repo/.git/refs/heads/tyba")"#));
        assert!(policy.contains(r#"(subpath "/private/repo/.git/logs/refs/heads/tyba")"#));
    }

    #[test]
    fn policy_never_grants_write_on_main_or_the_whole_refs_tree() {
        let policy = build_policy(&spec());
        // Só as linhas de ALLOW: `packed-refs` e `refs/heads/main` agora aparecem
        // também no deny final, que é exatamente onde devem estar.
        for line in policy
            .lines()
            .filter(|l| l.starts_with("(allow file-write*"))
        {
            assert!(
                !line.contains(r#"(subpath "/private/repo/.git/refs")"#),
                "{line}"
            );
            assert!(
                !line.contains("packed-refs"),
                "packed-refs gravável deixa reescrever main: {line}"
            );
        }
    }

    #[test]
    fn policy_never_grants_write_on_repo_root_or_home() {
        let policy = build_policy(&spec());
        for line in policy.lines().filter(|l| l.contains("file-write*")) {
            assert!(!line.contains("(subpath \"/private/repo\")"), "{line}");
            assert!(!line.contains("(subpath \"/Users/nobody\")"), "{line}");
        }
    }

    #[test]
    fn policy_hook_socket_allowed_even_without_network() {
        let mut s = spec();
        s.allow_network = false;
        let policy = build_policy(&s);
        assert!(policy
            .contains(r#"(allow network-outbound (literal "/private/tmp/tyba-abc/hook.sock")"#));
        assert!(!policy.contains("(allow system-socket)"));
        assert!(!policy.contains("com.apple.SecurityServer"));
    }

    #[test]
    fn policy_network_denies_docker_sockets_after_allowing_the_rest() {
        let policy = build_policy(&spec());
        let allow = policy
            .lines()
            .position(|l| l == "(allow network-outbound)")
            .unwrap();
        let deny = policy
            .lines()
            .position(|l| l.starts_with("(deny network-outbound"))
            .unwrap();
        assert!(deny > allow, "no SBPL a última regra que casa vence");
        let lines: Vec<&str> = policy.lines().collect();
        let deny_line = lines[deny];
        assert!(deny_line.contains(r#"(literal "/var/run/docker.sock")"#));
        assert!(deny_line.contains(r#"(literal "/var/run/podman/podman.sock")"#));
        assert!(deny_line.contains(r#"(subpath "/Users/nobody/.docker/run")"#));
        assert!(deny_line.contains(r#"(subpath "/Users/nobody/.rd")"#));
        assert!(deny_line.contains(r#"(subpath "/Users/nobody/.lima")"#));
        assert!(
            !deny_line.contains(r#"(subpath "/var/run")"#),
            "negar /var/run inteiro mata o socket do mDNSResponder e quebra DNS"
        );
        let hook_reallow = lines
            .iter()
            .rposition(|l| l.starts_with("(allow network-outbound (literal"))
            .unwrap();
        assert!(
            hook_reallow > deny,
            "o socket de hook precisa sobreviver ao deny de sockets de container"
        );
        assert!(policy.contains("com.apple.SecurityServer"));
        assert!(policy.contains("com.apple.trustd"));
        assert!(policy.contains("com.apple.dnssd.service"));
    }

    #[test]
    fn hard_denies_are_the_last_word_over_any_allow() {
        let mut s = spec();
        s.read_allow_extra = vec![s.home.clone()];
        let policy = build_policy(&s);
        let lines: Vec<&str> = policy.lines().collect();
        let secret_deny = lines
            .iter()
            .position(|l| l.starts_with("(deny file-read* file-write*"))
            .unwrap();
        let last_allow = lines
            .iter()
            .rposition(|l| l.starts_with("(allow file-read*"))
            .unwrap();
        assert!(
            secret_deny > last_allow,
            "read_allow do usuário reexporia o segredo se o deny viesse antes"
        );
        let deny = lines[secret_deny];
        for secret in [
            r#"(subpath "/Users/nobody/.ssh")"#,
            r#"(subpath "/Users/nobody/.aws")"#,
            r#"(subpath "/Users/nobody/Library/Keychains")"#,
            r#"(subpath "/Users/nobody/Library/Application Support/dev.tyba.app")"#,
            r#"(regex #"^/Users/nobody/\.git-credentials")"#,
            r#"(literal "/var/run/docker.sock")"#,
        ] {
            assert!(deny.contains(secret), "faltou deny de {secret}");
        }
    }

    #[test]
    fn attached_main_checkout_can_never_move_main_master_or_packed_refs() {
        let mut s = spec();
        // Sessão anexada ao checkout principal: gitdir do worktree == .git do repo,
        // então o allow de escrita do gitdir alcançaria refs/heads/main.
        s.worktree_git_dir = s.repo_git_dir.clone();
        let policy = build_policy(&s);
        let lines: Vec<&str> = policy.lines().collect();
        let deny = lines
            .iter()
            .rposition(|l| l.starts_with("(deny file-write*"))
            .unwrap();
        let last_allow = lines
            .iter()
            .rposition(|l| l.starts_with("(allow file-write*"))
            .unwrap();
        assert!(deny > last_allow, "o deny precisa ser a última palavra");

        let line = lines[deny];
        for frag in [
            r#"(subpath "/private/repo/.git/refs/heads/main")"#,
            r#"(subpath "/private/repo/.git/refs/heads/master")"#,
            r#"(subpath "/private/repo/.git/logs/refs/heads/main")"#,
            r#"(subpath "/private/repo/.git/packed-refs")"#,
        ] {
            assert!(line.contains(frag), "faltou deny de {frag}");
        }
        assert!(
            !line.contains(r#"(subpath "/private/repo/.git/refs/heads")"#),
            "negar refs/heads inteiro tira o commit de qualquer branch — o agente pararia de funcionar"
        );
    }

    #[test]
    fn tyba_binary_and_configs_are_never_writable() {
        let policy = build_policy(&spec());
        let deny = policy
            .lines()
            .find(|l| l.starts_with("(deny file-write*"))
            .unwrap();
        assert!(deny.contains("/Apps/Tyba.app"));
        assert!(deny.contains("/Users/nobody/.tyba/config.toml"));
        assert!(deny.contains("/private/repo/.tyba/config.toml"));
        assert!(deny.contains("/private/wt/session-a/.tyba/config.toml"));
        assert!(deny.contains("/private/tmp/tyba-abc"));
    }

    #[test]
    fn policy_excludes_other_tyba_runtime_dirs_but_reads_own() {
        let policy = build_policy(&spec());
        assert!(policy.contains(r#"(regex #"^/private/tmp/tyba-")"#));
        assert!(policy.contains(r#"(subpath "/private/tmp/tyba-abc")"#));
    }

    #[test]
    fn policy_tmpdir_grant_never_reaches_sibling_runtime_dirs() {
        let policy = build_policy(&spec());
        let tmp_lines: Vec<&str> = policy
            .lines()
            .filter(|l| l.contains("/private/var/folders/xx/T"))
            .collect();
        assert_eq!(tmp_lines.len(), 2);
        for line in tmp_lines {
            assert!(
                line.contains(r#"(regex #"^/private/var/folders/xx/T/tyba-")"#),
                "{line}"
            );
        }
    }

    #[test]
    fn policy_toolchain_read_excludes_credential_files() {
        let policy = build_policy(&spec());
        let line = policy
            .lines()
            .find(|l| l.contains(".cargo\"") && l.contains("file-read*"))
            .unwrap();
        assert!(line.contains(".cargo/credentials"));
        assert!(line.contains(".git-credentials"));
        assert!(line.contains(".netrc"));
    }

    #[test]
    fn policy_skips_exec_path_dirs_that_swallow_home() {
        let mut s = spec();
        s.exec_path_dirs = vec![PathBuf::from("/Users/nobody"), PathBuf::from("/Users")];
        let policy = build_policy(&s);
        for line in policy.lines().filter(|l| l.contains("file-read*")) {
            assert!(!line.contains("(subpath \"/Users\")"), "{line}");
        }
    }

    #[test]
    fn policy_reads_tyba_app_bundle_for_hook_exec() {
        let policy = build_policy(&spec());
        assert!(policy.contains(r#"(literal "/Apps/Tyba.app/Contents/MacOS/tyba")"#));
        assert!(policy.contains(r#"(subpath "/Apps/Tyba.app")"#));
    }

    #[test]
    fn wrap_prefixes_sandbox_exec_and_marks_env() {
        let mut cmd = CommandBuilder::new("claude");
        cmd.arg("--settings");
        cmd.arg("/x/hooks.json");
        cmd.cwd("/private/wt/session-a");
        let wrapped = SeatbeltSandbox.wrap(cmd, &spec()).unwrap();
        let argv: Vec<String> = wrapped
            .get_argv()
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(argv[0], SANDBOX_EXEC);
        assert_eq!(argv[1], "-p");
        assert!(argv[2].contains("(deny default)"));
        assert_eq!(argv[3], "--");
        assert_eq!(&argv[4..], &["claude", "--settings", "/x/hooks.json"]);
        assert_eq!(
            wrapped.get_env("TYBA_SANDBOX"),
            Some(std::ffi::OsStr::new("seatbelt"))
        );
        assert_eq!(
            wrapped.get_cwd(),
            Some(&std::ffi::OsString::from("/private/wt/session-a"))
        );
    }

    #[test]
    fn wrap_refuses_empty_command() {
        let cmd = CommandBuilder::from_argv(vec![]);
        assert!(SeatbeltSandbox.wrap(cmd, &spec()).is_err());
    }
}
