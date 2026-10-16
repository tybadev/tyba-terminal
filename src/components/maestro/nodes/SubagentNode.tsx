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
} from "@/lib/maestroSubagents";
import { useClock, useMaestro } from "./NodeFrame";

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

/** Cartão do subagente (quadro MaestroSubagentes): estado, tempo, pedido, tipo, atividade. */
export function SubagentNode({ id, data }: NodeProps<SubagentFlowNode>) {
  const { t } = useTranslation();
  const { openSubagent, setOpenSubagent } = useMaestro();
  const { run } = data;
  const state = subagentState(run);
  const line = subagentLine(run);
  const time = useSubagentTime(run);
  const open = openSubagent === id;

  // O nó efêmero não é selecionável nem arrastável, e o xyflow põe
  // `pointer-events: none` nele: o clique só chega com o cartão pedindo de
  // volta (`pointer-events-auto`), fora do arrastar/mover do canvas.
  return (
    <button
      type="button"
      onClick={() => setOpenSubagent(open ? null : id)}
      aria-pressed={open}
      className={`nodrag nopan pointer-events-auto flex h-full w-full cursor-pointer flex-col gap-1.5 rounded-[8px] border px-3 py-2.5 text-left font-mono text-[12px] ${
        open
          ? "border-tyba-violet bg-tyba-raised shadow-[0_0_0_4px_color-mix(in_srgb,var(--tyba-violet)_15%,transparent)]"
          : "border-tyba-border-strong bg-tyba-surface hover:border-tyba-text-faint"
      }`}
    >
      <Handle
        type="target"
        position={Position.Top}
        isConnectable={false}
        style={{ opacity: 0, width: 1, height: 1, minWidth: 0, minHeight: 0, border: 0 }}
      />
      <span className="flex w-full items-center justify-between gap-2">
        <span className={`flex items-center gap-1.5 ${TONE[state]}`}>
          <SubagentMark state={state} />
          {t(`maestroSub_${state}`)}
        </span>
        <span className="text-tyba-text-faint">{time}</span>
      </span>
      <span className="w-full truncate text-[13px] text-tyba-text">
        {run.description || t("maestroSubNoDescription")}
      </span>
      <span className="w-full truncate text-tyba-text-faint">{run.agent_type}</span>
      {line?.kind === "activity" && (
        <span className="w-full truncate text-tyba-text-muted">{line.text}</span>
      )}
      {line?.kind === "summary" && (
        <span className="line-clamp-2 w-full leading-[1.5] text-tyba-text-muted">{line.text}</span>
      )}
      {line?.kind === "interrupted" && (
        <span className="w-full text-tyba-text-muted">{t("maestroSubInterrupted")}</span>
      )}
      {line?.kind === "finished" && (
        <span className="w-full text-tyba-text-muted">{t("maestroSubFinished")}</span>
      )}
    </button>
  );
}

/** "▸ rodada anterior · N subagentes, recolhida". */
export function SubagentRoundsNode({ data }: NodeProps<SubagentRoundsFlowNode>) {
  const { t } = useTranslation();
  return (
    <span className="flex h-full items-center gap-1.5 font-mono text-[12px] whitespace-nowrap text-tyba-text-faint">
      <CaretRight size={11} weight="bold" />
      {t("maestroSubPrevious", { count: data.count })}
    </span>
  );
}
