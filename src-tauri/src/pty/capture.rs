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

use super::SessionCwdPayload;

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
    Running {
        command: Option<String>,
        agent_match: bool,
    },
    Idle,
    Record(CommandRecord),
    Wipe,
    Finalize(FinishedBlock),
    PromptMode(bool),
    Cwd(SessionCwdPayload),
}

pub(super) struct CaptureMachine {
    osc: OscParser,
    tracker: Tracker,
    capture: Capture,
    capturing: bool,
    swallow_echo: bool,
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

        // A tela ao vivo é decidida com o estado de ANTES dos eventos: o que
        // está chegando é o eco do comando que acabou de ser submetido.
        if self.swallow_echo {
            // O eco cai entre `133;B` e `133;C`, e o cartão já mostra o
            // comando — na tela ao vivo ele aparecia duas vezes.
            //
            // O corte é no MARCADOR, não no chunk: a saída costuma vir grudada
            // no `133;C`, e jogar o chunk fora levaria o começo dela junto.
            //
            // E a limpeza vai junto com os bytes, no mesmo canal: pelo evento
            // de comando ela chegava ANTES do eco, e limpava uma tela onde ele
            // ainda não estava.
            if let Some(at) = command_start_at(chunk) {
                actions.push(Action::LiveRestart(at..chunk.len()));
                self.swallow_echo = false;
            }
        } else {
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
                    if self.capturing {
                        self.push_capture(&chunk[from..end], alt_screen);
                        // O estado de tela do core é o que reata a sessão ao
                        // trocar de aba. Sem limpá-lo junto com o xterm, o
                        // histórico inteiro voltaria dentro do cartão ativo no
                        // reattach.
                        actions.push(Action::ClearCoreScreen);
                    }
                    actions.push(Action::Running {
                        command: self.tracker.command().map(str::to_string),
                        agent_match: self.agent_match,
                    });
                }
                // `133;D` traz exit code; prompt novo sem ele é comando que não
                // terminou (Ctrl+C no meio, shell reiniciado) — grava sem
                // código em vez de sumir com ele.
                ShellEvent::CommandEnd(code) => actions.extend(self.finish(Some(code), now_ms)),
                ShellEvent::PromptStart => actions.extend(self.finish(None, now_ms)),
                ShellEvent::InputStart => {
                    // Daqui até o `133;C` só vem o eco do que foi submetido.
                    // Fora do modo prompt quem digitou foi o terminal, e aí o
                    // eco é a própria digitação.
                    self.swallow_echo = self.prompt_mode == Some(true);
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
        actions
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
        actions.push(Action::Idle);
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
        assert!(actions.iter().any(|a| matches!(a, Action::Running { .. })));
        assert!(actions.iter().any(|a| matches!(a, Action::Idle)));
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

        let idle_at = actions.iter().position(|a| matches!(a, Action::Idle));
        assert!(idle_at.is_some(), "front precisa ouvir running: false");
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
        assert_eq!(actions[0], Action::Idle);
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
            Action::Running { command, agent_match: true } if command.as_deref() == Some("claude --continue")
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
