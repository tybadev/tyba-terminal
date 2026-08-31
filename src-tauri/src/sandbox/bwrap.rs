use std::ffi::OsString;
use std::path::{Path, PathBuf};

use portable_pty::CommandBuilder;

use super::policy::{AgentAccess, Rule, RuleSet};
use super::SandboxSpec;

pub const BWRAP: &str = "/usr/bin/bwrap";
pub const SECCOMP_BPF_FILE: &str = "seccomp.bpf";
pub const SECCOMP_FD: &str = "3";
const SECCOMP_EXEC_MODE: &str = "_seccomp-exec";

const SYSTEM_RO_DIRS: [&str; 9] = [
    "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/etc", "/opt", "/sys",
];

const RESOLVER_RO_DIRS: [&str; 2] = ["/run/systemd/resolve", "/run/NetworkManager"];

use super::PROTECTED_BRANCHES;

struct Args(Vec<OsString>);

impl Args {
    fn flag(&mut self, f: &str) {
        self.0.push(f.into());
    }

    fn op(&mut self, op: &str, src: &Path, dest: &Path) {
        self.0.push(op.into());
        self.0.push(src.into());
        self.0.push(dest.into());
    }

    fn ro_bind(&mut self, p: &Path) {
        self.op("--ro-bind", p, p);
    }

    fn ro_bind_try(&mut self, p: &Path) {
        self.op("--ro-bind-try", p, p);
    }

    fn bind(&mut self, p: &Path) {
        self.op("--bind", p, p);
    }

    fn tmpfs(&mut self, p: &Path) {
        self.0.push("--tmpfs".into());
        self.0.push(p.into());
    }

    fn mask_file(&mut self, p: &Path) {
        self.op("--ro-bind", Path::new("/dev/null"), p);
    }
}

fn ensure_dir(p: &Path) -> Result<(), String> {
    std::fs::create_dir_all(p).map_err(|e| format!("preparação do sandbox ({}): {e}", p.display()))
}

fn ensure_file(p: &Path) -> Result<(), String> {
    if p.exists() {
        return Ok(());
    }
    if let Some(parent) = p.parent() {
        ensure_dir(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
        .map(|_| ())
        .map_err(|e| format!("preparação do sandbox ({}): {e}", p.display()))
}

/// Pré-criação obrigatória de um write-except (§3.4, M4): `--ro-bind` de um
/// path ausente ABORTA o bwrap (rc=1, "Can't find source path") — a sessão nem
/// sobe. Não é higiene, é condição de lançamento. Conteúdo inerte por
/// extensão: `{}` para JSON (é o que o dono do arquivo espera encontrar se
/// abrir fora do TYBA), vazio para o resto (CLAUDE.md); nunca `/dev/null`
/// (M4: isso funciona mas deixa um arquivo 0444 no host, ilegível para o
/// próprio dono depois que a jaula morre). Modo 0644.
fn ensure_inert_file(p: &Path) -> Result<(), String> {
    if p.exists() {
        return Ok(());
    }
    if let Some(parent) = p.parent() {
        ensure_dir(parent)?;
    }
    let content = if p.extension().and_then(|e| e.to_str()) == Some("json") {
        "{}"
    } else {
        ""
    };
    std::fs::write(p, content)
        .map_err(|e| format!("preparação do sandbox ({}): {e}", p.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o644))
            .map_err(|e| format!("preparação do sandbox ({}): {e}", p.display()))?;
    }
    Ok(())
}

fn expand_matches(rule: &Rule) -> Vec<PathBuf> {
    let p = rule.path();
    match rule {
        Rule::Family(_) | Rule::Prefix(_) => {
            let (Some(parent), Some(name)) = (p.parent(), p.file_name()) else {
                return Vec::new();
            };
            let name = name.to_string_lossy().into_owned();
            let Ok(entries) = std::fs::read_dir(parent) else {
                return Vec::new();
            };
            let mut out: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    match rule {
                        Rule::Prefix(_) => n.starts_with(&name),
                        _ => {
                            n == name
                                || n.strip_prefix(&name)
                                    .and_then(|rest| rest.chars().next())
                                    .map(|c| matches!(c, '-' | '.' | '~'))
                                    .unwrap_or(false)
                        }
                    }
                })
                .map(|e| e.path())
                .collect();
            out.sort();
            out
        }
        _ => vec![p.to_path_buf()],
    }
}

/// Onde, dentro do argv final, uma operação de `agent_access` pousa quando duas
/// disputam a MESMA profundidade de path (ordenação estável, ver `agent_access`).
/// A ordem numérica IMPORTA: bwrap aplica mounts em sequência e o último que
/// toca um path vence — então em profundidade igual quem vem depois nesta
/// enumeração cobre quem vem antes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Phase {
    ReadAllow = 0,
    WriteAllow = 1,
    ReadExcept = 2,
    WriteExcept = 3,
}

#[derive(Debug)]
enum MountOp {
    RoBind(PathBuf),
    Bind(PathBuf),
    Tmpfs(PathBuf),
    MaskFile(PathBuf),
}

fn path_depth(p: &Path) -> usize {
    p.components().count()
}

