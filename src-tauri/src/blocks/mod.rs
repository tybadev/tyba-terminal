//! Blocos de comando.
//!
//! Um bloco é comando + saída + exit code + duração. A saída é o que veio entre
//! `133;C` e `133;D` — fronteira limpa porque, no modo prompt do TYBA, o `PS1`
//! não é desenhado (ver `features/terminal-blocks` no cofre).
//!
//! O `vt100` processa os bytes **uma única vez**, no finalize. O que sobrevive
//! não são os bytes crus: são **linhas lógicas** (soft-wrap desfeito) com runs
//! de estilo. Reflow no resize vira re-quebra na renderização, sem reparse —
//! ADR de 2026-07-10.

use serde::{Deserialize, Serialize};

/// Teto por bloco, igual ao scrollback do xterm vivo: nada "se perde ao virar
/// bloco". Eviction do começo, com o contador exposto no header.
pub const MAX_LINES: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Color {
    /// Cor padrão do terminal — quem resolve é o tema, na renderização.
    #[default]
    Default,
    /// Índice da paleta. Preservado como índice, e não convertido para RGB,
    /// para o bloco acompanhar troca de tema.
    Idx(u8),
    Rgb(u8, u8, u8),
}

impl From<vt100::Color> for Color {
    fn from(color: vt100::Color) -> Self {
        match color {
            vt100::Color::Default => Color::Default,
            vt100::Color::Idx(i) => Color::Idx(i),
            vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Style {
    fg: Color,
    bg: Color,
    bold: bool,
    italic: bool,
    underline: bool,
}

impl Style {
    fn is_plain(&self) -> bool {
        *self == Style::default()
    }
}

/// Trecho de uma linha com o mesmo estilo. `start`/`end` são offsets **de
/// bytes** no texto da linha.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleRun {
    pub start: usize,
    pub end: usize,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LogicalLine {
    pub text: String,
    /// Só os trechos que fogem do padrão. Saída sem cor nenhuma custa zero.
    pub runs: Vec<StyleRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub id: u64,
    pub session_id: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub lines: Vec<LogicalLine>,
    /// Linhas descartadas pelo teto, para o header dizer que faltou coisa em
    /// vez de mentir que aquilo é a saída inteira.
    pub truncated: usize,
}

/// Teto de bytes acumulados por bloco antes do finalize.
///
/// A saída fica em memória enquanto o comando roda; sem teto, um `yes` ou um
/// log em loop comeriam a máquina. Passou daqui, o começo é descartado e o
/// bloco diz que foi truncado em vez de mentir.
pub const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

/// A saída de um comando enquanto ele roda.
///
/// Acumula na thread emitter, que é caminho quente: aqui só se copia bytes. O
/// parse, a redação e a gravação acontecem depois, fora dela.
#[derive(Debug, Default)]
pub struct Capture {
    bytes: Vec<u8>,
    dropped: bool,
    /// Alt-screen no meio do comando (vim, htop): por decisão da spec isso não
    /// vira bloco — a tela é do programa, e recortá-la produziria lixo.
    alt_screen: bool,
}

impl Capture {
    pub fn push(&mut self, chunk: &[u8]) {
        if self.bytes.len() + chunk.len() > MAX_CAPTURE_BYTES {
            let overflow = self.bytes.len() + chunk.len() - MAX_CAPTURE_BYTES;
            let cut = overflow.min(self.bytes.len());
            self.bytes.drain(0..cut);
            self.dropped = true;
        }
        self.bytes.extend_from_slice(chunk);
    }

    pub fn saw_alt_screen(&mut self) {
        self.alt_screen = true;
    }

    pub fn is_alt_screen(&self) -> bool {
        self.alt_screen
    }

    pub fn dropped(&self) -> bool {
        self.dropped
    }

    pub fn take(&mut self) -> Vec<u8> {
        self.dropped = false;
        self.alt_screen = false;
        std::mem::take(&mut self.bytes)
    }
}

struct RawRow {
    text: String,
    runs: Vec<StyleRun>,
    wrapped: bool,
}

fn read_row(screen: &vt100::Screen, row: u16, cols: u16) -> RawRow {
    let mut text = String::new();
    let mut runs: Vec<StyleRun> = Vec::new();
    let mut open: Option<(Style, usize)> = None;

    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else {
            continue;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        let style = Style {
            fg: cell.fgcolor().into(),
            bg: cell.bgcolor().into(),
            bold: cell.bold(),
            italic: cell.italic(),
            underline: cell.underline(),
        };
        let contents = cell.contents();
        // Célula vazia é espaço: a grade é retangular, o texto não.
        let piece = if contents.is_empty() {
            " ".to_string()
        } else {
            contents
        };

        match &open {
            Some((current, start)) if *current == style => {}
            Some((current, start)) => {
                if !current.is_plain() {
                    runs.push(finish_run(*current, *start, text.len()));
                }
                open = Some((style, text.len()));
            }
            None => open = Some((style, text.len())),
        }
        text.push_str(&piece);
    }

    if let Some((style, start)) = open {
        if !style.is_plain() {
            runs.push(finish_run(style, start, text.len()));
        }
    }

    let wrapped = screen.row_wrapped(row);
    if !wrapped {
        // Linha não embrulhada vem preenchida de espaço até o fim da grade;
        // guardar isso engordaria o bloco e sujaria qualquer cópia.
        let trimmed = text.trim_end().len();
        text.truncate(trimmed);
        runs.retain(|run| run.start < trimmed);
        for run in runs.iter_mut() {
            run.end = run.end.min(trimmed);
        }
    }

    RawRow {
        text,
        runs,
        wrapped,
    }
}

fn finish_run(style: Style, start: usize, end: usize) -> StyleRun {
    StyleRun {
        start,
        end,
        fg: style.fg,
        bg: style.bg,
        bold: style.bold,
        italic: style.italic,
        underline: style.underline,
    }
}

/// Junta as linhas visuais em linhas lógicas, desfazendo o soft-wrap.
///
/// É o que permite o reflow sem reparse: a fronteira guardada é a **lógica**, e
/// quem quebra por largura é o renderizador.
fn join_wrapped(rows: Vec<RawRow>) -> Vec<LogicalLine> {
    let mut lines: Vec<LogicalLine> = Vec::new();
    let mut current = LogicalLine::default();
    let mut open = false;

    for row in rows {
        let offset = current.text.len();
        current.text.push_str(&row.text);
        for mut run in row.runs {
            run.start += offset;
            run.end += offset;
            current.runs.push(run);
        }
        open = true;
        if !row.wrapped {
            lines.push(std::mem::take(&mut current));
            open = false;
        }
    }
    if open {
        lines.push(current);
    }
    lines
}

/// Teto de linhas que a saída pode ocupar, para dimensionar a grade.
///
/// Cada `\n` começa uma linha, e o resto só pode acrescentar por wrap — usar o
/// tamanho em bytes como proxy dos imprimíveis superestima (escape conta como
/// texto), o que é o lado seguro de errar.
fn rows_needed(bytes: &[u8], cols: u16) -> u16 {
    let newlines = bytes.iter().filter(|b| **b == b'\n').count();
    let wrapped = bytes.len() / cols.max(1) as usize;
    let estimate = newlines + wrapped + 2;
    estimate.clamp(1, MAX_LINES).min(u16::MAX as usize) as u16
}

/// Processa os bytes do bloco uma vez e devolve as linhas lógicas.
///
/// `cols` é a largura real do terminal na captura — é ela que decide o wrap, e
/// o wrap é a fronteira que o reflow depois desfaz.
///
/// A **altura** não é a do terminal: a grade nasce alta o bastante para a saída
/// inteira caber sem rolar.
///
/// > O `vt100 0.15.2` não permite paginar o scrollback: `Grid::visible_rows`
/// > faz `rows_len - scrollback_offset`, que estoura quando o offset passa da
/// > altura da tela. Na prática só dá para voltar UMA tela, e o resto do
/// > histórico é inalcançável pela API pública. O ADR de 2026-07-10 assumia que
/// > bastava "paginar via offset" — não basta.
///
/// Grade alta é equivalente para o que um bloco guarda: programa de tela cheia
/// usa alt-screen e, por decisão da spec, não vira bloco; e posicionamento de
/// cursor é ancorado no topo, que não se move.
pub fn extract_lines(bytes: &[u8], cols: u16, _rows: u16) -> (Vec<LogicalLine>, usize) {
    let cols = cols.max(1);
    let height = rows_needed(bytes, cols);
    let mut parser = vt100::Parser::new(height, cols, 0);
    parser.process(bytes);

    let screen = parser.screen();
    let rows_vec: Vec<RawRow> = (0..height).map(|row| read_row(screen, row, cols)).collect();
    let mut lines = join_wrapped(rows_vec);

    // O rabo em branco é o resto da tela, não saída do comando.
    while lines
        .last()
        .is_some_and(|line| line.text.trim().is_empty() && line.runs.is_empty())
    {
        lines.pop();
    }

    let truncated = lines.len().saturating_sub(MAX_LINES);
    if truncated > 0 {
        lines.drain(0..truncated);
    }
    (lines, truncated)
}

/// O que a thread emitter entrega ao finalizar um comando.
pub struct Finished {
    pub session_id: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub bytes: Vec<u8>,
    pub cols: u16,
    pub rows: u16,
    pub dropped: bool,
}

struct Finalizer {
    tx: std::sync::mpsc::SyncSender<Finished>,
}

static FINALIZER: std::sync::OnceLock<Finalizer> = std::sync::OnceLock::new();
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Parse, redação e emissão saem da thread emitter.
///
/// O finalize de um bloco de 10 mil linhas é trabalho de verdade; fazê-lo no
/// caminho quente seguraria o flush de output do terminal por todo esse tempo.
/// A emitter só entrega os bytes e segue.
pub fn install(app: tauri::AppHandle, store: std::sync::Arc<crate::session::store::Store>) {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Finished>(16);
    if FINALIZER.set(Finalizer { tx }).is_err() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("block-finalizer".into())
        .spawn(move || {
            use tauri::Emitter;
            while let Ok(finished) = rx.recv() {
                let block = build(finished);
                // EMITE ANTES DE GRAVAR. O olho do usuário não pode esperar o
                // disco: o bloco aparece, e a persistência acontece depois. O
                // id vale para a sessão viva; ao reabrir, quem manda é o rowid
                // que a leitura devolve.
                let event = format!("block://finalized/{}", block.session_id);
                let _ = app.emit(&event, block.clone());
                if let Err(err) = store.insert_block(&block) {
                    eprintln!("tyba: bloco não persistido: {err}");
                }
            }
        });
}

fn build(finished: Finished) -> Block {
    let (lines, truncated) = extract_lines(&finished.bytes, finished.cols, finished.rows);
    // Redação sobre a linha lógica inteira, não sobre chunk cru: chunk pode
    // partir um `sk-…` no meio e o padrão escapar (princípio #10).
    let lines = lines
        .into_iter()
        .map(|line| LogicalLine {
            text: crate::session::redact::redact(&line.text).into_owned(),
            runs: line.runs,
        })
        .collect();
    Block {
        id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        session_id: finished.session_id,
        command: crate::session::redact::redact(&finished.command).into_owned(),
        exit_code: finished.exit_code,
        started_at_ms: finished.started_at_ms,
        finished_at_ms: finished.finished_at_ms,
        lines,
        truncated: truncated + usize::from(finished.dropped),
    }
}

/// Nunca bloqueia: fila cheia descarta o bloco em vez de segurar o terminal.
pub fn finalize(finished: Finished) {
    if let Some(finalizer) = FINALIZER.get() {
        let _ = finalizer.tx.try_send(finished);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(bytes: &[u8], cols: u16, rows: u16) -> Vec<String> {
        extract_lines(bytes, cols, rows)
            .0
            .into_iter()
            .map(|line| line.text)
            .collect()
    }

    #[test]
    fn capture_drops_the_beginning_instead_of_eating_the_machine() {
        let mut capture = Capture::default();
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..10 {
            capture.push(&chunk);
        }
        assert!(capture.dropped(), "precisa admitir que truncou");
        assert!(capture.take().len() <= MAX_CAPTURE_BYTES);
    }

    #[test]
    fn capture_keeps_the_end_which_is_what_the_user_is_looking_at() {
        let mut capture = Capture::default();
        capture.push(&vec![b'a'; MAX_CAPTURE_BYTES]);
        capture.push(b"FIM");
        let bytes = capture.take();
        assert!(bytes.ends_with(b"FIM"));
    }

    #[test]
    fn taking_the_capture_resets_the_flags_for_the_next_command() {
        let mut capture = Capture::default();
        capture.saw_alt_screen();
        capture.push(b"x");
        let _ = capture.take();
        assert!(!capture.is_alt_screen());
        assert!(!capture.dropped());
    }

    #[test]
    fn plain_output_becomes_one_line_each() {
        assert_eq!(
            lines_of(b"um\r\ndois\r\ntres\r\n", 80, 24),
            vec!["um", "dois", "tres"]
        );
    }

    #[test]
    fn soft_wrap_is_undone_into_one_logical_line() {
        // O reflow sem reparse depende disto: a fronteira guardada é a lógica.
        let long = "a".repeat(25);
        let out = lines_of(format!("{long}\r\n").as_bytes(), 10, 6);
        assert_eq!(out, vec![long]);
    }

    #[test]
    fn trailing_blank_screen_is_not_output() {
        // A grade é retangular; a saída do comando não.
        assert_eq!(lines_of(b"ola\r\n", 20, 10), vec!["ola"]);
    }

    #[test]
    fn keeps_blank_lines_between_content() {
        assert_eq!(
            lines_of(b"um\r\n\r\ndois\r\n", 20, 10),
            vec!["um", "", "dois"]
        );
    }

    #[test]
    fn carriage_return_overwrite_keeps_only_what_is_on_screen() {
        // Barra de progresso: o que vale é o estado final da linha, não os
        // redraws — é por isso que o bloco guarda a tela parseada, não os bytes.
        assert_eq!(lines_of(b"10%\r100%\r\n", 20, 5), vec!["100%"]);
    }

    #[test]
    fn output_taller_than_the_screen_survives_via_scrollback() {
        let mut input = Vec::new();
        for i in 0..50 {
            input.extend_from_slice(format!("linha{i}\r\n").as_bytes());
        }
        let out = lines_of(&input, 20, 5);
        assert_eq!(out.len(), 50);
        assert_eq!(out.first().unwrap(), "linha0");
        assert_eq!(out.last().unwrap(), "linha49");
    }

    #[test]
    fn style_runs_mark_only_what_escapes_the_default() {
        let (lines, _) = extract_lines(b"normal \x1b[31mvermelho\x1b[0m fim\r\n", 40, 5);
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert_eq!(line.text, "normal vermelho fim");
        assert_eq!(line.runs.len(), 1, "só o trecho colorido vira run");
        let run = &line.runs[0];
        assert_eq!(&line.text[run.start..run.end], "vermelho");
        assert_eq!(run.fg, Color::Idx(1));
    }

    #[test]
    fn indexed_color_is_kept_as_index_so_the_theme_still_decides() {
        let (lines, _) = extract_lines(b"\x1b[33mamarelo\x1b[0m\r\n", 40, 5);
        assert_eq!(lines[0].runs[0].fg, Color::Idx(3));
    }

    #[test]
    fn rgb_and_attributes_survive() {
        let (lines, _) = extract_lines(b"\x1b[1;4;38;2;10;20;30mforte\x1b[0m\r\n", 40, 5);
        let run = &lines[0].runs[0];
        assert_eq!(run.fg, Color::Rgb(10, 20, 30));
        assert!(run.bold);
        assert!(run.underline);
    }

    #[test]
    fn style_offsets_survive_the_wrap_join() {
        // O run tem de continuar apontando para o texto certo depois que duas
        // linhas visuais viram uma lógica.
        let (lines, _) = extract_lines(b"aaaaa\x1b[31mbbbbb\x1b[0m\r\n", 5, 5);
        assert_eq!(lines.len(), 1);
        let line = &lines[0];
        assert_eq!(line.text, "aaaaabbbbb");
        let run = &line.runs[0];
        assert_eq!(&line.text[run.start..run.end], "bbbbb");
    }

    #[test]
    fn empty_output_produces_no_lines() {
        assert!(lines_of(b"", 80, 24).is_empty());
    }

    #[test]
    fn wide_characters_are_not_duplicated() {
        let out = lines_of("日本語\r\n".as_bytes(), 20, 5);
        assert_eq!(out, vec!["日本語"]);
    }
}
