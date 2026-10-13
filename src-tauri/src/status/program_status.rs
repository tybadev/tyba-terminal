//! OSC 7501, o "Program Status Protocol": o programa diz, pelo PTY, se está
//! trabalhando, parado à espera de alguém ou pronto.
//!
//! Especificação: <https://www.superlogical.com/rex/docs/build/program-status>
//! (revisão 0.4). Resumo do que importa aqui:
//!
//! - relatório: `OSC 7501 ; key=value:key=value… ST`, com `ST` = `ESC \` ou `BEL`;
//! - `state` obrigatório (`idle|working|done|blocked|error|clear`), estado
//!   desconhecido descarta o relatório; chave desconhecida é ignorada; par sem
//!   `=` é pulado; chave repetida, vale a última;
//! - estourar um limite, base64 inválido ou caractere de controle em
//!   `msg`/`title` descarta o relatório **inteiro**;
//! - detecção: o programa manda `OSC 7501 ; ? ST` (seguido de `CSI c`); o
//!   terminal que suporta responde `OSC 7501 ; ? ST` antes do DA.
//!
//! ## Segurança
//!
//! Qualquer processo que escreve no PTY emite isto — um `cat` de arquivo
//! basta. Então o que sai daqui é **só** status e aviso, nunca aprovação nem
//! ação nenhuma. Do texto livre, nada é guardado: `msg` e `title` são validados
//! (porque a especificação manda descartar o relatório que os traz inválidos) e
//! jogados fora. O único texto que sobrevive é `app`, restrito a
//! `[A-Za-z0-9._-]{1,32}` porque vira rótulo na UI.
//!
//! ## Escopo deliberadamente menor que o da especificação
//!
//! O status de sessão olha só o registro **raiz** (relatório sem `id`). Os
//! registros filhos (`id=…`) são validados e ignorados — a sessão tem uma
//! linha no quadro, não uma árvore.

use base64::Engine;

use crate::session::{ObservedState, ObservedWait};

/// Teto da sequência inteira, pela especificação.
pub const MAX_SEQUENCE_LEN: usize = 4096;
const MAX_KEY_LEN: usize = 16;
const MAX_MSG_ENCODED: usize = 2732;
const MAX_MSG_DECODED: usize = 2048;
const MAX_TITLE_ENCODED: usize = 256;
const MAX_TITLE_DECODED: usize = 192;
const MAX_APP_LEN: usize = 32;
const MAX_ID_LEN: usize = 128;
const MAX_ID_SEGMENT: usize = 32;
const MAX_ID_DEPTH: usize = 8;

