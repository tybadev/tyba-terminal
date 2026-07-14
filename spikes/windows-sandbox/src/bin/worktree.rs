#![cfg(windows)]

use std::ffi::c_void;
use std::process::Command;

use probe::*;
use windows_sys::Win32::Security::Authorization::*;

const SET_ACCESS_MODE: i32 = 2;
const TRUSTEE_FORM_SID: i32 = 0;
const TRUSTEE_TYPE_UNKNOWN: i32 = 0;
const SUB_CONTAINERS_AND_OBJECTS: u32 = 0x3;
const FILE_ALL: u32 = 0x001F_01FF;
const SE_FILE: i32 = 1;
const DACL_INFO: u32 = 0x0000_0004;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "--child" {
        std::process::exit(child(&args[2], &args[3]));
    }
    parent();
}

fn grant_write(dir: &str, sid: *mut c_void) -> Result<(), String> {
    unsafe {
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
            return Err(format!("SetNamedSecurityInfoW falhou: {rc}"));
        }
        Ok(())
    }
}

fn git(dir: &str, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn parent() {
    banner("SONDA B — git escrevendo só no worktree (o modelo de escrita da jaula)");
    println!("Pergunta: com o SID restrito recebendo escrita SÓ no worktree, o agente");
    println!("enjaulado consegue commitar ali dentro e é NEGADO fora? É o que amarra o");
    println!("'push para main é recusado' a uma base de filesystem, não só a política.\n");

    let base = std::env::temp_dir().join(format!("tyba-spike-wt-{}", std::process::id()));
    let worktree = base.join("repo");
    let outside = base.join("outside");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let wt = worktree.to_string_lossy().to_string();
    let out = outside.to_string_lossy().to_string();

    if !git(&wt, &["init", "-q"])
        || !git(&wt, &["config", "user.email", "spike@tyba.dev"])
        || !git(&wt, &["config", "user.name", "spike"])
    {
        println!("[ERRO] não consegui inicializar o repo de teste (git no PATH?)");
        return;
    }
    std::fs::write(worktree.join("seed.txt"), b"seed").unwrap();
    git(&wt, &["add", "-A"]);
    git(&wt, &["commit", "-q", "-m", "seed"]);

    let restricted = match build_restricted_low_il() {
        Ok(r) => r,
        Err(e) => {
            println!("[ERRO] montar token restrito: {e}");
            return;
        }
    };
    if let Err(e) = grant_write(&wt, restricted.synthetic_sid) {
        println!("[ERRO] conceder escrita ao SID no worktree: {e}");
        return;
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
    let git_commit_ok = code & 4 != 0;

    println!();
    verdict(
        "escreve dentro do worktree",
        worktree_write_ok,
        "o SID restrito tem escrita concedida só aqui — o agente trabalha na sua árvore",
    );
    verdict(
        "escrita FORA do worktree negada",
        outside_denied,
        "sem ACE do SID lá fora, o WRITE_RESTRICTED barra — resíduo fica num diretório nosso",
    );
    verdict(
        "git.exe commita dentro da jaula",
        git_commit_ok,
        "git escreve lockfile, index, refs — se commita só no worktree, o modelo se sustenta",
    );

    let _ = std::fs::remove_dir_all(&base);
    println!("\ncódigo de saída do filho (bitmask): {code}  (bit0 escreve-dentro, bit1 nega-fora, bit2 git-ok)");
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

    git(worktree, &["add", "-A"]);
    if git(worktree, &["commit", "-q", "-m", "agent commit under jail"]) {
        code |= 4;
    }

    code
}
