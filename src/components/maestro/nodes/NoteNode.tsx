import { useTranslation } from "react-i18next";
import { NotePencil } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import type { NodeKind } from "@/lib/maestro";
import { NodeFrame, useCanvasActions, type MaestroFlowNode } from "./NodeFrame";

/** Nota em Markdown: vira arquivo para quem estiver ligado nela. */
export function NoteNode({ id, data, selected }: NodeProps<MaestroFlowNode>) {
  const { t } = useTranslation();
  const { updateNode } = useCanvasActions();
  const node = data.node;
  const kind = node.kind as Extract<NodeKind, { type: "note" }>;

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<NotePencil size={13} weight="bold" />}
      markTone="text-tyba-violet"
      paper
      details={[
        { label: t("maestroDetailKind"), value: t("maestroKindNote") },
        { label: t("maestroOutput"), value: `${t("maestroEdge_context")}, ${t("maestroEdge_file")}` },
      ]}
    >
      <textarea
        aria-label={t("maestroNoteBody")}
        placeholder={t("maestroNoteBody")}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        value={kind.markdown}
        onChange={(e) => {
          const markdown = e.target.value;
          updateNode(id, (n) => ({ ...n, kind: { type: "note", markdown } }));
        }}
        className="nodrag nowheel min-h-0 w-full flex-1 resize-none bg-transparent px-4 pt-1 pb-3 font-sans text-[13px] leading-[1.6] text-tyba-text outline-none placeholder:text-tyba-text-ghost"
      />
    </NodeFrame>
  );
}
