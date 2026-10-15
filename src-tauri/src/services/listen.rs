//! Quem está escutando: portas TCP em LISTEN e o PID dono de cada uma.
//!
//! macOS: `lsof -nP -iTCP -sTCP:LISTEN -F pcn` por caminho absoluto, saída
//! limitada. Linux: `/proc/net/tcp{,6}` (estado `0A`) e o inode do socket
//! casado com `/proc/<pid>/fd`. Os dois parsers são puros e testados com
//! entradas escritas à mão.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listener {
    pub pid: u32,
    pub port: u16,
    /// Endereço do bind: `*` (todas as interfaces), `127.0.0.1`, `::1`...
    pub address: String,
}

/// `host:porta` do `lsof -n -P` → (endereço, porta). IPv6 vem entre
/// colchetes; `*` é o bind em todas as interfaces. Conexão (`a->b`) não é
/// escuta e fica de fora.
fn split_address(name: &str) -> Option<(String, u16)> {
    if name.contains("->") {
        return None;
    }
    let (host, port) = name.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    if port == 0 {
        return None;
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.is_empty() {
        return None;
    }
    Some((normalize_unspecified(host), port))
}

fn normalize_unspecified(host: &str) -> String {
    match host {
        "0.0.0.0" | "::" | "*" => "*".to_string(),
        other => other.to_string(),
    }
}

/// Saída de `lsof -F pcn`: uma linha por campo, a primeira letra é o campo.
/// `p` abre o conjunto do processo; `f` abre cada arquivo; `n` é o nome
/// (`*:3000`, `127.0.0.1:5432`, `[::1]:5432`). Linha que não se entende é
/// ignorada, nunca derruba a lista.
pub fn parse_lsof(raw: &[u8]) -> Vec<Listener> {
    let text = String::from_utf8_lossy(raw);
    let mut pid: Option<u32> = None;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for line in text.lines() {
        let mut chars = line.chars();
        let Some(field) = chars.next() else {
            continue;
        };
        let value = chars.as_str();
        match field {
            'p' => pid = value.parse().ok().filter(|p| *p > 0),
            'n' => {
                let (Some(pid), Some((address, port))) = (pid, split_address(value)) else {
                    continue;
                };
                if seen.insert((pid, port, address.clone())) {
                    out.push(Listener { pid, port, address });
                }
            }
            _ => {}
        }
    }
    out
}

/// Um socket em LISTEN de `/proc/net/tcp{,6}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcSocket {
    pub address: String,
    pub port: u16,
    pub inode: u64,
}

/// O endereço de `/proc/net/tcp`: hex de 32 bits na ordem da máquina
/// (little-endian). IPv6 são quatro palavras assim.
fn decode_hex_addr(hex: &str) -> Option<String> {
    let word = |s: &str| u32::from_str_radix(s, 16).ok().map(u32::to_le_bytes);
    match hex.len() {
        8 => {
            let ip = std::net::Ipv4Addr::from(word(hex)?);
            Some(normalize_unspecified(&ip.to_string()))
        }
        32 => {
            let mut bytes = [0u8; 16];
            for i in 0..4 {
                bytes[i * 4..i * 4 + 4].copy_from_slice(&word(&hex[i * 8..i * 8 + 8])?);
            }
            let ip = std::net::Ipv6Addr::from(bytes);
            Some(normalize_unspecified(&ip.to_string()))
        }
        _ => None,
    }
}

/// Linhas de `/proc/net/tcp` ou `tcp6` em estado `0A` (LISTEN). A primeira
/// linha é cabeçalho; os campos são posicionais: [1] endereço local, [3]
/// estado, [9] inode.
pub fn parse_proc_net_tcp(text: &str) -> Vec<ProcSocket> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.get(3) != Some(&"0A") {
                return None;
            }
            let (addr, port) = fields.get(1)?.split_once(':')?;
            let port = u16::from_str_radix(port, 16).ok().filter(|p| *p > 0)?;
            let inode: u64 = fields.get(9)?.parse().ok().filter(|i| *i > 0)?;
            Some(ProcSocket {
                address: decode_hex_addr(addr)?,
                port,
                inode,
            })
        })
        .collect()
}

