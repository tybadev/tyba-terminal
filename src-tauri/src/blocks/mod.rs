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

/// Quantas linhas de grade a saída ocupa, sem teto.
///
/// Cada `\n` começa uma linha, e o resto só pode acrescentar por wrap — usar o
/// tamanho em bytes como proxy dos imprimíveis superestima (escape conta como
/// texto). Superestimar é o lado seguro: a grade nasce maior do que precisa,
/// nunca menor, e é ser "nunca menor" que garante que nada role para fora dela.
fn rows_of(bytes: &[u8], cols: u16) -> usize {
    let newlines = logical_lines(bytes);
    let wrapped = bytes.len() / cols.max(1) as usize;
    newlines + wrapped + 2
}

/// Altura da grade para esta saída, com o teto do bloco.
fn rows_needed(bytes: &[u8], cols: u16) -> u16 {
    rows_of(bytes, cols).clamp(1, MAX_LINES) as u16
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
/// recebe cabe inteiro, e o que ficou de fora está medido. O corte cai logo
/// depois de um `\n` para não partir linha ao meio; o preço é o estado de cor
/// herdado do trecho descartado, e quem colore reemite o SGR a cada linha.
fn head_cut(bytes: &[u8], cols: u16) -> usize {
    if rows_of(bytes, cols) <= MAX_LINES {
        return 0;
    }
    let width = cols.max(1) as usize;
    // A mesma folga de 2 que `rows_of` dá, para o corte e a grade concordarem.
    let budget = MAX_LINES.saturating_sub(2);
    let mut used = 0;
    let mut end = bytes.len();
    while end > 0 {
        let start = bytes[..end - 1]
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|at| at + 1)
            .unwrap_or(0);
        let cost = 1 + (end - start) / width;
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

fn build(finished: Finished) -> Block {
    // Tela alternada não tem corpo para extrair: o que ficou nos bytes é o
    // desenho de um programa, e recortá-lo produziria lixo com cara de saída.
    let (lines, truncated) = if finished.alt_screen {
        (Vec::new(), 0)
    } else {
        extract_lines(&finished.bytes, finished.cols, finished.rows)
    };
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
}
