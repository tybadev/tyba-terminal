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

/// Trecho de uma linha com o mesmo estilo.
///
/// Os offsets são em **unidades UTF-16**, e o nome do campo diz isso porque o
/// contrato não estava declarado na fronteira e ninguém percebeu: eram offsets
/// de BYTE (`text.len()` do Rust) e o webview os aplicava com
/// `String.prototype.slice`, que indexa por unidade UTF-16. Os dois só
/// coincidem em ASCII puro — `á` são 2 bytes e 1 unidade, `日` 3 e 1, `😀` 4 e
/// 2 —, então qualquer acento deslocava a cor da primeira ocorrência até o fim
/// da linha. Em saída em português isso é o caso comum, não o excepcional.
///
/// Converter aqui e não no front é de propósito: o consumidor é um webview e
/// sempre será, e o finalize já é o lugar onde o parse acontece uma vez só —
/// converter na renderização se pagaria a cada frame, do lado que já é o
/// gargalo.
///
/// A CHAVE de serialização continua `start`/`end` por compatibilidade: bloco já
/// gravado no SQLite tem os offsets antigos sob esses nomes, e renomear faria o
/// `serde` falhar na leitura — o histórico inteiro sumiria da lista, porque
/// `list_blocks` descarta silenciosamente a linha que não desserializa.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleRun {
    #[serde(rename = "start")]
    pub start_utf16: usize,
    #[serde(rename = "end")]
    pub end_utf16: usize,
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
    /// Onde o comando rodou. Um `ls` diz pouco sem isto, e a sessão pode ter
    /// mudado de diretório depois — o bloco guarda o de quando aconteceu.
    pub cwd: Option<String>,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub lines: Vec<LogicalLine>,
    /// Linhas descartadas, para o header dizer que faltou coisa em vez de
    /// mentir que aquilo é a saída inteira. Soma as duas perdas possíveis: o
    /// teto de captura, que corta enquanto o comando roda, e o teto do bloco,
    /// que corta na hora de virar linha.
    ///
    /// Saída sem `\n` nenhum (barra de progresso, blob binário) perde conteúdo
    /// sem perder linha, e aí este número é zero com razão — a unidade é linha,
    /// e não havia linha.
    pub truncated: usize,
    /// O comando pintou a tela alternada (`vim`, `bat`, `htop`).
    ///
    /// A saída não é guardada — recortar a tela de um programa produz lixo —,
    /// mas o BLOCO existe. Sumir com o comando inteiro apagaria do registro
    /// algo que a pessoa rodou, que é pior do que um bloco sem corpo.
    pub alt_screen: bool,
}

/// Teto de bytes acumulados por bloco antes do finalize.
///
/// A saída fica em memória enquanto o comando roda; sem teto, um `yes` ou um
/// log em loop comeriam a máquina. Passou daqui, o começo é descartado e o
/// bloco diz que foi truncado em vez de mentir.
pub const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

/// Quantas linhas lógicas há num trecho de saída.
///
/// Linha lógica termina em `\n`: o wrap não cria linha, só quebra a mesma linha
/// na tela. É o que permite contar o que foi descartado sem parsear — o que se
/// perde são linhas, e linha é `\n`.
fn logical_lines(bytes: &[u8]) -> usize {
    bytes.iter().filter(|b| **b == b'\n').count()
}

/// A saída de um comando enquanto ele roda.
///
/// Acumula na thread emitter, que é caminho quente: aqui só se copia bytes. O
/// parse, a redação e a gravação acontecem depois, fora dela.
#[derive(Debug, Default)]
pub struct Capture {
    bytes: Vec<u8>,
    dropped_lines: usize,
    /// Alt-screen no meio do comando (vim, htop): por decisão da spec isso não
    /// vira bloco — a tela é do programa, e recortá-la produziria lixo.
    alt_screen: bool,
}

impl Capture {
    pub fn push(&mut self, chunk: &[u8]) {
        if self.bytes.len() + chunk.len() > MAX_CAPTURE_BYTES {
            let overflow = self.bytes.len() + chunk.len() - MAX_CAPTURE_BYTES;
            let cut = overflow.min(self.bytes.len());
            // Contadas ANTES do dreno: depois estes bytes não existem mais.
            // Antes daqui saía um `bool`, e quem o consumia somava 1 ao total
            // de linhas cortadas — oito megabytes de saída perdidos viravam
            // "1 linha" no rodapé do bloco.
            self.dropped_lines += logical_lines(&self.bytes[..cut]);
            self.bytes.drain(0..cut);
        }
        self.bytes.extend_from_slice(chunk);
    }

    pub fn saw_alt_screen(&mut self) {
        self.alt_screen = true;
    }

    pub fn is_alt_screen(&self) -> bool {
        self.alt_screen
    }

    /// Linhas que o teto de captura comeu enquanto o comando ainda rodava.
    pub fn dropped(&self) -> usize {
        self.dropped_lines
    }

