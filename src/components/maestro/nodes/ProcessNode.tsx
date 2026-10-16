import { useTranslation } from "react-i18next";
import { TerminalWindow } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import { Select } from "@/components/ui/select";
import {
  OUTPUT_KINDS,
  nodeDir,
  nodeSession,
  shortDir,
  urlPort,
  type NodeKind,
  type OutputKind,
} from "@/lib/maestro";
import { firstLine } from "@/lib/nodeSummary";
import { NodeDir, NodeFrame, fieldClass, termClass, useMaestro, type MaestroFlowNode } from "./NodeFrame";
import { NodeTerminal } from "./NodeTerminal";

type ProcessKind = Extract<NodeKind, { type: "process" }>;

/** Nó de processo: um comando na pasta do nó e o tipo da saída. */
export function ProcessNode({ id, data, selected }: NodeProps<MaestroFlowNode>) {
  const { t } = useTranslation();
  const { updateNode, run, live, openInBrowser, graphDir } = useMaestro();
  const node = data.node;
  const kind = node.kind as ProcessKind;
  const session = nodeSession(kind, run, id, live);
  // A porta que a sessão do nó abriu (o core detecta; chega com ele rodando).
  const url = run?.nodes[id]?.url ?? null;
  const port = urlPort(url);
  const set = (patch: Partial<ProcessKind>) =>
    updateNode(id, (n) => ({ ...n, kind: { ...(n.kind as ProcessKind), ...patch } }));
  const dir = nodeDir(kind, graphDir);
  const command = firstLine(kind.command);

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<TerminalWindow size={13} weight="bold" />}
      markTone="text-tyba-text-muted"
      extra={
        url && port !== null ? (
          <button
            type="button"
            title={t("maestroOpenInBrowser")}
            onClick={() => openInBrowser(id, url)}
            className="nodrag shrink-0 rounded-full bg-tyba-cyan/[.12] px-2.5 text-[11.5px] leading-[20px] text-tyba-cyan hover:bg-tyba-cyan/[.2]"
          >
            :{port}
          </button>
        ) : undefined
      }
      meta={session ? [command, dir ? shortDir(dir) : ""].filter(Boolean).join(" · ") : undefined}
      details={[
        { label: t("maestroCommand"), value: command },
        { label: t("maestroOutput"), value: t(`maestroEdge_${kind.output}`) },
        { label: t("maestroDetailDir"), value: dir ? shortDir(dir) : "" },
        { label: t("maestroDetailPort"), value: port !== null ? `:${port}` : "" },
      ]}
    >
      {!session && (
        <div className="flex shrink-0 items-center gap-1.5 border-b border-tyba-border px-3.5 pb-1.5 text-[11px] text-tyba-text-faint">
          <span>{t("maestroOutput")}</span>
          <div className="nodrag">
            <Select
              value={kind.output}
              options={OUTPUT_KINDS.map((k) => ({ value: k, label: t(`maestroEdge_${k}`) }))}
              onChange={(v) => set({ output: v as OutputKind })}
              className="h-6 text-[11px]"
            />
          </div>
          {kind.output === "file" && (
            <input
              aria-label={t("maestroOutputFile")}
              placeholder={t("maestroOutputFile")}
              spellCheck={false}
              autoCorrect="off"
              autoCapitalize="off"
              value={kind.file ?? ""}
              onChange={(e) => set({ file: e.target.value || null })}
              className={`${fieldClass} min-w-0 flex-1`}
            />
          )}
          <span className="flex-1" />
          <NodeDir id={id} kind={kind} />
        </div>
      )}
      {session ? (
        <NodeTerminal nodeId={id} sessionId={session} selected={selected} />
      ) : (
        <div className="flex min-h-0 flex-1 bg-tyba-sunken">
          <span className="select-none pt-2.5 pl-3.5 font-mono text-[11.5px] leading-[17px] text-tyba-green">
            $
          </span>
          <textarea
            aria-label={t("maestroCommand")}
            placeholder={t("maestroCommand")}
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            value={kind.command}
            onChange={(e) => set({ command: e.target.value })}
            className={`${termClass} pl-2 text-tyba-text`}
          />
        </div>
      )}
    </NodeFrame>
  );
}
