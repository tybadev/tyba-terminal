use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use super::policy::AgentAccess;
use super::seatbelt::{build_policy, SeatbeltSandbox, SANDBOX_EXEC};
use super::Sandbox;
use super::SandboxSpec;

use crate::agent::{AgentRunner, ClaudeCodeRunner};

fn seatbelt_usable() -> bool {
    Command::new(SANDBOX_EXEC)
        .args(["-p", "(version 1)(allow default)", "--", "/usr/bin/true"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct Fixture {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    worktree: PathBuf,
    neighbor: PathBuf,
    runtime: PathBuf,
    spec: SandboxSpec,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.runtime);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// O agente recebe o PATH do shell de LOGIN (`shell_path::agent_path`), que é onde
/// mora o git de verdade. Forçar `PATH=/usr/bin` no teste obriga o shim do Xcode —
/// um caminho que o produto não usa e que num runner sem Command Line Tools nem
/// existe. O teste passa a espelhar o produto: o dir do git real entra no PATH e
/// nos `exec_path_dirs` (que a política libera pra leitura).
fn real_git_dir() -> Option<PathBuf> {
    let from_login_shell = crate::shell_path::agent_path();
    let from_process = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&from_login_shell)
        .chain(std::env::split_paths(&from_process))
        .find(|dir| {
            // `/usr/bin/git` é o shim do Xcode: sem Command Line Tools ele precisa
            // do `xcodebuild`, que a jaula bloqueia. Queremos o git de verdade.
            dir != Path::new("/usr/bin") && dir.join("git").is_file()
        })
}

fn fixture() -> Fixture {
    let real_home = PathBuf::from(std::env::var("HOME").expect("HOME"));
    let tmp = tempfile::TempDir::new_in(&real_home).unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let home = base.join("home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/id_rsa"), "SECRET-KEY-MATERIAL").unwrap();
    std::fs::write(home.join(".git-credentials"), "https://user:tok@host").unwrap();

    let repo = base.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);

    let worktree = base.join("wt");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "tyba/test",
            worktree.to_str().unwrap(),
        ],
    );
    let neighbor = base.join("wt-neighbor");
    std::fs::create_dir_all(&neighbor).unwrap();
    std::fs::write(neighbor.join("file.txt"), "NEIGHBOR").unwrap();

    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let runtime = PathBuf::from(format!(
        "/private/tmp/tyba-test-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::write(runtime.join("hooks.json"), "{}").unwrap();

    let git_dirs = crate::worktree::resolved_git_dirs(&worktree).unwrap();
    let spec = SandboxSpec {
        writable_root: worktree.clone(),
        readable_root: repo.clone(),
        allow_network: false,
        repo_git_dir: git_dirs.common_dir,
        worktree_git_dir: git_dirs.git_dir,
        runtime_dir: runtime.clone(),
        hook_socket: runtime.join("hook.sock"),
        tyba_exe: PathBuf::from("/usr/bin/true"),
        tyba_data_dir: home.join("Library/Application Support/dev.tyba.app"),
        home: home.clone(),
        tmpdir: std::env::var_os("TMPDIR").map(PathBuf::from),
        exec_path_dirs: real_git_dir().into_iter().collect(),
        agent: AgentAccess::default(),
        read_allow_extra: vec![],
        data_dir_reads: vec![],
    };
    Fixture {
        _tmp: tmp,
        home,
        worktree,
        neighbor,
        runtime,
        spec,
    }
}

fn sandbox_path(spec: &SandboxSpec) -> String {
    let mut dirs: Vec<String> = spec
        .exec_path_dirs
        .iter()
        .map(|d| d.to_string_lossy().into_owned())
        .collect();
    dirs.push("/usr/bin:/bin:/usr/sbin:/sbin".into());
    dirs.join(":")
}

fn run_in_sandbox(spec: &SandboxSpec, cwd: &Path, script: &str) -> Output {
    let mut cmd = Command::new(SANDBOX_EXEC);
    cmd.arg("-p")
        .arg(build_policy(spec))
        .arg("--")
        .arg("/bin/sh")
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", sandbox_path(spec))
        .env("HOME", &spec.home);
    // Mesmo env que o agente recebe: sem DEVELOPER_DIR o shim do git em
    // /usr/bin chama xcodebuild e morre dentro da jaula (numa máquina com Xcode
    // completo). O core resolve isso fora do sandbox — o teste precisa espelhar.
    if let Some(dir) =
        crate::repo_config::agent_env(None, &std::env::vars().collect()).get("DEVELOPER_DIR")
    {
        cmd.env("DEVELOPER_DIR", dir);
    }
    if let Some(tmpdir) = &spec.tmpdir {
        cmd.env("TMPDIR", tmpdir);
    }
    cmd.output().expect("sandbox-exec")
}

fn assert_denied(out: &Output, what: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{what}: devia falhar, passou");
    assert!(
        stderr.contains("Operation not permitted") || stderr.contains("Permission denied"),
        "{what}: stderr inesperado: {stderr}"
    );
}

macro_rules! require_seatbelt {
    () => {
        if !seatbelt_usable() {
            eprintln!("pulando: sandbox-exec indisponível ou sandbox aninhado");
            return;
        }
    };
}

#[test]
fn generated_policy_is_accepted_and_a_process_boots() {
    require_seatbelt!();
    let f = fixture();
    let out = run_in_sandbox(&f.spec, &f.worktree, "/bin/echo boot-ok");
    assert!(
        out.status.success(),
        "policy recusada ou processo não sobe (exit {:?}): {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "boot-ok");
}

#[test]
fn worktree_is_writable_and_secret_home_is_not_readable() {
    require_seatbelt!();
    let f = fixture();
    let ok = run_in_sandbox(
        &f.spec,
        &f.worktree,
        "echo conteudo > out.txt && cat out.txt",
    );
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(f.worktree.join("out.txt"))
            .unwrap()
            .trim(),
        "conteudo"
    );

    let ssh = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("cat {}/.ssh/id_rsa", f.home.display()),
    );
    assert_denied(&ssh, "leitura de ~/.ssh/id_rsa");
    assert!(!String::from_utf8_lossy(&ssh.stdout).contains("SECRET"));
    assert_eq!(
        std::fs::read_to_string(f.home.join(".ssh/id_rsa")).unwrap(),
        "SECRET-KEY-MATERIAL"
    );

    let creds = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("cat {}/.git-credentials", f.home.display()),
    );
    assert_denied(&creds, "leitura de ~/.git-credentials");
}

