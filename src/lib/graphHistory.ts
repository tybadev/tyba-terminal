// Desfazer/refazer do canvas (canvas-5): uma pilha de retratos do grafo.
// O retrato é o grafo inteiro (nós com posição e tamanho, ligações) em JSON,
// o mesmo `fromFlow` que vai ao core — voltar a um retrato é trocar o canvas
// por ele, e o salvar com debounce leva a volta ao banco como qualquer edição.

import { fromFlow, toFlow, type FlowEdgeLike, type FlowNode, type Graph } from "./maestro";

/** Quantos passos o ⌘Z alcança. */
export const HISTORY_LIMIT = 100;

export interface History {
  /** Retratos anteriores, o mais novo no fim. */
  past: readonly string[];
  /** Retratos desfeitos, o próximo do ⇧⌘Z no fim. */
  future: readonly string[];
}

export const EMPTY_HISTORY: History = { past: [], future: [] };

/** Uma edição terminou: `previous` (o retrato de antes) entra na pilha e o refazer se perde. */
export function record(h: History, previous: string): History {
  if (h.past[h.past.length - 1] === previous) return { past: h.past, future: [] };
  const past = [...h.past, previous];
  return { past: past.slice(Math.max(0, past.length - HISTORY_LIMIT)), future: [] };
}

/** ⌘Z: volta ao retrato anterior; `current` vai para o refazer. `null` = nada a desfazer. */
export function undo(h: History, current: string): { history: History; shot: string } | null {
  const shot = h.past[h.past.length - 1];
  if (shot === undefined) return null;
  return { history: { past: h.past.slice(0, -1), future: [...h.future, current] }, shot };
}

/** ⇧⌘Z: refaz o último desfeito. `null` = nada a refazer. */
export function redo(h: History, current: string): { history: History; shot: string } | null {
  const shot = h.future[h.future.length - 1];
  if (shot === undefined) return null;
  return { history: { past: [...h.past, current], future: h.future.slice(0, -1) }, shot };
}

const NO_META = { id: "", name: "", dir: "", variables: {} };

/** O retrato do canvas: só nós e ligações (nome e variáveis têm o próprio campo). */
export function graphShot(
  nodes: readonly Pick<FlowNode, "id" | "position" | "width" | "height" | "data">[],
  edges: readonly FlowEdgeLike[],
): string {
  const g = fromFlow(NO_META, nodes, edges);
  return JSON.stringify({ nodes: g.nodes, edges: g.edges });
}

/** O retrato de volta em nós e ligações do canvas. */
export function restoreShot(shot: string): ReturnType<typeof toFlow> {
  const { nodes, edges } = JSON.parse(shot) as Pick<Graph, "nodes" | "edges">;
  return toFlow({ ...NO_META, nodes, edges });
}

/** Ids dos nós que saíram de `before` para `after` (o aviso "Nó apagado · Desfazer"). */
export function removedNodes(before: string, after: string): string[] {
  const ids = (shot: string) => (JSON.parse(shot) as Pick<Graph, "nodes">).nodes.map((n) => n.id);
  const kept = new Set(ids(after));
  return ids(before).filter((id) => !kept.has(id));
}
