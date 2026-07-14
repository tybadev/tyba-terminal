#![cfg(windows)]

use std::ffi::c_void;
use std::io::Read;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::*;

pub const STARTF_USESTDHANDLES_FLAG: u32 = 0x0000_0100;
pub const PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY: usize = 0x0002_0007;

pub const WRITE_RESTRICTED_FLAG: u32 = 8;
pub const DISABLE_MAX_PRIVILEGE_FLAG: u32 = 1;
pub const LUA_TOKEN_FLAG: u32 = 4;
pub const SE_GROUP_INTEGRITY_FLAG: u32 = 0x0000_0020;
pub const TOKEN_INTEGRITY_LEVEL_CLASS: i32 = 25;
pub const MANDATORY_LOW_RID: u32 = 0x0000_1000;
pub const HANDLE_LIST_ATTRIBUTE: usize = 0x0002_0002;

pub type Handle = *mut c_void;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn node_exe() -> Option<String> {
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

pub fn spawn_handles(
    token: Handle,
    command_line: &str,
    creation_flags: u32,
) -> Result<(Handle, Handle), String> {
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let mut cmd = wide(command_line);
        let flags = creation_flags | CREATE_UNICODE_ENVIRONMENT;
        let ok = CreateProcessAsUserW(
            token,
            std::ptr::null(),
            cmd.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            flags,
            std::ptr::null(),
            std::ptr::null(),
            &si,
            &mut pi,
        );
        if ok == 0 {
            return Err(format!("CreateProcessAsUserW falhou: {}", last_error()));
        }
        Ok((pi.hProcess, pi.hThread))
    }
}

pub fn last_error() -> u32 {
    unsafe { GetLastError() }
}

pub struct Restricted {
    pub token: Handle,
    pub synthetic_sid: *mut c_void,
}

pub const TOKEN_LOGON_SID_CLASS: i32 = 28;
pub const SECURITY_WORLD_RID: u32 = 0;

pub struct TokenSpec {
    pub low_il: bool,
    pub add_logon_sid: bool,
    pub add_everyone: bool,
    pub flags: u32,
}

impl Default for TokenSpec {
    fn default() -> Self {
        TokenSpec {
            low_il: true,
            add_logon_sid: false,
            add_everyone: false,
            flags: WRITE_RESTRICTED_FLAG,
        }
    }
}

pub fn build_restricted_low_il() -> Result<Restricted, String> {
    build_restricted(&TokenSpec::default())
}

pub fn jail_spec(low_il: bool) -> TokenSpec {
    TokenSpec {
        low_il,
        add_logon_sid: true,
        add_everyone: true,
        flags: WRITE_RESTRICTED_FLAG | DISABLE_MAX_PRIVILEGE_FLAG | LUA_TOKEN_FLAG,
    }
}

pub fn build_restricted(spec: &TokenSpec) -> Result<Restricted, String> {
    unsafe {
        let mut source: Handle = std::ptr::null_mut();
        let want = TOKEN_DUPLICATE
            | TOKEN_QUERY
            | TOKEN_ASSIGN_PRIMARY
            | TOKEN_ADJUST_DEFAULT
            | TOKEN_ADJUST_SESSIONID;
        if OpenProcessToken(GetCurrentProcess(), want, &mut source) == 0 {
            return Err(format!("OpenProcessToken falhou: {}", last_error()));
        }

        let synthetic = make_synthetic_sid()?;

        let mut restrict = vec![SID_AND_ATTRIBUTES {
            Sid: synthetic,
            Attributes: 0,
        }];

        let _logon_buf = if spec.add_logon_sid {
            let (buf, sid) = match logon_sid(source) {
                Ok(v) => v,
                Err(e) => {
                    CloseHandle(source);
                    return Err(e);
                }
            };
            restrict.push(SID_AND_ATTRIBUTES {
                Sid: sid,
                Attributes: 0,
            });
            Some(buf)
        } else {
            None
        };

        if spec.add_everyone {
            let world = match everyone_sid() {
                Ok(s) => s,
                Err(e) => {
                    CloseHandle(source);
                    return Err(e);
                }
            };
            restrict.push(SID_AND_ATTRIBUTES {
                Sid: world,
                Attributes: 0,
            });
        }

        let mut restricted: Handle = std::ptr::null_mut();
        let ok = CreateRestrictedToken(
            source,
            spec.flags,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            restrict.len() as u32,
            restrict.as_mut_ptr(),
            &mut restricted,
        );
        CloseHandle(source);
        drop(_logon_buf);
        if ok == 0 {
            return Err(format!("CreateRestrictedToken falhou: {}", last_error()));
        }

        if spec.low_il {
            set_low_integrity(restricted)?;
        }

        Ok(Restricted {
            token: restricted,
            synthetic_sid: synthetic,
        })
    }
}

