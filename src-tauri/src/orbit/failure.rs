//! Por que um processo do Orbit caiu, numa linha: o rodapé do card falho
//! ("saiu com 1 · porta 8765 em uso por Python (pid 17369)") e o motivo que
//! o `orbit_create_process` devolve ao agente.
//!
//! O reconhecimento é puro (texto do log, comando e título → [`Diagnosis`]);
//! quem usa a porta sai de [`port_owner`], que roda `lsof`/`netstat` com teto
//! de tempo curto e nunca na main thread (quem chama é o fim do PTY ou o
//! handler das ferramentas).

use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;

/// Teto do motivo "última linha do log", em caracteres.
pub const REASON_MAX_CHARS: usize = 160;
/// Linhas do fim do log que se olham para achar o erro.
const DIAGNOSE_LINES: usize = 40;
/// Teto da consulta de quem usa a porta.
const OWNER_TIMEOUT: Duration = Duration::from_millis(1500);
const OWNER_MAX_BYTES: usize = 64 * 1024;

/// O que o log diz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diagnosis {
    /// Porta ocupada; a porta, quando a mensagem, o comando ou o título dizem.
    PortInUse(Option<u16>),
    /// Outro erro: a última linha não vazia, cortada.
    LastLine(String),
}

/// Quem escuta na porta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortOwner {
    pub pid: u32,
    pub name: String,
}

/// Mensagens de porta ocupada: Python (`[Errno 48|98] Address already in
/// use`), Node (`listen EADDRINUSE`), Vite (`Port 5173 is already in use`),
/// Go (`bind: address already in use`), Bun (`Is port 3000 in use?`), Windows
/// (`Only one usage of each socket address`).
static PORT_IN_USE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)address already in use|eaddrinuse|port\s+(?:\d{1,5}\s+)?is\s+already\s+in\s+use|is\s+port\s+\d{1,5}\s+in\s+use|only one usage of each socket address",
    )
    .expect("regex de porta em uso")
});
/// `port 5173`, `--port=5173`, `-p 5173`, `PORT=5173`.
static PORT_WORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:\bport|--port|\s-p)[\s=:]*(\d{1,5})\b").expect("regex de porta")
});
/// `127.0.0.1:8080`, `:::3000`, `(:8765)`.
static PORT_COLON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r":(\d{2,5})\b").expect("regex de :porta"));

fn port_number(s: &str) -> Option<u16> {
    s.parse::<u16>().ok().filter(|p| *p > 0)
}

/// A porta num texto: palavra (`port 3000`, `--port 3000`), `:3000`, ou um
/// número solto a partir de 1024 (`http.server 8765`) quando `bare`.
fn port_in(text: &str, bare: bool) -> Option<u16> {
    let all = |re: &'static Regex| {
        re.captures_iter(text)
            .filter_map(|c| port_number(c.get(1)?.as_str()))
    };
    // De `:porta`, a última: a hora no começo da linha (`20:05:01`) não é.
    all(&PORT_WORD)
        .next()
        .or_else(|| all(&PORT_COLON).last())
        .or_else(|| {
            bare.then(|| {
                text.split(|c: char| c.is_whitespace() || "()[]'\"=,".contains(c))
                    .filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()))
                    // Número solto só de 1024 para cima: `exit 1`, `sleep 30`
                    // e `[Errno 48]` não são porta.
                    .find_map(|t| port_number(t).filter(|p| *p >= 1024))
            })
            .flatten()
        })
}

