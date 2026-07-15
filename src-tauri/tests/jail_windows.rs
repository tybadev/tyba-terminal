//! Verificação ponta a ponta da jaula do Windows (Camada A) pela API pública.
//!
//! Roda como teste de INTEGRAÇÃO (não `--lib`) de propósito: só o binário de teste
//! de integração recebe o manifesto comctl6 (via `rustc-link-arg-tests` no
//! `build.rs`) que o `rfd`/`muda` exigem para carregar — o binário de teste de
//! unidade falha ao iniciar sem ele. E exercita a jaula pela superfície pública
//! (`Sandbox::jailed_spawner`), o mesmo caminho que a camada de sessão usa.
#![cfg(windows)]

use std::path::PathBuf;

use portable_pty::{CommandBuilder, PtySize};
use tyba_lib::pty::conpty_jailed;
use tyba_lib::sandbox::policy::AgentAccess;
use tyba_lib::sandbox::windows::WindowsSandbox;
use tyba_lib::sandbox::{Sandbox, SandboxSpec};

/// Via de PRODUÇÃO: `conpty_jailed::spawn` com token nulo (shell, sem jaula) sobe
/// um `cmd` e entrega o output do filho pelo `MasterPty` real — o mesmo caminho que
/// o `PtyPool` usa agora no Windows. Prova que a troca do portable-pty resolve.
#[test]
fn spawn_shell_entrega_output_pela_via_de_producao() {
    use std::io::Read;
    use std::time::Duration;

    let mut cmd = CommandBuilder::new("cmd.exe");
    cmd.arg("/k");
    cmd.arg("echo");
    cmd.arg("PROD_MARKER_XYZ");
    for k in ["SystemRoot", "PATH", "TEMP", "TMP", "ComSpec"] {
        if let Ok(v) = std::env::var(k) {
            cmd.env(k, v);
        }
    }
    let params = conpty_jailed::JailSpawnParams {
        token: std::ptr::null_mut(),
        command_line: conpty_jailed::encode_command_line(&cmd).expect("cmdline"),
        env_block: conpty_jailed::encode_env_block(&cmd, &[]),
        cwd: None,
        size: PtySize {
            rows: 30,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        },
        mitigation: None,
    };
    let (master, mut child) = conpty_jailed::spawn(params).expect("spawn de produção");
    let mut reader = master.try_clone_reader().expect("reader");
    let handle = std::thread::spawn(move || {
        let mut buf = vec![0u8; 8192];
        let mut acc = String::new();
        for _ in 0..20 {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => acc.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(_) => break,
            }
            if acc.contains("PROD_MARKER_XYZ") {
                break;
            }
        }
        acc
    });
    std::thread::sleep(Duration::from_millis(1500));
    let _ = child.kill();
    drop(master);
    let out = handle.join().unwrap_or_default();
    assert!(
        out.contains("PROD_MARKER_XYZ"),
        "a via de produção tem que entregar o output do shell: {out:?}"
    );
}

/// Quoting da cmdline: argumentos com espaço/aspas sobrevivem ao re-encode manual
/// (os getters `cmdline` do portable-pty são `pub(crate)`, então reconstruímos).
#[test]
fn encode_command_line_quota_espaco_e_aspas() {
    let mut cmd = CommandBuilder::new("prog.exe");
    cmd.arg("um dois");
    cmd.arg("simples");
    cmd.arg("com\"aspas");
    let line = conpty_jailed::encode_command_line(&cmd).expect("encode");
    assert_eq!(line.last(), Some(&0), "cmdline termina em NUL");
    let end = line.iter().position(|&c| c == 0).unwrap_or(line.len());
    let s = String::from_utf16_lossy(&line[..end]);
    assert!(s.contains("\"um dois\""), "espaço vira aspas: {s}");
    assert!(s.contains(" simples"), "arg simples sem aspas: {s}");
    assert!(s.contains("\\\""), "aspas internas escapadas: {s}");
}

fn spec_at(base: &std::path::Path, worktree: &std::path::Path) -> SandboxSpec {
    SandboxSpec {
        writable_root: worktree.to_path_buf(),
        readable_root: base.to_path_buf(),
        allow_network: false,
        repo_git_dir: base.join(".git"),
        worktree_git_dir: base.join(".git"),
        runtime_dir: base.join("runtime"),
        hook_socket: base.join("runtime/hook.sock"),
        tyba_exe: base.join("tyba.exe"),
        tyba_data_dir: base.join("data"),
        home: base.to_path_buf(),
        tmpdir: Some(base.to_path_buf()),
        exec_path_dirs: vec![],
        agent: AgentAccess::default(),
        read_allow_extra: vec![],
    }
}

