import { useEffect, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { List, LockSimple, LockSimpleOpen, NotePencil } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import type { NodeKind, StepState } from "@/lib/orbit";
import { checklistCount, isPlanNote, noteEditable, parseChecklist } from "@/lib/orbitPlan";
import { clockTime, freshWrite, memoryAgents, noteMemoryLabel } from "@/lib/orbitTalk";
import { NodeFrame, useCanvasActions, useOrbit, type OrbitFlowNode } from "./NodeFrame";

/** Ícone e cor de cada estado do passo (quadro OrbitPlano). */
const STEP_MARK: Record<StepState, { mark: string; tone: string }> = {
  done: { mark: "✓", tone: "text-tyba-green" },
  doing: { mark: "◐", tone: "text-tyba-blue" },
  todo: { mark: "○", tone: "text-tyba-text-faint" },
  failed: { mark: "✕", tone: "text-tyba-red" },
};

/**
 * A escrita de agente já posta em cada nota, e quando a tela abriu: o card
 * remontado (trocar de grafo, voltar à tela) não desfaz o que o dono editou
 * depois — mesma ideia do `freshPlans`.
 */
const appliedWrites = new Map<string, number>();
const OPENED_AT = Date.now();

/**
 * Nota em Markdown: vira arquivo para quem estiver ligado nela. Texto que é
 * checklist aparece como lista de passos; clicar volta ao markdown cru para
 * editar, e sair renderiza de novo. A nota de plano é do agente: só
 * leitura, mostra o checklist e nada mais. Ligada por memória, é nota viva:
 * os agentes escrevem, a cápsula diz quantos e tem o cadeado (só leitura),
 * e o rodapé diz quem escreveu por último.
 */
export function NoteNode({ id, data, selected }: NodeProps<OrbitFlowNode>) {
  const { t } = useTranslation();
  const { updateNode } = useCanvasActions();
  const { graph, run } = useOrbit();
  const node = data.node;
  const kind = node.kind as Extract<NodeKind, { type: "note" }>;
  const locked = kind.locked === true;
  const [editing, setEditing] = useState(false);
  const steps = parseChecklist(kind.markdown);
  const plan = isPlanNote(id, graph.edges);
  const editable = noteEditable(id, graph.edges, locked);
  const count = steps ? checklistCount(steps) : null;
  // Nota viva (quadro MaestroConversa): agentes ligados por memória leem e
  // escrevem; o cadeado deixa só leitura (o core recusa a escrita).
  const agents = memoryAgents(id, graph.edges);
  const memory = noteMemoryLabel(agents, locked, run?.status === "running");
  const written = run?.nodes[id]?.written ?? null;
  // Agente escreveu: o core já gravou o texto no grafo salvo; o canvas
  // acompanha, para o próximo salvar não o desfazer.
  useEffect(() => {
    if (!written || !freshWrite(id, written.at, appliedWrites, OPENED_AT)) return;
    appliedWrites.set(id, written.at);
    if (written.markdown === kind.markdown) return;
    const markdown = written.markdown;
    updateNode(id, (n) =>
      n.kind.type === "note" ? { ...n, kind: { ...n.kind, markdown } } : n,
    );
    // Só quando um agente escreve de novo.
  }, [written?.at]);
  const setLocked = (on: boolean) =>
    updateNode(id, (n) => (n.kind.type === "note" ? { ...n, kind: { ...n.kind, locked: on } } : n));

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={plan ? <List size={13} weight="bold" /> : <NotePencil size={13} weight="bold" />}
      markTone="text-tyba-violet"
      paper
      extra={
        <>
          {count && (
            <span className="shrink-0 text-[11.5px] text-tyba-violet">
              {t("orbitPlanCount", count)}
            </span>
          )}
          {memory && (
            <span className="min-w-0 shrink truncate text-[11.5px] text-tyba-violet">
              {t(memory.key, memory.params)}
            </span>
          )}
          {!plan && (agents > 0 || locked) && (
            <button
              type="button"
              aria-pressed={locked}
              aria-label={t("orbitNoteLock")}
              title={t(locked ? "orbitNoteLockedHint" : "orbitNoteUnlockedHint")}
              onClick={() => setLocked(!locked)}
              className={`nodrag flex shrink-0 items-center gap-1 rounded-full px-1.5 py-0.5 text-[11.5px] hover:bg-tyba-text/[.06] ${
                locked ? "text-tyba-violet" : "text-tyba-text-faint hover:text-tyba-text"
              }`}
            >
              {locked ? <LockSimple size={12} weight="bold" /> : <LockSimpleOpen size={12} />}
              {t(locked ? "orbitYesShort" : "orbitNoShort")}
            </button>
          )}
        </>
      }
      meta={
        plan
          ? t("orbitPlanFooter")
          : written
            ? t("orbitNoteWrittenBy", { name: written.by, time: clockTime(written.at) })
            : undefined
      }
      details={[
        { label: t("orbitDetailKind"), value: t("orbitKindNote") },
        { label: t("orbitOutput"), value: `${t("orbitEdge_context")}, ${t("orbitEdge_file")}` },
      ]}
    >
      {!editable || (steps && !editing) ? (
        <div
          {...(editable && {
            role: "button",
            tabIndex: 0,
            "aria-label": t("orbitNoteEdit"),
            title: t("orbitNoteEdit"),
            onClick: () => setEditing(true),
            onKeyDown: (e: KeyboardEvent) => e.key === "Enter" && setEditing(true),
          })}
          className={`nodrag nowheel min-h-0 flex-1 overflow-y-auto px-4 pt-1 pb-3 font-sans text-[13px] leading-[1.45] text-tyba-text${editable ? " cursor-text" : ""}`}
        >
          {!steps && <div className="whitespace-pre-wrap">{kind.markdown}</div>}
          {steps?.map((s, i) => (
            <div key={i}>
              <div className="flex items-start gap-2.5 py-1.5">
                <span className={`w-4 shrink-0 text-center ${STEP_MARK[s.state].tone}`}>
                  {STEP_MARK[s.state].mark}
                </span>
                <span
                  className={
                    s.state === "done"
                      ? "text-tyba-text-muted line-through decoration-tyba-text-faint"
                      : undefined
                  }
                >
                  {s.text}
                </span>
              </div>
              {s.made.map((m, j) => (
                <div
                  key={j}
                  className="-mt-1 pl-[26px] font-mono text-[11px] text-tyba-text-faint"
                >
                  {t("orbitPlanMade", { made: m })}
                </div>
              ))}
            </div>
          ))}
        </div>
      ) : (
        <textarea
          aria-label={t("orbitNoteBody")}
          placeholder={t("orbitNoteBody")}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          autoFocus={editing}
          value={kind.markdown}
          onChange={(e) => {
            const markdown = e.target.value;
            updateNode(id, (n) =>
              n.kind.type === "note" ? { ...n, kind: { ...n.kind, markdown } } : n,
            );
          }}
          onBlur={() => setEditing(false)}
          className="nodrag nowheel min-h-0 w-full flex-1 resize-none bg-transparent px-4 pt-1 pb-3 font-sans text-[13px] leading-[1.6] text-tyba-text outline-none placeholder:text-tyba-text-ghost"
        />
      )}
    </NodeFrame>
  );
}
