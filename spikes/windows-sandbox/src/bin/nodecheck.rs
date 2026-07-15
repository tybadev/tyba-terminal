#![cfg(windows)]

use std::ffi::c_void;
use std::io::Read;
use std::os::windows::io::{FromRawHandle, RawHandle};

use probe::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::*;

const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
const STATUS_DLL_INIT_FAILED: u32 = 0xC000_0142;
const INTERACTIVE_DESKTOP: &str = "Winsta0\\Default";

struct Captured {
    exit_code: u32,
    stdout: String,
}

fn codex_recipe(low_il: bool) -> TokenSpec {
    jail_spec(low_il)
}

fn main() {
    let node = match find_node() {
        Some(p) => p,
        None => {
            println!("[ERRO] node.exe não encontrado no PATH — instale o Node ou ajuste o PATH");
            std::process::exit(2);
        }
    };

    banner("SONDA nodecheck — node.exe real sob a jaula: qual receita faz o agente iniciar?");
    println!("O bare (WRITE_RESTRICTED + IL Low, sem lpDesktop) dá 0xc0000142 igual ao git.");
    println!("Esta escada ablaciona a receita conhecida-boa (codex windows-sandbox-rs):");
    println!("logon SID + Everyone no restricting set, flags LUA_TOKEN, e lpDesktop explícito.\n");
    println!("node: {node}\n");

    let cmd = format!("\"{node}\" -e \"console.log('jailed ok')\"");

    let mut rungs: Vec<Rung> = Vec::new();

    rungs.push(measure(
        "controle: node sem jaula",
        &cmd,
        None,
        None,
        "baseline — prova comando e captura de stdout",
    ));
    rungs.push(measure(
        "bare: só SID sintético, IL Low, SEM lpDesktop (a jaula original)",
        &cmd,
        Some(TokenSpec::default()),
        None,
        "reproduz o 0xc0000142; é o ponto de partida a consertar",
    ));
    rungs.push(measure(
        "RECEITA codex: +logon +Everyone, LUA_TOKEN, IL Medium, lpDesktop=Winsta0\\Default",
        &cmd,
        Some(codex_recipe(false)),
        Some(INTERACTIVE_DESKTOP),
        "receita conhecida-boa completa — se PASS, o agente roda enjaulado",
    ));
    rungs.push(measure(
        "ablação lpDesktop: receita codex mas SEM lpDesktop",
        &cmd,
        Some(codex_recipe(false)),
        None,
        "se FAIL só aqui, o lpDesktop explícito é o fator decisivo",
    ));
    rungs.push(measure(
        "ablação Everyone: +logon SEM Everyone, IL Medium, lpDesktop set",
        &cmd,
        Some(TokenSpec {
            low_il: false,
            add_logon_sid: true,
            add_everyone: false,
            flags: WRITE_RESTRICTED_FLAG | DISABLE_MAX_PRIVILEGE_FLAG | LUA_TOKEN_FLAG,
        }),
        Some(INTERACTIVE_DESKTOP),
        "se FAIL só aqui, o Everyone no restricting set é necessário para iniciar",
    ));
    rungs.push(measure(
        "ablação IL: receita codex mas em IL Low (defesa em profundidade)",
        &cmd,
        Some(codex_recipe(true)),
        Some(INTERACTIVE_DESKTOP),
        "se PASS, dá pra manter IL Low com lpDesktop set; se FAIL, o agente fica em Medium",
    ));

    println!();
    for r in &rungs {
        verdict(&r.label, r.started, &r.meaning);
        if let Some(sids) = &r.restricting {
            println!("       tok: {sids}");
        }
        println!(
            "       -> exit {} | stdout: {:?}",
            fmt_code(r.exit_code),
            r.stdout.trim()
        );
    }

    print_conclusion(&rungs);
}

struct Rung {
    label: String,
    meaning: String,
    started: bool,
    exit_code: u32,
    stdout: String,
    restricting: Option<String>,
}

fn measure(label: &str, cmd: &str, spec: Option<TokenSpec>, desktop: Option<&str>, meaning: &str) -> Rung {
    let (token, restricting) = match spec {
        None => (std::ptr::null_mut(), None),
        Some(s) => match build_restricted(&s) {
            Ok(r) => {
                let sids = describe_restricting_sids(r.token);
                (r.token, Some(sids))
            }
            Err(e) => {
                return Rung {
                    label: label.to_string(),
                    meaning: format!("{meaning} [ERRO ao montar token: {e}]"),
                    started: false,
                    exit_code: 0,
                    stdout: String::new(),
                    restricting: None,
                }
            }
        },
    };

    match spawn_capturing(token, cmd, desktop) {
        Ok(c) => {
            let started = c.exit_code == 0 && c.stdout.contains("jailed ok");
            Rung {
                label: label.to_string(),
                meaning: meaning.to_string(),
                started,
                exit_code: c.exit_code,
                stdout: c.stdout,
                restricting,
            }
        }
        Err(e) => Rung {
            label: label.to_string(),
            meaning: format!("{meaning} [ERRO no spawn: {e}]"),
            started: false,
            exit_code: 0,
            stdout: String::new(),
            restricting,
        },
    }
}

fn print_conclusion(rungs: &[Rung]) {
    let recipe = rungs.get(2).map(|r| r.started).unwrap_or(false);
    let no_desktop = rungs.get(3).map(|r| r.started).unwrap_or(false);
    let no_everyone = rungs.get(4).map(|r| r.started).unwrap_or(false);
    let low_il = rungs.get(5).map(|r| r.started).unwrap_or(false);

    println!("\n======== conclusão ========");
    if !recipe {
        println!("A receita codex NÃO fez o node iniciar — ver exit acima. O modelo do produto");
        println!("(jaula por token restrito) precisa de revisão antes de virar sandbox/windows.rs.");
        return;
    }
    println!("A receita codex FAZ o node iniciar enjaulado. Fatores isolados:");
    println!(
        "  - lpDesktop explícito (Winsta0\\Default): {}",
        if no_desktop { "dispensável" } else { "NECESSÁRIO (sem ele volta o 0xc0000142)" }
    );
    println!(
        "  - Everyone no restricting set: {}",
        if no_everyone { "dispensável (logon SID basta)" } else { "NECESSÁRIO" }
    );
    println!(
        "  - IL Low em vez de Medium: {}",
        if low_il { "OK — dá pra manter Low IL (defesa em profundidade)" } else { "não inicia em Low; agente fica em Medium IL" }
    );
    println!("\nO SID sintético permanece no restricting set — é a chave da confinação de");
    println!("escrita ao worktree (sonda B). Próximo passo: re-rodar a sonda worktree com");
    println!("ESTA receita (logon+Everyone presentes) e confirmar que a negação-fora sobrevive.");
}

fn fmt_code(code: u32) -> String {
    if code == STATUS_DLL_INIT_FAILED {
        format!("{code} (0xC0000142 STATUS_DLL_INIT_FAILED)")
    } else {
        format!("{code} (0x{code:08X})")
    }
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

fn spawn_capturing(token: Handle, command_line: &str, desktop: Option<&str>) -> Result<Captured, String> {
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

        let mut desktop_w = desktop.map(wide);

        let mut si_ex: STARTUPINFOEXW = std::mem::zeroed();
        si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si_ex.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si_ex.StartupInfo.hStdInput = std::ptr::null_mut();
        si_ex.StartupInfo.hStdOutput = write_h;
        si_ex.StartupInfo.hStdError = write_h;
        if let Some(d) = desktop_w.as_mut() {
            si_ex.StartupInfo.lpDesktop = d.as_mut_ptr();
        }

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
