// ⌘D, ⌘C e ⌘V do canvas (canvas-9): copiar nós do grafo com id novo. Puro.

import type { OrbitEdge, OrbitNode } from "./orbit";

export interface Clip {
  nodes: OrbitNode[];
  /** Só as ligações entre os copiados: a cópia não pega entrada de fora. */
  edges: OrbitEdge[];
}

/** O que ⌘C guarda: os nós escolhidos e as ligações entre eles. */
export function copyNodes(
  nodes: readonly OrbitNode[],
  edges: readonly OrbitEdge[],
  ids: ReadonlySet<string>,
): Clip {
  return {
    nodes: nodes.filter((n) => ids.has(n.id)).map((n) => structuredClone(n)),
    edges: edges.filter((e) => ids.has(e.from) && ids.has(e.to)).map((e) => ({ ...e })),
  };
}

/** O retângulo que envolve os nós (para achar onde a cópia cabe). */
export function clipBounds(clip: Clip): { x: number; y: number; w: number; h: number } | null {
  if (clip.nodes.length === 0) return null;
  const x = Math.min(...clip.nodes.map((n) => n.position.x));
  const y = Math.min(...clip.nodes.map((n) => n.position.y));
  const right = Math.max(...clip.nodes.map((n) => n.position.x + n.size.w));
  const bottom = Math.max(...clip.nodes.map((n) => n.position.y + n.size.h));
  return { x, y, w: right - x, h: bottom - y };
}

/**
 * A cópia colada com o canto em `at`: ids novos (de `newId`), a mesma forma
 * entre os nós e as ligações internas apontando para os novos.
 */
export function pasteClip(clip: Clip, at: { x: number; y: number }, newId: () => string): Clip {
  const box = clipBounds(clip);
  if (!box) return { nodes: [], edges: [] };
  const ids = new Map(clip.nodes.map((n) => [n.id, newId()]));
  return {
    nodes: clip.nodes.map((n) => ({
      ...structuredClone(n),
      id: ids.get(n.id)!,
      position: {
        x: Math.round(at.x + n.position.x - box.x),
        y: Math.round(at.y + n.position.y - box.y),
      },
    })),
    edges: clip.edges.map((e) => ({
      ...e,
      id: newId(),
      from: ids.get(e.from)!,
      to: ids.get(e.to)!,
    })),
  };
}
