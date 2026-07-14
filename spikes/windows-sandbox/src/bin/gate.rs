#![cfg(windows)]

use std::ffi::c_void;
use std::io::{Read, Write};
use std::os::windows::io::{FromRawHandle, RawHandle};

use probe::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Pipes::*;

const GEN_READ: u32 = 0x8000_0000;
const GEN_WRITE: u32 = 0x4000_0000;
const ERROR_ACCESS_DENIED_CODE: u32 = 5;
const SDDL_REV1: u32 = 1;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "--child" {
        let handle_value: usize = args[2].parse().unwrap_or(0);
        std::process::exit(child(handle_value as Handle, &args[3]));
    }
    if args.len() >= 3 && args[1] == "--control" {
        std::process::exit(control(&args[2]));
    }
    parent();
}

fn parent() {
    banner("SONDA A — gate por named pipe herdado (a peça sem a qual não há produto)");
    println!("Duas metades. (1) O agente enjaulado FALA com o inbox por um handle HERDADO.");
    println!("(2) O acesso ao pipe PELO NOME é NEGADO. Esta versão isola o mecanismo: o pipe");
    println!("do teste-por-nome tem instâncias LIVRES (sem PIPE_BUSY) e DACL liberando Everyone,");
    println!("com rótulo IL Medium (no-write-up) — então só o IL Low nega. PAR POSITIVO: um");
    println!("processo NÃO-enjaulado abre o mesmo nome (prova que o nome é abrível, não é BUSY).\n");

    let pid = std::process::id();
    let name_a = format!(r"\\.\pipe\tyba-spike-inh-{pid}");
    let name_b = format!(r"\\.\pipe\tyba-spike-name-{pid}");
    let wa = wide(&name_a);

    unsafe {
        // Pipe A — o gate real: handle herdado.
        let server_a = CreateNamedPipeW(
            wa.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_WAIT,
            1,
            4096,
            4096,
            0,
            std::ptr::null(),
        );
        if server_a == INVALID_HANDLE_VALUE {
            println!("[ERRO] CreateNamedPipeW(A): {}", last_error());
            return;
        }
        let client_a = CreateFileW(
            wa.as_ptr(),
            GEN_READ | GEN_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if client_a == INVALID_HANDLE_VALUE {
            println!("[ERRO] CreateFileW(A client): {}", last_error());
            return;
        }
        SetHandleInformation(client_a, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);

        // Pipe B — teste por-nome. DACL: Everyone GENERIC_ALL (DACL não nega).
        // Rótulo: IL Medium no-write-up (só o IL Low nega). Duas instâncias livres → sem PIPE_BUSY.
        let sddl = wide("D:(A;;GA;;;WD)S:(ML;;NW;;;ME)");
        let mut psd: *mut c_void = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REV1,
            &mut psd,
            std::ptr::null_mut(),
        ) == 0
        {
            println!("[ERRO] Convert SDDL(B): {}", last_error());
            return;
        }
        let mut sa: SECURITY_ATTRIBUTES = std::mem::zeroed();
        sa.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
        sa.lpSecurityDescriptor = psd;
        sa.bInheritHandle = 0;
        let wb = wide(&name_b);
        for _ in 0..2 {
            let inst = CreateNamedPipeW(
                wb.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_WAIT,
                2,
                4096,
                4096,
                0,
                &sa,
            );
            if inst == INVALID_HANDLE_VALUE {
                println!("[ERRO] CreateNamedPipeW(B): {}", last_error());
                return;
            }
        }

        // PAR POSITIVO: processo NÃO-enjaulado abre o pipe B pelo nome.
        let exe = std::env::current_exe().unwrap();
        let control_code = std::process::Command::new(&exe)
            .arg("--control")
            .arg(&name_b)
            .status()
            .ok()
            .and_then(|s| s.code())
            .unwrap_or(-1);
        let control_opened = control_code == 0;

        // TESTE ENJAULADO: o filho fala por A (herdado) e tenta abrir B pelo nome.
        let restricted = match build_restricted(&jail_spec(true)) {
            Ok(r) => r,
            Err(e) => {
                println!("[ERRO] token da jaula: {e}");
                return;
            }
        };
        let cmd = format!("\"{}\" --child {} {}", exe.display(), client_a as usize, name_b);
        if let Err(e) = spawn_with_token(restricted.token, &cmd, &[client_a]) {
            println!("[ERRO] spawn enjaulado: {e}");
            return;
        }

        let mut server_file = std::fs::File::from_raw_handle(server_a as RawHandle);
        let mut buf = [0u8; 128];
        let n = server_file.read(&mut buf).unwrap_or(0);
        let report = String::from_utf8_lossy(&buf[..n]).to_string();

        let inherited_ok = report.starts_with("ping");
        let byname_denied = report.contains("byname=denied");
        let gle = report
            .split("gle=")
            .nth(1)
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);

        println!();
        verdict(
            "handle herdado usável sob a jaula",
            inherited_ok,
            "o filho escreveu no pipe pelo handle que herdou — o inbox recebe o PreToolUse mesmo enjaulado",
        );
        verdict(
            "CONTROLE: processo NÃO-enjaulado abre o pipe B pelo nome",
            control_opened,
            "prova que o nome É abrível (instância livre, DACL libera) — descarta PIPE_BUSY e inexistência",
        );
        verdict(
            "acesso ao pipe PELO NOME negado à jaula, com ACCESS_DENIED (não BUSY)",
            byname_denied && gle == ERROR_ACCESS_DENIED_CODE && control_opened,
            "o filho Low IL foi negado por integrity level (no-write-up ao pipe Medium); GetLastError=5 confirma que é o IL, não instância ocupada",
        );

        println!("\nrelato do filho: {report:?}");
        println!("controle (não-enjaulado) abrindo por nome: exit {control_code} (0 = abriu)");
        println!("GetLastError do filho no open-por-nome: {gle} (5 = ACCESS_DENIED, 231 = PIPE_BUSY)");
    }
}

fn child(inherited: Handle, name_b: &str) -> i32 {
    unsafe {
        let by_name = CreateFileW(
            wide(name_b).as_ptr(),
            GEN_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        let (denied, gle) = if by_name == INVALID_HANDLE_VALUE {
            (true, last_error())
        } else {
            CloseHandle(by_name);
            (false, 0)
        };

        let mut pipe = std::fs::File::from_raw_handle(inherited as RawHandle);
        let msg = format!(
            "ping;byname={};gle={}",
            if denied { "denied" } else { "opened" },
            gle
        );
        let wrote = pipe.write_all(msg.as_bytes()).and_then(|_| pipe.flush()).is_ok();
        std::mem::forget(pipe);

        match (wrote, denied) {
            (true, true) => 13,
            (true, false) => 12,
            (false, _) => 11,
        }
    }
}

fn control(name_b: &str) -> i32 {
    unsafe {
        let h = CreateFileW(
            wide(name_b).as_ptr(),
            GEN_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return last_error() as i32;
        }
        CloseHandle(h);
        0
    }
}
