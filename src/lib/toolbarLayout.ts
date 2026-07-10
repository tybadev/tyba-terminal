import type { ChipId, ToolbarPref } from "./repoSnapshots";

export type ToolbarZone = "left" | "right" | "hidden";

export const TOOLBAR_ZONES: readonly ToolbarZone[] = ["left", "right", "hidden"];

export function isToolbarZone(value: unknown): value is ToolbarZone {
  return TOOLBAR_ZONES.includes(value as ToolbarZone);
}

export function zoneOf(pref: ToolbarPref, id: ChipId): ToolbarZone | null {
  for (const zone of TOOLBAR_ZONES) {
    if (pref[zone].includes(id)) return zone;
  }
  return null;
}

export function dropTarget(
  pref: ToolbarPref,
  overId: unknown,
): { zone: ToolbarZone; index: number } | null {
  if (isToolbarZone(overId)) {
    return { zone: overId, index: pref[overId].length };
  }
  const zone = zoneOf(pref, overId as ChipId);
  if (!zone) return null;
  return { zone, index: pref[zone].indexOf(overId as ChipId) };
}

export function moveChip(
  pref: ToolbarPref,
  id: ChipId,
  zone: ToolbarZone,
  index: number,
): ToolbarPref {
  const from = zoneOf(pref, id);
  if (!from) return pref;

  const source = pref[from].filter((chip) => chip !== id);
  const target = from === zone ? source : [...pref[zone]];
  const clamped = Math.max(0, Math.min(index, target.length));
  target.splice(clamped, 0, id);

  if (from === zone && target.every((chip, i) => chip === pref[zone][i])) {
    return pref;
  }

  return {
    ...pref,
    [from]: from === zone ? target : source,
    [zone]: target,
  };
}