fn everyone_sid() -> Result<*mut c_void, String> {
    unsafe {
        let authority = SID_IDENTIFIER_AUTHORITY {
            Value: [0, 0, 0, 0, 0, 1],
        };
        let mut sid: *mut c_void = std::ptr::null_mut();
        if AllocateAndInitializeSid(&authority, 1, SECURITY_WORLD_RID, 0, 0, 0, 0, 0, 0, 0, &mut sid)
            == 0
        {
            return Err(format!("Everyone SID falhou: {}", last_error()));
        }
        Ok(sid)
    }
}

pub fn describe_restricting_sids(token: Handle) -> String {
    unsafe {
        let mut size: u32 = 0;
        GetTokenInformation(token, 11, std::ptr::null_mut(), 0, &mut size);
        if size == 0 {
            return format!("(TokenRestrictedSids tamanho 0: {})", last_error());
        }
        let mut buf = vec![0u8; size as usize];
        if GetTokenInformation(token, 11, buf.as_mut_ptr() as *mut c_void, size, &mut size) == 0 {
            return format!("(TokenRestrictedSids falhou: {})", last_error());
        }
        let groups = buf.as_ptr() as *const TOKEN_GROUPS;
        let count = (*groups).GroupCount;
        let base = (*groups).Groups.as_ptr();
        let mut out = format!("{count} restricting SID(s): ");
        for i in 0..count {
            let sid = base.add(i as usize).read().Sid;
            let mut str_ptr: *mut u16 = std::ptr::null_mut();
            if ConvertSidToStringSidW(sid, &mut str_ptr) != 0 {
                let mut len = 0;
                while *str_ptr.add(len) != 0 {
                    len += 1;
                }
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(str_ptr, len));
                out.push_str(&s);
            } else {
                out.push_str("<?>");
            }
            if i + 1 < count {
                out.push_str(", ");
            }
        }
        out
    }
}

unsafe fn logon_sid(token: Handle) -> Result<(Vec<u8>, *mut c_void), String> {
    let mut size: u32 = 0;
    GetTokenInformation(
        token,
        TOKEN_LOGON_SID_CLASS,
        std::ptr::null_mut(),
        0,
        &mut size,
    );
    if size == 0 {
        return Err(format!("GetTokenInformation(TokenLogonSid) tamanho: {}", last_error()));
    }
    let mut buf = vec![0u8; size as usize];
    if GetTokenInformation(
        token,
        TOKEN_LOGON_SID_CLASS,
        buf.as_mut_ptr() as *mut c_void,
        size,
        &mut size,
    ) == 0
    {
        return Err(format!("GetTokenInformation(TokenLogonSid) falhou: {}", last_error()));
    }
    let groups = buf.as_ptr() as *const TOKEN_GROUPS;
    if (*groups).GroupCount < 1 {
        return Err("token sem logon SID".to_string());
    }
    let sid = (*groups).Groups.as_ptr().read().Sid;
    if sid.is_null() {
        return Err("logon SID nulo".to_string());
    }
    Ok((buf, sid))
}

fn make_synthetic_sid() -> Result<*mut c_void, String> {
    unsafe {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(1);
        let rid = 0x8000_0000u32 | (std::process::id() ^ nanos);
        let authority = SID_IDENTIFIER_AUTHORITY {
            Value: [0, 0, 0, 0, 0, 42],
        };
        let mut sid: *mut c_void = std::ptr::null_mut();
        let ok = AllocateAndInitializeSid(&authority, 1, rid, 0, 0, 0, 0, 0, 0, 0, &mut sid);
        if ok == 0 {
            return Err(format!("AllocateAndInitializeSid falhou: {}", last_error()));
        }
        Ok(sid)
    }
}

fn set_low_integrity(token: Handle) -> Result<(), String> {
    unsafe {
        let authority = SID_IDENTIFIER_AUTHORITY {
            Value: [0, 0, 0, 0, 0, 16],
        };
        let mut low: *mut c_void = std::ptr::null_mut();
        if AllocateAndInitializeSid(
            &authority,
            1,
            MANDATORY_LOW_RID,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            &mut low,
        ) == 0
        {
            return Err(format!("low IL SID falhou: {}", last_error()));
        }
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: low,
                Attributes: SE_GROUP_INTEGRITY_FLAG,
            },
        };
        let ok = SetTokenInformation(
            token,
            TOKEN_INTEGRITY_LEVEL_CLASS,
            &label as *const _ as *const c_void,
            std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32,
        );
        let err = last_error();
        FreeSid(low);
        if ok == 0 {
            return Err(format!(
                "SetTokenInformation(IntegrityLevel Low) falhou: {}",
                err
            ));
        }
        Ok(())
    }
}

