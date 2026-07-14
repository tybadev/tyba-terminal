//! Transporte de named pipe do gate no Windows (o AF_UNIX não atravessa a jaula;
//! spike `spikes/windows-sandbox`). Espelha a sonda `gate`: o servidor cria
//! instâncias do pipe e aceita; o cliente abre pelo nome. O nome deriva do path
//! do socket de forma determinística (server e client chegam ao mesmo).

use std::ffi::c_void;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::Path;

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Pipes::*;

const ERROR_PIPE_CONNECTED_CODE: u32 = 535;
const PIPE_BUFFER: u32 = 65536;

pub(crate) type Pipe = std::fs::File;

fn wide(name: &str) -> Vec<u16> {
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `\\.\pipe\tyba-hook-<hash do path>` — determinístico entre processos (sha2).
pub(crate) fn pipe_name(socket_path: &Path) -> Vec<u16> {
    let mut hasher = Sha256::new();
    hasher.update(socket_path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let name = format!(
        r"\\.\pipe\tyba-hook-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7]
    );
    wide(&name)
}

pub(crate) fn last_error() -> u32 {
    unsafe { GetLastError() }
}

/// Cria uma instância listening do pipe. Retorna o handle (INVALID em erro).
pub(crate) fn create_instance(name: &[u16]) -> *mut c_void {
    unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            std::ptr::null(),
        )
    }
}

/// Bloqueia até um cliente conectar. `true` = conectado (inclui o caso de o
/// cliente ter conectado antes do ConnectNamedPipe, ERROR_PIPE_CONNECTED).
pub(crate) fn wait_connect(handle: *mut c_void) -> bool {
    unsafe {
        if ConnectNamedPipe(handle, std::ptr::null_mut()) != 0 {
            return true;
        }
        last_error() == ERROR_PIPE_CONNECTED_CODE
    }
}

pub(crate) fn close(handle: *mut c_void) {
    unsafe {
        CloseHandle(handle);
    }
}

/// Envolve o handle de instância conectada num `File` (Read+Write). Ao dropar,
/// o handle fecha e a instância some.
pub(crate) fn into_file(handle: *mut c_void) -> Pipe {
    unsafe { std::fs::File::from_raw_handle(handle as RawHandle) }
}

/// Lado cliente: abre o pipe pelo nome. `None` se indisponível (retry no chamador).
pub(crate) fn connect(name: &[u16]) -> Option<Pipe> {
    unsafe {
        let handle = CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        Some(std::fs::File::from_raw_handle(handle as RawHandle))
    }
}