/// O que o fim do log diz, com a porta tirada da linha do erro, do comando ou
/// do título. Log vazio: `None`.
pub fn diagnose(log: &str, command: &str, title: &str) -> Option<Diagnosis> {
    if let Some(line) = log.lines().rev().find(|l| PORT_IN_USE.is_match(l)) {
        let port = port_in(line, false)
            .or_else(|| port_in(command, true))
            .or_else(|| port_in(title, true));
        return Some(Diagnosis::PortInUse(port));
    }
    let last = log.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    Some(Diagnosis::LastLine(cut(last, REASON_MAX_CHARS)))
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// O motivo em pt-BR: "porta 8765 em uso por Python (pid 17369)".
pub fn reason(d: &Diagnosis, owner: Option<&PortOwner>) -> String {
    match d {
        Diagnosis::PortInUse(port) => {
            let mut s = match port {
                Some(p) => format!("porta {p} em uso"),
                None => "porta em uso".to_string(),
            };
            if let Some(o) = owner {
                s.push_str(&format!(" por {} (pid {})", o.name, o.pid));
            }
            s
        }
        Diagnosis::LastLine(line) => line.clone(),
    }
}

/// O motivo da queda lendo o `log.txt` (o fim, sem cor e redigido) e, em
/// porta ocupada, quem a usa. Bloqueia até o teto do `lsof`: fora da main
/// thread.
pub fn failure_reason(log_path: &Path, command: &str, title: &str) -> Option<String> {
    let log = super::orchestrator::log_tail(log_path, DIAGNOSE_LINES)?;
    let d = diagnose(&log, command, title)?;
    let owner = match &d {
        Diagnosis::PortInUse(Some(port)) => port_owner(*port),
        _ => None,
    };
    Some(reason(&d, owner.as_ref()))
}

/// Saída de `lsof -F pc`: o primeiro processo (`p<pid>`, `c<nome>`).
pub fn parse_lsof_owner(raw: &str) -> Option<PortOwner> {
    let mut pid = None;
    for line in raw.lines() {
        if let Some(v) = line.strip_prefix('p') {
            if pid.is_some() {
                break;
            }
            pid = v.trim().parse::<u32>().ok().filter(|p| *p > 0);
        } else if let (Some(p), Some(name)) = (pid, line.strip_prefix('c')) {
            return Some(PortOwner {
                pid: p,
                name: clean_name(name),
            });
        }
    }
    pid.map(|pid| PortOwner {
        pid,
        name: "?".into(),
    })
}

/// Nome de processo vindo de fora: sem caractere de controle, curto.
fn clean_name(name: &str) -> String {
    let s: String = name.trim().chars().filter(|c| !c.is_control()).collect();
    if s.is_empty() {
        "?".into()
    } else {
        cut(&s, 40)
    }
}

/// Saída de `netstat -ano`: o PID de quem está em LISTENING na porta.
pub fn parse_netstat_pid(raw: &str, port: u16) -> Option<u32> {
    let suffix = format!(":{port}");
    raw.lines().find_map(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        // Proto  Local  Remoto  Estado  PID
        if cols.len() < 5 || !cols[0].eq_ignore_ascii_case("tcp") {
            return None;
        }
        if !cols[1].ends_with(&suffix) || !cols[3].eq_ignore_ascii_case("listening") {
            return None;
        }
        cols[4].parse::<u32>().ok().filter(|p| *p > 0)
    })
}

/// Saída de `tasklist /FO CSV /NH`: `"python.exe","1234",...` → o nome.
pub fn parse_tasklist_name(raw: &str) -> Option<String> {
    let first = raw.lines().find(|l| l.starts_with('"'))?;
    let name = first.trim_start_matches('"').split('"').next()?;
    (!name.is_empty()).then(|| clean_name(name))
}

/// Quem escuta na porta, ou `None` (sem ferramenta, sem permissão, teto).
#[cfg(unix)]
pub fn port_owner(port: u16) -> Option<PortOwner> {
    let filter = format!("-iTCP:{port}");
    ["/usr/sbin/lsof", "/usr/bin/lsof"]
        .iter()
        .map(Path::new)
        .filter(|p| p.exists())
        .find_map(|bin| {
            let out = crate::services::run_bounded_within(
                bin,
                &["-nP", &filter, "-sTCP:LISTEN", "-F", "pc"],
                OWNER_MAX_BYTES,
                OWNER_TIMEOUT,
            )
            .ok()?;
            parse_lsof_owner(&String::from_utf8_lossy(&out))
        })
}

#[cfg(windows)]
pub fn port_owner(port: u16) -> Option<PortOwner> {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let system = Path::new(&root).join("System32");
    let out = crate::services::run_bounded_within(
        &system.join("netstat.exe"),
        &["-ano", "-p", "TCP"],
        OWNER_MAX_BYTES * 4,
        OWNER_TIMEOUT,
    )
    .ok()?;
    let pid = parse_netstat_pid(&String::from_utf8_lossy(&out), port)?;
    let filter = format!("PID eq {pid}");
    let name = crate::services::run_bounded_within(
        &system.join("tasklist.exe"),
        &["/FI", &filter, "/FO", "CSV", "/NH"],
        OWNER_MAX_BYTES,
        OWNER_TIMEOUT,
    )
    .ok()
    .and_then(|out| parse_tasklist_name(&String::from_utf8_lossy(&out)))
    .unwrap_or_else(|| "?".into());
    Some(PortOwner { pid, name })
}

