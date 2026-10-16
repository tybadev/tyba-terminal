import { useTranslation } from "react-i18next";
import { NotePencil } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import type { NodeKind } from "@/lib/orbit";
import { NodeFrame, useCanvasActions, type OrbitFlowNode } from "./NodeFrame";

/** Nota em Markdown: vira arquivo para quem estiver ligado nela. */
export function NoteNode({ id, data, selected }: NodeProps<OrbitFlowNode>) {
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
        { label: t("orbitDetailKind"), value: t("orbitKindNote") },
        { label: t("orbitOutput"), value: `${t("orbitEdge_context")}, ${t("orbitEdge_file")}` },
      ]}
    >
      <textarea
        aria-label={t("orbitNoteBody")}
        placeholder={t("orbitNoteBody")}
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