#[test]
fn git_commit_works_inside_the_sandbox() {
    require_seatbelt!();
    let f = fixture();
    let out = run_in_sandbox(
        &f.spec,
        &f.worktree,
        "echo change > file.txt && git add file.txt && git commit -q -m sandboxed && git log --oneline -1",
    );
    assert!(
        out.status.success(),
        "commit dentro do sandbox: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("sandboxed"));
}

#[test]
fn git_hooks_config_and_pointer_are_not_writable() {
    require_seatbelt!();
    let f = fixture();
    let gitdir = f.spec.worktree_git_dir.display().to_string();
    let repo_git = f.spec.repo_git_dir.display().to_string();

    for (what, script) in [
        (
            "hook no gitdir do worktree",
            format!("mkdir -p {gitdir}/hooks && echo x > {gitdir}/hooks/pre-commit"),
        ),
        (
            "hook no .git do repo",
            format!("mkdir -p {repo_git}/hooks 2>/dev/null; echo x > {repo_git}/hooks/pre-commit"),
        ),
        (
            "config do repo",
            format!("echo '[core]' >> {repo_git}/config"),
        ),
        (
            "config.worktree do gitdir",
            format!("echo x >> {gitdir}/config.worktree"),
        ),
        (
            "exclude executável via info/",
            format!("mkdir -p {repo_git}/info && echo x > {repo_git}/info/exclude"),
        ),
        (
            "ponteiro .git do worktree",
            "echo hijack > .git".to_string(),
        ),
        ("unlink do ponteiro .git", "rm -f .git".to_string()),
    ] {
        let out = run_in_sandbox(&f.spec, &f.worktree, &script);
        assert_denied(&out, what);
    }
    assert!(!f.spec.repo_git_dir.join("hooks/pre-commit").exists());
    let pointer = std::fs::read(f.worktree.join(".git")).unwrap();
    assert!(String::from_utf8_lossy(&pointer).starts_with("gitdir:"));
}

#[test]
fn user_read_allow_can_never_reopen_a_hard_denied_secret() {
    require_seatbelt!();
    let mut f = fixture();
    f.spec.read_allow_extra = vec![f.home.clone()];
    let db_dir = f.home.join("Library/Application Support/dev.tyba.app");
    std::fs::create_dir_all(&db_dir).unwrap();
    std::fs::write(db_dir.join("tyba.db"), "CONSENTS-AND-HISTORY").unwrap();

    std::fs::write(f.home.join("allowed.txt"), "ok").unwrap();
    let allowed = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("cat {}/allowed.txt", f.home.display()),
    );
    assert!(
        allowed.status.success(),
        "o read_allow do usuário precisa continuar funcionando pro que não é segredo: {}",
        String::from_utf8_lossy(&allowed.stderr)
    );

    for (what, path) in [
        ("tyba.db", db_dir.join("tyba.db")),
        ("chave ssh", f.home.join(".ssh/id_rsa")),
        ("git-credentials", f.home.join(".git-credentials")),
    ] {
        let out = run_in_sandbox(&f.spec, &f.worktree, &format!("cat '{}'", path.display()));
        assert_denied(&out, what);
    }
    assert_eq!(
        std::fs::read_to_string(db_dir.join("tyba.db")).unwrap(),
        "CONSENTS-AND-HISTORY"
    );
}

