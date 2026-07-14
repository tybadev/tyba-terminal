#![cfg(windows)]

use std::ffi::c_void;

use probe::*;
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;

const SET_ACCESS_MODE: i32 = 2;
const TRUSTEE_FORM_SID: i32 = 0;
const TRUSTEE_TYPE_UNKNOWN: i32 = 0;
const SUB_CONTAINERS_AND_OBJECTS: u32 = 0x3;
const FILE_ALL: u32 = 0x001F_01FF;
const SE_FILE: i32 = 1;
const DACL_INFO: u32 = 0x0000_0004;
const LABEL_INFO: u32 = 0x0000_0010;
const SDDL_REV1: u32 = 1;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "--child" {
        std::process::exit(child(&args[2], &args[3]));
    }
    parent();
}

unsafe fn grant_write(dir: &str, sid: *mut c_void) -> Result<(), String> {
    let mut ea: EXPLICIT_ACCESS_W = std::mem::zeroed();
    ea.grfAccessPermissions = FILE_ALL;
    ea.grfAccessMode = SET_ACCESS_MODE;
    ea.grfInheritance = SUB_CONTAINERS_AND_OBJECTS;
    ea.Trustee.TrusteeForm = TRUSTEE_FORM_SID;
    ea.Trustee.TrusteeType = TRUSTEE_TYPE_UNKNOWN;
    ea.Trustee.ptstrName = sid as *mut u16;

    let mut new_acl = std::ptr::null_mut();
    let rc = SetEntriesInAclW(1, &ea, std::ptr::null(), &mut new_acl);
    if rc != 0 {
        return Err(format!("SetEntriesInAclW falhou: {rc}"));
    }
    let mut wdir = wide(dir);
    let rc = SetNamedSecurityInfoW(
        wdir.as_mut_ptr(),
        SE_FILE,
        DACL_INFO,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        new_acl,
        std::ptr::null_mut(),
    );
    if rc != 0 {
        return Err(format!("SetNamedSecurityInfoW(DACL) falhou: {rc}"));
    }
    Ok(())
}

unsafe fn label_low(dir: &str) -> Result<(), String> {
    let sddl = wide("S:(ML;OICI;NW;;;LW)");
    let mut psd: *mut c_void = std::ptr::null_mut();
    if ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl.as_ptr(),
        SDDL_REV1,
        &mut psd,
        std::ptr::null_mut(),
    ) == 0
    {
        return Err(format!("Convert SDDL falhou: {}", last_error()));
    }
    let wdir = wide(dir);
    let ok = SetFileSecurityW(wdir.as_ptr(), LABEL_INFO, psd);
    if ok == 0 {
        return Err(format!("SetFileSecurityW(label) falhou: {}", last_error()));
    }
    Ok(())
}

fn parent() {
    banner("SONDA B — confinação de escrita da jaula (worktree Low + SID restrito)");
    println!("Pergunta: o agente enjaulado (WRITE_RESTRICTED + IL Low) escreve DENTRO do");
    println!("worktree — que nasce rotulado Low e com escrita concedida ao SID da sessão —");
    println!("e é NEGADO fora dele? É o que amarra 'push para main é recusado' ao filesystem.\n");

    let base = std::env::temp_dir().join(format!("tyba-spike-wt-{}", std::process::id()));
    let worktree = base.join("repo");
    let outside = base.join("outside");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let wt = worktree.to_string_lossy().to_string();
    let out = outside.to_string_lossy().to_string();

    let restricted = match build_restricted(&jail_spec(true)) {
        Ok(r) => r,
        Err(e) => {
            println!("[ERRO] montar token restrito: {e}");
            return;
        }
    };
    println!("token da jaula real: {}", describe_restricting_sids(restricted.token));
    println!("(logon SID + Everyone estão no restricting set — o que faz o node bootar;");
    println!(" esta sonda confirma que a negação-fora sobrevive a esse conjunto mais amplo)\n");
    unsafe {
        if let Err(e) = label_low(&wt) {
            println!("[ERRO] rotular o worktree como Low IL: {e}");
            return;
        }
        if let Err(e) = grant_write(&wt, restricted.synthetic_sid) {
            println!("[ERRO] conceder escrita ao SID no worktree: {e}");
            return;
        }
    }

    let exe = std::env::current_exe().unwrap();
    let cmd = format!("\"{}\" --child \"{}\" \"{}\"", exe.display(), wt, out);
    let code = match spawn_with_token(restricted.token, &cmd, &[]) {
        Ok(c) => c,
        Err(e) => {
            println!("[ERRO] spawn enjaulado: {e}");
            return;
        }
    };

    let worktree_write_ok = code & 1 != 0;
    let outside_denied = code & 2 != 0;

    println!();
    verdict(
        "escreve dentro do worktree",
        worktree_write_ok,
        "worktree rotulado Low + ACE do SID restrito → o agente trabalha na sua árvore",
    );
    verdict(
        "escrita FORA do worktree negada",
        outside_denied,
        "sem rótulo Low nem ACE lá fora, IL e WRITE_RESTRICTED barram — resíduo fica só no que é nosso",
    );

    let _ = std::fs::remove_dir_all(&base);
    println!("\ncódigo de saída do filho (bitmask): {code}  (bit0 escreve-dentro, bit1 nega-fora)");
    println!("O 0xc0000142 do git/node sob a jaula foi RESOLVIDO pela sonda nodecheck: era o");
    println!("restricting set estreito (só o SID sintético). Com logon SID + Everyone no");
    println!("conjunto — como acima — processos reais iniciam, e esta sonda confirma que a");
    println!("negação-fora sobrevive a esse conjunto mais amplo (IL Low + DACL seguram).");
}

fn child(worktree: &str, outside: &str) -> i32 {
    let mut code = 0;

    let inside = std::path::Path::new(worktree).join("agent_wrote.txt");
    if std::fs::write(&inside, b"agent").is_ok() {
        code |= 1;
    }

    let out = std::path::Path::new(outside).join("agent_wrote.txt");
    if std::fs::write(&out, b"agent").is_err() {
        code |= 2;
    }

    code
}
