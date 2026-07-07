//! StatusDetector (Fase 4).
//!
//! Dois modos, por confiabilidade (docs/ARCHITECTURE.md):
//! 1. Estruturado: eventos stream-json do runner (preferido).
//! 2. Heurístico: OSC 133 (A=prompt, C=executando, D=terminou) +
//!    timeout de silêncio com frame final em padrão de pergunta.

// TODO(fase 4): parser OSC 133 incremental sobre o stream do PTY.
// TODO(fase 4): heurística de AwaitingInput (silêncio + `? `, `[y/n]`, `❯`).
