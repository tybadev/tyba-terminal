//! Spawn enjaulado com ConPTY no Windows (Camada A, decisão de integração Opção B).
//!
//! A jaula do Windows não é prefixo de argv (Seatbelt/bwrap); ela se aplica no
//! spawn. Aqui o core cria o pseudoconsole (ConPTY) e sobe o agente direto sob o
//! token restrito (`sandbox::windows::session_token`) com o atributo
//! `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` — medido no spike `spikes/windows-sandbox`
//! (sonda `conpty`: o agente enjaulado enxerga `isTTY=true`). O processo é
//! atribuído a um Job Object `KILL_ON_JOB_CLOSE` para dar a paridade com o
//! `killpg` (princípio #9): o TYBA sumindo leva a árvore do agente junto.
//!
//! As implementações de `MasterPty`/`Child` espelham as do `portable-pty` (mesmas
//! primitivas de ConPTY), então o leitor/emissor/resize/kill do `PtyPool` seguem
//! sem tocar — a única diferença é o token e o Job Object.

use std::ffi::c_void;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Error as IoError, Read, Result as IoResult, Write};
use std::os::windows::ffi::OsStrExt;
#[cfg(test)]
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::Path;
use std::sync::{Arc, Mutex};

use portable_pty::{Child, ChildKiller, CommandBuilder, ExitStatus, MasterPty, PtySize};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::*;

type Handle = *mut c_void;

const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;
/// `PSEUDOCONSOLE_RESIZE_QUIRK` — não impede o output (ao contrário de
/// `INHERIT_CURSOR 0x1`, que trava o conhost esperando um DSR de cursor que nunca
/// respondemos). Só evita o cursor pular no resize; combinado com `windowsPty` no
/// xterm, corta o reflow duplo que embaralha o terminal. NUNCA usar `0x1` aqui.
const PSEUDOCONSOLE_RESIZE_QUIRK: u32 = 0x0000_0002;
const PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY: usize = 0x0002_0007;
const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
const CREATE_SUSPENDED_FLAG: u32 = 0x0000_0004;
const STILL_ACTIVE_CODE: u32 = 259;
const INFINITE_MS: u32 = 0xFFFF_FFFF;

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

/// Tudo que o spawn enjaulado precisa, já re-encodado para a jaula. A cmdline e o
/// bloco de env vêm reconstruídos na mão (os getters `cmdline`/`environment_block`
/// do `CommandBuilder` são `pub(crate)` — ver `encode_*` abaixo).
pub struct JailSpawnParams {
    /// Token restrito da sessão (`sandbox::windows::session_token`). Emprestado:
    /// o processo recebe a própria cópia; o chamador segue dono do handle.
    pub token: Handle,
    /// Linha de comando wide, terminada em NUL (formato do `CreateProcessW`).
    pub command_line: Vec<u16>,
    /// Bloco de ambiente wide (pares `K=V\0`, terminado em `\0\0`).
    pub env_block: Vec<u16>,
    /// Diretório de trabalho wide terminado em NUL, ou `None`.
    pub cwd: Option<Vec<u16>>,
    pub size: PtySize,
    /// Conjunto de process mitigations que o node tolera (spike `mitigations`),
    /// ou `None`.
    pub mitigation: Option<u64>,
}

/// Sobe o agente enjaulado num ConPTY novo. Devolve o master/child que o `PtyPool`
/// consome como qualquer PTY.
pub fn spawn(params: JailSpawnParams) -> Result<super::JailedPtyPair, String> {
    unsafe { spawn_inner(params) }
}

