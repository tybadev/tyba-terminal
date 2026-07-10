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
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { DotsSixVertical } from "@phosphor-icons/react";

import {
  DEFAULT_TOOLBAR,
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

interface Props {
  pref: ToolbarPref;
  onChange: (next: ToolbarPref) => void;
}

function ChipPill({ id, overlay }: { id: ChipId; overlay?: boolean }) {
  const { t } = useTranslation();
  return (
    <span
      className={`flex w-full items-center gap-1.5 rounded-[5px] border border-tyba-border bg-tyba-surface px-2 py-1 text-[12px] text-tyba-text ${
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
    <div className="flex min-w-0 flex-col gap-1.5">
      <span className="tyba-label">{t(ZONE_LABEL_KEYS[zone])}</span>
      <SortableContext items={chips} strategy={verticalListSortingStrategy}>
        <ul
          ref={setNodeRef}
          className={`flex min-h-[72px] flex-col gap-1 rounded-[6px] border p-1.5 ${
            chips.length === 0
              ? "border-dashed border-tyba-border"
              : "border-tyba-border/60"
          }`}
        >
          {chips.map((id) => (
            <SortableChip key={id} id={id} />
          ))}
          {chips.length === 0 && (
            <li className="flex flex-1 items-center justify-center text-[11px] text-tyba-text-faint">
              {t("chipsZoneEmpty")}
            </li>
          )}
        </ul>
      </SortableContext>
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
    <div className="mt-3 rounded-[8px] border border-tyba-border p-3">
      <div className="flex items-baseline justify-between pb-2">
        <div>
          <div className="text-[13px] text-tyba-text">{t("chipsEditorTitle")}</div>
          <div className="text-[11px] text-tyba-text-faint">
            {t("chipsEditorHint")}
          </div>
        </div>
        <button
          onClick={() => onChange({ ...DEFAULT_TOOLBAR, enabled: pref.enabled })}
          className="shrink-0 text-[11px] text-tyba-text-muted transition-colors hover:text-tyba-text"
        >
          {t("chipsEditorReset")}
        </button>
      </div>
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
        <div className="grid grid-cols-3 gap-2">
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
