// Onde um nó novo cai no canvas sem cobrir outro (canvas-6, canvas-8, nos-8).
// Puro: recebe os retângulos ocupados e devolve a posição. O core faz o
// mesmo para os nós que o orquestrador cria (`orchestrator::free_spot`).

import type { RingState } from "./orbitCard";

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
  data?: unknown;
}): Rect {
  const size = (n.data as { node?: { size?: { w: number; h: number } } } | undefined)?.node?.size;
  return {
    x: n.position.x,
    y: n.position.y,
    w: n.width ?? size?.w ?? 0,
    h: n.height ?? size?.h ?? 0,
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

/**
 * O nó aparece na tela (com `margin` px de folga em volta)? O terminal do nó
 * só monta o xterm quando aparece; fora da tela fica a prévia em texto, para
 * o canvas cheio de nós não manter um xterm por sessão. Área ainda sem
 * medida conta como à vista.
 */
export function onScreen(
  rect: Rect,
  view: { x: number; y: number; zoom: number },
  size: { w: number; h: number },
  margin = 0,
): boolean {
  if (size.w <= 0 || size.h <= 0 || view.zoom <= 0) return true;
  const left = view.x + rect.x * view.zoom;
  const top = view.y + rect.y * view.zoom;
  const right = left + rect.w * view.zoom;
  const bottom = top + rect.h * view.zoom;
  return right > -margin && bottom > -margin && left < size.w + margin && top < size.h + margin;
}

/**
 * Cor do nó no minimapa (canvas-10), pelo estado do anel: esperando o dono
 * âmbar, rodando verde, falhou vermelho; criado pelo orquestrador violeta;
 * na fila apagado; pronto verde mais fraco; fora de execução a borda.
 */
export function minimapTone(ring: RingState, created: boolean): string {
  switch (ring) {
    case "owner":
      return "var(--tyba-amber)";
    case "working":
    case "live":
      return "var(--tyba-green)";
    case "failed":
      return "var(--tyba-red)";
    default:
      break;
  }
  if (created) return "var(--tyba-violet)";
  if (ring === "done") return "color-mix(in srgb, var(--tyba-green) 45%, transparent)";
  if (ring === "waiting") return "var(--tyba-text-faint)";
  return "var(--tyba-border-strong)";
}
