// Desfazer/refazer do canvas (canvas-5): uma pilha de retratos do grafo.
// O retrato é o grafo inteiro (nós com posição e tamanho, ligações) em JSON,
// o mesmo `fromFlow` que vai ao core — voltar a um retrato é trocar o canvas
// por ele, e o salvar com debounce leva a volta ao banco como qualquer edição.

import { fromFlow, toFlow, type FlowEdgeLike, type FlowNode, type Graph } from "./orbit";

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

/**
 * "Manter no grafo" não é passo do ⌘Z. Quem decide é o core: ele grava os
 * criados no grafo e marca `kept` no run, e daí o canvas deixa de desenhá-los
 * como efêmeros. Um retrato de antes do Manter, sem eles, os tiraria do grafo
 * salvo com o run ainda dizendo "mantido" — o nó sumiria do canvas com o
 * processo vivo. Então os nós `ids` (com as ligações deles, como estão em
 * `shot`) passam a existir em todo retrato da pilha: desfazer e refazer
 * mexem só no que o dono fez. Ligação só entra onde as duas pontas existem.
 */
export function keepInHistory(h: History, shot: string, ids: ReadonlySet<string>): History {
  if (ids.size === 0) return h;
  const kept = JSON.parse(shot) as Pick<Graph, "nodes" | "edges">;
  const nodes = kept.nodes.filter((n) => ids.has(n.id));
  const edges = kept.edges.filter((e) => ids.has(e.from) || ids.has(e.to));
  const adopt = (s: string): string => {
    const g = JSON.parse(s) as Pick<Graph, "nodes" | "edges">;
    const have = new Set(g.nodes.map((n) => n.id));
    const addNodes = nodes.filter((n) => !have.has(n.id));
    for (const n of addNodes) have.add(n.id);
    const haveEdges = new Set(g.edges.map((e) => e.id));
    const addEdges = edges.filter(
      (e) => !haveEdges.has(e.id) && have.has(e.from) && have.has(e.to),
    );
    if (addNodes.length === 0 && addEdges.length === 0) return s;
    return JSON.stringify({ nodes: [...g.nodes, ...addNodes], edges: [...g.edges, ...addEdges] });
  };
  return { past: h.past.map(adopt), future: h.future.map(adopt) };
}

/**
 * A nota de plano que o core escreve (`orbit_plan`) também não é passo do
 * ⌘Z: um retrato de antes dela, posto de volta, apagaria a nota (ou voltaria
 * o texto velho) e o salvar levaria isso ao core. Como no `keepInHistory`, as
 * notas `ids` e as ligações delas, como estão em `shot`, entram em todo
 * retrato da pilha; nos que já têm a nota, só o texto troca (a posição é a
 * que o dono deixou naquele passo).
 */
export function plansInHistory(h: History, shot: string, ids: ReadonlySet<string>): History {
  if (ids.size === 0) return h;
  const plans = JSON.parse(shot) as Pick<Graph, "nodes" | "edges">;
  const kinds = new Map(plans.nodes.filter((n) => ids.has(n.id)).map((n) => [n.id, n.kind]));
  const withText = (s: string): string => {
    const g = JSON.parse(s) as Pick<Graph, "nodes" | "edges">;
    let changed = false;
    const nodes = g.nodes.map((n) => {
      const kind = kinds.get(n.id);
      if (!kind || JSON.stringify(kind) === JSON.stringify(n.kind)) return n;
      changed = true;
      return { ...n, kind };
    });
    return changed ? JSON.stringify({ nodes, edges: g.edges }) : s;
  };
  const adopted = keepInHistory(h, shot, ids);
  return { past: adopted.past.map(withText), future: adopted.future.map(withText) };
}
