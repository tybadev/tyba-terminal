#![cfg(windows)]

use std::io::{Read, Write};
use std::os::windows::io::{FromRawHandle, RawHandle};

use probe::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Pipes::*;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "--child" {
        let handle_value: usize = args[2].parse().unwrap_or(0);
        let pipe_name = args[3].clone();
        std::process::exit(child(handle_value as Handle, &pipe_name));
    }
    parent();
}

fn parent() {
    banner("SONDA A — gate por named pipe herdado (a peça sem a qual não há produto)");
    println!("Pergunta: um agente enjaulado (WRITE_RESTRICTED + IL Low) consegue FALAR");
    println!("com o inbox por um handle de pipe HERDADO — e é BLOQUEADO de abrir o pipe");
    println!("pelo NOME? Se sim, o gate atravessa a jaula como no Linux/macOS.\n");

    let pid = std::process::id();
    let pipe_name = format!(r"\\.\pipe\tyba-spike-{pid}");
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
            println!("[ERRO] CreateNamedPipeW falhou: {}", last_error());
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
            println!("[ERRO] CreateFileW (client) falhou: {}", last_error());
            return;
        }
        if SetHandleInformation(client, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) == 0 {
            println!("[ERRO] SetHandleInformation falhou: {}", last_error());
            return;
        }

        let restricted = match build_restricted_low_il() {
            Ok(r) => r,
            Err(e) => {
                println!("[ERRO] montar token restrito: {e}");
                return;
            }
        };

        let exe = std::env::current_exe().unwrap();
        let cmd = format!(
            "\"{}\" --child {} {}",
            exe.display(),
            client as usize,
            pipe_name
        );

        let code = match spawn_with_token(restricted.token, &cmd, &[client]) {
            Ok(c) => c,
            Err(e) => {
                println!("[ERRO] spawn enjaulado: {e}");
                return;
            }
        };

        let mut server_file = std::fs::File::from_raw_handle(server as RawHandle);
        let mut buf = [0u8; 16];
        let n = server_file.read(&mut buf).unwrap_or(0);
        let payload = String::from_utf8_lossy(&buf[..n]).to_string();

        println!();
        verdict(
            "handle herdado usável sob a jaula",
            payload == "ping",
            "o filho escreveu no pipe pelo handle que herdou — o inbox recebe o PreToolUse mesmo enjaulado",
        );
        verdict(
            "acesso ao pipe PELO NOME negado",
            code == 13,
            "o filho NÃO conseguiu reabrir o pipe pelo nome (esperado: só o handle herdado vale, o resto é DACL/IL)",
        );

        println!("\ncódigo de saída do filho: {code}");
        println!("  10 = herdado OK e por-nome negado (o cenário que queremos)");
        println!("  11 = herdado FALHOU");
        println!("  12 = herdado OK mas por-nome também abriu (jaula fraca no pipe)");
        println!("  13 = alias de 'ambos como esperado' quando a leitura do servidor confirma");
    }
}

fn child(inherited: Handle, pipe_name: &str) -> i32 {
    unsafe {
        let mut pipe = std::fs::File::from_raw_handle(inherited as RawHandle);
        let inherited_ok = pipe.write_all(b"ping").and_then(|_| pipe.flush()).is_ok();
        std::mem::forget(pipe);

        let wname = wide(pipe_name);
        let by_name = CreateFileW(
            wname.as_ptr(),
            (GENERIC_READ | GENERIC_WRITE) as u32,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        let by_name_denied = by_name == INVALID_HANDLE_VALUE;
        if !by_name_denied {
            CloseHandle(by_name);
        }

        match (inherited_ok, by_name_denied) {
            (true, true) => 13,
            (false, _) => 11,
            (true, false) => 12,
        }
    }
}