/// Par positivo da jaula: sobe um `cmd.exe` real sob o token restrito num ConPTY,
/// pela API pública, e mede pelo efeito no filesystem que ele (a) escreve DENTRO
/// do worktree, (b) é NEGADO fora, (c) NÃO lê um segredo rotulado NO_READ_UP mesmo
/// por caminho direto, e (d) LÊ um arquivo aberto (o positivo, que prova que a
/// jaula não nega por acidente).
#[test]
fn jaula_confina_escrita_e_nega_leitura_de_segredo() {
    let base: PathBuf = std::env::temp_dir().join(format!("tyba-jail-e2e-{}", std::process::id()));
    let worktree = base.join("worktree");
    let outside = base.join("outside");
    let ssh = base.join(".ssh"); // vira segredo via `secret_paths(spec)` (home/.ssh)
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(&ssh).unwrap();
    let secret = ssh.join("id_rsa");
    let open = base.join("open.txt");
    std::fs::write(&secret, b"SEGREDO-NAO-PODE-VAZAR").unwrap();
    std::fs::write(&open, b"conteudo-publico").unwrap();

    // Monta a jaula pela API pública: aplica rótulo Low + ACE no worktree e deny de
    // leitura nos segredos derivados de `spec.home` (aqui, `base/.ssh`).
    let spawner = WindowsSandbox
        .jailed_spawner(&spec_at(&base, &worktree))
        .expect("jailed_spawner")
        .expect("Windows devolve Some");

    let script = format!(
        "@echo off\r\n\
         echo dentro> \"{wt}\\in.txt\"\r\n\
         echo fora> \"{out}\\out.txt\"\r\n\
         type \"{sec}\" > \"{wt}\\read_secret.txt\" 2>&1\r\n\
         type \"{opn}\" > \"{wt}\\read_open.txt\" 2>&1\r\n",
        wt = worktree.display(),
        out = outside.display(),
        sec = secret.display(),
        opn = open.display(),
    );
    let bat = base.join("run.bat");
    std::fs::write(&bat, script).unwrap();

    let comspec =
        std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_string());
    let mut cmd = CommandBuilder::new(&comspec);
    cmd.arg("/c");
    cmd.arg(bat.to_string_lossy().to_string());
    cmd.cwd(worktree.to_string_lossy().to_string());
    for k in ["SystemRoot", "PATH", "TEMP", "TMP", "ComSpec"] {
        if let Ok(v) = std::env::var(k) {
            cmd.env(k, v);
        }
    }

    let size = PtySize {
        rows: 30,
        cols: 120,
        pixel_width: 0,
        pixel_height: 0,
    };
    let (master, mut child) = match spawner.spawn_jailed(&cmd, size) {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&base);
            panic!("spawn enjaulado falhou: {e}");
        }
    };

    let mut reader = master.try_clone_reader().expect("reader do ConPTY");
    let drain = std::thread::spawn(move || {
        use std::io::Read;
        let mut sink = Vec::new();
        let _ = reader.read_to_end(&mut sink);
    });
    let _ = child.wait();
    drop(master); // fecha o ConPTY → EOF → a thread de drain termina
    let _ = drain.join();

    let wrote_in = worktree.join("in.txt").exists();
    let wrote_out = outside.join("out.txt").exists();
    let read_secret = std::fs::read_to_string(worktree.join("read_secret.txt")).unwrap_or_default();
    let read_open = std::fs::read_to_string(worktree.join("read_open.txt")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&base);

    assert!(wrote_in, "o agente deve escrever DENTRO do worktree");
    assert!(!wrote_out, "o agente NÃO pode escrever FORA do worktree");
    assert!(
        !read_secret.contains("SEGREDO"),
        "segredo rotulado NO_READ_UP não pode vazar (nem por caminho direto): {read_secret:?}"
    );
    assert!(
        read_open.contains("conteudo-publico"),
        "arquivo aberto deve ser legível — par positivo: {read_open:?}"
    );
}