unsafe fn spawn_inner(mut params: JailSpawnParams) -> Result<super::JailedPtyPair, String> {
    // Pipes não-herdáveis: o ConPTY duplica as pontas que precisa; o filho não
    // herda as nossas (ele fala com o pseudoconsole pelo atributo).
    let mut sa: SECURITY_ATTRIBUTES = std::mem::zeroed();
    sa.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
    sa.bInheritHandle = 0;

    let mut in_read: Handle = std::ptr::null_mut();
    let mut in_write: Handle = std::ptr::null_mut();
    let mut out_read: Handle = std::ptr::null_mut();
    let mut out_write: Handle = std::ptr::null_mut();
    if CreatePipe(&mut in_read, &mut in_write, &sa, 0) == 0 {
        return Err(format!("CreatePipe(stdin) falhou: {}", last_error()));
    }
    if CreatePipe(&mut out_read, &mut out_write, &sa, 0) == 0 {
        let e = last_error();
        CloseHandle(in_read);
        CloseHandle(in_write);
        return Err(format!("CreatePipe(stdout) falhou: {e}"));
    }

    let coord = COORD {
        X: params.size.cols as i16,
        Y: params.size.rows as i16,
    };
    let mut hpcon: HPCON = 0;
    let hr = CreatePseudoConsole(
        coord,
        in_read,
        out_write,
        PSEUDOCONSOLE_RESIZE_QUIRK,
        &mut hpcon,
    );
    // O ConPTY duplicou in_read/out_write; soltamos as nossas cópias.
    CloseHandle(in_read);
    CloseHandle(out_write);
    if hr != 0 {
        CloseHandle(in_write);
        CloseHandle(out_read);
        return Err(format!("CreatePseudoConsole falhou: 0x{hr:08X}"));
    }
    // A partir daqui, erro precisa fechar o hpcon também.
    let con = ConHandle(hpcon);

    let job = match create_kill_on_close_job() {
        Ok(j) => j,
        Err(e) => {
            drop(con);
            CloseHandle(in_write);
            CloseHandle(out_read);
            return Err(e);
        }
    };

    // Lista de atributos: pseudoconsole (sempre) + mitigation (se houver).
    let attr_count = 1 + params.mitigation.is_some() as u32;
    let mut attr_size: usize = 0;
    InitializeProcThreadAttributeList(std::ptr::null_mut(), attr_count, 0, &mut attr_size);
    let mut attr_buf = vec![0u8; attr_size];
    let attr_list = attr_buf.as_mut_ptr() as *mut _;
    let cleanup = |in_write: Handle, out_read: Handle| {
        CloseHandle(in_write);
        CloseHandle(out_read);
    };
    if InitializeProcThreadAttributeList(attr_list, attr_count, 0, &mut attr_size) == 0 {
        let e = last_error();
        drop(job);
        drop(con);
        cleanup(in_write, out_read);
        return Err(format!("InitializeProcThreadAttributeList falhou: {e}"));
    }
    if UpdateProcThreadAttribute(
        attr_list,
        0,
        PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
        hpcon as *const c_void,
        std::mem::size_of::<HPCON>(),
        std::ptr::null_mut(),
        std::ptr::null(),
    ) == 0
    {
        let e = last_error();
        DeleteProcThreadAttributeList(attr_list);
        drop(job);
        drop(con);
        cleanup(in_write, out_read);
        return Err(format!(
            "UpdateProcThreadAttribute(pseudoconsole) falhou: {e}"
        ));
    }
    let mut policy: u64 = params.mitigation.unwrap_or(0);
    if params.mitigation.is_some()
        && UpdateProcThreadAttribute(
            attr_list,
            0,
            PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
            &mut policy as *mut u64 as *const c_void,
            std::mem::size_of::<u64>(),
            std::ptr::null_mut(),
            std::ptr::null(),
        ) == 0
    {
        let e = last_error();
        DeleteProcThreadAttributeList(attr_list);
        drop(job);
        drop(con);
        cleanup(in_write, out_read);
        return Err(format!("UpdateProcThreadAttribute(mitigation) falhou: {e}"));
    }

    let mut si: STARTUPINFOEXW = std::mem::zeroed();
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    // Espelha o portable-pty: stdio inválido + USESTDHANDLES para o filho não
    // herdar por acidente handles redirecionados do pai — ele usa só o ConPTY.
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    si.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    si.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    si.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    si.lpAttributeList = attr_list;

    let flags = EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED_FLAG;
    let cwd_ptr = params
        .cwd
        .as_ref()
        .map(|c| c.as_ptr())
        .unwrap_or(std::ptr::null());
    let mut pi: PROCESS_INFORMATION = std::mem::zeroed();

    // Suspenso: atribui ao Job Object ANTES de rodar, então nenhum filho escapa
    // do kill-on-close entre o spawn e o assign. Token nulo = sessão de shell (sem
    // jaula) → CreateProcessW; com token = agente enjaulado → CreateProcessAsUserW.
    let created = if params.token.is_null() {
        CreateProcessW(
            std::ptr::null(),
            params.command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            flags,
            params.env_block.as_mut_ptr() as *const c_void,
            cwd_ptr,
            &si.StartupInfo,
            &mut pi,
        )
    } else {
        CreateProcessAsUserW(
            params.token,
            std::ptr::null(),
            params.command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            flags,
            params.env_block.as_mut_ptr() as *const c_void,
            cwd_ptr,
            &si.StartupInfo,
            &mut pi,
        )
    };
    if created == 0 {
        let e = last_error();
        DeleteProcThreadAttributeList(attr_list);
        drop(job);
        drop(con);
        cleanup(in_write, out_read);
        return Err(format!(
            "CreateProcess (token={}) falhou: {e}",
            !params.token.is_null()
        ));
    }
    DeleteProcThreadAttributeList(attr_list);

    if AssignProcessToJobObject(job.0.as_raw_handle() as Handle, pi.hProcess) == 0 {
        // Sem o Job Object não há paridade com killpg — fail-closed.
        let e = last_error();
        TerminateProcess(pi.hProcess, 1);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
        drop(job);
        drop(con);
        cleanup(in_write, out_read);
        return Err(format!("AssignProcessToJobObject falhou: {e}"));
    }
    ResumeThread(pi.hThread);
    CloseHandle(pi.hThread);

    let pid = GetProcessId(pi.hProcess);
    let proc = OwnedHandle::from_raw_handle(pi.hProcess as RawHandle);

    let master = JailedMaster {
        con,
        reader: Mutex::new(Some(File::from_raw_handle(out_read as RawHandle))),
        writer: Mutex::new(Some(File::from_raw_handle(in_write as RawHandle))),
        size: Mutex::new(params.size),
    };
    let child = JailedChild {
        proc: Mutex::new(proc),
        job: Arc::new(job.0),
        pid,
    };
    Ok((Box::new(master), Box::new(child)))
}