pub fn git_exe() -> Option<String> {
    let mut roots: Vec<String> = Vec::new();
    if let Ok(path) = std::env::var("PATH") {
        roots.extend(path.split(';').map(|s| s.to_string()));
    }
    roots.push(r"C:\Program Files\Git\cmd".to_string());
    roots.push(r"C:\Program Files\Git\bin".to_string());
    for dir in roots {
        if dir.trim().is_empty() {
            continue;
        }
        let candidate = std::path::Path::new(dir.trim()).join("git.exe");
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

pub fn spawn_inherit_async(
    token: Handle,
    command_line: &str,
    inherit: &[Handle],
) -> Result<(Handle, Handle), String> {
    unsafe {
        let mut si_ex: STARTUPINFOEXW = std::mem::zeroed();
        si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;

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
        let mut handles: Vec<Handle> = inherit.to_vec();
        if !handles.is_empty()
            && UpdateProcThreadAttribute(
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
        let created = CreateProcessAsUserW(
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
        );
        DeleteProcThreadAttributeList(attr_list);
        if created == 0 {
            return Err(format!("CreateProcessAsUserW falhou: {}", last_error()));
        }
        Ok((pi.hProcess, pi.hThread))
    }
}

pub fn spawn_with_token(
    token: Handle,
    command_line: &str,
    inherit: &[Handle],
) -> Result<u32, String> {
    unsafe {
        let mut si_ex: STARTUPINFOEXW = std::mem::zeroed();
        si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;

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

        let mut handles: Vec<Handle> = inherit.to_vec();
        if !handles.is_empty() {
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
                return Err(format!(
                    "UpdateProcThreadAttribute falhou: {}",
                    last_error()
                ));
            }
            si_ex.lpAttributeList = attr_list;
        }

        let mut cmd = wide(command_line);
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let flags = EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT;

        let created = CreateProcessAsUserW(
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
        );

        if created == 0 {
            let as_user_err = last_error();
            let created2 = CreateProcessWithTokenW(
                token,
                0,
                std::ptr::null(),
                cmd.as_mut_ptr(),
                flags,
                std::ptr::null(),
                std::ptr::null(),
                &si_ex.StartupInfo,
                &mut pi,
            );
            if created2 == 0 {
                DeleteProcThreadAttributeList(attr_list);
                return Err(format!(
                    "CreateProcessAsUserW falhou ({}) e CreateProcessWithTokenW falhou ({})",
                    as_user_err,
                    last_error()
                ));
            }
        }

        WaitForSingleObject(pi.hProcess, INFINITE);
        let mut code: u32 = 0;
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
        DeleteProcThreadAttributeList(attr_list);
        Ok(code)
    }
}

pub fn make_env_block(pairs: &[(String, String)]) -> Vec<u16> {
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in pairs {
        block.extend(format!("{k}={v}").encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

pub fn capture(
    token: Handle,
    command_line: &str,
    desktop: Option<&str>,
    env_block: Option<&[u16]>,
    extra_flags: u32,
    mitigation: Option<u64>,
) -> Result<(u32, String), String> {
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
        si_ex.StartupInfo.dwFlags = STARTF_USESTDHANDLES_FLAG;
        si_ex.StartupInfo.hStdInput = std::ptr::null_mut();
        si_ex.StartupInfo.hStdOutput = write_h;
        si_ex.StartupInfo.hStdError = write_h;
        if let Some(d) = desktop_w.as_mut() {
            si_ex.StartupInfo.lpDesktop = d.as_mut_ptr();
        }

        let attr_count = 1 + mitigation.is_some() as u32;
        let mut attr_size: usize = 0;
        InitializeProcThreadAttributeList(std::ptr::null_mut(), attr_count, 0, &mut attr_size);
        let mut attr_buf = vec![0u8; attr_size];
        let attr_list = attr_buf.as_mut_ptr() as *mut _;
        if InitializeProcThreadAttributeList(attr_list, attr_count, 0, &mut attr_size) == 0 {
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
        let mut policy: u64 = mitigation.unwrap_or(0);
        if mitigation.is_some() {
            if UpdateProcThreadAttribute(
                attr_list,
                0,
                PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
                &mut policy as *mut u64 as *const c_void,
                std::mem::size_of::<u64>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ) == 0
            {
                DeleteProcThreadAttributeList(attr_list);
                return Err(format!(
                    "UpdateProcThreadAttribute(mitigation) falhou: {}",
                    last_error()
                ));
            }
        }
        si_ex.lpAttributeList = attr_list;

        let mut cmd = wide(command_line);
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let mut flags = EXTENDED_STARTUPINFO_PRESENT | extra_flags;
        let env_ptr = match env_block {
            Some(b) => {
                flags |= CREATE_UNICODE_ENVIRONMENT;
                b.as_ptr() as *const c_void
            }
            None => {
                flags |= CREATE_UNICODE_ENVIRONMENT;
                std::ptr::null()
            }
        };

        let created = if token.is_null() {
            CreateProcessW(
                std::ptr::null(),
                cmd.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                flags,
                env_ptr,
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
                env_ptr,
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

        Ok((code, out))
    }
}

pub fn banner(title: &str) {
    println!("\n========================================================");
    println!("  {title}");
    println!("========================================================");
}

pub fn verdict(label: &str, ok: bool, meaning: &str) {
    let mark = if ok { "PASS" } else { "FAIL" };
    println!("[{mark}] {label}");
    println!("       -> {meaning}");
}
