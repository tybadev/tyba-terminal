import { useTranslation } from "react-i18next";
import { TerminalWindow } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import { Select } from "@/components/ui/select";
import { OUTPUT_KINDS, nodeSession, urlPort, type NodeKind, type OutputKind } from "@/lib/maestro";
import { firstLine } from "@/lib/nodeSummary";
import {
  NodeDir,
  NodeFrame,
  RunningLine,
  fieldClass,
  termClass,
  useMaestro,
  type MaestroFlowNode,
} from "./NodeFrame";
import { FocusButton, NodeTerminal } from "./NodeTerminal";

type ProcessKind = Extract<NodeKind, { type: "process" }>;

/** Nó de processo: um comando na pasta do nó e o tipo da saída. */
export function ProcessNode({ id, data, selected }: NodeProps<MaestroFlowNode>) {
  const { t } = useTranslation();
  const { updateNode, run, live, openInBrowser } = useMaestro();
  const node = data.node;
  const kind = node.kind as ProcessKind;
  const session = nodeSession(kind, run, id, live);
  // A porta que a sessão do nó abriu (o core detecta; chega com ele rodando).
  const url = run?.nodes[id]?.url ?? null;
  const port = urlPort(url);
  const set = (patch: Partial<ProcessKind>) =>
    updateNode(id, (n) => ({ ...n, kind: { ...(n.kind as ProcessKind), ...patch } }));

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<TerminalWindow size={14} weight="bold" />}
      markClass="bg-tyba-raised text-tyba-text-muted"
      label={t("maestroKindProcess").toLowerCase()}
      extra={session ? <FocusButton nodeId={id} /> : undefined}
    >
      {session ? (
        <RunningLine text={firstLine(kind.command) && `$ ${firstLine(kind.command)}`} kind={kind} />
      ) : (
        <div className="flex shrink-0 items-center gap-1.5 border-b border-tyba-border px-3.5 py-1.5 text-[11px] text-tyba-text-faint">
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
      {url && port !== null && (
        <div className="flex shrink-0 items-center gap-2 border-b border-tyba-border px-3.5 py-1.5 text-[11px]">
          <span className="rounded-[4px] border border-tyba-cyan/50 px-2 leading-[18px] text-tyba-cyan">
            {t("maestroPort", { port })}
          </span>
          <button
            type="button"
            onClick={() => openInBrowser(id, url)}
            className="nodrag rounded-[4px] border border-tyba-border-strong px-2 leading-[18px] text-tyba-text-muted hover:text-tyba-text"
          >
            {t("maestroOpenInBrowser")}
          </button>
        </div>
      )}
      {session ? (
        <NodeTerminal nodeId={id} sessionId={session} focused={selected} />
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
