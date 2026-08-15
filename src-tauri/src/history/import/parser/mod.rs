//! Um parser por formato de arquivo de histórico.
//!
//! São a parte frágil do import, e por isso são função pura sobre `BufRead`:
//! testáveis com fixture sintética, sem disco e sem shell instalado.

pub mod bash;
pub mod zsh;
