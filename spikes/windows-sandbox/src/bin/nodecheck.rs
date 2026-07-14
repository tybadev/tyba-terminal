#![cfg(windows)]

use std::ffi::c_void;
use std::io::Read;
use std::os::windows::io::{FromRawHandle, RawHandle};

use probe::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::StationsAndDesktops::*;
use windows_sys::Win32::System::Threading::*;

const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
const STATUS_DLL_INIT_FAILED: u32 = 0xC000_0142;
const GENERIC_ALL: u32 = 0x1000_0000;
const SET_ACCESS_MODE: i32 = 2;
const TRUSTEE_FORM_SID: i32 = 0;
const TRUSTEE_TYPE_UNKNOWN: i32 = 0;
const SUB_CONTAINERS_AND_OBJECTS: u32 = 0x3;
const SE_WINDOW_OBJECT: i32 = 7;
const DACL_INFO: u32 = 0x0000_0004;

struct Captured {
    exit_code: u32,
    stdout: String,
}

fn main() {
    let node = match find_node() {
        Some(p) => p,
        None => {
            println!("[ERRO] node.exe não encontrado no PATH — instale o Node ou ajuste o PATH");
            std::process::exit(2);
        }
    };

    banner("SONDA nodecheck — node.exe real sob a jaula (o agente inicia ou morre como o git?)");
    println!("Pergunta: um node.exe DE VERDADE (WRITE_RESTRICTED + IL Low, spawn por");
    println!("CreateProcessAsUserW) chega a executar JS — ou cai no 0xc0000142");
    println!("(STATUS_DLL_INIT_FAILED) que o git.exe deu, sinal de window station negada?");
    println!("E se cair: conceder o SID sintético à winsta+desktop conserta? (precedente Chromium)\n");
    println!("node: {node}\n");

    let cmd = format!("\"{node}\" -e \"console.log('jailed ok')\"");

    let control = match spawn_capturing(std::ptr::null_mut(), &cmd) {
        Ok(c) => c,
        Err(e) => {
            println!("[ERRO] controle (node sem jaula) não spawnou: {e}");
            std::process::exit(2);
        }
    };

    let restricted = match build_restricted_low_il() {
        Ok(r) => r,
        Err(e) => {
            println!("[ERRO] montar token restrito: {e}");
            std::process::exit(2);
        }
    };
    let bare = match spawn_capturing(restricted.token, &cmd) {
        Ok(c) => c,
        Err(e) => {
            println!("[ERRO] spawn enjaulado (bare) do node: {e}");
            std::process::exit(2);
        }
    };

    let granted: Option<Result<Captured, String>> = if bare.exit_code == STATUS_DLL_INIT_FAILED {
        let grant = unsafe { grant_winsta_desktop(restricted.synthetic_sid) };
        Some(match grant {
            Ok(()) => spawn_capturing(restricted.token, &cmd),
            Err(e) => Err(format!("conceder winsta/desktop ao SID: {e}")),
        })
    } else {
        None
    };

    let medium = match build_restricted(false) {
        Ok(r) => spawn_capturing(r.token, &cmd),
        Err(e) => Err(format!("montar token restrito Medium IL: {e}")),
    };

    let control_ok = control.exit_code == 0 && control.stdout.contains("jailed ok");
    let bare_started = bare.exit_code == 0 && bare.stdout.contains("jailed ok");

    println!();
    verdict(
        "controle: node inicia SEM jaula",
        control_ok,
        "prova que a linha de comando e a captura de stdout estão certas antes de acusar a jaula",
    );
    verdict(
        "jaula bare: node inicia sob WRITE_RESTRICTED + IL Low (sem concessão)",
        bare_started,
        "se FAIL com 0xc0000142, o token restrito não alcança a window station — igual ao git",
    );

    match &medium {
        Ok(m) => {
            let med_started = m.exit_code == 0 && m.stdout.contains("jailed ok");
            verdict(
                "isola IL: node inicia RESTRITO mas em IL Medium (sem rebaixar integridade)",
                med_started,
                "se PASS aqui e FAIL no bare, o culpado do 0xc0000142 é o IL Low (desktop nega write-up), não a restrição do token",
            );
        }
        Err(e) => println!("[ERRO] medição IL Medium: {e}"),
    }

    println!("\ncontrole   -> exit {} | stdout: {:?}", fmt_code(control.exit_code), control.stdout.trim());
    println!("bare(Low)  -> exit {} | stdout: {:?}", fmt_code(bare.exit_code), bare.stdout.trim());
    if let Ok(m) = &medium {
        println!("restr(Med) -> exit {} | stdout: {:?}", fmt_code(m.exit_code), m.stdout.trim());
    }

    match &granted {
        None => {
            if bare_started {
                println!("\n>>> node inicia enjaulado SEM tocar a window station. A Camada A pode");
                println!("    spawnar o agente direto — sem a concessão de winsta que o git exigiria.");
            } else {
                println!("\n>>> bare falhou, mas NÃO com 0xc0000142 — falha diferente do git.");
                println!("    Ler o exit code acima antes de assumir a causa; sem etapa de concessão.");
            }
        }
        Some(Err(e)) => {
            verdict(
                "concessão winsta+desktop ao SID e re-spawn",
                false,
                "a própria concessão falhou — ver erro abaixo",
            );
            println!("\n>>> falha ao conceder/re-spawnar: {e}");
        }
        Some(Ok(g)) => {
            let granted_started = g.exit_code == 0 && g.stdout.contains("jailed ok");
            verdict(
                "jaula + concessão: node inicia após conceder winsta+desktop ao SID",
                granted_started,
                "se PASS, a jaula do Windows precisa conceder window station/desktop ao SID da sessão (como o Chromium) e então o agente roda",
            );
            println!("granted    -> exit {} | stdout: {:?}", fmt_code(g.exit_code), g.stdout.trim());
            if granted_started {
                println!("\n>>> DECIDIDO: bare morre em 0xc0000142; conceder winsta0\\default + desktop");
                println!("    ao SID sintético faz o node.exe iniciar enjaulado. A jaula real (Camada A)");
                println!("    deve incluir essa concessão no spawn — é a etapa que o git expôs.");
            } else {
                println!("\n>>> A concessão de winsta+desktop ao SID NÃO bastou (exit {}).", fmt_code(g.exit_code));
                println!("    Cruzando com a medição de IL Medium (que falha igual), fica isolado:");
                println!("    o 0xc0000142 NÃO é o IL Low nem a DACL da window station pelo SID.");
                println!("    Sobra a própria restrição do token barrando a conexão com a sessão");
                println!("    interativa (csrss/winsta). Próximo spike: winsta+desktop dedicados com");
                println!("    trustee correto (modelo Chromium), não um grant de DACL na winsta0.");
            }
        }
    }
}

