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
//! entra pela camada de sessão via `jailed_spawner`, que devolve o spawner que cria
//! o ConPTY e sobe o agente sob este token. O par positivo em `tests/jail_windows.rs`
//! mede a confinação (escreve no worktree, negado fora, não lê segredo `NO_READ_UP`).

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::System::Threading::*;

use super::{Sandbox, SandboxSpec};
use crate::pty::{conpty_jailed, JailedSpawner};

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
        if AllocateAndInitializeSid(
            &authority,
            1,
            SECURITY_WORLD_RID,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            &mut sid,
        ) == 0
        {
            return Err(format!("Everyone SID falhou: {}", last_error()));
        }
        Ok(sid)
    }
}

/// Extrai o LOGON SID do token (buffer precisa viver até o CreateRestrictedToken).
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

// --- Tradução SandboxSpec → jaula (rótulos IL + DACL) -----------------------

const SDDL_REVISION_1: u32 = 1;
const SE_FILE_OBJECT: i32 = 1;
const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
const LABEL_SECURITY_INFORMATION: u32 = 0x0000_0010;
const SET_ACCESS_MODE: i32 = 2;
const TRUSTEE_FORM_SID: i32 = 0;
const TRUSTEE_TYPE_UNKNOWN: i32 = 0;
const SUB_CONTAINERS_AND_OBJECTS: u32 = 0x3;
const FILE_ALL_ACCESS: u32 = 0x001F_01FF;

// Conjunto seguro de process mitigations (spike `mitigations`): o node tolera
// estes; ACG (dynamic code) e win32k QUEBRAM o node — não entram.
const MIT_DEP_ENABLE: u64 = 0x01;
const MIT_FORCE_RELOCATE_IMAGES: u64 = 0x01 << 8;
const MIT_BOTTOM_UP_ASLR: u64 = 0x01 << 16;
const MIT_STRICT_HANDLE_CHECKS: u64 = 0x01 << 20;
const MIT_HIGH_ENTROPY_ASLR: u64 = 0x01 << 24;
const MIT_EXTENSION_POINT_DISABLE: u64 = 0x01 << 32;

fn mitigation_set() -> u64 {
    MIT_DEP_ENABLE
        | MIT_FORCE_RELOCATE_IMAGES
        | MIT_BOTTOM_UP_ASLR
        | MIT_STRICT_HANDLE_CHECKS
        | MIT_HIGH_ENTROPY_ASLR
        | MIT_EXTENSION_POINT_DISABLE
}

fn wide(s: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn wide_str(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Rótulo IL Low no worktree: nasce Low e o herda para dentro (OICI). Junto com a
/// ACE do SID sintético, é o que confina a escrita do agente à sua árvore.
unsafe fn label_low(dir: &Path) -> Result<(), String> {
    set_sacl_label(dir, "S:(ML;OICI;NW;;;LW)")
}

/// Concede escrita total (FILE_ALL) ao SID sintético da sessão no worktree —
/// a outra metade da confinação de escrita (o SID só existe no restricting set
/// deste token, então nenhuma outra sessão herda a permissão).
unsafe fn grant_write(dir: &Path, sid: *mut c_void) -> Result<(), String> {
    let mut ea: EXPLICIT_ACCESS_W = std::mem::zeroed();
    ea.grfAccessPermissions = FILE_ALL_ACCESS;
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
        SE_FILE_OBJECT,
        DACL_SECURITY_INFORMATION,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        new_acl,
        std::ptr::null_mut(),
    );
    if rc != 0 {
        return Err(format!(
            "SetNamedSecurityInfoW(DACL do worktree) falhou: {rc}"
        ));
    }
    Ok(())
}

/// Aplica um SACL (rótulo de integridade) a um caminho via SDDL.
unsafe fn set_sacl_label(path: &Path, sddl: &str) -> Result<(), String> {
    let sddl_w = wide_str(sddl);
    let mut psd: *mut c_void = std::ptr::null_mut();
    if ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl_w.as_ptr(),
        SDDL_REVISION_1,
        &mut psd,
        std::ptr::null_mut(),
    ) == 0
    {
        return Err(format!("Convert SDDL `{sddl}` falhou: {}", last_error()));
    }
    let path_w = wide(path);
    let ok = SetFileSecurityW(path_w.as_ptr(), LABEL_SECURITY_INFORMATION, psd);
    let err = last_error();
    // psd é um bloco LocalAlloc; deixamos o SO liberar no fim do processo (bloco
    // minúsculo, uma vez por caminho por sessão) — evita puxar Win32_System_Memory.
    if ok == 0 {
        return Err(format!("SetFileSecurityW(label) em {path:?} falhou: {err}"));
    }
    Ok(())
}

