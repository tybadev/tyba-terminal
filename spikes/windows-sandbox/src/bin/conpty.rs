#![cfg(windows)]

use std::ffi::c_void;
use std::io::Read;
use std::os::windows::io::{FromRawHandle, RawHandle};

use probe::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::*;

const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;

fn main() {
    banner("SONDA conpty — o agente enxerga um TTY sob a jaula? (ConPTY + token restrito)");
    println!("O agente roda num pseudo-console (ConPTY) — é dele que vêm cores, prompt e isatty.");
    println!("Pergunta: um filho sob token restrito (jail_spec), spawnado com o atributo");
    println!("PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, vê process.stdout.isTTY == true — ou a jaula");
    println!("mata o ConPTY? PAR POSITIVO: o mesmo spawn SEM jaula deve dar isTTY=true.");
    println!("IMPORTANTE: rode com CONSOLE REAL (Start-Process conpty.exe -Wait), não com stdout");
    println!("redirecionado — ConPTY exige pai com console; resultado também em %TEMP%\\tyba-conpty-result.txt.\n");

    let node = match node_exe() {
        Some(p) => p,
        None => {
            println!("[ERRO] node.exe não encontrado");
            std::process::exit(2);
        }
    };
    // isTTY vira exit code — inequívoco, sem depender de capturar a saída do ConPTY.
    let cmd = format!("\"{node}\" -e \"process.exit(process.stdout.isTTY?7:8)\"");

    let control = run_in_conpty(&cmd, std::ptr::null_mut());
    let jailed = match build_restricted(&jail_spec(true)) {
        Ok(r) => run_in_conpty(&cmd, r.token),
        Err(e) => Err(format!("token da jaula: {e}")),
    };

    let control_tty = matches!(&control, Ok(7));
    let jailed_tty = matches!(&jailed, Ok(7));

    println!();
    verdict(
        "CONTROLE: node no ConPTY SEM jaula vê um TTY (exit 7 = isTTY true)",
        control_tty,
        "prova que o ConPTY do teste funciona — se isto falhar, nada abaixo é atribuível à jaula",
    );
    verdict(
        "node no ConPTY SOB a jaula (token restrito) vê um TTY (exit 7)",
        jailed_tty,
        "se PASS, o atributo PSEUDOCONSOLE sobrevive ao CreateProcessAsUserW — a jaula pode dar TTY ao agente direto",
    );

    let fmt = |r: &Result<u32, String>| match r {
        Ok(7) => "exit 7 (isTTY=true)".to_string(),
        Ok(8) => "exit 8 (isTTY=false)".to_string(),
        Ok(c) => format!("exit {c}"),
        Err(e) => format!("ERRO: {e}"),
    };
    println!("\ncontrole  -> {}", fmt(&control));
    println!("enjaulado -> {}", fmt(&jailed));

    // grava num arquivo para permitir rodar com console REAL (Start-Process, sem pipe)
    let result_file = std::env::temp_dir().join("tyba-conpty-result.txt");
    let _ = std::fs::write(
        &result_file,
        format!("controle={}\nenjaulado={}\n", fmt(&control), fmt(&jailed)),
    );

    if !control_tty {
        println!("\n>>> CONTROLE VERMELHO — não é conclusão sobre a jaula. O ConPTY exige um pai");
        println!("    com CONSOLE REAL; se o stdout do probe estiver redirecionado (pipe/arquivo,");
        println!("    ex.: `cargo run | ...`), o isatty falha por artefato do harness. Rode com");
        println!("    console real: `Start-Process conpty.exe -Wait` e leia %TEMP%\\tyba-conpty-result.txt.");
    } else if control_tty && !jailed_tty {
        println!("\n>>> O ConPTY NÃO sobrevive ao token restrito com o atributo direto. O agente");
        println!("    precisaria de outro caminho de TTY (helper com handles herdados, ou o core");
        println!("    dono do ConPTY spawnando o agente enjaulado). Próximo spike decide qual.");
    } else if jailed_tty {
        println!("\n>>> DECIDIDO: o atributo PSEUDOCONSOLE + CreateProcessAsUserW(token restrito)");
        println!("    dá TTY ao agente enjaulado. sandbox/windows.rs pode spawnar o agente direto");
        println!("    sob a jaula com o ConPTY — sem helper de passagem de handle.");
    }
}

fn run_in_conpty(command_line: &str, token: Handle) -> Result<u32, String> {
    unsafe {
        let mut sa: SECURITY_ATTRIBUTES = std::mem::zeroed();
        sa.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
        sa.bInheritHandle = 0;

        let mut in_read: Handle = std::ptr::null_mut();
        let mut in_write: Handle = std::ptr::null_mut();
        let mut out_read: Handle = std::ptr::null_mut();
        let mut out_write: Handle = std::ptr::null_mut();
        if CreatePipe(&mut in_read, &mut in_write, &sa, 0) == 0 {
            return Err(format!("CreatePipe(in): {}", last_error()));
        }
        if CreatePipe(&mut out_read, &mut out_write, &sa, 0) == 0 {
            return Err(format!("CreatePipe(out): {}", last_error()));
        }

        let size = COORD { X: 120, Y: 30 };
        let mut hpcon: HPCON = 0;
        let hr = CreatePseudoConsole(size, in_read, out_write, 0, &mut hpcon);
        if hr != 0 {
            return Err(format!("CreatePseudoConsole: 0x{hr:08X}"));
        }
        // o ConPTY duplicou as pontas; fechamos as nossas cópias
        CloseHandle(in_read);
        CloseHandle(out_write);

        // thread que drena a saída do ConPTY até EOF
        let out_read_val = out_read as usize;
        let reader = std::thread::spawn(move || {
            let mut file = std::fs::File::from_raw_handle(out_read_val as RawHandle);
            let mut buf = Vec::new();
            let _ = file.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).to_string()
        });

        // lista de atributos com o pseudo-console
        let mut si_ex: STARTUPINFOEXW = std::mem::zeroed();
        si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        let mut attr_size: usize = 0;
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attr_size);
        let mut attr_buf = vec![0u8; attr_size];
        let attr_list = attr_buf.as_mut_ptr() as *mut _;
        if InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_size) == 0 {
            return Err(format!("InitializeProcThreadAttributeList: {}", last_error()));
        }
        if UpdateProcThreadAttribute(
            attr_list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            hpcon as *const c_void,
            std::mem::size_of::<HPCON>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        ) == 0
        {
            DeleteProcThreadAttributeList(attr_list);
            return Err(format!("UpdateProcThreadAttribute(pseudoconsole): {}", last_error()));
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
                0,
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
                0,
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
            ClosePseudoConsole(hpcon);
            CloseHandle(in_write);
            return Err(format!("CreateProcess (jailed={}): {err}", !token.is_null()));
        }

        WaitForSingleObject(pi.hProcess, 8000);
        let mut code: u32 = 0;
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
        DeleteProcThreadAttributeList(attr_list);

        // fechar o console solta o EOF na saída → a thread leitora termina
        ClosePseudoConsole(hpcon);
        CloseHandle(in_write);

        let _ = reader.join();
        Ok(code)
    }
}