#[test]
fn tyba_binary_and_user_config_are_not_writable() {
    require_seatbelt!();
    let mut f = fixture();
    let exe = f.home.join("tyba-fake-exe");
    std::fs::write(&exe, "#!/bin/sh\nexit 0\n").unwrap();
    f.spec.tyba_exe = exe.clone();
    std::fs::create_dir_all(f.home.join(".tyba")).unwrap();
    let config = f.home.join(".tyba/config.toml");
    std::fs::write(&config, "[sandbox]\n").unwrap();

    for (what, path) in [
        ("binário do Tyba", exe.clone()),
        ("config do usuário", config.clone()),
    ] {
        let out = run_in_sandbox(
            &f.spec,
            &f.worktree,
            &format!("echo pwned > {}", path.display()),
        );
        assert_denied(&out, what);
    }
    assert_eq!(std::fs::read_to_string(&config).unwrap(), "[sandbox]\n");
    assert!(std::fs::read_to_string(&exe).unwrap().starts_with("#!"));
}

#[test]
fn neighbor_worktree_is_invisible() {
    require_seatbelt!();
    let f = fixture();
    let neighbor = f.neighbor.display().to_string();
    let read = run_in_sandbox(&f.spec, &f.worktree, &format!("cat {neighbor}/file.txt"));
    assert_denied(&read, "leitura do worktree vizinho");
    let write = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("echo planted > {neighbor}/file.txt"),
    );
    assert_denied(&write, "escrita no worktree vizinho");
    assert_eq!(
        std::fs::read_to_string(f.neighbor.join("file.txt")).unwrap(),
        "NEIGHBOR"
    );
}

#[test]
fn own_runtime_dir_readable_but_sibling_session_dirs_are_not() {
    require_seatbelt!();
    let f = fixture();
    let ok = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("cat {}/hooks.json", f.runtime.display()),
    );
    assert!(ok.status.success());

    let sibling = PathBuf::from(format!(
        "/private/tmp/tyba-sibling-{}",
        f.runtime.file_name().unwrap().to_string_lossy()
    ));
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("secret"), "OTHER-SESSION").unwrap();
    let read = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("cat {}/secret", sibling.display()),
    );
    let _ = std::fs::remove_dir_all(&sibling);
    assert_denied(&read, "runtime dir de outra sessão");
}

#[test]
fn sibling_runtime_dir_in_real_tmpdir_is_not_writable() {
    require_seatbelt!();
    let Some(tmpdir) = std::env::var_os("TMPDIR").map(PathBuf::from) else {
        eprintln!("pulando: sem TMPDIR");
        return;
    };
    let mut f = fixture();
    let seq = f
        .runtime
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let sibling = tmpdir.join(format!("tyba-sib-{seq}"));
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("hooks.json"), "{\"real\":true}").unwrap();
    f.spec.tmpdir = Some(tmpdir);

    let write = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("echo pwned > {}/hooks.json", sibling.display()),
    );
    let read = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("cat {}/hooks.json", sibling.display()),
    );
    let intact = std::fs::read_to_string(sibling.join("hooks.json")).unwrap();
    let _ = std::fs::remove_dir_all(&sibling);
    assert_denied(&write, "escrita no runtime dir vizinho no TMPDIR real");
    assert_denied(&read, "leitura do runtime dir vizinho no TMPDIR real");
    assert_eq!(
        intact, "{\"real\":true}",
        "um agente reescreveu o hooks.json de outra sessão e derrubaria o gate dela"
    );
}

