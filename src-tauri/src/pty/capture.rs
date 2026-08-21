//! A máquina de estados do ciclo de comando: `633;E` → `133;C` → `133;D`.
//!
//! Mora aqui, e não dentro da thread emitter, pela mesma razão do
//! `history::Tracker`: é onde estão as decisões que importam (o que entra no
//! bloco, o que vai para a tela ao vivo, o que acontece com o comando que nunca
//! terminou), e dentro de uma closure de trezentas linhas nenhuma delas era
//! testável sem GUI e sem pty.
//!
//! O contrato é `on_chunk` → lista de ações. A máquina não toca tela, canal nem
//! `AppHandle`: quem executa é o emitter, na ordem em que as ações saem.

use std::ops::Range;
use std::path::PathBuf;

use crate::blocks::Capture;
use crate::history::{CommandRecord, Tracker};
use crate::status::{OscParser, ShellEvent};

use super::{SessionCommandPayload, SessionCwdPayload};

fn marker_at(chunk: &[u8], mark: &[u8]) -> Option<usize> {
    chunk.windows(mark.len()).position(|w| w == mark)
}

/// Onde começa o `133;C` dentro do chunk, se estiver inteiro nele.
///
/// Busca a sequência crua em vez de pedir a posição ao `OscParser`: ele existe
/// para dizer O QUE aconteceu, e dar-lhe offsets por causa de um recorte de
/// tela sujaria a peça mais frágil do sistema. Marcador partido entre dois
/// chunks devolve `None`, e quem chama tem a rede do evento.
pub(super) fn command_start_at(chunk: &[u8]) -> Option<usize> {
    marker_at(chunk, b"\x1b]133;C")
}

/// Onde começa o `133;D` — o fim da saída dentro do chunk.
fn command_end_at(chunk: &[u8]) -> Option<usize> {
    marker_at(chunk, b"\x1b]133;D")
}

/// O bloco fechado, sem o que é do terminal e não do comando: `cols`, `rows` e
/// o id da sessão entram no emitter.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct FinishedBlock {
    pub command: String,
    pub exit_code: Option<i32>,
    pub cwd: Option<String>,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub bytes: Vec<u8>,
    /// Linhas que o teto de captura comeu enquanto o comando rodava.
    pub dropped: usize,
    pub alt_screen: bool,
}

/// Fotografia do comando em execução, para o checkpoint.
pub(super) struct Snapshot {
    pub command: String,
    pub started_at_ms: i64,
    pub bytes: Vec<u8>,
}

/// O que o emitter faz depois de entregar o chunk. A ordem é significativa.
#[derive(Debug, PartialEq)]
pub(super) enum Action {
    /// Trecho do chunk que vai para a tela ao vivo.
    Live(Range<usize>),
    /// O corte do eco: joga fora o que está na fila da tela ao vivo, limpa a
    /// tela e emenda o trecho do chunk.
    LiveRestart(Range<usize>),
    /// Limpa só o estado de tela do core — é ele que reata a sessão ao trocar
    /// de aba.
    ClearCoreScreen,
    /// O bloco assumiu a saída: limpa o core e a tela ao vivo.
    ResetScreen,
    /// O ciclo de comando INTEIRO — rodando, ocioso, em continuação —, num
    /// payload só.
    ///
    /// Armadilha que custou caro: `running` e `continuation` já foram ações
    /// separadas, e o emitter mandava cada uma como um `SessionCommandPayload`
    /// completo. Como o front guarda o payload por sessão substituindo o
    /// objeto (`{...prev, [id]: payload}`), o último a escrever ganha — e um
    /// `Continuation(false)` empurrado logo depois do `Running` apagava o
    /// comando em execução. `continuation` é CAMPO deste estado, não fato
    /// independente; quem emite um campo emite todos.
    CommandState(SessionCommandPayload),
    Record(CommandRecord),
    Wipe,
    Finalize(FinishedBlock),
    PromptMode(bool),
    Cwd(SessionCwdPayload),
}

/// Teto do que se engole esperando a quebra de linha do eco.
///
/// Nada é para sempre: uma linha de comando não passa de um buffer de leitura,
/// e se a quebra nunca vier — caminho que ninguém previu — a tela volta a
/// receber bytes em vez de escurecer de vez.
const MAX_ECHO_BYTES: usize = 8 * 1024;

/// Onde está a linha depois do Enter. O shell não marca `PS2` com OSC nenhum —
/// o estado é inferido pela AUSÊNCIA, e por isso precisa de três casas, não de
/// um booleano.
///
/// A distinção entre as duas últimas é o achado do review: enquanto "eco
/// terminou" e "o shell está em continuação" eram a mesma flag, todo `ls -la`
/// era anunciado como continuação no intervalo entre o Enter e o `133;C`, e a
/// linha do TYBA dizia "o shell espera o resto do comando" para um comando de
/// uma linha só.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Submission {
    /// Prompt primário, ou comando em execução: nada pendente.
    Idle,
    /// O eco terminou e o shell ainda não disse o que fez com a linha. Não se
    /// anuncia NADA daqui: é o intervalo em que o comando comum também está.
    AwaitingShell,
    /// O shell pintou outra coisa em vez de rodar — é o `PS2`. Fica até um
    /// marcador (`633;E`, `133;C`, `133;B`) ou o fim do comando desfazer.
    Continuing,
}

