import type { RepoSnapshot } from "./ipc";

export function snapshotForDir(
  snapshots: Record<string, RepoSnapshot>,
  dir: string,
): RepoSnapshot | undefined {
  let best: RepoSnapshot | undefined;
  let bestLength = -1;
  for (const root of Object.keys(snapshots)) {
    if (root.length <= bestLength) continue;
    if (dir === root || dir.startsWith(root.endsWith("/") ? root : root + "/")) {
      best = snapshots[root];
      bestLength = root.length;
    }
  }
  return best;
}

const CHIP_IDS = [
  "cwd",
  "branch",
  "diffCount",
  "aheadBehind",
  "clock",
] as const;

export type ChipId = (typeof CHIP_IDS)[number];

export interface ToolbarPref {
  version: number;
  enabled: boolean;
  left: ChipId[];
  right: ChipId[];
  hidden: ChipId[];
}

export const DEFAULT_TOOLBAR: ToolbarPref = {
  version: 1,
  enabled: true,
  left: ["diffCount"],
  right: ["aheadBehind", "branch", "cwd", "clock"],
  hidden: [],
};

function chipList(value: unknown): ChipId[] {
  if (!Array.isArray(value)) return [];
  return value.filter((id): id is ChipId =>
    CHIP_IDS.includes(id as ChipId),
  );
}

export function parseToolbarPref(raw: string | null): ToolbarPref {
  if (!raw) return DEFAULT_TOOLBAR;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return DEFAULT_TOOLBAR;
  }
  if (typeof parsed !== "object" || parsed === null) return DEFAULT_TOOLBAR;
  const pref = parsed as Record<string, unknown>;
  const version = typeof pref.version === "number" ? pref.version : 0;
  if (version > DEFAULT_TOOLBAR.version) return DEFAULT_TOOLBAR;
  return {
    version: DEFAULT_TOOLBAR.version,
    enabled: typeof pref.enabled === "boolean" ? pref.enabled : true,
    left: chipList(pref.left),
    right: chipList(pref.right),
    hidden: chipList(pref.hidden),
  };
}
