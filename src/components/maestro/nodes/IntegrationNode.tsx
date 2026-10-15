import { useTranslation } from "react-i18next";
import { PlugsConnected, Warning } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import type { NodeKind } from "@/lib/maestro";
import { NodeFrame, useMaestro, type MaestroFlowNode } from "./NodeFrame";

/**
 * Nó de integração: aponta para um script salvo na biblioteca. Aqui só se
 * mostra; o script se edita no painel de Integrações (B8).
 */
export function IntegrationNode({ id, data, selected }: NodeProps<MaestroFlowNode>) {
  const { t } = useTranslation();
  const { integrations } = useMaestro();
  const node = data.node;
  const kind = node.kind as Extract<NodeKind, { type: "integration" }>;
  const integration = integrations.get(kind.integrationId);

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<PlugsConnected size={14} weight="bold" />}
      markClass="bg-tyba-blue-tint text-tyba-blue"
      label={integration?.name ?? t("maestroKindIntegration").toLowerCase()}
    >
      {integration ? (
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
