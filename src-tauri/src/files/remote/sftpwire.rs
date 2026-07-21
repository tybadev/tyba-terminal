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

/// Opções de ssh para connect e exec: `BatchMode` (nunca prompt interativo, o
/// ControlMaster já autenticou), `ConnectTimeout` (host morto não trava minutos
/// — mesmo valor do tmux.rs) e keepalive (detecta o canal morto em ~45s em vez
/// de pendurar para sempre).
const SSH_LIVENESS_OPTS: &[&str] = &[
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=10",
    "-o",
    "ServerAliveInterval=15",
    "-o",
    "ServerAliveCountMax=3",
];

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

    fn take(&mut self, n: usize) -> RemoteResult<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|e| *e <= self.data.len())
            .ok_or_else(|| RemoteError::Protocol("leitura além do fim da frame".into()))?;
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn u32(&mut self) -> RemoteResult<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes(bytes.try_into().unwrap()))
    }

    fn u64(&mut self) -> RemoteResult<u64> {
        let bytes = self.take(8)?;
        Ok(u64::from_be_bytes(bytes.try_into().unwrap()))
    }

    fn string(&mut self) -> RemoteResult<Vec<u8>> {
        let len = self.u32()? as usize;
        Ok(self.take(len)?.to_vec())
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

/// Teto de frame: um `length` mentiroso do servidor não pode nos fazer alocar
/// gigabytes. Nossas maiores respostas (READ de 32 KiB, NAME de um diretório
/// capado em 2000 entradas) cabem folgado abaixo disto.
const MAX_FRAME: usize = 32 * 1024 * 1024;

/// Teto de entradas acumuladas num readdir: um host hostil pode responder NAME
/// pra sempre (sem EOF). Bem acima de qualquer diretório real, mas finito — o
/// painel ainda trunca a exibição em MAX_DIR_ENTRIES por cima.
const READDIR_CAP: usize = 65_536;

/// Escreve uma frame SFTP: `uint32 length | byte type | payload`. O length cobre
/// o byte de tipo mais o payload.
fn write_frame<W: Write>(w: &mut W, ty: u8, payload: &[u8]) -> RemoteResult<()> {
    let mut frame = Vec::with_capacity(5 + payload.len());
    put_u32(&mut frame, 1 + payload.len() as u32);
    frame.push(ty);
    frame.extend_from_slice(payload);
    w.write_all(&frame).map_err(|_| RemoteError::Disconnected)?;
    w.flush().map_err(|_| RemoteError::Disconnected)?;
    Ok(())
}

/// Lê uma frame SFTP inteira. `read_exact` remonta a frame mesmo quando o stream
/// entrega os bytes picados; frames coladas na mesma leitura ficam para a leitura
/// seguinte (o length delimita cada uma). O teto barra um `length` mentiroso
/// antes de qualquer alocação; um corte no meio da frame vira `Disconnected`.
fn read_frame<R: Read>(r: &mut R) -> RemoteResult<(u8, Vec<u8>)> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)
        .map_err(|_| RemoteError::Disconnected)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 {
        return Err(RemoteError::Protocol("frame de tamanho zero".into()));
    }
    if len > MAX_FRAME {
        return Err(RemoteError::Protocol(format!(
            "frame de {len} bytes acima do teto de {MAX_FRAME}"
        )));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)
        .map_err(|_| RemoteError::Disconnected)?;
    let ty = body[0];
    Ok((ty, body[1..].to_vec()))
}

/// Versão negociada no VERSION: só falamos v3, então qualquer outra é recusada
/// explicitamente em vez de tolerada.
fn check_version(body: &[u8]) -> RemoteResult<u32> {
    let mut cur = Cursor::new(body);
    let version = cur.u32()?;
    if version != SFTP_VERSION {
        return Err(RemoteError::Protocol(format!(
            "versão SFTP {version} != {SFTP_VERSION} (só falamos v3)"
        )));
    }
    Ok(version)
}

