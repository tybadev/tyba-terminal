// Manuseio do card no canvas (quadro OrbitManuseio): guias ao arrastar,
// quem pergunta antes de apagar e o aviso de nó apagado. Tudo puro: o canvas
// só aplica o que sai daqui.

import type { Rect } from "./graphLayout";
import { nodeSession, type NodeKind, type OrbitNode, type Run } from "./orbit";

/** Quão perto (px na tela) a borda ou o centro gruda no do outro card. */
export const SNAP_PX = 6;

/** Linha guia em coordenadas do grafo: vertical (`x`) ou horizontal (`y`). */
export interface Guide {
  axis: "x" | "y";
  /** Onde a linha está no eixo dela. */
  at: number;
  /** De onde até onde ela vai no outro eixo (cobre os dois cards). */
  from: number;
  to: number;
}

const lines = (start: number, size: number) => [start, start + size / 2, start + size];

/** O menor deslocamento (|d| ≤ limite) que alinha uma linha do card com uma do outro. */
function bestShift(mine: number[], theirs: number[][], threshold: number): number | null {
  let best: number | null = null;
  for (const others of theirs) {
    for (const o of others) {
      for (const m of mine) {
        const d = o - m;
        if (Math.abs(d) <= threshold && (best === null || Math.abs(d) < Math.abs(best))) best = d;
      }
    }
  }
  return best;
}

/**
 * O card arrastado gruda de leve quando a borda (esquerda, centro, direita;
 * topo, meio, base) chega a `threshold` da de outro card, e as guias violeta
 * mostram cada alinhamento, de um card ao outro. Os dois eixos são
 * independentes.
 */
export function snapToGuides(
  moving: Rect,
  others: readonly Rect[],
  threshold: number = SNAP_PX,
): { x: number; y: number; guides: Guide[] } {
  const dx = bestShift(
    lines(moving.x, moving.w),
    others.map((o) => lines(o.x, o.w)),
    threshold,
  );
  const dy = bestShift(
    lines(moving.y, moving.h),
    others.map((o) => lines(o.y, o.h)),
    threshold,
  );
  const x = moving.x + (dx ?? 0);
  const y = moving.y + (dy ?? 0);
  const guides: Guide[] = [];
  const near = (a: number, b: number) => Math.abs(a - b) < 0.5;
  for (const o of others) {
    if (dx !== null) {
      for (const at of lines(x, moving.w)) {
        if (lines(o.x, o.w).some((l) => near(l, at))) {
          guides.push({
            axis: "x",
            at,
            from: Math.min(y, o.y),
            to: Math.max(y + moving.h, o.y + o.h),
          });
        }
      }
    }
    if (dy !== null) {
      for (const at of lines(y, moving.h)) {
        if (lines(o.y, o.h).some((l) => near(l, at))) {
          guides.push({
            axis: "y",
            at,
            from: Math.min(x, o.x),
            to: Math.max(x + moving.w, o.x + o.w),
          });
        }
      }
    }
  }
  return { x, y, guides };
}

/** A cápsula pequena do redimensionar: "430 × 230". */
export const sizeLabel = (w: number, h: number) => `${Math.round(w)} × ${Math.round(h)}`;

/**
 * O que apagar faz com o nó no run à vista: `ask` quando ele está rodando
 * (agente, processo, serviço) ou tem sessão viva (o agente depois do turno) —
 * a cápsula pergunta e o core para antes; `stop` também para o que ainda
 * espera no run aberto, para não subir um processo sem card depois.
 */
export function deletePlan(
  kind: NodeKind,
  run: Pick<Run, "status" | "nodes" | "liveAgents"> | null,
  nodeId: string,
  live: boolean,
): { ask: boolean; stop: boolean } {
  const status = run?.nodes[nodeId]?.status;
  const ask = status === "running" || nodeSession(kind, run, nodeId, live) !== null;
  const stop = ask || (status === "waiting" && run?.status === "running");
  return { ask, stop };
}

/**
 * O aviso embaixo do canvas: um nó diz o tipo e o nome ("Nota “Achados”
 * apagada"); vários, só quantos.
 */
export function deletedNotice(
  nodes: readonly Pick<OrbitNode, "title" | "kind">[],
): { key: string; params: Record<string, string | number> } {
  if (nodes.length === 1) {
    return { key: `orbitDeleted_${nodes[0].kind.type}`, params: { title: nodes[0].title } };
  }
  return { key: "graphNodesDeleted", params: { count: nodes.length } };
}