fn fmt_code(code: u32) -> String {
    format!("{code} (0x{code:08X})")
}

fn find_node() -> Option<String> {
    let mut roots: Vec<String> = Vec::new();
    if let Ok(path) = std::env::var("PATH") {
        roots.extend(path.split(';').map(|s| s.to_string()));
    }
    roots.push(r"C:\Program Files\nodejs".to_string());
    for dir in roots {
        if dir.trim().is_empty() {
            continue;
        }
        let candidate = std::path::Path::new(dir.trim()).join("node.exe");
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

unsafe fn grant_object(handle: Handle, sid: *mut c_void) -> Result<(), String> {
    let mut old_dacl: *mut ACL = std::ptr::null_mut();
    let mut psd: *mut c_void = std::ptr::null_mut();
    let rc = GetSecurityInfo(
        handle,
        SE_WINDOW_OBJECT,
        DACL_INFO,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        &mut old_dacl,
        std::ptr::null_mut(),
        &mut psd,
    );
    if rc != 0 {
        return Err(format!("GetSecurityInfo falhou: {rc}"));
    }

    let mut ea: EXPLICIT_ACCESS_W = std::mem::zeroed();
    ea.grfAccessPermissions = GENERIC_ALL;
    ea.grfAccessMode = SET_ACCESS_MODE;
    ea.grfInheritance = SUB_CONTAINERS_AND_OBJECTS;
    ea.Trustee.TrusteeForm = TRUSTEE_FORM_SID;
    ea.Trustee.TrusteeType = TRUSTEE_TYPE_UNKNOWN;
    ea.Trustee.ptstrName = sid as *mut u16;

    let mut new_dacl: *mut ACL = std::ptr::null_mut();
    let rc = SetEntriesInAclW(1, &ea, old_dacl, &mut new_dacl);
    if rc != 0 {
        return Err(format!("SetEntriesInAclW falhou: {rc}"));
    }

    let rc = SetSecurityInfo(
        handle,
        SE_WINDOW_OBJECT,
        DACL_INFO,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        new_dacl,
        std::ptr::null_mut(),
    );
    if rc != 0 {
        return Err(format!("SetSecurityInfo falhou: {rc}"));
    }
    Ok(())
}

unsafe fn grant_winsta_desktop(sid: *mut c_void) -> Result<(), String> {
    let winsta = GetProcessWindowStation() as Handle;
    if winsta.is_null() {
        return Err(format!("GetProcessWindowStation falhou: {}", last_error()));
    }
    grant_object(winsta, sid).map_err(|e| format!("winsta: {e}"))?;

    let desktop = GetThreadDesktop(GetCurrentThreadId()) as Handle;
    if desktop.is_null() {
        return Err(format!("GetThreadDesktop falhou: {}", last_error()));
    }
    grant_object(desktop, sid).map_err(|e| format!("desktop: {e}"))?;
    Ok(())
}

fn spawn_capturing(token: Handle, command_line: &str) -> Result<Captured, String> {
    unsafe {
        let mut sa: SECURITY_ATTRIBUTES = std::mem::zeroed();
        sa.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
        sa.bInheritHandle = 1;

        let mut read_h: Handle = std::ptr::null_mut();
        let mut write_h: Handle = std::ptr::null_mut();
        if CreatePipe(&mut read_h, &mut write_h, &sa, 0) == 0 {
            return Err(format!("CreatePipe falhou: {}", last_error()));
        }
        if SetHandleInformation(read_h, HANDLE_FLAG_INHERIT, 0) == 0 {
            return Err(format!("SetHandleInformation(read) falhou: {}", last_error()));
        }

        let mut si_ex: STARTUPINFOEXW = std::mem::zeroed();
        si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si_ex.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si_ex.StartupInfo.hStdInput = std::ptr::null_mut();
        si_ex.StartupInfo.hStdOutput = write_h;
        si_ex.StartupInfo.hStdError = write_h;

        let mut attr_size: usize = 0;
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attr_size);
        let mut attr_buf = vec![0u8; attr_size];
        let attr_list = attr_buf.as_mut_ptr() as *mut _;
        if InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_size) == 0 {
            return Err(format!(
                "InitializeProcThreadAttributeList falhou: {}",
                last_error()
            ));
        }
        let mut handles: Vec<Handle> = vec![write_h];
        if UpdateProcThreadAttribute(
            attr_list,
            0,
            HANDLE_LIST_ATTRIBUTE,
            handles.as_mut_ptr() as *const c_void,
            handles.len() * std::mem::size_of::<Handle>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        ) == 0
        {
            DeleteProcThreadAttributeList(attr_list);
            return Err(format!("UpdateProcThreadAttribute falhou: {}", last_error()));
        }
        si_ex.lpAttributeList = attr_list;

        let mut cmd = wide(command_line);
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let flags = EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT;

        let created = if token.is_null() {
            CreateProcessW(
                std::ptr::null(),
                cmd.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                flags,
                std::ptr::null(),
                std::ptr::null(),
                &si_ex.StartupInfo,
                &mut pi,
            )
        } else {
            CreateProcessAsUserW(
                token,
                std::ptr::null(),
                cmd.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                flags,
                std::ptr::null(),
                std::ptr::null(),
                &si_ex.StartupInfo,
                &mut pi,
            )
        };

        if created == 0 {
            let err = last_error();
            DeleteProcThreadAttributeList(attr_list);
            CloseHandle(read_h);
            CloseHandle(write_h);
            return Err(format!("CreateProcess falhou: {err}"));
        }

        CloseHandle(write_h);
        DeleteProcThreadAttributeList(attr_list);

        let mut out = String::new();
        let mut reader = std::fs::File::from_raw_handle(read_h as RawHandle);
        let _ = reader.read_to_string(&mut out);

        WaitForSingleObject(pi.hProcess, INFINITE);
        let mut code: u32 = 0;
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);

        Ok(Captured {
            exit_code: code,
            stdout: out,
        })
    }
}