/// O shell PINTOU alguma coisa aqui, ou só mexeu no terminal?
///
/// É a pergunta que separa `ls -la` de `for … do`: entre o Enter e o `633;E`
/// do preexec o shell não desenha glifo nenhum — o `\e[?2004l` com que o `zle`
/// fecha o bracketed paste é controle, não texto. Já o `PS2` é texto por
/// definição (`for> `, `dquote> `). Contar byte cru em vez de glifo daria
/// continuação a todo comando cujo desligamento do bracketed paste caísse num
/// chunk sozinho.
///
/// Sem estado entre chamadas de propósito: o `HoldBack` da thread de leitura
/// entrega chunks que só terminam fora de sequência de escape, então nenhuma
/// delas chega partida aqui.
fn paints_glyph(bytes: &[u8]) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Scan {
        Ground,
        Esc,
        Csi,
        /// Corpo de OSC/DCS/APC/PM/SOS — termina em `BEL` ou `ST`.
        StringBody,
    }

    let mut state = Scan::Ground;
    for &byte in bytes {
        state = match (state, byte) {
            // `ESC` aborta o que estava em curso, em qualquer estado — é
            // também como o `ST` (`ESC \`) fecha o corpo de string.
            (_, 0x1b) => Scan::Esc,
            (Scan::Esc, b'[') => Scan::Csi,
            (Scan::Esc, b']' | b'P' | b'X' | b'^' | b'_') => Scan::StringBody,
            (Scan::Esc, _) => Scan::Ground,
            (Scan::Csi, 0x40..=0x7e) => Scan::Ground,
            (Scan::Csi, _) => Scan::Csi,
            (Scan::StringBody, 0x07) => Scan::Ground,
            (Scan::StringBody, _) => Scan::StringBody,
            // Fora de sequência, o que não é controle vira glifo. `\r`, `\n` e
            // `\x7f` mexem no cursor, não pintam.
            (Scan::Ground, 0x20..=0x7e) | (Scan::Ground, 0x80..=0xff) => return true,
            (Scan::Ground, _) => Scan::Ground,
        };
    }
    false
}

pub(super) struct CaptureMachine {
    osc: OscParser,
    tracker: Tracker,
    capture: Capture,
    capturing: bool,
    swallow_echo: bool,
    /// Bytes já engolidos à espera da quebra de linha do eco.
    swallowed: usize,
    submission: Submission,
    /// O último estado do ciclo de comando ENTREGUE ao front — o objeto que ele
    /// tem em mãos agora. É daqui que sai todo payload, e é contra ele que a
    /// transição de continuação é comparada: assim o campo viaja junto do
    /// estado certo, em vez de um payload parcial sobrescrever o inteiro.
    command: SessionCommandPayload,
    prompt_mode: Option<bool>,
    agent_match: bool,
    cwd: Option<PathBuf>,
    canonical_cwd: Option<PathBuf>,
}

impl CaptureMachine {
    pub(super) fn new(session_id: String) -> Self {
        Self {
            osc: OscParser::new(),
            tracker: Tracker::new(session_id),
            capture: Capture::default(),
            capturing: false,
            swallow_echo: false,
            swallowed: 0,
            submission: Submission::Idle,
            command: SessionCommandPayload::default(),
            prompt_mode: None,
            agent_match: false,
            cwd: None,
            canonical_cwd: None,
        }
    }

