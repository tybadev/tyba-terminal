//! Portas do servidor numa sessão SSH: só leitura.
//!
//! Um `ss -ltnp` pela conexão multiplexada do Host (o canal de
//! `ssh::query`), com `head -c` no servidor e teto de novo do lado de cá
//! (`exec_bounded`: bytes e tempo) — o servidor não é confiável. O que volta
//! é texto de terceiro: nomes e endereços saem sem caractere de controle nem
//! de formatação (bidi), com teto de tamanho, e a lista tem teto de portas.

use serde::Serialize;

/// Teto da saída lida do servidor.
pub const MAX_BYTES: usize = 256 * 1024;

/// Teto de portas e de endereços por porta que viram linha na tela.
const MAX_PORTS: usize = 4096;
const MAX_ADDRESSES: usize = 16;

/// Script fixo, sem nada vindo da UI. `127` = o servidor não tem `ss`; `3` =
/// o `ss` falhou. Sem `-H` (o iproute2 antigo não tem): o cabeçalho não
/// começa com `LISTEN` e o parser o pula.
pub fn script() -> String {
    format!(
        "command -v ss >/dev/null 2>&1 || exit 127; out=$(ss -ltnp 2>/dev/null) || exit 3; printf '%s\\n' \"$out\" | head -c {MAX_BYTES}"
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
    raw.chars()
        .filter(|c| !super::cmdline::is_hidden_char(*c))
        .take(64)
        .collect()
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
    Some((clean_name(host), port))
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
        let full = out.len() >= MAX_PORTS;
        match out.iter_mut().find(|p| p.port == port) {
            Some(entry) => {
                if entry.addresses.len() < MAX_ADDRESSES && !entry.addresses.contains(&address) {
                    entry.addresses.push(address);
                }
                if entry.process.is_none() {
                    entry.process = process;
                    entry.pid = pid;
                }
            }
            None if full => {}
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

/// Porta local livre para o túnel: nada escuta nela (pelo retrato local) e
/// o bind de teste passa.
pub fn free_locally(
    port: u16,
    busy: &std::collections::HashSet<u16>,
    bind_ok: impl Fn(u16) -> bool,
) -> bool {
    !busy.contains(&port) && bind_ok(port)
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
    fn header_of_old_ss_is_skipped_and_text_is_cleaned() {
        let raw = "State  Recv-Q Send-Q Local Address:Port Peer Address:Port Process\n\
LISTEN 0 1 10.0.0.\u{202e}5:8080 *:* users:((\"a\u{2066}b\",pid=5,fd=1))\n";
        let ports = parse_ss(raw.as_bytes());
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].addresses, vec!["10.0.0.5".to_string()]);
        assert_eq!(ports[0].process.as_deref(), Some("ab"));
    }

    #[test]
    fn port_and_address_lists_are_capped() {
        let mut raw = String::new();
        for i in 0..30 {
            raw.push_str(&format!("LISTEN 0 1 10.0.0.{i}:80 *:*\n"));
        }
        assert_eq!(parse_ss(raw.as_bytes())[0].addresses.len(), MAX_ADDRESSES);
        let long = format!("LISTEN 0 1 {}:81 *:*\n", "a".repeat(500));
        assert_eq!(parse_ss(long.as_bytes())[0].addresses[0].len(), 64);
    }

    #[test]
    fn script_has_no_header_flag_and_signals_failure() {
        let s = script();
        assert!(!s.contains("-H"));
        assert!(s.contains("exit 3"));
    }

    #[test]
    fn a_port_someone_listens_on_is_never_free() {
        let busy = std::collections::HashSet::from([5432u16]);
        assert!(!free_locally(5432, &busy, |_| true));
        assert!(free_locally(5433, &busy, |_| true));
        assert!(!free_locally(5433, &busy, |_| false));
        assert_eq!(
            pick_local_port(5432, |p| free_locally(p, &busy, |_| true)),
            Some(5433)
        );
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