unsafe fn create_kill_on_close_job() -> Result<JobHandle, String> {
    let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
    if job.is_null() {
        return Err(format!("CreateJobObjectW falhou: {}", last_error()));
    }
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if SetInformationJobObject(
        job,
        JobObjectExtendedLimitInformation,
        &info as *const _ as *const c_void,
        std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
    ) == 0
    {
        let e = last_error();
        CloseHandle(job);
        return Err(format!(
            "SetInformationJobObject(kill-on-close) falhou: {e}"
        ));
    }
    Ok(JobHandle(OwnedHandle::from_raw_handle(job as RawHandle)))
}

/// Wrapper do `HPCON` (é `isize`, portanto `Send`/`Sync`) que fecha o
/// pseudoconsole no drop — solta o EOF na saída, terminando a thread leitora.
struct ConHandle(HPCON);

impl Drop for ConHandle {
    fn drop(&mut self) {
        unsafe { ClosePseudoConsole(self.0) };
    }
}

/// RAII sobre o handle do Job Object.
struct JobHandle(OwnedHandle);

struct JailedMaster {
    con: ConHandle,
    reader: Mutex<Option<File>>,
    writer: Mutex<Option<File>>,
    size: Mutex<PtySize>,
}

impl MasterPty for JailedMaster {
    fn resize(&self, size: PtySize) -> Result<(), anyhow::Error> {
        let coord = COORD {
            X: size.cols as i16,
            Y: size.rows as i16,
        };
        let hr = unsafe { ResizePseudoConsole(self.con.0, coord) };
        if hr != 0 {
            anyhow::bail!("ResizePseudoConsole falhou: 0x{hr:08X}");
        }
        *self.size.lock().unwrap() = size;
        Ok(())
    }

    fn get_size(&self) -> Result<PtySize, anyhow::Error> {
        Ok(*self.size.lock().unwrap())
    }

    fn try_clone_reader(&self) -> Result<Box<dyn Read + Send>, anyhow::Error> {
        self.reader
            .lock()
            .unwrap()
            .take()
            .map(|f| Box::new(f) as Box<dyn Read + Send>)
            .ok_or_else(|| anyhow::anyhow!("reader do ConPTY já foi tomado"))
    }

    fn take_writer(&self) -> Result<Box<dyn Write + Send>, anyhow::Error> {
        self.writer
            .lock()
            .unwrap()
            .take()
            .map(|f| Box::new(f) as Box<dyn Write + Send>)
            .ok_or_else(|| anyhow::anyhow!("writer do ConPTY já foi tomado"))
    }
}

#[derive(Debug)]
struct JailedChild {
    proc: Mutex<OwnedHandle>,
    /// Job Object kill-on-close. Fechar o último handle (drop) mata a árvore do
    /// agente — é o que dá paridade com o killpg quando o TYBA some. Compartilhado
    /// com os killers clonados via `Arc`, então o kill-on-close só dispara quando
    /// todos somem.
    job: Arc<OwnedHandle>,
    pid: u32,
}

