//! O protocolo de modo de controle do tmux (`tmux -C`), puro e sem processo.
//!
//! Conferido contra o `man tmux` do 3.7c local (seção CONTROL MODE) e contra um
//! `tmux -C new-session` de verdade em 2026-09-17. Três frases do manual mandam
//! no desenho daqui:
//!
//! - *"Each command will produce one block of output"* — todo comando que o
//!   TYBA manda volta embrulhado num `%begin`/`%end` (ou `%error`);
//! - *"A notification will never occur inside an output block"* — dentro do
//!   bloco não existe `%output`, então corpo de bloco e saída do pane nunca se
//!   confundem;
//! - *"value escapes non-printable characters and backslash as octal \xxx"* —
//!   como a contrabarra também vem escapada, toda contrabarra no valor abre um
//!   escape de três dígitos. UTF-8 passa cru (medido: `é` chega como `\xc3\xa9`).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// O que uma leitura do PTY produziu depois de passar pelo protocolo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEvent {
    /// Bytes do pane, já desescapados — o que um PTY cru teria entregue.
    Output(Vec<u8>),
    /// `%exit`: o cliente de controle está saindo.
    Exit,
    /// Notificação que este decodificador não conhece.
    Notification(String),
    /// Linha que não coube no protocolo (escape quebrado, linha gigante).
    Unparsed(String),
}

/// Teto da linha em construção. Uma linha de `%output` é limitada pelo buffer
/// do tmux (alguns KiB); este teto existe para que um fluxo sem `\n` — servidor
/// confuso, protocolo trocado — não coma a memória da máquina do dono.
pub const MAX_LINE: usize = 1024 * 1024;

/// Teto do corpo de um bloco de captura guardado em memória. O comando que o
/// TYBA monta já limita a captura a [`CAPTURE_LINES`] linhas; isto é o cinto.
pub const MAX_BLOCK_BODY: usize = 8 * 1024 * 1024;

/// Quantas linhas de histórico o redesenho de uma sessão reatada pede — o mesmo
/// `history-limit` que o embrulho do tmux remoto já configura (regra 5).
pub const CAPTURE_LINES: u32 = 5000;

/// O que a thread leitora e quem escreve no PTY compartilham de uma sessão em
/// modo de controle.
///
/// Existe porque as duas pontas do transporte moram em threads diferentes: o
/// decodificador (thread leitora) é quem descobre que o protocolo começou, e
/// `PtyPool::write`/`resize`/`redraw_from_capture` (thread de quem chama) é
/// quem precisa saber disso para escolher entre byte cru e comando de tmux.
#[derive(Clone, Default)]
pub struct ControlLink {
    inner: Arc<LinkState>,
}

#[derive(Default)]
struct LinkState {
    in_control: AtomicBool,
    armed: AtomicBool,
    pending_capture: AtomicBool,
    /// `cols << 16 | rows`, o tamanho que o cliente de controle deve declarar.
    size: AtomicU32,
}

impl ControlLink {
    /// O protocolo já começou? Enquanto for `false`, o que sai do PTY é byte
    /// cru (banner do ssh, pedido de senha) e o que entra tem de ir cru também.
    pub fn in_control(&self) -> bool {
        self.inner.in_control.load(Ordering::Acquire)
    }

    /// O próximo bloco de comando COM corpo é uma captura de tela.
    pub fn arm_capture(&self) {
        self.inner.armed.store(true, Ordering::Release);
    }

    fn armed(&self) -> bool {
        self.inner.armed.load(Ordering::Acquire)
    }

    fn disarm(&self) {
        self.inner.armed.store(false, Ordering::Release);
    }

    /// Pede uma captura para quando o protocolo começar — é o caso de reatar,
    /// em que `redraw_from_capture` é chamado antes de o tmux existir.
    pub fn queue_capture(&self) {
        self.inner.pending_capture.store(true, Ordering::Release);
    }

