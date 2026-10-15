import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  MeasuringStrategy,
  PointerSensor,
  closestCorners,
  useDroppable,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragOverEvent,
  type UniqueIdentifier,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  rectSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { DotsSixVertical } from "@phosphor-icons/react";

import {
  type ChipId,
  type ToolbarPref,
} from "../lib/repoSnapshots";
import {
  dropTarget,
  isToolbarZone,
  moveChip,
  zoneOf,
  TOOLBAR_ZONES,
  type ToolbarZone,
} from "../lib/toolbarLayout";

const CHIP_LABEL_KEYS: Record<ChipId, string> = {
  cwd: "toolbarCwd",
  branch: "toolbarBranch",
  diffCount: "toolbarDiff",
  reviewDiff: "toolbarReviewDiff",
  aheadBehind: "toolbarAheadBehind",
  clock: "toolbarClock",
};

const ZONE_LABEL_KEYS: Record<ToolbarZone, string> = {
  left: "chipsZoneLeft",
  right: "chipsZoneRight",
  hidden: "chipsZoneHidden",
};

/**
 * A barra como ela vai ficar, com valores de exemplo — e acompanhando o
 * arrasto: o chip aparece no lugar novo enquanto ainda está na mão.
 */
function ToolbarPreview({ pref }: { pref: ToolbarPref }) {
  const { t } = useTranslation();
  const sample = (id: ChipId) => {
    switch (id) {
      case "cwd":
        return <span>tyba-terminal</span>;
      case "branch":
        return <span className="text-tyba-blue">main</span>;
      case "diffCount":
        return (
          <span>
            <span className="text-tyba-green">+12</span>{" "}
            <span className="text-tyba-red">−3</span>
          </span>
        );
      case "reviewDiff":
        return (
          <span className="rounded-[4px] border border-tyba-border-field px-1.5">
            {t("toolbarReviewDiff")}
          </span>
        );
      case "aheadBehind":
        return <span className="text-tyba-green">↑2</span>;
      case "clock":
        return <span className="tabular-nums">23:50</span>;
    }
  };
  return (
    <div aria-hidden className="mb-3 overflow-hidden rounded-[6px] border border-tyba-border">
      <div className="flex h-7 items-center gap-3.5 bg-tyba-raised px-3 font-mono text-[11px] text-tyba-text-muted">
        {pref.left.map((id) => (
          <span key={id}>{sample(id)}</span>
        ))}
        <span className="flex-1" />
        {pref.right.map((id) => (
          <span key={id}>{sample(id)}</span>
        ))}
      </div>
      <div className="h-9 bg-tyba-sunken px-3 pt-2 font-mono text-[12px] text-tyba-text-ghost">❯ _</div>
    </div>
  );
}

interface Props {
  pref: ToolbarPref;
  onChange: (next: ToolbarPref) => void;
}

function ChipPill({ id, overlay }: { id: ChipId; overlay?: boolean }) {
  const { t } = useTranslation();
  return (
    <span
      className={`flex items-center gap-1.5 rounded-[6px] border border-tyba-border-field bg-tyba-raised py-[5px] pr-[9px] pl-1.5 text-[12px] text-tyba-text ${
        overlay ? "shadow-lg" : ""
      }`}
    >
      <DotsSixVertical size={12} className="shrink-0 text-tyba-text-muted" />
      {t(CHIP_LABEL_KEYS[id])}
    </span>
  );
}

function SortableChip({ id }: { id: ChipId }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } =
    useSortable({ id });
  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={`cursor-grab touch-none focus-visible:outline-1 focus-visible:outline-tyba-green ${
        isDragging ? "opacity-40" : ""
      }`}
      {...attributes}
      {...listeners}
    >
      <ChipPill id={id} />
    </li>
  );
}

function Zone({ zone, chips }: { zone: ToolbarZone; chips: ChipId[] }) {
  const { t } = useTranslation();
  const { setNodeRef } = useDroppable({ id: zone });
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-2 rounded-[8px] border border-tyba-border bg-tyba-raised px-3.5 py-3">
      <span className="text-[11px] uppercase tracking-[.06em] text-tyba-text-faint">
        {t(ZONE_LABEL_KEYS[zone])}
      </span>
      <SortableContext items={chips} strategy={rectSortingStrategy}>
        <ul ref={setNodeRef} className="flex min-h-[30px] flex-wrap gap-1.5">
          {chips.map((id) => (
            <SortableChip key={id} id={id} />
          ))}
        </ul>
      </SortableContext>
      <span className="text-[10.5px] text-tyba-text-ghost">
        {zone === "hidden" ? t("chipsZoneHiddenHint") : t("chipsZoneDropHint")}
      </span>
    </div>
  );
}

export function ToolbarChipsEditor({ pref, onChange }: Props) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<ToolbarPref | null>(null);
  const [activeId, setActiveId] = useState<ChipId | null>(null);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const shown = draft ?? pref;

  const labelOf = (id: UniqueIdentifier): string =>
    isToolbarZone(id)
      ? t(ZONE_LABEL_KEYS[id])
      : t(CHIP_LABEL_KEYS[id as ChipId]);

  const handleOver = (event: DragOverEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    setDraft((prev) => {
      const current = prev ?? pref;
      const target = dropTarget(current, over.id);
      if (!target || target.zone === zoneOf(current, active.id as ChipId)) {
        return prev;
      }
      return moveChip(current, active.id as ChipId, target.zone, target.index);
    });
  };

  const handleEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (over) {
      const current = draft ?? pref;
      let final = current;
      if (active.id !== over.id) {
        const target = dropTarget(current, over.id);
        if (target) {
          final = moveChip(current, active.id as ChipId, target.zone, target.index);
        }
      }
      if (final !== pref) onChange(final);
    }
    setDraft(null);
    setActiveId(null);
  };

  return (
    <div className="border-b border-tyba-border-soft px-[18px] pt-1 pb-3">
      <ToolbarPreview pref={shown} />
      <DndContext
        sensors={sensors}
        collisionDetection={closestCorners}
        measuring={{ droppable: { strategy: MeasuringStrategy.Always } }}
        accessibility={{
          screenReaderInstructions: { draggable: t("chipsDragInstructions") },
          announcements: {
            onDragStart: ({ active }) =>
              t("chipsDragStart", { chip: labelOf(active.id) }),
            onDragOver: ({ active, over }) =>
              over
                ? t("chipsDragOver", {
                    chip: labelOf(active.id),
                    target: labelOf(over.id),
                  })
                : t("chipsDragStart", { chip: labelOf(active.id) }),
            onDragEnd: ({ active, over }) =>
              over
                ? t("chipsDragEnd", {
                    chip: labelOf(active.id),
                    target: labelOf(over.id),
                  })
                : t("chipsDragCancel", { chip: labelOf(active.id) }),
            onDragCancel: ({ active }) =>
              t("chipsDragCancel", { chip: labelOf(active.id) }),
          },
        }}
        onDragStart={(event) => {
          setActiveId(event.active.id as ChipId);
          setDraft(pref);
        }}
        onDragOver={handleOver}
        onDragEnd={handleEnd}
        onDragCancel={() => {
          setDraft(null);
          setActiveId(null);
        }}
      >
        <div className="flex gap-2.5">
          {TOOLBAR_ZONES.map((zone) => (
            <Zone key={zone} zone={zone} chips={shown[zone]} />
          ))}
        </div>
        <DragOverlay>
          {activeId ? <ChipPill id={activeId} overlay /> : null}
        </DragOverlay>
      </DndContext>
    </div>
  );
}
