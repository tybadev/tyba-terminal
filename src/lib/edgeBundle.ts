// Várias ligações do mesmo par (A → B) desenhadas como uma só (sem DOM, testável).
//
// Com a porta única (canvas-3) toda ligação sai da mesma bolinha e chega na
// mesma entrada: o xyflow calcula a curva só pelo nó e pela porta
// (`getEdgePosition` em @xyflow/system), então A→B de contexto e A→B de diff
// caem uma em cima da outra, etiquetas inclusive. Grafo salvo antes da porta
// única pode ter isso. No canvas elas viram UMA ligação com a etiqueta
// "contexto + diff"; no modelo continuam todas, cada uma com o seu id.

import type { EdgeKind, FlowEdgeData } from "./orbit";

/** Uma ligação do modelo dentro da desenhada. */
export interface BundleMember {
  id: string;
  kind: EdgeKind;
}

/** O `data` da ligação desenhada: o tipo da primeira e, se juntou, todas. */
export type BundledEdgeData = FlowEdgeData & { bundle?: BundleMember[] };

interface EdgeLike {
  id: string;
  source: string;
  target: string;
  selected?: boolean;
  data?: FlowEdgeData;
}

const BUNDLE_PREFIX = "bundle:";

/**
 * As ligações que o canvas desenha: uma por par com tipo (as sem `data` —
 * subagente, criador → criado — passam como estão). Par com uma ligação só
 * sai igual; sem nenhum par repetido, a mesma lista. `members` diz, para cada
 * ligação juntada, os ids do modelo que ela leva.
 */
export function bundleEdges<E extends EdgeLike>(
  edges: readonly E[],
): { edges: readonly (E & { data?: BundledEdgeData })[]; members: Map<string, string[]> } {
  const groups = new Map<string, E[]>();
  for (const e of edges) {
    if (!e.data) continue;
    const key = `${e.source}\u0000${e.target}`;
    const g = groups.get(key);
    if (g) g.push(e);
    else groups.set(key, [e]);
  }
  const members = new Map<string, string[]>();
  if (![...groups.values()].some((g) => g.length > 1)) return { edges, members };
  const out: (E & { data?: BundledEdgeData })[] = [];
  for (const e of edges) {
    const g = e.data ? groups.get(`${e.source}\u0000${e.target}`) : undefined;
    if (!g || g.length === 1) {
      out.push(e);
      continue;
    }
    if (g[0] !== e) continue;
    const id = BUNDLE_PREFIX + e.id;
    members.set(
      id,
      g.map((m) => m.id),
    );
    out.push({
      ...e,
      id,
      selected: g.some((m) => m.selected),
      data: { kind: e.data!.kind, bundle: g.map((m) => ({ id: m.id, kind: m.data!.kind })) },
    });
  }
  return { edges: out, members };
}

/**
 * Mudança do xyflow na ligação desenhada → mudança em cada ligação do modelo
 * que ela leva: apagar a juntada apaga todas, selecionar seleciona todas.
 */
export function unbundleEdgeChanges<C extends { type: string }>(
  changes: readonly C[],
  members: ReadonlyMap<string, readonly string[]>,
): C[] {
  if (members.size === 0) return [...changes];
  return changes.flatMap((c) => {
    const id = "id" in c ? (c.id as string) : null;
    const ids = id === null ? undefined : members.get(id);
    if (!ids) return [c];
    if (c.type !== "remove" && c.type !== "select") return [];
    return ids.map((m) => ({ ...c, id: m }));
  });
}
