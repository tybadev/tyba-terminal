#![cfg(windows)]

use std::ffi::c_void;

use probe::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::Threading::*;

const CREATE_SUSPENDED_FLAG: u32 = 0x0000_0004;
const JOB_KILL_ON_CLOSE: u32 = 0x0000_2000;
const JOB_EXTENDED_LIMIT_CLASS: i32 = 9;
const WAIT_TIMEOUT_CODE: u32 = 0x0000_0102;
const WAIT_SIGNALED: u32 = 0x0000_0000;

fn main() {
    banner("SONDA jobobject — kill-on-close: o agente morre com o TYBA (paridade com killpg)");
    println!("Pergunta: um node.exe enjaulado, atribuído a um Job Object com");
    println!("JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, é mantido vivo enquanto o job está aberto");
    println!("(controle) e MORRE quando o handle do job fecha (o TYBA some)? É o princípio #9.\n");

    let node = match node_exe() {
        Some(p) => p,
        None => {
            println!("[ERRO] node.exe não encontrado");
            std::process::exit(2);
        }
    };
    let cmd = format!("\"{node}\" -e \"setTimeout(()=>process.exit(0), 60000)\"");

    let restricted = match build_restricted(&jail_spec(true)) {
        Ok(r) => r,
        Err(e) => {
            println!("[ERRO] token da jaula: {e}");
            std::process::exit(2);
        }
    };

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            println!("[ERRO] CreateJobObjectW falhou: {}", last_error());
            std::process::exit(2);
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_KILL_ON_CLOSE;
        if SetInformationJobObject(
            job,
            JOB_EXTENDED_LIMIT_CLASS,
            &info as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ) == 0
        {
            println!("[ERRO] SetInformationJobObject falhou: {}", last_error());
            std::process::exit(2);
        }

        let (proc, thread) = match spawn_handles(restricted.token, &cmd, CREATE_SUSPENDED_FLAG) {
            Ok(v) => v,
            Err(e) => {
                println!("[ERRO] spawn suspenso do node enjaulado: {e}");
                std::process::exit(2);
            }
        };

        let assigned = AssignProcessToJobObject(job, proc) != 0;
        let assign_err = last_error();
        ResumeThread(thread);

        let alive_while_open = WaitForSingleObject(proc, 400) == WAIT_TIMEOUT_CODE;

        CloseHandle(job);

        let died_on_close = WaitForSingleObject(proc, 4000) == WAIT_SIGNALED;

        if !died_on_close {
            TerminateProcess(proc, 1);
        }
        CloseHandle(proc);
        CloseHandle(thread);

        println!();
        verdict(
            "node enjaulado é atribuído ao Job Object",
            assigned,
            "AssignProcessToJobObject aceita o processo do token restrito — o job envolve a jaula",
        );
        if !assigned {
            println!("       (erro do assign: {assign_err})");
        }
        verdict(
            "controle: node segue vivo enquanto o job está aberto",
            alive_while_open,
            "prova que o teste não está confundindo 'morreu por kill-on-close' com 'nem subiu'",
        );
        verdict(
            "node MORRE quando o handle do job fecha",
            died_on_close,
            "kill-on-close: o TYBA sumindo leva o agente e toda a árvore junto — paridade com killpg",
        );
    }
}
