//! O que o manifesto de tela enxerga, e o portão que evita reavaliar à toa.
//!
//! O `vt100::Parser` que o PTY já mantém por sessão entrega tudo: `title()` sai
//! do OSC 0/2, o texto da tela sai **já sem ANSI** e `alternate_screen()` diz se
//! um app de tela cheia está no comando. Não há raspagem de bytes crus aqui —
//! este módulo só recorta e resume.
//!
//! O recorte é lido **linha a linha, de baixo para cima**, e não do
//! `contents()`: ver [`row_text`]. A tela é lida sob o lock do PTY, e renderizar
//! oitenta linhas para ficar com oito é o tipo de custo que só aparece na
//! máquina de quem abriu o terminal em tela cheia.
//!
//! O recorte é a defesa contra o risco central: **scrollback antigo não é estado
//! atual**. Um "Do you want to proceed?" de dez minutos atrás continua no
//! buffer, e olhar a tela inteira classificaria a sessão como bloqueada para
//! sempre.

use std::hash::{Hash, Hasher};

/// Quantas linhas do fim da tela o manifesto pode olhar.
///
/// Teto, não default: uma regra pede menos, nunca mais. É o que impede um
/// manifesto de terceiro de transformar "olhar a tela" em "varrer o scrollback".
pub const MAX_REGION_LINES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScreenSnapshot {
    /// OSC 0/2. É a fonte de identidade no SSH, onde não há árvore de processos.
    pub title: String,
    /// Tela cheia no comando (`htop`, `vim`, `git log`). Enquanto verdadeiro, o
    /// manifesto não opina: o que está na tela é do app, não do agente.
    pub alt_screen: bool,
    /// As últimas linhas não vazias, de cima para baixo.
    pub bottom_lines: Vec<String>,
}

impl ScreenSnapshot {
    /// Resumo barato do que o manifesto olharia.
    ///
    /// O portão de sequência compara isto entre chunks: igual significa que
    /// nada que importa mudou, e a avaliação inteira é pulada. É o que mantém a
    /// regra de terceiro fora do caminho quente na esmagadora maioria dos
    /// chunks — output rolando embaixo de um título estável não reavalia nada.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.title.hash(&mut hasher);
        self.alt_screen.hash(&mut hasher);
        self.bottom_lines.hash(&mut hasher);
        hasher.finish()
    }
}

/// O texto de UMA linha física, e se ela continua na de baixo.
///
/// `contents_between(row, 0, row + 1, 0)` e não `contents()`: o `contents()`
/// renderiza a tela **inteira** para entregar oito linhas, e isso acontece sob
/// o lock de tela, por sessão, a cada flush. O `contents_between` com
/// `start_row < end_row` cai no ramo que faz `skip(start_row)` sobre um
/// iterador de slice — pula linha sem renderizar —, e o `end_col = 0` faz a
/// linha seguinte contribuir com zero caractere. Sai uma linha, custa uma.
///
/// O booleano é wrap: o vt100 só escreve o `\n` quando a linha **não** continua
/// na de baixo, e é essa ausência que denuncia o wrap — não há `wrapped()`
/// público. Sem isso, uma linha lógica que passou da margem viraria duas, e um
/// `contains` que atravessa a dobra deixaria de casar.
fn row_text(screen: &vt100::Screen, row: u16) -> (String, bool) {
    let mut raw = screen.contents_between(row, 0, row + 1, 0);
    let wrapped = !raw.ends_with('\n');
    if !wrapped {
        raw.pop();
    }
    (raw, wrapped)
}

fn push_line(found: &mut Vec<String>, line: Option<String>) {
    let Some(text) = line else {
        return;
    };
    // Linha em branco é descartada, não conta para o `n`: o terminal quase
    // sempre termina em linhas vazias (o cursor está no meio da tela), e
    // contá-las faria a região "últimas 3 linhas" devolver três linhas vazias
    // exatamente quando o agente acabou de escrever algo.
    if !text.trim().is_empty() {
        found.push(text.trim_end().to_string());
    }
}

