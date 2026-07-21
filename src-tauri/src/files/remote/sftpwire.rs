use std::io::{BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use parking_lot::Mutex;

use super::fs::{ExecOutput, RemoteDirEntry, RemoteError, RemoteFs, RemoteResult, RemoteStat};

const FXP_INIT: u8 = 1;
const FXP_VERSION: u8 = 2;
const FXP_OPEN: u8 = 3;
const FXP_CLOSE: u8 = 4;
const FXP_READ: u8 = 5;
const FXP_WRITE: u8 = 6;
const FXP_LSTAT: u8 = 7;
const FXP_OPENDIR: u8 = 11;
const FXP_READDIR: u8 = 12;
const FXP_REMOVE: u8 = 13;
const FXP_MKDIR: u8 = 14;
const FXP_RMDIR: u8 = 15;
const FXP_REALPATH: u8 = 16;
const FXP_STAT: u8 = 17;
const FXP_RENAME: u8 = 18;
const FXP_EXTENDED: u8 = 200;

const FXP_STATUS: u8 = 101;
const FXP_HANDLE: u8 = 102;
const FXP_DATA: u8 = 103;
const FXP_NAME: u8 = 104;
const FXP_ATTRS: u8 = 105;

const FX_OK: u32 = 0;
const FX_EOF: u32 = 1;
const FX_NO_SUCH_FILE: u32 = 2;
const FX_PERMISSION_DENIED: u32 = 3;

const PFLAG_READ: u32 = 0x0000_0001;
const PFLAG_WRITE: u32 = 0x0000_0002;
const PFLAG_CREAT: u32 = 0x0000_0008;
const PFLAG_EXCL: u32 = 0x0000_0020;

const ATTR_SIZE: u32 = 0x0000_0001;
const ATTR_PERMISSIONS: u32 = 0x0000_0004;

const WRITE_CHUNK: usize = 32 * 1024;
const SFTP_VERSION: u32 = 3;

fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn put_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn put_string(buf: &mut Vec<u8>, bytes: &[u8]) {
    put_u32(buf, bytes.len() as u32);
    buf.extend_from_slice(bytes);
}

/// Atributos mínimos: só o campo de permissões (para write_new/mkdir preservarem
/// mode). `flags=0` = "sem atributos".
fn put_attrs_perms(buf: &mut Vec<u8>, mode: Option<u32>) {
    match mode {
        Some(m) => {
            put_u32(buf, ATTR_PERMISSIONS);
            put_u32(buf, m & 0o7777);
        }
        None => put_u32(buf, 0),
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn u32(&mut self) -> RemoteResult<u32> {
        if self.pos + 4 > self.data.len() {
            return Err(RemoteError::Protocol("u32 curto".into()));
        }
        let v = u32::from_be_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }

    fn u64(&mut self) -> RemoteResult<u64> {
        if self.pos + 8 > self.data.len() {
            return Err(RemoteError::Protocol("u64 curto".into()));
        }
        let v = u64::from_be_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Ok(v)
    }

    fn string(&mut self) -> RemoteResult<Vec<u8>> {
        let len = self.u32()? as usize;
        if self.pos + len > self.data.len() {
            return Err(RemoteError::Protocol("string curta".into()));
        }
        let out = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(out)
    }
}

/// Consome um bloco de atributos SFTP v3, devolvendo (size, mode). Campos
/// ausentes viram 0.
fn parse_attrs(cur: &mut Cursor<'_>) -> RemoteResult<RemoteStat> {
    let flags = cur.u32()?;
    let mut size = 0u64;
    let mut mode = 0u32;
    if flags & ATTR_SIZE != 0 {
        size = cur.u64()?;
    }
    if flags & 0x0000_0002 != 0 {
        let _uid = cur.u32()?;
        let _gid = cur.u32()?;
    }
    if flags & ATTR_PERMISSIONS != 0 {
        mode = cur.u32()?;
    }
    if flags & 0x0000_0008 != 0 {
        let _atime = cur.u32()?;
        let _mtime = cur.u32()?;
    }
    if flags & 0x8000_0000 != 0 {
        let count = cur.u32()?;
        for _ in 0..count {
            let _ = cur.string()?;
            let _ = cur.string()?;
        }
    }
    Ok(RemoteStat { size, mode })
}

fn status_to_err(code: u32, message: String) -> RemoteError {
    match code {
        FX_NO_SUCH_FILE => RemoteError::NotFound,
        FX_PERMISSION_DENIED => RemoteError::Denied,
        _ => RemoteError::Sftp { code, message },
    }
}

struct Conn {
    reader: BufReader<ChildStdout>,
    writer: ChildStdin,
    next_id: u32,
    child: Child,
}

impl Conn {
    fn send(&mut self, ty: u8, payload: &[u8]) -> RemoteResult<()> {
        let mut frame = Vec::with_capacity(5 + payload.len());
        put_u32(&mut frame, 1 + payload.len() as u32);
        frame.push(ty);
        frame.extend_from_slice(payload);
        self.writer
            .write_all(&frame)
            .map_err(|_| RemoteError::Disconnected)?;
        self.writer.flush().map_err(|_| RemoteError::Disconnected)?;
        Ok(())
    }

    fn recv(&mut self) -> RemoteResult<(u8, Vec<u8>)> {
        let mut len_buf = [0u8; 4];
        self.reader
            .read_exact(&mut len_buf)
            .map_err(|_| RemoteError::Disconnected)?;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len == 0 || len > 512 * 1024 * 1024 {
            return Err(RemoteError::Protocol("frame com tamanho inválido".into()));
        }
        let mut body = vec![0u8; len];
        self.reader
            .read_exact(&mut body)
            .map_err(|_| RemoteError::Disconnected)?;
        let ty = body[0];
        Ok((ty, body[1..].to_vec()))
    }

    fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    /// Envia uma requisição e lê a resposta (sem pipelining: a próxima frame é a
    /// resposta desta requisição). Confere o id.
    fn request(&mut self, ty: u8, id: u32, payload: &[u8]) -> RemoteResult<(u8, Vec<u8>)> {
        self.send(ty, payload)?;
        let (rty, body) = self.recv()?;
        let mut cur = Cursor::new(&body);
        let resp_id = cur.u32()?;
        if resp_id != id {
            return Err(RemoteError::Protocol(format!(
                "id de resposta {resp_id} != {id}"
            )));
        }
        Ok((rty, body))
    }

    fn expect_status_ok(&mut self, ty: u8, id: u32, payload: &[u8]) -> RemoteResult<()> {
        let (rty, body) = self.request(ty, id, payload)?;
        if rty != FXP_STATUS {
            return Err(RemoteError::Protocol("esperava STATUS".into()));
        }
        let mut cur = Cursor::new(&body);
        let _id = cur.u32()?;
        let code = cur.u32()?;
        if code == FX_OK {
            return Ok(());
        }
        let msg = cur.string().unwrap_or_default();
        Err(status_to_err(
            code,
            String::from_utf8_lossy(&msg).into_owned(),
        ))
    }

    fn open_handle(&mut self, path: &str, pflags: u32, mode: Option<u32>) -> RemoteResult<Vec<u8>> {
        let id = self.alloc_id();
        let mut p = Vec::new();
        put_u32(&mut p, id);
        put_string(&mut p, path.as_bytes());
        put_u32(&mut p, pflags);
        put_attrs_perms(&mut p, mode);
        let (rty, body) = self.request(FXP_OPEN, id, &p)?;
        handle_or_status(rty, &body)
    }

    fn close_handle(&mut self, handle: &[u8]) -> RemoteResult<()> {
        let id = self.alloc_id();
        let mut p = Vec::new();
        put_u32(&mut p, id);
        put_string(&mut p, handle);
        self.expect_status_ok(FXP_CLOSE, id, &p)
    }
}

pub struct SshRemote {
    alias: String,
    conn: Mutex<Conn>,
}

impl SshRemote {
    /// Abre o subsistema SFTP na conexão SSH multiplexada existente (`ssh <alias>
    /// -s sftp` reusa o ControlMaster do ssh_config — zero handshake novo). Faz o
    /// INIT/VERSION do protocolo v3.
    pub fn connect(alias: &str) -> RemoteResult<Self> {
        let mut child = Command::new("ssh")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg(alias)
            .arg("-s")
            .arg("sftp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| RemoteError::Exec(e.to_string()))?;

        let writer = child.stdin.take().ok_or(RemoteError::Disconnected)?;
        let reader = BufReader::new(child.stdout.take().ok_or(RemoteError::Disconnected)?);
        let mut conn = Conn {
            reader,
            writer,
            next_id: 1,
            child,
        };

        let mut init = Vec::new();
        put_u32(&mut init, SFTP_VERSION);
        conn.send(FXP_INIT, &init)?;
        let (ty, body) = conn.recv()?;
        if ty != FXP_VERSION {
            return Err(RemoteError::Protocol("esperava VERSION".into()));
        }
        let mut cur = Cursor::new(&body);
        let version = cur.u32()?;
        if version < 3 {
            return Err(RemoteError::Protocol(format!(
                "versão SFTP {version} sem suporte"
            )));
        }
        Ok(Self {
            alias: alias.to_string(),
            conn: Mutex::new(conn),
        })
    }
}

fn handle_or_status(rty: u8, body: &[u8]) -> RemoteResult<Vec<u8>> {
    let mut cur = Cursor::new(body);
    let _id = cur.u32()?;
    if rty == FXP_HANDLE {
        return cur.string();
    }
    if rty == FXP_STATUS {
        let code = cur.u32()?;
        let msg = cur.string().unwrap_or_default();
        return Err(status_to_err(
            code,
            String::from_utf8_lossy(&msg).into_owned(),
        ));
    }
    Err(RemoteError::Protocol("esperava HANDLE".into()))
}

/// Parseia a resposta NAME (usada por REALPATH e READDIR).
fn parse_name(body: &[u8]) -> RemoteResult<Vec<RemoteDirEntry>> {
    let mut cur = Cursor::new(body);
    let _id = cur.u32()?;
    let count = cur.u32()?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name = cur.string()?;
        let _longname = cur.string()?;
        let stat = parse_attrs(&mut cur)?;
        out.push(RemoteDirEntry {
            name: String::from_utf8_lossy(&name).into_owned(),
            stat,
        });
    }
    Ok(out)
}

impl RemoteFs for SshRemote {
    fn host(&self) -> &str {
        &self.alias
    }

    fn realpath(&self, path: &str) -> RemoteResult<String> {
        let mut conn = self.conn.lock();
        let id = conn.alloc_id();
        let mut p = Vec::new();
        put_u32(&mut p, id);
        put_string(&mut p, path.as_bytes());
        let (rty, body) = conn.request(FXP_REALPATH, id, &p)?;
        if rty == FXP_STATUS {
            let mut cur = Cursor::new(&body);
            let _id = cur.u32()?;
            let code = cur.u32()?;
            let msg = cur.string().unwrap_or_default();
            return Err(status_to_err(
                code,
                String::from_utf8_lossy(&msg).into_owned(),
            ));
        }
        if rty != FXP_NAME {
            return Err(RemoteError::Protocol("esperava NAME".into()));
        }
        let entries = parse_name(&body)?;
        entries
            .into_iter()
            .next()
            .map(|e| e.name)
            .ok_or_else(|| RemoteError::Protocol("NAME vazio no realpath".into()))
    }

    fn readdir(&self, path: &str) -> RemoteResult<Vec<RemoteDirEntry>> {
        let mut conn = self.conn.lock();
        let id = conn.alloc_id();
        let mut p = Vec::new();
        put_u32(&mut p, id);
        put_string(&mut p, path.as_bytes());
        let (rty, body) = conn.request(FXP_OPENDIR, id, &p)?;
        let handle = handle_or_status(rty, &body)?;

        let mut entries = Vec::new();
        loop {
            let rid = conn.alloc_id();
            let mut rp = Vec::new();
            put_u32(&mut rp, rid);
            put_string(&mut rp, &handle);
            let (rty, body) = conn.request(FXP_READDIR, rid, &rp)?;
            if rty == FXP_STATUS {
                let mut cur = Cursor::new(&body);
                let _id = cur.u32()?;
                let code = cur.u32()?;
                if code == FX_EOF {
                    break;
                }
                let msg = cur.string().unwrap_or_default();
                let _ = conn.close_handle(&handle);
                return Err(status_to_err(
                    code,
                    String::from_utf8_lossy(&msg).into_owned(),
                ));
            }
            if rty != FXP_NAME {
                let _ = conn.close_handle(&handle);
                return Err(RemoteError::Protocol("esperava NAME no readdir".into()));
            }
            for entry in parse_name(&body)? {
                if entry.name == "." || entry.name == ".." {
                    continue;
                }
                entries.push(entry);
            }
        }
        conn.close_handle(&handle)?;
        Ok(entries)
    }

    fn lstat(&self, path: &str) -> RemoteResult<RemoteStat> {
        stat_op(self, FXP_LSTAT, path)
    }

    fn stat(&self, path: &str) -> RemoteResult<RemoteStat> {
        stat_op(self, FXP_STAT, path)
    }

    fn read_chunk(&self, path: &str, offset: u64, len: usize) -> RemoteResult<Vec<u8>> {
        let mut conn = self.conn.lock();
        let handle = conn.open_handle(path, PFLAG_READ, None)?;
        let mut out = Vec::with_capacity(len);
        let mut pos = offset;
        while out.len() < len {
            let want = (len - out.len()).min(WRITE_CHUNK) as u32;
            let id = conn.alloc_id();
            let mut p = Vec::new();
            put_u32(&mut p, id);
            put_string(&mut p, &handle);
            put_u64(&mut p, pos);
            put_u32(&mut p, want);
            let (rty, body) = conn.request(FXP_READ, id, &p)?;
            if rty == FXP_STATUS {
                let mut cur = Cursor::new(&body);
                let _id = cur.u32()?;
                let code = cur.u32()?;
                if code == FX_EOF {
                    break;
                }
                let msg = cur.string().unwrap_or_default();
                let _ = conn.close_handle(&handle);
                return Err(status_to_err(
                    code,
                    String::from_utf8_lossy(&msg).into_owned(),
                ));
            }
            if rty != FXP_DATA {
                let _ = conn.close_handle(&handle);
                return Err(RemoteError::Protocol("esperava DATA".into()));
            }
            let mut cur = Cursor::new(&body);
            let _id = cur.u32()?;
            let chunk = cur.string()?;
            if chunk.is_empty() {
                break;
            }
            pos += chunk.len() as u64;
            out.extend_from_slice(&chunk);
        }
        conn.close_handle(&handle)?;
        Ok(out)
    }

    fn write_new(&self, path: &str, data: &[u8], mode: u32) -> RemoteResult<()> {
        let mut conn = self.conn.lock();
        let handle = conn.open_handle(path, PFLAG_WRITE | PFLAG_CREAT | PFLAG_EXCL, Some(mode))?;
        let mut pos = 0u64;
        for chunk in data.chunks(WRITE_CHUNK) {
            let id = conn.alloc_id();
            let mut p = Vec::new();
            put_u32(&mut p, id);
            put_string(&mut p, &handle);
            put_u64(&mut p, pos);
            put_string(&mut p, chunk);
            if let Err(e) = conn.expect_status_ok(FXP_WRITE, id, &p) {
                let _ = conn.close_handle(&handle);
                return Err(e);
            }
            pos += chunk.len() as u64;
        }
        conn.close_handle(&handle)
    }

    fn posix_rename(&self, from: &str, to: &str) -> RemoteResult<()> {
        let mut conn = self.conn.lock();
        let id = conn.alloc_id();
        let mut p = Vec::new();
        put_u32(&mut p, id);
        put_string(&mut p, b"posix-rename@openssh.com");
        put_string(&mut p, from.as_bytes());
        put_string(&mut p, to.as_bytes());
        conn.expect_status_ok(FXP_EXTENDED, id, &p)
    }

    fn rename(&self, from: &str, to: &str) -> RemoteResult<()> {
        let mut conn = self.conn.lock();
        let id = conn.alloc_id();
        let mut p = Vec::new();
        put_u32(&mut p, id);
        put_string(&mut p, from.as_bytes());
        put_string(&mut p, to.as_bytes());
        conn.expect_status_ok(FXP_RENAME, id, &p)
    }

    fn remove(&self, path: &str) -> RemoteResult<()> {
        let mut conn = self.conn.lock();
        let id = conn.alloc_id();
        let mut p = Vec::new();
        put_u32(&mut p, id);
        put_string(&mut p, path.as_bytes());
        conn.expect_status_ok(FXP_REMOVE, id, &p)
    }

    fn rmdir(&self, path: &str) -> RemoteResult<()> {
        let mut conn = self.conn.lock();
        let id = conn.alloc_id();
        let mut p = Vec::new();
        put_u32(&mut p, id);
        put_string(&mut p, path.as_bytes());
        conn.expect_status_ok(FXP_RMDIR, id, &p)
    }

    fn mkdir(&self, path: &str, mode: u32) -> RemoteResult<()> {
        let mut conn = self.conn.lock();
        let id = conn.alloc_id();
        let mut p = Vec::new();
        put_u32(&mut p, id);
        put_string(&mut p, path.as_bytes());
        put_attrs_perms(&mut p, Some(mode));
        conn.expect_status_ok(FXP_MKDIR, id, &p)
    }

    fn exec(&self, argv: &[&str]) -> RemoteResult<ExecOutput> {
        let cmd_string = argv
            .iter()
            .map(|a| sh_quote(a))
            .collect::<Vec<_>>()
            .join(" ");
        let out = Command::new("ssh")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg(&self.alias)
            .arg(&cmd_string)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| RemoteError::Exec(e.to_string()))?;
        Ok(ExecOutput {
            code: out.status.code().unwrap_or(-1),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    }
}

impl Drop for SshRemote {
    fn drop(&mut self) {
        let mut conn = self.conn.lock();
        let _ = conn.writer.flush();
        let _ = conn.child.kill();
        let _ = conn.child.wait();
    }
}

fn stat_op(remote: &SshRemote, ty: u8, path: &str) -> RemoteResult<RemoteStat> {
    let mut conn = remote.conn.lock();
    let id = conn.alloc_id();
    let mut p = Vec::new();
    put_u32(&mut p, id);
    put_string(&mut p, path.as_bytes());
    let (rty, body) = conn.request(ty, id, &p)?;
    if rty == FXP_STATUS {
        let mut cur = Cursor::new(&body);
        let _id = cur.u32()?;
        let code = cur.u32()?;
        let msg = cur.string().unwrap_or_default();
        return Err(status_to_err(
            code,
            String::from_utf8_lossy(&msg).into_owned(),
        ));
    }
    if rty != FXP_ATTRS {
        return Err(RemoteError::Protocol("esperava ATTRS".into()));
    }
    let mut cur = Cursor::new(&body);
    let _id = cur.u32()?;
    parse_attrs(&mut cur)
}

/// Escapa um argumento para o shell remoto (single-quote POSIX). O git remoto
/// recebe o path da raiz e, no gutter, o rel — nomes hostis não escapam do quote.
fn sh_quote(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('\'');
    for ch in arg.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs_bytes(flags: u32, size: Option<u64>, mode: Option<u32>) -> Vec<u8> {
        let mut b = Vec::new();
        put_u32(&mut b, flags);
        if let Some(s) = size {
            put_u64(&mut b, s);
        }
        if let Some(m) = mode {
            put_u32(&mut b, m);
        }
        b
    }

    #[test]
    fn parse_attrs_reads_size_and_permissions() {
        let bytes = attrs_bytes(ATTR_SIZE | ATTR_PERMISSIONS, Some(4096), Some(0o100644));
        let mut cur = Cursor::new(&bytes);
        let stat = parse_attrs(&mut cur).unwrap();
        assert_eq!(stat.size, 4096);
        assert_eq!(stat.perm(), 0o644);
        assert!(stat.is_file());
        assert!(!stat.is_dir());
    }

    #[test]
    fn parse_attrs_flags_a_symlink_by_mode() {
        let bytes = attrs_bytes(ATTR_PERMISSIONS, None, Some(0o120777));
        let mut cur = Cursor::new(&bytes);
        let stat = parse_attrs(&mut cur).unwrap();
        assert!(stat.is_symlink());
        assert!(!stat.is_dir());
        assert!(!stat.is_file());
    }

    #[test]
    fn parse_name_reads_entries_with_hostile_bytes() {
        // NAME: id, count, then per-entry (filename, longname, attrs).
        let mut body = Vec::new();
        put_u32(&mut body, 7); // id
        put_u32(&mut body, 2); // count
                               // entry 1: a dir with a space in the name
        put_string(&mut body, b"weird dir");
        put_string(&mut body, b"drwxr-xr-x ... weird dir");
        body.extend_from_slice(&attrs_bytes(ATTR_PERMISSIONS, None, Some(0o040755)));
        // entry 2: a regular file with a control char
        put_string(&mut body, b"na\x01me.txt");
        put_string(&mut body, b"-rw-r--r-- ... na?me.txt");
        body.extend_from_slice(&attrs_bytes(
            ATTR_SIZE | ATTR_PERMISSIONS,
            Some(12),
            Some(0o100644),
        ));

        let entries = parse_name(&body).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "weird dir");
        assert!(entries[0].stat.is_dir());
        assert_eq!(entries[1].name, "na\x01me.txt");
        assert!(entries[1].stat.is_file());
        assert_eq!(entries[1].stat.size, 12);
    }

    #[test]
    fn realpath_request_is_length_prefixed_and_typed() {
        // Reconstrói o que send() escreveria para um REALPATH e confere o framing.
        let mut payload = Vec::new();
        put_u32(&mut payload, 42);
        put_string(&mut payload, b"/home/u/proj");
        let mut frame = Vec::new();
        put_u32(&mut frame, 1 + payload.len() as u32);
        frame.push(FXP_REALPATH);
        frame.extend_from_slice(&payload);

        let declared = u32::from_be_bytes(frame[0..4].try_into().unwrap()) as usize;
        assert_eq!(declared, frame.len() - 4);
        assert_eq!(frame[4], FXP_REALPATH);
    }

    #[test]
    fn sh_quote_escapes_single_quotes_and_wraps() {
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
        assert_eq!(sh_quote("/root"), "'/root'");
    }
}