fn collect_read(pending: &mut Vec<(usize, Phase, MountOp)>, set: &RuleSet) {
    for rule in &set.allow {
        match rule {
            Rule::Subpath(p) | Rule::Node(p) => {
                if p.exists() {
                    pending.push((path_depth(p), Phase::ReadAllow, MountOp::RoBind(p.clone())));
                }
            }
            Rule::Literal(p) => {
                if p.is_file() {
                    pending.push((path_depth(p), Phase::ReadAllow, MountOp::RoBind(p.clone())));
                }
            }
            Rule::Family(_) | Rule::Prefix(_) => {
                for m in expand_matches(rule) {
                    pending.push((path_depth(&m), Phase::ReadAllow, MountOp::RoBind(m)));
                }
            }
        }
    }
    for rule in &set.except {
        for m in expand_matches(rule) {
            if m.is_dir() {
                pending.push((path_depth(&m), Phase::ReadExcept, MountOp::Tmpfs(m)));
            } else if m.exists() {
                pending.push((path_depth(&m), Phase::ReadExcept, MountOp::MaskFile(m)));
            }
        }
    }
}

/// Diferença deliberada de `Rule::Node` vs `Rule::Subpath` no except de escrita
/// (§2/§3.4 da entrega B): `Node` continua LEGÍVEL — vira `--ro-bind` do
/// diretório real, porque é config/hook que o agente ainda precisa ler (só não
/// pode reescrever). `Subpath` é isolamento total (hoje só `projects/`, furo
/// F4) — vira `--tmpfs`, igual ao except de leitura, porque o problema ali não
/// é só escrita, é visibilidade cruzada entre repos.
fn collect_write(pending: &mut Vec<(usize, Phase, MountOp)>, set: &RuleSet) -> Result<(), String> {
    for rule in &set.allow {
        match rule {
            Rule::Subpath(p) | Rule::Node(p) => {
                ensure_dir(p)?;
                pending.push((path_depth(p), Phase::WriteAllow, MountOp::Bind(p.clone())));
            }
            Rule::Literal(p) => {
                if p.is_file() {
                    pending.push((path_depth(p), Phase::WriteAllow, MountOp::Bind(p.clone())));
                } else if !p.exists() {
                    ensure_dir(p)?;
                }
            }
            Rule::Family(_) | Rule::Prefix(_) => {
                for m in expand_matches(rule) {
                    if m.is_file() {
                        pending.push((path_depth(&m), Phase::WriteAllow, MountOp::Bind(m)));
                    }
                }
            }
        }
    }
    for rule in &set.except {
        match rule {
            Rule::Subpath(p) => {
                // Bwrap cria o mountpoint do tmpfs sozinho mesmo se ausente (M4) —
                // sem pré-criação aqui, ao contrário dos outros dois braços.
                pending.push((path_depth(p), Phase::WriteExcept, MountOp::Tmpfs(p.clone())));
            }
            Rule::Node(p) => {
                // M4: --ro-bind de path inexistente ABORTA o bwrap — a sessão nem
                // sobe. Por isso o mountpoint é condição de lançamento, não higiene.
                ensure_dir(p)?;
                pending.push((path_depth(p), Phase::WriteExcept, MountOp::RoBind(p.clone())));
            }
            Rule::Literal(p) => {
                ensure_inert_file(p)?;
                pending.push((path_depth(p), Phase::WriteExcept, MountOp::RoBind(p.clone())));
            }
            Rule::Family(_) | Rule::Prefix(_) => {
                for m in expand_matches(rule) {
                    if m.exists() {
                        pending.push((path_depth(&m), Phase::WriteExcept, MountOp::RoBind(m)));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Entrega B / M1: `~/.claude` vira `--bind` rw (write allow), e isso é RASO —
/// menos componentes de path que qualquer sombra dentro dele. Bwrap aplica
/// mounts na ordem do argv, e um mount raso aplicado DEPOIS de um mount fundo
/// enterra esse mount fundo (a view inteira daquele path é substituída). Por
/// isso as operações de `read` e `write` não podem mais ser emitidas em dois
/// laços sequenciais (o de antes: todo `read` primeiro, todo `write` depois) —
/// têm de ser coletadas juntas e emitidas ordenadas por (profundidade
/// crescente, depois fase), ordenação ESTÁVEL para manter o determinismo dos
/// testes de argv (M1, medido em bwrap real). Mounts fora de `agent_access`
/// (git/worktree, segredos, read_allow_extra) não entram nesta ordenação — eles
/// continuam na posição sequencial de sempre em `build_args`.
fn agent_access(args: &mut Args, access: &AgentAccess) -> Result<(), String> {
    let mut pending: Vec<(usize, Phase, MountOp)> = Vec::new();
    for set in &access.read {
        collect_read(&mut pending, set);
    }
    for set in &access.write {
        collect_write(&mut pending, set)?;
    }
    pending.sort_by_key(|(depth, phase, _)| (*depth, *phase));
    for (_, _, op) in pending {
        match op {
            MountOp::RoBind(p) => args.ro_bind(&p),
            MountOp::Bind(p) => args.bind(&p),
            MountOp::Tmpfs(p) => args.tmpfs(&p),
            MountOp::MaskFile(p) => args.mask_file(&p),
        }
    }
    Ok(())
}

fn exec_path_dirs(args: &mut Args, spec: &SandboxSpec) {
    let home = crate::repo::canonicalize_or(&spec.home);
    let mut seen = std::collections::HashSet::new();
    for dir in &spec.exec_path_dirs {
        if !dir.is_absolute() {
            continue;
        }
        let canon = crate::repo::canonicalize_or(dir);
        if home.starts_with(&canon) {
            continue;
        }
        if seen.insert(canon.clone()) {
            args.ro_bind_try(&canon);
        }
    }
}

fn secret_shadow_dirs(spec: &SandboxSpec) -> Vec<PathBuf> {
    let home = &spec.home;
    let mut dirs = vec![
        home.join(".ssh"),
        home.join(".aws"),
        home.join(".gnupg"),
        home.join(".kube"),
        home.join(".config/gcloud"),
        home.join(".config/gh"),
        home.join(".local/share/keyrings"),
        home.join(".docker"),
        home.join(".colima"),
        home.join(".orbstack"),
        home.join(".rd"),
        home.join(".lima"),
        home.join(".local/share/containers"),
        home.join(".local/share/nerdctl"),
        home.join(".config/containers"),
        spec.tyba_data_dir.clone(),
        home.join(".tyba"),
    ];
    dirs.dedup();
    dirs
}

fn secret_shadow_files(spec: &SandboxSpec) -> Vec<Rule> {
    let home = &spec.home;
    vec![
        Rule::Prefix(home.join(".git-credentials")),
        Rule::Prefix(home.join(".netrc")),
        Rule::Prefix(home.join(".npmrc")),
        Rule::Prefix(home.join(".pypirc")),
        Rule::Prefix(home.join(".cargo/credentials")),
    ]
}

fn worktree_mounts(args: &mut Args, spec: &SandboxSpec) {
    args.bind(&spec.writable_root);
    let pointer = spec.writable_root.join(".git");
    if pointer.is_file() {
        args.ro_bind(&pointer);
    }
    let dot_tyba = spec.writable_root.join(".tyba");
    if dot_tyba.exists() {
        args.ro_bind(&dot_tyba);
    }
}

fn shadow_swallows_worktree(spec: &SandboxSpec) -> bool {
    secret_shadow_dirs(spec)
        .iter()
        .any(|dir| dir.is_dir() && spec.writable_root.starts_with(dir))
}

fn worktree_git(args: &mut Args, spec: &SandboxSpec) -> Result<(), String> {
    worktree_mounts(args, spec);

    ensure_dir(&spec.worktree_git_dir)?;
    args.bind(&spec.worktree_git_dir);
    for dir in ["hooks", "info"] {
        let p = spec.worktree_git_dir.join(dir);
        ensure_dir(&p)?;
        args.ro_bind(&p);
    }
    for file in ["config", "config.worktree"] {
        let p = spec.worktree_git_dir.join(file);
        ensure_file(&p)?;
        args.ro_bind(&p);
    }

    for dir in ["objects", "refs/heads/tyba", "logs/refs/heads/tyba"] {
        let p = spec.repo_git_dir.join(dir);
        ensure_dir(&p)?;
        args.bind(&p);
    }

    // Sessão anexada ao checkout principal (review/conflitos num repo comum): ali
    // o gitdir do worktree É o `.git` do repo, então o bind rw acima alcançaria
    // `refs/heads/main`. O agente não daria push — o core recusa —, mas moveria a
    // main LOCAL, que é a mesma regra vista de outro ângulo. Num worktree gerido
    // esses paths já ficam fora do bind; aqui o ro-bind é o que segura.
    for name in PROTECTED_BRANCHES {
        let head = spec.repo_git_dir.join("refs/heads").join(name);
        if head.is_file() {
            args.ro_bind(&head);
        }
        let log = spec.repo_git_dir.join("logs/refs/heads").join(name);
        if log.is_file() {
            args.ro_bind(&log);
        }
    }
    // Sem isso a main volta a ser reescrevível por outro caminho: `git pack-refs`
    // move as refs pra dentro deste arquivo, e editá-lo reescreve qualquer branch.
    let packed = spec.repo_git_dir.join("packed-refs");
    if packed.is_file() {
        args.ro_bind(&packed);
    }
    Ok(())
}

pub fn build_args(spec: &SandboxSpec) -> Result<Vec<OsString>, String> {
    let mut args = Args(Vec::new());
    let home = &spec.home;

    args.flag("--die-with-parent");
    for ns in [
        "--unshare-user",
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-cgroup-try",
    ] {
        args.flag(ns);
    }
    if !spec.allow_network {
        args.flag("--unshare-net");
    }

    args.0.push("--proc".into());
    args.0.push("/proc".into());
    args.0.push("--dev".into());
    args.0.push("/dev".into());

    for dir in SYSTEM_RO_DIRS {
        args.ro_bind_try(Path::new(dir));
    }
    for dir in RESOLVER_RO_DIRS {
        args.ro_bind_try(Path::new(dir));
    }

    args.tmpfs(Path::new("/tmp"));
    if let Some(tmpdir) = spec.tmpdir.as_deref() {
        let canon = crate::repo::canonicalize_or(tmpdir);
        if !canon.starts_with("/tmp") {
            args.tmpfs(&canon);
        }
    }
    args.tmpfs(&home.join(".cache"));

    args.ro_bind_try(&home.join(".gitconfig"));
    args.ro_bind_try(&home.join(".config/git"));
    for dir in super::TOOLCHAIN_HOME_DIRS {
        args.ro_bind_try(&home.join(dir));
    }

    exec_path_dirs(&mut args, spec);

    args.ro_bind(&spec.tyba_exe);
    if let Some(parent) = spec.tyba_exe.parent() {
        args.ro_bind_try(parent);
    }

    args.ro_bind(&spec.readable_root);
    if spec.repo_git_dir.exists() {
        args.ro_bind(&spec.repo_git_dir);
    }

    worktree_git(&mut args, spec)?;

    args.ro_bind(&spec.runtime_dir);
    if spec.hook_socket.exists() {
        args.op("--bind", &spec.hook_socket, &spec.hook_socket);
    }

    for extra in &spec.read_allow_extra {
        args.ro_bind_try(extra);
    }

    agent_access(&mut args, &spec.agent)?;

    for dir in secret_shadow_dirs(spec) {
        if dir.is_dir() {
            args.tmpfs(&dir);
        }
    }
    for rule in secret_shadow_files(spec) {
        for m in expand_matches(&rule) {
            if m.is_file() {
                args.mask_file(&m);
            }
        }
    }

    if shadow_swallows_worktree(spec) {
        worktree_mounts(&mut args, spec);
    }

    let lsp_root = spec.tyba_data_dir.join("lsp");
    for carved in &spec.data_dir_reads {
        let is_real_dir = std::fs::symlink_metadata(carved)
            .map(|m| m.is_dir())
            .unwrap_or(false);
        if carved.starts_with(&lsp_root) && is_real_dir {
            args.ro_bind(carved);
        }
    }

    args.0.push("--remount-ro".into());
    args.0.push("/".into());
    args.0.push("--seccomp".into());
    args.0.push(SECCOMP_FD.into());

    Ok(args.0)
}

pub fn wrap_command(
    mut cmd: CommandBuilder,
    spec: &SandboxSpec,
    bpf_path: &Path,
) -> Result<CommandBuilder, String> {
    let bwrap_args = build_args(spec)?;
    let argv = cmd.get_argv_mut();
    if argv.is_empty() {
        return Err("comando de agente vazio".into());
    }
    let mut wrapped: Vec<OsString> = vec![
        spec.tyba_exe.clone().into(),
        SECCOMP_EXEC_MODE.into(),
        bpf_path.into(),
        "--".into(),
        BWRAP.into(),
    ];
    wrapped.extend(bwrap_args);
    wrapped.push("--".into());
    wrapped.append(argv);
    *argv = wrapped;
    cmd.env("TYBA_SANDBOX", "bwrap");
    Ok(cmd)
}

pub fn maybe_run_seccomp_exec() -> Option<i32> {
    if std::env::args().nth(1).as_deref() != Some(SECCOMP_EXEC_MODE) {
        return None;
    }
    Some(run_seccomp_exec())
}

#[cfg(unix)]
fn run_seccomp_exec() -> i32 {
    use std::os::unix::process::CommandExt;

    let args: Vec<OsString> = std::env::args_os().collect();
    let (Some(bpf_path), Some(sep)) = (args.get(2), args.get(3)) else {
        eprintln!("uso: tyba {SECCOMP_EXEC_MODE} <bpf> -- <programa> [args...]");
        return 126;
    };
    if sep != "--" || args.len() < 5 {
        eprintln!("uso: tyba {SECCOMP_EXEC_MODE} <bpf> -- <programa> [args...]");
        return 126;
    }
    let file = match std::fs::File::open(bpf_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("filtro seccomp ilegível: {e}");
            return 126;
        }
    };
    let target_fd: i32 = SECCOMP_FD.parse().unwrap_or(3);
    {
        use std::os::unix::io::AsRawFd;
        let raw = file.as_raw_fd();
        let rc = if raw == target_fd {
            unsafe { libc::fcntl(raw, libc::F_SETFD, 0) }
        } else {
            unsafe { libc::dup2(raw, target_fd) }
        };
        if rc < 0 {
            eprintln!("fd do filtro seccomp: {}", std::io::Error::last_os_error());
            return 126;
        }
    }
    let err = std::process::Command::new(&args[4]).args(&args[5..]).exec();
    eprintln!("exec do sandbox falhou: {err}");
    126
}

#[cfg(not(unix))]
fn run_seccomp_exec() -> i32 {
    eprintln!("modo {SECCOMP_EXEC_MODE} indisponível nesta plataforma");
    126
}

/// O bwrap existe E o kernel deixa ele criar user namespace? Distro com AppArmor
/// restritivo (Ubuntu 24.04) ou kernel hardened responde não. Cacheado: a jaula do
/// git consulta isto por operação e não pode pagar um subprocesso toda vez.
#[cfg(target_os = "linux")]
pub fn userns_usable() -> bool {
    static USABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *USABLE.get_or_init(|| probe_bwrap().is_ok())
}

#[cfg(target_os = "linux")]
fn probe_bwrap() -> Result<(), String> {
    if !Path::new(BWRAP).is_file() {
        return Err(format!(
            "{BWRAP} não existe — instale o pacote bubblewrap para sessões de agente"
        ));
    }
    let probe = std::process::Command::new(BWRAP)
        .args(["--unshare-user", "--ro-bind", "/", "/", "true"])
        .output()
        .map_err(|e| format!("probe do bubblewrap falhou: {e}"))?;
    if !probe.status.success() {
        return Err(format!(
            "bubblewrap não consegue criar user namespace neste sistema — sessão de agente \
             recusada (fail-closed). Habilite user namespaces sem privilégio \
             (kernel.apparmor_restrict_unprivileged_userns=0 no Ubuntu 24.04). Detalhe: {}",
            String::from_utf8_lossy(&probe.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub struct BwrapSandbox;

#[cfg(target_os = "linux")]
impl BwrapSandbox {
    pub fn new() -> Result<Self, String> {
        probe_bwrap()?;
        Ok(BwrapSandbox)
    }
}

#[cfg(target_os = "linux")]
impl super::Sandbox for BwrapSandbox {
    fn wrap(&self, cmd: CommandBuilder, spec: &SandboxSpec) -> Result<CommandBuilder, String> {
        let bpf = super::seccomp::compile()?;
        let bpf_path = spec.runtime_dir.join(SECCOMP_BPF_FILE);
        crate::session::write_private_bytes(&spec.runtime_dir, SECCOMP_BPF_FILE, &bpf)
            .map_err(|e| format!("escrita do filtro seccomp: {e}"))?;
        wrap_command(cmd, spec, &bpf_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::policy::AgentAccess;

    fn fixture() -> (tempfile::TempDir, SandboxSpec) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let home = root.join("home");
        let repo = root.join("repo");
        let wt = root.join("wt/session-a");
        let runtime = root.join("tmp/tyba-abc");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::create_dir_all(home.join(".cargo")).unwrap();
        std::fs::write(home.join(".cargo/credentials.toml"), "tok").unwrap();
        std::fs::create_dir_all(repo.join(".git/worktrees/session-a")).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            wt.join(".git"),
            "gitdir: ../../repo/.git/worktrees/session-a",
        )
        .unwrap();
        std::fs::create_dir_all(&runtime).unwrap();
        let spec = SandboxSpec {
            writable_root: wt,
            readable_root: repo.clone(),
            allow_network: true,
            repo_git_dir: repo.join(".git"),
            worktree_git_dir: repo.join(".git/worktrees/session-a"),
            runtime_dir: runtime.clone(),
            hook_socket: runtime.join("hook.sock"),
            tyba_exe: root.join("bin/tyba"),
            tyba_data_dir: home.join(".local/share/dev.tyba.app"),
            home,
            tmpdir: None,
            exec_path_dirs: vec![],
            agent: AgentAccess::default(),
            read_allow_extra: vec![],
            data_dir_reads: vec![],
        };
        std::fs::create_dir_all(spec.tyba_exe.parent().unwrap()).unwrap();
        std::fs::write(&spec.tyba_exe, "").unwrap();
        std::fs::write(&spec.hook_socket, "").unwrap();
        (tmp, spec)
    }

    fn managed_fixture() -> (tempfile::TempDir, SandboxSpec) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let home = root.join("home");
        let repo = root.join("repo");
        let worktrees = home.join(".tyba/worktrees/repo");
        let wt = worktrees.join("task-a");
        let sibling = worktrees.join("task-b");
        let runtime = root.join("tmp/tyba-abc");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(home.join(".tyba/config.toml"), "secret").unwrap();
        std::fs::create_dir_all(repo.join(".git/worktrees/task-a")).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: ../../../../repo/.git").unwrap();
        std::fs::create_dir_all(&runtime).unwrap();
        let spec = SandboxSpec {
            writable_root: wt,
            readable_root: repo.clone(),
            allow_network: true,
            repo_git_dir: repo.join(".git"),
            worktree_git_dir: repo.join(".git/worktrees/task-a"),
            runtime_dir: runtime.clone(),
            hook_socket: runtime.join("hook.sock"),
            tyba_exe: root.join("bin/tyba"),
            tyba_data_dir: home.join(".local/share/dev.tyba.app"),
            home,
            tmpdir: None,
            exec_path_dirs: vec![],
            agent: AgentAccess::default(),
            read_allow_extra: vec![],
            data_dir_reads: vec![],
        };
        std::fs::create_dir_all(spec.tyba_exe.parent().unwrap()).unwrap();
        std::fs::write(&spec.tyba_exe, "").unwrap();
        std::fs::write(&spec.hook_socket, "").unwrap();
        (tmp, spec)
    }

    fn strs(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn tmpfs_pos(argv: &[String], p: &std::path::Path) -> Option<usize> {
        let s = p.to_string_lossy();
        argv.windows(2).position(|w| w[0] == "--tmpfs" && w[1] == s)
    }

    #[test]
    fn managed_worktree_survives_the_dot_tyba_shadow() {
        let (_tmp, spec) = managed_fixture();
        let argv = strs(&build_args(&spec).unwrap());
        let dot_tyba = spec.home.join(".tyba");
        let wt = spec.writable_root.to_string_lossy().into_owned();

        let shadow = tmpfs_pos(&argv, &dot_tyba).expect("~/.tyba precisa continuar sombreado");
        let last_bind = argv
            .windows(3)
            .rposition(|w| w[0] == "--bind" && w[1] == wt && w[2] == wt)
            .expect("o worktree precisa estar montado rw");

        assert!(
            last_bind > shadow,
            "o tmpfs de ~/.tyba é o último mount sobre o worktree: a jaula enterra o \
             diretório onde o agente ia trabalhar (bwrap aplica mounts em ordem)"
        );
    }

    #[test]
    fn worktree_pointer_stays_ro_after_the_shadow_is_undone() {
        let (_tmp, spec) = managed_fixture();
        let argv = strs(&build_args(&spec).unwrap());
        let wt = spec.writable_root.to_string_lossy().into_owned();
        let pointer = spec
            .writable_root
            .join(".git")
            .to_string_lossy()
            .into_owned();

        let last_bind = argv
            .windows(3)
            .rposition(|w| w[0] == "--bind" && w[1] == wt && w[2] == wt)
            .unwrap();
        let last_pointer = argv
            .windows(3)
            .rposition(|w| w[0] == "--ro-bind" && w[1] == pointer && w[2] == pointer)
            .expect("o ponteiro .git do worktree precisa ser ro");

        assert!(
            last_pointer > last_bind,
            "o re-bind do worktree devolveu o ponteiro .git como gravável"
        );
    }

    #[test]
    fn worktree_outside_home_is_not_rebound() {
        let (_tmp, spec) = fixture();
        let argv = strs(&build_args(&spec).unwrap());
        let wt = spec.writable_root.to_string_lossy().into_owned();
        let binds = argv
            .windows(3)
            .filter(|w| w[0] == "--bind" && w[1] == wt && w[2] == wt)
            .count();
        assert_eq!(
            binds, 1,
            "sem shadow engolindo o worktree, não há o que refazer"
        );
    }

    #[test]
    fn dot_tyba_shadow_still_hides_the_other_sessions() {
        let (_tmp, spec) = managed_fixture();
        let argv = strs(&build_args(&spec).unwrap());
        let sibling = spec.home.join(".tyba/worktrees/repo/task-b");
        let config = spec.home.join(".tyba/config.toml");

        assert!(
            !argv.iter().any(|a| a == &sibling.to_string_lossy()),
            "o worktree de outra sessão não pode entrar na jaula"
        );
        assert!(
            !argv.iter().any(|a| a == &config.to_string_lossy()),
            "o config do TYBA não pode entrar na jaula"
        );
    }

    fn triple_pos(argv: &[String], op: &str, src: &str, dest: &str) -> Option<usize> {
        argv.windows(3)
            .position(|w| w[0] == op && w[1] == src && w[2] == dest)
    }

    fn has_triple(argv: &[String], op: &str, path: &std::path::Path) -> bool {
        let s = path.to_string_lossy();
        triple_pos(argv, op, &s, &s).is_some()
    }

    #[test]
    fn unshares_everything_and_keeps_network_when_allowed() {
        let (_tmp, spec) = fixture();
        let argv = strs(&build_args(&spec).unwrap());
        for ns in [
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--unshare-cgroup-try",
            "--die-with-parent",
        ] {
            assert!(argv.contains(&ns.to_string()), "faltou {ns}");
        }
        assert!(!argv.contains(&"--unshare-net".to_string()));

        let mut s = spec;
        s.allow_network = false;
        let argv = strs(&build_args(&s).unwrap());
        assert!(argv.contains(&"--unshare-net".to_string()));
    }

    #[test]
    fn home_is_never_bound_whole() {
        let (_tmp, spec) = fixture();
        let argv = strs(&build_args(&spec).unwrap());
        let home = spec.home.to_string_lossy().into_owned();
        for w in argv.windows(3) {
            if w[0].ends_with("bind") {
                assert_ne!(w[2], home, "home inteira montada: {w:?}");
            }
        }
    }

    #[test]
    fn resolver_paths_survive_etc_bind() {
        let (_tmp, spec) = fixture();
        let argv = strs(&build_args(&spec).unwrap());
        assert!(triple_pos(
            argv.as_slice(),
            "--ro-bind-try",
            "/run/systemd/resolve",
            "/run/systemd/resolve"
        )
        .is_some());
    }

    #[test]
    fn worktree_rw_with_git_pointer_ro_after() {
        let (_tmp, spec) = fixture();
        let argv = strs(&build_args(&spec).unwrap());
        let wt = spec.writable_root.to_string_lossy().into_owned();
        let pointer = format!("{wt}/.git");
        let rw = triple_pos(&argv, "--bind", &wt, &wt).unwrap();
        let ro = triple_pos(&argv, "--ro-bind", &pointer, &pointer).unwrap();
        assert!(
            ro > rw,
            "o ponteiro .git precisa sombrear o bind rw do worktree"
        );
    }

    #[test]
    fn gitdir_excepts_are_precreated_and_ro() {
        let (_tmp, spec) = fixture();
        let argv = strs(&build_args(&spec).unwrap());
        let gitdir = &spec.worktree_git_dir;
        let rw = triple_pos(
            &argv,
            "--bind",
            &gitdir.to_string_lossy(),
            &gitdir.to_string_lossy(),
        )
        .unwrap();
        for name in ["hooks", "info", "config", "config.worktree"] {
            let p = gitdir.join(name);
            assert!(p.exists(), "{name} não foi pré-criado");
            let ro = triple_pos(
                &argv,
                "--ro-bind",
                &p.to_string_lossy(),
                &p.to_string_lossy(),
            )
            .unwrap_or_else(|| panic!("faltou ro-bind de {name}"));
            assert!(ro > rw, "{name} precisa vir depois do bind rw do gitdir");
        }
    }

    #[test]
    fn repo_refs_writable_only_under_tyba_namespace() {
        let (_tmp, spec) = fixture();
        let argv = strs(&build_args(&spec).unwrap());
        assert!(has_triple(
            &argv,
            "--bind",
            &spec.repo_git_dir.join("objects")
        ));
        assert!(has_triple(
            &argv,
            "--bind",
            &spec.repo_git_dir.join("refs/heads/tyba")
        ));
        let refs_root = spec
            .repo_git_dir
            .join("refs")
            .to_string_lossy()
            .into_owned();
        assert!(
            triple_pos(&argv, "--bind", &refs_root, &refs_root).is_none(),
            "refs inteiro gravável deixa reescrever main"
        );
    }

    #[test]
    fn runtime_ro_and_only_hook_socket_rw() {
        let (_tmp, spec) = fixture();
        let argv = strs(&build_args(&spec).unwrap());
        assert!(has_triple(&argv, "--ro-bind", &spec.runtime_dir));
        assert!(has_triple(&argv, "--bind", &spec.hook_socket));
    }

    #[test]
    fn absent_hook_socket_is_not_bound() {
        let (_tmp, mut spec) = fixture();
        spec.hook_socket = spec.runtime_dir.join(".no-hook.sock");
        let argv = strs(&build_args(&spec).unwrap());
        assert!(
            !has_triple(&argv, "--bind", &spec.hook_socket),
            "socket inexistente (jaula do LSP) viraria --bind e o bwrap abortaria: {argv:?}"
        );
    }

    #[test]
    fn secrets_are_shadowed_even_when_user_allows_home() {
        let (_tmp, mut spec) = fixture();
        spec.read_allow_extra = vec![spec.home.clone()];
        let argv = strs(&build_args(&spec).unwrap());
        let home = spec.home.to_string_lossy().into_owned();
        let allow = triple_pos(&argv, "--ro-bind-try", &home, &home).unwrap();
        let ssh = spec.home.join(".ssh").to_string_lossy().into_owned();
        let shadow = argv
            .windows(2)
            .position(|w| w[0] == "--tmpfs" && w[1] == ssh)
            .unwrap();
        assert!(
            shadow > allow,
            "o tmpfs de ~/.ssh precisa vencer o read_allow do usuário"
        );
    }

    #[test]
    fn cargo_credentials_masked_with_dev_null() {
        let (_tmp, spec) = fixture();
        let argv = strs(&build_args(&spec).unwrap());
        let cred = spec
            .home
            .join(".cargo/credentials.toml")
            .to_string_lossy()
            .into_owned();
        assert!(
            triple_pos(&argv, "--ro-bind", "/dev/null", &cred).is_some(),
            "credentials do cargo precisa virar /dev/null"
        );
    }

    /// Entrega B (M1): com `~/.claude` virando `--bind` rw, o mount raso enterra
    /// qualquer sombra funda emitida ANTES dele (bwrap aplica mounts em ordem —
    /// mount raso por cima cobre o que tinha por baixo). A correção é emitir tudo
    /// que passa por `agent_access` ordenado por profundidade crescente,
    /// independente de vir de `read` ou de `write`. Este teste é adversarial:
    /// mistura um allow de write raso (`~/.claude`) com sombras fundas de read
    /// (`projects/`) e de write (`settings.json`), e um re-grant mais fundo
    /// ainda (`projects/<este>`) — a ordem final tem de ser rasa→funda sempre.
    #[test]
    fn claude_dir_e_rw_e_as_sombras_vem_depois() {
        let (_tmp, mut spec) = fixture();
        let claude = spec.home.join(".claude");
        let projects = claude.join("projects");
        let project = projects.join("-wt-session-a");
        let settings = claude.join("settings.json");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::write(&settings, "{}").unwrap();

        spec.agent = AgentAccess {
            read: vec![RuleSet {
                allow: vec![Rule::Subpath(claude.clone())],
                except: vec![Rule::Subpath(projects.clone())],
            }],
            write: vec![
                RuleSet {
                    allow: vec![Rule::Node(claude.clone())],
                    except: vec![Rule::Literal(settings.clone())],
                },
                RuleSet::allow(vec![Rule::Node(project.clone())]),
            ],
        };

        let argv = strs(&build_args(&spec).unwrap());
        let claude_s = claude.to_string_lossy().into_owned();
        let settings_s = settings.to_string_lossy().into_owned();
        let project_s = project.to_string_lossy().into_owned();

        let claude_rw = triple_pos(&argv, "--bind", &claude_s, &claude_s)
            .expect("~/.claude precisa virar bind rw");
        let projects_shadow =
            tmpfs_pos(&argv, &projects).expect("projects/ precisa continuar sombreado (tmpfs)");
        let settings_shadow = triple_pos(&argv, "--ro-bind", &settings_s, &settings_s)
            .expect("settings.json precisa continuar somente-leitura");
        let project_regrant = triple_pos(&argv, "--bind", &project_s, &project_s)
            .expect("o projeto atual precisa ser re-liberado");

        assert!(
            projects_shadow > claude_rw,
            "o bind rw de ~/.claude precisa vir ANTES do tmpfs de projects/, senão o mount \
             raso enterra a sombra funda que veio antes dele no argv: {argv:?}"
        );
        assert!(
            settings_shadow > claude_rw,
            "settings.json precisa continuar sombreado depois que ~/.claude vira rw: {argv:?}"
        );
        assert!(
            project_regrant > projects_shadow,
            "o re-grant do projeto atual precisa vir depois do tmpfs de projects/, por cima: {argv:?}"
        );
    }

    #[test]
    fn agent_access_shadows_then_regrants_project() {
        let (_tmp, mut spec) = fixture();
        let claude = spec.home.join(".claude");
        let projects = claude.join("projects");
        let project = projects.join("-wt-session-a");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::write(claude.join("history.jsonl"), "x").unwrap();
        spec.agent = AgentAccess {
            read: vec![RuleSet {
                allow: vec![Rule::Subpath(claude.clone())],
                except: vec![Rule::Subpath(projects.clone())],
            }],
            write: vec![RuleSet::allow(vec![Rule::Node(project.clone())])],
        };
        let argv = strs(&build_args(&spec).unwrap());
        let claude_s = claude.to_string_lossy().into_owned();
        let projects_s = projects.to_string_lossy().into_owned();
        let project_s = project.to_string_lossy().into_owned();
        let ro = triple_pos(&argv, "--ro-bind", &claude_s, &claude_s).unwrap();
        let shadow = argv
            .windows(2)
            .position(|w| w[0] == "--tmpfs" && w[1] == projects_s)
            .unwrap();
        let rw = triple_pos(&argv, "--bind", &project_s, &project_s).unwrap();
        assert!(
            shadow > ro && rw > shadow,
            "ordem allow → shadow → re-grant"
        );
        assert!(project.is_dir(), "pasta do projeto precisa ser pré-criada");
    }

    #[test]
    fn read_except_file_is_masked() {
        let (_tmp, mut spec) = fixture();
        let claude = spec.home.join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        let history = claude.join("history.jsonl");
        std::fs::write(&history, "x").unwrap();
        spec.agent = AgentAccess {
            read: vec![RuleSet {
                allow: vec![Rule::Subpath(claude)],
                except: vec![Rule::Literal(history.clone())],
            }],
            write: vec![],
        };
        let argv = strs(&build_args(&spec).unwrap());
        let h = history.to_string_lossy().into_owned();
        assert!(triple_pos(&argv, "--ro-bind", "/dev/null", &h).is_some());
    }

    #[test]
    fn family_rule_binds_existing_siblings_rw() {
        let (_tmp, mut spec) = fixture();
        std::fs::write(spec.home.join(".claude.json"), "{}").unwrap();
        std::fs::write(spec.home.join(".claude.json.backup"), "{}").unwrap();
        std::fs::write(spec.home.join(".claude.jsonNOT"), "{}").unwrap();
        spec.agent = AgentAccess {
            read: vec![],
            write: vec![RuleSet::allow(vec![Rule::Family(
                spec.home.join(".claude.json"),
            )])],
        };
        let argv = strs(&build_args(&spec).unwrap());
        assert!(has_triple(&argv, "--bind", &spec.home.join(".claude.json")));
        assert!(has_triple(
            &argv,
            "--bind",
            &spec.home.join(".claude.json.backup")
        ));
        assert!(!has_triple(
            &argv,
            "--bind",
            &spec.home.join(".claude.jsonNOT")
        ));
    }

    #[test]
    fn attached_main_checkout_cannot_move_main_but_can_move_a_feature_branch() {
        let (_tmp, mut spec) = fixture();
        // Sessão anexada ao checkout principal: gitdir do worktree == .git do repo.
        spec.worktree_git_dir = spec.repo_git_dir.clone();
        let heads = spec.repo_git_dir.join("refs/heads");
        std::fs::create_dir_all(&heads).unwrap();
        for b in ["main", "master", "feature"] {
            std::fs::write(heads.join(b), "sha\n").unwrap();
        }
        std::fs::write(spec.repo_git_dir.join("packed-refs"), "# pack\n").unwrap();

        let argv = strs(&build_args(&spec).unwrap());
        let gitdir = spec.repo_git_dir.to_string_lossy().into_owned();
        let rw = triple_pos(&argv, "--bind", &gitdir, &gitdir).unwrap();

        for protegido in ["main", "master"] {
            let p = heads.join(protegido);
            let ro = triple_pos(
                &argv,
                "--ro-bind",
                &p.to_string_lossy(),
                &p.to_string_lossy(),
            )
            .unwrap_or_else(|| panic!("refs/heads/{protegido} ficou gravável"));
            assert!(ro > rw, "o ro-bind precisa sombrear o bind rw do gitdir");
        }
        let packed = spec.repo_git_dir.join("packed-refs");
        assert!(
            has_triple(&argv, "--ro-bind", &packed),
            "packed-refs gravável reescreve a main por outro caminho (pack-refs)"
        );

        // Par positivo: proteger a main não pode custar o commit numa branch comum.
        let feature = heads.join("feature");
        assert!(
            triple_pos(
                &argv,
                "--ro-bind",
                &feature.to_string_lossy(),
                &feature.to_string_lossy()
            )
            .is_none(),
            "branch comum precisa continuar gravável — senão o agente não commita"
        );
    }

    #[test]
    fn remount_ro_and_seccomp_close_the_args() {
        let (_tmp, spec) = fixture();
        let argv = strs(&build_args(&spec).unwrap());
        let n = argv.len();
        assert_eq!(&argv[n - 4..], &["--remount-ro", "/", "--seccomp", "3"]);
    }

    #[test]
    fn exec_path_dirs_never_swallow_home() {
        let (_tmp, mut spec) = fixture();
        spec.exec_path_dirs = vec![spec.home.clone(), PathBuf::from("relative/bin")];
        let argv = strs(&build_args(&spec).unwrap());
        let home = spec.home.to_string_lossy().into_owned();
        assert!(triple_pos(&argv, "--ro-bind-try", &home, &home).is_none());
    }

    #[test]
    fn wrap_command_prefixes_helper_and_marks_env() {
        let (_tmp, spec) = fixture();
        let mut cmd = CommandBuilder::new("claude");
        cmd.arg("--settings");
        cmd.arg("/x/hooks.json");
        cmd.cwd(&spec.writable_root);
        let bpf = spec.runtime_dir.join(SECCOMP_BPF_FILE);
        let wrapped = wrap_command(cmd, &spec, &bpf).unwrap();
        let argv = strs(wrapped.get_argv());
        assert_eq!(argv[0], spec.tyba_exe.to_string_lossy());
        assert_eq!(argv[1], "_seccomp-exec");
        assert_eq!(argv[2], bpf.to_string_lossy());
        assert_eq!(argv[3], "--");
        assert_eq!(argv[4], BWRAP);
        let sep = argv.iter().rposition(|a| a == "--").unwrap();
        assert_eq!(&argv[sep + 1..], &["claude", "--settings", "/x/hooks.json"]);
        assert_eq!(
            wrapped.get_env("TYBA_SANDBOX"),
            Some(std::ffi::OsStr::new("bwrap"))
        );
    }

    #[test]
    fn wrap_command_refuses_empty_command() {
        let (_tmp, spec) = fixture();
        let cmd = CommandBuilder::from_argv(vec![]);
        assert!(wrap_command(cmd, &spec, Path::new("/x.bpf")).is_err());
    }
}
