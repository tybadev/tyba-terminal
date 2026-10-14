//! O que o `ps` sabe de cada processo e o `process_probe` não: grupo, dono,
//! CPU, memória e a linha de comando (que só sai daqui já redigida).
//!
//! Um `ps -A` por varredura, por caminho absoluto, com `LC_ALL=C` (o `pcpu`
//! sai com vírgula em pt-BR) e saída limitada.

#[derive(Debug, Clone, PartialEq)]
pub struct PsRow {
    pub pid: u32,
    pub ppid: u32,
    pub pgid: u32,
    pub uid: u32,
    pub cpu: f32,
    pub rss_kb: u64,
    /// Crua: quem mostra passa por `cmdline::redact_command` antes.
    pub args: String,
}

/// Corta o próximo campo separado por espaço e devolve o resto intacto
/// (a linha de comando, o último campo, tem espaços de verdade).
fn next_field<'a>(rest: &mut &'a str) -> Option<&'a str> {
    let s = rest.trim_start();
    if s.is_empty() {
        return None;
    }
    let end = s.find(char::is_whitespace).unwrap_or(s.len());
    let (field, tail) = s.split_at(end);
    *rest = tail;
    Some(field)
}

/// Linhas de `ps -o pid=,ppid=,pgid=,uid=,pcpu=,rss=,args=`. Linha que não
/// fecha os seis números é ignorada.
pub fn parse_ps(raw: &str) -> Vec<PsRow> {
    raw.lines()
        .filter_map(|line| {
            let mut rest = line;
            let pid = next_field(&mut rest)?.parse().ok()?;
            let ppid = next_field(&mut rest)?.parse().ok()?;
            let pgid = next_field(&mut rest)?.parse().ok()?;
            let uid = next_field(&mut rest)?.parse().ok()?;
            let cpu = next_field(&mut rest)?.parse().ok()?;
            let rss_kb = next_field(&mut rest)?.parse().ok()?;
            Some(PsRow {
                pid,
                ppid,
                pgid,
                uid,
                cpu,
                rss_kb,
                args: rest.trim().to_string(),
            })
        })
        .collect()
}

#[cfg(unix)]
const PS_CANDIDATES: &[&str] = &["/bin/ps", "/usr/bin/ps"];
/// Algumas centenas de processos com argumentos longos cabem com folga.
#[cfg(unix)]
const PS_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Todos os processos visíveis. Nada vindo da UI vira argumento.
#[cfg(unix)]
pub fn all() -> Result<Vec<PsRow>, crate::error::AppError> {
    let bin = PS_CANDIDATES
        .iter()
        .map(std::path::Path::new)
        .find(|p| p.is_file())
        .ok_or_else(|| crate::error::AppError::new("services.ps_missing"))?;
    let out = super::run_bounded(
        bin,
        &["-A", "-ww", "-o", "pid=,ppid=,pgid=,uid=,pcpu=,rss=,args="],
        PS_MAX_BYTES,
    )?;
    Ok(parse_ps(&String::from_utf8_lossy(&out)))
}

#[cfg(not(unix))]
pub fn all() -> Result<Vec<PsRow>, crate::error::AppError> {
    Err(crate::error::AppError::new("services.unsupported"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_columns_and_keeps_spaces_in_args() {
        let raw =
            "  101     1   101   501   2.5  184320 /usr/local/bin/node  dist/api.js --port 8080\n\
                   202   101   101     0   0.0    1024 /usr/sbin/sshd -D\n";
        let rows = parse_ps(raw);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].pid, 101);
        assert_eq!(rows[0].ppid, 1);
        assert_eq!(rows[0].pgid, 101);
        assert_eq!(rows[0].uid, 501);
        assert!((rows[0].cpu - 2.5).abs() < f32::EPSILON);
        assert_eq!(rows[0].rss_kb, 184320);
        assert_eq!(rows[0].args, "/usr/local/bin/node  dist/api.js --port 8080");
        assert_eq!(rows[1].uid, 0);
    }

    #[test]
    fn process_without_args_has_empty_command() {
        let rows = parse_ps("7 1 7 501 0.0 10\n");
        assert_eq!(rows[0].args, "");
    }

    #[test]
    fn broken_lines_are_skipped() {
        let raw = "\nabc 1 1 1 0 0 x\n12 1 1\n12 1 1 501 x,5 10 bad cpu\n13 1 13 501 0.1 20 ok\n";
        let rows = parse_ps(raw);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pid, 13);
    }
}