/// Nega leitura ao agente (IL Low) marcando o caminho com o rótulo `NO_READ_UP`
/// no nível Medium. Para diretórios, rotula recursivamente CADA arquivo — só o
/// rótulo no diretório não basta: o agente abriria um segredo por caminho direto
/// (o bypass de traverse é concedido por padrão). Caminho inexistente é ignorado.
fn deny_read_tree(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let is_dir = path.is_dir();
    let sddl = if is_dir {
        "S:(ML;OICI;NRNW;;;ME)"
    } else {
        "S:(ML;;NRNW;;;ME)"
    };
    unsafe { set_sacl_label(path, sddl)? };
    if is_dir {
        let entries = std::fs::read_dir(path)
            .map_err(|e| format!("ler diretório de segredo {path:?}: {e}"))?;
        for entry in entries.flatten() {
            let child = entry.path();
            // Não segue symlink (evita escapar da árvore do segredo ao rotular).
            let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
            if is_symlink {
                unsafe { set_sacl_label(&child, "S:(ML;;NRNW;;;ME)")? };
            } else {
                deny_read_tree(&child)?;
            }
        }
    }
    Ok(())
}

/// Segredos do dono que o agente não pode ler — espelha `secret_denies` do
/// seatbelt/bwrap, filtrado para o que existe no Windows.
fn secret_paths(spec: &SandboxSpec) -> Vec<PathBuf> {
    let home = &spec.home;
    vec![
        home.join(".ssh"),
        home.join(".aws"),
        home.join(".gnupg"),
        home.join(".kube"),
        home.join(".config/gcloud"),
        home.join(".config/gh"),
        spec.tyba_data_dir.clone(),
        home.join(".git-credentials"),
        home.join(".netrc"),
        home.join(".npmrc"),
        home.join(".pypirc"),
        home.join(".docker/config.json"),
        home.join(".cargo/credentials"),
    ]
}

/// A jaula pronta de uma sessão: token restrito + o que o spawn enjaulado precisa.
/// As mutações de filesystem (rótulo Low + ACE no worktree, deny de leitura nos
/// segredos) já foram aplicadas na construção (`jailed_spawner`).
pub struct WindowsJail {
    token: SessionToken,
    mitigation: u64,
    env_extra: Vec<(String, String)>,
}

// O token e o SID são handles/ponteiros opacos do SO — seguros de mover entre
// threads (o spawn roda na thread da sessão).
unsafe impl Send for WindowsJail {}

impl Drop for WindowsJail {
    fn drop(&mut self) {
        unsafe {
            if !self.token.token.is_null() {
                CloseHandle(self.token.token);
            }
            if !self.token.synthetic_sid.is_null() {
                FreeSid(self.token.synthetic_sid);
            }
        }
    }
}

impl JailedSpawner for WindowsJail {
    fn spawn_jailed(
        &self,
        cmd: &CommandBuilder,
        size: PtySize,
    ) -> Result<(Box<dyn MasterPty + Send>, Box<dyn Child + Send + Sync>), String> {
        let command_line = conpty_jailed::encode_command_line(cmd)?;
        let env_block = conpty_jailed::encode_env_block(cmd, &self.env_extra);
        let cwd = conpty_jailed::encode_cwd(cmd);
        conpty_jailed::spawn(conpty_jailed::JailSpawnParams {
            token: self.token.token,
            command_line,
            env_block,
            cwd,
            size,
            mitigation: Some(self.mitigation),
        })
    }
}

