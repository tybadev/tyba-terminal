#![cfg(windows)]

use probe::*;

const DEP_ENABLE: u64 = 0x01;
const FORCE_RELOCATE_IMAGES: u64 = 0x01 << 8;
const BOTTOM_UP_ASLR: u64 = 0x01 << 16;
const STRICT_HANDLE_CHECKS: u64 = 0x01 << 20;
const HIGH_ENTROPY_ASLR: u64 = 0x01 << 24;
const WIN32K_DISABLE: u64 = 0x01 << 28;
const EXTENSION_POINT_DISABLE: u64 = 0x01 << 32;
const PROHIBIT_DYNAMIC_CODE: u64 = 0x01 << 36; // ACG — quebra o JIT do V8
const BLOCK_NON_MS_BINARIES: u64 = 0x01 << 44; // CIG — node.exe não é assinado pela MS

fn main() {
    banner("SONDA mitigations — quais process mitigations o node.exe tolera (menos ACG)");
    println!("O tech-spec lista win32k/CIG/CFG 'menos ACG', mas isso é suposição. Esta escada");
    println!("aplica cada mitigação isolada (PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY) num node");
    println!("enjaulado e mede se ele ainda inicia. O produto usa o MAIOR conjunto que não quebra.\n");

    let node = match node_exe() {
        Some(p) => p,
        None => {
            println!("[ERRO] node.exe não encontrado");
            std::process::exit(2);
        }
    };
    let cmd = format!("\"{node}\" -e \"console.log('jailed ok')\"");

    let cases: &[(&str, Option<u64>, bool)] = &[
        ("baseline (nenhuma mitigação)", None, true),
        ("DEP_ENABLE", Some(DEP_ENABLE), true),
        ("FORCE_RELOCATE_IMAGES", Some(FORCE_RELOCATE_IMAGES), true),
        ("BOTTOM_UP_ASLR", Some(BOTTOM_UP_ASLR), true),
        ("STRICT_HANDLE_CHECKS", Some(STRICT_HANDLE_CHECKS), true),
        ("HIGH_ENTROPY_ASLR", Some(HIGH_ENTROPY_ASLR), true),
        ("EXTENSION_POINT_DISABLE", Some(EXTENSION_POINT_DISABLE), true),
        ("WIN32K_DISABLE (bloqueio de syscalls GUI)", Some(WIN32K_DISABLE), false),
        ("BLOCK_NON_MS_BINARIES (CIG) [boota puro; ver alerta]", Some(BLOCK_NON_MS_BINARIES), true),
        ("PROHIBIT_DYNAMIC_CODE (ACG)", Some(PROHIBIT_DYNAMIC_CODE), false),
    ];

    let mut safe: Vec<&str> = Vec::new();
    let mut breaks: Vec<&str> = Vec::new();

    println!();
    for (label, mit, expect_boot) in cases {
        let token = match build_restricted(&jail_spec(true)) {
            Ok(r) => r.token,
            Err(e) => {
                println!("[ERRO] token da jaula ({label}): {e}");
                continue;
            }
        };
        let (code, out) = match capture(token, &cmd, Some("Winsta0\\Default"), None, 0, *mit) {
            Ok(v) => v,
            Err(e) => {
                println!("[{label}] ERRO no spawn: {e}");
                continue;
            }
        };
        let booted = code == 0 && out.contains("jailed ok");
        if booted {
            safe.push(label);
        } else {
            breaks.push(label);
        }
        let mark = if booted == *expect_boot { "PASS" } else { "FAIL" };
        let outcome = if booted { "node INICIA" } else { "node QUEBRA" };
        println!(
            "[{mark}] {label}: {outcome} (exit 0x{code:08X}){}",
            if booted == *expect_boot { "" } else { "  <- diferente do esperado" }
        );
    }

    println!("\n======== conclusão ========");
    println!("Mitigações que o node TOLERA (aplicar na jaula):");
    for s in &safe {
        println!("  + {s}");
    }
    println!("Mitigações que QUEBRAM o node (não aplicar):");
    for b in &breaks {
        println!("  - {b}");
    }
    println!("\nAchados vs tech-spec:");
    println!("  - ACG (PROHIBIT_DYNAMIC_CODE): quebra o JIT do V8 (0x80000003). Confirmado — NÃO usar.");
    println!("  - WIN32K_DISABLE: quebra o node (0xc0000142) — severa a conexão de window station");
    println!("    que o boot precisa. O tech-spec listava win32k; medido, NÃO pode ser aplicado.");
    println!("  - CIG (BLOCK_NON_MS_BINARIES): 'passa' só porque `node -e` puro carrega apenas");
    println!("    node.exe + DLLs da MS. ALERTA: bloquearia .node addons nativos (não assinados");
    println!("    pela MS) que o agente real usa — precisa de spike com addon antes de adotar.");
}
