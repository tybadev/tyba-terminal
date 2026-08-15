//! Parser do `~/.zsh_history`.
//!
//! Formato conferido contra o zsh real em 2026-08-15, com arquivo gerado só
//! para essa verificação — nunca com o histórico de alguém (`CLAUDE.md`).
//!
//! Três coisas quebram parser ingênuo aqui:
//!
//! 1. **Formato estendido** (`EXTENDED_HISTORY`): `: <epoch>:<duração>;<comando>`.
//!    Sem a opção, a linha é o comando cru — e o mesmo arquivo pode ter as duas
//!    formas, porque a opção é ligada em algum momento da vida do usuário.
//! 2. **Comando multilinha**: a linha termina em `\` e continua na seguinte. A
//!    quebra faz parte do comando, não separa entradas.
//! 3. **Metafication**: o zsh grava certos bytes ≥ 0x80 como `0x83, b ^ 0x20`.
//!    Medido: `echo olá ç` sai como `6f 6c c3 83 81 20 c3 a7`. Sem desfazer, a
//!    entrada não decodifica em UTF-8 e some no descarte.

use std::io::{self, BufRead};

use super::super::ParsedEntry;

/// O byte de escape do zsh. O byte seguinte vale `b ^ 0x20`.
const META: u8 = 0x83;

/// Entradas do arquivo, sem decodificar nada.
///
/// Existe separada do `parse` porque a data sintetizada de entrada sem
/// timestamp é contada a partir do fim: é preciso saber quantas são antes de
/// datar a primeira.
pub fn count<R: BufRead>(reader: R) -> io::Result<usize> {
    let mut entries = 0;
    for_each_record(reader, |_| entries += 1)?;
    Ok(entries)
}

/// Lê o arquivo em fluxo e entrega cada entrada. Devolve quantas foram
/// descartadas por não decodificarem em UTF-8.
///
/// `total` vem do [`count`]: entrada sem timestamp é datada para trás a partir
/// do `mtime`, 1 s por entrada, preservando a ordem do arquivo. Datar tudo com
/// "agora" faria o importado atropelar o comando real de ontem; zerar jogaria
/// tudo para 1970, onde a recência o mata.
pub fn parse<R: BufRead>(
    reader: R,
    mtime_ms: i64,
    total: usize,
    on_entry: &mut dyn FnMut(ParsedEntry),
) -> io::Result<usize> {
    let mut index = 0usize;
    let mut discarded = 0usize;
    for_each_record(reader, |record| {
        let synthesized = mtime_ms - (total.saturating_sub(index + 1) as i64) * 1_000;
        match decode(record, synthesized) {
            Some(entry) => on_entry(entry),
            None => discarded += 1,
        }
        index += 1;
    })?;
    Ok(discarded)
}

/// Junta as linhas de uma entrada e entrega os bytes crus dela.
///
/// Um único lugar decide o que é entrada, para que `count` e `parse` nunca
/// discordem — se discordassem, a data sintetizada sairia deslocada.
fn for_each_record<R: BufRead>(mut reader: R, mut on_record: impl FnMut(&[u8])) -> io::Result<()> {
    let mut record: Vec<u8> = Vec::new();
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        while line.last() == Some(&b'\n') || line.last() == Some(&b'\r') {
            line.pop();
        }
        let continues = line.last() == Some(&b'\\');
        if continues {
            line.pop();
        }
        record.extend_from_slice(&line);
        if continues {
            record.push(b'\n');
            continue;
        }
        if !record.is_empty() {
            on_record(&record);
        }
        record.clear();
    }
    // Arquivo terminando em `\`: a última entrada não tem linha seguinte.
    if !record.is_empty() {
        on_record(&record);
    }
    Ok(())
}

/// `: <epoch>:<duração>;<comando>` ou o comando cru.
fn decode(record: &[u8], synthesized_ms: i64) -> Option<ParsedEntry> {
    let (started_at_ms, duration_ms, command) = match split_extended(record) {
        Some((epoch, elapsed, command)) => (epoch * 1_000, Some(elapsed * 1_000), command),
        None => (synthesized_ms, None, record),
    };
    let command = String::from_utf8(unmetafy(command)).ok()?;
    Some(ParsedEntry {
        command,
        started_at_ms,
        duration_ms,
    })
}

