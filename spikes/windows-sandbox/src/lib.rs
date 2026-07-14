#![cfg(windows)]

use std::ffi::c_void;
use std::time::{SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::System::Threading::*;

pub const WRITE_RESTRICTED_FLAG: u32 = 8;
pub const SE_GROUP_INTEGRITY_FLAG: u32 = 0x0000_0020;
pub const TOKEN_INTEGRITY_LEVEL_CLASS: i32 = 25;
pub const MANDATORY_LOW_RID: u32 = 0x0000_1000;
pub const HANDLE_LIST_ATTRIBUTE: usize = 0x0002_0002;

pub type Handle = *mut c_void;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn last_error() -> u32 {
    unsafe { GetLastError() }
}

pub struct Restricted {
    pub token: Handle,
    pub synthetic_sid: *mut c_void,
}

pub fn build_restricted_low_il() -> Result<Restricted, String> {
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

        let mut restrict = [SID_AND_ATTRIBUTES {
            Sid: synthetic,
            Attributes: 0,
        }];

        let mut restricted: Handle = std::ptr::null_mut();
        let ok = CreateRestrictedToken(
            source,
            WRITE_RESTRICTED_FLAG,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            restrict.len() as u32,
            restrict.as_mut_ptr(),
            &mut restricted,
        );
        CloseHandle(source);
        if ok == 0 {
            return Err(format!("CreateRestrictedToken falhou: {}", last_error()));
        }

        set_low_integrity(restricted)?;

        Ok(Restricted {
            token: restricted,
            synthetic_sid: synthetic,
        })
    }
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
