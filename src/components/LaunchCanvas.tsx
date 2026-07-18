import { Robot, TerminalWindow, TreeStructure } from "@phosphor-icons/react";
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
}

const COMPACT_AREA = 90;

export function LaunchCanvas({
  root,
  slots,
  selected,
  onSelect,
  onChange,
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

  return (
    <div
      ref={surface}
      className="relative aspect-[16/10] w-full overflow-hidden rounded-md border border-tyba-border-strong bg-tyba-sunken"
      style={{
        backgroundImage:
          "linear-gradient(to right, var(--tyba-border) 1px, transparent 1px), linear-gradient(to bottom, var(--tyba-border) 1px, transparent 1px)",
        backgroundSize: "24px 24px",
        backgroundPosition: "center",
      }}
    >
      {layout.panes.map((rect) => {
        const slotId = findSlotOfPane(root, rect.pane);
        const slot = slotId ? slotById.get(slotId) : undefined;
        const isSelected = slotId != null && slotId === selected;
        const compact = rect.w * rect.h < COMPACT_AREA;
        const isAgent = slot?.kind.type === "agent";
        return (
          <button
            key={rect.pane}
            type="button"
            onClick={() => slotId && onSelect(slotId)}
            aria-pressed={isSelected}
            aria-label={slot?.name ?? t("launchSlotUnnamed")}
            className={`tyba-focusable absolute flex flex-col gap-1 overflow-hidden p-2 text-left transition-colors ${
              isSelected
                ? "border-2 border-tyba-green bg-tyba-green-tint"
                : "border border-tyba-border-strong bg-tyba-surface hover:border-tyba-text-faint"
            }`}
            style={{
              left: `${rect.x}%`,
              top: `${rect.y}%`,
              width: `${rect.w}%`,
              height: `${rect.h}%`,
            }}
          >
            <span className="flex items-center gap-1.5 truncate">
              {isAgent ? (
                <Robot
                  size={13}
                  weight="fill"
                  className={isSelected ? "text-tyba-green" : "text-tyba-violet"}
                />
              ) : (
                <TerminalWindow size={13} className="text-tyba-text-faint" />
              )}
              <span className="truncate font-mono text-[11px] text-tyba-text">
                {slot?.name ?? t("launchSlotUnnamed")}
              </span>
            </span>
            {!compact && (
              <span className="truncate font-mono text-[10px] text-tyba-text-faint">
                {slot?.cwd_rel?.trim() ? slot.cwd_rel : "."}
                {slot?.isolate ? " · worktree" : ""}
              </span>
            )}
          </button>
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
                active ? "bg-tyba-green" : "bg-transparent"
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