#[cfg(not(any(unix, windows)))]
pub fn port_owner(_port: u16) -> Option<PortOwner> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mensagens escritas à mão no formato de cada ferramenta.
    const PYTHON: &str = "Traceback (most recent call last):\n  File \"server.py\", line 136, in server_bind\n    self.socket.bind(self.server_address)\nOSError: [Errno 48] Address already in use\n";
    const NODE: &str = "node:events:497\n      throw er; // Unhandled 'error' event\n      ^\n\nError: listen EADDRINUSE: address already in use :::3000\n    at Server.setupListenHandle [as _listen2] (node:net:1872:16)\n    at listenInCluster (node:net:1920:12)\n";
    const VITE: &str = "  VITE v5.4.0  ready\nerror when starting dev server:\nError: Port 5173 is already in use\n    at Server.onError (file:///app/node_modules/vite/dist/node/chunks/dep.js:1:1)\n";
    const GO: &str = "2026/10/16 20:05:01 listen tcp 127.0.0.1:8080: bind: address already in use\nexit status 1\n";

    #[test]
    fn recognizes_port_in_use_from_python_node_vite_and_go() {
        assert_eq!(
            diagnose(PYTHON, "python3 -m http.server 8765 --bind 127.0.0.1", "x"),
            Some(Diagnosis::PortInUse(Some(8765)))
        );
        assert_eq!(
            diagnose(NODE, "node server.js", "api"),
            Some(Diagnosis::PortInUse(Some(3000)))
        );
        assert_eq!(
            diagnose(VITE, "bun run dev", "web"),
            Some(Diagnosis::PortInUse(Some(5173)))
        );
        assert_eq!(
            diagnose(GO, "go run .", "api"),
            Some(Diagnosis::PortInUse(Some(8080)))
        );
        assert_eq!(
            diagnose("Errno 98 Address already in use", "", ""),
            Some(Diagnosis::PortInUse(None))
        );
        assert_eq!(
            diagnose(
                "error: Failed to start server. Is port 3001 in use?",
                "bun x",
                ""
            ),
            Some(Diagnosis::PortInUse(Some(3001)))
        );
    }

    #[test]
    fn port_comes_from_the_message_then_the_command_then_the_title() {
        // A mensagem do Python não diz a porta: vem do comando.
        assert_eq!(
            port_in("python3 -m http.server 8765 --bind 127.0.0.1", true),
            Some(8765)
        );
        assert_eq!(port_in("bun run dev --port 4000", true), Some(4000));
        assert_eq!(port_in("PORT=3002 node index.js", true), Some(3002));
        assert_eq!(port_in("php -S localhost:8000", true), Some(8000));
        assert_eq!(port_in("python3 -m http.server", true), None);
        assert_eq!(port_in("sleep 30; exit 1", true), None);
        // Sem nada no comando: o título ("resumo muriki (:8765)").
        assert_eq!(
            diagnose(PYTHON, "python3 -m http.server", "resumo muriki (:8765)"),
            Some(Diagnosis::PortInUse(Some(8765)))
        );
        // A porta da mensagem ganha da do comando.
        assert_eq!(
            diagnose(NODE, "node server.js --port 9999", ""),
            Some(Diagnosis::PortInUse(Some(3000)))
        );
        // `[Errno 48]` e o endereço sem porta não viram porta.
        assert_eq!(
            port_in("OSError: [Errno 48] Address already in use", false),
            None
        );
    }

    #[test]
    fn other_errors_give_the_last_non_empty_line_cut() {
        assert_eq!(
            diagnose(
                "compilando\nModuleNotFoundError: No module named 'flask'\n\n  \n",
                "",
                ""
            ),
            Some(Diagnosis::LastLine(
                "ModuleNotFoundError: No module named 'flask'".into()
            ))
        );
        let long = "x".repeat(400);
        let Some(Diagnosis::LastLine(line)) = diagnose(&long, "", "") else {
            panic!("sem linha");
        };
        assert_eq!(line.chars().count(), REASON_MAX_CHARS);
        assert!(line.ends_with('…'));
        assert_eq!(diagnose("\n \n", "", ""), None);
    }

    #[test]
    fn reason_names_port_and_owner() {
        let owner = PortOwner {
            pid: 17369,
            name: "Python".into(),
        };
        assert_eq!(
            reason(&Diagnosis::PortInUse(Some(8765)), Some(&owner)),
            "porta 8765 em uso por Python (pid 17369)"
        );
        assert_eq!(
            reason(&Diagnosis::PortInUse(Some(8765)), None),
            "porta 8765 em uso"
        );
        assert_eq!(reason(&Diagnosis::PortInUse(None), None), "porta em uso");
    }

    #[test]
    fn parses_lsof_netstat_and_tasklist() {
        assert_eq!(
            parse_lsof_owner("p17369\ncPython\nf4\np999\ncnode\n"),
            Some(PortOwner {
                pid: 17369,
                name: "Python".into()
            })
        );
        assert_eq!(parse_lsof_owner(""), None);
        let netstat = "\r\nActive Connections\r\n\r\n  Proto  Local Address          Foreign Address        State           PID\r\n  TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1012\r\n  TCP    127.0.0.1:8765         0.0.0.0:0              LISTENING       4242\r\n  TCP    127.0.0.1:18765        0.0.0.0:0              LISTENING       7\r\n";
        assert_eq!(parse_netstat_pid(netstat, 8765), Some(4242));
        assert_eq!(parse_netstat_pid(netstat, 9000), None);
        assert_eq!(
            parse_tasklist_name("\"python.exe\",\"4242\",\"Console\",\"1\",\"12,345 K\"\r\n"),
            Some("python.exe".into())
        );
        assert_eq!(parse_tasklist_name("INFO: No tasks are running"), None);
    }

    #[test]
    fn failure_reason_reads_the_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.txt");
        std::fs::write(&path, "subindo\n\x1b[31mError: boom\x1b[0m\n").unwrap();
        assert_eq!(
            failure_reason(&path, "", "").as_deref(),
            Some("Error: boom")
        );
        assert_eq!(failure_reason(&dir.path().join("nada"), "", ""), None);
    }
}