/// As `n` últimas linhas lógicas não vazias da tela, de cima para baixo.
///
/// Lê de baixo para cima e para assim que junta as `n` — é o que evita
/// renderizar a tela inteira. Uma linha lógica pode ocupar mais de uma física
/// (wrap), e as duas são coladas de volta antes de contar.
pub fn bottom_lines(screen: &vt100::Screen, n: usize) -> Vec<String> {
    let n = n.min(MAX_REGION_LINES);
    let (rows, _) = screen.size();
    let mut found: Vec<String> = Vec::with_capacity(n);
    // A parte de cima de uma linha lógica cujo fim já foi lido.
    let mut line: Option<String> = None;

    for row in (0..rows).rev() {
        let (text, wrapped) = row_text(screen, row);
        if wrapped {
            line = Some(match line {
                Some(rest) => text + &rest,
                None => text,
            });
            continue;
        }
        // Linha física que não continua: fecha a lógica que vinha de baixo.
        push_line(&mut found, line.take());
        if found.len() >= n {
            return finish(found);
        }
        line = Some(text);
    }
    push_line(&mut found, line.take());
    found.truncate(n);
    finish(found)
}

fn finish(mut found: Vec<String>) -> Vec<String> {
    found.reverse();
    found
}

/// O recorte da tela de uma sessão.
pub fn snapshot(screen: &vt100::Screen, region_lines: usize) -> ScreenSnapshot {
    ScreenSnapshot {
        title: screen.title().to_string(),
        alt_screen: screen.alternate_screen(),
        // Tela cheia não tem região útil: o conteúdo é do app, e recortá-lo só
        // gastaria hash para depois ser ignorado.
        bottom_lines: if screen.alternate_screen() {
            Vec::new()
        } else {
            bottom_lines(screen, region_lines)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> vt100::Parser {
        parse_sized(24, 80, bytes)
    }

    fn parse_sized(rows: u16, cols: u16, bytes: &[u8]) -> vt100::Parser {
        let mut parser = vt100::Parser::new(rows, cols, 0);
        parser.process(bytes);
        parser
    }

    /// O texto que uma sequência de linhas desenha, com `\r\n` de verdade — a
    /// unidade em que a região é afirmável é a TELA, não uma string de entrada.
    fn screen_of(lines: &[&str]) -> vt100::Parser {
        parse(lines.join("\r\n").as_bytes())
    }

    #[test]
    fn pega_as_ultimas_nao_vazias_de_cima_para_baixo() {
        let parser = screen_of(&["primeira", "segunda", "terceira", "quarta"]);
        assert_eq!(bottom_lines(parser.screen(), 2), vec!["terceira", "quarta"]);
    }

    #[test]
    fn linha_em_branco_nao_consome_a_cota() {
        // O caso que a implementação ingênua erra: o terminal termina em linhas
        // vazias quase sempre, e contá-las devolveria vazio justamente quando o
        // agente acabou de escrever.
        let parser = screen_of(&["trabalhando", "", "", "", ""]);
        assert_eq!(bottom_lines(parser.screen(), 3), vec!["trabalhando"]);
    }

    #[test]
    fn menos_linhas_que_o_pedido_devolve_o_que_ha() {
        assert_eq!(bottom_lines(screen_of(&["so uma"]).screen(), 5), ["so uma"]);
        assert!(bottom_lines(screen_of(&["", "", ""]).screen(), 3).is_empty());
    }

    #[test]
    fn o_teto_de_regiao_vence_o_pedido_do_manifesto() {
        // Um manifesto de terceiro não transforma "olhar a tela" em "varrer o
        // scrollback" pedindo uma região gigante.
        let linhas: Vec<String> = (1..=20).map(|i| format!("linha{i}")).collect();
        let parser = screen_of(&linhas.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(bottom_lines(parser.screen(), 999).len(), MAX_REGION_LINES);
    }

    /// Linha que passou da margem é UMA linha lógica, não duas. Sem colar as
    /// físicas de volta, um `contains` que atravessa a dobra deixa de casar —
    /// e a dobra depende da largura da janela do usuário, não do agente.
    #[test]
    fn linha_dobrada_pela_margem_volta_inteira() {
        let comprida = "x".repeat(30) + "esc to interrupt";
        let parser = parse_sized(24, 20, format!("antes\r\n{comprida}").as_bytes());

        let linhas = bottom_lines(parser.screen(), 2);

        assert_eq!(linhas, vec!["antes", comprida.as_str()]);
    }

    #[test]
    fn o_titulo_sai_do_osc_sem_precisar_de_parser_proprio() {
        let parser = parse(b"\x1b]0;Codex \xe2\x80\x94 Action Required\x07pronto\r\n");
        let snap = snapshot(parser.screen(), 3);

        assert_eq!(snap.title, "Codex — Action Required");
        assert!(!snap.alt_screen);
    }

    #[test]
    fn tela_cheia_nao_entrega_regiao() {
        // Enquanto `htop` está no comando, o que está na tela é dele. Recortar
        // ali seria classificar o agente pelo conteúdo de outro programa.
        let parser = parse(b"\x1b[?1049hCPU 100%\r\nMEM 2G\r\n");
        let snap = snapshot(parser.screen(), 3);

        assert!(snap.alt_screen);
        assert!(snap.bottom_lines.is_empty());
    }

    #[test]
    fn o_portao_muda_quando_o_titulo_muda() {
        let a = snapshot(parse(b"\x1b]0;Ready\x07x\r\n").screen(), 3);
        let b = snapshot(parse(b"\x1b]0;Working\x07x\r\n").screen(), 3);

        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn o_portao_muda_quando_a_regiao_muda() {
        let a = snapshot(parse(b"\x1b]0;t\x07esperando\r\n").screen(), 3);
        let b = snapshot(parse(b"\x1b]0;t\x07rodando\r\n").screen(), 3);

        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn o_portao_nao_muda_quando_a_mudanca_esta_fora_da_regiao() {
        // A prova de que o portão de fato porta: linha antiga saindo da região
        // não pode custar uma reavaliação de manifesto. Sem isto, o gate seria
        // decoração e a regra de terceiro rodaria a cada chunk.
        let mut a = vt100::Parser::new(24, 80, 0);
        a.process(b"\x1b]0;t\x07antiga um\r\nfim\r\n");
        let mut b = vt100::Parser::new(24, 80, 0);
        b.process(b"\x1b]0;t\x07antiga DOIS\r\nfim\r\n");

        assert_eq!(
            snapshot(a.screen(), 1).fingerprint(),
            snapshot(b.screen(), 1).fingerprint()
        );
    }

    /// Uma tela de agente inteira recortada em oito linhas, com folga contra o
    /// erro que este código existe para evitar: ler linha por linha com uma API
    /// que renderiza tudo acima dela a cada leitura (o `nth` do `contents_between`
    /// com `start_row == end_row` faz isso) custaria uma ordem de grandeza.
    ///
    /// Medido em 2026-08-24 (Apple Silicon, tela cheia de texto):
    ///
    /// | tela    | `contents()` | por linha | ganho |
    /// |---------|--------------|-----------|-------|
    /// | 24x80   | 41 us        | 15 us     | 2,8x  |
    /// | 50x200  | 52 us        | 10 us     | 5,4x  |
    /// | 80x400  | 68 us        | 9 us      | 7,2x  |
    ///
    /// (em debug, o mesmo 80x400 vai de 595 us para 75 us.)
    ///
    /// O ganho **some** quando a tela está quase toda em branco — um shell com
    /// quatro linhas no topo de 80: 19,5 us antes, 21,1 us depois, porque a
    /// varredura sobe até achar texto e o vt100 não tem como dizer "linha
    /// vazia" sem percorrer as colunas. Não é regressão de valor: é o caso em
    /// que não havia o que ganhar, e ele continua duas ordens de grandeza
    /// abaixo do orçamento.
    #[test]
    fn recortar_a_tela_cheia_cabe_no_orcamento() {
        let mut parser = vt100::Parser::new(80, 400, 0);
        let texto: String = (0..80)
            .map(|i| format!("linha {i} com texto de agente ocupando um tanto da largura\r\n"))
            .collect();
        parser.process(texto.as_bytes());

        const RODADAS: u32 = 200;
        let inicio = std::time::Instant::now();
        for _ in 0..RODADAS {
            std::hint::black_box(snapshot(
                std::hint::black_box(parser.screen()),
                MAX_REGION_LINES,
            ));
        }
        let por_recorte = inicio.elapsed() / RODADAS;

        assert!(
            por_recorte < std::time::Duration::from_micros(500),
            "recorte levou {por_recorte:?} — a leitura por linha voltou a renderizar a tela inteira"
        );
    }
}