#[test]
fn runtime_dir_is_not_writable_by_the_agent() {
    require_seatbelt!();
    let f = fixture();
    let out = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("echo pwned > {}/hooks.json", f.runtime.display()),
    );
    assert_denied(&out, "escrita no runtime dir");
    assert_eq!(
        std::fs::read_to_string(f.runtime.join("hooks.json")).unwrap(),
        "{}"
    );
}

#[test]
fn hook_socket_is_connectable_from_inside() {
    require_seatbelt!();
    let f = fixture();
    let sock_path = f.spec.hook_socket.clone();
    let listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    let server = std::thread::spawn(move || {
        if let Ok((mut conn, _)) = listener.accept() {
            let mut buf = [0u8; 16];
            let _ = conn.read(&mut buf);
        }
    });
    let out = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("printf ping | nc -w 2 -U {}", sock_path.display()),
    );
    server.join().unwrap();
    assert!(
        out.status.success(),
        "conexão no socket de hook: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn agent_is_interactive_in_a_real_pty() {
    require_seatbelt!();
    use portable_pty::{CommandBuilder, PtySize};

    let f = fixture();
    let mut cmd = CommandBuilder::new("/bin/sh");
    cmd.arg("-c");
    cmd.arg("test -t 0 && test -t 1 && stty size >/dev/null && echo TTY-OK || echo TTY-BROKEN");
    cmd.cwd(&f.worktree);
    cmd.env_clear();
    cmd.env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
    cmd.env("HOME", &f.home);
    let cmd = SeatbeltSandbox.wrap(cmd, &f.spec).unwrap();

    let pair = portable_pty::native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    drop(pair.master);

    let mut out = String::new();
    let _ = reader.read_to_string(&mut out);
    let _ = child.wait();

    assert!(
        out.contains("TTY-OK"),
        "o PTY nasce fora do sandbox: sem file-ioctl em /dev/ttys*, o isatty() falha e a TUI do \
         agente quebra (shell não-interativo, sem cores). Saída: {out:?}"
    );
}

#[test]
fn sysctl_for_node_runtime_works() {
    require_seatbelt!();
    let f = fixture();
    let out = run_in_sandbox(
        &f.spec,
        &f.worktree,
        "/usr/sbin/sysctl -n machdep.cpu.brand_string hw.model",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.stdout.is_empty());
}

#[test]
fn claude_agent_dirs_follow_the_granular_rules() {
    require_seatbelt!();
    let mut f = fixture();
    let claude = f.home.join(".claude");
    std::fs::create_dir_all(claude.join("projects/-other-repo")).unwrap();
    std::fs::write(
        claude.join("projects/-other-repo/t.jsonl"),
        "OTHER-TRANSCRIPT",
    )
    .unwrap();
    std::fs::write(claude.join("settings.json"), "{}").unwrap();
    f.spec.agent = ClaudeCodeRunner.sandbox_access(&f.home, &f.worktree);

    let project = claude
        .join("projects")
        .join(crate::agent::claude_project_dir_name(&f.worktree));
    let ok = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!(
            "mkdir -p {p} && echo t > {p}/x.jsonl && cat {s}",
            p = project.display(),
            s = claude.join("settings.json").display()
        ),
    );
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );

    let other = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("cat {}/projects/-other-repo/t.jsonl", claude.display()),
    );
    assert_denied(&other, "transcript de outro projeto");

    let settings = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("echo pwned > {}/settings.json", claude.display()),
    );
    assert_denied(&settings, "escrita em settings.json");
    assert_eq!(
        std::fs::read_to_string(claude.join("settings.json")).unwrap(),
        "{}"
    );

    let session_env = claude.join("session-env/00000000-fake-uuid");
    let hook_env = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!(
            "mkdir -p {d} && echo 'export X=1' > {d}/env.sh",
            d = session_env.display()
        ),
    );
    assert!(
        hook_env.status.success(),
        "hook SessionStart precisa criar seu session-env: {}",
        String::from_utf8_lossy(&hook_env.stderr)
    );
    assert!(session_env.join("env.sh").exists());
}

