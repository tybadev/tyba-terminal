#![cfg(windows)]

use std::ffi::c_void;

use probe::*;
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;

const DENY_ACCESS_MODE: i32 = 3;
const TRUSTEE_FORM_SID: i32 = 0;
const TRUSTEE_TYPE_UNKNOWN: i32 = 0;
const GENERIC_READ_MASK: u32 = 0x8000_0000;
const SE_FILE: i32 = 1;
const DACL_INFO: u32 = 0x0000_0004;
const LABEL_INFO: u32 = 0x0000_0010;
const SDDL_REV1: u32 = 1;

fn main() {
    banner("SONDA denyacl — negar leitura de segredo ao agente (~/.ssh etc.), sem afetar o dono");
    println!("O agente Low IL herda a identidade do usuário, então um deny-ACE no SID sintético");
    println!("NÃO barra leitura (ele só conta no 2º check de escrita). Esta sonda mede os dois");
    println!("mecanismos: deny-ACE por SID (esperado: não barra) vs rótulo IL NO_READ_UP (barra).\n");

    let node = match node_exe() {
        Some(p) => p,
        None => {
            println!("[ERRO] node.exe não encontrado");
            std::process::exit(2);
        }
    };

    let restricted = match build_restricted(&jail_spec(true)) {
        Ok(r) => r,
        Err(e) => {
            println!("[ERRO] token da jaula: {e}");
            std::process::exit(2);
        }
    };

    let base = std::env::temp_dir().join(format!("tyba-spike-deny-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let open = base.join("open.txt");
    let dacl = base.join("dacl_secret.txt");
    let il = base.join("il_secret.txt");
    std::fs::write(&open, b"nao-secreto").unwrap();
    std::fs::write(&dacl, b"segredo-via-dacl").unwrap();
    std::fs::write(&il, b"segredo-via-il").unwrap();

    unsafe {
        if let Err(e) = deny_read_for_sid(&dacl.to_string_lossy(), restricted.synthetic_sid) {
            println!("[ERRO] aplicar deny-ACE no SID: {e}");
        }
        if let Err(e) = label_no_read_up(&il.to_string_lossy()) {
            println!("[ERRO] rotular IL NO_READ_UP: {e}");
        }
    }

    let script = "const fs=require('fs'); const a=process.argv; \
        const r=p=>{try{fs.readFileSync(p);return 'ok'}catch(e){return e.code||'err'}}; \
        console.log(JSON.stringify({open:r(a[1]),dacl:r(a[2]),il:r(a[3])}))";
    let cmd = format!(
        "\"{node}\" -e \"{script}\" \"{}\" \"{}\" \"{}\"",
        open.display(),
        dacl.display(),
        il.display()
    );

    let (code, out) = match capture(restricted.token, &cmd, Some("Winsta0\\Default"), None, 0, None) {
        Ok(v) => v,
        Err(e) => {
            println!("[ERRO] spawn enjaulado: {e}");
            let _ = std::fs::remove_dir_all(&base);
            std::process::exit(2);
        }
    };
    let trimmed = out.trim();

    let open_ok = trimmed.contains("\"open\":\"ok\"");
    let il_denied = !trimmed.contains("\"il\":\"ok\"") && code == 0;
    let dacl_ineffective = trimmed.contains("\"dacl\":\"ok\"") && code == 0;

    println!();
    verdict(
        "controle: agente LÊ um arquivo normal (open.txt)",
        open_ok,
        "prova que a jaula não nega leitura por acidente — o que for negado abaixo é intencional",
    );
    verdict(
        "rótulo IL NO_READ_UP nega a leitura do segredo ao agente",
        il_denied,
        "mecanismo que FUNCIONA para leitura: Low IL não lê arquivo Medium com NR; o dono (Medium) lê normal",
    );
    verdict(
        "confirmado: deny-ACE por SID é inócuo para leitura (achado — usar IL, não DACL-por-SID)",
        dacl_ineffective,
        "o agente leu o arquivo com deny-ACE no SID sintético; SID só conta no 2º check de ESCRITA. Corrige o tech-spec: confinação de LEITURA é por rótulo IL",
    );

    println!("\nexit {code} | leituras do node: {trimmed}");
    println!("(open=ok esperado · il=negado esperado · dacl=ok esperado, provando que SID-deny não barra leitura)");

    let _ = std::fs::remove_dir_all(&base);
}

unsafe fn deny_read_for_sid(path: &str, sid: *mut c_void) -> Result<(), String> {
    let mut ea: EXPLICIT_ACCESS_W = std::mem::zeroed();
    ea.grfAccessPermissions = GENERIC_READ_MASK;
    ea.grfAccessMode = DENY_ACCESS_MODE;
    ea.grfInheritance = 0;
    ea.Trustee.TrusteeForm = TRUSTEE_FORM_SID;
    ea.Trustee.TrusteeType = TRUSTEE_TYPE_UNKNOWN;
    ea.Trustee.ptstrName = sid as *mut u16;

    let mut new_acl = std::ptr::null_mut();
    let rc = SetEntriesInAclW(1, &ea, std::ptr::null(), &mut new_acl);
    if rc != 0 {
        return Err(format!("SetEntriesInAclW: {rc}"));
    }
    let mut wpath = wide(path);
    let rc = SetNamedSecurityInfoW(
        wpath.as_mut_ptr(),
        SE_FILE,
        DACL_INFO,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        new_acl,
        std::ptr::null_mut(),
    );
    if rc != 0 {
        return Err(format!("SetNamedSecurityInfoW: {rc}"));
    }
    Ok(())
}

unsafe fn label_no_read_up(path: &str) -> Result<(), String> {
    let sddl = wide("S:(ML;;NRNW;;;ME)");
    let mut psd: *mut c_void = std::ptr::null_mut();
    if ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl.as_ptr(),
        SDDL_REV1,
        &mut psd,
        std::ptr::null_mut(),
    ) == 0
    {
        return Err(format!("Convert SDDL: {}", last_error()));
    }
    let wpath = wide(path);
    if SetFileSecurityW(wpath.as_ptr(), LABEL_INFO, psd) == 0 {
        return Err(format!("SetFileSecurityW(label): {}", last_error()));
    }
    Ok(())
}
