use std::os::unix::io::IntoRawFd;
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use super::bwrap::{build_args, BwrapSandbox, BWRAP};
use super::policy::AgentAccess;
use super::SandboxSpec;

fn bwrap_unavailable(test: &str) -> bool {
    match BwrapSandbox::new() {
        Ok(_) => false,
        Err(e) => {
            eprintln!("SKIP {test}: {e}");
            true
        }
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    _listener: UnixListener,
    spec: SandboxSpec,
    sibling_worktree: PathBuf,
}

fn git(dir: &Path, home: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("HOME", home)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let home = root.join("home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/id_rsa"), "SEGREDO").unwrap();
    std::fs::write(home.join("marcador.txt"), "home ok").unwrap();
    let data_dir = home.join(".local/share/dev.tyba.app");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("tyba.db"), "CONSENT").unwrap();

    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&root, &home, &["init", "-b", "main", "repo"]);
    git(&repo, &home, &["commit", "--allow-empty", "-m", "raiz"]);
    let wt = root.join("wt-a");
    git(
        &repo,
        &home,
        &["worktree", "add", "-b", "tyba/s1", wt.to_str().unwrap()],
    );
    let sibling = root.join("wt-b");
    git(
        &repo,
        &home,
        &[
            "worktree",
            "add",
            "-b",
            "tyba/s2",
            sibling.to_str().unwrap(),
        ],
    );
    std::fs::write(sibling.join("segredo-vizinho.txt"), "x").unwrap();

    let git_dirs = crate::worktree::resolved_git_dirs(&wt).unwrap();
    let runtime = root.join("tyba-rt");
    std::fs::create_dir_all(&runtime).unwrap();
    let hook_socket = runtime.join("hook.sock");
    let listener = UnixListener::bind(&hook_socket).unwrap();

    let spec = SandboxSpec {
        writable_root: wt,
        readable_root: repo,
        allow_network: true,
        repo_git_dir: git_dirs.common_dir,
        worktree_git_dir: git_dirs.git_dir,
        runtime_dir: runtime,
        hook_socket,
        tyba_exe: PathBuf::from("/usr/bin/env"),
        tyba_data_dir: data_dir,
        home,
        tmpdir: None,
        exec_path_dirs: vec![],
        agent: AgentAccess::default(),
        read_allow_extra: vec![],
    };
    Fixture {
        _tmp: tmp,
        _listener: listener,
        spec,
        sibling_worktree: sibling,
    }
}