// ---------------------------------------------------------------------------
// Entrega B — a credencial do agente sobrevive à jaula no Linux (§12, item 19:
// mesmo par positivo/negativo dos itens 3-9 no Seatbelt). `seatbelt.rs` não
// muda de código nenhum para B (§8) — a política é a mesma AgentAccess do
// Linux, só renderizada em SBPL; estes testes provam que a tradução para
// sandbox-exec produz o mesmo resultado observável.
// ---------------------------------------------------------------------------

/// Contrato de cobertura itens 3 e 4 (par macOS): settings.json,
/// settings.local.json, daemon.json, CLAUDE.md (arquivos sombreados
/// obrigatórios) e um representante de cada diretório somente-leitura
/// (plugins/, hooks/, agents/) nunca são graváveis — nem um arquivo que já
/// existia, nem um novo dentro de um diretório já sombreado.
#[test]
fn claude_config_and_hook_surfaces_are_never_writable_on_macos() {
    require_seatbelt!();
    let mut f = fixture();
    let claude = f.home.join(".claude");
    std::fs::create_dir_all(claude.join("plugins/x")).unwrap();
    std::fs::write(claude.join("plugins/x/hook.sh"), "#!/bin/sh\necho pwn\n").unwrap();
    std::fs::create_dir_all(claude.join("hooks")).unwrap();
    std::fs::write(claude.join("settings.json"), "{}").unwrap();
    std::fs::write(claude.join("settings.local.json"), "{}").unwrap();
    std::fs::write(claude.join("daemon.json"), "{}").unwrap();
    std::fs::write(claude.join("CLAUDE.md"), "memória do dono").unwrap();
    f.spec.agent = ClaudeCodeRunner.sandbox_access(&f.home, &f.worktree);

    for target in [
        "settings.json",
        "settings.local.json",
        "daemon.json",
        "CLAUDE.md",
        "plugins/x/hook.sh",
        "plugins/novo-plugin.json",
        "hooks/pre-commit.sh",
    ] {
        let path = claude.join(target);
        let before = std::fs::read(&path).ok();
        let out = run_in_sandbox(
            &f.spec,
            &f.worktree,
            &format!("echo pwn > {}", path.display()),
        );
        assert_denied(&out, target);
        assert_eq!(
            std::fs::read(&path).ok(),
            before,
            "{target} mudou de conteúdo apesar do write ter falhado"
        );
    }
}

/// Contrato de cobertura item 6 (par macOS): `file-write*` no Seatbelt cobre a
/// família inteira (create/unlink/rename/data), então `rm` e `mv` por cima de
/// um filho sombreado também precisam falhar — não só a escrita in-place.
#[test]
fn shadowed_children_resist_rm_and_move_over_on_macos() {
    require_seatbelt!();
    let mut f = fixture();
    let claude = f.home.join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(claude.join("settings.json"), "{}").unwrap();
    f.spec.agent = ClaudeCodeRunner.sandbox_access(&f.home, &f.worktree);
    let settings = claude.join("settings.json");

    let rm = run_in_sandbox(&f.spec, &f.worktree, &format!("rm {}", settings.display()));
    assert_denied(&rm, "rm sobre settings.json sombreado");
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{}");

    let mv = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!(
            "echo pwn > /tmp/pwn-{}.json && mv /tmp/pwn-{}.json {}",
            std::process::id(),
            std::process::id(),
            settings.display()
        ),
    );
    assert_denied(&mv, "mv por cima de settings.json sombreado");
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{}");
}

/// Contrato de cobertura item 8 (par macOS) — PAR POSITIVO, não opcional: sem
/// ele, "passa" tendo quebrado tudo (o mesmo V8 que motivou a inversão no
/// Linux vale para a política compartilhada).
#[test]
fn previously_denied_state_dirs_are_writable_on_macos() {
    require_seatbelt!();
    let mut f = fixture();
    let claude = f.home.join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    f.spec.agent = ClaudeCodeRunner.sandbox_access(&f.home, &f.worktree);

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
        let out = run_in_sandbox(
            &f.spec,
            &f.worktree,
            &format!(
                "mkdir -p {} && echo ok > {}",
                dir_path.display(),
                target.display()
            ),
        );
        assert!(
            out.status.success(),
            "{dir} precisa ser gravável: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap().trim(), "ok");
    }
}

