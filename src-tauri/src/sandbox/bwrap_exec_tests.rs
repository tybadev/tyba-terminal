use std::os::unix::io::IntoRawFd;
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use super::bwrap::{build_args, BwrapSandbox, BWRAP};
use super::policy::AgentAccess;
use super::SandboxSpec;
use crate::agent::{AgentRunner, ClaudeCodeRunner};

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
        data_dir_reads: vec![],
    };
    Fixture {
        _tmp: tmp,
        _listener: listener,
        spec,
        sibling_worktree: sibling,
    }
}

/// Entrega B: `~/.claude` povoado como o do dono (V7/V8) — populado o
/// suficiente para exercitar cada linha do §2 (sombreados e não-sombreados) —
/// e `spec.agent` vem do `ClaudeCodeRunner` real (`sandbox_access`), não de
/// uma política de teste escrita à mão: o que está sob prova é a política de
/// produção, não uma paráfrase dela. Fixture SINTÉTICA (CLAUDE.md) — nenhum
/// conteúdo aqui vem de uma sessão real.
fn claude_fixture() -> Fixture {
    let mut f = fixture();
    let claude = f.spec.home.join(".claude");
    std::fs::create_dir_all(&claude).unwrap();

    std::fs::write(claude.join("settings.json"), r#"{"hooks":{}}"#).unwrap();
    std::fs::write(claude.join("settings.local.json"), "{}").unwrap();
    std::fs::write(claude.join("daemon.json"), "{}").unwrap();
    std::fs::create_dir_all(claude.join("daemon")).unwrap();
    std::fs::write(claude.join("daemon/schedule.json"), "{}").unwrap();
    std::fs::create_dir_all(claude.join("plugins/x")).unwrap();
    std::fs::write(claude.join("plugins/x/hook.sh"), "#!/bin/sh\necho pwn\n").unwrap();
    std::fs::create_dir_all(claude.join("cowork_plugins")).unwrap();
    std::fs::create_dir_all(claude.join("hooks")).unwrap();
    std::fs::write(claude.join("mcp.json"), "{}").unwrap();
    std::fs::create_dir_all(claude.join("agents")).unwrap();
    std::fs::create_dir_all(claude.join("commands")).unwrap();
    std::fs::create_dir_all(claude.join("skills")).unwrap();
    std::fs::create_dir_all(claude.join("output-styles")).unwrap();
    std::fs::create_dir_all(claude.join("rules")).unwrap();
    std::fs::create_dir_all(claude.join("workflows")).unwrap();
    std::fs::write(claude.join("CLAUDE.md"), "memória do dono").unwrap();
    // v0.6.2 (review de segurança r2, BLOCKING): remote-settings.json
    // voltou pra MANDATORY — sombreado sempre, presente aqui ou não (ver
    // `remote_settings_json_absent_is_pre_created_and_shadowed_read_only`
    // pro caso ausente). Criado aqui só pra exercitar o caso "já existia no
    // spawn" com conteúdo real, não `{}` de pré-criação.
    // `.config.json` de propósito NÃO nasce aqui: é o caso "ausente", coberto
    // por `config_json_absent_from_fixture_is_not_created_by_mounting_the_cage`.
    std::fs::write(claude.join("remote-settings.json"), "{}").unwrap();

    // V9: o nome real não está em nenhuma lista fixa — só a forma (executável)
    // classifica.
    std::fs::write(
        claude.join("statusline-command.sh"),
        "#!/bin/sh\necho status\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            claude.join("statusline-command.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }

    std::fs::write(claude.join("history.jsonl"), "linha-privada\n").unwrap();
    std::fs::create_dir_all(claude.join("projects/-other-repo")).unwrap();
    std::fs::write(
        claude.join("projects/-other-repo/secret.txt"),
        "segredo do outro repo",
    )
    .unwrap();

    f.spec.agent = ClaudeCodeRunner.sandbox_access(&f.spec.home, &f.spec.writable_root);
    f
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

// ---------------------------------------------------------------------------
// Entrega B — a credencial do agente sobrevive à jaula no Linux (§12, T1)
// ---------------------------------------------------------------------------

/// Contrato de cobertura item 2 / claim C3 parcial: a credencial usa o mesmo
/// padrão tmp+rename que o resto do binário (M2), e agora o pai é um bind rw
/// de verdade (não mountpoint do $HOME) — o rename funciona de ponta a ponta,
/// dentro da jaula, e o conteúdo chega ao host.
#[test]
fn credential_survives_atomic_rename_and_reaches_host() {
    if bwrap_unavailable("credential_survives_atomic_rename_and_reaches_host") {
        return;
    }
    let f = claude_fixture();
    let claude = f.spec.home.join(".claude");
    let cred = claude.join(".credentials.json");
    assert!(!cred.exists(), "fixture precisa nascer sem credencial");

    let out = run_sh(
        &f.spec,
        &format!(
            "echo '{{\"token\":\"novo\"}}' > {0}/.credentials.json.tmp.abc12345 && \
             mv {0}/.credentials.json.tmp.abc12345 {0}/.credentials.json && \
             cat {0}/.credentials.json",
            claude.display()
        ),
    );
    assert_cage_booted(&out);
    assert!(
        out.status.success(),
        "a credencial precisa sobreviver ao padrão tmp+rename do próprio Claude Code: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout(&out).contains("novo"));
    let on_host = std::fs::read_to_string(&cred).unwrap();
    assert!(
        on_host.contains("novo"),
        "a escrita atômica precisa chegar ao host: {on_host}"
    );
}

/// Contrato de cobertura itens 3, 4 e 5: settings.json, settings.local.json,
/// daemon.json, daemon/, plugins/, plugins/x/hook.sh, hooks/, mcp.json,
/// agents/, commands/, skills/, output-styles/, rules/, workflows/, CLAUDE.md
/// e o statusline-command.sh do dono (V9, achado por forma, não por nome) —
/// nenhum gravável, mesmo com `~/.claude` inteiro virando bind rw. Inclui
/// caminhos que NÃO existiam na fixture (daemon/novo.json,
/// plugins/novo-plugin.json): o diretório sombreado bloqueia criar filho
/// novo, não só reescrever o que já tinha.
#[test]
fn claude_config_and_hook_surfaces_are_never_writable() {
    if bwrap_unavailable("claude_config_and_hook_surfaces_are_never_writable") {
        return;
    }
    let f = claude_fixture();
    let claude = f.spec.home.join(".claude");
    let targets = [
        "settings.json",
        "settings.local.json",
        "daemon.json",
        "daemon/schedule.json",
        "daemon/novo.json",
        "plugins/x/hook.sh",
        "plugins/novo-plugin.json",
        "hooks/pre-commit.sh",
        "mcp.json",
        "agents/reviewer.md",
        "commands/deploy.md",
        "skills/x/SKILL.md",
        "output-styles/terse.md",
        "rules/security.md",
        "workflows/ci.yaml",
        "CLAUDE.md",
        "statusline-command.sh",
        // v0.6.2 (review de segurança r2, BLOCKING): remote-settings.json é
        // MANDATORY de novo -- sombreado read-only sempre, presente ou não
        // no spawn (o caso ausente, pré-criado E sombreado, é
        // `remote_settings_json_absent_is_pre_created_and_shadowed_read_only`).
        // `.config.json` SAIU desta lista de propósito (v0.6.2): é o store
        // de login do Claude, não pode mais ser sombreado nem pré-criado —
        // ver `config_json_absent_from_fixture_is_not_created_by_mounting_the_cage`
        // e `oauth_account_written_to_config_json_persists_to_host`.
        "remote-settings.json",
    ];
    for target in targets {
        let path = claude.join(target);
        let before = std::fs::read(&path).ok();
        let out = run_sh(&f.spec, &format!("echo pwn > {}", path.display()));
        assert_cage_booted(&out);
        assert!(
            !out.status.success(),
            "{target} não pode ser gravável: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            std::fs::read(&path).ok(),
            before,
            "{target} mudou de conteúdo apesar do write ter falhado"
        );
    }
}

/// Contrato de cobertura item 6 / M1: sobre filho sombreado, `rm` e `mv` por
/// cima também falham (EBUSY) — a sombra de arquivo dentro de pai rw não
/// resiste só à escrita in-place, resiste a substituição e a remoção.
#[test]
fn shadowed_children_resist_rm_and_move_over() {
    if bwrap_unavailable("shadowed_children_resist_rm_and_move_over") {
        return;
    }
    let f = claude_fixture();
    let claude = f.spec.home.join(".claude");
    let settings = claude.join("settings.json");
    let before = std::fs::read_to_string(&settings).unwrap();

    let out = run_sh(&f.spec, &format!("rm {}", settings.display()));
    assert_cage_booted(&out);
    assert!(
        !out.status.success(),
        "rm sobre sombra precisa falhar (EBUSY, M1)"
    );
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), before);

    let out = run_sh(
        &f.spec,
        &format!(
            "echo pwn > /tmp/pwn.json && mv /tmp/pwn.json {}",
            settings.display()
        ),
    );
    assert_cage_booted(&out);
    assert!(
        !out.status.success(),
        "mv por cima da sombra precisa falhar (EBUSY, M1)"
    );
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), before);

    let plugins = claude.join("plugins");
    let out = run_sh(&f.spec, &format!("rmdir {}", plugins.display()));
    assert_cage_booted(&out);
    assert!(
        !out.status.success(),
        "rmdir sobre diretório sombreado precisa falhar"
    );
    assert!(plugins.is_dir());
}

