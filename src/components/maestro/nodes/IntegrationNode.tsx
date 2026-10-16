import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Play, PlugsConnected, Warning } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import { nodeDir, nodeSession, shortDir, type NodeKind } from "@/lib/maestro";
import { firstLine } from "@/lib/nodeSummary";
import { NodeDir, NodeFrame, useMaestro, type MaestroFlowNode } from "./NodeFrame";
import { NodeTerminal } from "./NodeTerminal";

/**
 * Nó de integração: aponta para um script salvo na biblioteca. Aqui só se
 * mostra; o script se edita no painel de Integrações (B8).
 */
export function IntegrationNode({ id, data, selected }: NodeProps<MaestroFlowNode>) {
  const { t } = useTranslation();
  const { integrations, run, live, runNode, graphDir } = useMaestro();
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
  const dir = nodeDir(kind, graphDir);
  const trigger = integration
    ? integration.trigger.type === "manual"
      ? t("maestroTriggerManual")
      : t("maestroTriggerOnInput", { kind: t(`maestroEdge_${integration.trigger.kind}`) })
    : "";

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<PlugsConnected size={13} weight="bold" />}
      markTone="text-tyba-violet"
      // Esperando o ▶: o botão já diz; a palavra "aguardando" sairia em dobro.
      status={canRelease ? false : undefined}
      extra={
        canRelease && run ? (
          <button
            type="button"
            title={t("maestroRunNode")}
            onClick={() => {
              setReleased(run.id);
              runNode(id);
            }}
            className="nodrag flex shrink-0 items-center gap-1 rounded-full bg-tyba-green px-3 text-[11.5px] leading-[22px] text-tyba-text-inverse hover:opacity-90"
          >
            <Play size={10} weight="fill" />
            {t("maestroRelease")}
          </button>
        ) : null
      }
      meta={
        integration && nodeRun
          ? [firstLine(integration.script), trigger].filter(Boolean).join(" · ")
          : undefined
      }
      details={[
        { label: t("maestroDetailKind"), value: integration?.name ?? "" },
        { label: t("maestroDetailTrigger"), value: trigger },
        {
          label: t("maestroOutput"),
          value: integration ? t(`maestroEdge_${integration.output}`) : "",
        },
        { label: t("maestroDetailDir"), value: dir ? shortDir(dir) : "" },
      ]}
    >
      {!session && (
        <div className="flex shrink-0 items-center border-b border-tyba-border px-3.5 pb-1">
          <NodeDir id={id} kind={kind} />
        </div>
      )}
      {session ? (
        <NodeTerminal nodeId={id} sessionId={session} selected={selected} />
      ) : integration ? (
        <div className="flex min-h-0 flex-1 flex-col bg-tyba-sunken px-3.5 py-2.5 font-mono text-[11.5px] leading-[17px] text-tyba-text-muted">
          <span className="shrink-0 text-tyba-text-faint">{trigger}</span>
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