fn run_argv(spec: &SandboxSpec, argv: &[&str]) -> Output {
    let args = build_args(spec).unwrap();
    let bpf = super::seccomp::compile().unwrap();
    let bpf_file = spec.runtime_dir.join("test-seccomp.bpf");
    std::fs::write(&bpf_file, &bpf).unwrap();
    let raw = std::fs::File::open(&bpf_file).unwrap().into_raw_fd();

    let mut cmd = Command::new(BWRAP);
    cmd.args(&args);
    cmd.arg("--");
    cmd.args(argv);
    cmd.env_clear();
    cmd.env("PATH", "/usr/bin:/bin");
    cmd.env("HOME", &spec.home);
    cmd.env("GIT_AUTHOR_NAME", "t");
    cmd.env("GIT_AUTHOR_EMAIL", "t@t");
    cmd.env("GIT_COMMITTER_NAME", "t");
    cmd.env("GIT_COMMITTER_EMAIL", "t@t");
    unsafe {
        cmd.pre_exec(move || {
            let rc = if raw == 3 {
                libc::fcntl(3, libc::F_SETFD, 0)
            } else {
                libc::dup2(raw, 3)
            };
            if rc < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd.output().unwrap()
}

fn run_sh(spec: &SandboxSpec, script: &str) -> Output {
    run_argv(spec, &["/bin/sh", "-c", script])
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Um bwrap que não sobe faz TODO teste de deny passar em falso: o comando falha,
/// o arquivo protegido continua intacto, e a asserção fica verde sem que política
/// nenhuma tenha sido aplicada. Todo teste passa por aqui antes de concluir.
fn assert_cage_booted(out: &Output) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    for line in stderr.lines() {
        assert!(
            !line.starts_with("bwrap:"),
            "a jaula não chegou a subir — o resultado deste teste não prova nada: {line}"
        );
    }
}

#[test]
fn positive_worktree_is_writable_and_readable() {
    if bwrap_unavailable("positive_worktree_is_writable_and_readable") {
        return;
    }
    let f = fixture();
    let wt = f.spec.writable_root.display().to_string();
    let out = run_sh(
        &f.spec,
        &format!("echo conteudo > {wt}/novo.txt && cat {wt}/novo.txt"),
    );
    assert_cage_booted(&out);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout(&out), "conteudo");
    assert_eq!(
        std::fs::read_to_string(f.spec.writable_root.join("novo.txt")).unwrap(),
        "conteudo\n",
        "a escrita precisa atravessar pro host — par positivo do desenho"
    );
}

#[test]
fn git_pointer_is_not_writable() {
    if bwrap_unavailable("git_pointer_is_not_writable") {
        return;
    }
    let f = fixture();
    let pointer = f.spec.writable_root.join(".git");
    let before = std::fs::read_to_string(&pointer).unwrap();
    let out = run_sh(&f.spec, &format!("echo pwn > {}", pointer.display()));
    assert_cage_booted(&out);
    assert!(!out.status.success());
    assert_eq!(std::fs::read_to_string(&pointer).unwrap(), before);
}

#[test]
fn gitdir_hooks_are_not_creatable_but_gitdir_is_writable() {
    if bwrap_unavailable("gitdir_hooks_are_not_creatable_but_gitdir_is_writable") {
        return;
    }
    let f = fixture();
    let gitdir = f.spec.worktree_git_dir.display().to_string();
    let out = run_sh(&f.spec, &format!("echo pwn > {gitdir}/hooks/pre-commit"));
    assert_cage_booted(&out);
    assert!(
        !out.status.success(),
        "hook plantado = RCE no próximo git do dono"
    );
    assert!(!f.spec.worktree_git_dir.join("hooks/pre-commit").exists());

    let out = run_sh(
        &f.spec,
        &format!("touch {gitdir}/sonda && rm {gitdir}/sonda"),
    );
    assert_cage_booted(&out);
    assert!(
        out.status.success(),
        "gitdir precisa continuar gravável (par positivo): {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn git_commit_works_inside_the_cage() {
    if bwrap_unavailable("git_commit_works_inside_the_cage") {
        return;
    }
    let f = fixture();
    let wt = f.spec.writable_root.display().to_string();
    let out = run_sh(
        &f.spec,
        &format!("cd {wt} && echo x > f.txt && git add f.txt && git commit -m dentro && git rev-parse HEAD"),
    );
    assert_cage_booted(&out);
    assert!(
        out.status.success(),
        "commit é o mínimo vital do agente: {}\n{}",
        stdout(&out),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn main_branch_ref_is_not_writable() {
    if bwrap_unavailable("main_branch_ref_is_not_writable") {
        return;
    }
    let f = fixture();
    let main_ref = f.spec.repo_git_dir.join("refs/heads/main");
    let before = std::fs::read_to_string(&main_ref).unwrap();
    let wt = f.spec.writable_root.display().to_string();
    let out = run_sh(
        &f.spec,
        &format!("cd {wt} && git commit --allow-empty -m x && git update-ref refs/heads/main HEAD"),
    );
    assert_cage_booted(&out);
    assert!(!out.status.success(), "reescrever main de dentro da jaula");
    assert_eq!(std::fs::read_to_string(&main_ref).unwrap(), before);
}

#[test]
fn home_secrets_do_not_exist_by_default() {
    if bwrap_unavailable("home_secrets_do_not_exist_by_default") {
        return;
    }
    let f = fixture();
    let home = f.spec.home.display().to_string();
    let out = run_sh(
        &f.spec,
        &format!("test ! -e {home}/.ssh/id_rsa && test ! -e {home}/marcador.txt && echo LIMPO"),
    );
    assert_cage_booted(&out);
    assert!(out.status.success());
    assert_eq!(
        stdout(&out),
        "LIMPO",
        "home não montada não pode vazar nada"
    );
}

#[test]
fn read_allow_of_home_still_hides_secrets() {
    if bwrap_unavailable("read_allow_of_home_still_hides_secrets") {
        return;
    }
    let mut f = fixture();
    f.spec.read_allow_extra = vec![f.spec.home.clone()];
    let home = f.spec.home.display().to_string();
    let out = run_sh(
        &f.spec,
        &format!(
            "cat {home}/marcador.txt && test ! -e {home}/.ssh/id_rsa && test ! -e {home}/.local/share/dev.tyba.app/tyba.db && echo PROTEGIDO"
        ),
    );
    assert_cage_booted(&out);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout(&out).contains("home ok"), "o allow do dono funciona");
    assert!(stdout(&out).contains("PROTEGIDO"), "o deny vence o allow");
}

#[test]
fn sibling_worktree_is_invisible() {
    if bwrap_unavailable("sibling_worktree_is_invisible") {
        return;
    }
    let f = fixture();
    let out = run_sh(
        &f.spec,
        &format!("test ! -e {} && echo ISOLADO", f.sibling_worktree.display()),
    );
    assert_cage_booted(&out);
    assert!(out.status.success());
    assert_eq!(
        stdout(&out),
        "ISOLADO",
        "F6: sessão A não enxerga worktree da sessão B"
    );
}

#[test]
fn hook_socket_accepts_connection_from_inside() {
    if bwrap_unavailable("hook_socket_accepts_connection_from_inside") {
        return;
    }
    if !Path::new("/usr/bin/python3").exists() {
        eprintln!("SKIP hook_socket_accepts_connection_from_inside: sem python3");
        return;
    }
    let f = fixture();
    let listener = f._listener.try_clone().unwrap();
    let acceptor = std::thread::spawn(move || {
        use std::io::Read;
        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = [0u8; 2];
        conn.read_exact(&mut buf).unwrap();
        buf
    });
    let sock = f.spec.hook_socket.display().to_string();
    let out = run_argv(
        &f.spec,
        &[
            "/usr/bin/python3",
            "-c",
            &format!(
                "import socket\ns = socket.socket(socket.AF_UNIX)\ns.connect({sock:?})\ns.sendall(b'oi')\nprint('CONECTADO')"
            ),
        ],
    );
    assert_cage_booted(&out);
    assert!(
        out.status.success(),
        "F1: sem o socket o gate morre e todo tool use vira deny: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout(&out), "CONECTADO");
    assert_eq!(&acceptor.join().unwrap(), b"oi");
}

#[test]
fn host_processes_are_invisible_in_proc() {
    if bwrap_unavailable("host_processes_are_invisible_in_proc") {
        return;
    }
    let f = fixture();
    let out = run_sh(&f.spec, "ls /proc | grep -c '^[0-9][0-9]*$'");
    assert_cage_booted(&out);
    assert!(out.status.success());
    let count: usize = stdout(&out).parse().unwrap();
    assert!(
        count < 5,
        "pidns compartilhado deixa ler /proc/<pid>/environ de terceiros: {count} processos visíveis"
    );
    assert_cage_booted(&out);
}

#[test]
fn seccomp_denies_ptrace_with_eperm() {
    if bwrap_unavailable("seccomp_denies_ptrace_with_eperm") {
        return;
    }
    if !Path::new("/usr/bin/python3").exists() {
        eprintln!("SKIP seccomp_denies_ptrace_with_eperm: sem python3");
        return;
    }
    let f = fixture();
    let out = run_argv(
        &f.spec,
        &[
            "/usr/bin/python3",
            "-c",
            "import ctypes\nlibc = ctypes.CDLL(None, use_errno=True)\nr = libc.ptrace(0, 0, 0, 0)\nprint(r, ctypes.get_errno())",
        ],
    );
    assert_cage_booted(&out);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        stdout(&out),
        format!("-1 {}", libc::EPERM),
        "PTRACE_TRACEME sem filtro retorna 0; com filtro precisa ser EPERM"
    );
}

/// `/proc/net/dev` e `ip link` não servem de oráculo aqui: dentro do namespace o
/// procfs pode reusar um superbloco do host e listar as interfaces de fora mesmo
/// com a netns já trocada. O inode da netns é a fonte da verdade.
#[test]
fn unshare_net_puts_the_agent_in_a_fresh_netns() {
    if bwrap_unavailable("unshare_net_puts_the_agent_in_a_fresh_netns") {
        return;
    }
    let host_ns = std::fs::read_link("/proc/self/ns/net").unwrap();
    let host_ns = host_ns.to_string_lossy().into_owned();

    let mut f = fixture();
    f.spec.allow_network = false;
    let out = run_sh(&f.spec, "readlink /proc/self/ns/net");
    assert_cage_booted(&out);
    assert!(out.status.success());
    assert_ne!(
        stdout(&out),
        host_ns,
        "sem rede o agente precisa cair numa netns própria"
    );

    let mut f = fixture();
    f.spec.allow_network = true;
    let out = run_sh(&f.spec, "readlink /proc/self/ns/net");
    assert_cage_booted(&out);
    assert_eq!(
        stdout(&out),
        host_ns,
        "com rede o agente compartilha a netns do host (par positivo: agente sem rede não sobe)"
    );
}

#[test]
fn agent_without_network_cannot_reach_the_wire() {
    if bwrap_unavailable("agent_without_network_cannot_reach_the_wire") {
        return;
    }
    if !Path::new("/usr/bin/python3").exists() {
        eprintln!("SKIP agent_without_network_cannot_reach_the_wire: sem python3");
        return;
    }
    let mut f = fixture();
    f.spec.allow_network = false;
    let out = run_argv(
        &f.spec,
        &[
            "/usr/bin/python3",
            "-c",
            "import socket\ns = socket.socket()\ns.settimeout(3)\ntry:\n    s.connect(('1.1.1.1', 443))\n    print('ABERTA')\nexcept OSError as e:\n    print('BLOQUEADA', e.errno)",
        ],
    );
    assert_cage_booted(&out);
    assert!(
        stdout(&out).starts_with("BLOQUEADA"),
        "runner sem rede precisa falhar no connect, não só ter uma netns diferente: {}",
        stdout(&out)
    );
}

#[test]
fn tmp_is_fresh_and_writable() {
    if bwrap_unavailable("tmp_is_fresh_and_writable") {
        return;
    }
    let f = fixture();
    let canary = format!("/tmp/tyba-canary-{}", std::process::id());
    std::fs::write(&canary, "host").unwrap();
    let out = run_sh(
        &f.spec,
        &format!("test ! -e {canary} && echo dentro > /tmp/x && cat /tmp/x"),
    );
    assert_cage_booted(&out);
    let _ = std::fs::remove_file(&canary);
    assert!(out.status.success());
    assert_eq!(stdout(&out), "dentro", "/tmp novo por sessão, mas gravável");
}

#[test]
fn shell_gets_a_real_tty() {
    if bwrap_unavailable("shell_gets_a_real_tty") {
        return;
    }
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    let f = fixture();
    let mut args = build_args(&f.spec).unwrap();
    args.truncate(args.len() - 2);
    let mut argv: Vec<std::ffi::OsString> = vec![BWRAP.into()];
    argv.extend(args);
    argv.push("--".into());
    for a in ["/bin/sh", "-c", "test -t 0 && echo TTY_OK"] {
        argv.push(a.into());
    }
    let mut cmd = CommandBuilder::from_argv(argv);
    cmd.env("PATH", "/usr/bin:/bin");
    cmd.env("HOME", &f.spec.home);

    let pty = native_pty_system().openpty(PtySize::default()).unwrap();
    let mut child = pty.slave.spawn_command(cmd).unwrap();
    drop(pty.slave);
    let mut reader = pty.master.try_clone_reader().unwrap();
    let mut out = String::new();
    use std::io::Read;
    let _ = reader.read_to_string(&mut out);
    assert!(
        !out.contains("bwrap:"),
        "a jaula não subiu — o TTY não prova nada: {out}"
    );
    let status = child.wait().unwrap();
    assert!(status.success(), "saída: {out}");
    assert!(
        out.contains("TTY_OK"),
        "sem isatty o shell vira não-interativo e TUIs quebram: {out}"
    );
}
