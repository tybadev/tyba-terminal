//! Import do histórico que o usuário já tem no shell.
//!
//! O motor de frecência nasce sem dado: quem instala o TYBA tem anos em
//! `~/.zsh_history` e abre uma paleta vazia. Aqui esse histórico entra.

pub mod parser;
pub mod source;

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
