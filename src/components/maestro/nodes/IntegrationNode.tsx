import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Play, PlugsConnected, Warning } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import { nodeSession, type NodeKind } from "@/lib/maestro";
import { NodeDir, NodeFrame, useMaestro, type MaestroFlowNode } from "./NodeFrame";
import { FocusButton, NodeTerminal } from "./NodeTerminal";

/**
 * Nó de integração: aponta para um script salvo na biblioteca. Aqui só se
 * mostra; o script se edita no painel de Integrações (B8).
 */
export function IntegrationNode({ id, data, selected }: NodeProps<MaestroFlowNode>) {
  const { t } = useTranslation();
  const { integrations, run, live, runNode } = useMaestro();
  const node = data.node;
  const kind = node.kind as Extract<NodeKind, { type: "integration" }>;
  const integration = integrations.get(kind.integrationId);
  const nodeRun = run?.nodes[id];
  const session = nodeSession(kind, run, id, live);
  // Run em que o dono já deu ▶: o nó segue `waiting` até as entradas chegarem.
  const [released, setReleased] = useState<string | null>(null);
  const canRelease =
    integration?.trigger.type === "manual" &&
    run?.status === "running" &&
    nodeRun?.status === "waiting" &&
    released !== run.id;

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<PlugsConnected size={14} weight="bold" />}
      markClass="bg-tyba-blue-tint text-tyba-blue"
      label={integration?.name ?? t("maestroKindIntegration").toLowerCase()}
      extra={
        canRelease && run ? (
          <button
            type="button"
            aria-label={t("maestroRunNode")}
            title={t("maestroRunNode")}
            onClick={() => {
              setReleased(run.id);
              runNode(id);
            }}
            className="nodrag flex shrink-0 items-center rounded-[5px] border border-tyba-green/40 bg-tyba-green-tint p-1 text-tyba-green hover:border-tyba-green"
          >
            <Play size={11} weight="fill" />
          </button>
        ) : session ? (
          <FocusButton nodeId={id} />
        ) : null
      }
    >
      <div className="flex shrink-0 items-center border-b border-tyba-border px-3.5 py-1">
        <NodeDir id={id} kind={kind} />
      </div>
      {session ? (
        <NodeTerminal nodeId={id} sessionId={session} focused={selected} />
      ) : integration ? (
        <div className="flex min-h-0 flex-1 flex-col bg-tyba-sunken px-3.5 py-2.5 font-mono text-[11.5px] leading-[17px] text-tyba-text-muted">
          <span className="shrink-0 text-tyba-text-faint">
            {integration.trigger.type === "manual"
              ? t("maestroTriggerManual")
              : t("maestroTriggerOnInput", {
                  kind: t(`maestroEdge_${integration.trigger.kind}`),
                })}
          </span>
          <pre className="nowheel min-h-0 flex-1 overflow-hidden whitespace-pre text-ellipsis">
            {integration.script}
          </pre>
        </div>
      ) : (
        <div className="flex flex-1 items-center gap-2 px-3.5 text-[11.5px] text-tyba-amber">
          <Warning size={13} />
          {t("maestroIntegrationMissing")}
        </div>
      )}
    </NodeFrame>
  );
}