/// Contrato de cobertura item 9 (par macOS): completa
/// `claude_agent_dirs_follow_the_granular_rules` (que já prova leitura negada)
/// com a metade que faltava — escrita em `projects/<outro>` também precisa
/// falhar.
#[test]
fn other_project_is_not_writable_on_macos() {
    require_seatbelt!();
    let mut f = fixture();
    let claude = f.home.join(".claude");
    std::fs::create_dir_all(claude.join("projects/-other-repo")).unwrap();
    f.spec.agent = ClaudeCodeRunner.sandbox_access(&f.home, &f.worktree);

    let target = claude.join("projects/-other-repo/pwn.txt");
    let out = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("echo pwn > {}", target.display()),
    );
    assert_denied(&out, "escrita em projects/<outro>");
    assert!(!target.exists());
}

#[test]
#[ignore = "exige ./target/debug/tyba compilado (cargo build antes)"]
fn tyba_hook_crosses_the_sandbox_and_gets_a_real_decision() {
    require_seatbelt!();
    let mut f = fixture();
    let tyba = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tyba");
    assert!(
        tyba.exists(),
        "compile o binário primeiro: cargo build (esperado em {})",
        tyba.display()
    );
    f.spec.tyba_exe = tyba.canonicalize().unwrap();

    let server = crate::hook_ipc::HookServer::bind(
        &f.spec.hook_socket,
        std::sync::Arc::new(
            |_e: crate::hook_ipc::HookEvent| crate::hook_ipc::HookAction::Deny {
                reason: "decisao-atravessou-o-sandbox".into(),
            },
        ),
    )
    .unwrap();

    let payload = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git push"},"cwd":"/wt"}"#;
    let out = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!(
            "printf '{payload}' | TYBA_HOOK_SOCKET={sock} {tyba} _hook",
            sock = f.spec.hook_socket.display(),
            tyba = f.spec.tyba_exe.display()
        ),
    );
    server.shutdown();

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("decisao-atravessou-o-sandbox"),
        "hook não recebeu a decisão do core: {stdout}"
    );
    assert!(
        !stdout.contains("transporte de hook indisponível"),
        "hook caiu em fail-closed — socket inalcançável dentro do sandbox: {stdout}"
    );
}

