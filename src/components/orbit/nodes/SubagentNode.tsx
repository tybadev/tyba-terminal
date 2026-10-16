import { useTranslation } from "react-i18next";
import { CaretRight, Check, CircleHalf, X } from "@phosphor-icons/react";
import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";

import type { SubagentRun } from "@/lib/ipc";
import {
  formatSubagentTime,
  subagentElapsed,
  subagentLine,
  subagentState,
  type SubagentState,
} from "@/lib/orbitSubagents";
import { useClock, useOrbit } from "./NodeFrame";

/** Nó-filho efêmero: nunca entra no grafo salvo. */
export type SubagentFlowNode = Node<{ run: SubagentRun }, "subagent">;
export type SubagentRoundsFlowNode = Node<{ count: number }, "subagentRounds">;

const TONE: Record<SubagentState, string> = {
  running: "text-tyba-blue",
  done: "text-tyba-green",
  stopped: "text-tyba-red",
};

/** Marca do estado: ◐ rodando (gira de lado), ✓ pronto, ✕ parou. */
export function SubagentMark({ state, size = 12 }: { state: SubagentState; size?: number }) {
  if (state === "running")
    return <CircleHalf size={size} weight="fill" className="tyba-half-flip shrink-0" />;
  if (state === "done") return <Check size={size} weight="bold" className="shrink-0" />;
  return <X size={size} weight="bold" className="shrink-0" />;
}

export const subagentTone = (state: SubagentState) => TONE[state];

/** Tempo do subagente no formato do quadro ("12s" rodando, "10,4s" pronto). */
export function useSubagentTime(run: SubagentRun): string {
  const { i18n } = useTranslation();
  const now = useClock();
  return formatSubagentTime(
    subagentElapsed(run, Math.max(now, run.started_at_ms)),
    run.ended_at_ms !== null,
    i18n.language,
  );
}

/**
 * Subagente em pílula (quadro OrbitCardsD): estado, o pedido e o tempo. O
 * resto (tipo, atividade, transcript) está no painel que o clique abre.
 */
export function SubagentNode({ id, data }: NodeProps<SubagentFlowNode>) {
  const { t } = useTranslation();
  const { openSubagent, setOpenSubagent } = useOrbit();
  const { run } = data;
  const state = subagentState(run);
  const line = subagentLine(run);
  const time = useSubagentTime(run);
  const open = openSubagent === id;
  const description = run.description || t("orbitSubNoDescription");

  // A pílula arrasta (nos-8: o dono tira de cima de outro nó) e o clique sem
  // mover abre o painel; não se seleciona. `pointer-events-auto` fica para a
  // rodada anterior e qualquer efêmero sem arrasto, onde o xyflow põe `none`.
  return (
    <button
      type="button"
      onClick={() => setOpenSubagent(open ? null : id)}
      aria-pressed={open}
      title={[description, run.agent_type, line?.kind === "activity" ? line.text : ""]
        .filter(Boolean)
        .join(" · ")}
      className={`nopan pointer-events-auto flex h-full w-full cursor-pointer items-center gap-2 rounded-full border bg-[color-mix(in_srgb,var(--tyba-text)_5%,var(--tyba-surface))] px-3 text-left font-mono text-[12px] ${
        open
          ? "border-tyba-violet shadow-[0_0_0_4px_color-mix(in_srgb,var(--tyba-violet)_15%,transparent)]"
          : "border-[color-mix(in_srgb,var(--tyba-text)_10%,transparent)] hover:border-tyba-text-faint"
      }`}
    >
      <Handle
        type="target"
        position={Position.Top}
        isConnectable={false}
        style={{ opacity: 0, width: 1, height: 1, minWidth: 0, minHeight: 0, border: 0 }}
      />
      <span className={`flex ${TONE[state]}`} aria-label={t(`orbitSub_${state}`)}>
        <SubagentMark state={state} />
      </span>
      <span className="min-w-0 flex-1 truncate text-tyba-text">{description}</span>
      <span className="shrink-0 text-tyba-text-faint">{time}</span>
    </button>
  );
}

/** "▸ rodada anterior · N subagentes, recolhida". */
export function SubagentRoundsNode({ data }: NodeProps<SubagentRoundsFlowNode>) {
  const { t } = useTranslation();
  return (
    <span className="flex h-full items-center gap-1.5 font-mono text-[12px] whitespace-nowrap text-tyba-text-faint">
      <CaretRight size={11} weight="bold" />
      {t("orbitSubPrevious", { count: data.count })}
    </span>
  );
}
