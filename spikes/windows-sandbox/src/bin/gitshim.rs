#![cfg(windows)]

use std::io::{Read, Write};
use std::os::windows::io::{FromRawHandle, RawHandle};

use probe::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Pipes::*;
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--child" {
        let handle_value: usize = args[2].parse().unwrap_or(0);
        std::process::exit(child(handle_value as Handle));
    }
    parent();
}

fn parent() {
    banner("SONDA gitshim — git roteado pelo core: o agente delega git ao pai não-enjaulado");
    println!("Duas perguntas. (1) Controle: com a receita real da jaula, o git.exe já INICIA");
    println!("(mudou o motivo do shim: de 'git não roda' para 'roteamento por controle')?");
    println!("(2) Relay: um processo enjaulado manda args de git pelo pipe herdado, o core roda");
    println!("o git REAL no worktree e devolve a saída — o PreToolUse continua vendo tudo.\n");

    let git = match git_exe() {
        Some(g) => g,
        None => {
            println!("[ERRO] git.exe não encontrado");
            std::process::exit(2);
        }
    };

    control_git_boots(&git);
    relay(&git);
}

fn control_git_boots(git: &str) {
    let token = match build_restricted(&jail_spec(true)) {
        Ok(r) => r.token,
        Err(e) => {
            println!("[ERRO] token da jaula: {e}");
            return;
        }
    };
    let cmd = format!("\"{git}\" --version");
    let (code, out) = match capture(token, &cmd, Some("Winsta0\\Default"), None, 0, None) {
        Ok(v) => v,
        Err(e) => {
            println!("[ERRO] spawn do git enjaulado: {e}");
            return;
        }
    };
    let booted = code == 0 && out.contains("git version");
    println!();
    verdict(
        "controle: git.exe INICIA sob a receita real da jaula (logon+Everyone)",
        booted,
        "com o restricting set certo, git não dá mais 0xc0000142 — o shim passa a ser sobre CONTROLE, não capacidade",
    );
    println!("       git enjaulado -> exit 0x{code:08X} | {:?}", out.trim());
}

fn relay(git: &str) {
    let repo = std::env::temp_dir().join(format!("tyba-spike-gitshim-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(git)
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
    };
    let _ = run(&["init", "-q"]);
    std::fs::write(repo.join("marker.txt"), b"agente tocou aqui").unwrap();

    let pid = std::process::id();
    let pipe_name = format!(r"\\.\pipe\tyba-gitshim-{pid}");
    let wname = wide(&pipe_name);

    unsafe {
        let server = CreateNamedPipeW(
            wname.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_WAIT,
            1,
            4096,
            4096,
            0,
            std::ptr::null(),
        );
        if server == INVALID_HANDLE_VALUE {
            println!("[ERRO] CreateNamedPipeW: {}", last_error());
            return;
        }
        let client = CreateFileW(
            wname.as_ptr(),
            (GENERIC_READ | GENERIC_WRITE) as u32,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if client == INVALID_HANDLE_VALUE {
            println!("[ERRO] CreateFileW(client): {}", last_error());
            return;
        }
        SetHandleInformation(client, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);

        let token = match build_restricted(&jail_spec(true)) {
            Ok(r) => r.token,
            Err(e) => {
                println!("[ERRO] token da jaula: {e}");
                return;
            }
        };

        let exe = std::env::current_exe().unwrap();
        let cmd = format!("\"{}\" --child {}", exe.display(), client as usize);
        let (proc, thread) = match spawn_inherit_async(token, &cmd, &[client]) {
            Ok(v) => v,
            Err(e) => {
                println!("[ERRO] spawn do shim enjaulado: {e}");
                return;
            }
        };

        let mut server_file = std::fs::File::from_raw_handle(server as RawHandle);
        let mut buf = [0u8; 512];
        let n = server_file.read(&mut buf).unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]).trim().to_string();

        let git_args: Vec<&str> = request.split_whitespace().collect();
        let output = std::process::Command::new(git)
            .arg("-C")
            .arg(&repo)
            .args(&git_args)
            .output();
        let response = match output {
            Ok(o) => format!(
                "{}\n{}",
                o.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&o.stdout)
            ),
            Err(e) => format!("-1\nerro ao rodar git: {e}"),
        };
        let _ = server_file.write_all(response.as_bytes());
        let _ = server_file.flush();

        WaitForSingleObject(proc, 5000);
        let mut child_code: u32 = 0;
        GetExitCodeProcess(proc, &mut child_code);
        CloseHandle(proc);
        CloseHandle(thread);

        println!();
        verdict(
            "o core recebeu o pedido de git do processo enjaulado pelo pipe herdado",
            !request.is_empty(),
            "o shim (enjaulado) encaminhou args de git ao core sem tocar git.exe — request via handle herdado",
        );
        verdict(
            "o core rodou o git REAL no worktree e o shim recebeu a resposta certa",
            child_code == 0,
            "round-trip completo: git rodou FORA da jaula no repo do core, saída voltou ao agente. É o roteamento do tech-spec",
        );
        println!("\n  pedido do shim: {request:?}");
        println!("  (o core rodou 'git -C <repo> {request}' não-enjaulado; child exit {child_code})");

        let _ = std::fs::remove_dir_all(&repo);
    }
}

fn child(pipe: Handle) -> i32 {
    let mut file = unsafe { std::fs::File::from_raw_handle(pipe as RawHandle) };
    if file
        .write_all(b"status --porcelain")
        .and_then(|_| file.flush())
        .is_err()
    {
        return 11;
    }
    let mut buf = [0u8; 1024];
    let n = file.read(&mut buf).unwrap_or(0);
    let response = String::from_utf8_lossy(&buf[..n]);
    std::mem::forget(file);
    if response.contains("marker.txt") {
        0
    } else {
        12
    }
}
