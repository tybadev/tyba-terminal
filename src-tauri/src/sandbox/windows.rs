//! Jaula de agente no Windows (Camada A) — token restrito + IL Low.
//!
//! Mecanismo medido no spike `spikes/windows-sandbox/` (ver ADR
//! `2026-07-14-windows-token-restrito-nao-appcontainer` e o tech-spec). A receita
//! que faz o agente (`node.exe`) iniciar é `session_token`: `CreateRestrictedToken`
//! com `WRITE_RESTRICTED | DISABLE_MAX_PRIVILEGE | LUA_TOKEN` e restricting SIDs
//! `{SID sintético por sessão, LOGON SID, Everyone}`, mais Integrity Level Low.
//! O SID sintético é a chave da confinação de escrita (ACE só no worktree).
//!
//! **Aplicação (decisão de integração, Opção B):** a jaula do Windows NÃO é um
//! prefixo de argv como Seatbelt/bwrap — ela se aplica no spawn (`CreateProcessAsUserW`
//! com o token + atributo pseudoconsole). Logo `wrap` é fail-closed aqui; a Camada A
//! entra pela camada de sessão, que cria o ConPTY e spawna o agente sob este token.
//! Enquanto esse caminho não existe, `platform_sandbox()` segue recusando a sessão.

use std::ffi::c_void;
use std::time::{SystemTime, UNIX_EPOCH};

use portable_pty::CommandBuilder;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::System::Threading::*;

use super::{Sandbox, SandboxSpec};

const WRITE_RESTRICTED_FLAG: u32 = 8;
const DISABLE_MAX_PRIVILEGE_FLAG: u32 = 1;
const LUA_TOKEN_FLAG: u32 = 4;
const SE_GROUP_INTEGRITY_FLAG: u32 = 0x0000_0020;
const TOKEN_INTEGRITY_LEVEL_CLASS: i32 = 25;
const TOKEN_LOGON_SID_CLASS: i32 = 28;
const MANDATORY_LOW_RID: u32 = 0x0000_1000;
const SECURITY_WORLD_RID: u32 = 0;

type Handle = *mut c_void;

/// Token restrito da sessão + o SID sintético que a confinação de escrita usa.
pub struct SessionToken {
    pub token: Handle,
    pub synthetic_sid: *mut c_void,
}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

/// Monta o token da jaula (`jail_spec` do spike): WRITE_RESTRICTED + LUA_TOKEN,
/// restricting SIDs {sintético, logon, Everyone}, e IL Low se `low_il`.
pub fn session_token(low_il: bool) -> Result<SessionToken, String> {
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

        let synthetic = match make_synthetic_sid() {
            Ok(s) => s,
            Err(e) => {
                CloseHandle(source);
                return Err(e);
            }
        };

        let mut restrict = vec![SID_AND_ATTRIBUTES {
            Sid: synthetic,
            Attributes: 0,
        }];

        let (logon_buf, logon) = match logon_sid(source) {
            Ok(v) => v,
            Err(e) => {
                CloseHandle(source);
                return Err(e);
            }
        };
        restrict.push(SID_AND_ATTRIBUTES {
            Sid: logon,
            Attributes: 0,
        });

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

        let flags = WRITE_RESTRICTED_FLAG | DISABLE_MAX_PRIVILEGE_FLAG | LUA_TOKEN_FLAG;
        let mut restricted: Handle = std::ptr::null_mut();
        let ok = CreateRestrictedToken(
            source,
            flags,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            restrict.len() as u32,
            restrict.as_mut_ptr(),
            &mut restricted,
        );
        CloseHandle(source);
        drop(logon_buf);
        if ok == 0 {
            return Err(format!("CreateRestrictedToken falhou: {}", last_error()));
        }

        if low_il {
            if let Err(e) = set_low_integrity(restricted) {
                CloseHandle(restricted);
                return Err(e);
            }
        }

        Ok(SessionToken {
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
        if AllocateAndInitializeSid(&authority, 1, rid, 0, 0, 0, 0, 0, 0, 0, &mut sid) == 0 {
            return Err(format!("AllocateAndInitializeSid falhou: {}", last_error()));
        }
        Ok(sid)
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

/// Extrai o LOGON SID do token (buffer precisa viver até o CreateRestrictedToken).
unsafe fn logon_sid(token: Handle) -> Result<(Vec<u8>, *mut c_void), String> {
    let mut size: u32 = 0;
    GetTokenInformation(token, TOKEN_LOGON_SID_CLASS, std::ptr::null_mut(), 0, &mut size);
    if size == 0 {
        return Err(format!("TokenLogonSid tamanho: {}", last_error()));
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
        return Err(format!("TokenLogonSid falhou: {}", last_error()));
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
            return Err(format!("SetTokenInformation(IL Low) falhou: {err}"));
        }
        Ok(())
    }
}

pub struct WindowsSandbox;

impl WindowsSandbox {
    /// Fail-closed: enquanto o spawn com ConPTY sob o token não existe, a jaula do
    /// Windows não pode ser aplicada — `new` recusa para que `platform_sandbox`
    /// mantenha a sessão de agente negada, em vez de rodar o agente sem jaula.
    pub fn new() -> Result<Self, String> {
        Err("jaula do Windows (Camada A) ainda não aplica no spawn — sessão de agente recusada (fail-closed)".into())
    }
}

impl Sandbox for WindowsSandbox {
    fn wrap(&self, _cmd: CommandBuilder, _spec: &SandboxSpec) -> Result<CommandBuilder, String> {
        // A jaula do Windows se aplica no spawn (token + ConPTY), não por reescrita de
        // argv — ver a nota de integração no topo do módulo. `wrap` é fail-closed.
        Err("jaula do Windows não se aplica via wrap — usar o spawn enjaulado da sessão".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_token_monta_com_os_tres_restricting_sids() {
        let t = session_token(true).expect("montar token da jaula");
        assert!(!t.token.is_null(), "token não pode ser nulo");
        assert!(!t.synthetic_sid.is_null(), "SID sintético não pode ser nulo");
        unsafe {
            CloseHandle(t.token);
        }
    }
}