    pub fn take_queued_capture(&self) -> bool {
        self.inner.pending_capture.swap(false, Ordering::AcqRel)
    }

    pub fn set_size(&self, cols: u16, rows: u16) {
        self.inner
            .size
            .store((u32::from(cols) << 16) | u32::from(rows), Ordering::Release);
    }

    pub fn size(&self) -> (u16, u16) {
        let packed = self.inner.size.load(Ordering::Acquire);
        ((packed >> 16) as u16, (packed & 0xffff) as u16)
    }
}

/// Em que ponto do fluxo a sessão está.
enum Phase {
    /// Antes do marco de troca: tudo é byte cru do ssh. `held` é o pedaço que
    /// ainda pode virar o marco — no máximo `marker.len() - 1` bytes.
    Raw {
        marker: Vec<u8>,
        held: Vec<u8>,
    },
    Control,
}

struct Block {
    number: Vec<u8>,
    lines: usize,
    body: Vec<u8>,
}

pub struct ControlDecoder {
    phase: Phase,
    link: ControlLink,
    line: Vec<u8>,
    /// Estourou o teto: engole tudo até a próxima quebra de linha.
    discarding: bool,
    block: Option<Block>,
}

impl Default for ControlDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ControlDecoder {
    /// Sem marco: o primeiro byte já é protocolo (um `tmux -C` local).
    pub fn new() -> Self {
        Self::with_marker(None)
    }

    /// Com marco: tudo antes dele passa cru — é o banner do ssh, o pedido de
    /// senha e o marco de login do Cano, que o `CanoWatch` precisa ver inteiro.
    /// O marco em si é consumido e nunca chega à tela.
    pub fn with_marker(marker: Option<&str>) -> Self {
        let phase = match marker {
            Some(marker) if !marker.is_empty() => Phase::Raw {
                marker: marker.as_bytes().to_vec(),
                held: Vec::new(),
            },
            _ => Phase::Control,
        };
        let link = ControlLink::default();
        if matches!(phase, Phase::Control) {
            link.inner.in_control.store(true, Ordering::Release);
        }
        Self {
            phase,
            link,
            line: Vec::new(),
            discarding: false,
            block: None,
        }
    }