    pub(super) fn on_chunk(&mut self, chunk: &[u8], now_ms: i64, alt_screen: bool) -> Vec<Action> {
        let mut actions = Vec::new();

        // Até onde a varredura de marcadores já foi neste chunk. Sem o cursor,
        // dois comandos no mesmo chunk achariam ambos o PRIMEIRO `133;C`, e o
        // segundo bloco nasceria com a saída do primeiro.
        let mut scan_from = 0;

        // A saída do comando que JÁ estava rodando entra antes dos eventos, e
        // para no `133;D` quando ele cai neste mesmo chunk: o que vem depois do
        // marcador é prompt novo, não saída.
        if self.capturing {
            let end = command_end_at(chunk).unwrap_or(chunk.len());
            self.push_capture(&chunk[..end], alt_screen);
            scan_from = end;
        }

        // O que o SHELL escreveu neste chunk, já sem o eco do que foi digitado.
        // É a matéria-prima do assentamento da submissão lá embaixo: entre o
        // Enter e o preexec o shell não pinta nada, e o `PS2` é a exceção.
        let shell_reply: &[u8];

        // A tela ao vivo é decidida com o estado de ANTES dos eventos: o que
        // está chegando é o eco do comando que acabou de ser submetido.
        if self.swallow_echo {
            // O eco cai entre `133;B` e `133;C`, e o cartão já mostra o
            // comando — na tela ao vivo ele aparecia duas vezes.
            //
            // O fim do eco é a QUEBRA DE LINHA que o tty devolve pelo Enter,
            // não o `133;C`. Esperar pelo `133;C` era o bug: comando incompleto
            // (`for … do`, `while`, `if`, `cat <<EOF`, aspas abertas) não faz o
            // zsh rodar preexec, então `133;C` não vem NUNCA — e a flag ficava
            // ligada indefinidamente. O `for> ` do PS2 sumia, o eco do que se
            // digitava depois sumia, e a tela ficava preta até o próximo
            // comando completo. Todo comando multi-linha caía nisso.
            //
            // Desarmar na quebra de linha cobre qualquer caminho que não emita
            // `133;C`, inclusive os que ninguém previu, e não custa relógio.
            //
            // Mas desarmar o eco e ANUNCIAR continuação são duas perguntas
            // diferentes, e compartilhavam esta flag: aqui a linha só passa a
            // `AwaitingShell` — quem decide que virou `PS2` é o assentamento,
            // no fim do chunk.
            match chunk.iter().position(|b| *b == b'\n') {
                Some(at) => {
                    self.swallow_echo = false;
                    self.submission = Submission::AwaitingShell;
                    shell_reply = &chunk[at + 1..];
                    if at + 1 < chunk.len() {
                        actions.push(Action::Live(at + 1..chunk.len()));
                    }
                }
                None => {
                    // Tudo o que veio é eco: o shell ainda não respondeu nada.
                    shell_reply = &[];
                    self.swallowed += chunk.len();
                    if self.swallowed > MAX_ECHO_BYTES {
                        self.swallow_echo = false;
                        actions.push(Action::Live(0..chunk.len()));
                    }
                }
            }
        } else {
            shell_reply = chunk;
            actions.push(Action::Live(0..chunk.len()));
        }

        for event in self.osc.feed(chunk) {
            match event {
                ShellEvent::CommandLine(cmd) => {
                    // O espaço à esquerda sobrevive ao parser (é o
                    // `ignorespace`); quem classifica o comando quer a linha
                    // limpa.
                    self.agent_match = crate::rich_input::agent_matcher().matches(cmd.trim_start());
                    self.tracker.on_command_line(cmd);
                    // O `633;E` sai do preexec: se ele veio, o shell ACEITOU a
                    // linha e não está em continuação. Zerar aqui, e não só no
                    // `133;C`, evita anunciar continuação no intervalo entre os
                    // dois marcadores.
                    self.submission = Submission::Idle;
                }
                ShellEvent::CommandStart => {
                    self.tracker.on_start(now_ms);
                    // Bloco só existe com o modo prompt: sem ele o prompt é
                    // desenhado (e repintado) dentro da região, e o recorte
                    // sairia errado. Ver features/terminal-blocks.
                    self.capturing = self.prompt_mode == Some(true);
                    let _ = self.capture.take();
                    // Rede: se o marcador veio partido entre dois chunks, o
                    // corte por posição não achou nada e a tela ao vivo ficaria
                    // muda o comando inteiro.
                    self.swallow_echo = false;
                    // A saída vem grudada no `133;C` — o mesmo fato que já
                    // obrigava o corte da tela ao vivo. Sem recortar aqui
                    // também, o bloco nasce VAZIO sempre que marcador, saída e
                    // `133;D` caem no mesmo chunk: a captura só ligava depois
                    // de o chunk inteiro ter passado.
                    //
                    // Marcador partido entre dois chunks não é achado: aí a
                    // captura começa onde a varredura estava, e o preço é o
                    // rabo da sequência OSC dentro do bloco — que o `vt100` do
                    // finalize come.
                    let from = command_start_at(&chunk[scan_from..])
                        .map(|at| scan_from + at)
                        .unwrap_or(scan_from);
                    let end = command_end_at(&chunk[from..])
                        .map(|at| from + at)
                        .unwrap_or(chunk.len());
                    scan_from = end;
                    self.submission = Submission::Idle;
                    if self.capturing {
                        self.push_capture(&chunk[from..end], alt_screen);
                        // O estado de tela do core é o que reata a sessão ao
                        // trocar de aba. Sem limpá-lo junto com o xterm, o
                        // histórico inteiro voltaria dentro do cartão ativo no
                        // reattach.
                        actions.push(Action::ClearCoreScreen);
                        // E a tela ao vivo recomeça no marcador. Isto morava no
                        // ramo do eco, e por isso dependia de ele ter achado o
                        // `133;C` — agora o eco desarma sozinho na quebra de
                        // linha, e quem manda limpar é o comando começar, que é
                        // o fato que de verdade justifica a limpeza.
                        //
                        // O corte é no MARCADOR, não no chunk: a saída vem
                        // grudada nele, e limpar o chunk inteiro levaria o
                        // começo dela junto. A limpeza vai no mesmo canal dos
                        // bytes — pelo evento de comando ela chegava ANTES do
                        // eco, e limpava uma tela onde ele ainda não estava.
                        actions.push(Action::LiveRestart(from..chunk.len()));
                    }
                    actions.push(self.announce(SessionCommandPayload {
                        command: self.tracker.command().map(str::to_string),
                        running: true,
                        agent_match: self.agent_match,
                        continuation: false,
                    }));
                }
                // `133;D` traz exit code; prompt novo sem ele é comando que não
                // terminou (Ctrl+C no meio, shell reiniciado) — grava sem
                // código em vez de sumir com ele.
                ShellEvent::CommandEnd(code) => actions.extend(self.finish(Some(code), now_ms)),
                ShellEvent::PromptStart => actions.extend(self.finish(None, now_ms)),
                ShellEvent::InputStart => {
                    // Daqui até a quebra de linha só vem o eco do que foi
                    // submetido. Fora do modo prompt quem digitou foi o
                    // terminal, e aí o eco é a própria digitação.
                    self.swallow_echo = self.prompt_mode == Some(true);
                    self.swallowed = 0;
                    // `133;B` é o PS1, ou seja prompt PRIMÁRIO: o shell está
                    // pronto para comando novo, não continuando um.
                    self.submission = Submission::Idle;
                }
                ShellEvent::PromptMode(on) => {
                    self.prompt_mode = Some(on);
                    actions.push(Action::PromptMode(on));
                }
                ShellEvent::Cwd(path) => {
                    if self.cwd.as_deref() != Some(path.as_path()) {
                        self.cwd = Some(path.clone());
                        let payload = SessionCwdPayload::of(&path);
                        // O histórico guarda o caminho CANÔNICO. No macOS
                        // `/tmp` é symlink para `/private/tmp`: gravar o cru e
                        // consultar pelo canônico (que é o que o front tem)
                        // faria o escopo nunca casar, em silêncio.
                        self.canonical_cwd = Some(PathBuf::from(&payload.canonical));
                        actions.push(Action::Cwd(payload));
                    }
                }
            }
        }

        // O ASSENTAMENTO da submissão. O shell não marca prompt de continuação
        // — o `PS2` não emite OSC nenhum —, então o sinal é o que ele fez em
        // vez de rodar: pintou. Nenhum marcador desfez a submissão neste chunk
        // E veio glifo depois da quebra de linha do eco: é `PS2`.
        //
        // A quebra de linha sozinha não serve, e era esse o bug: ela chega no
        // mesmo instante do Enter, muito antes de o shell responder, e com ela
        // todo `ls -la` passava pela tela anunciado como continuação.
        if self.submission == Submission::AwaitingShell && paints_glyph(shell_reply) {
            self.submission = Submission::Continuing;
        }

        // `continuation` viaja no payload INTEIRO, montado sobre o estado que o
        // front já tem: emitido sozinho, com `running: false` dentro, ele
        // apagava o comando em execução (o front substitui o objeto do id).
        // Quando o `133;C` deste mesmo chunk já reescreveu o estado, não sobra
        // transição nenhuma para anunciar — e é por isso que não há aqui
        // nenhuma reordenação de ações.
        //
        // `running` manda: os dois campos são estados diferentes do mesmo
        // ciclo, e o payload documenta que nunca vêm juntos. Um shell que
        // emitisse `133;B` sem fechar o comando anterior chegaria aqui com os
        // dois — a invariante se sustenta no core, não na boa vontade do hook.
        let continuation = self.submission == Submission::Continuing && !self.command.running;
        if continuation != self.command.continuation {
            let mut next = self.command.clone();
            next.continuation = continuation;
            actions.push(self.announce(next));
        }
        actions
    }