impl JailedChild {
    fn exit_status(&self) -> IoResult<Option<ExitStatus>> {
        let mut code: u32 = 0;
        let proc = self.proc.lock().unwrap();
        let ok = unsafe { GetExitCodeProcess(proc.as_raw_handle() as Handle, &mut code) };
        if ok == 0 {
            return Ok(None);
        }
        if code == STILL_ACTIVE_CODE {
            Ok(None)
        } else {
            Ok(Some(ExitStatus::with_exit_code(code)))
        }
    }
}

impl Child for JailedChild {
    fn try_wait(&mut self) -> IoResult<Option<ExitStatus>> {
        self.exit_status()
    }

    fn wait(&mut self) -> IoResult<ExitStatus> {
        if let Ok(Some(status)) = self.exit_status() {
            return Ok(status);
        }
        let raw = self.proc.lock().unwrap().as_raw_handle() as Handle;
        unsafe { WaitForSingleObject(raw, INFINITE_MS) };
        let mut code: u32 = 0;
        let ok = unsafe { GetExitCodeProcess(raw, &mut code) };
        if ok == 0 {
            return Err(IoError::last_os_error());
        }
        Ok(ExitStatus::with_exit_code(code))
    }

    fn process_id(&self) -> Option<u32> {
        (self.pid != 0).then_some(self.pid)
    }

    fn as_raw_handle(&self) -> Option<RawHandle> {
        Some(self.proc.lock().unwrap().as_raw_handle())
    }
}

impl ChildKiller for JailedChild {
    fn kill(&mut self) -> IoResult<()> {
        // Mata a árvore inteira, não só a raiz — o Job Object é a paridade do killpg.
        unsafe { TerminateJobObject(self.job.as_raw_handle() as Handle, 1) };
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(JailedKiller {
            job: Arc::clone(&self.job),
        })
    }
}

#[derive(Debug)]
struct JailedKiller {
    job: Arc<OwnedHandle>,
}

impl ChildKiller for JailedKiller {
    fn kill(&mut self) -> IoResult<()> {
        unsafe { TerminateJobObject(self.job.as_raw_handle() as Handle, 1) };
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(JailedKiller {
            job: Arc::clone(&self.job),
        })
    }
}

// --- Re-encode da cmdline/env/cwd -------------------------------------------
//
// `CommandBuilder::cmdline`/`environment_block`/`current_directory` são
// `pub(crate)` no portable-pty (inacessíveis daqui), então reconstruímos a mesma
// coisa a partir dos getters públicos (`get_argv`, `get_env`, `iter_full_env_as_str`,
// `get_cwd`). O quoting espelha o `append_quoted` do portable-pty (regras do
// ArgvQuote), senão argumentos com espaço/aspas quebram sob a jaula.

/// Linha de comando wide terminada em NUL, resolvendo o executável pelo PATH da
/// jaula (como o `portable-pty` faz), pronta para o `CreateProcessAsUserW`.
pub fn encode_command_line(cmd: &CommandBuilder) -> Result<Vec<u16>, String> {
    let argv = cmd.get_argv();
    if argv.is_empty() {
        return Err("comando da jaula sem argv".into());
    }
    let exe = search_path(cmd, &argv[0]);
    let mut line: Vec<u16> = Vec::new();
    append_quoted(&exe, &mut line);
    for arg in argv.iter().skip(1) {
        if arg.encode_wide().any(|c| c == 0) {
            return Err(format!("argumento com NUL embutido: {arg:?}"));
        }
        line.push(' ' as u16);
        append_quoted(arg, &mut line);
    }
    line.push(0);
    Ok(line)
}

/// Bloco de ambiente wide (`K=V\0`…`\0`) montado do env do comando (já allowlist)
/// mais `extra` — que sobrescreve chaves iguais (case-insensitive), ex.: o marcador
/// `TYBA_SANDBOX`.
pub fn encode_env_block(cmd: &CommandBuilder, extra: &[(String, String)]) -> Vec<u16> {
    let mut pairs: Vec<(String, String)> = cmd
        .iter_full_env_as_str()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    for (k, v) in extra {
        pairs.retain(|(ek, _)| !ek.eq_ignore_ascii_case(k));
        pairs.push((k.clone(), v.clone()));
    }
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in &pairs {
        block.extend(k.encode_utf16());
        block.push('=' as u16);
        block.extend(v.encode_utf16());
        block.push(0);
    }
    block.push(0);
    block
}

/// Diretório de trabalho wide terminado em NUL (absoluto), ou `None`.
pub fn encode_cwd(cmd: &CommandBuilder) -> Option<Vec<u16>> {
    let cwd = cmd.get_cwd()?;
    let path = Path::new(cwd);
    let mut wide: Vec<u16> = if path.is_relative() {
        match std::env::current_dir() {
            Ok(base) => base.join(path).as_os_str().encode_wide().collect(),
            Err(_) => path.as_os_str().encode_wide().collect(),
        }
    } else {
        path.as_os_str().encode_wide().collect()
    };
    wide.push(0);
    Some(wide)
}

