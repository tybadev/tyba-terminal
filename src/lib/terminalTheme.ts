import type { ITheme } from "@xterm/xterm";

import type { TerminalPalette } from "./ipc";

/**
 * Só cor que o CSSOM entende passa para o xterm.
 *
 * O fundo sai de uma custom property, e custom property guarda TEXTO: um tema
 * importado pode trazer ali qualquer coisa. Sem o filtro, o que não for cor vira
 * fundo vazio no canvas do terminal.
 */
const CSS_COLOR = /^(#[0-9a-f]{3,8}|rgba?\(|hsla?\()/i;

export function isCssColor(value: string): boolean {
  return CSS_COLOR.test(value.trim());
}

/**
 * A paleta do tema vira tema do xterm, com o FUNDO vindo do design system.
 *
 * A área de shell tinha duas fontes de verdade para a mesma cor: painel, lista
 * e cartões pintavam com `--tyba-sunken`, e a caixa do xterm com o `background`
 * da paleta do tema. Comparados os dois valores nos 17 temas, 16 divergiam —
 * `dracula` dá `#1e1f29` contra `#282a36`, `mono-dark` dá `#0f0d0d` contra
 * `#151313`. Como o terminal ocupa uma faixa fixa de metade da altura do painel
 * (`LIVE_FRACTION`), a diferença aparecia como um degrau cortando o painel ao
 * meio. Só o `tyba-dark` casava, e é por isso que isso passou despercebido.
 *
 * O token de UI vence, e não o contrário, por dois motivos: a hierarquia de
 * superfícies do design system continua íntegra (pelo caminho oposto, no
 * `github-dark` o sunken viraria igual ao fundo do app e a área de terminal
 * deixaria de ser uma camada mais funda); e tema importado não consegue reabrir
 * a costura — corrigir os 16 temas na mão não fecharia essa porta.
 *
 * O resto da paleta continua do tema: as 16 cores ANSI, o foreground, o cursor
 * e a seleção. Só o fundo é do TYBA.
 *
 * `surface` ausente ou impresentável devolve o fundo da própria paleta, que é o
 * comportamento de antes — o terminal nunca fica sem fundo.
 */
export function terminalTheme(
  palette: TerminalPalette,
  surface: string | null,
): ITheme {
  const [
    black, red, green, yellow, blue, magenta, cyan, white,
    brightBlack, brightRed, brightGreen, brightYellow,
    brightBlue, brightMagenta, brightCyan, brightWhite,
  ] = palette.ansi;
  const background =
    surface && isCssColor(surface) ? surface.trim() : palette.background;
  // `cursorAccent` é a cor do glifo DEBAIXO do cursor em bloco, e todo tema a
  // declara igual ao fundo. Deixá-la para trás pintaria o caractere sob o
  // cursor na cor de um fundo que não existe mais. Quem tiver escolhido outra
  // coisa de propósito continua com a escolha.
  const sameAsBackground =
    palette.cursorAccent?.toLowerCase() === palette.background.toLowerCase();
  const cursorAccent = sameAsBackground ? background : palette.cursorAccent;
  return {
    background,
    foreground: palette.foreground,
    cursor: palette.cursor,
    cursorAccent,
    selectionBackground: palette.selectionBackground,
    black, red, green, yellow, blue, magenta, cyan, white,
    brightBlack, brightRed, brightGreen, brightYellow,
    brightBlue, brightMagenta, brightCyan, brightWhite,
  };
}
