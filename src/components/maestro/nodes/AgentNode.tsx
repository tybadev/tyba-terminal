import { useTranslation } from "react-i18next";
import { Robot } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import type { NodeKind, Runner } from "@/lib/maestro";
import { NodeFrame, fieldClass, termClass, useMaestro, type MaestroFlowNode } from "./NodeFrame";

type AgentKind = Extract<NodeKind, { type: "agent" }>;

const RUNNERS: Runner[] = ["claude", "codex"];

/** Nó de agente: Claude (âmbar) ou Codex (ciano), papel e a tarefa. */
export function AgentNode({ id, data, selected }: NodeProps<MaestroFlowNode>) {
  const { t } = useTranslation();
  const { updateNode } = useMaestro();
  const node = data.node;
  const kind = node.kind as AgentKind;
  const set = (patch: Partial<AgentKind>) =>
    updateNode(id, (n) => ({ ...n, kind: { ...(n.kind as AgentKind), ...patch } }));
  const claude = kind.runner === "claude";

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<Robot size={14} weight="bold" />}
      markClass={claude ? "bg-tyba-amber-tint text-tyba-amber" : "bg-tyba-cyan-tint text-tyba-cyan"}
      label={kind.role || t("maestroKindAgent")}
    >
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
          className={`${fieldClass} flex-1`}
        />
      </div>
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
    </NodeFrame>
  );
}