/// Contrato de cobertura item 8 — PAR POSITIVO, não opcional: sem ele, "passa"
/// tendo quebrado tudo (V8: estes sete diretórios falhavam EROFS em silêncio
/// sob a allowlist antiga).
#[test]
fn previously_erofs_state_dirs_are_now_writable() {
    if bwrap_unavailable("previously_erofs_state_dirs_are_now_writable") {
        return;
    }
    let f = claude_fixture();
    let claude = f.spec.home.join(".claude");
    for dir in [
        "backups",
        "jobs",
        "cache",
        "paste-cache",
        "sessions",
        "chrome",
        "downloads",
    ] {
        let dir_path = claude.join(dir);
        let target = dir_path.join("novo.txt");
        let out = run_sh(
            &f.spec,
            &format!(
                "mkdir -p {} && echo ok > {}",
                dir_path.display(),
                target.display()
            ),
        );
        assert_cage_booted(&out);
        assert!(
            out.status.success(),
            "{dir} precisa ser gravável (V8 — falhava EROFS em silêncio): {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap().trim(),
            "ok",
            "{dir} não persistiu no host"
        );
    }
}

/// Contrato de cobertura item 7: session-env continua gravável — é onde os
/// hooks SessionStart do próprio Claude Code criam estado; regredir aqui
/// quebra hooks que já funcionavam antes de B.
#[test]
fn session_env_is_still_writable_no_regression() {
    if bwrap_unavailable("session_env_is_still_writable_no_regression") {
        return;
    }
    let f = claude_fixture();
    let claude = f.spec.home.join(".claude");
    let dir = claude.join("session-env/uuid-1");
    let token = dir.join("token");
    let out = run_sh(
        &f.spec,
        &format!(
            "mkdir -p {} && echo tok > {}",
            dir.display(),
            token.display()
        ),
    );
    assert_cage_booted(&out);
    assert!(
        out.status.success(),
        "session-env não pode regredir: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read_to_string(&token).unwrap().trim(), "tok");
}

/// Contrato de cobertura item 9 / furo F4: `projects/<outro>` nem lê nem
/// escreve; `projects/<este>` lê e escreve, via re-grant por cima da sombra.
#[test]
fn other_project_is_invisible_current_project_is_readable_and_writable() {
    if bwrap_unavailable("other_project_is_invisible_current_project_is_readable_and_writable") {
        return;
    }
    let f = claude_fixture();
    let claude = f.spec.home.join(".claude");
    let other = claude.join("projects/-other-repo/secret.txt");
    let out = run_sh(
        &f.spec,
        &format!("test ! -e {} && echo ISOLADO", other.display()),
    );
    assert_cage_booted(&out);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        stdout(&out),
        "ISOLADO",
        "projects/<outro> não pode ser legível nem gravável (F4)"
    );

    let project_name = crate::agent::claude_project_dir_name(&f.spec.writable_root);
    let current = claude.join("projects").join(&project_name);
    let marker = current.join("marca.jsonl");
    let out = run_sh(
        &f.spec,
        &format!(
            "mkdir -p {} && echo x > {} && cat {}",
            current.display(),
            marker.display(),
            marker.display()
        ),
    );
    assert_cage_booted(&out);
    assert!(
        out.status.success(),
        "projects/<este> precisa ser gravável (e legível, via cat): {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout(&out), "x");
}

/// Contrato de cobertura item 10: history.jsonl mascarado mesmo com o pai
/// (`~/.claude`) inteiro virando bind rw — não regride o furo que a leitura já
/// fechava antes de B.
#[test]
fn history_jsonl_is_masked_even_with_writable_parent() {
    if bwrap_unavailable("history_jsonl_is_masked_even_with_writable_parent") {
        return;
    }
    let f = claude_fixture();
    let history = f.spec.home.join(".claude/history.jsonl");
    let before = std::fs::read_to_string(&history).unwrap();
    let out = run_sh(&f.spec, &format!("echo pwn > {}", history.display()));
    assert_cage_booted(&out);
    assert!(
        !out.status.success(),
        "history.jsonl não pode ser gravável mesmo com ~/.claude rw"
    );
    assert_eq!(std::fs::read_to_string(&history).unwrap(), before);
}

// ---------------------------------------------------------------------------
// v0.6.2 — fix da regressão de login: `.config.json` não pode ser sombreado
// nem pré-criado (ver o comentário de `SENSITIVE_CLAUDE_FILES_MANDATORY` em
// agent/mod.rs para a causa raiz completa). Estes quatro testes são o par
// positivo que faltou na entrega B original e deixou o bug passar: sem eles,
// a jaula podia sombrear/pré-criar `.config.json` e nenhum teste acusava.
// ---------------------------------------------------------------------------

/// Item 1 do contrato de cobertura: `.config.json` não pode aparecer como
/// alvo de `--ro-bind`/`--ro-bind-try` no argv REAL da política de produção
/// (`ClaudeCodeRunner::sandbox_access` -> `build_args`) — nem quando já
/// existe no host no momento do spawn. Não precisa do binário `bwrap`: o
/// argv é computado antes de qualquer exec, mas mora aqui porque usa o mesmo
/// `claude_fixture()` das outras provas deste arquivo (a política de
/// produção de verdade, não uma paráfrase escrita à mão).
#[test]
fn config_json_is_never_a_ro_bind_target_in_the_real_argv() {
    let f = claude_fixture();
    let claude = f.spec.home.join(".claude");
    std::fs::write(claude.join(".config.json"), r#"{"oauthAccount":{"x":1}}"#).unwrap();

    let argv = build_args(&f.spec).unwrap();
    let config_json = claude.join(".config.json");
    let mut i = 0;
    while i < argv.len() {
        let op = argv[i].as_os_str();
        if (op == "--ro-bind" || op == "--ro-bind-try") && i + 2 < argv.len() {
            assert_ne!(
                Path::new(&argv[i + 2]),
                config_json,
                ".config.json não pode ser --ro-bind: {argv:?}"
            );
            i += 3;
            continue;
        }
        i += 1;
    }
}

/// Item 2 do contrato de cobertura: fixture SEM `.config.json` -> depois de
/// montar a jaula real (o mesmo `run_sh` que todo o resto deste arquivo usa
/// para provar a política em bwrap de verdade), o host não ganhou um
/// `.config.json`. Antes do fix, `ensure_inert_file` (bwrap.rs) escrevia
/// `{}` ali sempre que ausente -- e esse `{}` passava a vencer o
/// `.claude.json` real do dono via `q()` no binário do Claude, inclusive
/// fora do TYBA.
#[test]
fn config_json_absent_from_fixture_is_not_created_by_mounting_the_cage() {
    if bwrap_unavailable("config_json_absent_from_fixture_is_not_created_by_mounting_the_cage") {
        return;
    }
    let f = claude_fixture();
    let config_json = f.spec.home.join(".claude/.config.json");
    assert!(
        !config_json.exists(),
        "fixture precisa nascer sem .config.json"
    );

    let out = run_sh(&f.spec, "true");
    assert_cage_booted(&out);
    assert!(out.status.success());
    assert!(
        !config_json.exists(),
        ".config.json não pode nascer no host só por a jaula ter subido"
    );
}

/// Item 3 do contrato de cobertura -- o PAR POSITIVO que faltou: dentro da
/// jaula real, escrever `oauthAccount` em `.config.json` (o mesmo padrão
/// tmp+rename que `credential_survives_atomic_rename_and_reaches_host` já
/// prova para `.credentials.json`) precisa persistir no host. Sem este
/// teste, "passar" só provava ausência de erro -- nunca provou que o login
/// realmente sobrevive ao resume.
#[test]
fn oauth_account_written_to_config_json_persists_to_host() {
    if bwrap_unavailable("oauth_account_written_to_config_json_persists_to_host") {
        return;
    }
    let f = claude_fixture();
    let claude = f.spec.home.join(".claude");
    let config_json = claude.join(".config.json");
    assert!(
        !config_json.exists(),
        "fixture precisa nascer sem .config.json"
    );

    let out = run_sh(
        &f.spec,
        &format!(
            "echo '{{\"oauthAccount\":{{\"emailAddress\":\"dono@tyba.dev\"}}}}' > \
             {0}/.config.json.tmp.abc12345 && \
             mv {0}/.config.json.tmp.abc12345 {0}/.config.json && \
             cat {0}/.config.json",
            claude.display()
        ),
    );
    assert_cage_booted(&out);
    assert!(
        out.status.success(),
        "o login precisa sobreviver ao padrão tmp+rename do próprio Claude Code: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout(&out).contains("dono@tyba.dev"));
    let on_host = std::fs::read_to_string(&config_json).unwrap();
    assert!(
        on_host.contains("oauthAccount"),
        "o login gravado dentro da jaula precisa chegar ao host: {on_host}"
    );
}

/// Item 4 do contrato de cobertura, REVERTIDO pelo review de segurança r2
/// (v0.6.2, BLOCKING): `remote-settings.json` ausente no spawn é PRÉ-CRIADO
/// `{}` (M4, `ensure_inert_file`) e sombreado read-only -- o oposto do que
/// esta entrega tinha antes da correção (IF_PRESENT deixava o caso ausente,
/// que é o comum, gravável E invisível pro alarme de deriva -- um agente
/// enjaulado plantando hooks/permissions ali seria exec silencioso na
/// próxima vez que o dono rodasse `claude` fora do TYBA). Ao contrário de
/// `.config.json`, `remote-settings.json` NÃO é o config global (`q()` não
/// olha ele) -- sombrear sempre nunca quebra login.
#[test]
fn remote_settings_json_absent_is_pre_created_and_shadowed_read_only() {
    if bwrap_unavailable("remote_settings_json_absent_is_pre_created_and_shadowed_read_only") {
        return;
    }
    let mut f = fixture();
    let claude = f.spec.home.join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    f.spec.agent = ClaudeCodeRunner.sandbox_access(&f.spec.home, &f.spec.writable_root);
    let remote_settings = claude.join("remote-settings.json");
    assert!(
        !remote_settings.exists(),
        "fixture precisa nascer sem remote-settings.json"
    );

    // Contra-caso pedido pelo review: um agente enjaulado tentando plantar
    // hooks ali dentro precisa falhar -- é exatamente o vetor de exec que
    // promover este arquivo de volta a MANDATORY fecha.
    let out = run_sh(
        &f.spec,
        &format!(
            "echo '{{\"hooks\":{{\"PreToolUse\":[]}}}}' > {}",
            remote_settings.display()
        ),
    );
    assert_cage_booted(&out);
    assert!(
        !out.status.success(),
        "remote-settings.json ausente no spawn precisa vir sombreado (M4, pré-criação) -- um \
         agente não pode plantar hooks nele: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // A pré-criação (M4: --ro-bind de path ausente aborta o bwrap) precisa
    // ter deixado um {} benigno no host -- não o payload de hooks que a
    // jaula recusou gravar.
    assert!(
        remote_settings.exists(),
        "remote-settings.json ausente precisa ser pré-criado no host (M4)"
    );
    assert_eq!(
        std::fs::read_to_string(&remote_settings).unwrap(),
        "{}",
        "a pré-criação é o {{}} inerte de ensure_inert_file, não o payload de hooks recusado"
    );
}
