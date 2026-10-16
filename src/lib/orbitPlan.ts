// Nota de plano do Orquestrador (quadro OrbitPlano). Quem escreve é o core
// (`orbit_plan` → `plan_markdown`); aqui só se lê o checklist para desenhar
// e se põe na tela a cópia que o run manda.

import type { OrbitEdge, OrbitNode, Plan, StepState } from "./orbit";
import { NEW_IN_HANDLE, OUT_HANDLE } from "./orbit";

/** Um passo do checklist e o que ele criou (as linhas "  criou …" sob ele). */
export interface ChecklistStep {
  state: StepState;
  text: string;
  made: string[];
}

const MARK: Record<string, StepState> = {
  " ": "todo",
  "~": "doing",
  x: "done",
  X: "done",
  "!": "failed",
};

const STEP = /^- \[([ ~xX!])\] (.*\S.*)$/;
const MADE = /^ {2,}criou (.*\S.*)$/;

/**
 * O checklist da nota (`- [ ]` falta, `- [~]` fazendo, `- [x]` pronto, `- [!]`
 * falhou; `  criou …` recuado sob o passo), ou `null` se o texto não é só
 * isso — aí a nota mostra o texto como sempre.
 */
export function parseChecklist(markdown: string): ChecklistStep[] | null {
  const steps: ChecklistStep[] = [];
  for (const raw of markdown.split("\n")) {
    const line = raw.trimEnd();
    if (line === "") continue;
    const step = STEP.exec(line);
    if (step) {
      steps.push({ state: MARK[step[1]], text: step[2].trim(), made: [] });
      continue;
    }
    const made = MADE.exec(line);
    if (made && steps.length > 0) {
      steps[steps.length - 1].made.push(made[1].trim());
      continue;
    }
    return null;
  }
  return steps.length > 0 ? steps : null;
}

/** "3 de 5": os prontos e o total. */
export const checklistCount = (steps: readonly ChecklistStep[]) => ({
  done: steps.filter((s) => s.state === "done").length,
  total: steps.length,
});

/** A nota é um plano: recebe uma ligação `plan`. */
export const isPlanNote = (id: string, edges: readonly Pick<OrbitEdge, "to" | "kind">[]) =>
  edges.some((e) => e.to === id && e.kind === "plan");

/**
 * Os planos do run que a tela ainda não tem: o core grava a nota no grafo
 * salvo, mas a tela salva a cópia dela no debounce, e sem a nota (ou com o
 * texto velho) apagaria o que o core escreveu. Vale o plano escrito depois de
 * `since` (quando a tela leu o grafo) e mais novo que o já aplicado.
 */
export function freshPlans(
  plans: readonly Plan[],
  applied: ReadonlyMap<string, number>,
  since: number,
): Plan[] {
  return plans.filter((p) => p.updatedAt > Math.max(since, applied.get(p.note.id) ?? 0));
}

/**
 * Põe os planos na tela: nota que falta entra como o core a criou; a que já
 * está fica onde o dono a deixou e só troca o texto. A ligação entra se faltar
 * (e se o orquestrador ainda está na tela).
 */
export function applyPlans<
  N extends { id: string; data: { node: OrbitNode } },
  E extends { id: string },
>(
  nodes: readonly N[],
  edges: readonly E[],
  plans: readonly Plan[],
): { nodes: N[]; edges: E[] } {
  let outNodes = [...nodes];
  let outEdges = [...edges];
  for (const { note, edge } of plans) {
    const found = outNodes.find((n) => n.id === note.id);
    outNodes = found
      ? outNodes.map((n) =>
          n.id === note.id
            ? ({ ...n, data: { node: { ...n.data.node, kind: note.kind } } } as N)
            : n,
        )
      : [
          ...outNodes,
          {
            id: note.id,
            type: note.kind.type,
            position: { ...note.position },
            width: note.size.w,
            height: note.size.h,
            data: { node: note },
          } as unknown as N,
        ];
    const linked = outNodes.some((n) => n.id === edge.from);
    if (linked && !outEdges.some((e) => e.id === edge.id)) {
      outEdges = [
        ...outEdges,
        {
          id: edge.id,
          type: "orbit",
          source: edge.from,
          target: edge.to,
          sourceHandle: OUT_HANDLE,
          targetHandle: NEW_IN_HANDLE,
          data: { kind: edge.kind },
        } as unknown as E,
      ];
    }
  }
  return { nodes: outNodes, edges: outEdges };
}

/**
 * Plano que chega com a nota `editing` em edição (textarea com foco) espera:
 * trocar o texto ali apagaria o que o dono está digitando. Os outros entram.
 */
export function splitEditing(
  plans: readonly Plan[],
  editing: string | null,
): { now: Plan[]; later: Plan[] } {
  return {
    now: plans.filter((p) => p.note.id !== editing),
    later: plans.filter((p) => p.note.id === editing),
  };
}

/**
 * A edição acabou: o plano que esperou entra se o dono não mudou o texto
 * (`start` → `end`). Se mudou, o texto dele vale até o próximo `orbit_plan` —
 * o core devolve a edição ao agente para ele incorporar.
 */
export function planAfterEdit(pending: Plan | undefined, start: string, end: string): Plan | null {
  if (!pending || start !== end) return null;
  return pending;
}
