//! Entrega C — o parser puro do scanner de runtime: janela rolante sobre o
//! stream de bytes do PTY, strip de ANSI/CSI/OSC (disciplina de
//! [`super::MAX_OSC_LEN`], tolera sequência partida entre chunks), match
//! case-insensitive com espaço colapsado contra a tabela de strings medidas
//! no binário `claude` real (ver o design da Entrega C).

use crate::agent::auth_alert::AuthAlertKind;
use crate::session::AgentRunnerKind;

/// Mesmo teto de [`super::MAX_OSC_LEN`] — a janela rolante do scanner segue
/// a mesma disciplina de memória do resto do módulo de status, não um número
/// novo escolhido à parte.
const AUTH_SCAN_WINDOW: usize = super::MAX_OSC_LEN;

/// As strings medidas rodando o binário `claude 2.1.257` real (ver o design
/// da Entrega C) — existem verbatim num `Set` de erros dentro do binário.
/// Só o Claude tem tabela: `patterns_for` devolve vazio pra Codex/Custom
/// (R10), então nenhuma sessão que não seja Claude Code paga o custo do
/// scanner nem corre risco de falso positivo com vocabulário que ninguém
/// mediu.
const CLAUDE_PATTERNS: &[(&str, AuthAlertKind)] = &[
    ("credit balance is too low", AuthAlertKind::CreditBalanceLow),
    (
        "oauth access token has expired",
        AuthAlertKind::TokenExpiredOrRevoked,
    ),
    (
        "refresh token has expired",
        AuthAlertKind::TokenExpiredOrRevoked,
    ),
    (
        "oauth access token has been revoked",
        AuthAlertKind::TokenExpiredOrRevoked,
    ),
    ("not logged in", AuthAlertKind::NotLoggedIn),
    ("please run /login", AuthAlertKind::NotLoggedIn),
    ("invalid api key", AuthAlertKind::InvalidApiKey),
];

/// A tabela de strings pra este runner. Claude-only (decisão do dono, GATE
/// 1): `&[]` pra Codex/Custom desliga o scanner por completo pra eles — a
/// forma (fábrica + tabela) fica pronta pro Codex depois, só a tabela dele
/// ainda não existe.
pub fn patterns_for(kind: &AgentRunnerKind) -> &'static [(&'static str, AuthAlertKind)] {
    match kind {
        AgentRunnerKind::ClaudeCode => CLAUDE_PATTERNS,
        AgentRunnerKind::Codex | AgentRunnerKind::Custom(_) => &[],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StripState {
    Text,
    Esc,
    Csi,
    Osc,
    OscEsc,
}

/// Parser incremental que separa texto puro de sequência de escape — molde
/// do [`super::OscParser`], generalizado pra CSI e pro resto do ESC (aqui o
/// destino é o TEXTO que sobra, não uma sequência OSC específica).
pub struct AuthScanner {
    patterns: &'static [(&'static str, AuthAlertKind)],
    strip: StripState,
    /// Texto puro acumulado: minúsculo, espaço colapsado, sem nenhum byte de
    /// sequência de escape. `Vec<u8>` e não `String` de propósito — um chunk
    /// de PTY pode partir um caractere UTF-8 multibyte ao meio, e um buffer
    /// de bytes não impõe validade UTF-8 em nenhum ponto intermediário.
    window: Vec<u8>,
    /// Collapse de espaço atravessa `feed()`: sem isto, um chunk terminando
    /// em espaço seguido de um chunk começando em espaço duplicaria o
    /// espaço na janela — e "not logged  in" (dois espaços) não bate contra
    /// "not logged in" (um).
    last_was_space: bool,
}

