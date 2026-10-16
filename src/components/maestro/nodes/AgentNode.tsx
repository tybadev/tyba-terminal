import { useTranslation } from "react-i18next";
import { CircleHalf } from "@phosphor-icons/react";
import { Handle, Position, type NodeProps } from "@xyflow/react";

import { ClaudeIcon } from "@/components/icons/ClaudeIcon";
import { OpenAIIcon } from "@/components/icons/OpenAIIcon";
import { ORCHESTRATOR_HANDLE, type NodeKind, type Runner } from "@/lib/maestro";
import { SUBAGENT_HANDLE, groupRounds, roundTally } from "@/lib/maestroSubagents";
import {
  NodeDir,
  NodeFrame,
  fieldClass,
  termClass,
  useMaestro,
  type MaestroFlowNode,
} from "./NodeFrame";
import { FocusButton, NodeTerminal, nodeSession } from "./NodeTerminal";

type AgentKind = Extract<NodeKind, { type: "agent" }>;

const RUNNERS: Runner[] = ["claude", "codex"];

/** Nó de agente: Claude (âmbar) ou Codex (ciano), papel, pasta e a tarefa. */
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
  const orchestrates = kind.orchestrate === true;
  const hidden = { opacity: 0, width: 1, height: 1, minWidth: 0, minHeight: 0, border: 0 };

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={claude ? <ClaudeIcon size={14} /> : <OpenAIIcon size={14} />}
      markClass={claude ? "bg-tyba-amber-tint text-tyba-amber" : "bg-tyba-cyan-tint text-tyba-cyan"}
      label={kind.role || t("maestroKindAgent")}
      extra={
        orchestrates || session ? (
          <>
            {orchestrates && (
              <span className="shrink-0 rounded-[4px] border border-tyba-violet/50 px-1.5 text-[11px] leading-[18px] text-tyba-violet">
                {t("maestroOrchestrates")}
              </span>
            )}
            {session && <FocusButton nodeId={id} />}
          </>
        ) : undefined
      }
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
        style={hidden}
      />
      {/* Saída invisível das ligações tracejadas até os nós que ele criou. */}
      {orchestrates && (
        <Handle
          type="source"
          id={ORCHESTRATOR_HANDLE}
          position={Position.Right}
          isConnectable={false}
          style={hidden}
        />
      )}
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
          className={`${fieldClass} min-w-0 flex-1`}
        />
        <button
          type="button"
          aria-pressed={orchestrates}
          title={t("maestroOrchestrateHint")}
          onClick={() => set({ orchestrate: !orchestrates })}
          className={`nodrag shrink-0 rounded-[4px] border px-1.5 text-[11px] leading-[20px] ${
            orchestrates
              ? "border-tyba-violet/50 bg-tyba-violet-tint text-tyba-violet"
              : "border-tyba-border text-tyba-text-faint hover:text-tyba-text"
          }`}
        >
          {t("maestroOrchestrate")}
        </button>
        <NodeDir id={id} kind={kind} />
      </div>
      {session ? (
        <NodeTerminal nodeId={id} sessionId={session} focused={selected} />
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
