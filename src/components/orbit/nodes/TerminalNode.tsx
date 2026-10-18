import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Terminal, X } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import { nodeDir, nodeSession, shortDir, terminalLive, type NodeKind } from "@/lib/orbit";
import { NodeDir, NodeFrame, detailBtn, useOrbit, type OrbitFlowNode } from "./NodeFrame";
import { NodeTerminal } from "./NodeTerminal";

type TerminalKind = Extract<NodeKind, { type: "terminal" }>;

/**
 * Nó Terminal (quadro OrbitTerminal 3): o shell do dono no canvas. Aberto,
 * a cápsula diz "ao vivo" e o corpo é o terminal de verdade; fechado (o nó
 * salvo, ao reabrir o app), o corpo tem só o Abrir — nunca abre sozinho.
 */
export function TerminalNode({ id, data, selected }: NodeProps<OrbitFlowNode>) {
  const { t } = useTranslation();
  const { run, live, graphDir, openTerminal, closeTerminal } = useOrbit();
  const node = data.node;
  const kind = node.kind as TerminalKind;
  const open = terminalLive(run, id);
  const session = open ? nodeSession(kind, run, id, live) : null;
  const dir = nodeDir(kind, graphDir);
  // Trava o Abrir até o core responder: sem dois cliques abrindo dois shells.
  const [opening, setOpening] = useState(false);
  const openNow = () => {
    if (opening) return;
    setOpening(true);
    void openTerminal(id).finally(() => setOpening(false));
  };

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<Terminal size={14} weight="bold" />}
      markTone={open ? "text-tyba-green" : "text-tyba-text-muted"}
      ring={open ? "live" : "idle"}
      status={
        open ? (
          <span className="shrink-0 rounded-full bg-tyba-green/[.14] px-2.5 text-[11.5px] leading-[20px] text-tyba-green">
            {t("orbitTerminalLive")}
          </span>
        ) : (
          false
        )
      }
      meta={open && dir ? shortDir(dir) : undefined}
      details={[{ label: t("orbitDetailDir"), value: dir ? shortDir(dir) : "" }]}
      actions={(close) =>
        open && (
          <button
            type="button"
            onClick={() => {
              close();
              closeTerminal(id);
            }}
            className={`${detailBtn} text-tyba-text hover:bg-tyba-text/[.12]`}
          >
            <X size={12} />
            {t("orbitTerminalClose")}
          </button>
        )
      }
    >
      {session ? (
        <NodeTerminal nodeId={id} sessionId={session} selected={selected} />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 bg-tyba-sunken">
          <button
            type="button"
            disabled={opening}
            onClick={openNow}
            className="nodrag inline-flex items-center gap-1.5 rounded-full border border-tyba-green/45 bg-tyba-green-tint px-3.5 py-1 text-[12px] font-medium text-tyba-green hover:border-tyba-green disabled:opacity-60"
          >
            <Terminal size={13} weight="bold" />
            {t("orbitTerminalOpen")}
          </button>
          <NodeDir id={id} kind={kind} />
        </div>
      )}
    </NodeFrame>
  );
}