    /// Guarda o estado que o front passa a ter e devolve a ação que o leva.
    /// Toda saída de ciclo de comando passa por aqui — é o único lugar onde
    /// `self.command` e o que o webview tem em mãos podem divergir.
    fn announce(&mut self, next: SessionCommandPayload) -> Action {
        self.command = next.clone();
        Action::CommandState(next)
    }

    /// O PTY morreu com um comando em voo (`exec sleep 3`, `kill -9 $$`, sessão
    /// morta pela UI).
    ///
    /// Sem isto ninguém fecha o bloco: o único `finalize` do crate está no braço
    /// do `133;D`, que nunca chega. O cartão fica pulsando para sempre e a linha
    /// do TYBA nunca volta a aceitar comando, porque o front devolve o teclado
    /// ao terminal enquanto achar que há comando rodando.
    ///
    /// Exit code é `None`: bloco fechado por EOF tem código DESCONHECIDO, e
    /// desconhecido não é fracasso (ADR de 2026-08-15).
    pub(super) fn on_eof(&mut self, now_ms: i64) -> Vec<Action> {
        self.finish(None, now_ms)
    }

    /// Fotografia do que já saiu, quando há bloco em voo. Não consome: o
    /// comando segue.
    pub(super) fn checkpoint(&self, now_ms: i64) -> Option<Snapshot> {
        if !self.capturing || self.capture.is_alt_screen() {
            return None;
        }
        Some(Snapshot {
            command: self.tracker.command().unwrap_or_default().to_string(),
            started_at_ms: self.tracker.started_at().unwrap_or(now_ms),
            bytes: self.capture.snapshot(),
        })
    }

    fn push_capture(&mut self, bytes: &[u8], alt_screen: bool) {
        self.capture.push(bytes);
        if alt_screen {
            self.capture.saw_alt_screen();
        }
    }