/// `socket:[12345]` → 12345.
pub fn socket_inode(link: &str) -> Option<u64> {
    link.strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

/// Casa sockets com donos. `owners` é (pid, inode) de cada fd de socket.
pub fn join_sockets(sockets: &[ProcSocket], owners: &HashMap<u64, u32>) -> Vec<Listener> {
    let mut seen = HashSet::new();
    sockets
        .iter()
        .filter_map(|s| {
            let pid = *owners.get(&s.inode)?;
            seen.insert((pid, s.port, s.address.clone()))
                .then(|| Listener {
                    pid,
                    port: s.port,
                    address: s.address.clone(),
                })
        })
        .collect()
}

#[cfg(target_os = "macos")]
const LSOF: &str = "/usr/sbin/lsof";
/// Teto da saída do lsof: uma máquina normal tem dezenas de linhas.
#[cfg(target_os = "macos")]
const LSOF_MAX_BYTES: usize = 1024 * 1024;

#[cfg(target_os = "macos")]
pub fn listeners() -> Result<Vec<Listener>, crate::error::AppError> {
    let out = super::run_bounded(
        std::path::Path::new(LSOF),
        &["-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pcn"],
        LSOF_MAX_BYTES,
    )?;
    // `lsof` sai com 1 quando não acha nada: a saída vazia é a resposta.
    Ok(parse_lsof(&out))
}

#[cfg(target_os = "linux")]
pub fn listeners() -> Result<Vec<Listener>, crate::error::AppError> {
    let mut sockets = Vec::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            sockets.extend(parse_proc_net_tcp(&text));
        }
    }
    if sockets.is_empty() {
        return Ok(Vec::new());
    }
    let wanted: HashSet<u64> = sockets.iter().map(|s| s.inode).collect();
    let mut owners: HashMap<u64, u32> = HashMap::new();
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return Ok(Vec::new());
    };
    'procs: for entry in procs.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        // Só os processos que o usuário pode ler: os de outro usuário ficam de
        // fora (e não são encerráveis de qualquer jeito).
        let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(link) = std::fs::read_link(fd.path()) else {
                continue;
            };
            if let Some(inode) = link.to_str().and_then(socket_inode) {
                if wanted.contains(&inode) {
                    owners.entry(inode).or_insert(pid);
                    if owners.len() == wanted.len() {
                        break 'procs;
                    }
                }
            }
        }
    }
    Ok(join_sockets(&sockets, &owners))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn listeners() -> Result<Vec<Listener>, crate::error::AppError> {
    Err(crate::error::AppError::new("services.unsupported"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(pid: u32, port: u16, address: &str) -> Listener {
        Listener {
            pid,
            port,
            address: address.into(),
        }
    }

    #[test]
    fn lsof_fields_group_names_under_their_process() {
        let raw =
            b"p101\ncnode\nf20\nn*:3000\nf21\nn[::1]:3000\np202\ncpostgres\nf5\nn127.0.0.1:5432\n";
        assert_eq!(
            parse_lsof(raw),
            vec![
                l(101, 3000, "*"),
                l(101, 3000, "::1"),
                l(202, 5432, "127.0.0.1")
            ]
        );
    }

    #[test]
    fn lsof_duplicates_from_ipv4_and_ipv6_wildcards_collapse() {
        // O mesmo `*:7000` aparece em dois fds (v4 e v6): uma linha só.
        let raw = b"p9\ncx\nf10\nn*:7000\nf11\nn*:7000\n";
        assert_eq!(parse_lsof(raw), vec![l(9, 7000, "*")]);
    }

    #[test]
    fn lsof_ignores_garbage_connections_and_names_before_any_pid() {
        let raw = b"n*:1\n\nzzz\np0\nn*:2\np7\nn10.0.0.1:55000->10.0.0.2:443\nn*:notaport\nn*:0\nnsemporta\nn*:8080\n";
        assert_eq!(parse_lsof(raw), vec![l(7, 8080, "*")]);
    }

    #[test]
    fn lsof_comm_with_odd_characters_does_not_shift_fields() {
        // O comm vem inteiro na linha `c`: espaço ou dois-pontos não viram nome.
        let raw = b"p55\ncMy App: helper\nf3\nn127.0.0.1:9229\n";
        assert_eq!(parse_lsof(raw), vec![l(55, 9229, "127.0.0.1")]);
    }

    #[test]
    fn proc_net_tcp_keeps_only_listen_and_decodes_addresses() {
        let text = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
   0: 0100007F:0CEA 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4001 1 0000000000000000 100 0 0 10 0\n\
   1: 00000000:0BB8 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 4002 1 0000000000000000 100 0 0 10 0\n\
   2: 0100007F:0BB8 0100007F:D431 01 00000000:00000000 00:00000000 00000000  1000        0 4003 1 0000000000000000 20 4 30 10 -1\n";
        assert_eq!(
            parse_proc_net_tcp(text),
            vec![
                ProcSocket {
                    address: "127.0.0.1".into(),
                    port: 3306,
                    inode: 4001
                },
                ProcSocket {
                    address: "*".into(),
                    port: 3000,
                    inode: 4002
                },
            ]
        );
    }

    #[test]
    fn proc_net_tcp6_decodes_loopback_and_wildcard() {
        let text = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
   0: 00000000000000000000000001000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 5001 1 0000000000000000 100 0 0 10 0\n\
   1: 00000000000000000000000000000000:0016 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 5002 1 0000000000000000 100 0 0 10 0\n";
        let parsed = parse_proc_net_tcp(text);
        assert_eq!(parsed[0].address, "::1");
        assert_eq!(parsed[0].port, 8080);
        assert_eq!(parsed[1].address, "*");
        assert_eq!(parsed[1].port, 22);
    }

    #[test]
    fn proc_net_tcp_survives_truncated_lines() {
        let text = "header\n   0: 0100007F:0CEA\n   1: zz:zz 0 0A\n";
        assert!(parse_proc_net_tcp(text).is_empty());
    }

    #[test]
    fn socket_links_and_join() {
        assert_eq!(socket_inode("socket:[123]"), Some(123));
        assert_eq!(socket_inode("pipe:[123]"), None);
        assert_eq!(socket_inode("socket:[x]"), None);
        let sockets = vec![
            ProcSocket {
                address: "*".into(),
                port: 3000,
                inode: 1,
            },
            ProcSocket {
                address: "*".into(),
                port: 22,
                inode: 2,
            },
        ];
        let owners = HashMap::from([(1, 42)]);
        // Socket sem dono legível (outro usuário) fica de fora.
        assert_eq!(join_sockets(&sockets, &owners), vec![l(42, 3000, "*")]);
    }
}
