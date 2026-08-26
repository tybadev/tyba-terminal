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

pub mod agent_events;
pub mod subagent_transcript;
pub mod transcript;

pub mod manifest;
pub mod observed_notify;
pub mod observer;
pub mod registry;
pub mod screen;

use base64::Engine;

/// Eventos de shell integration extraídos do stream do PTY.
///
/// OSC 133 marca o ciclo prompt→comando→fim (semantic prompt marks);
/// OSC 633;E carrega a linha de comando (base64) emitida pelo hook do TYBA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEvent {
    /// `OSC 133 ; A` — prompt começou (shell ocioso, aguardando input).
    PromptStart,
    /// `OSC 133 ; B` — fim do prompt, começo da zona de input do usuário.
    InputStart,
    /// `OSC 133 ; C` — comando começou a executar.
    CommandStart,
    /// `OSC 633 ; E ; <base64>` — texto da linha de comando.
    CommandLine(String),
    /// `OSC 633 ; P ; tyba-prompt=<0|1>` — o shell confirma se está no modo
    /// prompt do TYBA. É a resposta do hook, não um palpite do app: sem ela o
    /// front não sabe se o `PS1` saiu mesmo da tela.
    PromptMode(bool),
    /// `OSC 633 ; P ; tyba-path=<valor>` — o `$PATH` **efetivo** da sessão.
    ///
    /// Não é o que o core passou no spawn: `nvm`, `asdf` e `direnv` reescrevem
    /// o `PATH` dentro do rc, depois do spawn, e são exatamente os shims que
    /// importam. Quem varre os diretórios continua sendo o core — o shell só
    /// diz QUAIS são.
    ShellPath(String),
    /// `OSC 633 ; P ; tyba-commands=<lote>` — os comandos que só o shell
    /// conhece: alias, função e builtin.
    ///
    /// Vem em LOTES, e o core faz a união — o payload não cabe numa sequência
    /// só (1302 funções numa máquina real contra `MAX_OSC_LEN`). O conteúdo é
    /// atacante-controlável como qualquer OSC; quem valida cada nome é
    /// `completion::binary::parse_batch`, e o resultado é display-only.
    ShellCommands(String),
    /// `OSC 133 ; D [ ; <code> ]` — comando terminou (exit code).
    CommandEnd(i32),
    /// `OSC 7 ; file://<host><path>` — diretório de trabalho.
    ///
    /// Atacante-controlável: qualquer processo emite. Display-only.
    Cwd(std::path::PathBuf),
}

/// Teto de uma sequência OSC. `pub(crate)` porque o hook do shell precisa
/// caber aqui dentro, e um teste prende os dois juntos: quem baixar este número
/// sem olhar o emissor descobre pelo teste, não por uma lista que sumiu.
pub(crate) const MAX_OSC_LEN: usize = 8 * 1024;

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

fn parse_cwd(rest: &str) -> Option<ShellEvent> {
    let after = rest.strip_prefix("file://")?;
    let slash = after.find('/')?;
    let decoded = percent_encoding::percent_decode_str(&after[slash..])
        .decode_utf8()
        .ok()?;
    if decoded.chars().any(|c| c.is_control()) {
        return None;
    }
    let path = std::path::PathBuf::from(decoded.as_ref());
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(ShellEvent::Cwd(path))
    }
}

