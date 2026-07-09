//! StatusDetector (Fase 4).
//!
//! Dois modos, por confiabilidade (docs/ARCHITECTURE.md):
//! 1. Estruturado: eventos stream-json do runner (preferido).
//! 2. Heurístico: OSC 133 (A=prompt, C=executando, D=terminou) +
//!    timeout de silêncio com frame final em padrão de pergunta.
//!
//! Este módulo implementa o parser OSC incremental sobre o stream do PTY.
//! É a parte frágil (bytes podem chegar partidos entre chunks), então tem
//! testes obrigatórios (convenção do repo).

use base64::Engine;

/// Eventos de shell integration extraídos do stream do PTY.
///
/// OSC 133 marca o ciclo prompt→comando→fim (semantic prompt marks);
/// OSC 633;E carrega a linha de comando (base64) emitida pelo hook do TYBA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEvent {
    /// `OSC 133 ; A` — prompt começou (shell ocioso, aguardando input).
    PromptStart,
    /// `OSC 133 ; C` — comando começou a executar.
    CommandStart,
    /// `OSC 633 ; E ; <base64>` — texto da linha de comando.
    CommandLine(String),
    /// `OSC 133 ; D [ ; <code> ]` — comando terminou (exit code).
    CommandEnd(i32),
}

const MAX_OSC_LEN: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Text,
    Esc,
    Osc,
    OscEsc,
}

/// Parser incremental de sequências OSC (`ESC ] … ST`) focado em OSC 133/633.
/// Ignora todo o resto do stream. Tolera sequências partidas entre chunks.
pub struct OscParser {
    state: State,
    buf: Vec<u8>,
}

impl Default for OscParser {
    fn default() -> Self {
        Self::new()
    }
}

impl OscParser {
    pub fn new() -> Self {
        Self {
            state: State::Text,
            buf: Vec::new(),
        }
    }

    /// Alimenta um chunk de bytes e devolve os eventos completos encontrados.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<ShellEvent> {
        let mut events = Vec::new();
        for &b in bytes {
            match self.state {
                State::Text => {
                    if b == 0x1b {
                        self.state = State::Esc;
                    }
                }
                State::Esc => {
                    if b == b']' {
                        self.state = State::Osc;
                        self.buf.clear();
                    } else {
                        // ESC seguido de outra coisa: não é OSC.
                        self.state = State::Text;
                    }
                }
                State::Osc => {
                    match b {
                        0x07 => {
                            // BEL termina o OSC.
                            self.finish(&mut events);
                            self.state = State::Text;
                        }
                        0x1b => self.state = State::OscEsc,
                        _ => {
                            if self.buf.len() < MAX_OSC_LEN {
                                self.buf.push(b);
                            } else {
                                // Sequência absurda: aborta pra não crescer sem limite.
                                self.state = State::Text;
                                self.buf.clear();
                            }
                        }
                    }
                }
                State::OscEsc => {
                    // Esperando `\` do ST (ESC \). Qualquer outra coisa aborta.
                    if b == b'\\' {
                        self.finish(&mut events);
                    }
                    self.state = State::Text;
                }
            }
        }
        events
    }

    fn finish(&mut self, events: &mut Vec<ShellEvent>) {
        if let Some(ev) = parse_osc(&self.buf) {
            events.push(ev);
        }
        self.buf.clear();
    }
}

fn parse_osc(payload: &[u8]) -> Option<ShellEvent> {
    let text = std::str::from_utf8(payload).ok()?;
    let mut parts = text.split(';');
    let code = parts.next()?;
    match code {
        "133" => match parts.next()? {
            "A" | "B" => Some(ShellEvent::PromptStart),
            "C" => Some(ShellEvent::CommandStart),
            "D" => {
                let exit = parts
                    .next()
                    .and_then(|c| c.parse::<i32>().ok())
                    .unwrap_or(0);
                Some(ShellEvent::CommandEnd(exit))
            }
            _ => None,
        },
        "633" => {
            if parts.next()? != "E" {
                return None;
            }
            let encoded = parts.next()?;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()?;
            let cmd = String::from_utf8_lossy(&decoded).trim().to_string();
            if cmd.is_empty() {
                None
            } else {
                Some(ShellEvent::CommandLine(cmd))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    #[test]
    fn parses_prompt_command_end_cycle() {
        let mut p = OscParser::new();
        let stream = format!(
            "\x1b]133;A\x07some prompt $ \x1b]633;E;{}\x07\x1b]133;C\x07output\n\x1b]133;D;0\x07",
            b64("cargo test")
        );
        let events = p.feed(stream.as_bytes());
        assert_eq!(
            events,
            vec![
                ShellEvent::PromptStart,
                ShellEvent::CommandLine("cargo test".into()),
                ShellEvent::CommandStart,
                ShellEvent::CommandEnd(0),
            ]
        );
    }

    #[test]
    fn handles_st_terminator_esc_backslash() {
        let mut p = OscParser::new();
        let events = p.feed(b"\x1b]133;C\x1b\\");
        assert_eq!(events, vec![ShellEvent::CommandStart]);
    }

    #[test]
    fn tolerates_sequence_split_across_chunks() {
        let mut p = OscParser::new();
        let cmd = b64("npm run dev");
        let full = format!("\x1b]633;E;{cmd}\x07");
        let bytes = full.as_bytes();
        let mut events = Vec::new();
        for byte in bytes {
            events.extend(p.feed(&[*byte]));
        }
        assert_eq!(events, vec![ShellEvent::CommandLine("npm run dev".into())]);
    }

    #[test]
    fn command_end_without_code_defaults_zero() {
        let mut p = OscParser::new();
        assert_eq!(p.feed(b"\x1b]133;D\x07"), vec![ShellEvent::CommandEnd(0)]);
    }

    #[test]
    fn command_end_with_nonzero_code() {
        let mut p = OscParser::new();
        assert_eq!(
            p.feed(b"\x1b]133;D;130\x07"),
            vec![ShellEvent::CommandEnd(130)]
        );
    }

    #[test]
    fn ignores_unrelated_osc_and_plain_text() {
        let mut p = OscParser::new();
        // OSC 0 (title), OSC 7 (cwd) e texto puro são ignorados.
        let events = p.feed(b"\x1b]0;my title\x07plain text\x1b]7;file:///tmp\x07");
        assert!(events.is_empty());
    }

    #[test]
    fn ignores_invalid_base64_command_line() {
        let mut p = OscParser::new();
        assert!(p.feed(b"\x1b]633;E;not_valid_base64!!!\x07").is_empty());
    }

    #[test]
    fn caps_runaway_sequence() {
        let mut p = OscParser::new();
        let mut huge = b"\x1b]133;".to_vec();
        huge.extend(std::iter::repeat(b'x').take(MAX_OSC_LEN + 100));
        huge.extend_from_slice(b"\x07");
        // Não deve entrar em pânico nem crescer sem limite; nenhum evento válido.
        assert!(p.feed(&huge).is_empty());
        // Parser volta a funcionar depois.
        assert_eq!(p.feed(b"\x1b]133;C\x07"), vec![ShellEvent::CommandStart]);
    }
}
