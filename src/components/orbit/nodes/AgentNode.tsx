import { useTranslation } from "react-i18next";
import { CircleHalf } from "@phosphor-icons/react";
import { Handle, Position, type NodeProps } from "@xyflow/react";

import { ClaudeIcon } from "@/components/icons/ClaudeIcon";
import { OpenAIIcon } from "@/components/icons/OpenAIIcon";
import { Switch } from "@/components/ui/switch";
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
import { inboxSenders, lastNoteWritten, undeliveredReason } from "@/lib/orbitTalk";
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
  const { updateNode, run, live, subagents, createdBy, graphDir, telemetry, graph } = useOrbit();
  const node = data.node;
  const kind = node.kind as AgentKind;
  const session = nodeSession(kind, run, id, live);
  const rounds = session ? groupRounds(subagents.get(id) ?? []) : null;
  const tally = rounds ? roundTally(rounds.current) : null;
  const set = (patch: Partial<AgentKind>) =>
    updateNode(id, (n) => ({ ...n, kind: { ...(n.kind as AgentKind), ...patch } }));
  const claude = kind.runner === "claude";
  // Todo agente do grafo cria nós (porta das tracejadas); o criado por
  // outro agente não (sem cascata).
  const created = createdBy.has(id);
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
  // Conversa e memória (quadro MaestroConversa): a fila esperando ele fechar
  // o turno (ou a entrega que falhou), a última mensagem que ele mandou e a
  // última nota em que escreveu.
  const inbox = nodeRun?.inbox ?? 0;
  const sent = nodeRun?.sent ?? null;
  const wroteIn = lastNoteWritten(id, graph.nodes, graph.edges, run);
  const senders = inboxSenders(nodeRun, graph.nodes);
  const undelivered = nodeRun?.undelivered ?? null;
  const footer = [
    sent && (
      <div key="sent" className="truncate">
        <span className="text-tyba-amber">{t("orbitSentTo", { name: sent.to })}</span>{" "}
        <span className="text-tyba-text-muted">“{sent.text}”</span>
      </div>
    ),
    wroteIn && (
      <div key="wrote" className="truncate text-tyba-violet">
        {t("orbitWroteIn", { name: wroteIn })}
      </div>
    ),
    undelivered ? (
      <div key="undelivered" className="truncate text-tyba-red" title={undelivered}>
        {t("orbitUndelivered", { reason: undeliveredReason(undelivered) })}
      </div>
    ) : (
      inbox > 0 &&
      senders.length > 0 && (
        <div key="inbox" className="truncate">
          {t("orbitInboxNext", { count: inbox, names: senders.join(", ") })}
        </div>
      )
    ),
    session && meta && (
      <div key="meta" className="truncate">
        {meta}
      </div>
    ),
  ].filter(Boolean);

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={claude ? <ClaudeIcon size={13} /> : <OpenAIIcon size={13} />}
      markTone={claude ? "text-tyba-amber" : "text-tyba-cyan"}
      meta={footer.length > 0 ? footer : undefined}
      extra={
        inbox > 0 && (
          <span className="shrink-0 text-[11.5px] whitespace-nowrap text-tyba-amber">
            {t("orbitInboxWaiting", { count: inbox })}
          </span>
        )
      }
      // Grade do quadro OrbitCardsD (o tempo entra na ordem dela).
      stateRows={false}
      details={[
        ...agentDetailRows({
          runner: kind.runner,
          role: kind.role,
          dir: dir ? shortDir(dir) : "",
          task: firstLine(kind.prompt),
          telemetry: usage,
        }).map((row) => ({
          label: t(row.key),
          value: row.value === null ? nodeRun && <Elapsed nodeRun={nodeRun} /> : row.value,
        })),
        // O criado por outro agente já nasce sem criar (sem cascata).
        ...(created
          ? []
          : [
              {
                label: t("orbitNoCreate"),
                value: (
                  <Switch
                    aria-label={t("orbitNoCreate")}
                    title={t("orbitNoCreateHint")}
                    checked={kind.noCreate === true}
                    onCheckedChange={(on) => set({ noCreate: on })}
                    className="nodrag"
                  />
                ),
              },
            ]),
      ]}
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
      {!created && (
        <Handle
          type="source"
          id={ORCHESTRATOR_HANDLE}
          position={Position.Right}
          isConnectable={false}
          // Na ponta direita da cápsula, junto da porta de saída.
          style={{ ...hidden, top: 20, right: 14 }}
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
