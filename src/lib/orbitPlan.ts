// Nota de plano do Orquestrador (quadro OrbitPlano). Quem escreve é o core
// (`orbit_plan` → `plan_markdown`); aqui só se lê o checklist para desenhar
// e se põe na tela a cópia que o run manda.

import type { OrbitEdge, OrbitNode, Plan, Run, StepState } from "./orbit";
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
 * Nota → "#N" da execução mais nova que escreveu nela (ou a zerou no ▶).
 * `runs` vem do mais novo ao mais velho, como no `RunView`.
 */
export function planWriters(
  runs: readonly Pick<Run, "id" | "plans">[],
  numberOf: (runId: string) => number,
): Map<string, number> {
  const out = new Map<string, number>();
  for (const r of runs) {
    for (const p of r.plans ?? []) {
      if (!out.has(p.note.id)) out.set(p.note.id, numberOf(r.id));
    }
  }
  return out;
}

/** O que a nota de plano mostra (quadro OrbitPlano, "Plano · execução #N"). */
export interface PlanShown {
  /** O texto: o da execução à vista, ou o que ficou no grafo. */
  markdown: string;
  /** "#N" da execução de quem é o plano; `null` sem saber. */
  number: number | null;
  /** Apagada, "da execução anterior": a execução dele acabou ou não é a à vista. */
  previous: boolean;
  /** Execução rodando e o agente ainda não escreveu. */
  waiting: boolean;
  /** A execução acabou sem o agente escrever. */
  none: boolean;
}

/**
 * A nota de plano à vista. Com o run à vista tendo plano para ela (o ▶ já
 * deixa um pendente, sem passos), vale o texto do run — é assim que
 * Execuções → #N mostra o plano daquela execução; atual só com ele rodando
 * (assentado conta: o agente segue vivo e marca). Sem, vale o texto do
 * grafo, apagado, com o número de quem o escreveu por último. Nota criada
 * por agente do run é dele.
 */
export function planShown(
  noteId: string,
  markdown: string,
  view: {
    run: Pick<Run, "status" | "plans"> | null;
    number: number;
    live: boolean;
    writer: number | undefined;
    created: boolean;
  },
): PlanShown {
  const { run } = view;
  const own = run?.plans?.find((p) => p.note.id === noteId);
  if (run && (own || view.created)) {
    const text = own?.note.kind.type === "note" ? own.note.kind.markdown : markdown;
    const current = view.live && run.status === "running";
    const empty = text.trim() === "";
    return {
      markdown: text,
      number: view.number || null,
      previous: !current,
      waiting: empty && current,
      none: empty && !current,
    };
  }
  return {
    markdown,
    number: view.writer ?? null,
    previous: view.writer !== undefined,
    waiting: false,
    none: false,
  };
}

/**
 * A nota de plano é do agente (quem escreve e marca): só leitura na tela. O
 * cadeado não conta aqui: ele tranca só os agentes, o dono segue editando.
 */
export function noteEditable(id: string, edges: readonly Pick<OrbitEdge, "to" | "kind">[]): boolean {
  return !isPlanNote(id, edges);
}