#[test]
fn own_keychain_item_is_readable_but_writes_are_denied() {
    require_seatbelt!();
    let svc = format!("tyba-sandbox-selftest-{}", std::process::id());
    let added = format!("tyba-sandbox-added-{}", std::process::id());
    let created = Command::new("/usr/bin/security")
        .args([
            "add-generic-password",
            "-a",
            "tyba",
            "-s",
            &svc,
            "-w",
            "disposable",
            "-U",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !created {
        eprintln!("pulando: keychain de login indisponível/bloqueado pra criar item disposable");
        return;
    }

    let mut f = fixture();
    f.spec.home = PathBuf::from(std::env::var("HOME").unwrap());
    f.spec.allow_network = true;

    let read = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("security find-generic-password -s '{svc}' -w > /dev/null"),
    );
    let add = run_in_sandbox(
        &f.spec,
        &f.worktree,
        &format!("security add-generic-password -a x -s '{added}' -w dummy > /dev/null 2>&1"),
    );

    let _ = Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-s", &svc])
        .output();
    let _ = Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-s", &added])
        .output();

    assert!(
        read.status.success(),
        "o agente precisa ler a própria credencial do keychain: {}",
        String::from_utf8_lossy(&read.stderr)
    );
    assert!(
        !add.status.success(),
        "o agente não pode gravar/injetar itens no keychain"
    );
}

fn real_agent_boots_under_production_policy(runner: &dyn AgentRunner, binary: &str) {
    if crate::agent::resolved_binary(&runner.kind()).is_none() {
        eprintln!("pulando: binário `{binary}` não está no PATH do shell");
        return;
    }
    let f = fixture();
    let real_home = PathBuf::from(std::env::var("HOME").unwrap());
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert("HOME".into(), real_home.display().to_string());
    env.insert("PATH".into(), crate::shell_path::agent_path());
    if let Ok(tmpdir) = std::env::var("TMPDIR") {
        env.insert("TMPDIR".into(), tmpdir);
    }
    let exe = std::env::current_exe().unwrap();
    let spec = crate::agent::session::sandbox_spec(
        runner,
        &env,
        &f.spec.readable_root,
        &f.worktree,
        &f.runtime,
        &f.spec.hook_socket,
        &exe,
    )
    .expect("spec de produção");

    let mut cmd = Command::new(SANDBOX_EXEC);
    cmd.arg("-p").arg(build_policy(&spec)).arg("--");
    cmd.arg(crate::agent::resolved_binary(&runner.kind()).unwrap());
    cmd.arg("--version");
    cmd.current_dir(&f.worktree).env_clear();
    for (k, v) in &env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn do agente sandboxado");
    assert!(
        out.status.success(),
        "`{binary} --version` não roda dentro do sandbox de produção — a sessão real morreria no spawn.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!String::from_utf8_lossy(&out.stdout).trim().is_empty());
}

#[test]
#[ignore = "exige o binário `claude` no PATH do shell de login"]
fn claude_binary_boots_under_the_production_policy() {
    require_seatbelt!();
    real_agent_boots_under_production_policy(&ClaudeCodeRunner, "claude");
}

#[test]
#[ignore = "exige o binário `codex` no PATH do shell de login"]
fn codex_binary_boots_under_the_production_policy() {
    require_seatbelt!();
    real_agent_boots_under_production_policy(&crate::agent::CodexRunner, "codex");
}

#[test]
#[ignore = "precisa de rede real"]
fn https_works_with_network_enabled() {
    require_seatbelt!();
    let mut f = fixture();
    f.spec.allow_network = true;
    let out = run_in_sandbox(
        &f.spec,
        &f.worktree,
        "curl -sS --max-time 20 -o /dev/null -w '%{http_code}' https://api.anthropic.com",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let code = String::from_utf8_lossy(&out.stdout);
    assert!(
        !code.trim().is_empty() && code.trim() != "000",
        "http_code: {code}"
    );
}

#[test]
#[ignore = "precisa de rede real"]
fn docker_socket_is_unreachable_even_with_network() {
    require_seatbelt!();
    let mut f = fixture();
    f.spec.allow_network = true;
    let out = run_in_sandbox(
        &f.spec,
        &f.worktree,
        "curl -sS --max-time 5 --unix-socket /var/run/docker.sock http://localhost/version",
    );
    assert!(!out.status.success());
}

/// Sessão anexada ao checkout principal (review/conflitos num repo comum): ali o
/// gitdir do worktree É o `.git` do repo, então a área gravável do gitdir
/// alcançaria `refs/heads/main`. Push já é recusado pelo core; mover a main
/// LOCAL é a mesma regra vista de outro ângulo.
#[test]
fn agent_attached_to_the_main_checkout_cannot_move_main() {
    require_seatbelt!();
    let mut f = fixture();
    f.spec.writable_root = f.spec.readable_root.clone();
    f.spec.worktree_git_dir = f.spec.repo_git_dir.clone();
    let spec = &f.spec;
    let repo = spec.writable_root.clone();
    let main_ref = spec.repo_git_dir.join("refs/heads/main");
    let before = std::fs::read_to_string(&main_ref).unwrap();

    for (what, script) in [
        ("update-ref", "git update-ref refs/heads/main HEAD~0 2>&1"),
        (
            "escrita crua na ref",
            "echo 0000000000000000000000000000000000000000 > .git/refs/heads/main",
        ),
        ("pack-refs", "git pack-refs --all"),
    ] {
        let out = run_in_sandbox(spec, &repo, script);
        assert!(
            !out.status.success() || std::fs::read_to_string(&main_ref).unwrap() == before,
            "{what} moveu a main de dentro do sandbox"
        );
        assert_eq!(
            std::fs::read_to_string(&main_ref).unwrap(),
            before,
            "a ref da main mudou via {what}"
        );
    }

    // Par positivo: proteger a main não pode custar o trabalho normal do agente.
    let out = run_in_sandbox(
        spec,
        &repo,
        "git checkout -q -b feature-x && echo x > novo.txt && git add novo.txt \
         && git commit -q -m dentro && git rev-parse --abbrev-ref HEAD",
    );
    assert!(
        out.status.success(),
        "commit numa branch comum precisa funcionar: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("feature-x"));
}
