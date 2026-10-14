//! Portas do servidor numa sessão SSH: só leitura.
//!
//! Um `ss -Hltnp` pela conexão multiplexada do Host (o canal de
//! `ssh::query`), com `head -c` no servidor — a saída é limitada na origem.
//! O que volta é texto de terceiro: nomes de processo saem sem caractere de
//! controle e com teto de tamanho.

use serde::Serialize;

/// Teto da saída lida do servidor.
pub const MAX_BYTES: usize = 256 * 1024;

/// Script fixo, sem nada vindo da UI. `127` = o servidor não tem `ss`.
pub fn script() -> String {
    format!(
        "command -v ss >/dev/null 2>&1 || exit 127; ss -Hltnp 2>/dev/null | head -c {MAX_BYTES}"
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemotePort {
    pub port: u16,
    pub addresses: Vec<String>,
    /// O nome do processo, quando o `ss` pôde ver (os do próprio usuário).
    pub process: Option<String>,
    pub pid: Option<u32>,
}

fn clean_name(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).take(64).collect()
}

/// `users:(("node",pid=123,fd=20),...)` → ("node", 123).
fn parse_users(field: &str) -> (Option<String>, Option<u32>) {
    let Some(rest) = field.strip_prefix("users:((\"") else {
        return (None, None);
    };
    let Some((name, tail)) = rest.split_once('"') else {
        return (None, None);
    };
    let pid = tail
        .split(',')
        .find_map(|p| p.strip_prefix("pid="))
        .and_then(|p| p.trim_end_matches(')').parse().ok());
    (Some(clean_name(name)), pid)
}

/// Endereço local do `ss`: `127.0.0.1:5432`, `[::]:22`, `*:80`,
/// `127.0.0.53%lo:53`.
fn split_local(field: &str) -> Option<(String, u16)> {
    let (host, port) = field.rsplit_once(':')?;
    let port: u16 = port.parse().ok().filter(|p| *p > 0)?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let host = host.split('%').next().unwrap_or(host);
    let host = match host {
        "0.0.0.0" | "::" | "*" | "" => "*",
        other => other,
    };
    Some((host.to_string(), port))
}

/// Linhas de `ss -Hltnp`: `State Recv-Q Send-Q Local Peer [Process]`.
/// Uma entrada por porta, com os endereços juntos; ordenado por porta.
pub fn parse_ss(raw: &[u8]) -> Vec<RemotePort> {
    let text = String::from_utf8_lossy(raw);
    let mut out: Vec<RemotePort> = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.first() != Some(&"LISTEN") {
            continue;
        }
        let Some((address, port)) = fields.get(3).and_then(|f| split_local(f)) else {
            continue;
        };
        let (process, pid) = fields
            .get(5)
            .map(|f| parse_users(f))
            .unwrap_or((None, None));
        match out.iter_mut().find(|p| p.port == port) {
            Some(entry) => {
                if !entry.addresses.contains(&address) {
                    entry.addresses.push(address);
                }
                if entry.process.is_none() {
                    entry.process = process;
                    entry.pid = pid;
                }
            }
            None => out.push(RemotePort {
                port,
                addresses: vec![address],
                process,
                pid,
            }),
        }
    }
    out.sort_by_key(|p| p.port);
    out
}

/// A porta local para "Trazer para cá": a mesma do servidor se estiver livre,
/// senão a primeira livre logo acima (até 20 tentativas).
pub fn pick_local_port(remote: u16, is_free: impl Fn(u16) -> bool) -> Option<u16> {
    (0..=20u16)
        .filter_map(|i| remote.checked_add(i))
        .find(|p| is_free(*p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ss_lines_become_ports_with_process() {
        let raw = b"LISTEN 0      4096       127.0.0.1:5432      0.0.0.0:*    users:((\"postgres\",pid=812,fd=6))\n\
LISTEN 0      511          0.0.0.0:3000      0.0.0.0:*    users:((\"node\",pid=901,fd=20),(\"node\",pid=902,fd=20))\n\
LISTEN 0      511             [::]:3000         [::]:*    users:((\"node\",pid=901,fd=21))\n\
LISTEN 0      128             [::]:22           [::]:*\n\
LISTEN 0      4096   127.0.0.53%lo:53        0.0.0.0:*\n";
        let ports = parse_ss(raw);
        assert_eq!(
            ports,
            vec![
                RemotePort {
                    port: 22,
                    addresses: vec!["*".into()],
                    process: None,
                    pid: None
                },
                RemotePort {
                    port: 53,
                    addresses: vec!["127.0.0.53".into()],
                    process: None,
                    pid: None
                },
                RemotePort {
                    port: 3000,
                    addresses: vec!["*".into()],
                    process: Some("node".into()),
                    pid: Some(901)
                },
                RemotePort {
                    port: 5432,
                    addresses: vec!["127.0.0.1".into()],
                    process: Some("postgres".into()),
                    pid: Some(812)
                },
            ]
        );
    }

    #[test]
    fn ss_garbage_and_other_states_are_ignored() {
        let raw = b"State Recv-Q\nESTAB 0 0 10.0.0.1:22 10.0.0.2:5000\nLISTEN 0 1 nada\nLISTEN 0 1 x:0 y\n";
        assert!(parse_ss(raw).is_empty());
    }

    #[test]
    fn process_names_from_the_server_are_cleaned() {
        let raw = b"LISTEN 0 1 *:80 *:* users:((\"ng\x1b[31minx\",pid=5,fd=1))\n";
        assert_eq!(parse_ss(raw)[0].process.as_deref(), Some("ng[31minx"));
    }

    #[test]
    fn local_port_prefers_the_same_number() {
        assert_eq!(pick_local_port(5432, |_| true), Some(5432));
        assert_eq!(
            pick_local_port(5432, |p| p != 5432 && p != 5433),
            Some(5434)
        );
        assert_eq!(pick_local_port(5432, |_| false), None);
        assert_eq!(pick_local_port(65535, |p| p != 65535), None);
    }
}
