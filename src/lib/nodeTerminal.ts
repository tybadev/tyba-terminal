/**
 * Como o terminal de um nó do grafo se comporta no zoom do canvas.
 *
 * O xterm fica montado em qualquer zoom (dá para digitar sempre). O WebGL
 * pinta numa textura do tamanho do nó, e o transform do canvas estica essa
 * textura: fora de 100% o texto sai borrado. O renderer DOM é texto de
 * verdade, que o navegador rasteriza já na escala final — nítido. As colunas
 * não mudam com o zoom (o xterm mede a célula por `offsetWidth`, que ignora
 * o transform), então o PTY não é redimensionado.
 */
export function nodeTerminalWebgl(zoom: number): boolean {
  return Math.round(zoom * 100) === 100;
}

/**
 * Classes do invólucro do terminal no nó. A roda é do terminal (rolar o
 * scrollback) só com o nó selecionado; senão é do canvas (pan/zoom), para o
 * canvas cheio de nós não ter roda morta. Arrastar e teclado são sempre do
 * terminal (selecionar texto, digitar).
 */
export function nodeTerminalClass(selected: boolean): string {
  return selected ? "nodrag nowheel nopan nokey" : "nodrag nopan nokey";
}
