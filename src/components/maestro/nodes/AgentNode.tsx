import { useTranslation } from "react-i18next";
import { CircleHalf, Robot } from "@phosphor-icons/react";
import { Handle, Position, type NodeProps } from "@xyflow/react";

import type { NodeKind, Runner } from "@/lib/maestro";
import { SUBAGENT_HANDLE, groupRounds, roundTally } from "@/lib/maestroSubagents";
import { NodeFrame, fieldClass, termClass, useMaestro, type MaestroFlowNode } from "./NodeFrame";
import { NodeTerminal, nodeSession } from "./NodeTerminal";

type AgentKind = Extract<NodeKind, { type: "agent" }>;

const RUNNERS: Runner[] = ["claude", "codex"];

/** Nó de agente: Claude (âmbar) ou Codex (ciano), papel e a tarefa. */
export function AgentNode({ id, data, selected }: NodeProps<MaestroFlowNode>) {
  const { t } = useTranslation();
  const { updateNode, run, live, subagents } = useMaestro();
  const node = data.node;
  const kind = node.kind as AgentKind;
  const session = nodeSession(kind, run?.nodes[id], live);
  const rounds = session ? groupRounds(subagents.get(id) ?? []) : null;
  const tally = rounds ? roundTally(rounds.current) : null;
  const set = (patch: Partial<AgentKind>) =>
    updateNode(id, (n) => ({ ...n, kind: { ...(n.kind as AgentKind), ...patch } }));
  const claude = kind.runner === "claude";

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<Robot size={14} weight="bold" />}
      markClass={claude ? "bg-tyba-amber-tint text-tyba-amber" : "bg-tyba-cyan-tint text-tyba-cyan"}
      label={kind.role || t("maestroKindAgent")}
      status={
        rounds && tally && tally.running > 0 ? (
          <span className="flex shrink-0 items-center gap-1.5 text-[11.5px] whitespace-nowrap text-tyba-blue">
            <CircleHalf size={12} weight="fill" className="tyba-half-flip" />
            {t("maestroSubWorking", { count: rounds.current.length })}
          </span>
        ) : undefined
      }
    >
      {/* Saída invisível das arestas dos subagentes (embaixo, no meio). */}
      <Handle
        type="source"
        id={SUBAGENT_HANDLE}
        position={Position.Bottom}
        isConnectable={false}
        style={{ opacity: 0, width: 1, height: 1, minWidth: 0, minHeight: 0, border: 0 }}
      />
      <div className="flex shrink-0 items-center gap-1.5 border-b border-tyba-border px-3.5 py-1.5">
        <div className="nodrag flex rounded-[5px] border border-tyba-border p-px" role="group">
          {RUNNERS.map((r) => (
            <button
              key={r}
              type="button"
              aria-pressed={kind.runner === r}
              onClick={() => set({ runner: r })}
              className={`rounded-[4px] px-2 text-[11px] leading-[20px] ${
                kind.runner === r ? "bg-tyba-raised text-tyba-text" : "text-tyba-text-faint"
              }`}
            >
              {r === "claude" ? "Claude" : "Codex"}
            </button>
          ))}
        </div>
        <input
          aria-label={t("maestroRole")}
          placeholder={t("maestroRole")}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          value={kind.role}
          onChange={(e) => set({ role: e.target.value })}
          className={`${fieldClass} flex-1`}
        />
      </div>
      {session ? (
        <NodeTerminal sessionId={session} focused={selected} />
      ) : (
        <textarea
          aria-label={t("maestroPrompt")}
          placeholder={t("maestroPrompt")}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          value={kind.prompt}
          onChange={(e) => set({ prompt: e.target.value })}
          className={termClass}
        />
      )}
      {rounds && tally && (
        <div className="shrink-0 truncate border-t border-tyba-border px-3.5 py-1.5 text-[11.5px] text-tyba-text-faint">
          {t("maestroSubRound", { round: rounds.round, count: rounds.current.length })}
          {" — "}
          {[
            tally.running > 0 && t("maestroSubTallyRunning", { count: tally.running }),
            tally.done > 0 && t("maestroSubTallyDone", { count: tally.done }),
            tally.stopped > 0 && t("maestroSubTallyStopped", { count: tally.stopped }),
          ]
            .filter(Boolean)
            .join(", ")}
        </div>
      )}
    </NodeFrame>
  );
}
