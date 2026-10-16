/**
 * Como o terminal de um nó do grafo se comporta no zoom do canvas.
 *
 * O xterm nunca fica sob a escala do canvas: o invólucro dele desfaz o zoom
 * (`scale(1/zoom)`) e o texto é desenhado já no tamanho da tela, com a fonte
 * `base × zoom`. A escala total fica 1, então o WebGL pinta nítido e o mouse
 * do xterm (que divide a posição na tela pela célula SEM escala) acerta a
 * célula. As colunas e linhas são as do fit em 100% e não mudam com o zoom:
 * o PTY nunca é redimensionado por zoom.
 */

export interface Cell {
  w: number;
  h: number;
}

export interface Grid {
  cols: number;
  rows: number;
}

/** O mínimo do FitAddon do xterm. */
const MIN_COLS = 2;
const MIN_ROWS = 1;

/**
 * Colunas e linhas do nó: as contas do FitAddon em 100% — a caixa do nó sem
 * zoom (`box`, px do layout, que o transform do canvas não muda) menos a
 * barra de rolagem, dividida pela célula na fonte base. Sem zoom aqui de
 * propósito.
 */
export function nodeGrid(box: Cell, cell: Cell, scrollbar: number): Grid {
  return {
    cols: Math.max(MIN_COLS, Math.floor(Math.max(0, box.w - scrollbar) / cell.w)),
    rows: Math.max(MIN_ROWS, Math.floor(box.h / cell.h)),
  };
}

/**
 * O invólucro do xterm: `zoom ×` o tamanho do nó, encolhido de volta por
 * `scale(1/zoom)`. Sob o `scale(zoom)` do canvas, a escala total é 1.
 */
export function nodeZoomStyle(zoom: number) {
  return {
    position: "absolute" as const,
    left: 0,
    top: 0,
    width: `${zoom * 100}%`,
    height: `${zoom * 100}%`,
    transform: `scale(${1 / zoom})`,
    transformOrigin: "0 0",
  };
}

/** Passos de recuo da fonte quando a grade não cabe; cada um tira 2%. */
const FONT_STEPS = 12;
const FONT_STEP = 0.98;
/** Folga da comparação (px): arredondamento de medida, não corte. */
const FIT_SLACK = 0.5;

/**
 * A fonte do nó no zoom: `base × zoom`, a menos que a grade fixa não caiba
 * na caixa. A célula do WebGL não é linear na fonte (`floor(largura × dpr) /
 * dpr`, e a altura arredonda para cima): em 150% ela pode sair maior que
 * 1,5× a de 100%, e a última coluna ficaria cortada. Então recua a fonte até
 * a grade caber. `measure` aplica a fonte no xterm e devolve a célula que
 * ele vai desenhar; quem chama fica com a última fonte aplicada.
 */
export function nodeFontSize(
  base: number,
  zoom: number,
  grid: Grid,
  avail: Cell,
  measure: (fontSize: number) => Cell,
): number {
  let size = base * zoom;
  for (let i = 0; i < FONT_STEPS; i++) {
    const cell = measure(size);
    const fits =
      grid.cols * cell.w <= avail.w + FIT_SLACK && grid.rows * cell.h <= avail.h + FIT_SLACK;
    if (fits || i === FONT_STEPS - 1) return size;
    size *= FONT_STEP;
  }
  return size;
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
