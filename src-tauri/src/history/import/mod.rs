//! Import do histórico que o usuário já tem no shell.
//!
//! O motor de frecência nasce sem dado: quem instala o TYBA tem anos em
//! `~/.zsh_history` e abre uma paleta vazia. Aqui esse histórico entra.

pub mod parser;
pub mod source;

use sha2::{Digest, Sha256};

use source::ImportSource;

/// Uma entrada lida de arquivo de histórico, antes de virar linha no banco.
///
/// `exit_code` não aparece: nenhuma das fontes de texto grava código, e nulo é
/// o que a frecência trata como desconhecido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEntry {
    pub command: String,
    pub started_at_ms: i64,
    pub duration_ms: Option<i64>,
}

/// Uma entrada pronta para gravar: comando já redigido e chave já calculada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRow {
    pub command: String,
    pub started_at_ms: i64,
    pub duration_ms: Option<i64>,
    pub import_key: String,
}

/// A chave que impede a duplicata ao reimportar.
///
/// **A posição da entrada no arquivo fica de fora, de propósito.** Incluí-la
/// faria o reimport duplicar tudo depois que o zsh apara o arquivo por
/// `SAVEHIST`, porque as posições andam. Deixá-la de fora custa perder uma
/// segunda ocorrência idêntica no mesmo segundo — uma unidade de contagem em um
/// comando que já vai aparecer. Perder contagem é barato; duplicar o corpus
/// corrompe o ranking inteiro.
///
/// O comando entra **já redigido**: a chave precisa casar com o que está no
/// banco, e o que está no banco passou pela redação.
pub fn import_key(source: ImportSource, redacted_command: &str, started_at_ms: i64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(redacted_command.as_bytes());
    hasher.update([0x1f]);
    hasher.update(started_at_ms.to_string().as_bytes());
    format!("{}:{:x}", source.key(), hasher.finalize())
}