impl AuthScanner {
    pub fn new(patterns: &'static [(&'static str, AuthAlertKind)]) -> Self {
        Self {
            patterns,
            strip: StripState::Text,
            window: Vec::new(),
            last_was_space: false,
        }
    }

    fn push_text_byte(&mut self, b: u8) {
        if b.is_ascii_whitespace() {
            if self.last_was_space {
                return;
            }
            self.window.push(b' ');
            self.last_was_space = true;
        } else {
            self.window.push(b.to_ascii_lowercase());
            self.last_was_space = false;
        }
    }

    /// Alimenta um chunk de bytes crus do PTY. Devolve o primeiro kind que
    /// bater contra a tabela deste runner, ou `None`.
    ///
    /// Ao casar, a janela é ESVAZIADA: o sinal já foi devolvido, e manter o
    /// texto ali faria a MESMA ocorrência bater de novo em toda chamada
    /// seguinte até rolar para fora do teto — o dedupe por `(sessão, kind)`
    /// é responsabilidade de quem chama ([`crate::agent::auth_watch`]), não
    /// desta função; aqui cada chamada devolve no máximo uma ocorrência NOVA
    /// de texto.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<AuthAlertKind> {
        if self.patterns.is_empty() {
            return None;
        }
        for &b in bytes {
            match self.strip {
                StripState::Text => {
                    if b == 0x1b {
                        self.strip = StripState::Esc;
                    } else {
                        self.push_text_byte(b);
                    }
                }
                StripState::Esc => {
                    self.strip = match b {
                        b'[' => StripState::Csi,
                        b']' => StripState::Osc,
                        // Melhor-esforço, como o `OscParser`: a esmagadora
                        // maioria das sequências ESC fora de CSI/OSC que um
                        // programa de terminal emite (`ESC c`, `ESC =`,
                        // `ESC 7`/`8`...) tem exatamente mais um byte.
                        _ => StripState::Text,
                    };
                }
                StripState::Csi => {
                    // CSI termina no primeiro byte "final" (`@`..`~`) — até
                    // lá é parâmetro/intermediário, e nada disso é texto.
                    if (0x40..=0x7e).contains(&b) {
                        self.strip = StripState::Text;
                    }
                }
                StripState::Osc => match b {
                    0x07 => self.strip = StripState::Text,
                    0x1b => self.strip = StripState::OscEsc,
                    _ => {}
                },
                StripState::OscEsc => {
                    // Esperando o `\` do ST (`ESC \`); qualquer coisa fecha.
                    self.strip = StripState::Text;
                }
            }
        }
        if self.window.len() > AUTH_SCAN_WINDOW {
            let excess = self.window.len() - AUTH_SCAN_WINDOW;
            self.window.drain(..excess);
        }
        let hit = self
            .patterns
            .iter()
            .find(|(needle, _)| contains_bytes(&self.window, needle.as_bytes()))
            .map(|(_, kind)| *kind);
        if hit.is_some() {
            self.window.clear();
            self.last_was_space = false;
        }
        hit
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_scanner() -> AuthScanner {
        AuthScanner::new(CLAUDE_PATTERNS)
    }

    /// R1: cada substring medida vira o kind certo.
    #[test]
    fn each_measured_substring_maps_to_its_kind() {
        let cases: &[(&str, AuthAlertKind)] = &[
            ("Credit balance is too low", AuthAlertKind::CreditBalanceLow),
            (
                "OAuth access token has expired",
                AuthAlertKind::TokenExpiredOrRevoked,
            ),
            (
                "Refresh token has expired",
                AuthAlertKind::TokenExpiredOrRevoked,
            ),
            (
                "OAuth access token has been revoked",
                AuthAlertKind::TokenExpiredOrRevoked,
            ),
            ("Not logged in", AuthAlertKind::NotLoggedIn),
            ("Please run /login", AuthAlertKind::NotLoggedIn),
            ("Invalid API key", AuthAlertKind::InvalidApiKey),
        ];
        for (text, expected) in cases {
            let mut scanner = claude_scanner();
            assert_eq!(scanner.feed(text.as_bytes()), Some(*expected), "{text}");
        }
    }

    /// R2: a frase chega partida entre dois chunks — o byte stream do PTY
    /// não promete que uma sentença cabe inteira num `read()` só. A janela
    /// persiste entre chamadas de `feed`, então o pedaço final completa o
    /// que já estava acumulado.
    #[test]
    fn a_pattern_split_across_two_chunks_still_matches() {
        let mut scanner = claude_scanner();
        assert_eq!(scanner.feed(b"turno abortado: not logged"), None);
        assert_eq!(
            scanner.feed(b" in -- rode /login"),
            Some(AuthAlertKind::NotLoggedIn)
        );
    }

    /// R2, três pedaços: a mesma garantia não pode depender de a quebra cair
    /// exatamente em dois chunks.
    #[test]
    fn a_pattern_split_across_three_chunks_still_matches() {
        let mut scanner = claude_scanner();
        assert_eq!(scanner.feed(b"inval"), None);
        assert_eq!(scanner.feed(b"id api "), None);
        assert_eq!(scanner.feed(b"key"), Some(AuthAlertKind::InvalidApiKey));
    }

    /// R3: cor/estilo ANSI (CSI) intercalado no meio da frase não pode
    /// quebrar o casamento nem colar palavras que na tela ficam separadas
    /// por espaço.
    #[test]
    fn ansi_csi_interleaved_in_the_middle_of_the_pattern_is_stripped() {
        let mut scanner = claude_scanner();
        let bytes = b"\x1b[31mnot\x1b[0m logged\x1b[33m in\x1b[0m";
        assert_eq!(scanner.feed(bytes), Some(AuthAlertKind::NotLoggedIn));
    }

    /// R3: título OSC (`ESC ] 0 ; ... BEL`) no meio da frase, mesma garantia
    /// -- o payload do OSC inteiro é engolido, nunca vira texto.
    #[test]
    fn osc_sequence_interleaved_is_stripped_without_leaking_into_the_window() {
        let mut scanner = claude_scanner();
        let bytes = b"credit balance\x1b]0;claude\x07 is too low";
        assert_eq!(scanner.feed(bytes), Some(AuthAlertKind::CreditBalanceLow));
    }

    /// R4: maiúscula/minúscula não importa -- é exatamente como o binário
    /// real imprime ("Not logged in.", com maiúscula inicial e ponto).
    #[test]
    fn matching_is_case_insensitive() {
        let mut scanner = claude_scanner();
        assert_eq!(
            scanner.feed(b"NOT LOGGED IN. Please run /login"),
            Some(AuthAlertKind::NotLoggedIn)
        );
    }

    /// R4: espaço colapsado -- múltiplos espaços, tab ou quebra de linha no
    /// meio da frase (reflow de terminal, indentação) ainda casam contra o
    /// padrão de um espaço só.
    #[test]
    fn matching_collapses_whitespace_runs_to_a_single_space() {
        let mut scanner = claude_scanner();
        assert_eq!(
            scanner.feed(b"not   logged\t\nin"),
            Some(AuthAlertKind::NotLoggedIn)
        );
    }

    /// Depois de casar, a janela esvazia -- alimentar mais texto sem
    /// nenhuma substring nova não repete o kind antigo por a mesma
    /// ocorrência ainda estar no buffer.
    #[test]
    fn a_match_clears_the_window_so_the_same_text_does_not_rematch() {
        let mut scanner = claude_scanner();
        assert_eq!(
            scanner.feed(b"not logged in"),
            Some(AuthAlertKind::NotLoggedIn)
        );
        assert_eq!(scanner.feed(b" mais saida qualquer"), None);
    }

    /// Texto comum, sem nenhuma das strings medidas, nunca casa -- prova
    /// negativa de que o scanner não é gatilho fácil.
    #[test]
    fn ordinary_output_never_matches() {
        let mut scanner = claude_scanner();
        assert_eq!(scanner.feed(b"Reading src/main.rs...\n"), None);
        assert_eq!(scanner.feed(b"Running tests, 12 passed\n"), None);
    }

    /// R10: só o Claude tem tabela -- Codex e Custom devolvem vazio, e um
    /// `AuthScanner` construído sobre uma tabela vazia nunca casa nada,
    /// mesmo alimentado com uma das strings medidas do Claude.
    #[test]
    fn patterns_for_is_empty_outside_claude_code() {
        assert!(patterns_for(&AgentRunnerKind::Codex).is_empty());
        assert!(patterns_for(&AgentRunnerKind::Custom("aider".into())).is_empty());
        assert!(!patterns_for(&AgentRunnerKind::ClaudeCode).is_empty());

        let mut scanner = AuthScanner::new(patterns_for(&AgentRunnerKind::Codex));
        assert_eq!(scanner.feed(b"not logged in"), None);
    }

    /// A janela rolante não cresce sem teto: alimentada bem além do cap,
    /// segue enxergando um padrão que chega inteiro DEPOIS do excesso —
    /// prova de que o corte é do INÍCIO (o mais velho), não do fim.
    #[test]
    fn the_rolling_window_is_capped_and_still_matches_recent_text() {
        let mut scanner = claude_scanner();
        let filler = vec![b'x'; AUTH_SCAN_WINDOW * 2];
        assert_eq!(scanner.feed(&filler), None);
        assert_eq!(
            scanner.feed(b" not logged in"),
            Some(AuthAlertKind::NotLoggedIn)
        );
    }
}