struct Conn<R: Read, W: Write> {
    reader: R,
    writer: W,
    next_id: u32,
    child: Option<Child>,
}

impl<R: Read, W: Write> Conn<R, W> {
    fn send(&mut self, ty: u8, payload: &[u8]) -> RemoteResult<()> {
        write_frame(&mut self.writer, ty, payload)
    }

    fn recv(&mut self) -> RemoteResult<(u8, Vec<u8>)> {
        read_frame(&mut self.reader)
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

    /// OPENDIR + laço de READDIR até EOF, com teto `cap` no acumulado (host
    /// hostil sem EOF é cortado) e `close_handle` garantido em todo caminho de
    /// erro — sem vazar handle no parse malformado.
    fn read_dir(&mut self, path: &str, cap: usize) -> RemoteResult<Vec<RemoteDirEntry>> {
        let id = self.alloc_id();
        let mut p = Vec::new();
        put_u32(&mut p, id);
        put_string(&mut p, path.as_bytes());
        let (rty, body) = self.request(FXP_OPENDIR, id, &p)?;
        let handle = handle_or_status(rty, &body)?;

        let mut entries: Vec<RemoteDirEntry> = Vec::new();
        loop {
            if entries.len() >= cap {
                break;
            }
            let rid = self.alloc_id();
            let mut rp = Vec::new();
            put_u32(&mut rp, rid);
            put_string(&mut rp, &handle);
            let (rty, body) = match self.request(FXP_READDIR, rid, &rp) {
                Ok(v) => v,
                Err(e) => {
                    let _ = self.close_handle(&handle);
                    return Err(e);
                }
            };
            if rty == FXP_STATUS {
                let mut cur = Cursor::new(&body);
                let _id = cur.u32()?;
                let code = cur.u32()?;
                if code == FX_EOF {
                    break;
                }
                let msg = cur.string().unwrap_or_default();
                let _ = self.close_handle(&handle);
                return Err(status_to_err(
                    code,
                    String::from_utf8_lossy(&msg).into_owned(),
                ));
            }
            if rty != FXP_NAME {
                let _ = self.close_handle(&handle);
                return Err(RemoteError::Protocol("esperava NAME no readdir".into()));
            }
            let names = match parse_name(&body) {
                Ok(n) => n,
                Err(e) => {
                    let _ = self.close_handle(&handle);
                    return Err(e);
                }
            };
            for entry in names {
                if entry.name == "." || entry.name == ".." {
                    continue;
                }
                entries.push(entry);
                if entries.len() >= cap {
                    break;
                }
            }
        }
        self.close_handle(&handle)?;
        Ok(entries)
    }

    /// Laço de READ até `len` bytes ou EOF. NÃO fecha o handle — quem abriu fecha
    /// em todo caminho (sucesso ou erro), então um corpo malformado aqui não
    /// vaza handle.
    fn read_all(&mut self, handle: &[u8], offset: u64, len: usize) -> RemoteResult<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        let mut pos = offset;
        while out.len() < len {
            let want = (len - out.len()).min(WRITE_CHUNK) as u32;
            let id = self.alloc_id();
            let mut p = Vec::new();
            put_u32(&mut p, id);
            put_string(&mut p, handle);
            put_u64(&mut p, pos);
            put_u32(&mut p, want);
            let (rty, body) = self.request(FXP_READ, id, &p)?;
            if rty == FXP_STATUS {
                let mut cur = Cursor::new(&body);
                let _id = cur.u32()?;
                let code = cur.u32()?;
                if code == FX_EOF {
                    break;
                }
                let msg = cur.string().unwrap_or_default();
                return Err(status_to_err(
                    code,
                    String::from_utf8_lossy(&msg).into_owned(),
                ));
            }
            if rty != FXP_DATA {
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
        Ok(out)
    }
}

pub struct SshRemote {
    alias: String,
    conn: Mutex<Conn<BufReader<ChildStdout>, ChildStdin>>,
}

impl SshRemote {
    /// Abre o subsistema SFTP na conexão SSH multiplexada existente (`ssh <alias>
    /// -s sftp` reusa o ControlMaster do ssh_config — zero handshake novo). Faz o
    /// INIT/VERSION do protocolo v3.
    pub fn connect(alias: &str) -> RemoteResult<Self> {
        let mut child = Command::new("ssh")
            .args(SSH_LIVENESS_OPTS)
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
            child: Some(child),
        };

        let mut init = Vec::new();
        put_u32(&mut init, SFTP_VERSION);
        conn.send(FXP_INIT, &init)?;
        let (ty, body) = conn.recv()?;
        if ty != FXP_VERSION {
            return Err(RemoteError::Protocol("esperava VERSION".into()));
        }
        check_version(&body)?;
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

/// Parseia a resposta NAME (usada por REALPATH e READDIR). A capacidade NUNCA é
/// pré-dimensionada pelo `count` do servidor (um `count` mentiroso de ~4 bilhões
/// abortaria o app no `alloc`); o teto real é o tamanho da frame (cada entrada
/// consome bytes, e a frame já é capada em MAX_FRAME).
fn parse_name(body: &[u8]) -> RemoteResult<Vec<RemoteDirEntry>> {
    let mut cur = Cursor::new(body);
    let _id = cur.u32()?;
    let count = cur.u32()?;
    let mut out = Vec::with_capacity((count as usize).min(READDIR_CAP));
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
        self.conn.lock().read_dir(path, READDIR_CAP)
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
        let result = conn.read_all(&handle, offset, len);
        let _ = conn.close_handle(&handle);
        result
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
            .args(SSH_LIVENESS_OPTS)
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
        if let Some(child) = conn.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
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

    /// Reader que entrega no máximo `step` bytes por chamada — simula o stream
    /// picando uma frame em pedaços (o caso real de rede).
    struct ChunkyReader {
        data: Vec<u8>,
        pos: usize,
        step: usize,
    }

    impl std::io::Read for ChunkyReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let n = self.step.min(buf.len()).min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    #[test]
    fn read_frame_reassembles_a_frame_delivered_one_byte_at_a_time() {
        let mut payload = Vec::new();
        put_u32(&mut payload, 7);
        put_string(&mut payload, b"hi");
        let mut framed = Vec::new();
        write_frame(&mut framed, FXP_STATUS, &payload).unwrap();

        let mut r = ChunkyReader {
            data: framed,
            pos: 0,
            step: 1,
        };
        let (ty, body) = read_frame(&mut r).unwrap();
        assert_eq!(ty, FXP_STATUS);
        assert_eq!(
            body, payload,
            "read_exact remonta a frame mesmo picada em 1 byte por leitura"
        );
    }

    #[test]
    fn read_frame_keeps_glued_frames_separate() {
        let mut pl1 = Vec::new();
        put_u32(&mut pl1, 1);
        let mut pl2 = Vec::new();
        put_u32(&mut pl2, 2);
        put_string(&mut pl2, b"payload dois");
        let mut framed = Vec::new();
        write_frame(&mut framed, FXP_STATUS, &pl1).unwrap();
        write_frame(&mut framed, FXP_DATA, &pl2).unwrap();

        let mut r = std::io::Cursor::new(framed);
        let (t1, b1) = read_frame(&mut r).unwrap();
        let (t2, b2) = read_frame(&mut r).unwrap();
        assert_eq!((t1, &b1), (FXP_STATUS, &pl1));
        assert_eq!(
            (t2, &b2),
            (FXP_DATA, &pl2),
            "a segunda frame colada não pode ser contaminada pela primeira"
        );
    }

    #[test]
    fn read_frame_rejects_a_lying_oversize_length_before_allocating() {
        let mut hdr = Vec::new();
        put_u32(&mut hdr, (MAX_FRAME + 1) as u32);
        let mut r = std::io::Cursor::new(hdr);
        match read_frame(&mut r) {
            Err(RemoteError::Protocol(_)) => {}
            other => panic!("length mentiroso acima do teto tem que ser recusado; veio {other:?}"),
        }
    }

    #[test]
    fn read_frame_rejects_a_zero_length_frame() {
        let mut hdr = Vec::new();
        put_u32(&mut hdr, 0);
        let mut r = std::io::Cursor::new(hdr);
        assert!(matches!(read_frame(&mut r), Err(RemoteError::Protocol(_))));
    }

    #[test]
    fn read_frame_on_a_truncated_body_reports_disconnected() {
        // length diz 100, mas só há 11 bytes no stream.
        let mut framed = Vec::new();
        put_u32(&mut framed, 100);
        framed.push(FXP_STATUS);
        framed.extend_from_slice(&[0u8; 10]);
        let mut r = std::io::Cursor::new(framed);
        assert_eq!(read_frame(&mut r), Err(RemoteError::Disconnected));
    }

    #[test]
    fn request_rejects_a_response_with_a_mismatched_id() {
        let mut resp = Vec::new();
        put_u32(&mut resp, 99);
        put_u32(&mut resp, FX_OK);
        let mut framed = Vec::new();
        write_frame(&mut framed, FXP_STATUS, &resp).unwrap();

        let mut conn = Conn {
            reader: std::io::Cursor::new(framed),
            writer: Vec::new(),
            next_id: 1,
            child: None,
        };
        match conn.request(FXP_STAT, 1, &[]) {
            Err(RemoteError::Protocol(_)) => {}
            other => panic!("resposta com id fora de ordem tem que ser recusada; veio {other:?}"),
        }
    }

    #[test]
    fn request_accepts_a_response_with_the_matching_id() {
        let mut resp = Vec::new();
        put_u32(&mut resp, 5);
        put_u32(&mut resp, FX_OK);
        let mut framed = Vec::new();
        write_frame(&mut framed, FXP_STATUS, &resp).unwrap();

        let mut conn = Conn {
            reader: std::io::Cursor::new(framed),
            writer: Vec::new(),
            next_id: 1,
            child: None,
        };
        let (ty, body) = conn.request(FXP_STAT, 5, &[]).unwrap();
        assert_eq!(ty, FXP_STATUS);
        let mut cur = Cursor::new(&body);
        assert_eq!(cur.u32().unwrap(), 5);
    }

    #[test]
    fn check_version_accepts_only_v3() {
        let mut v3 = Vec::new();
        put_u32(&mut v3, 3);
        assert_eq!(check_version(&v3).unwrap(), 3);
        for bad in [2u32, 4, 6] {
            let mut b = Vec::new();
            put_u32(&mut b, bad);
            assert!(
                check_version(&b).is_err(),
                "versão {bad} tem que ser recusada — só falamos v3"
            );
        }
    }

    #[test]
    fn parse_name_tolerates_nul_and_invalid_utf8_in_a_filename() {
        let mut body = Vec::new();
        put_u32(&mut body, 1);
        put_u32(&mut body, 1);
        put_string(&mut body, b"na\x00me\xffx.txt");
        put_string(&mut body, b"longname");
        body.extend_from_slice(&attrs_bytes(ATTR_PERMISSIONS, None, Some(0o100644)));

        let entries = parse_name(&body).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].name.contains('\u{0}'),
            "o NUL do nome hostil é preservado (lossy), sem panic"
        );
        assert!(entries[0].stat.is_file());
    }

    #[test]
    fn parse_attrs_errors_on_flags_that_lie_about_present_fields() {
        // Flag ATTR_SIZE ligada, mas sem os 8 bytes de size.
        let mut b = Vec::new();
        put_u32(&mut b, ATTR_SIZE);
        let mut cur = Cursor::new(&b);
        assert!(
            parse_attrs(&mut cur).is_err(),
            "flag que promete um campo ausente não pode ler lixo — erro de protocolo"
        );
    }

    #[test]
    fn parse_name_never_preallocates_by_a_giant_server_count() {
        // count = 0xFFFFFFFF mas corpo sem entradas: `with_capacity` é capado em
        // READDIR_CAP (não ~172 GB), e o parse erra ao esgotar o corpo. Rodar sem
        // abortar já prova que não houve alloc gigante.
        let mut body = Vec::new();
        put_u32(&mut body, 1);
        put_u32(&mut body, 0xFFFF_FFFF);
        assert!(parse_name(&body).is_err());
    }

    #[test]
    fn cursor_string_rejects_a_giant_length_without_overflowing() {
        // len declarado = 0xFFFFFFFF sobre um buffer minúsculo: `checked_add` no
        // bounds evita overflow em 32-bit; vira erro em vez de panic.
        let data = [0xffu8, 0xff, 0xff, 0xff, 0, 0];
        let mut cur = Cursor::new(&data);
        assert!(cur.string().is_err());
    }

    fn handle_frame(id: u32, handle: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        put_u32(&mut body, id);
        put_string(&mut body, handle);
        let mut framed = Vec::new();
        write_frame(&mut framed, FXP_HANDLE, &body).unwrap();
        framed
    }

    fn status_frame(id: u32, code: u32) -> Vec<u8> {
        let mut body = Vec::new();
        put_u32(&mut body, id);
        put_u32(&mut body, code);
        let mut framed = Vec::new();
        write_frame(&mut framed, FXP_STATUS, &body).unwrap();
        framed
    }

    fn name_frame(id: u32, names: &[&str]) -> Vec<u8> {
        let mut body = Vec::new();
        put_u32(&mut body, id);
        put_u32(&mut body, names.len() as u32);
        for n in names {
            put_string(&mut body, n.as_bytes());
            put_string(&mut body, b"longname");
            body.extend_from_slice(&attrs_bytes(ATTR_PERMISSIONS, None, Some(0o100644)));
        }
        let mut framed = Vec::new();
        write_frame(&mut framed, FXP_NAME, &body).unwrap();
        framed
    }

    #[test]
    fn read_dir_caps_the_accumulated_entries_at_the_given_cap() {
        // Um NAME hostil com mais entradas que o cap: o laço para no cap.
        let mut stream = Vec::new();
        stream.extend(handle_frame(1, b"H"));
        stream.extend(name_frame(2, &["a", "b", "c", "d", "e"]));
        stream.extend(status_frame(3, FX_OK));
        let mut conn = Conn {
            reader: std::io::Cursor::new(stream),
            writer: Vec::new(),
            next_id: 1,
            child: None,
        };
        let entries = conn.read_dir("/x", 3).unwrap();
        assert_eq!(entries.len(), 3, "o acumulado do readdir é capado em cap");
    }

    #[test]
    fn read_dir_errors_and_closes_the_handle_on_a_malformed_name() {
        let mut stream = Vec::new();
        stream.extend(handle_frame(1, b"H"));
        // NAME dizendo count=2 mas sem entradas → parse_name erra.
        let mut malformed = Vec::new();
        put_u32(&mut malformed, 2);
        put_u32(&mut malformed, 2);
        let mut nameframe = Vec::new();
        write_frame(&mut nameframe, FXP_NAME, &malformed).unwrap();
        stream.extend(nameframe);
        // Resposta do CLOSE que o read_dir dispara no caminho de erro.
        stream.extend(status_frame(3, FX_OK));

        let mut conn = Conn {
            reader: std::io::Cursor::new(stream),
            writer: Vec::new(),
            next_id: 1,
            child: None,
        };
        assert!(
            conn.read_dir("/x", 10).is_err(),
            "NAME malformado vira erro; o CLOSE (id 3) é consumido do stream, provando o fechamento"
        );
    }
}