    /// A ponta compartilhada com quem escreve no PTY.
    pub fn link(&self) -> ControlLink {
        self.link.clone()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<ControlEvent> {
        let mut events = Vec::new();
        let leftover;
        let bytes: &[u8] = if matches!(self.phase, Phase::Raw { .. }) {
            match self.feed_raw(bytes, &mut events) {
                Some(rest) => {
                    leftover = rest;
                    &leftover
                }
                None => return events,
            }
        } else {
            bytes
        };
        for &byte in bytes {
            if byte != b'\n' {
                if self.discarding {
                    continue;
                }
                self.line.push(byte);
                if self.line.len() > MAX_LINE {
                    self.discarding = true;
                    self.line.clear();
                    self.line.shrink_to_fit();
                    events.push(ControlEvent::Unparsed(format!(
                        "linha de controle acima de {MAX_LINE} bytes, descartada"
                    )));
                }
                continue;
            }
            if self.discarding {
                self.discarding = false;
                continue;
            }
            // O `\r` final é do ONLCR do tty por onde o tmux fala; o payload
            // do `%output` nunca traz CR literal (vem como `\015`).
            if self.line.last() == Some(&b'\r') {
                self.line.pop();
            }
            let line = std::mem::take(&mut self.line);
            if let Some(event) = self.dispatch(&line) {
                events.push(event);
            }
        }
        events
    }

    /// A fase crua: devolve o que sobrou depois do marco, ou `None` enquanto
    /// ele não aparecer.
    fn feed_raw(&mut self, bytes: &[u8], events: &mut Vec<ControlEvent>) -> Option<Vec<u8>> {
        let Phase::Raw { marker, held } = std::mem::replace(&mut self.phase, Phase::Control) else {
            return Some(bytes.to_vec());
        };
        let mut buf = held;
        buf.extend_from_slice(bytes);
        if let Some(at) = find(&buf, &marker) {
            if at > 0 {
                events.push(ControlEvent::Output(buf[..at].to_vec()));
            }
            self.link.inner.in_control.store(true, Ordering::Release);
            return Some(buf[at + marker.len()..].to_vec());
        }
        // Só o que ainda PODE virar o marco fica retido. Reter sempre
        // `marker.len() - 1` bytes seguraria o fim de um "Password: " — que não
        // tem quebra de linha e pode ser a última coisa que o ssh escreve antes
        // de esperar o dono digitar.
        let keep = tail_prefix_of(&buf, &marker);
        let split = buf.len() - keep;
        if split > 0 {
            events.push(ControlEvent::Output(buf[..split].to_vec()));
        }
        self.phase = Phase::Raw {
            marker,
            held: buf[split..].to_vec(),
        };
        None
    }

    fn dispatch(&mut self, line: &[u8]) -> Option<ControlEvent> {
        if self.block.is_some() {
            return self.dispatch_in_block(line);
        }
        // Linha vazia fora de bloco não é linha mal formada: é o eco do que o
        // TYBA escreveu voltando pelo tty. Anunciá-la gastaria a única queixa
        // que a regra 6 permite por sessão.
        if line.is_empty() {
            return None;
        }
        if let Some(rest) = line.strip_prefix(b"%begin ") {
            self.block = Some(Block {
                number: rest
                    .split(|&b| b == b' ')
                    .nth(1)
                    .unwrap_or_default()
                    .to_vec(),
                lines: 0,
                body: Vec::new(),
            });
            return None;
        }
        if let Some(rest) = line.strip_prefix(b"%output ") {
            // Regra 6: escape quebrado é uma linha perdida, nunca uma sessão
            // perdida — e nunca bytes meio-desescapados na tela.
            let payload = rest
                .iter()
                .position(|&b| b == b' ')
                .and_then(|space| unescape(&rest[space + 1..]));
            return Some(match payload {
                Some(bytes) => ControlEvent::Output(bytes),
                None => ControlEvent::Unparsed(String::from_utf8_lossy(line).into_owned()),
            });
        }
        let name = line.split(|&b| b == b' ').next().unwrap_or_default();
        if name == b"%exit" {
            return Some(ControlEvent::Exit);
        }
        if KNOWN_NOTIFICATIONS.contains(&name) {
            return None;
        }
        if !line.starts_with(b"%") {
            return Some(ControlEvent::Unparsed(
                String::from_utf8_lossy(line).into_owned(),
            ));
        }
        // Regra 6: o que este decodificador não conhece é anunciado uma vez e
        // segue o baile — versão nova de tmux não derruba a sessão.
        Some(ControlEvent::Notification(
            String::from_utf8_lossy(line).into_owned(),
        ))
    }

    /// Dentro de um `%begin`. O manual garante que notificação nenhuma cai
    /// aqui, então toda linha é corpo de resposta de comando — e corpo de
    /// resposta nunca vai à tela, salvo quando a captura está armada
    /// (`redraw_from_capture`).
    ///
    /// > [!warning] O que distingue a captura dos outros blocos é ter corpo.
    /// > Os comandos que o TYBA manda — `send-keys`, `refresh-client` — voltam
    /// > com bloco VAZIO (medido no 3.7c local), e a partida do tmux também.
    /// > Quem passar a mandar um comando que imprime algo precisa voltar aqui:
    /// > o primeiro bloco com corpo depois de armar seria dele, não da
    /// > captura, e a tela receberia o texto errado.
    fn dispatch_in_block(&mut self, line: &[u8]) -> Option<ControlEvent> {
        let mut fields = line.split(|&b| b == b' ');
        let name = fields.next().unwrap_or_default();
        let number = fields.nth(1);
        let closes = matches!(name, b"%end" | b"%error")
            && self
                .block
                .as_ref()
                .is_some_and(|block| Some(block.number.as_slice()) == number);
        if closes {
            let block = self.block.take()?;
            let failed = name == b"%error";
            if self.link.armed() && (failed || block.lines > 0) {
                self.link.disarm();
                if !failed && !block.body.is_empty() {
                    return Some(ControlEvent::Output(block.body));
                }
            }
            return None;
        }
        let armed = self.link.armed();
        let block = self.block.as_mut()?;
        block.lines += 1;
        if armed && block.body.len() + line.len() + 2 <= MAX_BLOCK_BODY {
            // CRLF explícito: a captura chega em linhas, e a tela precisa do
            // retorno de carro para não escadear.
            block.body.extend_from_slice(line);
            block.body.extend_from_slice(b"\r\n");
        }
        None
    }
}

/// Entrada do pane, byte a byte em hexadecimal.
///
/// `-H` (`man tmux`): *"expects each key to be a hexadecimal number for an
/// ASCII character"* — um byte por argumento, então UTF-8 vai byte a byte e
/// chega inteiro do outro lado. Nunca `-l`: ali o tmux interpretaria nome de
/// tecla e o `\r` do dono viraria outra coisa.
pub fn send_keys(target: &str, bytes: &[u8]) -> String {
    let mut out = format!("send-keys -t {} -H", quote(target));
    for byte in bytes {
        out.push_str(&format!(" {byte:02x}"));
    }
    out
}

/// O tamanho de um cliente de modo de controle não vem do tty: vem daqui
/// (`man tmux`, `refresh-client -C`).
pub fn refresh_client(cols: u16, rows: u16) -> String {
    format!("refresh-client -C {cols}x{rows}")
}

/// O redesenho de quem reata (regra 5). `-p` joga em stdout (= corpo do
/// bloco), `-e` preserva os atributos, `-J` junta linha quebrada, `-S -N`
/// começa N linhas atrás no histórico.
pub fn capture_pane(target: &str, lines: u32) -> String {
    format!("capture-pane -p -e -J -t {} -S -{lines}", quote(target))
}

/// Aspas no estilo do sh — o lexer do tmux entende as mesmas (conferido no
/// 3.7c local com uma sessão de nome `ty ba-'q'-probe`).
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// O maior sufixo de `buf` que ainda é começo de `needle`.
fn tail_prefix_of(buf: &[u8], needle: &[u8]) -> usize {
    let max = needle.len().saturating_sub(1).min(buf.len());
    (1..=max)
        .rev()
        .find(|&k| buf[buf.len() - k..] == needle[..k])
        .unwrap_or(0)
}

/// Toda notificação de `man tmux` (3.7c) que o TYBA consome em silêncio.
///
/// `%extended-output` entra na lista por completude: ele só aparece com
/// `refresh-client -A pane:pause`, que o TYBA nunca liga — se um dia ligar,
/// a saída daquele pane some da tela sem ninguém acusar.
const KNOWN_NOTIFICATIONS: &[&[u8]] = &[
    b"%begin",
    b"%end",
    b"%error",
    b"%client-detached",
    b"%client-session-changed",
    b"%config-error",
    b"%continue",
    b"%extended-output",
    b"%layout-change",
    b"%message",
    b"%pane-mode-changed",
    b"%paste-buffer-changed",
    b"%paste-buffer-deleted",
    b"%pause",
    b"%session-changed",
    b"%session-renamed",
    b"%session-window-changed",
    b"%sessions-changed",
    b"%subscription-changed",
    b"%unlinked-window-add",
    b"%unlinked-window-close",
    b"%unlinked-window-renamed",
    b"%window-add",
    b"%window-close",
    b"%window-pane-changed",
    b"%window-renamed",
];

/// `value escapes non-printable characters and backslash as octal \xxx`
/// (`man tmux`, CONTROL MODE). Como a própria contrabarra vem escapada, toda
/// contrabarra no valor abre um escape de três dígitos — não há caso ambíguo.
fn unescape(value: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(value.len());
    let mut i = 0;
    while i < value.len() {
        if value[i] != b'\\' {
            out.push(value[i]);
            i += 1;
            continue;
        }
        let digits = value.get(i + 1..i + 4)?;
        let mut byte: u32 = 0;
        for &digit in digits {
            if !(b'0'..=b'7').contains(&digit) {
                return None;
            }
            byte = byte * 8 + u32::from(digit - b'0');
        }
        out.push(u8::try_from(byte).ok()?);
        i += 4;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outputs(events: Vec<ControlEvent>) -> Vec<u8> {
        let mut out = Vec::new();
        for event in events {
            if let ControlEvent::Output(bytes) = event {
                out.extend_from_slice(&bytes);
            }
        }
        out
    }

    #[test]
    fn output_de_um_pane_vira_os_bytes_que_o_programa_escreveu() {
        let mut decoder = ControlDecoder::new();
        let events = decoder.feed(b"%output %0 sh-3.2$ \n");
        assert_eq!(outputs(events), b"sh-3.2$ ");
    }

    /// Medido no tmux 3.7c local: o escape é octal, cobre a contrabarra e o
    /// não-imprimível, e deixa o UTF-8 passar cru.
    #[test]
    fn o_escape_octal_do_tmux_volta_ao_byte_original() {
        let mut decoder = ControlDecoder::new();
        let events = decoder.feed(b"%output %0 printf 'a\\134\\134b\xc3\xa9\\134r'\\015\\012\n");
        assert_eq!(
            outputs(events),
            "printf 'a\\\\bé\\r'\r\n".as_bytes(),
            "a contrabarra sai como \\134 e o ESC/CR/LF como \\033/\\015/\\012"
        );
    }

    #[test]
    fn payload_partido_entre_duas_leituras_sai_inteiro_e_so_no_fim_da_linha() {
        let mut decoder = ControlDecoder::new();
        assert_eq!(
            outputs(decoder.feed(b"%output %0 meta")),
            b"",
            "sem a quebra de linha o payload ainda pode crescer"
        );
        assert_eq!(outputs(decoder.feed(b"de\\040linha\n")), b"metade linha");
    }

    #[test]
    fn payload_partido_no_meio_do_escape_octal_sai_inteiro() {
        let mut decoder = ControlDecoder::new();
        let mut seen = Vec::new();
        for chunk in [&b"%output %0 a\\0"[..], &b"1"[..], &b"5b\n"[..]] {
            seen.extend_from_slice(&outputs(decoder.feed(chunk)));
        }
        assert_eq!(seen, b"a\rb");
    }

    /// Rajada de partida capturada do tmux 3.7c local em 2026-09-17.
    #[test]
    fn a_rajada_de_notificacoes_da_partida_nao_chega_a_tela() {
        let mut decoder = ControlDecoder::new();
        let events = decoder.feed(
            b"%begin 1789691623 279 0\n%end 1789691623 279 0\n\
              %window-add @0\n%sessions-changed\n\
              %session-changed $0 tyba-blk1-probe\n%window-renamed @0 zsh\n\
              %output %0 sh-3.2$ \n%layout-change @0 a87d,100x30,0,0,0 a87d,100x30,0,0,0 *\n\
              %client-detached /dev/ttys001\n%subscription-changed n $0 @0 0 %0 : v\n\
              %pause %0\n%continue %0\n",
        );
        assert_eq!(outputs(events.clone()), b"sh-3.2$ ");
        assert_eq!(
            events
                .iter()
                .filter(|e| !matches!(e, ControlEvent::Output(_)))
                .count(),
            0,
            "nada além da saída do pane: {events:?}"
        );
    }

    #[test]
    fn notificacao_desconhecida_e_anunciada_e_o_fluxo_segue() {
        let mut decoder = ControlDecoder::new();
        let events = decoder.feed(b"%algo-do-futuro %0 42\n%output %0 vivo\n");
        assert_eq!(
            events,
            vec![
                ControlEvent::Notification("%algo-do-futuro %0 42".into()),
                ControlEvent::Output(b"vivo".to_vec()),
            ]
        );
    }

    #[test]
    fn escape_quebrado_nao_vira_saida_e_a_linha_seguinte_ainda_chega() {
        let mut decoder = ControlDecoder::new();
        let events = decoder.feed(b"%output %0 a\\09\n%output %0 depois\n");
        assert_eq!(outputs(events.clone()), b"depois");
        assert!(
            matches!(&events[0], ControlEvent::Unparsed(line) if line.contains("a\\09")),
            "{events:?}"
        );
    }

    #[test]
    fn linha_maior_que_o_teto_e_descartada_e_o_decodificador_ressincroniza() {
        let mut decoder = ControlDecoder::new();
        let mut gigante = b"%output %0 ".to_vec();
        gigante.resize(MAX_LINE + 4096, b'x');
        let events = decoder.feed(&gigante);
        assert!(
            matches!(events.as_slice(), [ControlEvent::Unparsed(_)]),
            "a linha é descartada assim que estoura o teto: {events:?}"
        );
        assert_eq!(
            outputs(decoder.feed(b"resto-da-linha-gigante\n%output %0 vivo\n")),
            b"vivo",
            "o resto da linha estourada é jogado fora até a quebra de linha"
        );
    }

    /// Capturado do tmux 3.7c local: `send-keys` num alvo inexistente devolve
    /// a explicação dentro de um `%error`.
    #[test]
    fn corpo_de_bloco_de_comando_nunca_chega_a_tela() {
        let mut decoder = ControlDecoder::new();
        let events = decoder.feed(
            b"%begin 1789691627 292 1\ncan't find pane: nao-existe\n%error 1789691627 292 1\n\
              %output %0 vivo\n",
        );
        assert_eq!(events, vec![ControlEvent::Output(b"vivo".to_vec())]);
    }

    #[test]
    fn com_a_captura_armada_o_corpo_do_bloco_vira_a_tela_uma_vez_so() {
        let mut decoder = ControlDecoder::new();
        decoder.link().arm_capture();
        let events = decoder.feed(
            // O bloco vazio do `refresh-client` que corre na frente não
            // desarma: quem desarma é o primeiro bloco COM corpo.
            b"%begin 1 10 1\n%end 1 10 1\n\
              %begin 1 11 1\n$ echo oi\noi\n%end 1 11 1\n\
              %begin 1 12 1\nnao sou captura\n%end 1 12 1\n",
        );
        assert_eq!(outputs(events), b"$ echo oi\r\noi\r\n");
    }

    #[test]
    fn captura_que_falha_desarma_sem_pintar_o_erro_na_tela() {
        let mut decoder = ControlDecoder::new();
        decoder.link().arm_capture();
        let events = decoder.feed(
            b"%begin 1 10 1\ncan't find pane: x\n%error 1 10 1\n\
              %begin 1 11 1\nnao sou captura\n%end 1 11 1\n",
        );
        assert_eq!(outputs(events), b"");
    }

    #[test]
    fn linha_do_corpo_que_parece_um_fim_de_bloco_alheio_nao_fecha_o_nosso() {
        let mut decoder = ControlDecoder::new();
        decoder.link().arm_capture();
        let events = decoder.feed(b"%begin 1 10 1\n%end 1 9 1\ncorpo\n%end 1 10 1\n");
        assert_eq!(
            outputs(events),
            b"%end 1 9 1\r\ncorpo\r\n",
            "o bloco fecha no %end de MESMO numero, nao no primeiro que aparecer"
        );
    }

    const MARKER: &str = "\x1b]633;P;tyba-ctl=0123456789abcdef0123456789abcdef\x07";

    #[test]
    fn antes_do_marco_o_byte_e_cru_e_depois_dele_e_protocolo() {
        let mut decoder = ControlDecoder::with_marker(Some(MARKER));
        assert!(!decoder.link().in_control());
        let banner = b"Last login: Wed\r\n\x1b]633;P;tyba-ssh-login=abc\x07";
        assert_eq!(
            outputs(decoder.feed(banner)),
            banner,
            "o marco de login do Cano atravessa inteiro, como o CanoWatch espera"
        );
        let events =
            decoder.feed(format!("{MARKER}%output %0 ja\\040e\\040protocolo\n").as_bytes());
        assert_eq!(outputs(events), b"ja e protocolo");
        assert!(decoder.link().in_control());
    }

    #[test]
    fn o_marco_partido_entre_leituras_nao_vaza_pela_tela() {
        let mut decoder = ControlDecoder::with_marker(Some(MARKER));
        let (head, tail) = MARKER.split_at(10);
        assert_eq!(
            outputs(decoder.feed(format!("senha? {head}").as_bytes())),
            b"senha? ",
            "o que ainda pode virar o marco fica retido, o resto sai na hora"
        );
        let events = decoder.feed(format!("{tail}%output %0 oi\n").as_bytes());
        assert_eq!(outputs(events), b"oi");
    }

    #[test]
    fn prefixo_que_nao_era_o_marco_sai_pela_tela() {
        let mut decoder = ControlDecoder::with_marker(Some(MARKER));
        assert_eq!(outputs(decoder.feed(b"\x1b]633;P")), b"");
        assert_eq!(
            outputs(decoder.feed(b";tyba-ssh-login=abc\x07pronto")),
            b"\x1b]633;P;tyba-ssh-login=abc\x07pronto",
            "o marco de login compartilha o comeco com o de controle e nao pode sumir"
        );
        assert!(!decoder.link().in_control());
    }

    #[test]
    fn send_keys_manda_byte_a_byte_em_hexadecimal() {
        assert_eq!(
            send_keys("tyba-a3f", "é\r".as_bytes()),
            "send-keys -t 'tyba-a3f' -H c3 a9 0d",
            "-H espera um numero hexadecimal por byte; UTF-8 vai byte a byte"
        );
        assert_eq!(send_keys("tyba-a3f", b""), "send-keys -t 'tyba-a3f' -H");
    }

    /// O tmux tem lexer próprio e entende aspas simples no estilo do sh —
    /// conferido no 3.7c local com uma sessão chamada `ty ba-'q'-probe`.
    #[test]
    fn o_alvo_e_citado_para_o_lexer_do_tmux() {
        assert_eq!(
            send_keys("ty ba-'q'", b"A"),
            "send-keys -t 'ty ba-'\\''q'\\''' -H 41"
        );
    }

    #[test]
    fn refresh_client_declara_o_tamanho_do_cliente_de_controle() {
        assert_eq!(refresh_client(100, 30), "refresh-client -C 100x30");
    }

    #[test]
    fn capture_pane_pede_a_tela_com_atributos_e_o_historico_da_regra_5() {
        assert_eq!(
            capture_pane("tyba-a3f", CAPTURE_LINES),
            "capture-pane -p -e -J -t 'tyba-a3f' -S -5000"
        );
    }

    #[test]
    fn o_eco_do_proprio_comando_nao_gasta_a_queixa_da_sessao() {
        let mut decoder = ControlDecoder::new();
        let events = decoder.feed(b"send-keys -t 'x' -H 41\r\n\r\n%output %0 oi\n");
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, ControlEvent::Unparsed(_)))
                .count(),
            1,
            "o eco do comando é uma linha estranha só; a linha vazia não conta: {events:?}"
        );
    }

    #[test]
    fn exit_do_cliente_de_controle_vira_evento_proprio() {
        let mut decoder = ControlDecoder::new();
        assert_eq!(decoder.feed(b"%exit\n"), vec![ControlEvent::Exit]);
        assert_eq!(
            decoder.feed(b"%exit server exited\n"),
            vec![ControlEvent::Exit]
        );
    }
}
