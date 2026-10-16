import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowClockwise, TerminalWindow } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import { Select } from "@/components/ui/select";
import {
  OUTPUT_KINDS,
  nodeDir,
  nodeLog,
  nodeSession,
  shortDir,
  urlPort,
  type NodeKind,
  type OutputKind,
} from "@/lib/orbit";
import { firstLine } from "@/lib/nodeSummary";
import { showsFailureLog } from "@/lib/orbitCard";
import {
  NodeDir,
  NodeFrame,
  fieldClass,
  soonClass,
  termClass,
  useOrbit,
  type OrbitFlowNode,
} from "./NodeFrame";
import { NodeTerminal } from "./NodeTerminal";

type ProcessKind = Extract<NodeKind, { type: "process" }>;

/** Nó de processo: um comando na pasta do nó e o tipo da saída. */
export function ProcessNode({ id, data, selected }: NodeProps<OrbitFlowNode>) {
  const { t } = useTranslation();
  const { updateNode, run, live, openInBrowser, graphDir } = useOrbit();
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
  const failed = run?.nodes[id]?.status === "failed";
  const failureLog = run && showsFailureLog(kind, run.nodes[id]) ? run.id : null;

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<TerminalWindow size={13} weight="bold" />}
      markTone="text-tyba-text-muted"
      // Quadro: falhou, a cápsula leva só o ↻ (o anel vermelho já diz).
      status={failed ? false : undefined}
      extra={
        failed ? (
          <button
            type="button"
            aria-disabled
            aria-label={t("orbitRestart")}
            title={t("orbitRestartSoon")}
            className={`nodrag shrink-0 rounded-full bg-tyba-text/[.08] px-2.5 py-[3px] ${soonClass}`}
          >
            <ArrowClockwise size={12} />
          </button>
        ) : url && port !== null ? (
          <button
            type="button"
            title={t("orbitOpenInBrowser")}
            onClick={() => openInBrowser(id, url)}
            className="nodrag shrink-0 rounded-full bg-tyba-cyan/[.12] px-2.5 text-[11.5px] leading-[20px] text-tyba-cyan hover:bg-tyba-cyan/[.2]"
          >
            :{port}
          </button>
        ) : undefined
      }
      meta={session ? [command, dir ? shortDir(dir) : ""].filter(Boolean).join(" · ") : undefined}
      details={[
        { label: t("orbitCommand"), value: command },
        { label: t("orbitOutput"), value: t(`orbitEdge_${kind.output}`) },
        { label: t("orbitDetailDir"), value: dir ? shortDir(dir) : "" },
        { label: t("orbitDetailPort"), value: port !== null ? `:${port}` : "" },
      ]}
    >
      {!session && (
        <div className="flex shrink-0 items-center gap-1.5 border-b border-tyba-border px-3.5 pb-1.5 text-[11px] text-tyba-text-faint">
          <span>{t("orbitOutput")}</span>
          <div className="nodrag">
            <Select
              value={kind.output}
              options={OUTPUT_KINDS.map((k) => ({ value: k, label: t(`orbitEdge_${k}`) }))}
              onChange={(v) => set({ output: v as OutputKind })}
              className="h-6 text-[11px]"
            />
          </div>
          {kind.output === "file" && (
            <input
              aria-label={t("orbitOutputFile")}
              placeholder={t("orbitOutputFile")}
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
      ) : failureLog ? (
        <FailureLog runId={failureLog} nodeId={id} command={command} />
      ) : (
        <div className="flex min-h-0 flex-1 bg-tyba-sunken">
          <span className="select-none pt-2.5 pl-3.5 font-mono text-[11.5px] leading-[17px] text-tyba-green">
            $
          </span>
          <textarea
            aria-label={t("orbitCommand")}
            placeholder={t("orbitCommand")}
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

const FAILURE_LOG_LINES = 14;

/**
 * O processo que caiu: o comando e o fim do `log.txt`, lido do core (a sessão
 * já morreu e o terminal saiu do card). Quem cai em 0s deixava só o comando
 * à vista, sem o erro.
 */
function FailureLog({ runId, nodeId, command }: { runId: string; nodeId: string; command: string }) {
  const [lines, setLines] = useState<string[]>([]);
  useEffect(() => {
    let disposed = false;
    void nodeLog(runId, nodeId, FAILURE_LOG_LINES)
      .then((rows) => {
        if (!disposed) setLines(rows);
      })
      .catch(() => {});
    return () => {
      disposed = true;
    };
  }, [runId, nodeId]);

  return (
    <div
      role="log"
      className="relative flex min-h-0 flex-1 flex-col justify-end overflow-hidden bg-tyba-sunken px-3.5 pt-2.5 pb-3 font-mono text-[11.5px] leading-[17px] text-tyba-text-muted"
    >
      {/* Como a prévia do terminal: linhas de altura fixa, o excesso sai por cima. */}
      {[`$ ${command}`, ...lines].map((line, i) => (
        <div key={i} className="h-[17px] shrink-0 overflow-hidden text-ellipsis whitespace-pre">
          {line || " "}
        </div>
      ))}
    </div>
  );
}
