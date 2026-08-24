//! Parser do `fish_history`.
//!
//! Formato conferido contra o fish 4.8.1 em 2026-08-15, com arquivo gerado só
//! para essa verificação — nunca com o histórico de alguém (`CLAUDE.md`).
//!
//! Parece YAML e não é: o fish tem escritor e leitor próprios. Medido:
//!
//! ```text
//! - cmd: echo ola
//!   when: 1786820677
//! - cmd: for i in 1 2\n  echo $i\nend
//!   when: 1786820677
//!   paths:
//!     - /algum/caminho
//! ```
//!
//! O que isso decide no parser:
//!
//! 1. **Uma entrada nunca ocupa mais de uma linha de `cmd`.** Comando multilinha
//!    vem com `\n` **literal** (dois caracteres), não com a quebra.
//! 2. **A barra invertida é escapada como `\\`.** Só essas duas sequências
//!    existem; qualquer outra `\X` é texto do comando e fica como está.
//! 3. **Não usar YAML de verdade.** `- cmd: echo com: dois pontos` é entrada
//!    válida e derrubaria um parser YAML estrito; aqui o que vale é o prefixo.
//! 4. **Não metafica**, ao contrário do zsh: UTF-8 cru.

use std::io::{self, BufRead};

use super::super::ParsedEntry;

const CMD: &[u8] = b"- cmd: ";
const WHEN: &[u8] = b"when: ";

/// Entradas do arquivo: uma por linha `- cmd: `.
pub fn count<R: BufRead>(reader: R) -> io::Result<usize> {
    let mut entries = 0;
    for_each_line(reader, |line| {
        if line.starts_with(CMD) {
            entries += 1;
        }
    })?;
    Ok(entries)
}

/// Lê o arquivo em fluxo e entrega cada entrada. Devolve quantas foram
/// descartadas por não decodificarem em UTF-8.
pub fn parse<R: BufRead>(
    reader: R,
    mtime_ms: i64,
    total: usize,
    on_entry: &mut dyn FnMut(ParsedEntry),
) -> io::Result<usize> {
    let mut index = 0usize;
    let mut discarded = 0usize;
    let mut pending: Option<Vec<u8>> = None;
    let mut when: Option<i64> = None;

    let mut flush = |command: Option<Vec<u8>>, when: Option<i64>, index: &mut usize| {
        let Some(command) = command else {
            return;
        };
        let started_at_ms =
            when.unwrap_or_else(|| mtime_ms - (total.saturating_sub(*index + 1) as i64) * 1_000);
        match String::from_utf8(unescape(&command)) {
            Ok(command) => on_entry(ParsedEntry {
                command,
                started_at_ms,
                duration_ms: None,
            }),
            Err(_) => discarded += 1,
        }
        *index += 1;
    };

    for_each_line(reader, |line| {
        if let Some(command) = line.strip_prefix(CMD) {
            flush(pending.take(), when.take(), &mut index);
            pending = Some(command.to_vec());
            return;
        }
        if pending.is_some() && when.is_none() {
            if let Some(epoch) = line.strip_prefix(b"  ").and_then(|f| f.strip_prefix(WHEN)) {
                when = std::str::from_utf8(epoch)
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .map(|seconds| seconds * 1_000);
            }
        }
        // `paths:` e o que mais o fish escreva ficam de fora: o import guarda
        // comando, e caminho de arquivo tocado não é o cwd de onde ele rodou.
    })?;
    flush(pending, when, &mut index);
    Ok(discarded)
}

fn for_each_line<R: BufRead>(mut reader: R, mut on_line: impl FnMut(&[u8])) -> io::Result<()> {
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        while line.last() == Some(&b'\n') || line.last() == Some(&b'\r') {
            line.pop();
        }
        if !line.is_empty() {
            on_line(&line);
        }
    }
    Ok(())
}

/// Desfaz o escape do fish: `\\` vira `\` e `\n` vira quebra de linha. Qualquer
/// outra sequência é texto do comando e passa inteira.
fn unescape(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                    continue;
                }
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(bytes[i]);
        i += 1;
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
    fn a_record_carries_the_command_and_its_date() {
        let (found, discarded) = entries("- cmd: echo olá ç\n  when: 1786820677\n".as_bytes(), 0);
        assert_eq!(discarded, 0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "echo olá ç");
        assert_eq!(found[0].started_at_ms, 1_786_820_677_000);
        assert_eq!(found[0].duration_ms, None);
    }

    /// Medido: o multilinha vem com `\n` literal, não com a quebra.
    #[test]
    fn an_escaped_newline_becomes_a_real_line_break() {
        let (found, _) = entries(
            b"- cmd: for i in 1 2\\n  echo $i\\nend\n  when: 1\n- cmd: pwd\n  when: 2\n",
            0,
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].command, "for i in 1 2\n  echo $i\nend");
        assert_eq!(found[1].command, "pwd");
    }

    #[test]
    fn an_escaped_backslash_becomes_one_backslash() {
        let (found, _) = entries(b"- cmd: echo barra \\\\ invertida\n  when: 1\n", 0);
        assert_eq!(found[0].command, "echo barra \\ invertida");
    }

    /// `- cmd: echo com: dois pontos` derrubaria um parser YAML estrito.
    #[test]
    fn a_colon_inside_the_command_is_kept() {
        let (found, _) = entries(b"- cmd: echo com: dois pontos\n  when: 1\n", 0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "echo com: dois pontos");
    }

    #[test]
    fn a_record_without_a_date_is_dated_backwards_from_the_mtime() {
        let mtime = 10_000_000i64;
        let (found, _) = entries(b"- cmd: primeiro\n- cmd: segundo\n", mtime);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].started_at_ms, mtime - 1_000);
        assert_eq!(found[1].started_at_ms, mtime);
    }

    #[test]
    fn the_paths_block_is_ignored() {
        let (found, _) = entries(
            b"- cmd: cat a.txt\n  when: 1\n  paths:\n    - /tmp/a.txt\n- cmd: pwd\n  when: 2\n",
            0,
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].command, "cat a.txt");
        assert_eq!(found[1].command, "pwd");
    }

    #[test]
    fn an_entry_that_is_not_utf8_is_discarded_not_fatal() {
        let mut source = b"- cmd: bom\n  when: 1\n- cmd: ".to_vec();
        source.extend_from_slice(&[0xff, 0xfe, b'\n']);
        source.extend_from_slice(b"  when: 2\n- cmd: tambem bom\n  when: 3\n");
        let (found, discarded) = entries(&source, 0);
        assert_eq!(discarded, 1);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].command, "tambem bom");
    }

    #[test]
    fn an_empty_file_has_no_entries() {
        let (found, discarded) = entries(b"", 0);
        assert!(found.is_empty());
        assert_eq!(discarded, 0);
    }
}
