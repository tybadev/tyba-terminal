/**
 * Quanto espaço um bloco vai ocupar, ANTES de ele ser desenhado.
 *
 * O virtualizador posiciona cada cartão pela estimativa e só corrige quando o
 * elemento existe e é medido. Item que nunca foi montado fica onde a estimativa
 * disse — então estimativa errada não é imprecisão, é cartão em cima de cartão.
 *
 * O erro que motivou este módulo: a conta antiga era `linhas × altura de linha`,
 * e linha ali era linha LÓGICA. O corpo do cartão desenha com
 * `whitespace-pre-wrap`, ou seja, uma linha de 200 caracteres num painel de 80
 * colunas ocupa três. Num split a estimativa errava por 2–3× e a lista
 * embaralhava.
 */

/** Uma linha lógica ocupa quantas linhas na tela, dada a largura em colunas. */
export function visualLines(text: string, cols: number): number {
  // Largura desconhecida cai no comportamento de antes: uma linha lógica, uma
  // linha visual. Chutar para cima aqui daria uma barra de rolagem que mente.
  if (!Number.isFinite(cols) || cols <= 0) return 1;
  const len = text.length;
  if (len <= cols) return 1;
  return Math.ceil(len / cols);
}

/**
 * Quantas linhas visuais o corpo de um bloco ocupa, até `limit` linhas lógicas.
 *
 * `limit` é o mesmo teto de linhas desenhadas do cartão (`BODY_LIMIT`): o que
 * está atrás do "mostrar tudo" não ocupa altura porque não está no DOM.
 */
export function bodyLines(
  lines: ReadonlyArray<{ text: string }>,
  cols: number,
  limit: number,
): number {
  const drawn = Math.min(lines.length, limit);
  let total = 0;
  for (let i = 0; i < drawn; i += 1) total += visualLines(lines[i].text, cols);
  return total;
}

/**
 * Quantos caracteres cabem na largura útil do cartão.
 *
 * `charWidthPx` vem MEDIDO do xterm (largura da tela ÷ colunas), pelo mesmo
 * motivo que a altura de linha vem medida: o avanço do glifo numa Nerd Font não
 * é o que a conta a partir do `font-size` diria.
 *
 * Zero significa "não sei" — quem chama trata como largura desconhecida.
 */
export function columnsFor(
  widthPx: number,
  charWidthPx: number,
  paddingPx: number,
): number {
  if (!Number.isFinite(widthPx) || widthPx <= 0) return 0;
  if (!Number.isFinite(charWidthPx) || charWidthPx <= 0) return 0;
  const pad = Number.isFinite(paddingPx) && paddingPx > 0 ? paddingPx : 0;
  return Math.max(1, Math.floor((widthPx - pad) / charWidthPx));
}