/// Monta a jaula de uma sessão a partir do spec: token restrito, confinação de
/// escrita no worktree e deny de leitura nos segredos. Aplica as mutações de
/// filesystem antes de devolver o spawner.
fn build_jail(spec: &SandboxSpec) -> Result<WindowsJail, String> {
    let token = session_token(true)?;
    let mut result = (|| unsafe {
        label_low(&spec.writable_root)?;
        grant_write(&spec.writable_root, token.synthetic_sid)?;
        Ok::<(), String>(())
    })();
    if result.is_ok() {
        for path in secret_paths(spec) {
            if let Err(e) = deny_read_tree(&path) {
                result = Err(e);
                break;
            }
        }
    }
    if let Err(e) = result {
        // Fail-closed: qualquer parte da confinação que falhe derruba a sessão,
        // em vez de rodar o agente com a jaula pela metade.
        unsafe {
            CloseHandle(token.token);
            FreeSid(token.synthetic_sid);
        }
        return Err(e);
    }
    Ok(WindowsJail {
        token,
        mitigation: mitigation_set(),
        env_extra: vec![("TYBA_SANDBOX".to_string(), "windows".to_string())],
    })
}

pub struct WindowsSandbox;

impl WindowsSandbox {
    /// A Camada A do Windows está pronta e verificada: o par positivo de
    /// `tests/jail_windows.rs` sobe um processo real sob o token restrito num
    /// ConPTY (pelo caminho público `jailed_spawner` → `spawn_jailed`) e mede a
    /// confinação de escrita e o deny de leitura do segredo. O fail-closed continua
    /// onde importa — `jailed_spawner` recusa a sessão se qualquer parte da jaula
    /// (token, rótulo, ACE, deny) falhar; `new` só sinaliza que a jaula existe.
    pub fn new() -> Result<Self, String> {
        Ok(WindowsSandbox)
    }
}

impl Sandbox for WindowsSandbox {
    fn wrap(&self, _cmd: CommandBuilder, _spec: &SandboxSpec) -> Result<CommandBuilder, String> {
        // A jaula do Windows se aplica no spawn (token + ConPTY), não por reescrita de
        // argv — ver a nota de integração no topo do módulo. `wrap` é fail-closed.
        Err("jaula do Windows não se aplica via wrap — usar o spawn enjaulado da sessão".into())
    }

    fn jailed_spawner(&self, spec: &SandboxSpec) -> Result<Option<Box<dyn JailedSpawner>>, String> {
        Ok(Some(Box::new(build_jail(spec)?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_token_monta_com_os_tres_restricting_sids() {
        let t = session_token(true).expect("montar token da jaula");
        assert!(!t.token.is_null(), "token não pode ser nulo");
        assert!(
            !t.synthetic_sid.is_null(),
            "SID sintético não pode ser nulo"
        );
        unsafe {
            CloseHandle(t.token);
            FreeSid(t.synthetic_sid);
        }
    }

    #[test]
    fn mitigation_set_exclui_acg_e_win32k() {
        let m = mitigation_set();
        assert_eq!(m & (0x01u64 << 36), 0, "ACG (dynamic code) quebra o node");
        assert_eq!(m & (0x01u64 << 28), 0, "win32k lockdown quebra o node");
        assert_ne!(m & MIT_DEP_ENABLE, 0, "DEP faz parte do conjunto seguro");
    }

    // O par positivo da jaula ponta a ponta (spawn real sob o token + deny de
    // leitura + confinação de escrita) vive em `tests/jail_windows.rs`: ele precisa
    // do manifesto comctl6 que só os binários de teste de integração recebem, e
    // exercita a jaula pela API pública (`Sandbox::jailed_spawner`).
}
