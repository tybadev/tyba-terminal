//! O que o manifesto de tela enxerga, e o portão que evita reavaliar à toa.
//!
//! O `vt100::Parser` que o PTY já mantém por sessão entrega tudo: `title()` sai
//! do OSC 0/2, `contents()` sai da tela **já sem ANSI** e `alternate_screen()`
//! diz se um app de tela cheia está no comando. Não há raspagem de bytes crus
//! aqui — este módulo só recorta e resume.
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

/// As `n` últimas linhas não vazias de um `contents()`, de cima para baixo.
///
/// Linha em branco é descartada, não conta para o `n`: o terminal quase sempre
/// termina em linhas vazias (o cursor está no meio da tela), e contá-las faria
/// a região "últimas 3 linhas" devolver três linhas vazias exatamente quando o
/// agente acabou de escrever algo.
pub fn bottom_non_empty_lines(contents: &str, n: usize) -> Vec<String> {
    let n = n.min(MAX_REGION_LINES);
    let mut found: Vec<String> = contents
        .lines()
        .rev()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .take(n)
        .map(str::to_string)
        .collect();
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
            bottom_non_empty_lines(&screen.contents(), region_lines)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> vt100::Parser {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(bytes);
        parser
    }

    #[test]
    fn pega_as_ultimas_nao_vazias_de_cima_para_baixo() {
        let contents = "primeira\nsegunda\nterceira\nquarta\n";
        assert_eq!(
            bottom_non_empty_lines(contents, 2),
            vec!["terceira", "quarta"]
        );
    }

    #[test]
    fn linha_em_branco_nao_consome_a_cota() {
        // O caso que a implementação ingênua erra: o terminal termina em linhas
        // vazias quase sempre, e contá-las devolveria vazio justamente quando o
        // agente acabou de escrever.
        let contents = "trabalhando\n\n\n\n\n";
        assert_eq!(bottom_non_empty_lines(contents, 3), vec!["trabalhando"]);
    }

    #[test]
    fn menos_linhas_que_o_pedido_devolve_o_que_ha() {
        assert_eq!(bottom_non_empty_lines("so uma\n", 5), vec!["so uma"]);
        assert!(bottom_non_empty_lines("\n\n\n", 3).is_empty());
    }

    #[test]
    fn o_teto_de_regiao_vence_o_pedido_do_manifesto() {
        // Um manifesto de terceiro não transforma "olhar a tela" em "varrer o
        // scrollback" pedindo uma região gigante.
        let contents: String = (1..=40).map(|i| format!("linha{i}\n")).collect();
        assert_eq!(
            bottom_non_empty_lines(&contents, 999).len(),
            MAX_REGION_LINES
        );
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
}
