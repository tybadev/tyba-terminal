#![cfg(windows)]

use std::ffi::c_void;
use std::io::Read;
use std::net::TcpListener;
use std::process::Command;

use probe::*;
use windows_sys::core::GUID;
use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::*;
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;

const SPIKE_USER: &str = "tyba-agent-wfp";

const FWP_ACTION_BLOCK_V: u32 = 0x1001;
const FWP_MATCH_EQUAL_V: i32 = 0;
const FWP_SECURITY_DESCRIPTOR_TYPE_V: i32 = 14;
const FWP_V4_ADDR_MASK_V: i32 = 0x100;
const FWP_EMPTY_V: i32 = 0;
const FWPM_SESSION_FLAG_DYNAMIC_V: u32 = 0x0000_0001;
const RPC_C_AUTHN_WINNT_V: u32 = 10;
const SDDL_REV1: u32 = 1;

const LAYER_ALE_AUTH_CONNECT_V4: GUID = GUID {
    data1: 0xc38d_57d1,
    data2: 0x05a7,
    data3: 0x4c33,
    data4: [0x90, 0x4f, 0x7f, 0xbc, 0xee, 0xe6, 0x0e, 0x82],
};
const COND_ALE_USER_ID: GUID = GUID {
    data1: 0xaf04_3a0a,
    data2: 0xb34d,
    data3: 0x4f86,
    data4: [0x97, 0x9c, 0xc9, 0x03, 0x71, 0xaf, 0x6e, 0x66],
};
const COND_IP_REMOTE_ADDRESS: GUID = GUID {
    data1: 0xb235_ae9a,
    data2: 0x1d64,
    data3: 0x49b8,
    data4: [0xa4, 0x4c, 0x5f, 0xf3, 0xd9, 0x09, 0x50, 0x45],
};
const OUR_SUBLAYER: GUID = GUID {
    data1: 0x7ba1_c0de,
    data2: 0x0001,
    data3: 0x0002,
    data4: [0x00, 0x03, 0x00, 0x04, 0x00, 0x05, 0x00, 0x06],
};

// (rótulo, addr host-order, mask host-order)
const RANGES: &[(&str, u32, u32)] = &[
    ("loopback 127/8", 0x7F00_0000, 0xFF00_0000),
    ("RFC1918 10/8", 0x0A00_0000, 0xFF00_0000),
    ("RFC1918 172.16/12", 0xAC10_0000, 0xFFF0_0000),
    ("RFC1918 192.168/16", 0xC0A8_0000, 0xFFFF_0000),
];

fn main() {
    banner("SONDA wfp — Camada B: rede por WFP escopada ao SID (loopback + RFC1918 negados)");
    println!("Filtros WFP na camada ALE_AUTH_CONNECT_V4, condicionados ao SID do usuário");
    println!("dedicado (ALE_USER_ID) + endereço remoto. Sessão DINÂMICA: os filtros somem quando");
    println!("o processo morre — sem órfãos. A rede do DONO não é tocada (escopo por SID).\n");

    if !is_elevated() {
        println!("[BLOQUEADO] precisa de ELEVAÇÃO (UAC). Abra um PowerShell admin e rode:");
        println!("    cargo run --manifest-path spikes\\windows-sandbox\\Cargo.toml --bin wfp");
        return;
    }

    run();
}