    fn finish(&mut self, exit_code: Option<i32>, now_ms: i64) -> Vec<Action> {
        let mut actions = Vec::new();
        let command = self
            .tracker
            .command()
            .map(str::to_string)
            .unwrap_or_default();
        let started = self.tracker.started_at().unwrap_or(now_ms);
        if let Some(record) = self
            .tracker
            .on_end(exit_code, now_ms, self.canonical_cwd.as_deref())
        {
            actions.push(Action::Record(record));
        }
        if self.capturing {
            // `vim`, `bat`, `htop`: a saída não vai, o bloco vai. Descartar o
            // comando inteiro apagaria do registro algo que foi executado.
            let alt_screen = self.capture.is_alt_screen();
            let dropped = self.capture.dropped();
            let bytes = self.capture.take();
            if crate::blocks::wipes_the_screen(&command) {
                // Em modo bloco a tela É a lista de blocos: limpar sem levá-los
                // junto é o `clear` não fazer nada, que foi o que aconteceu.
                //
                // O bloco do próprio `clear` continua — some o que veio antes, e
                // fica quem mandou sumir. Sem ele a lista fica vazia e o painel
                // vira um preto sem explicação.
                actions.push(Action::Wipe);
            }
            if !command.is_empty() {
                actions.push(Action::Finalize(FinishedBlock {
                    command,
                    exit_code,
                    cwd: self
                        .canonical_cwd
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned()),
                    started_at_ms: started,
                    finished_at_ms: now_ms,
                    bytes: if alt_screen { Vec::new() } else { bytes },
                    dropped: if alt_screen { 0 } else { dropped },
                    alt_screen,
                }));
            }
            // O bloco assume a saída aqui. Sem esvaziar a tela ao vivo, a mesma
            // saída fica desenhada duas vezes: no cartão que acabou de nascer e
            // embaixo dele, no terminal.
            actions.push(Action::ResetScreen);
        }
        self.capturing = false;
        let _ = self.capture.take();
        self.agent_match = false;
        // O estado ocioso já diz ao front `continuation: false`; o interno tem
        // de acompanhar, senão a próxima transição não seria detectada.
        self.submission = Submission::Idle;
        actions.push(self.announce(SessionCommandPayload::default()));
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn b64(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    /// Uma máquina já no modo prompt — é a condição para existir bloco.
    fn in_prompt_mode() -> CaptureMachine {
        let mut machine = CaptureMachine::new("s1".into());
        machine.on_chunk(b"\x1b]633;P;tyba-prompt=1\x07", 1_000, false);
        machine
    }

    fn finalized(actions: Vec<Action>) -> Option<FinishedBlock> {
        actions.into_iter().find_map(|action| match action {
            Action::Finalize(block) => Some(block),
            _ => None,
        })
    }

    /// O bloco como o usuário o vê. Os marcadores OSC continuam nos bytes — o
    /// `vt100` do finalize os come —, então asserção byte a byte testaria o
    /// encanamento em vez do conteúdo.
    fn text_of(block: &FinishedBlock) -> Vec<String> {
        crate::blocks::extract_lines(&block.bytes, 80, 24)
            .0
            .into_iter()
            .map(|line| line.text)
            .collect()
    }

    /// Fixture sintética, escrita à mão: é o formato que o hook de
    /// `session/tyba-zsh-rc.sh` emite, com o comando em base64 no `633;E`.
    fn one_shot_chunk(command: &str, output: &str) -> Vec<u8> {
        format!(
            "\x1b]633;E;{}\x07\x1b]133;C\x07{output}\x1b]133;D;0\x07\x1b]133;A\x07",
            b64(command)
        )
        .into_bytes()
    }

    #[test]
    fn output_glued_to_the_start_marker_survives_in_the_same_chunk() {
        // O caso que perdia TUDO: marcador, saída e `133;D` no mesmo chunk. A
        // captura ligava só depois de o chunk inteiro ter passado, e o bloco
        // nascia vazio. Medido em 25-33% dos comandos com a máquina ocupada.
        let mut machine = in_prompt_mode();
        let actions = machine.on_chunk(&one_shot_chunk("echo hi", "hi\r\n"), 2_000, false);
        let block = finalized(actions).expect("bloco precisa nascer");
        assert_eq!(block.command, "echo hi");
        assert_eq!(block.exit_code, Some(0));
        assert_eq!(text_of(&block), ["hi"], "saída perdida: {block:?}");
    }

    #[test]
    fn output_split_across_chunks_still_lands_in_the_block() {
        let mut machine = in_prompt_mode();
        machine.on_chunk(
            format!("\x1b]633;E;{}\x07\x1b]133;C\x07", b64("ls")).as_bytes(),
            2_000,
            false,
        );
        machine.on_chunk(b"um\r\ndois\r\n", 2_100, false);
        let block = finalized(machine.on_chunk(b"\x1b]133;D;0\x07", 2_200, false))
            .expect("bloco precisa nascer");
        assert_eq!(text_of(&block), ["um", "dois"]);
    }

    #[test]
    fn what_comes_after_the_end_marker_is_not_output() {
        // Depois do `133;D` vem prompt novo. Sem o corte simétrico, o desenho
        // do prompt entraria no bloco do comando anterior.
        let mut machine = in_prompt_mode();
        let mut chunk = one_shot_chunk("echo hi", "hi\r\n");
        chunk.extend_from_slice(b"PROMPT-NOVO");
        let block = finalized(machine.on_chunk(&chunk, 2_000, false)).expect("bloco");
        assert_eq!(
            text_of(&block),
            ["hi"],
            "o prompt novo não é saída do comando"
        );
    }

    #[test]
    fn a_new_command_in_the_same_chunk_captures_only_its_own_output() {
        let mut machine = in_prompt_mode();
        let mut chunk = one_shot_chunk("primeiro", "saida-um\r\n");
        chunk.extend_from_slice(&one_shot_chunk("segundo", "saida-dois\r\n"));
        let blocks: Vec<FinishedBlock> = machine
            .on_chunk(&chunk, 2_000, false)
            .into_iter()
            .filter_map(|action| match action {
                Action::Finalize(block) => Some(block),
                _ => None,
            })
            .collect();
        assert_eq!(blocks.len(), 2, "dois comandos, dois blocos");
        assert_eq!(text_of(&blocks[0]), ["saida-um"]);
        assert_eq!(text_of(&blocks[1]), ["saida-dois"]);
    }

    #[test]
    fn without_prompt_mode_there_is_no_block_but_the_command_still_runs() {
        // Fora do modo prompt o `PS1` é desenhado dentro da região e o recorte
        // sairia errado — mas o front ainda precisa saber que há comando.
        let mut machine = CaptureMachine::new("s1".into());
        let actions = machine.on_chunk(&one_shot_chunk("echo hi", "hi\r\n"), 2_000, false);
        assert!(actions
            .iter()
            .any(|a| matches!(a, Action::CommandState(state) if state.running)));
        assert!(actions.contains(&idle()));
        assert!(!actions.iter().any(|a| matches!(a, Action::Finalize(_))));
    }

    #[test]
    fn eof_mid_command_closes_the_block_and_says_the_session_stopped() {
        // `exec sleep 3`: o `133;D` nunca chega. Sem fechar aqui, o cartão fica
        // pulsando para sempre e a linha do TYBA nunca volta.
        let mut machine = in_prompt_mode();
        machine.on_chunk(
            format!("\x1b]633;E;{}\x07\x1b]133;C\x07", b64("exec sleep 3")).as_bytes(),
            2_000,
            false,
        );
        machine.on_chunk(b"comecou\r\n", 2_100, false);

        let actions = machine.on_eof(9_000);

        assert!(
            actions.contains(&idle()),
            "front precisa ouvir running: false"
        );
        let block = finalized(actions).expect("bloco em voo precisa ser fechado");
        assert_eq!(block.command, "exec sleep 3");
        // Desconhecido não é fracasso: ADR de 2026-08-15.
        assert_eq!(block.exit_code, None);
        assert_eq!(block.finished_at_ms, 9_000);
        assert_eq!(text_of(&block), ["comecou"]);
    }

    #[test]
    fn eof_with_the_session_idle_only_reports_idle() {
        let mut machine = in_prompt_mode();
        let actions = machine.on_eof(9_000);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], idle());
    }

    #[test]
    fn eof_after_the_command_already_ended_does_not_duplicate_the_block() {
        let mut machine = in_prompt_mode();
        machine.on_chunk(&one_shot_chunk("echo hi", "hi\r\n"), 2_000, false);
        assert!(finalized(machine.on_eof(9_000)).is_none());
    }

    #[test]
    fn the_echo_is_cut_at_the_marker_not_at_the_chunk() {
        let mut machine = in_prompt_mode();
        // `133;B` abre a zona de eco; o que vem depois é o comando digitado.
        machine.on_chunk(b"\x1b]133;B\x07", 2_000, false);
        let chunk = b"echo hi\r\n\x1b]133;C\x07hi\r\n".to_vec();
        let actions = machine.on_chunk(&chunk, 2_100, false);
        let at = command_start_at(&chunk).expect("marcador inteiro");
        assert!(
            actions.contains(&Action::LiveRestart(at..chunk.len())),
            "a tela ao vivo perderia o começo da saída: {actions:?}"
        );
    }

    #[test]
    fn the_echo_cut_does_not_steal_the_output_from_the_block() {
        let mut machine = in_prompt_mode();
        machine.on_chunk(b"\x1b]133;B\x07", 2_000, false);
        let chunk = format!(
            "echo hi\r\n\x1b]633;E;{}\x07\x1b]133;C\x07hi\r\n\x1b]133;D;0\x07",
            b64("echo hi")
        )
        .into_bytes();
        let block = finalized(machine.on_chunk(&chunk, 2_100, false)).expect("bloco");
        assert_eq!(text_of(&block), ["hi"]);
    }

    /// Junta o que sai para a tela ao vivo, aplicando as ações na ordem —
    /// `LiveRestart` descarta o que já tinha entrado, como o emitter faz.
    fn live_screen(chunk: &[u8], actions: &[Action]) -> String {
        let mut out: Vec<u8> = Vec::new();
        for action in actions {
            match action {
                Action::Live(range) => out.extend_from_slice(&chunk[range.clone()]),
                Action::LiveRestart(range) => {
                    out.clear();
                    out.extend_from_slice(&chunk[range.clone()]);
                }
                Action::ResetScreen => out.clear(),
                _ => {}
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Uma sessão em modo prompt logo depois do `133;B`, que é onde o eco
    /// começa a ser engolido.
    fn at_the_prompt() -> CaptureMachine {
        let mut machine = in_prompt_mode();
        machine.on_chunk(b"\x1b]133;B\x07", 1_000, false);
        machine
    }

    #[test]
    fn an_incomplete_command_does_not_black_out_the_screen() {
        // `for i in 1 2 3; do` + Enter: o zsh não roda preexec, então NENHUM
        // `133;C` é emitido. Enquanto o desarme dependia dele, a flag ficava
        // ligada para sempre e nada mais chegava ao xterm — o `for> ` sumia, o
        // eco do que se digitava depois sumia, e a tela ficava preta.
        let mut machine = at_the_prompt();
        let chunk = b"for i in 1 2 3; do\r\nfor> ";
        let actions = machine.on_chunk(chunk, 2_000, false);
        assert_eq!(live_screen(chunk, &actions), "for> ");
    }

    #[test]
    fn the_screen_keeps_working_for_every_line_of_the_continuation() {
        let mut machine = at_the_prompt();
        machine.on_chunk(b"for i in 1 2 3; do\r\nfor> ", 2_000, false);
        let chunk = b"echo $i\r\nfor> ";
        let actions = machine.on_chunk(chunk, 2_100, false);
        assert_eq!(
            live_screen(chunk, &actions),
            "echo $i\r\nfor> ",
            "a partir da segunda linha o eco é a tela, não duplicata"
        );
    }

    #[test]
    fn the_echo_of_a_complete_command_is_still_hidden() {
        // O outro lado: desarmar cedo demais traz de volta o eco duplicado que
        // o `swallow_echo` existe para evitar.
        let mut machine = at_the_prompt();
        let chunk = format!(
            "ls -la\r\n\x1b]633;E;{}\x07\x1b]133;C\x07total 0\r\n",
            b64("ls -la")
        )
        .into_bytes();
        let actions = machine.on_chunk(&chunk, 2_000, false);
        let screen = live_screen(&chunk, &actions);
        assert!(
            !screen.contains("ls -la"),
            "eco vazou para a tela: {screen:?}"
        );
        assert!(screen.contains("total 0"), "saída sumiu: {screen:?}");
    }

    #[test]
    fn an_echo_split_across_chunks_is_still_swallowed_whole() {
        let mut machine = at_the_prompt();
        let first = b"ls -l";
        assert_eq!(
            live_screen(first, &machine.on_chunk(first, 2_000, false)),
            ""
        );
        let second = b"a\r\nresto";
        assert_eq!(
            live_screen(second, &machine.on_chunk(second, 2_010, false)),
            "resto"
        );
    }

    #[test]
    fn an_echo_that_never_breaks_the_line_gives_up_instead_of_going_dark() {
        // Caminho que ninguém previu: sem quebra de linha, o teto de bytes é o
        // que impede a tela de escurecer para sempre.
        let mut machine = at_the_prompt();
        let plausible = vec![b'x'; MAX_ECHO_BYTES];
        assert_eq!(
            live_screen(&plausible, &machine.on_chunk(&plausible, 2_000, false)),
            "",
            "até o teto ainda pode ser eco de uma linha comprida"
        );
        let past_it = b"agora aparece";
        assert_eq!(
            live_screen(past_it, &machine.on_chunk(past_it, 2_010, false)),
            "agora aparece",
            "passou do teto sem quebra de linha: a tela volta a receber bytes"
        );
    }

    #[test]
    fn the_continuation_is_announced_and_taken_back() {
        let mut machine = at_the_prompt();

        let entered = machine.on_chunk(b"for i in 1 2 3; do\r\nfor> ", 2_000, false);
        assert!(
            front_state(&entered).continuation,
            "o front precisa saber que a linha não vale como comando novo"
        );

        // Enquanto continua, não repete o evento a cada chunk.
        let more = machine.on_chunk(b"echo $i\r\nfor> ", 2_100, false);
        assert!(!more.iter().any(|a| matches!(a, Action::CommandState(_))));

        // `done` fecha o comando: o shell roda preexec e emite `633;E`+`133;C`.
        let closed = machine.on_chunk(
            format!(
                "done\r\n\x1b]633;E;{}\x07\x1b]133;C\x07",
                b64("for i in 1 2 3; do echo $i; done")
            )
            .as_bytes(),
            2_200,
            false,
        );
        // O mesmo payload diz as duas coisas: sem isso, um `Continuation(false)`
        // atrás do `Running` apagava o comando que acabou de começar.
        let state = front_state(&closed);
        assert!(!state.continuation);
        assert!(state.running);
        assert_eq!(
            state.command.as_deref(),
            Some("for i in 1 2 3; do echo $i; done")
        );
    }

    /// O estado que o front guarda depois de aplicar o lote inteiro.
    ///
    /// Não é conveniência de teste: o front faz
    /// `setSessionCommands(prev => ({ ...prev, [id]: payload }))` — cada
    /// payload SUBSTITUI o objeto do id, e o último a escrever ganha. Asserção
    /// ação a ação esconde justamente o bug em que uma ação parcial apaga o
    /// estado que a anterior tinha acabado de montar.
    fn front_state(actions: &[Action]) -> SessionCommandPayload {
        let mut state = SessionCommandPayload::default();
        for action in actions {
            if let Action::CommandState(next) = action {
                state = next.clone();
            }
        }
        state
    }

    /// Estado ocioso: é o que o front recebe quando não há comando.
    fn idle() -> Action {
        Action::CommandState(SessionCommandPayload::default())
    }

    #[test]
    fn the_running_command_survives_the_echo_and_the_marker_in_different_chunks() {
        // Pega todo comando cujo eco e `633;E`/`133;C` caiam em leituras
        // diferentes do PTY — e SEMPRE os multi-linha. Com o estado apagado a
        // faixa de bloco ao vivo não abria, o header nascia sem comando, e o
        // pior: o front devolvia o teclado à linha do TYBA COM comando
        // rodando, então o que o usuário digitasse ia para o stdin dele.
        let mut machine = at_the_prompt();
        machine.on_chunk(b"ls -la\r\n", 2_000, false);
        let actions = machine.on_chunk(
            format!("\x1b]633;E;{}\x07\x1b]133;C\x07total 0\r\n", b64("ls -la")).as_bytes(),
            2_010,
            false,
        );
        let state = front_state(&actions);
        assert!(
            state.running,
            "o estado final apagou o comando: {actions:?}"
        );
        assert_eq!(state.command.as_deref(), Some("ls -la"));
        assert!(!state.continuation);
    }

    #[test]
    fn a_line_that_was_just_submitted_is_not_a_continuation_yet() {
        // O intervalo entre o Enter e o `133;C` do shell. Anunciar continuação
        // aqui fazia a linha do TYBA dizer "o shell espera o resto do comando"
        // para um `ls -la`, toda vez.
        let mut machine = at_the_prompt();
        let actions = machine.on_chunk(b"ls -la\r\n", 2_000, false);
        assert!(
            !front_state(&actions).continuation,
            "continuação anunciada entre o Enter e o preexec: {actions:?}"
        );
    }

    #[test]
    fn an_incomplete_command_announces_the_continuation_and_keeps_the_screen() {
        let mut machine = at_the_prompt();
        let chunk = b"for i in 1 2 3; do\r\nfor> ";
        let actions = machine.on_chunk(chunk, 2_000, false);
        assert!(
            front_state(&actions).continuation,
            "o `PS2` é continuação: {actions:?}"
        );
        assert_eq!(
            live_screen(chunk, &actions),
            "for> ",
            "o `for> ` do PS2 não pode sumir da tela"
        );
    }

    #[test]
    fn the_continuation_waits_for_the_shell_to_paint_the_ps2() {
        // O mesmo `for … do`, com a quebra de linha e o `PS2` em leituras
        // diferentes do PTY. Quem anuncia é o glifo, não o Enter.
        let mut machine = at_the_prompt();
        let submitted = machine.on_chunk(b"for i in 1 2 3; do\r\n", 2_000, false);
        assert!(
            !front_state(&submitted).continuation,
            "só a quebra de linha não distingue `for` de `ls`: {submitted:?}"
        );
        let ps2 = machine.on_chunk(b"for> ", 2_010, false);
        assert!(front_state(&ps2).continuation, "o `PS2` chegou: {ps2:?}");
    }

    #[test]
    fn control_bytes_after_the_echo_are_not_a_continuation_prompt() {
        // O `zle` desliga o bracketed paste ao fechar a linha, ANTES do
        // preexec. Cair num chunk sozinho não pode virar `PS2`.
        let mut machine = at_the_prompt();
        let closing = machine.on_chunk(b"ls -la\r\n\x1b[?2004l", 2_000, false);
        assert!(
            !front_state(&closing).continuation,
            "controle não é prompt: {closing:?}"
        );
        let running = machine.on_chunk(
            format!("\x1b]633;E;{}\x07\x1b]133;C\x07", b64("ls -la")).as_bytes(),
            2_010,
            false,
        );
        assert!(front_state(&running).running);
    }

    #[test]
    fn the_continuation_is_not_repeated_while_the_user_types_the_body() {
        // O estado é pegajoso: enquanto nenhum marcador desfaz a submissão, o
        // eco de cada linha nova não vira evento.
        let mut machine = at_the_prompt();
        machine.on_chunk(b"for i in 1 2 3; do\r\nfor> ", 2_000, false);
        for chunk in [b"echo".as_slice(), b" $i", b"\r\n", b"for> "] {
            let actions = machine.on_chunk(chunk, 2_100, false);
            assert!(
                !actions.iter().any(|a| matches!(a, Action::CommandState(_))),
                "evento repetido em {chunk:?}: {actions:?}"
            );
        }
    }

    #[test]
    fn a_glyph_is_what_the_shell_draws_not_what_it_commands() {
        assert!(paints_glyph(b"for> "));
        assert!(
            paints_glyph(b"\x1b[32mfor> \x1b[0m"),
            "cor não esconde texto"
        );
        assert!(!paints_glyph(b""));
        assert!(!paints_glyph(b"\x1b[?2004l"), "bracketed paste é controle");
        assert!(!paints_glyph(b"\x1b]133;C\x07"), "OSC com BEL é controle");
        assert!(
            !paints_glyph(b"\x1b]0;titulo\x1b\\"),
            "OSC com ST é controle"
        );
        assert!(!paints_glyph(b"\r\n\x08\x7f"), "cursor não é glifo");
        assert!(paints_glyph("café".as_bytes()), "acento é glifo");
    }

    #[test]
    fn a_complete_command_never_announces_continuation() {
        let mut machine = at_the_prompt();
        let actions = machine.on_chunk(&one_shot_chunk("echo hi", "hi\r\n"), 2_000, false);
        assert!(
            !front_state(&actions).continuation,
            "piscada de continuação em comando normal: {actions:?}"
        );
    }

    #[test]
    fn outside_prompt_mode_nothing_is_swallowed() {
        // Sem modo prompt quem digitou foi o terminal, e o eco é a digitação.
        let mut machine = CaptureMachine::new("s1".into());
        machine.on_chunk(b"\x1b]133;B\x07", 1_000, false);
        let chunk = b"for i in 1 2 3; do\r\nfor> ";
        assert_eq!(
            live_screen(chunk, &machine.on_chunk(chunk, 2_000, false)),
            "for i in 1 2 3; do\r\nfor> "
        );
    }

    #[test]
    fn a_command_interrupted_by_a_new_prompt_is_kept_without_exit_code() {
        // Ctrl+C entre `133;C` e `133;D`.
        let mut machine = in_prompt_mode();
        machine.on_chunk(
            format!("\x1b]633;E;{}\x07\x1b]133;C\x07", b64("sleep 500")).as_bytes(),
            2_000,
            false,
        );
        let block = finalized(machine.on_chunk(b"^C\r\n\x1b]133;A\x07", 2_500, false))
            .expect("bloco precisa existir");
        assert_eq!(block.command, "sleep 500");
        assert_eq!(block.exit_code, None);
    }

    #[test]
    fn alt_screen_keeps_the_block_and_drops_the_body() {
        let mut machine = in_prompt_mode();
        machine.on_chunk(
            format!("\x1b]633;E;{}\x07\x1b]133;C\x07", b64("vim")).as_bytes(),
            2_000,
            false,
        );
        machine.on_chunk(b"desenho de tela cheia", 2_100, true);
        let block =
            finalized(machine.on_chunk(b"\x1b]133;D;0\x07", 2_200, false)).expect("bloco existe");
        assert!(block.alt_screen);
        assert!(block.bytes.is_empty(), "recorte de tela cheia vira lixo");
    }

    #[test]
    fn clear_asks_for_the_list_to_be_wiped_and_still_leaves_its_own_block() {
        let mut machine = in_prompt_mode();
        let actions = machine.on_chunk(&one_shot_chunk("clear", ""), 2_000, false);
        assert!(actions.contains(&Action::Wipe));
        assert!(finalized(actions).is_some(), "quem mandou sumir fica");
    }

    #[test]
    fn the_checkpoint_only_exists_while_a_block_is_in_flight() {
        let mut machine = in_prompt_mode();
        assert!(machine.checkpoint(2_000).is_none());
        machine.on_chunk(
            format!("\x1b]633;E;{}\x07\x1b]133;C\x07parcial", b64("build")).as_bytes(),
            2_000,
            false,
        );
        let snapshot = machine.checkpoint(2_100).expect("comando em voo");
        assert_eq!(snapshot.command, "build");
        assert!(snapshot.bytes.ends_with(b"parcial"));
        // Não consome: o comando segue.
        let block = finalized(machine.on_chunk(b" e o resto\x1b]133;D;0\x07", 2_200, false))
            .expect("bloco");
        assert_eq!(text_of(&block), ["parcial e o resto"]);
    }

    #[test]
    fn the_command_line_marks_the_agent_and_the_cwd_is_reported_once() {
        let mut machine = in_prompt_mode();
        let actions = machine.on_chunk(
            format!(
                "\x1b]7;file://host/tmp\x07\x1b]633;E;{}\x07\x1b]133;C\x07",
                b64("claude --continue")
            )
            .as_bytes(),
            2_000,
            false,
        );
        assert!(actions
            .iter()
            .any(|a| matches!(a, Action::Cwd(payload) if payload.cwd == "/tmp")));
        assert!(actions.iter().any(|action| matches!(
            action,
            Action::CommandState(state)
                if state.agent_match && state.command.as_deref() == Some("claude --continue")
        )));
        // Mesmo diretório de novo não vira evento.
        let again = machine.on_chunk(b"\x1b]7;file://host/tmp\x07", 2_100, false);
        assert!(!again.iter().any(|a| matches!(a, Action::Cwd(_))));
    }

    #[test]
    fn ignorespace_keeps_the_command_out_of_history_but_the_block_still_exists() {
        let mut machine = in_prompt_mode();
        let actions = machine.on_chunk(&one_shot_chunk(" export TOKEN=x", ""), 2_000, false);
        assert!(
            !actions.iter().any(|a| matches!(a, Action::Record(_))),
            "linha com espaço à esquerda não vai para o disco"
        );
        assert!(finalized(actions).is_some());
    }

    #[test]
    fn finds_the_marker_and_keeps_what_comes_after_it() {
        // O que vem grudado no `133;C` é a saída do comando: cortar o chunk
        // inteiro levaria o começo dela junto.
        let chunk = b"ls -la\r\n\x1b]133;C\x07total 0\r\n";
        let at = command_start_at(chunk).expect("marcador inteiro no chunk");
        assert_eq!(&chunk[at..], b"\x1b]133;C\x07total 0\r\n");
    }

    #[test]
    fn survives_the_marker_with_parameters() {
        let chunk = b"eco\x1b]133;C;cmdline=x\x1b\\saida";
        assert!(command_start_at(chunk).is_some());
    }

    #[test]
    fn says_nothing_when_the_marker_is_split_between_chunks() {
        // Quem chama tem a rede do evento do parser: sem ela, a tela ao vivo
        // ficaria muda o comando inteiro.
        assert_eq!(command_start_at(b"eco\r\n\x1b]133"), None);
        assert_eq!(command_end_at(b"saida\r\n\x1b]133;"), None);
    }

    #[test]
    fn says_nothing_on_a_chunk_of_plain_output() {
        assert_eq!(command_start_at(b"total 0\r\ndrwxr-xr-x\r\n"), None);
        assert_eq!(command_end_at(b"total 0\r\ndrwxr-xr-x\r\n"), None);
    }

    #[test]
    fn the_end_marker_is_found_with_and_without_exit_code() {
        assert_eq!(command_end_at(b"saida\x1b]133;D\x07"), Some(5));
        assert_eq!(command_end_at(b"saida\x1b]133;D;130\x07"), Some(5));
    }
}
