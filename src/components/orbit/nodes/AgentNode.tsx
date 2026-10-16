import { useTranslation } from "react-i18next";
import { CircleHalf } from "@phosphor-icons/react";
import { Handle, Position, type NodeProps } from "@xyflow/react";

import { ClaudeIcon } from "@/components/icons/ClaudeIcon";
import { OpenAIIcon } from "@/components/icons/OpenAIIcon";
import {
  ORCHESTRATOR_HANDLE,
  nodeDir,
  nodeSession,
  shortDir,
  type NodeKind,
  type Runner,
} from "@/lib/orbit";
import { SUBAGENT_HANDLE, groupRounds, roundTally } from "@/lib/orbitSubagents";
import { agentDetailRows, diffSummary } from "@/lib/orbitCard";
import { firstLine } from "@/lib/nodeSummary";
import {
  Elapsed,
  NodeDir,
  NodeFrame,
  fieldClass,
  termClass,
  useOrbit,
  type OrbitFlowNode,
} from "./NodeFrame";
import { NodeTerminal } from "./NodeTerminal";

type AgentKind = Extract<NodeKind, { type: "agent" }>;

const RUNNERS: Runner[] = ["claude", "codex"];

/** Nó de agente: Claude (âmbar) ou Codex (ciano), papel, pasta e a tarefa. */
export function AgentNode({ id, data, selected }: NodeProps<OrbitFlowNode>) {
  const { t } = useTranslation();
  const { updateNode, run, live, subagents, createdBy, graphDir, telemetry } = useOrbit();
  const node = data.node;
  const kind = node.kind as AgentKind;
  const session = nodeSession(kind, run, id, live);
  const rounds = session ? groupRounds(subagents.get(id) ?? []) : null;
  const tally = rounds ? roundTally(rounds.current) : null;
  const set = (patch: Partial<AgentKind>) =>
    updateNode(id, (n) => ({ ...n, kind: { ...(n.kind as AgentKind), ...patch } }));
  const claude = kind.runner === "claude";
  // Agente criado pelo orquestrador nunca orquestra: sem toggle e sem selo.
  const created = createdBy.has(id);
  const orchestrates = !created && kind.orchestrate === true;
  const hidden = { opacity: 0, width: 1, height: 1, minWidth: 0, minHeight: 0, border: 0 };
  const dir = nodeDir(kind, graphDir);
  const subTally =
    rounds && tally
      ? [
          tally.running > 0 && t("orbitSubTallyRunning", { count: tally.running }),
          tally.done > 0 && t("orbitSubTallyDone", { count: tally.done }),
          tally.stopped > 0 && t("orbitSubTallyStopped", { count: tally.stopped }),
        ]
          .filter(Boolean)
          .join(", ")
      : "";
  const usage = telemetry.get(id);
  // Quadro: "muriki-web · 2 subagentes · +42 −7".
  const meta = [dir ? shortDir(dir) : "", subTally, diffSummary(usage)].filter(Boolean).join(" · ");
  const nodeRun = run?.nodes[id];

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={claude ? <ClaudeIcon size={13} /> : <OpenAIIcon size={13} />}
      markTone={claude ? "text-tyba-amber" : "text-tyba-cyan"}
      meta={session ? meta : undefined}
      // Grade do quadro OrbitCardsD (o tempo entra na ordem dela).
      stateRows={false}
      details={agentDetailRows({
        runner: kind.runner,
        role: kind.role,
        dir: dir ? shortDir(dir) : "",
        task: firstLine(kind.prompt),
        telemetry: usage,
        orchestrate: created ? null : orchestrates,
      }).map((row) => ({
        label: t(row.key),
        value:
          row.value === null ? (
            nodeRun && <Elapsed nodeRun={nodeRun} />
          ) : row.key === "orbitOrchestrate" ? (
            t(row.value)
          ) : (
            row.value
          ),
      }))}
      status={
        rounds && tally && tally.running > 0 ? (
          <span className="flex shrink-0 items-center gap-1.5 text-[11.5px] whitespace-nowrap text-tyba-blue">
            <CircleHalf size={12} weight="fill" className="tyba-half-flip" />
            {t("orbitSubWorking", { count: rounds.current.length })}
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
      {!session && (
        <div className="flex shrink-0 items-center gap-1.5 border-b border-tyba-border px-3.5 pb-1.5">
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
            aria-label={t("orbitRole")}
            placeholder={t("orbitRole")}
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            value={kind.role}
            onChange={(e) => set({ role: e.target.value })}
            className={`${fieldClass} min-w-0 flex-1`}
          />
          {!created && (
            <button
              type="button"
              aria-pressed={orchestrates}
              title={t("orbitOrchestrateHint")}
              onClick={() => set({ orchestrate: !orchestrates })}
              className={`nodrag shrink-0 rounded-[4px] border px-1.5 text-[11px] leading-[20px] ${
                orchestrates
                  ? "border-tyba-violet/50 bg-tyba-violet-tint text-tyba-violet"
                  : "border-tyba-border text-tyba-text-faint hover:text-tyba-text"
              }`}
            >
              {t("orbitOrchestrate")}
            </button>
          )}
          <NodeDir id={id} kind={kind} />
        </div>
      )}
      {session ? (
        <NodeTerminal nodeId={id} sessionId={session} selected={selected} />
      ) : (
        <textarea
          aria-label={t("orbitPrompt")}
          placeholder={t("orbitPrompt")}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          value={kind.prompt}
          onChange={(e) => set({ prompt: e.target.value })}
          className={termClass}
        />
      )}
    </NodeFrame>
  );
}