fn parse_osc(payload: &[u8]) -> Option<ShellEvent> {
    let text = std::str::from_utf8(payload).ok()?;
    let (code, rest) = text.split_once(';')?;
    if code == "7" {
        return parse_cwd(rest);
    }
    let mut parts = rest.split(';');
    match code {
        "133" => match parts.next()? {
            "A" => Some(ShellEvent::PromptStart),
            "B" => Some(ShellEvent::InputStart),
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
            let sub = parts.next()?;
            if sub == "P" {
                let value = parts.next()?;
                if let Some(path) = value.strip_prefix("tyba-path=") {
                    return (!path.is_empty()).then(|| ShellEvent::ShellPath(path.to_string()));
                }
                if let Some(batch) = value.strip_prefix("tyba-commands=") {
                    // Lote vazio não vira evento: o shell não tem o que contar e
                    // acordar o core para uma lista vazia é trabalho por nada.
                    return (!batch.is_empty())
                        .then(|| ShellEvent::ShellCommands(batch.to_string()));
                }
                let value = value.strip_prefix("tyba-prompt=")?;
                return match value {
                    "1" => Some(ShellEvent::PromptMode(true)),
                    "0" => Some(ShellEvent::PromptMode(false)),
                    _ => None,
                };
            }
            if sub != "E" {
                return None;
            }
            let encoded = parts.next()?;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()?;
            // Só o fim é aparado: o espaço à ESQUERDA é sinal, não sujeira —
            // é a convenção `ignorespace` ("não guarde este comando"), e o
            // histórico precisa vê-la. Quem quer o comando limpo (matcher de
            // agente, UI) chama `trim_start` no consumo.
            let cmd = String::from_utf8_lossy(&decoded).trim_end().to_string();
            if cmd.trim().is_empty() {
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
    fn keeps_leading_space_and_drops_trailing_noise() {
        let mut p = OscParser::new();
        let events = p.feed(format!("\x1b]633;E;{}\x07", b64(" secret-cmd  \n")).as_bytes());
        assert_eq!(events, vec![ShellEvent::CommandLine(" secret-cmd".into())]);
    }

    #[test]
    fn parses_prompt_mode_report_from_the_hook() {
        let mut p = OscParser::new();
        assert_eq!(
            p.feed(b"\x1b]633;P;tyba-prompt=1\x07"),
            vec![ShellEvent::PromptMode(true)]
        );
        assert_eq!(
            p.feed(b"\x1b]633;P;tyba-prompt=0\x07"),
            vec![ShellEvent::PromptMode(false)]
        );
    }

    #[test]
    fn carries_the_command_batch_from_the_hook() {
        let mut p = OscParser::new();
        assert_eq!(
            p.feed(b"\x1b]633;P;tyba-commands=alias:gst,builtin:cd\x07"),
            vec![ShellEvent::ShellCommands("alias:gst,builtin:cd".into())]
        );
    }

    #[test]
    fn the_command_batch_survives_arriving_in_pieces() {
        // O lote tem ~1500 bytes e o chunk do PTY não tem tamanho garantido:
        // chegar partido é o caso comum, não o azar. Este é o teste obrigatório
        // do parser (convenção do repo).
        let mut p = OscParser::new();
        assert!(p.feed(b"\x1b]633;P;tyba-comm").is_empty());
        assert!(p.feed(b"ands=alias:gst,fun").is_empty());
        assert_eq!(
            p.feed(b"ction:mkcd\x07"),
            vec![ShellEvent::ShellCommands("alias:gst,function:mkcd".into())]
        );
    }

    #[test]
    fn carries_the_effective_path_from_the_hook() {
        // O `PATH` que o core passou no spawn não é o que vale: `nvm`, `asdf` e
        // `direnv` o reescrevem DENTRO do rc, depois do spawn. Quem sabe o
        // efetivo é o shell.
        let mut p = OscParser::new();
        assert_eq!(
            p.feed(b"\x1b]633;P;tyba-path=/opt/shims:/usr/bin\x07"),
            vec![ShellEvent::ShellPath("/opt/shims:/usr/bin".into())]
        );
    }

    #[test]
    fn ignores_other_633_properties() {
        // O `633;P` do VS Code carrega várias chaves; só a nossa vira evento.
        let mut p = OscParser::new();
        assert!(p.feed(b"\x1b]633;P;Cwd=/tmp\x07").is_empty());
        assert!(p.feed(b"\x1b]633;P;tyba-prompt=sim\x07").is_empty());
    }

    #[test]
    fn whitespace_only_command_is_not_an_event() {
        let mut p = OscParser::new();
        assert!(p
            .feed(format!("\x1b]633;E;{}\x07", b64("   ")).as_bytes())
            .is_empty());
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
        let events = p.feed(b"\x1b]0;my title\x07plain text\x1b]11;?\x07");
        assert!(events.is_empty());
    }

    #[test]
    fn distinguishes_prompt_start_from_input_start() {
        let mut p = OscParser::new();
        assert_eq!(p.feed(b"\x1b]133;A\x07"), vec![ShellEvent::PromptStart]);
        assert_eq!(p.feed(b"\x1b]133;B\x07"), vec![ShellEvent::InputStart]);
    }

    #[test]
    fn parses_osc7_cwd() {
        use std::path::PathBuf;
        let mut p = OscParser::new();
        assert_eq!(
            p.feed(b"\x1b]7;file://mac/tmp/proj\x07"),
            vec![ShellEvent::Cwd(PathBuf::from("/tmp/proj"))]
        );
    }

    #[test]
    fn decodes_percent_encoded_cwd() {
        use std::path::PathBuf;
        let mut p = OscParser::new();
        assert_eq!(
            p.feed("\x1b]7;file://mac/tmp/some%20dir/caf%C3%A9\x07".as_bytes()),
            vec![ShellEvent::Cwd(PathBuf::from("/tmp/some dir/café"))]
        );
    }

    #[test]
    fn osc7_empty_path_is_ignored() {
        let mut p = OscParser::new();
        assert!(p.feed(b"\x1b]7;file://mac\x07").is_empty());
    }

    #[test]
    fn osc7_invalid_percent_encoding_is_ignored() {
        let mut p = OscParser::new();
        assert!(p.feed(b"\x1b]7;file://mac/%ff%fe\x07").is_empty());
    }

    #[test]
    fn osc7_control_chars_are_rejected() {
        let mut p = OscParser::new();
        assert!(p.feed(b"\x1b]7;file://mac/tmp/%1b%0a%00\x07").is_empty());
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
        huge.extend(std::iter::repeat_n(b'x', MAX_OSC_LEN + 100));
        huge.extend_from_slice(b"\x07");
        // Não deve entrar em pânico nem crescer sem limite; nenhum evento válido.
        assert!(p.feed(&huge).is_empty());
        // Parser volta a funcionar depois.
        assert_eq!(p.feed(b"\x1b]133;C\x07"), vec![ShellEvent::CommandStart]);
    }
}
