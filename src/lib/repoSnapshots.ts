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

/**
 * Onde uma ação de repositório da sessão vai cair — o alvo real do checkout.
 *
 * Espelha `session_repo_context` no core, na mesma ordem: worktree primeiro,
 * cwd do processo depois.
 *
 * > [!warning] Não é o repositório do WORKSPACE. `workspaceGitDir` cai no cwd
 * > de qualquer sessão folha, e no fim em `repo_root` — que, para uma sessão de
 * > agente, é o repo principal, não o worktree dela. Com as duas fontes
 * > misturadas, a barra dizia `main` do repo principal enquanto o seletor
 * > trocava de branch dentro do worktree do agente: rótulo e alvo em
 * > repositórios diferentes, sem nada na tela denunciando.
 *
 * `null` onde o core também falharia: sem worktree e sem processo vivo não há
 * cwd para ler, e o seletor abriria só para dizer que a sessão não tem um
 * processo ativo.
 */
export function sessionRepoDir(input: {
  worktree: string | null;
  alive: boolean;
  cwd: string | null;
}): string | null {
  if (input.worktree) return input.worktree;
  if (!input.alive) return null;
  return input.cwd;
}

const CHIP_IDS = [
  "cwd",
  "branch",
  "diffCount",
  "reviewDiff",
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
  left: ["diffCount", "reviewDiff"],
  right: ["aheadBehind", "branch", "cwd", "clock"],
  hidden: [],
};

function chipList(value: unknown, fallback: ChipId[]): ChipId[] {
  if (!Array.isArray(value)) return fallback;
  return value.filter((id): id is ChipId => CHIP_IDS.includes(id as ChipId));
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
  const seen = new Set<ChipId>();
  const dedup = (list: ChipId[]): ChipId[] =>
    list.filter((id) => (seen.has(id) ? false : (seen.add(id), true)));
  const left = dedup(chipList(pref.left, DEFAULT_TOOLBAR.left));
  const right = dedup(chipList(pref.right, DEFAULT_TOOLBAR.right));
  const hidden = dedup(chipList(pref.hidden, DEFAULT_TOOLBAR.hidden));
  // Chip novo que a pref salva ainda não conhece entra na zona onde ele
  // mora por default (pref antiga não pode esconder feature nova).
  for (const id of CHIP_IDS) {
    if (seen.has(id)) continue;
    const zone = DEFAULT_TOOLBAR.left.includes(id)
      ? left
      : DEFAULT_TOOLBAR.hidden.includes(id)
        ? hidden
        : right;
    zone.push(id);
    seen.add(id);
  }
  return {
    version: DEFAULT_TOOLBAR.version,
    enabled: typeof pref.enabled === "boolean" ? pref.enabled : true,
    left,
    right,
    hidden,
  };
}
