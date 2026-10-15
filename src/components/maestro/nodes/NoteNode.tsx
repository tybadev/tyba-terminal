import { useTranslation } from "react-i18next";
import { NotePencil } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import type { NodeKind } from "@/lib/maestro";
import { NodeFrame, termClass, useMaestro, type MaestroFlowNode } from "./NodeFrame";

/** Nota em Markdown: vira arquivo para quem estiver ligado nela. */
export function NoteNode({ id, data, selected }: NodeProps<MaestroFlowNode>) {
  const { t } = useTranslation();
  const { updateNode } = useMaestro();
  const node = data.node;
  const kind = node.kind as Extract<NodeKind, { type: "note" }>;

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<NotePencil size={14} weight="bold" />}
      markClass="bg-tyba-violet-tint text-tyba-violet"
      label={t("maestroKindNote").toLowerCase()}
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
        className={`${termClass} bg-transparent leading-[19px]`}
      />
    </NodeFrame>
  );
}
