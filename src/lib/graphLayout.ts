// Onde um nó novo cai no canvas sem cobrir outro (canvas-6, canvas-8, nos-8).
// Puro: recebe os retângulos ocupados e devolve a posição. O core faz o
// mesmo para os nós que o orquestrador cria (`orchestrator::free_spot`).

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Folga mínima entre dois nós. */
export const LAYOUT_GAP = 40;

/** Quantas colunas (ou linhas) tentar antes de desistir e ficar na última. */
const MAX_LANES = 12;
/** Quantos desvios dentro de uma coluna (ou linha). */
const MAX_HOPS = 60;

const overlaps = (a: Rect, b: Rect, gap: number) =>
  a.x < b.x + b.w + gap && b.x < a.x + a.w + gap && a.y < b.y + b.h + gap && b.y < a.y + a.h + gap;

/**
 * A primeira posição livre a partir de `want` para um nó do tamanho `size`.
 * `slide: "down"` desce passando por baixo de quem estiver no caminho e, se a
 * coluna não tiver espaço, abre outra à direita (criados à direita do
 * criador, nó novo perto do selecionado). `slide: "right"` anda para a
 * direita e, sem espaço, desce uma linha (bloco de subagentes embaixo do pai).
 */
export function freeSpot(
  want: { x: number; y: number },
  size: { w: number; h: number },
  occupied: readonly Rect[],
  slide: "down" | "right" = "down",
  gap = LAYOUT_GAP,
): { x: number; y: number } {
  let last = { x: want.x, y: want.y };
  for (let lane = 0; lane < MAX_LANES; lane++) {
    let x = slide === "down" ? want.x + lane * (size.w + gap) : want.x;
    let y = slide === "down" ? want.y : want.y + lane * (size.h + gap);
    for (let hop = 0; hop < MAX_HOPS; hop++) {
      const box = { x, y, w: size.w, h: size.h };
      const hit = occupied.find((r) => overlaps(box, r, gap));
      if (!hit) return { x: Math.round(x), y: Math.round(y) };
      last = { x, y };
      if (slide === "down") y = hit.y + hit.h + gap;
      else x = hit.x + hit.w + gap;
      // Desviar para longe demais vira outra coluna (ou linha).
      if (slide === "down" ? y - want.y > 4000 : x - want.x > 4000) break;
    }
  }
  return { x: Math.round(last.x), y: Math.round(last.y) };
}

/** O retângulo de um nó do canvas (o tamanho medido vale sobre o do modelo). */
export function nodeRect(n: {
  position: { x: number; y: number };
  width?: number;
  height?: number;
  data?: { node?: { size: { w: number; h: number } } };
}): Rect {
  return {
    x: n.position.x,
    y: n.position.y,
    w: n.width ?? n.data?.node?.size.w ?? 0,
    h: n.height ?? n.data?.node?.size.h ?? 0,
  };
}

/** Logo à direita de `anchor`, alinhado pelo topo: onde nasce o vizinho dele. */
export const rightOf = (anchor: Rect, gap = 120) => ({
  x: anchor.x + anchor.w + gap,
  y: anchor.y,
});

/**
 * Algum nó sai da área visível? Então o minimapa liga sozinho (canvas-10).
 * `view` é o transform do canvas (`x`, `y`, `zoom`) e `size` a área em px.
 */
export function outsideView(
  rects: readonly Rect[],
  view: { x: number; y: number; zoom: number },
  size: { w: number; h: number },
): boolean {
  if (size.w <= 0 || size.h <= 0 || view.zoom <= 0) return false;
  const left = -view.x / view.zoom;
  const top = -view.y / view.zoom;
  const right = left + size.w / view.zoom;
  const bottom = top + size.h / view.zoom;
  return rects.some((r) => r.x < left || r.y < top || r.x + r.w > right || r.y + r.h > bottom);
}