/// A resposta à detecção, sem as listas: listas ausentes valem o conjunto fixo
/// da especificação, que é exatamente o que este parser aceita.
pub fn query_reply(bel: bool) -> &'static [u8] {
    if bel {
        b"\x1b]7501;?\x07"
    } else {
        b"\x1b]7501;?\x1b\\"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

impl ProgramState {
    /// O vocabulário do quadro é menor: `done`, `idle` e `error` são todos "não
    /// está trabalhando e não espera ninguém".
    pub fn observed(self) -> ObservedState {
        match self {
            ProgramState::Working => ObservedState::Running,
            ProgramState::Blocked => ObservedState::AwaitingInput,
            ProgramState::Idle | ProgramState::Done | ProgramState::Error => ObservedState::Idle,
        }
    }
}

/// O registro raiz de uma sessão.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramStatus {
    pub state: ProgramState,
    /// Só com `blocked`.
    pub kind: Option<ObservedWait>,
    /// 0..=100; `None` é indeterminado.
    pub progress: Option<u8>,
    /// Já filtrado para `[A-Za-z0-9._-]{1,32}`.
    pub app: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    Status {
        id: Option<String>,
        status: ProgramStatus,
    },
    Clear {
        id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramEvent {
    /// `OSC 7501 ; ?` exato. `bel` diz com que terminador veio, para a resposta
    /// sair com o mesmo.
    Query {
        bel: bool,
    },
    Report(Report),
    /// `OSC 133 ; A`: prompt novo. Pela especificação, `working` e `blocked`
    /// caem aqui.
    PromptStart,
    /// `OSC 133 ; C`: o shell entregou o terminal a outro comando.
    CommandStart,
    /// `ESC c` (RIS): reset total limpa todos os registros.
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Text,
    Esc,
    Osc,
    OscEsc,
    /// Corpo de OSC que não interessa ou estourou o teto: espera o terminador
    /// sem guardar nada.
    Skip,
    SkipEsc,
}

/// Parser incremental: tolera a sequência partida em qualquer byte.
pub struct Osc7501Scanner {
    state: State,
    buf: Vec<u8>,
}

impl Default for Osc7501Scanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Osc7501Scanner {
    pub fn new() -> Self {
        Self {
            state: State::Text,
            buf: Vec::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<ProgramEvent> {
        let mut events = Vec::new();
        for &b in bytes {
            self.state = match self.state {
                State::Text => {
                    if b == 0x1b {
                        State::Esc
                    } else {
                        State::Text
                    }
                }
                State::Esc => match b {
                    b']' => {
                        self.buf.clear();
                        State::Osc
                    }
                    b'c' => {
                        events.push(ProgramEvent::Reset);
                        State::Text
                    }
                    0x1b => State::Esc,
                    _ => State::Text,
                },
                State::Osc => match b {
                    0x07 => {
                        self.finish(true, &mut events);
                        State::Text
                    }
                    0x1b => State::OscEsc,
                    _ => {
                        self.buf.push(b);
                        if self.buf.len() > MAX_SEQUENCE_LEN || !relevant_prefix(&self.buf) {
                            self.buf.clear();
                            State::Skip
                        } else {
                            State::Osc
                        }
                    }
                },
                State::OscEsc => {
                    if b == b'\\' {
                        self.finish(false, &mut events);
                        State::Text
                    } else {
                        // ESC no meio do corpo: a sequência foi abortada. O
                        // ESC pode ser o começo de outra.
                        self.buf.clear();
                        if b == b']' {
                            State::Osc
                        } else {
                            State::Text
                        }
                    }
                }
                State::Skip => match b {
                    0x07 => State::Text,
                    0x1b => State::SkipEsc,
                    _ => State::Skip,
                },
                State::SkipEsc => {
                    if b == b']' {
                        self.buf.clear();
                        State::Osc
                    } else {
                        State::Text
                    }
                }
            };
        }
        events
    }

    fn finish(&mut self, bel: bool, events: &mut Vec<ProgramEvent>) {
        if let Some(ev) = parse_payload(&self.buf, bel) {
            events.push(ev);
        }
        self.buf.clear();
    }
}

/// O corpo ainda pode ser `7501;…` ou `133;…`? Corta cedo o resto (título,
/// cor, hyperlink), que nem precisa ser guardado.
fn relevant_prefix(buf: &[u8]) -> bool {
    let fits = |p: &[u8]| {
        let n = buf.len().min(p.len());
        buf[..n] == p[..n]
    };
    fits(b"7501;") || fits(b"133;")
}

fn parse_payload(payload: &[u8], bel: bool) -> Option<ProgramEvent> {
    if let Some(rest) = payload.strip_prefix(b"133;") {
        return match rest.first()? {
            b'A' if rest.len() == 1 || rest[1] == b';' => Some(ProgramEvent::PromptStart),
            b'C' if rest.len() == 1 || rest[1] == b';' => Some(ProgramEvent::CommandStart),
            _ => None,
        };
    }
    let body = payload.strip_prefix(b"7501;")?;
    if body == b"?" {
        return Some(ProgramEvent::Query { bel });
    }
    let body = std::str::from_utf8(body).ok()?;
    parse_report(body).map(ProgramEvent::Report)
}

/// Um relatório. `None` = descartado inteiro.
pub fn parse_report(body: &str) -> Option<Report> {
    let mut state: Option<&str> = None;
    let mut id: Option<&str> = None;
    let mut kind: Option<&str> = None;
    let mut progress: Option<&str> = None;
    let mut app: Option<&str> = None;
    let mut msg: Option<&str> = None;
    let mut title: Option<&str> = None;

    for pair in body.split(':') {
        let Some((key, value)) = pair.split_once('=') else {
            continue; // par malformado: pulado
        };
        if key.len() > MAX_KEY_LEN {
            return None;
        }
        match key {
            "state" => state = Some(value),
            "id" => id = Some(value),
            "kind" => kind = Some(value),
            "progress" => progress = Some(value),
            "app" => app = Some(value),
            "msg" => msg = Some(value),
            "title" => title = Some(value),
            _ => {} // chave desconhecida: ignorada
        }
    }

    // Limites e validação primeiro: qualquer violação descarta tudo.
    if let Some(id) = id {
        if !valid_id(id) {
            return None;
        }
    }
    if let Some(app) = app {
        if app.len() > MAX_APP_LEN {
            return None;
        }
    }
    if let Some(msg) = msg {
        valid_text(msg, MAX_MSG_ENCODED, MAX_MSG_DECODED)?;
    }
    if let Some(title) = title {
        valid_text(title, MAX_TITLE_ENCODED, MAX_TITLE_DECODED)?;
    }

    let id = id.map(str::to_string);
    let state = match state? {
        "idle" => ProgramState::Idle,
        "working" => ProgramState::Working,
        "done" => ProgramState::Done,
        "blocked" => ProgramState::Blocked,
        "error" => ProgramState::Error,
        "clear" => return Some(Report::Clear { id }),
        _ => return None,
    };
    let kind = match (state, kind) {
        (ProgramState::Blocked, Some("permission")) => Some(ObservedWait::Permission),
        (ProgramState::Blocked, Some("question")) => Some(ObservedWait::Question),
        (ProgramState::Blocked, Some("auth")) => Some(ObservedWait::Auth),
        _ => None,
    };
    let progress = match state {
        ProgramState::Working | ProgramState::Blocked => progress
            .and_then(|p| p.parse::<u8>().ok())
            .filter(|p| *p <= 100),
        _ => None,
    };
    Some(Report::Status {
        id,
        status: ProgramStatus {
            state,
            kind,
            progress,
            app: app.filter(|a| displayable_app(a)).map(str::to_string),
        },
    })
}

fn valid_id(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        return false;
    }
    let segments: Vec<&str> = id.split('/').collect();
    segments.len() <= MAX_ID_DEPTH
        && segments
            .iter()
            .all(|s| !s.is_empty() && s.len() <= MAX_ID_SEGMENT)
}

/// `app` vira rótulo na UI: só o alfabeto de identificador passa. Fora dele o
/// relatório vale, mas sem nome.
fn displayable_app(app: &str) -> bool {
    !app.is_empty()
        && app
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Base64 de UTF-8 sem controle e sem caractere invisível de direção. O texto
/// decodificado não é guardado — ver o doc do módulo.
fn valid_text(encoded: &str, max_encoded: usize, max_decoded: usize) -> Option<()> {
    if encoded.len() > max_encoded {
        return None;
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if decoded.len() > max_decoded {
        return None;
    }
    let text = std::str::from_utf8(&decoded).ok()?;
    if text.chars().any(|c| c.is_control() || is_bidi_control(c)) {
        return None;
    }
    Some(())
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// O registro raiz da sessão, com o ciclo de vida da especificação.
#[derive(Debug, Default)]
pub struct ProgramTracker {
    root: Option<ProgramStatus>,
}

impl ProgramTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Option<&ProgramStatus> {
        self.root.as_ref()
    }

    /// Aplica um evento. Devolve se o registro raiz mudou.
    pub fn apply(&mut self, event: &ProgramEvent) -> bool {
        let next = match event {
            ProgramEvent::Query { .. } => return false,
            ProgramEvent::Report(Report::Status { id: None, status }) => Some(status.clone()),
            // `clear` sem id remove todos os registros.
            ProgramEvent::Report(Report::Clear { id: None }) => None,
            // Registro filho: fora do escopo do status de sessão.
            ProgramEvent::Report(_) => return false,
            // Prompt novo: `working` e `blocked` caem (especificação); `idle`
            // também, que a especificação permite. `done` e `error` ficam —
            // são resultado que o usuário ainda não viu.
            ProgramEvent::PromptStart => self
                .root
                .clone()
                .filter(|s| matches!(s.state, ProgramState::Done | ProgramState::Error)),
            // O shell passou o terminal a outro comando: o que o anterior
            // relatou deixou de ser sobre quem está na tela.
            ProgramEvent::CommandStart | ProgramEvent::Reset => None,
        };
        if next == self.root {
            return false;
        }
        self.root = next;
        true
    }
}

/// Teto de respostas à detecção: um arquivo com mil consultas despejado no
/// terminal não pode virar mil respostas empurradas na entrada do shell.
/// Programa legítimo pergunta uma vez, ao subir.
pub struct QueryLimiter {
    window_start: Option<std::time::Instant>,
    count: u32,
}

const QUERY_REPLIES_PER_SECOND: u32 = 4;

impl Default for QueryLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryLimiter {
    pub fn new() -> Self {
        Self {
            window_start: None,
            count: 0,
        }
    }

    pub fn allow(&mut self, now: std::time::Instant) -> bool {
        match self.window_start {
            Some(start) if now.duration_since(start) < std::time::Duration::from_secs(1) => {
                if self.count >= QUERY_REPLIES_PER_SECOND {
                    return false;
                }
                self.count += 1;
                true
            }
            _ => {
                self.window_start = Some(now);
                self.count = 1;
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    fn status(state: ProgramState) -> ProgramStatus {
        ProgramStatus {
            state,
            kind: None,
            progress: None,
            app: None,
        }
    }

    fn root(status: ProgramStatus) -> ProgramEvent {
        ProgramEvent::Report(Report::Status { id: None, status })
    }

    #[test]
    fn parses_the_spec_example_with_st() {
        let mut s = Osc7501Scanner::new();
        let events = s.feed(
            b"\x1b]7501;state=blocked:kind=permission:app=terraform:msg=QXBwbHkgMyB0byBhZGQsIDEgdG8gY2hhbmdlLCAwIHRvIGRlc3Ryb3k/\x1b\\",
        );
        assert_eq!(
            events,
            vec![root(ProgramStatus {
                state: ProgramState::Blocked,
                kind: Some(ObservedWait::Permission),
                progress: None,
                app: Some("terraform".into()),
            })]
        );
    }

    #[test]
    fn bel_terminates_too() {
        let mut s = Osc7501Scanner::new();
        assert_eq!(
            s.feed(b"\x1b]7501;state=working:progress=40\x07"),
            vec![root(ProgramStatus {
                progress: Some(40),
                ..status(ProgramState::Working)
            })]
        );
    }

    #[test]
    fn every_state_is_recognized() {
        for (raw, state) in [
            ("idle", ProgramState::Idle),
            ("working", ProgramState::Working),
            ("done", ProgramState::Done),
            ("blocked", ProgramState::Blocked),
            ("error", ProgramState::Error),
        ] {
            assert_eq!(
                parse_report(&format!("state={raw}")),
                Some(Report::Status {
                    id: None,
                    status: status(state)
                })
            );
        }
        assert_eq!(
            parse_report("state=clear"),
            Some(Report::Clear { id: None })
        );
    }

    #[test]
    fn survives_being_split_at_every_byte() {
        let full = format!(
            "texto\x1b]7501;state=blocked:kind=question:msg={}\x1b\\resto",
            b64("Qual branch?")
        );
        let mut s = Osc7501Scanner::new();
        let mut events = Vec::new();
        for byte in full.as_bytes() {
            events.extend(s.feed(&[*byte]));
        }
        assert_eq!(
            events,
            vec![root(ProgramStatus {
                kind: Some(ObservedWait::Question),
                ..status(ProgramState::Blocked)
            })]
        );
    }

    #[test]
    fn split_in_two_chunks_at_the_st() {
        let mut s = Osc7501Scanner::new();
        assert!(s.feed(b"\x1b]7501;state=done\x1b").is_empty());
        assert_eq!(s.feed(b"\\"), vec![root(status(ProgramState::Done))]);
    }

    #[test]
    fn unknown_keys_are_ignored_and_last_repeat_wins() {
        assert_eq!(
            parse_report("futuro=1:state=idle:state=working:semigual"),
            Some(Report::Status {
                id: None,
                status: status(ProgramState::Working)
            })
        );
    }

    #[test]
    fn missing_or_unknown_state_discards() {
        assert_eq!(parse_report("kind=auth"), None);
        assert_eq!(parse_report("state=dancing"), None);
        assert_eq!(parse_report(""), None);
    }

    #[test]
    fn kind_only_counts_with_blocked() {
        assert_eq!(
            parse_report("state=working:kind=auth"),
            Some(Report::Status {
                id: None,
                status: status(ProgramState::Working)
            })
        );
        assert_eq!(
            parse_report("state=blocked:kind=telepathy"),
            Some(Report::Status {
                id: None,
                status: status(ProgramState::Blocked)
            })
        );
    }

    #[test]
    fn invalid_progress_is_indeterminate() {
        for p in ["101", "-1", "x", ""] {
            assert_eq!(
                parse_report(&format!("state=working:progress={p}")),
                Some(Report::Status {
                    id: None,
                    status: status(ProgramState::Working)
                }),
                "progress={p}"
            );
        }
    }

    #[test]
    fn limits_discard_the_whole_report() {
        let long_key = "k".repeat(MAX_KEY_LEN + 1);
        assert_eq!(parse_report(&format!("state=done:{long_key}=1")), None);
        let long_app = "a".repeat(MAX_APP_LEN + 1);
        assert_eq!(parse_report(&format!("state=done:app={long_app}")), None);
        assert_eq!(parse_report("state=done:id=a//b"), None);
        let deep = ["a"; MAX_ID_DEPTH + 1].join("/");
        assert_eq!(parse_report(&format!("state=done:id={deep}")), None);
        let big_msg = b64(&"x".repeat(MAX_MSG_DECODED + 1));
        assert_eq!(parse_report(&format!("state=done:msg={big_msg}")), None);
        let big_title = b64(&"x".repeat(MAX_TITLE_DECODED + 1));
        assert_eq!(parse_report(&format!("state=done:title={big_title}")), None);
    }

    #[test]
    fn hostile_text_discards_the_whole_report() {
        assert_eq!(parse_report("state=done:msg=!!nao-base64!!"), None);
        let esc = b64("oi\x1b[31mvermelho");
        assert_eq!(parse_report(&format!("state=done:msg={esc}")), None);
        let newline = b64("linha\noutra");
        assert_eq!(parse_report(&format!("state=done:title={newline}")), None);
        let bidi = b64("abc\u{202E}fdp.exe");
        assert_eq!(parse_report(&format!("state=done:msg={bidi}")), None);
        let not_utf8 = base64::engine::general_purpose::STANDARD.encode([0xff, 0xfe]);
        assert_eq!(parse_report(&format!("state=done:msg={not_utf8}")), None);
    }

    #[test]
    fn app_outside_identifier_alphabet_is_dropped_not_shown() {
        assert_eq!(
            parse_report("state=working:app=<b>x</b>"),
            Some(Report::Status {
                id: None,
                status: status(ProgramState::Working)
            })
        );
        assert_eq!(
            parse_report("state=working:app=claude-code"),
            Some(Report::Status {
                id: None,
                status: ProgramStatus {
                    app: Some("claude-code".into()),
                    ..status(ProgramState::Working)
                }
            })
        );
    }

    #[test]
    fn oversized_sequence_is_dropped_and_parser_recovers() {
        let mut s = Osc7501Scanner::new();
        let mut huge = b"\x1b]7501;state=working:pad=".to_vec();
        huge.extend(std::iter::repeat_n(b'x', MAX_SEQUENCE_LEN + 10));
        huge.extend_from_slice(b"\x07");
        assert!(s.feed(&huge).is_empty());
        assert_eq!(
            s.feed(b"\x1b]7501;state=done\x07"),
            vec![root(status(ProgramState::Done))]
        );
    }

    #[test]
    fn garbage_and_other_oscs_produce_nothing() {
        let mut s = Osc7501Scanner::new();
        assert!(s
            .feed(b"\x1b]0;titulo\x07\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\\x1b[31m\x00\xff\xfe")
            .is_empty());
        assert!(s.feed(b"\x1b]75010;state=done\x07").is_empty());
        assert!(s.feed(b"\x1b]7501state=done\x07").is_empty());
        // ESC no meio do corpo aborta a sequência.
        assert!(s.feed(b"\x1b]7501;state=done\x1bX\x07").is_empty());
        assert_eq!(
            s.feed(b"\x1b]7501;state=idle\x07"),
            vec![root(status(ProgramState::Idle))]
        );
    }

    #[test]
    fn child_records_carry_their_id() {
        assert_eq!(
            parse_report("state=working:id=us-east/pod-1"),
            Some(Report::Status {
                id: Some("us-east/pod-1".into()),
                status: status(ProgramState::Working)
            })
        );
    }

    #[test]
    fn only_the_exact_query_is_a_query() {
        let mut s = Osc7501Scanner::new();
        assert_eq!(
            s.feed(b"\x1b]7501;?\x1b\\"),
            vec![ProgramEvent::Query { bel: false }]
        );
        assert_eq!(
            s.feed(b"\x1b]7501;?\x07"),
            vec![ProgramEvent::Query { bel: true }]
        );
        for near in [
            &b"\x1b]7501;??\x07"[..],
            b"\x1b]7501;? \x07",
            b"\x1b]7501;?:x=1\x07",
            b"\x1b]7501?\x07",
            b"\x1b]17501;?\x07",
            b"7501;?\x07",
        ] {
            assert!(s.feed(near).is_empty(), "{near:?}");
        }
    }

    #[test]
    fn query_split_across_chunks() {
        let mut s = Osc7501Scanner::new();
        assert!(s.feed(b"\x1b]75").is_empty());
        assert!(s.feed(b"01;?\x1b").is_empty());
        assert_eq!(
            s.feed(b"\\\x1b[c"),
            vec![ProgramEvent::Query { bel: false }]
        );
    }

    #[test]
    fn reply_mirrors_the_terminator() {
        assert_eq!(query_reply(false), b"\x1b]7501;?\x1b\\");
        assert_eq!(query_reply(true), b"\x1b]7501;?\x07");
    }

    #[test]
    fn limiter_caps_replies_per_second() {
        let t0 = std::time::Instant::now();
        let mut l = QueryLimiter::new();
        let allowed = (0..10).filter(|_| l.allow(t0)).count();
        assert_eq!(allowed as u32, QUERY_REPLIES_PER_SECOND);
        assert!(l.allow(t0 + std::time::Duration::from_millis(1001)));
    }

    #[test]
    fn shell_marks_and_reset_are_events() {
        let mut s = Osc7501Scanner::new();
        assert_eq!(s.feed(b"\x1b]133;A\x07"), vec![ProgramEvent::PromptStart]);
        assert_eq!(
            s.feed(b"\x1b]133;C\x1b\\"),
            vec![ProgramEvent::CommandStart]
        );
        assert!(s.feed(b"\x1b]133;D;0\x07\x1b]133;B\x07").is_empty());
        assert_eq!(s.feed(b"\x1bc"), vec![ProgramEvent::Reset]);
    }

    #[test]
    fn tracker_follows_the_spec_lifecycle() {
        let mut t = ProgramTracker::new();
        assert!(t.apply(&root(status(ProgramState::Working))));
        assert!(
            !t.apply(&root(status(ProgramState::Working))),
            "repetição mudou"
        );
        // Prompt novo derruba `working`.
        assert!(t.apply(&ProgramEvent::PromptStart));
        assert_eq!(t.current(), None);

        // `done` sobrevive ao prompt…
        t.apply(&root(status(ProgramState::Done)));
        assert!(!t.apply(&ProgramEvent::PromptStart));
        assert_eq!(t.current().map(|s| s.state), Some(ProgramState::Done));
        // …mas não ao próximo comando.
        assert!(t.apply(&ProgramEvent::CommandStart));
        assert_eq!(t.current(), None);

        t.apply(&root(status(ProgramState::Blocked)));
        assert!(t.apply(&ProgramEvent::Report(Report::Clear { id: None })));
        assert_eq!(t.current(), None);

        t.apply(&root(status(ProgramState::Error)));
        assert!(t.apply(&ProgramEvent::Reset));
        assert_eq!(t.current(), None);
    }

    #[test]
    fn tracker_ignores_child_records_and_queries() {
        let mut t = ProgramTracker::new();
        assert!(!t.apply(&ProgramEvent::Report(Report::Status {
            id: Some("filho".into()),
            status: status(ProgramState::Blocked),
        })));
        assert!(!t.apply(&ProgramEvent::Report(Report::Clear {
            id: Some("filho".into())
        })));
        assert!(!t.apply(&ProgramEvent::Query { bel: true }));
        assert_eq!(t.current(), None);
    }
}