fn run() {
    // 1. usuário dedicado + SID (a rede vai ser escopada a ele)
    let pid = std::process::id();
    let pw = format!("Tb$w{}!Aa9", pid % 10000);
    let create = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                r#"$sp=ConvertTo-SecureString '{pw}' -AsPlainText -Force
New-LocalUser -Name '{SPIKE_USER}' -Password $sp -AccountNeverExpires -PasswordNeverExpires -ErrorAction SilentlyContinue | Out-Null
Add-LocalGroupMember -SID 'S-1-5-32-545' -Member '{SPIKE_USER}' -ErrorAction SilentlyContinue | Out-Null
Write-Output ((Get-LocalUser -Name '{SPIKE_USER}' -ErrorAction SilentlyContinue).SID.Value)"#
            ),
        ])
        .output();
    let sid = create
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if !sid.starts_with("S-1-5-21") {
        println!("[ERRO] não criou/resolveu o usuário dedicado (SID: {sid:?})");
        return;
    }

    // 2. filtros WFP dinâmicos, escopados ao SID
    let engine = match add_wfp_filters(&sid) {
        Ok(e) => e,
        Err(e) => {
            println!("[ERRO] WFP: {e}");
            cleanup_user();
            return;
        }
    };

    // 3. listener de loopback do DONO + controle positivo (o dono conecta)
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            drop(stream);
        }
    });
    let owner_loopback_ok = std::net::TcpStream::connect(("127.0.0.1", port)).is_ok();

    // 4. teste como o usuário dedicado (deve ser barrado no loopback, livre no externo)
    let base = std::env::temp_dir().join(format!("tyba-wfp-{pid}"));
    let _ = std::fs::create_dir_all(&base);
    let shared = std::env::temp_dir().join(format!("tyba-wfp-out-{pid}"));
    let shared_dir = format!("C:\\tyba-wfp-sh-{pid}");
    let _ = std::fs::create_dir_all(&shared_dir);
    let _ = Command::new("icacls")
        .args([&shared_dir, "/grant", "*S-1-1-0:(OI)(CI)M"])
        .output();
    let out_file = format!("{shared_dir}\\res.txt");

    let test = format!(
        r#"$sp=ConvertTo-SecureString '{pw}' -AsPlainText -Force
$cred=New-Object System.Management.Automation.PSCredential('{SPIKE_USER}',$sp)
$script='function T($h,$p){{try{{$c=New-Object Net.Sockets.TcpClient;$a=$c.BeginConnect($h,$p,$null,$null);if($a.AsyncWaitHandle.WaitOne(3000) -and $c.Connected){{$c.Close();return \"ok\"}}else{{$c.Close();return \"blocked\"}}}}catch{{return \"blocked\"}}}}; \"LOOP=\"+(T \"127.0.0.1\" {port})+\";EXT=\"+(T \"1.1.1.1\" 443) | Set-Content -Path \"{out_file}\"'
Start-Process -FilePath 'powershell.exe' -ArgumentList '-NoProfile','-Command',$script -Credential $cred -WorkingDirectory 'C:\' -Wait"#
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &test])
        .output();
    std::thread::sleep(std::time::Duration::from_millis(400));
    let mut res = String::new();
    let _ = std::fs::File::open(&out_file).and_then(|mut f| f.read_to_string(&mut res));
    let res = res.trim().to_string();
    let loop_blocked = res.contains("LOOP=blocked");
    let ext_ok = res.contains("EXT=ok");

    // 5. fecha a engine → sessão dinâmica remove TODOS os filtros (uninstaller sem órfão)
    unsafe {
        FwpmEngineClose0(engine);
    }
    let leftover = count_our_filters();

    // 6. limpeza de usuário + dirs
    cleanup_user();
    let _ = std::fs::remove_dir_all(&base);
    let _ = std::fs::remove_dir_all(&shared_dir);
    let _ = std::fs::remove_file(&shared);

    println!();
    verdict(
        "CONTROLE: o DONO conecta no listener de loopback",
        owner_loopback_ok,
        "prova que o listener funciona — o bloqueio abaixo é o WFP, não porta fechada",
    );
    verdict(
        "o agente (usuário dedicado) é BLOQUEADO no loopback pelo WFP",
        owner_loopback_ok && loop_blocked,
        "corta o agente de falar com Docker/ollama/TYBA em 127.0.0.1 — o que a Camada A não faz",
    );
    verdict(
        "CONTROLE: o agente AINDA alcança a internet externa (443)",
        ext_ok,
        "o filtro é escopado a loopback+RFC1918: o agente é um cliente de API, precisa de 443 externo",
    );
    verdict(
        "uninstaller: sessão dinâmica removeu os filtros ao fechar a engine (sem órfão)",
        leftover == 0,
        "fechar a engine apaga tudo — mesmo em crash, os filtros somem com o processo. O Codex deixa órfãos; aqui não",
    );

    println!("\nSID escopado: {sid}");
    println!("resultado do teste (como usuário dedicado): {res:?}");
    println!("filtros nossos restantes após fechar a engine: {leftover}");
}

