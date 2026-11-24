//! O fio entre o app e o daemon: um quadro = cabeçalho JSON + corpo em bytes.
//!
//! `[u32 BE tamanho do cabeçalho][u32 BE tamanho do corpo][cabeçalho][corpo]`.
//! O corpo leva o que é byte de terminal (saída, entrada, o anel no `attach`)
//! sem passar por JSON nem base64; o resto vai no cabeçalho.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Sobe quando um lado deixa de entender o outro. App e daemon de versões
/// diferentes não conversam: o `hello` recusa.
pub const PROTOCOL_VERSION: u32 = 1;

/// O env inteiro de um shell de login cabe com folga; um cabeçalho maior que
/// isto é lixo ou ataque.
pub const MAX_HEADER_BYTES: u32 = 1024 * 1024;
/// Uma colagem grande ou o anel inteiro de um `attach`.
pub const MAX_BODY_BYTES: u32 = 4 * 1024 * 1024;

/// Pedido do app. `seq` volta na resposta; eventos (`Output`, `Exit`) não têm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Hello {
        v: u32,
    },
    /// O id vem do app (é o id da sessão): a aba volta com o mesmo id.
    /// `env` é o env COMPLETO do processo; o do daemon nunca vaza para ele.
    Spawn {
        seq: u64,
        id: Uuid,
        argv: Vec<String>,
        env: Vec<(String, String)>,
        cwd: Option<String>,
        cols: u16,
        rows: u16,
    },
    /// Os bytes vão no corpo.
    Write {
        seq: u64,
        id: Uuid,
    },
    Resize {
        seq: u64,
        id: Uuid,
        cols: u16,
        rows: u16,
    },
    /// Mata o grupo de processos inteiro e esquece o PTY.
    Kill {
        seq: u64,
        id: Uuid,
    },
    List {
        seq: u64,
    },
    /// Responde com o anel no corpo e passa a mandar `Output` deste PTY.
    Attach {
        seq: u64,
        id: Uuid,
    },
    Detach {
        seq: u64,
        id: Uuid,
    },
    Query {
        seq: u64,
        id: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyInfo {
    pub id: Uuid,
    pub pid: Option<u32>,
    pub cols: u16,
    pub rows: u16,
    /// `None` enquanto vive; `Some(código)` depois que o processo saiu (o
    /// código é `None` quando o sistema não contou).
    pub exited: Option<Option<i32>>,
}

/// O que o daemon manda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum Reply {
    HelloOk {
        v: u32,
        pid: u32,
    },
    Ok {
        seq: u64,
    },
    Spawned {
        seq: u64,
        pid: Option<u32>,
    },
    Ptys {
        seq: u64,
        ptys: Vec<PtyInfo>,
    },
    /// O anel vai no corpo. Se o processo já saiu, `exited` diz com que código.
    Attached {
        seq: u64,
        id: Uuid,
        exited: Option<Option<i32>>,
    },
    State {
        seq: u64,
        foreground_group: Option<u32>,
        line_echo: Option<bool>,
    },
    Error {
        seq: u64,
        message: String,
    },
    /// Evento: bytes novos do PTY, no corpo.
    Output {
        id: Uuid,
    },
    /// Evento: o processo saiu.
    Exit {
        id: Uuid,
        code: Option<i32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame<T> {
    pub header: T,
    pub body: Vec<u8>,
}

impl<T> Frame<T> {
    pub fn new(header: T) -> Self {
        Frame {
            header,
            body: Vec::new(),
        }
    }

    pub fn with_body(header: T, body: Vec<u8>) -> Self {
        Frame { header, body }
    }
}

fn invalid(what: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, what.into())
}

pub fn write_frame<T: Serialize>(out: &mut impl Write, frame: &Frame<T>) -> std::io::Result<()> {
    let header = serde_json::to_vec(&frame.header).map_err(|e| invalid(e.to_string()))?;
    let header_len = u32::try_from(header.len())
        .ok()
        .filter(|n| *n <= MAX_HEADER_BYTES)
        .ok_or_else(|| invalid("cabeçalho grande demais"))?;
    let body_len = u32::try_from(frame.body.len())
        .ok()
        .filter(|n| *n <= MAX_BODY_BYTES)
        .ok_or_else(|| invalid("corpo grande demais"))?;
    let mut buf = Vec::with_capacity(8 + header.len() + frame.body.len());
    buf.extend_from_slice(&header_len.to_be_bytes());
    buf.extend_from_slice(&body_len.to_be_bytes());
    buf.extend_from_slice(&header);
    buf.extend_from_slice(&frame.body);
    out.write_all(&buf)?;
    out.flush()
}

/// `Ok(None)` no fim limpo do fluxo (o outro lado fechou entre quadros).
pub fn read_frame<T: for<'de> Deserialize<'de>>(
    input: &mut impl Read,
) -> std::io::Result<Option<Frame<T>>> {
    let mut lens = [0u8; 8];
    match input.read_exact(&mut lens[..1]) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    input.read_exact(&mut lens[1..])?;
    let header_len = u32::from_be_bytes([lens[0], lens[1], lens[2], lens[3]]);
    let body_len = u32::from_be_bytes([lens[4], lens[5], lens[6], lens[7]]);
    if header_len > MAX_HEADER_BYTES {
        return Err(invalid("cabeçalho grande demais"));
    }
    if body_len > MAX_BODY_BYTES {
        return Err(invalid("corpo grande demais"));
    }
    let mut header = vec![0u8; header_len as usize];
    input.read_exact(&mut header)?;
    let mut body = vec![0u8; body_len as usize];
    input.read_exact(&mut body)?;
    let header = serde_json::from_slice(&header).map_err(|e| invalid(e.to_string()))?;
    Ok(Some(Frame { header, body }))
}
