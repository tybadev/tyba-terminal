import { TreeStructure } from "@phosphor-icons/react";
import { useCallback, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { LaunchSlot, SlotId, SlotNode } from "@/lib/ipc";
import { computeRects, type DividerRect } from "@/lib/panes";
import { findSlotOfPane, setRatio, toPaneTree } from "@/lib/slotTree";

interface Props {
  root: SlotNode;
  slots: LaunchSlot[];
  selected: SlotId | null;
  onSelect: (slot: SlotId) => void;
  onChange: (root: SlotNode) => void;
  /** Ações do pane escolhido, no cabeçalho dele (quadro "Launch config"). */
  onSplit?: (kind: "v" | "h") => void;
  onRemove?: () => void;
}

/** A cor da bolinha de cada pane: a mesma linguagem do resto do app. */
function dotColor(slot: LaunchSlot | undefined): string {
  if (!slot) return "var(--tyba-text-faint)";
  if (slot.kind.type === "ssh") return "var(--tyba-cyan)";
  if (slot.kind.type === "agent") return slot.kind.runner === "codex" ? "var(--tyba-violet)" : "var(--tyba-amber)";
  return "var(--tyba-green)";
}

const COMPACT_AREA = 90;

export function LaunchCanvas({
  root,
  slots,
  selected,
  onSelect,
  onChange,
  onSplit,
  onRemove,
}: Props) {
  const { t } = useTranslation();
  const surface = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState<string | null>(null);

  const layout = useMemo(() => computeRects(toPaneTree(root)), [root]);
  const slotById = useMemo(
    () => new Map(slots.map((s) => [s.id, s])),
    [slots],
  );

  const onDividerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>, divider: DividerRect) => {
      e.preventDefault();
      e.currentTarget.setPointerCapture(e.pointerId);
      setDragging(divider.split);
    },
    [],
  );

  const onDividerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>, divider: DividerRect) => {
      if (dragging !== divider.split) return;
      const box = surface.current?.getBoundingClientRect();
      if (!box || box.width === 0 || box.height === 0) return;
      const pct =
        divider.kind === "v"
          ? ((e.clientX - box.left) / box.width) * 100
          : ((e.clientY - box.top) / box.height) * 100;
      onChange(
        setRatio(root, divider.split, (pct - divider.start) / divider.length),
      );
    },
    [dragging, onChange, root],
  );

  const endDrag = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    setDragging(null);
  }, []);

  const kindLabel = (slot: LaunchSlot | undefined) => {
    if (!slot) return "";
    if (slot.kind.type === "ssh") return t("launchSlotRemote");
    const parts: string[] = [];
    if (slot.kind.type === "agent") {
      parts.push(slot.kind.runner === "codex" ? "codex" : "claude");
      if (slot.kind.runner === "claude_code" && slot.model) parts.push(slot.model);
    } else {
      parts.push("shell");
    }
    if (slot.cwd_rel?.trim() && slot.cwd_rel !== ".") parts.push(slot.cwd_rel);
    if (slot.isolate) parts.push(t("launchOwnWorktree"));
    return parts.join(" · ");
  };

  return (
    <div ref={surface} className="relative h-full w-full">
      {layout.panes.map((rect) => {
        const slotId = findSlotOfPane(root, rect.pane);
        const slot = slotId ? slotById.get(slotId) : undefined;
        const isSelected = slotId != null && slotId === selected;
        const compact = rect.w * rect.h < COMPACT_AREA;
        const prompt = slot?.initial_prompt?.trim();
        return (
          <div
            key={rect.pane}
            role="button"
            tabIndex={0}
            onClick={() => slotId && onSelect(slotId)}
            onKeyDown={(e) => {
              if ((e.key === "Enter" || e.key === " ") && slotId) onSelect(slotId);
            }}
            aria-pressed={isSelected}
            aria-label={slot?.name ?? t("launchSlotUnnamed")}
            className={`tyba-focusable absolute flex flex-col overflow-hidden rounded-[8px] bg-tyba-sunken text-left ${
              isSelected
                ? "shadow-[0_0_0_2px_var(--tyba-green)]"
                : "shadow-[inset_0_0_0_1px_var(--tyba-border)] hover:shadow-[inset_0_0_0_1px_var(--tyba-border-strong)]"
            }`}
            style={{
              left: `calc(${rect.x}% + 4px)`,
              top: `calc(${rect.y}% + 4px)`,
              width: `calc(${rect.w}% - 8px)`,
              height: `calc(${rect.h}% - 8px)`,
            }}
          >
            <div className="flex items-center gap-2 border-b border-tyba-border-soft bg-tyba-raised px-2.5 py-[7px] text-[11.5px]">
              <span className="size-[7px] shrink-0 rounded-full" style={{ background: dotColor(slot) }} />
              <span className="truncate text-tyba-text">{slot?.name ?? t("launchSlotUnnamed")}</span>
              {!compact && <span className="truncate text-tyba-text-faint">{kindLabel(slot)}</span>}
              <span className="flex-1" />
              {isSelected && onSplit && (
                <span className="flex shrink-0 gap-1" onClick={(e) => e.stopPropagation()}>
                  <button onClick={() => onSplit("v")} className="rounded-[5px] bg-tyba-surface px-1.5 py-0.5 text-[10.5px] text-tyba-text-muted shadow-[inset_0_0_0_1px_var(--tyba-border-field)] hover:text-tyba-text">
                    ⇥ {t("launchSplitVerticalShort")}
                  </button>
                  <button onClick={() => onSplit("h")} className="rounded-[5px] bg-tyba-surface px-1.5 py-0.5 text-[10.5px] text-tyba-text-muted shadow-[inset_0_0_0_1px_var(--tyba-border-field)] hover:text-tyba-text">
                    ⤓ {t("launchSplitHorizontalShort")}
                  </button>
                  {onRemove && (
                    <button
                      onClick={onRemove}
                      aria-label={t("launchRemovePane")}
                      className="rounded-[5px] bg-tyba-surface px-1.5 py-0.5 text-[10.5px] text-tyba-text-muted shadow-[inset_0_0_0_1px_var(--tyba-border-field)] hover:text-tyba-red"
                    >
                      ✕
                    </button>
                  )}
                </span>
              )}
            </div>
            {!compact && (
              <div className="flex flex-col gap-0.5 px-3 py-2.5 font-mono text-[11px] leading-[17px]">
                <span className={prompt ? "truncate text-tyba-text-muted" : "text-tyba-text-faint"}>
                  ❯ {prompt || t("launchNoPrompt")}
                </span>
                {prompt && (
                  <span className="text-tyba-text-faint">
                    {slot?.kind.type === "shell"
                      ? slot.run_on_open
                        ? t("launchRunsOnOpen")
                        : t("launchTypedOnly")
                      : t("launchSlotPromptHelp")}
                  </span>
                )}
              </div>
            )}
          </div>
        );
      })}

      {layout.dividers.map((divider) => {
        const vertical = divider.kind === "v";
        const active = dragging === divider.split;
        return (
          <div
            key={divider.split}
            role="separator"
            aria-orientation={vertical ? "vertical" : "horizontal"}
            onPointerDown={(e) => onDividerDown(e, divider)}
            onPointerMove={(e) => onDividerMove(e, divider)}
            onPointerUp={endDrag}
            onPointerCancel={endDrag}
            className={`absolute z-10 ${
              vertical ? "cursor-col-resize" : "cursor-row-resize"
            }`}
            style={
              vertical
                ? {
                    left: `${divider.at}%`,
                    top: `${divider.crossStart}%`,
                    height: `${divider.crossLength}%`,
                    width: 11,
                    transform: "translateX(-50%)",
                  }
                : {
                    top: `${divider.at}%`,
                    left: `${divider.crossStart}%`,
                    width: `${divider.crossLength}%`,
                    height: 11,
                    transform: "translateY(-50%)",
                  }
            }
          >
            <span
              className={`absolute transition-colors ${
                active ? "bg-tyba-green-fill" : "bg-transparent"
              } ${
                vertical
                  ? "left-1/2 top-0 h-full w-px -translate-x-1/2"
                  : "top-1/2 left-0 w-full h-px -translate-y-1/2"
              }`}
            />
          </div>
        );
      })}

      {layout.panes.length === 1 && (
        <span className="pointer-events-none absolute bottom-2 left-1/2 flex -translate-x-1/2 items-center gap-1.5 text-[10px] text-tyba-text-faint">
          <TreeStructure size={12} />
          {t("launchCanvasHint")}
        </span>
      )}
    </div>
  );
}