/// Resolve o executável pelo PATH/PATHEXT do comando — mesma busca do portable-pty.
fn search_path(cmd: &CommandBuilder, exe: &OsStr) -> OsString {
    if let Some(path) = cmd.get_env("PATH") {
        let default_ext = OsStr::new(".EXE");
        let extensions = cmd.get_env("PATHEXT").unwrap_or(default_ext);
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(exe);
            if candidate.exists() {
                return candidate.into_os_string();
            }
            for ext in std::env::split_paths(&extensions) {
                if let Some(ext) = ext.to_str() {
                    let with_ext = dir.join(exe).with_extension(&ext[1.min(ext.len())..]);
                    if with_ext.exists() {
                        return with_ext.into_os_string();
                    }
                }
            }
        }
    }
    exe.to_owned()
}

/// Quoting do ArgvQuote (idêntico ao `append_quoted` do portable-pty).
fn append_quoted(arg: &OsStr, out: &mut Vec<u16>) {
    let needs_quote = arg.is_empty()
        || arg.encode_wide().any(|c| {
            c == ' ' as u16 || c == '\t' as u16 || c == '\n' as u16 || c == 0x0b || c == '"' as u16
        });
    if !needs_quote {
        out.extend(arg.encode_wide());
        return;
    }
    out.push('"' as u16);
    let chars: Vec<u16> = arg.encode_wide().collect();
    let mut i = 0;
    while i < chars.len() {
        let mut backslashes = 0;
        while i < chars.len() && chars[i] == '\\' as u16 {
            i += 1;
            backslashes += 1;
        }
        if i == chars.len() {
            for _ in 0..backslashes * 2 {
                out.push('\\' as u16);
            }
            break;
        } else if chars[i] == '"' as u16 {
            for _ in 0..backslashes * 2 + 1 {
                out.push('\\' as u16);
            }
            out.push(chars[i]);
        } else {
            for _ in 0..backslashes {
                out.push('\\' as u16);
            }
            out.push(chars[i]);
        }
        i += 1;
    }
    out.push('"' as u16);
}

#[cfg(test)]
fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    OsString::from_wide(&w[..end])
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmdline_quota_argumentos_com_espaco_e_aspas() {
        let mut cmd = CommandBuilder::new("prog.exe");
        cmd.arg("um dois");
        cmd.arg("tres");
        cmd.arg("com\"aspas");
        // Sem PATH no env, search_path devolve o exe cru — o quoting é o que importa.
        let line = encode_command_line(&cmd).expect("encode");
        assert!(line.ends_with(&[0]), "cmdline precisa terminar em NUL");
        let s = wide_to_string(&line);
        assert!(s.contains("\"um dois\""), "arg com espaço vira aspas: {s}");
        assert!(s.contains(" tres"), "arg simples fica sem aspas: {s}");
        assert!(s.contains("\\\""), "aspas internas escapadas: {s}");
    }

    #[test]
    fn env_block_termina_em_duplo_nul_e_injeta_marcador() {
        let mut cmd = CommandBuilder::new("prog.exe");
        cmd.env_clear();
        cmd.env("PATH", "C:\\bin");
        cmd.env("FOO", "bar");
        let block = encode_env_block(&cmd, &[("TYBA_SANDBOX".to_string(), "windows".to_string())]);
        assert_eq!(
            &block[block.len() - 2..],
            &[0, 0],
            "bloco de env termina em \\0\\0"
        );
        let joined: String = block
            .split(|&c| c == 0)
            .map(|seg| OsString::from_wide(seg).to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("FOO=bar"),
            "env herdado do comando: {joined}"
        );
        assert!(
            joined.contains("TYBA_SANDBOX=windows"),
            "marcador injetado: {joined}"
        );
    }

    #[test]
    fn env_extra_sobrescreve_chave_existente_case_insensitive() {
        let mut cmd = CommandBuilder::new("prog.exe");
        cmd.env_clear();
        cmd.env("Tyba_Sandbox", "errado");
        let block = encode_env_block(&cmd, &[("TYBA_SANDBOX".to_string(), "windows".to_string())]);
        let joined: String = block
            .split(|&c| c == 0)
            .map(|seg| OsString::from_wide(seg).to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("TYBA_SANDBOX=windows"));
        assert!(!joined.contains("errado"), "extra sobrescreve o antigo");
    }
}