fn add_wfp_filters(sid: &str) -> Result<*mut c_void, String> {
    unsafe {
        // SD que concede FWP_ACTRL_MATCH_FILTER (CC) ao SID — a condição ALE_USER_ID casa por ele.
        let sddl = wide(&format!("D:(A;;CC;;;{sid})"));
        let mut psd: *mut c_void = std::ptr::null_mut();
        let mut sd_size: u32 = 0;
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REV1,
            &mut psd,
            &mut sd_size,
        ) == 0
        {
            return Err(format!("Convert SDDL(SID): {}", last_error()));
        }

        let mut session: FWPM_SESSION0 = std::mem::zeroed();
        session.flags = FWPM_SESSION_FLAG_DYNAMIC_V;
        let mut engine: *mut c_void = std::ptr::null_mut();
        let rc = FwpmEngineOpen0(
            std::ptr::null(),
            RPC_C_AUTHN_WINNT_V,
            std::ptr::null(),
            &session,
            &mut engine,
        );
        if rc != 0 {
            return Err(format!("FwpmEngineOpen0: 0x{rc:08X}"));
        }

        let mut sublayer: FWPM_SUBLAYER0 = std::mem::zeroed();
        sublayer.subLayerKey = OUR_SUBLAYER;
        let mut sub_name = wide("tyba-wfp-spike");
        sublayer.displayData.name = sub_name.as_mut_ptr();
        sublayer.weight = 0x0100;
        let rc = FwpmSubLayerAdd0(engine, &sublayer, std::ptr::null_mut());
        if rc != 0 {
            FwpmEngineClose0(engine);
            return Err(format!("FwpmSubLayerAdd0: 0x{rc:08X}"));
        }

        let mut blob = FWP_BYTE_BLOB {
            size: sd_size,
            data: psd as *mut u8,
        };

        for (label, addr, mask) in RANGES {
            let mut v4 = FWP_V4_ADDR_AND_MASK {
                addr: *addr,
                mask: *mask,
            };

            let mut conds: [FWPM_FILTER_CONDITION0; 2] = std::mem::zeroed();
            conds[0].fieldKey = COND_ALE_USER_ID;
            conds[0].matchType = FWP_MATCH_EQUAL_V;
            conds[0].conditionValue.r#type = FWP_SECURITY_DESCRIPTOR_TYPE_V;
            conds[0].conditionValue.Anonymous.sd = &mut blob;
            conds[1].fieldKey = COND_IP_REMOTE_ADDRESS;
            conds[1].matchType = FWP_MATCH_EQUAL_V;
            conds[1].conditionValue.r#type = FWP_V4_ADDR_MASK_V;
            conds[1].conditionValue.Anonymous.v4AddrMask = &mut v4;

            let mut filter: FWPM_FILTER0 = std::mem::zeroed();
            let mut name = wide(&format!("tyba-block {label}"));
            filter.displayData.name = name.as_mut_ptr();
            filter.layerKey = LAYER_ALE_AUTH_CONNECT_V4;
            filter.subLayerKey = OUR_SUBLAYER;
            filter.weight.r#type = FWP_EMPTY_V;
            filter.numFilterConditions = 2;
            filter.filterCondition = conds.as_mut_ptr();
            filter.action.r#type = FWP_ACTION_BLOCK_V;

            let mut id: u64 = 0;
            let rc = FwpmFilterAdd0(engine, &filter, std::ptr::null_mut(), &mut id);
            if rc != 0 {
                FwpmEngineClose0(engine);
                return Err(format!("FwpmFilterAdd0({label}): 0x{rc:08X}"));
            }
        }

        Ok(engine)
    }
}

fn count_our_filters() -> u32 {
    // após fechar a engine dinâmica, abre uma nova e conta filtros na nossa sublayer
    unsafe {
        let mut engine: *mut c_void = std::ptr::null_mut();
        if FwpmEngineOpen0(
            std::ptr::null(),
            RPC_C_AUTHN_WINNT_V,
            std::ptr::null(),
            std::ptr::null(),
            &mut engine,
        ) != 0
        {
            return 0;
        }
        let mut enum_handle: *mut c_void = std::ptr::null_mut();
        let mut count: u32 = 0;
        if FwpmFilterCreateEnumHandle0(engine, std::ptr::null(), &mut enum_handle) == 0 {
            let mut entries: *mut *mut FWPM_FILTER0 = std::ptr::null_mut();
            let mut num: u32 = 0;
            if FwpmFilterEnum0(engine, enum_handle, 1024, &mut entries, &mut num) == 0 {
                for i in 0..num as isize {
                    let f = *entries.offset(i);
                    if guid_eq(&(*f).subLayerKey, &OUR_SUBLAYER) {
                        count += 1;
                    }
                }
            }
            FwpmFilterDestroyEnumHandle0(engine, enum_handle);
        }
        FwpmEngineClose0(engine);
        count
    }
}

fn guid_eq(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

fn cleanup_user() {
    let _ = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("Remove-LocalUser -Name '{SPIKE_USER}' -ErrorAction SilentlyContinue"),
        ])
        .output();
}