/// Quebra o cabeçalho estendido. Devolve `None` quando a entrada é comando cru,
/// inclusive quando o cabeçalho está corrompido — nesse caso a linha inteira
/// vira comando, que é melhor do que perdê-la.
fn split_extended(record: &[u8]) -> Option<(i64, i64, &[u8])> {
    let rest = record.strip_prefix(b": ")?;
    let colon = rest.iter().position(|b| *b == b':')?;
    let semicolon = rest.iter().position(|b| *b == b';')?;
    if semicolon < colon {
        return None;
    }
    let epoch = std::str::from_utf8(&rest[..colon]).ok()?.parse().ok()?;
    let elapsed = std::str::from_utf8(&rest[colon + 1..semicolon])
        .ok()?
        .parse()
        .ok()?;
    Some((epoch, elapsed, &rest[semicolon + 1..]))
}

/// Desfaz a metafication: `0x83 X` volta a ser `X ^ 0x20`.
fn unmetafy(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut iter = bytes.iter().copied();
    while let Some(byte) = iter.next() {
        if byte == META {
            match iter.next() {
                Some(escaped) => out.push(escaped ^ 0x20),
                // `0x83` solto no fim do arquivo: preserva o byte em vez de
                // engoli-lo, e o UTF-8 inválido cuida do descarte.
                None => out.push(byte),
            }
        } else {
            out.push(byte);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(source: &[u8], mtime_ms: i64) -> (Vec<ParsedEntry>, usize) {
        let total = count(source).unwrap();
        let mut out = Vec::new();
        let discarded = parse(source, mtime_ms, total, &mut |entry| out.push(entry)).unwrap();
        (out, discarded)
    }

    #[test]
    fn extended_format_carries_start_and_duration() {
        let (found, discarded) = entries(b": 1786820227:12;cargo test\n", 0);
        assert_eq!(discarded, 0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "cargo test");
        assert_eq!(found[0].started_at_ms, 1_786_820_227_000);
        assert_eq!(found[0].duration_ms, Some(12_000));
    }

    /// Sem `EXTENDED_HISTORY` o arquivo não tem data. Datar para trás a partir
    /// do `mtime` preserva a ordem sem fingir que tudo foi digitado agora.
    #[test]
    fn plain_format_dates_backwards_from_the_mtime() {
        let mtime = 10_000_000i64;
        let (found, _) = entries(b"primeiro\nsegundo\nterceiro\n", mtime);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].command, "primeiro");
        assert_eq!(found[0].started_at_ms, mtime - 2_000);
        assert_eq!(found[1].started_at_ms, mtime - 1_000);
        assert_eq!(found[2].started_at_ms, mtime);
        assert_eq!(found[0].duration_ms, None);
    }

    #[test]
    fn a_backslash_continues_the_same_command() {
        let source: &[u8] = b": 1:0;for i in 1 2; do\\\n  echo $i\\\ndone\n: 2:0;pwd\n";
        let (found, _) = entries(source, 0);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].command, "for i in 1 2; do\n  echo $i\ndone");
        assert_eq!(found[1].command, "pwd");
    }

    /// Medido contra o zsh real: `echo olá ç` sai metaficado.
    #[test]
    fn metafied_bytes_are_restored() {
        let mut source = b": 1:0;echo ol".to_vec();
        source.extend_from_slice(&[0xc3, META, 0x81, b' ', 0xc3, 0xa7, b'\n']);
        let (found, discarded) = entries(&source, 0);
        assert_eq!(discarded, 0);
        assert_eq!(found[0].command, "echo olá ç");
    }

    #[test]
    fn an_entry_that_is_not_utf8_is_discarded_not_fatal() {
        let mut source = b": 1:0;bom\n: 2:0;".to_vec();
        source.extend_from_slice(&[0xff, 0xfe, b'\n']);
        source.extend_from_slice(b": 3:0;tambem bom\n");
        let (found, discarded) = entries(&source, 0);
        assert_eq!(discarded, 1);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].command, "bom");
        assert_eq!(found[1].command, "tambem bom");
    }

    #[test]
    fn an_empty_file_has_no_entries() {
        let (found, discarded) = entries(b"", 0);
        assert!(found.is_empty());
        assert_eq!(discarded, 0);
    }

    /// `count` e `parse` precisam concordar, ou a data sintetizada sai deslocada.
    #[test]
    fn count_agrees_with_parse_on_multiline_entries() {
        let source: &[u8] = b"um\ndois\\\ncontinua\n\ntres\n";
        assert_eq!(count(source).unwrap(), 3);
        let (found, _) = entries(source, 0);
        assert_eq!(found.len(), 3);
        assert_eq!(found[1].command, "dois\ncontinua");
    }
}
