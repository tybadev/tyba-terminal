//! Parser do `~/.bash_history`.
//!
//! Formato conferido contra o bash real em 2026-08-15, com arquivo gerado só
//! para essa verificação — nunca com o histórico de alguém (`CLAUDE.md`).
//!
//! Mais simples que o zsh, e por dois motivos que só a medição mostra:
//!
//! 1. **Não metafica.** `echo olá ç` sai em UTF-8 cru, ao contrário do zsh.
//! 2. **Não usa `\` de continuação.** Com `cmdhist` ligado (o padrão), o bash
//!    junta o comando multilinha numa linha só, trocando as quebras por `;` —
//!    medido: `for i in 1 2; do   echo $i; done`. Tratar `\` final como
//!    continuação aqui corromperia comando que legitimamente termina em barra.
//!
//! A data só existe quando o usuário tem `HISTTIMEFORMAT` definido: aí o bash
//! grava uma linha `#<epoch>` antes de cada comando.
//!
//! > Limitação conhecida: com `shopt -s lithist` o bash grava a quebra literal e
//! > **não marca** a continuação. Nesse caso não há como distinguir de comandos
//! > separados, e cada linha vira uma entrada.

use std::io::{self, BufRead};

use super::super::ParsedEntry;

/// Entradas do arquivo. Linha de data não conta: ela pertence à entrada seguinte.
pub fn count<R: BufRead>(reader: R) -> io::Result<usize> {
    let mut entries = 0;
    for_each_record(reader, |record| {
        if timestamp(record).is_none() {
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
    let mut pending: Option<i64> = None;
    for_each_record(reader, |record| {
        if let Some(epoch) = timestamp(record) {
            pending = Some(epoch * 1_000);
            return;
        }
        let started_at_ms = pending
            .take()
            .unwrap_or_else(|| mtime_ms - (total.saturating_sub(index + 1) as i64) * 1_000);
        match String::from_utf8(record.to_vec()) {
            Ok(command) => on_entry(ParsedEntry {
                command,
                started_at_ms,
                duration_ms: None,
            }),
            Err(_) => discarded += 1,
        }
        index += 1;
    })?;
    Ok(discarded)
}

/// Uma linha do arquivo, sem o fim de linha. Linha vazia não é entrada.
fn for_each_record<R: BufRead>(mut reader: R, mut on_record: impl FnMut(&[u8])) -> io::Result<()> {
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
            on_record(&line);
        }
    }
    Ok(())
}

/// `#<epoch>` é marca de data; `# qualquer outra coisa` é comentário que o
/// usuário digitou, e comentário digitado é comando que foi para o histórico.
fn timestamp(record: &[u8]) -> Option<i64> {
    let digits = record.strip_prefix(b"#")?;
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
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

    /// Sem `HISTTIMEFORMAT` o arquivo é uma linha por comando, sem data.
    #[test]
    fn plain_format_dates_backwards_from_the_mtime() {
        let mtime = 10_000_000i64;
        let (found, discarded) = entries("primeiro\necho olá ç\nterceiro\n".as_bytes(), mtime);
        assert_eq!(discarded, 0);
        assert_eq!(found.len(), 3);
        assert_eq!(found[1].command, "echo olá ç");
        assert_eq!(found[0].started_at_ms, mtime - 2_000);
        assert_eq!(found[1].started_at_ms, mtime - 1_000);
        assert_eq!(found[2].started_at_ms, mtime);
        assert_eq!(found[0].duration_ms, None);
    }

    #[test]
    fn a_timestamp_line_dates_the_command_that_follows_it() {
        let (found, _) = entries(b"#1786820493\ncargo test\n", 0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "cargo test");
        assert_eq!(found[0].started_at_ms, 1_786_820_493_000);
    }

    /// O mesmo arquivo tem as duas formas: `HISTTIMEFORMAT` é definido em algum
    /// momento da vida do usuário, não desde sempre.
    #[test]
    fn a_file_with_and_without_timestamps_keeps_both() {
        let mtime = 10_000_000i64;
        let (found, _) = entries(b"antigo\n#1786820493\nnovo\n", mtime);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].command, "antigo");
        assert_eq!(found[0].started_at_ms, mtime - 1_000);
        assert_eq!(found[1].command, "novo");
        assert_eq!(found[1].started_at_ms, 1_786_820_493_000);
    }

    /// Comentário digitado no prompt vai para o histórico como comando.
    ///
    /// O caso do número negativo é o que exige a checagem de dígitos: sozinho,
    /// o `parse::<i64>` aceitaria `-123` e transformaria o comentário em data.
    #[test]
    fn a_comment_the_user_typed_is_a_command_not_a_timestamp() {
        let (found, _) = entries(b"# nota para depois\n#-123 nao e data\n#-123\n", 0);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].command, "# nota para depois");
        assert_eq!(found[1].command, "#-123 nao e data");
        assert_eq!(found[2].command, "#-123");
    }

    #[test]
    fn an_entry_that_is_not_utf8_is_discarded_not_fatal() {
        let mut source = b"bom\n".to_vec();
        source.extend_from_slice(&[0xff, 0xfe, b'\n']);
        source.extend_from_slice(b"tambem bom\n");
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

    /// Medido: o bash junta o multilinha com `;`, então a barra final é parte do
    /// comando e não pode virar continuação.
    #[test]
    fn a_trailing_backslash_is_part_of_the_command() {
        let (found, _) = entries(b"echo 'a\\'\npwd\n", 0);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].command, "echo 'a\\'");
        assert_eq!(found[1].command, "pwd");
    }
}