    /// Cópia do que já saiu, para o checkpoint. Não consome: o comando segue.
    pub fn snapshot(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    pub fn take(&mut self) -> Vec<u8> {
        self.dropped_lines = 0;
        self.alt_screen = false;
        std::mem::take(&mut self.bytes)
    }
}

/// Trecho com o mesmo estilo, ainda em offsets de BYTE — a unidade em que o
/// `vt100` e a `String` do Rust trabalham, e a única em que a aritmética de
/// junção e de `trim_end` abaixo está correta. Vira `StyleRun` só no fim, com o
/// texto da linha lógica já fechado.
#[derive(Debug, Clone, Copy)]
struct ByteRun {
    start: usize,
    end: usize,
    style: Style,
}

struct RawRow {
    text: String,
    runs: Vec<ByteRun>,
    wrapped: bool,
}

/// Linha lógica antes da conversão de unidade.
#[derive(Default)]
struct RawLine {
    text: String,
    runs: Vec<ByteRun>,
}

fn read_row(screen: &vt100::Screen, row: u16, cols: u16) -> RawRow {
    let mut text = String::new();
    let mut runs: Vec<ByteRun> = Vec::new();
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

fn finish_run(style: Style, start: usize, end: usize) -> ByteRun {
    ByteRun { start, end, style }
}

/// Fecha a linha convertendo os offsets de byte para unidades UTF-16.
///
/// É a única passagem da fronteira: daqui para a frente o número significa
/// unidade UTF-16, que é o que o `slice` do JS conta.
fn into_logical(raw: RawLine) -> LogicalLine {
    let runs = if raw.text.is_ascii() {
        // Em ASCII os dois números são o mesmo, e é o caso comum: não paga
        // varredura nenhuma.
        raw.runs
            .into_iter()
            .map(|run| style_run(run, run.start, run.end))
            .collect()
    } else {
        to_utf16(&raw.text, raw.runs)
    };
    LogicalLine {
        text: raw.text,
        runs,
    }
}

fn style_run(run: ByteRun, start_utf16: usize, end_utf16: usize) -> StyleRun {
    StyleRun {
        start_utf16,
        end_utf16,
        fg: run.style.fg,
        bg: run.style.bg,
        bold: run.style.bold,
        italic: run.style.italic,
        underline: run.style.underline,
    }
}

/// Uma passada pelo texto resolve todas as marcas dos runs.
///
/// Converter run a run com `text[..offset].encode_utf16().count()` seria
/// quadrático numa linha muito colorida — e linha muito colorida é justamente
/// a que tem run.
fn to_utf16(text: &str, runs: Vec<ByteRun>) -> Vec<StyleRun> {
    let mut marks: Vec<usize> = runs.iter().flat_map(|run| [run.start, run.end]).collect();
    marks.sort_unstable();
    marks.dedup();

    let mut units: Vec<usize> = Vec::with_capacity(marks.len());
    let mut mark = 0;
    let mut seen = 0;
    for (at, ch) in text.char_indices() {
        while mark < marks.len() && marks[mark] <= at {
            units.push(seen);
            mark += 1;
        }
        seen += ch.len_utf16();
    }
    // Marcas no fim do texto — `run.end` da última linha cai sempre aqui.
    while mark < marks.len() {
        units.push(seen);
        mark += 1;
    }

    let at_mark = |byte: usize| match marks.binary_search(&byte) {
        Ok(found) => units[found],
        Err(_) => seen,
    };
    runs.iter()
        .map(|run| style_run(*run, at_mark(run.start), at_mark(run.end)))
        .collect()
}

/// Junta as linhas visuais em linhas lógicas, desfazendo o soft-wrap.
///
/// É o que permite o reflow sem reparse: a fronteira guardada é a **lógica**, e
/// quem quebra por largura é o renderizador.
fn join_wrapped(rows: Vec<RawRow>) -> Vec<RawLine> {
    let mut lines: Vec<RawLine> = Vec::new();
    let mut current = RawLine::default();
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

/// Folga entre o que a saída ocupa e a altura com que a grade nasce.
///
/// A saída quase sempre termina em `\n`, e o cursor precisa de uma linha para
/// pousar depois dela: sem a folga esse `\n` final rola a grade e come a
/// PRIMEIRA linha — perda silenciosa, que é o que este módulo existe para não
/// ter. O rabo em branco que a folga cria é aparado no `extract_lines`.
const GRID_SLACK: usize = 2;

/// Onde o cursor está e até onde a saída já pintou, numa grade de `width`
/// colunas e altura ilimitada.
///
/// `painted` só anda quando um glifo cai numa linha. Mover o cursor não pinta:
/// contar o `\n` final como linha usada dobraria a conta de toda saída normal.
struct Pen {
    width: usize,
    row: usize,
    col: usize,
    painted: usize,
}

impl Pen {
    fn new(width: usize) -> Self {
        Self {
            width,
            row: 0,
            col: 0,
            painted: 0,
        }
    }

    /// Escreve um glifo de `cols` colunas, embrulhando se não couber.
    fn put(&mut self, cols: usize) {
        if self.col + cols > self.width {
            self.row = self.row.saturating_add(1);
            self.col = 0;
        }
        self.col += cols;
        self.painted = self.painted.max(self.row);
    }

    fn down(&mut self, rows: usize) {
        self.row = self.row.saturating_add(rows);
    }

    fn up(&mut self, rows: usize) {
        self.row = self.row.saturating_sub(rows);
    }

    fn at_col(&mut self, col: usize) {
        self.col = col.min(self.width);
    }

    fn rows(&self) -> usize {
        self.painted + 1
    }
}

/// Colunas que um caractere ocupa na grade.
///
/// Aproximação deliberada: as faixas East Asian Wide/Fullwidth mais emoji
/// valem 2, o resto vale 1. A tabela exata é a do `unicode-width`, que já está
/// na árvore por baixo do `vt100` — usá-la direto exigiria uma dependência
/// nova no `Cargo.toml`, e a diferença que sobra é a de baixo.
///
/// O que fica impreciso é a marca combinante, que tem largura ZERO e aqui vale
/// 1: `é` decomposto conta 2 colunas em vez de 1. O erro é para CIMA, ou seja
/// para o lado de descartar demais, e é limitado ao número de acentos
/// decompostos da linha. As faixas foram escolhidas para NÃO pegar traço de
/// tabela nem travessão — os caracteres de 3 bytes comuns em saída de CLI —,
/// porque um `─` contado como 2 colunas cortaria pela metade toda saída em
/// tabela.
fn char_cols(ch: u32) -> usize {
    const WIDE: &[(u32, u32)] = &[
        (0x1100, 0x115f),
        (0x2e80, 0x303e),
        (0x3041, 0x33ff),
        (0x3400, 0x4dbf),
        (0x4e00, 0x9fff),
        (0xa000, 0xa4cf),
        (0xac00, 0xd7a3),
        (0xf900, 0xfaff),
        (0xfe10, 0xfe19),
        (0xfe30, 0xfe6f),
        (0xff00, 0xff60),
        (0xffe0, 0xffe6),
        (0x1f300, 0x1f64f),
        (0x1f900, 0x1f9ff),
        (0x20000, 0x3fffd),
    ];
    if ch < 0x1100 {
        return 1;
    }
    if WIDE.iter().any(|(low, high)| ch >= *low && ch <= *high) {
        2
    } else {
        1
    }
}

/// Decodifica um caractere UTF-8 e devolve quantos bytes ele comeu.
///
/// Byte inválido ou sequência truncada anda 1 byte e vale um glifo: o `vt100`
/// também põe alguma coisa na tela, e parar aqui deixaria o resto da saída sem
/// medir.
fn decode(bytes: &[u8], at: usize) -> (u32, usize) {
    let lead = bytes[at];
    let (mut ch, len) = match lead {
        0x00..=0x7f => return (lead as u32, 1),
        0xc0..=0xdf => ((lead & 0x1f) as u32, 2),
        0xe0..=0xef => ((lead & 0x0f) as u32, 3),
        0xf0..=0xf7 => ((lead & 0x07) as u32, 4),
        _ => return (0xfffd, 1),
    };
    for step in 1..len {
        match bytes.get(at + step) {
            Some(byte) if (0x80..0xc0).contains(byte) => {
                ch = (ch << 6) | (byte & 0x3f) as u32;
            }
            _ => return (0xfffd, step),
        }
    }
    (ch, len)
}

/// Quantas linhas de grade a saída ocupa, sem teto.
///
/// ARMADILHA — esta conta tem DOIS consumidores cujos lados seguros são
/// OPOSTOS, e foi por não notar isso que a saída sumiu:
///
/// - `rows_needed` dimensiona a grade. Errar para MAIS só gasta memória; para
///   menos, o conteúdo rola para fora e some sem entrar em contagem nenhuma.
/// - `head_cut` decide o que DESCARTAR. Aqui é o contrário: errar para mais
///   joga fora saída que caberia.
///
/// A conta antiga era `bytes.len() / cols`, com o escape contando como texto.
/// Enquanto ela só dimensionava a grade, o exagero era o lado seguro — grade
/// maior não perde nada. Quando virou orçamento de descarte, o MESMO exagero
/// passou a apagar saída: `bun install` colorido gasta ~282 bytes para pintar
/// 60 colunas, e a barra de progresso do `docker pull` é um megabyte numa
/// linha só, porque `\r` redesenha sempre a mesma.
///
/// Por isso aqui se conta o que PINTA: escape custa zero coluna, `\r` volta à
/// coluna zero, e o wrap sai da largura, não do tamanho do buffer. É a única
/// métrica que serve aos dois consumidores ao mesmo tempo.
///
/// O que CONTINUA impreciso, e para que lado:
///
/// - Apagar não desconta. `CSI 2J` e `CSI 2K` não devolvem a linha ao contador
///   (`painted` só sobe), então quem limpa e repinta é contado duas vezes. Erra
///   para descartar demais. Programa que faz isso o tempo todo usa alt-screen,
///   e alt-screen não vira bloco.
/// - `CSI r;cH` com uma linha absurda (`CSI 999999H`) leva o cursor para lá de
///   verdade, enquanto o `vt100` o prenderia na altura da grade. Erra para
///   descartar demais, e só num programa que emita coordenada fora da tela.
/// - Marca combinante vale 1 em vez de 0 — ver `char_cols`.
fn grid_rows(bytes: &[u8], cols: u16) -> usize {
    /// Onde o scanner está dentro de uma sequência de escape. Mesma máquina do
    /// `paints_glyph` (`pty/capture.rs`), reescrita aqui porque este módulo
    /// precisa de mais do que "pintou ou não": precisa de quanto e onde.
    #[derive(Clone, Copy, PartialEq)]
    enum Scan {
        Ground,
        Esc,
        Csi,
        /// Corpo de OSC/DCS/APC/PM/SOS — termina em `BEL` ou `ST`.
        StringBody,
    }

    const TAB: usize = 8;

    let width = cols.max(1) as usize;
    let mut pen = Pen::new(width);
    let mut state = Scan::Ground;
    // Só os dois primeiros parâmetros interessam: linha e coluna do `CUP`.
    let mut params = [0usize; 2];
    let mut param_at = 0usize;
    let mut at = 0usize;

    while at < bytes.len() {
        let byte = bytes[at];
        let mut step = 1usize;

        // `ESC` aborta o que estava em curso, em qualquer estado — é também
        // como o `ST` (`ESC \`) fecha o corpo de string.
        if byte == 0x1b {
            state = Scan::Esc;
            at += 1;
            continue;
        }

        match state {
            Scan::Ground => match byte {
                b'\r' => pen.at_col(0),
                b'\n' | 0x0b | 0x0c => pen.down(1),
                0x08 => pen.at_col(pen.col.saturating_sub(1)),
                b'\t' => pen.at_col(((pen.col / TAB) + 1) * TAB),
                0x00..=0x1f | 0x7f => {}
                0x20..=0x7e => pen.put(1),
                _ => {
                    let (ch, len) = decode(bytes, at);
                    step = len;
                    pen.put(char_cols(ch));
                }
            },
            Scan::Esc => {
                state = Scan::Ground;
                match byte {
                    b'[' => {
                        state = Scan::Csi;
                        params = [0, 0];
                        param_at = 0;
                    }
                    b']' | b'P' | b'X' | b'^' | b'_' => state = Scan::StringBody,
                    // IND desce, NEL desce e volta à margem, RI sobe.
                    b'D' => pen.down(1),
                    b'E' => {
                        pen.down(1);
                        pen.at_col(0);
                    }
                    b'M' => pen.up(1),
                    _ => {}
                }
            }
            Scan::Csi => match byte {
                b'0'..=b'9' => {
                    let slot = &mut params[param_at];
                    *slot = slot
                        .saturating_mul(10)
                        .saturating_add((byte - b'0') as usize);
                }
                b';' => param_at = (param_at + 1).min(params.len() - 1),
                0x40..=0x7e => {
                    // Movimento de cursor é o que separa um `docker pull` de
                    // 8 camadas — que sobe e redesenha as MESMAS 8 linhas —
                    // de uma saída de 8 mil linhas. Ignorá-lo contaria cada
                    // redesenho como linha nova.
                    let first = params[0].max(1);
                    match byte {
                        b'A' => pen.up(first),
                        b'B' | b'e' => pen.down(first),
                        b'E' => {
                            pen.down(first);
                            pen.at_col(0);
                        }
                        b'F' => {
                            pen.up(first);
                            pen.at_col(0);
                        }
                        // O posicionamento absoluto é ancorado no TOPO da
                        // grade, e não no topo de uma tela de 24 linhas: é a
                        // mesma divergência que o `extract_lines` documenta.
                        b'H' | b'f' => {
                            pen.row = first - 1;
                            pen.at_col(params[1].max(1) - 1);
                        }
                        b'd' => pen.row = first - 1,
                        b'G' | b'`' => pen.at_col(first - 1),
                        b'C' | b'a' => pen.at_col(pen.col.saturating_add(first)),
                        b'D' => pen.at_col(pen.col.saturating_sub(first)),
                        _ => {}
                    }
                    state = Scan::Ground;
                }
                _ => {}
            },
            Scan::StringBody => {
                if byte == 0x07 {
                    state = Scan::Ground;
                }
            }
        }

        at += step;
    }

    pen.rows()
}

/// Altura da grade para esta saída, com o teto do bloco.
fn rows_needed(bytes: &[u8], cols: u16) -> u16 {
    (grid_rows(bytes, cols) + GRID_SLACK).clamp(1, MAX_LINES) as u16
}

/// Onde começa a parte da saída que cabe no bloco.
///
/// A grade do `vt100` não pode crescer sem limite — cada célula é um
/// `[char; 6]` mais atributos, então uma linha de 80 colunas custa perto de
/// 3 KB e o teto de 10 mil já são ~30 MB — e `Size::rows` é `u16`, o que fecha
/// a porta em 65 535 de qualquer jeito. O que passa do teto SOME DENTRO da
/// grade: rola para fora, e a API de scrollback do 0.15.2 não devolve (voltar
/// mais de uma tela estoura `visible_rows`, que faz `rows_len - offset`).
///
/// Some em silêncio, e era esse o bug: `seq 1 50000` perdia 40 mil linhas e o
/// bloco reportava zero cortado, porque a conta antiga era
/// `lines.len() - MAX_LINES` sobre uma lista que a própria grade já limitava a
/// `MAX_LINES`. Nunca dava mais que zero.
///
/// Cortar aqui, ANTES do parser, é o que torna a perda contável: o que a grade
/// recebe cabe inteiro, e o que ficou de fora está medido. Decidir DEPOIS do
/// parser seria exato por construção, mas é justamente o que não dá: a grade é
/// o recurso que precisa ser dimensionado antes, e o que passar dela rola para
/// fora sem volta. O corte cai logo depois de um `\n` para não partir linha ao
/// meio; o preço é o estado de cor herdado do trecho descartado, e quem colore
/// reemite o SGR a cada linha.
///
/// O orçamento se mede em `grid_rows`, e não em bytes — ver a armadilha lá. A
/// soma dos custos linha a linha é EXATA para saída normal (cada linha começa
/// na coluna zero, então a próxima começa exatamente `custo` linhas abaixo) e
/// nunca fica abaixo do que a grade vai precisar, que é o lado que importa.
fn head_cut(bytes: &[u8], cols: u16) -> usize {
    if grid_rows(bytes, cols) + GRID_SLACK <= MAX_LINES {
        return 0;
    }
    // A mesma folga que `rows_needed` dá, para o corte e a grade concordarem.
    let budget = MAX_LINES.saturating_sub(GRID_SLACK);
    let mut used = 0;
    let mut end = bytes.len();
    while end > 0 {
        let start = bytes[..end - 1]
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|at| at + 1)
            .unwrap_or(0);
        // O PTY entrega `\r\n` (é o `ONLCR` do termios), e aí cada linha
        // começa na coluna zero. Se um `\n` vier sozinho, a coluna atravessa a
        // quebra e a linha seguinte pode embrulhar uma vez mais cedo — uma
        // linha a mais, no máximo, e é essa que se paga aqui. Sem isso a soma
        // ficaria ABAIXO do que a grade precisa, e a diferença sumiria rolando.
        let carried = start >= 2 && bytes[start - 2] != b'\r';
        let cost = grid_rows(&bytes[start..end], cols) + usize::from(carried);
        if used + cost > budget {
            // Uma linha só maior que o teto inteiro: fica ela, truncada pela
            // grade, em vez de devolver bloco vazio.
            return if used == 0 { start } else { end };
        }
        used += cost;
        end = start;
    }
    0
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
    // O teto é imposto AQUI, no byte, e não depois sobre a lista de linhas: a
    // grade já não deixaria a lista passar de `MAX_LINES`, então o corte de lá
    // era código morto que reportava zero. Ver `head_cut`.
    let cut = head_cut(bytes, cols);
    let kept = &bytes[cut..];
    let height = rows_needed(kept, cols);
    let mut parser = vt100::Parser::new(height, cols, 0);
    parser.process(kept);

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

    // A conversão de unidade vem por último, depois de toda a aritmética de
    // junção e de corte — que só fecha em byte.
    let lines = lines.into_iter().map(into_logical).collect();
    (lines, logical_lines(&bytes[..cut]))
}

/// Intervalo entre fotografias do comando em execução.
///
/// Cinco segundos é o compromisso: um crash perde no máximo isso, e um comando
/// de dez minutos custa 120 gravações, não uma por chunk.
pub const CHECKPOINT_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// O que a thread emitter entrega ao finalizar um comando.
pub struct Finished {
    pub session_id: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub cwd: Option<String>,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub bytes: Vec<u8>,
    pub cols: u16,
    pub rows: u16,
    /// Linhas que o teto de captura comeu antes de o parser ver os bytes.
    pub dropped: usize,
    pub alt_screen: bool,
}

pub struct Checkpoint {
    pub session_id: String,
    pub command: String,
    pub started_at_ms: i64,
    pub bytes: Vec<u8>,
    pub cols: u16,
    pub rows: u16,
}

pub enum Work {
    Finish(Finished),
    Save(Checkpoint),
    Clear(String),
    /// `clear`/`reset`: a lista de blocos some, e some do disco também.
    Wipe(String),
}

/// O comando pede a tela limpa?
///
/// Só ele sozinho: `clear && ls` faz outra coisa depois, e apagar a sessão
/// inteira por causa do primeiro pedaço seria uma surpresa cara.
pub fn wipes_the_screen(command: &str) -> bool {
    matches!(command.trim(), "clear" | "reset")
}

struct Finalizer {
    tx: std::sync::mpsc::SyncSender<Work>,
}

static FINALIZER: std::sync::OnceLock<Finalizer> = std::sync::OnceLock::new();
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Põe o contador de ids vivos acima de tudo que está no disco.
///
/// Reabrir uma sessão mostra os blocos persistidos e os novos na MESMA lista, e
/// o front usa o id como chave. Com o contador nascendo em 1 a cada boot, o
/// primeiro comando de uma sessão reaberta recebe o id 1 — o mesmo do bloco mais
/// antigo que acabou de ser lido do SQLite.
///
/// `fetch_max` e não `store`: chamar duas vezes, ou tarde demais, nunca faz o
/// contador recuar para cima de um id já entregue.
pub fn seed_ids(max_persisted: u64) {
    NEXT_ID.fetch_max(
        max_persisted.saturating_add(1),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// Parse, redação e emissão saem da thread emitter.
///
/// O finalize de um bloco de 10 mil linhas é trabalho de verdade; fazê-lo no
/// caminho quente seguraria o flush de output do terminal por todo esse tempo.
/// A emitter só entrega os bytes e segue.
pub fn install(app: tauri::AppHandle, store: std::sync::Arc<crate::session::store::Store>) {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Work>(16);
    if FINALIZER.set(Finalizer { tx }).is_err() {
        return;
    }
    // Depois do dreno de checkpoints, que também grava blocos: semear antes
    // deixaria o contador abaixo das linhas que o dreno acabou de inserir.
    if let Ok(max) = store.max_block_id() {
        seed_ids(max);
    }
    let _ = std::thread::Builder::new()
        .name("block-finalizer".into())
        .spawn(move || {
            use tauri::Emitter;
            while let Ok(work) = rx.recv() {
                match work {
                    Work::Finish(finished) => {
                        let session = finished.session_id.clone();
                        let block = build(finished);
                        // EMITE ANTES DE GRAVAR. O olho do usuário não pode
                        // esperar o disco: o bloco aparece, e a persistência
                        // acontece depois. O id vale para a sessão viva; ao
                        // reabrir, quem manda é o rowid que a leitura devolve.
                        let event = format!("block://finalized/{session}");
                        let _ = app.emit(&event, block.clone());
                        if let Err(err) = store.insert_block(&block) {
                            eprintln!("tyba: bloco não persistido: {err}");
                        }
                        let _ = store.clear_checkpoint(&session);
                    }
                    Work::Save(cp) => {
                        let _ = store.save_checkpoint(
                            &cp.session_id,
                            &cp.command,
                            cp.started_at_ms,
                            cp.cols,
                            cp.rows,
                            &cp.bytes,
                        );
                    }
                    Work::Clear(session) => {
                        let _ = store.clear_checkpoint(&session);
                    }
                    Work::Wipe(session) => {
                        let _ = store.drop_blocks(&session);
                        let _ = store.clear_checkpoint(&session);
                        let event = format!("block://cleared/{session}");
                        let _ = app.emit(&event, ());
                    }
                }
            }
        });
}

/// Redige a linha inteira, e a linha redigida perde a cor.
///
/// Redação sobre a linha lógica, não sobre chunk cru: chunk pode partir um
/// `sk-…` no meio e o padrão escapar (princípio #10).
///
/// Os runs vão embora porque a redação ENCOLHE o texto — `[REDACTED]` tem 10
/// unidades e o menor segredo que o padrão pega tem 20 — e os offsets não
/// acompanham: todo run a partir do segredo passa a apontar para fora da linha.
/// O front fatia com `String.prototype.slice`, que clampa em silêncio em vez de
/// estourar, então o defeito não aparece como erro: a cor do segredo escorre
/// para o resto da linha.
///
/// Recalcular os offsets junto com o texto preservaria a cor, mas ao preço de
/// mais uma aritmética de offset — e é exatamente esse tipo de aritmética que já
/// desalinhou a cor duas vezes neste arquivo, num caminho (linha com segredo)
/// que quase nunca roda e portanto quase nunca seria observado. Uma linha por
/// sessão sem cor é barato; um offset errado que ninguém vê, não.
///
/// `Cow::Borrowed` é a prova de que a regex não achou nada. É o caminho comum e
/// não paga nada: nem cópia do texto, nem perda de cor.
fn redact_line(line: LogicalLine) -> LogicalLine {
    let redacted = match crate::session::redact::redact(&line.text) {
        std::borrow::Cow::Borrowed(_) => None,
        std::borrow::Cow::Owned(text) => Some(text),
    };
    match redacted {
        None => line,
        Some(text) => LogicalLine {
            text,
            runs: Vec::new(),
        },
    }
}

fn build(finished: Finished) -> Block {
    // Tela alternada não tem corpo para extrair: o que ficou nos bytes é o
    // desenho de um programa, e recortá-lo produziria lixo com cara de saída.
    let (lines, truncated) = if finished.alt_screen {
        (Vec::new(), 0)
    } else {
        extract_lines(&finished.bytes, finished.cols, finished.rows)
    };
    let lines = lines.into_iter().map(redact_line).collect();
    Block {
        id: next_id(),
        session_id: finished.session_id,
        command: crate::session::redact::redact(&finished.command).into_owned(),
        exit_code: finished.exit_code,
        cwd: finished.cwd,
        started_at_ms: finished.started_at_ms,
        finished_at_ms: finished.finished_at_ms,
        lines,
        // As duas perdas somam porque são a MESMA unidade: linhas. Antes o
        // segundo termo era um `bool` convertido em 0 ou 1.
        truncated: truncated + finished.dropped,
        alt_screen: finished.alt_screen,
    }
}

/// Nunca bloqueia: fila cheia descarta o trabalho em vez de segurar o terminal.
pub fn submit(work: Work) {
    if let Some(finalizer) = FINALIZER.get() {
        let _ = finalizer.tx.try_send(work);
    }
}

pub fn finalize(finished: Finished) {
    submit(Work::Finish(finished));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fatia como o `String.prototype.slice` do JS: por unidade UTF-16.
    ///
    /// É o teste ATRAVESSANDO a fronteira. A suíte antiga fatiava com
    /// `&text[run.start..run.end]`, que em Rust é fatiamento por byte — por
    /// isso ela passava verde enquanto o webview pintava errado. Os dois só
    /// concordam em ASCII.
    fn js_slice(text: &str, run: &StyleRun) -> String {
        let units: Vec<u16> = text.encode_utf16().collect();
        String::from_utf16_lossy(&units[run.start_utf16..run.end_utf16])
    }

    /// Roda a saída pelo pipeline e devolve o que o front pintaria colorido.
    fn painted(bytes: &[u8]) -> Vec<String> {
        let (lines, _) = extract_lines(bytes, 40, 5);
        lines
            .iter()
            .flat_map(|line| line.runs.iter().map(|run| js_slice(&line.text, run)))
            .collect()
    }

    #[test]
    fn accented_text_does_not_shift_the_color() {
        // `printf 'ação: \033[31mvermelho\033[0m\n'` — a reprodução mínima do
        // bug. `ação: ` tem 7 bytes e 6 unidades UTF-16, então o offset de byte
        // começava um caractere à direita e a última letra ficava sem cor.
        assert_eq!(
            painted("ação: \x1b[31mvermelho\x1b[0m\r\n".as_bytes()),
            vec!["vermelho"]
        );
    }

    #[test]
    fn cjk_and_emoji_do_not_shift_the_color() {
        // `日` são 3 bytes e 1 unidade; `😀` são 4 bytes e 2 unidades — o único
        // caractere aqui em que unidade UTF-16 não é o mesmo que caractere.
        assert_eq!(
            painted("日本語 \x1b[32mverde\x1b[0m\r\n".as_bytes()),
            vec!["verde"]
        );
        assert_eq!(
            painted("😀😀 \x1b[34mazul\x1b[0m\r\n".as_bytes()),
            vec!["azul"]
        );
    }

    #[test]
    fn the_accent_inside_the_colored_run_survives() {
        assert_eq!(
            painted("erro: \x1b[31mnão encontrado\x1b[0m\r\n".as_bytes()),
            vec!["não encontrado"]
        );
    }

    #[test]
    fn several_runs_on_one_accented_line_each_land_where_they_belong() {
        assert_eq!(
            painted(
                "ç \x1b[31mum\x1b[0m ã \x1b[32mdois\x1b[0m é \x1b[34mtrês\x1b[0m\r\n".as_bytes()
            ),
            vec!["um", "dois", "três"]
        );
    }

    #[test]
    fn the_offsets_survive_the_wrap_join_with_accents() {
        // A junção soma offsets de linha visual; ela só fecha em byte, e a
        // conversão tem de vir depois dela.
        let (lines, _) = extract_lines("ááááá\x1b[31mbbbbb\x1b[0m\r\n".as_bytes(), 5, 5);
        assert_eq!(lines.len(), 1);
        assert_eq!(js_slice(&lines[0].text, &lines[0].runs[0]), "bbbbb");
    }

    /// Chave sintética, escrita à mão — é a que a própria documentação da AWS
    /// usa como exemplo. Fixture de segredo nunca sai de sessão real.
    const FAKE_AWS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

    /// Um `Finished` mínimo: para o que `build` faz aqui, só os bytes importam.
    fn finished(bytes: &[u8]) -> Finished {
        Finished {
            session_id: "s1".into(),
            command: "printf".into(),
            exit_code: Some(0),
            cwd: None,
            started_at_ms: 0,
            finished_at_ms: 1,
            bytes: bytes.to_vec(),
            cols: 80,
            rows: 24,
            dropped: 0,
            alt_screen: false,
        }
    }

    /// Remonta a linha como o componente `Line` do front remonta, devolvendo
    /// `(pedaço, colorido?)` na ordem em que os spans entram no DOM.
    ///
    /// É o mesmo passeio de `src/components/BlockList.tsx` — trecho simples até
    /// o começo do run, trecho colorido até o fim dele, rabo depois do último
    /// run —, com o `slice` do JS, que CLAMPA no comprimento em vez de estourar.
    /// É esse clamp que faz um offset fora do texto virar cor no lugar errado em
    /// vez de um erro que alguém veria.
    fn rendered(line: &LogicalLine) -> Vec<(String, bool)> {
        let units: Vec<u16> = line.text.encode_utf16().collect();
        let cut = |from: usize, to: usize| {
            let from = from.min(units.len());
            let to = to.clamp(from, units.len());
            String::from_utf16_lossy(&units[from..to])
        };
        if line.runs.is_empty() {
            return vec![(line.text.clone(), false)];
        }
        let mut parts = Vec::new();
        let mut cursor = 0usize;
        for run in &line.runs {
            if run.start_utf16 > cursor {
                parts.push((cut(cursor, run.start_utf16), false));
            }
            parts.push((cut(run.start_utf16, run.end_utf16), true));
            cursor = run.end_utf16;
        }
        if cursor < units.len() {
            parts.push((cut(cursor, units.len()), false));
        }
        parts
    }

    /// `export AWS_KEY=<segredo em vermelho> ok`.
    ///
    /// O segredo tem 20 unidades e `[REDACTED]` tem 10: o texto encolhe 10 e
    /// todo offset dali para a frente passa a apontar para fora dele.
    fn redacted_line() -> LogicalLine {
        let bytes = format!("export AWS_KEY=\x1b[31m{FAKE_AWS_KEY}\x1b[0m ok\r\n");
        let mut block = build(finished(bytes.as_bytes()));
        assert_eq!(block.lines.len(), 1);
        block.lines.remove(0)
    }

    #[test]
    fn the_redaction_leaves_no_run_pointing_past_the_text() {
        let line = redacted_line();
        assert!(!line.text.contains(FAKE_AWS_KEY), "{}", line.text);
        let len = line.text.encode_utf16().count();
        for run in &line.runs {
            assert!(
                run.start_utf16 <= run.end_utf16 && run.end_utf16 <= len,
                "{run:?} fora de um texto de {len} unidades: {:?}",
                line.text
            );
        }
    }

    #[test]
    fn the_redaction_never_paints_what_was_not_colored() {
        let line = redacted_line();
        let parts = rendered(&line);

        // O front tem de conseguir remontar o texto inteiro a partir dos runs.
        let joined: String = parts.iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(joined, line.text);

        // E o que ele pinta é a marca da redação ou nada — nunca o ` ok`, que
        // veio depois do run e sem cor nenhuma.
        let colored: String = parts
            .iter()
            .filter(|(_, colored)| *colored)
            .map(|(text, _)| text.as_str())
            .collect();
        assert!(
            colored.is_empty() || colored == crate::session::redact::REDACTION_MARK,
            "pintou {colored:?}"
        );
    }

    #[test]
    fn only_the_line_the_redaction_touched_loses_its_color() {
        let bytes = format!("\x1b[32mok\x1b[0m\r\nAWS_KEY={FAKE_AWS_KEY} \x1b[31mfim\x1b[0m\r\n");
        let block = build(finished(bytes.as_bytes()));
        assert_eq!(block.lines.len(), 2);
        // A linha limpa não paga pelo segredo da vizinha.
        assert_eq!(
            js_slice(&block.lines[0].text, &block.lines[0].runs[0]),
            "ok"
        );
    }

    /// Bloco gravado antes desta mudança guarda offset de BYTE sob as chaves
    /// `start`/`end`. Renomear a chave faria o `serde` falhar e `list_blocks`
    /// descartar a linha inteira em silêncio — o histórico sumiria da tela.
    #[test]
    fn a_block_persisted_before_the_change_still_deserializes() {
        let old_row = r#"[{"text":"ola","runs":[{"start":0,"end":3,"fg":{"kind":"idx","value":1},"bg":{"kind":"default"},"bold":false,"italic":false,"underline":false}]}]"#;
        let lines: Vec<LogicalLine> = serde_json::from_str(old_row).expect("histórico antigo");
        assert_eq!(lines[0].runs[0].start_utf16, 0);
        assert_eq!(lines[0].runs[0].end_utf16, 3);
    }

    #[test]
    fn the_wire_keys_stay_the_ones_already_on_disk() {
        let line = LogicalLine {
            text: "ola".into(),
            runs: vec![StyleRun {
                start_utf16: 0,
                end_utf16: 3,
                fg: Color::Idx(1),
                bg: Color::Default,
                bold: false,
                italic: false,
                underline: false,
            }],
        };
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.contains(r#""start":0"#), "{json}");
        assert!(json.contains(r#""end":3"#), "{json}");
    }

    fn lines_of(bytes: &[u8], cols: u16, rows: u16) -> Vec<String> {
        extract_lines(bytes, cols, rows)
            .0
            .into_iter()
            .map(|line| line.text)
            .collect()
    }

    #[test]
    fn only_clear_and_reset_alone_wipe_the_list() {
        assert!(wipes_the_screen("clear"));
        assert!(wipes_the_screen("reset"));
        assert!(wipes_the_screen("  clear  "));
        // Apagar a sessão inteira porque a linha COMEÇA com clear seria uma
        // surpresa cara — e irreversível.
        assert!(!wipes_the_screen("clear && ls"));
        assert!(!wipes_the_screen("clear-cache"));
        assert!(!wipes_the_screen("echo clear"));
        assert!(!wipes_the_screen(""));
    }

    #[test]
    fn live_ids_start_above_everything_already_on_disk() {
        // Sem isto, o primeiro comando de uma sessão reaberta nasce com id 1 —
        // colidindo com o rowid do bloco mais antigo que ela acabou de ler.
        seed_ids(41);
        let first = next_id();
        let second = next_id();
        assert!(first >= 42, "id vivo abaixo do que está no disco: {first}");
        assert!(second > first);

        // Semear com um valor menor não recua o contador para cima de um id já
        // entregue — é o que torna a chamada segura fora de ordem.
        seed_ids(1);
        assert!(next_id() > second);
    }

    #[test]
    fn capture_drops_the_beginning_instead_of_eating_the_machine() {
        let mut capture = Capture::default();
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..10 {
            capture.push(&chunk);
        }
        assert!(capture.take().len() <= MAX_CAPTURE_BYTES);
    }

    /// Buraco conhecido, fixado aqui para ninguém o "consertar" mentindo.
    ///
    /// Saída sem `\n` nenhum — blob binário, barra de progresso que só usa
    /// `\r` — perde CONTEÚDO sem perder LINHA: a linha lógica continua uma só,
    /// mais curta. Zero é a resposta certa na unidade que o bloco reporta.
    /// Denunciar a perda exigiria um segundo número em bytes, e isso é campo
    /// novo no `Block` e coluna nova no SQLite.
    #[test]
    fn output_without_newlines_loses_bytes_without_losing_lines() {
        let mut capture = Capture::default();
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..10 {
            capture.push(&chunk);
        }
        assert_eq!(capture.dropped(), 0);
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
        assert_eq!(capture.dropped(), 0);
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
        assert_eq!(js_slice(&line.text, run), "vermelho");
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
        assert_eq!(js_slice(&line.text, run), "bbbbb");
    }

    #[test]
    fn empty_output_produces_no_lines() {
        assert!(lines_of(b"", 80, 24).is_empty());
    }

    /// Saída sintética de N linhas numeradas — é o `seq 1 N` do relato, escrito
    /// à mão.
    fn numbered(count: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for i in 1..=count {
            out.extend_from_slice(format!("linha{i}\r\n").as_bytes());
        }
        out
    }

    #[test]
    fn output_past_the_ceiling_says_how_much_it_lost() {
        // O bug: a grade tinha no máximo MAX_LINES linhas, então
        // `lines.len() - MAX_LINES` era SEMPRE zero e o rodapé nunca aparecia.
        // Quarenta mil linhas sumiam com o bloco jurando que estava inteiro.
        let (lines, truncated) = extract_lines(&numbered(MAX_LINES + 5_000), 80, 24);
        assert!(truncated > 0, "perda silenciosa de novo");
        assert!(lines.len() <= MAX_LINES);
        // Cada linha perdida está contada: o que ficou mais o que se foi é o
        // que entrou.
        assert_eq!(lines.len() + truncated, MAX_LINES + 5_000);
    }

    #[test]
    fn the_end_is_what_survives_the_ceiling() {
        // O usuário olha para o fim da saída — é lá que está o erro.
        let (lines, _) = extract_lines(&numbered(MAX_LINES + 100), 80, 24);
        assert_eq!(
            lines.last().map(|line| line.text.as_str()),
            Some(format!("linha{}", MAX_LINES + 100).as_str())
        );
    }

    #[test]
    fn output_within_the_ceiling_is_never_reported_as_cut() {
        let (lines, truncated) = extract_lines(&numbered(200), 80, 24);
        assert_eq!(truncated, 0);
        assert_eq!(lines.len(), 200);
    }

    #[test]
    fn trailing_blank_lines_are_not_mistaken_for_truncation() {
        // O rabo em branco da grade é descartado na leitura; contar a diferença
        // entre grade e linhas faria isso virar "linha cortada".
        let (_, truncated) = extract_lines(b"a\r\n\r\n\r\n", 80, 24);
        assert_eq!(truncated, 0);
    }

    #[test]
    fn a_single_line_taller_than_the_ceiling_is_kept_instead_of_emptying_the_block() {
        let monster = vec![b'x'; MAX_LINES * 80 + 10_000];
        let (lines, _) = extract_lines(&monster, 80, 24);
        assert!(!lines.is_empty(), "bloco vazio é pior que bloco truncado");
    }

    #[test]
    fn the_capture_ceiling_reports_lines_not_a_flag() {
        // `usize::from(dropped)` somava 1: oito megabytes viravam "1 linha".
        let mut capture = Capture::default();
        let line = b"uma linha de saida qualquer\r\n";
        // Enche até o teto, depois empurra mais um megabyte por cima.
        while capture.snapshot().len() + line.len() <= MAX_CAPTURE_BYTES {
            capture.push(line);
        }
        // O que sai é o COMEÇO, então a conta é sobre as linhas velhas que
        // foram expulsas para abrir espaço — não sobre as que entraram.
        let over = numbered(30_000);
        capture.push(&over);
        let evicted = over.len() / line.len();
        assert_eq!(capture.dropped(), evicted);
        assert!(
            capture.dropped() > 1,
            "o código antigo reportava exatamente 1 aqui, para qualquer perda"
        );
    }

    #[test]
    fn the_two_ceilings_add_up_in_the_same_unit() {
        // Uma perda é do teto de captura (bytes, enquanto roda) e a outra do
        // teto do bloco (linhas, no finalize). Só somam porque as duas são
        // convertidas para linha antes.
        let finished = Finished {
            session_id: "s1".into(),
            command: "seq".into(),
            exit_code: Some(0),
            cwd: None,
            started_at_ms: 0,
            finished_at_ms: 1,
            bytes: numbered(MAX_LINES + 100),
            cols: 80,
            rows: 24,
            dropped: 7,
            alt_screen: false,
        };
        let cut_by_the_block = extract_lines(&numbered(MAX_LINES + 100), 80, 24).1;
        let block = build(finished);
        assert!(cut_by_the_block > 0);
        assert_eq!(block.truncated, cut_by_the_block + 7);
    }

    #[test]
    fn wide_characters_are_not_duplicated() {
        let out = lines_of("日本語\r\n".as_bytes(), 20, 5);
        assert_eq!(out, vec!["日本語"]);
    }

    /// Linha "colorida por token", do jeito que `bun install`, `cargo build` e
    /// `pip` escrevem: cada palavra leva o seu próprio SGR. Com 20 tokens são
    /// 282 bytes no cano para 60 colunas na tela — a proporção medida, escrita
    /// à mão. Com 6 tokens, 86 bytes para 18 colunas.
    fn colored_line(tokens: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..tokens {
            // 7 bytes de SGR + 3 imprimíveis + 4 de reset = 14 bytes por token.
            out.extend_from_slice(b"\x1b[1;32mabc\x1b[0m");
        }
        out.extend_from_slice(b"\r\n");
        out
    }

    fn colored_output(lines: usize, tokens: usize) -> Vec<u8> {
        let line = colored_line(tokens);
        let mut out = Vec::with_capacity(lines * line.len());
        for _ in 0..lines {
            out.extend_from_slice(&line);
        }
        out
    }

    /// `docker pull`, `pip`, `curl`: preâmbulo, uma barra de progresso de 1 MB
    /// que redesenha a MESMA linha com `\r`, e o resultado no fim.
    fn progress_bar_output() -> Vec<u8> {
        let mut out = Vec::new();
        for at in 0..500 {
            out.extend_from_slice(format!("preambulo {at}\r\n").as_bytes());
        }
        while out.len() < 1024 * 1024 {
            for pct in 0..=100 {
                out.extend_from_slice(format!("\rbaixando [{pct:>3}%]").as_bytes());
            }
        }
        out.extend_from_slice(b"\r\n");
        for at in 0..10 {
            out.extend_from_slice(format!("resultado {at}\r\n").as_bytes());
        }
        out
    }

    fn texts(lines: &[LogicalLine]) -> Vec<&str> {
        lines.iter().map(|line| line.text.as_str()).collect()
    }

    #[test]
    fn color_per_token_does_not_shrink_what_fits() {
        assert_eq!(colored_line(20).len(), 282, "a fixture mede o que se mediu");
        let out = colored_output(5_000, 20);
        let (lines, truncated) = extract_lines(&out, 80, 24);
        // 60 colunas em 80: cada linha ocupa UMA linha de grade, e cinco mil
        // cabem nas dez mil do teto com folga de sobra.
        assert_eq!(truncated, 0, "descartou saída que cabia inteira");
        assert_eq!(lines.len(), 5_000);
    }

    #[test]
    fn a_short_colored_line_still_costs_one_row() {
        assert_eq!(colored_line(6).len(), 86);
        let out = colored_output(9_000, 6);
        let (lines, truncated) = extract_lines(&out, 80, 24);
        assert_eq!(truncated, 0, "18 colunas em 80 não são duas linhas");
        assert_eq!(lines.len(), 9_000);
    }

    #[test]
    fn the_progress_bar_does_not_swallow_what_came_before_it() {
        // `\r` redesenha a MESMA linha: um megabyte de barra é UMA linha de
        // grade. Medir a barra em bytes fazia ela sozinha estourar o teto e
        // levar junto tudo que veio antes dela.
        let out = progress_bar_output();
        let (lines, truncated) = extract_lines(&out, 80, 24);
        assert_eq!(truncated, 0, "a barra comeu o preâmbulo");
        assert_eq!(
            lines.first().map(|line| line.text.as_str()),
            Some("preambulo 0")
        );
        assert_eq!(
            lines.last().map(|line| line.text.as_str()),
            Some("resultado 9")
        );
        // 500 de preâmbulo + a barra, que é uma linha só + 10 de resultado.
        assert_eq!(lines.len(), 511);
    }

    fn plain_line(at: usize) -> String {
        format!("pacote {at:05} resolvido em 12ms")
    }

    /// A mesma linha, vestida: um rascunho do mesmo tamanho apagado pelo `\r`
    /// que vem atrás, e cada palavra com o seu SGR. Na tela sai idêntica à
    /// crua; no cano pesa três vezes mais.
    fn dressed_line(at: usize) -> Vec<u8> {
        let text = plain_line(at);
        let mut out = Vec::new();
        out.extend_from_slice(b"\x1b[2m");
        out.extend_from_slice(&vec![b'.'; text.len()]);
        out.extend_from_slice(b"\x1b[0m\r");
        let painted: Vec<String> = text
            .split(' ')
            .map(|word| format!("\x1b[1;36m{word}\x1b[0m"))
            .collect();
        out.extend_from_slice(painted.join(" ").as_bytes());
        out.extend_from_slice(b"\r\n");
        out
    }

    fn output_of(lines: usize, line: impl Fn(usize) -> Vec<u8>) -> Vec<u8> {
        let mut out = Vec::new();
        for at in 0..lines {
            out.extend_from_slice(&line(at));
        }
        out
    }

    /// A CLASSE, e não os três casos que a denunciaram.
    ///
    /// O orçamento do que descartar tem de se medir no que a TELA mostra. Byte
    /// que não pinta — SGR, `\r` de redesenho — não ocupa coluna nenhuma, logo
    /// não pode mudar uma linha sequer do que o bloco guarda nem do que ele diz
    /// ter perdido. Toda métrica que conte bytes falha aqui na hora, inclusive
    /// a próxima que tentar o mesmo atalho.
    ///
    /// Junto vai a conservação, nos dois trajes: o que ficou mais o que se foi
    /// é o que entrou. Ela é o outro lado da moeda — pega a métrica que
    /// SUBESTIMA e deixa a grade curta, onde a saída some rolando para fora
    /// sem nem ser contada.
    #[test]
    fn what_does_not_paint_does_not_cost() {
        for count in [100usize, 4_000, MAX_LINES - 1, MAX_LINES + 3_000] {
            let plain = output_of(count, |at| format!("{}\r\n", plain_line(at)).into_bytes());
            let dressed = output_of(count, dressed_line);
            assert!(
                dressed.len() > plain.len() * 2,
                "a fixture não pesa: o traje precisa custar bytes"
            );

            let (plain_lines, plain_cut) = extract_lines(&plain, 80, 24);
            let (dressed_lines, dressed_cut) = extract_lines(&dressed, 80, 24);

            assert_eq!(
                plain_cut, dressed_cut,
                "o traje mudou quanto se descarta (count={count})"
            );
            assert_eq!(
                texts(&plain_lines),
                texts(&dressed_lines),
                "o traje mudou o que sobrou (count={count})"
            );
            assert_eq!(
                plain_lines.len() + plain_cut,
                count,
                "conservação quebrada no cru (count={count})"
            );
            assert_eq!(
                dressed_lines.len() + dressed_cut,
                count,
                "conservação quebrada no vestido (count={count})"
            );
        }
    }

    /// O outro lado da métrica honesta, por dentro: ela não pode SUBESTIMAR.
    ///
    /// Se o custo sair menor do que a saída ocupa, o corte deixa passar mais do
    /// que a grade aguenta, o `clamp` de `rows_needed` morde, e o excedente
    /// rola para fora sem entrar no contador. É a mesma perda silenciosa,
    /// entrando pela outra porta. A folga da grade tem de sobreviver ao corte.
    #[test]
    fn what_survives_the_cut_fits_the_grid() {
        let shapes: Vec<(&str, Vec<u8>)> = vec![
            ("cru", numbered(MAX_LINES + 5_000)),
            ("colorido", colored_output(20_000, 20)),
            ("vestido", output_of(20_000, dressed_line)),
            ("barra de progresso", progress_bar_output()),
            ("redesenho de camadas", layered_redraw_output()),
        ];
        for (name, out) in shapes {
            let cut = head_cut(&out, 80);
            assert!(
                grid_rows(&out[cut..], 80) + GRID_SLACK <= MAX_LINES,
                "o que sobrou do corte não cabe na grade ({name})"
            );
        }
    }

    /// `docker pull`: cada refresh reimprime as MESMAS camadas e volta com
    /// `CSI nA`. São oito linhas na tela, não dezesseis mil.
    fn layered_redraw_output() -> Vec<u8> {
        let mut out = Vec::new();
        for round in 0..2_000 {
            for layer in 0..8 {
                out.extend_from_slice(format!("camada {layer}: {}%\r\n", round % 100).as_bytes());
            }
            out.extend_from_slice(b"\x1b[8A");
        }
        out
    }

    #[test]
    fn redrawing_the_same_lines_costs_those_lines_once() {
        let out = layered_redraw_output();
        let (lines, truncated) = extract_lines(&out, 80, 24);
        assert_eq!(truncated, 0, "contou cada refresh como linha nova");
        assert_eq!(lines.len(), 8, "oito camadas na tela, oito linhas");
    }

    #[test]
    fn the_metric_counts_columns_and_not_bytes() {
        // Escape não pinta: um SGR de sete bytes ocupa zero coluna.
        assert_eq!(grid_rows(b"\x1b[1;32mabc\x1b[0m", 80), 1);
        // `\r` volta à margem: o redesenho não consome linha.
        assert_eq!(grid_rows(&vec![b'x'; 10_000], 4), 2_500);
        let mut redraw = Vec::new();
        for _ in 0..10_000 {
            redraw.extend_from_slice(b"\rxxxx");
        }
        assert_eq!(grid_rows(&redraw, 4), 1);
        // O wrap sai da largura, não do buffer.
        assert_eq!(grid_rows(b"abcdefghij", 5), 2);
        // Caractere largo ocupa duas colunas.
        assert_eq!(grid_rows("日本語".as_bytes(), 4), 2);
        // Traço de tabela também tem 3 bytes, e UMA coluna: tratá-lo como
        // largo cortaria pela metade toda saída em tabela.
        assert_eq!(grid_rows("──────────".as_bytes(), 5), 2);
        // `\n` no fim não inventa linha — o rabo em branco não é saída.
        assert_eq!(grid_rows(b"um\r\ndois\r\n", 80), 2);
    }
}
