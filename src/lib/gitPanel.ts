import type { FileDiff, FileStatus, SessionDiff } from "./ipc";
import { isSensitivePath } from "./sensitivePaths";

/** Uma linha da lista de alterações do painel Git (staged + não staged). */
export type GitChangeRow = {
  path: string;
  letter: "A" | "M" | "D" | "R" | "C" | "T" | "U" | "?";
  tone: "green" | "amber" | "red" | "muted";
  insertions: number;
  deletions: number;
  sensitive: boolean;
};

const LETTER: Record<FileStatus, GitChangeRow["letter"]> = {
  added: "A",
  modified: "M",
  deleted: "D",
  renamed: "R",
  copied: "C",
  type_changed: "T",
  unmerged: "U",
  other: "?",
};

const TONE: Record<FileStatus, GitChangeRow["tone"]> = {
  added: "green",
  modified: "amber",
  deleted: "red",
  renamed: "amber",
  copied: "green",
  type_changed: "amber",
  unmerged: "red",
  other: "muted",
};

const row = (file: FileDiff, prev?: GitChangeRow): GitChangeRow => ({
  path: file.path,
  letter: LETTER[file.status],
  tone: TONE[file.status],
  insertions: (prev?.insertions ?? 0) + (file.insertions ?? 0),
  deletions: (prev?.deletions ?? 0) + (file.deletions ?? 0),
  sensitive: isSensitivePath(file.path),
});

/**
 * Separa o que o git já rastreia (preparado ou não, numa linha só, +/−
 * somados) do que é untracked — este nunca entra num commit sem o usuário
 * marcar.
 */
export function gitChangeRows(diff: SessionDiff | null): {
  rows: GitChangeRow[];
  untracked: GitChangeRow[];
  insertions: number;
  deletions: number;
} {
  if (!diff) return { rows: [], untracked: [], insertions: 0, deletions: 0 };
  const isUntracked = new Set(diff.untracked);
  const byPath = new Map<string, GitChangeRow>();
  const untracked: GitChangeRow[] = [];
  for (const file of diff.staged_files) byPath.set(file.path, row(file));
  for (const file of diff.unstaged_files) {
    if (isUntracked.has(file.path)) untracked.push(row(file));
    else byPath.set(file.path, row(file, byPath.get(file.path)));
  }
  const byName = (a: GitChangeRow, b: GitChangeRow) => a.path.localeCompare(b.path);
  const rows = [...byPath.values()].sort(byName);
  untracked.sort(byName);
  const all = [...rows, ...untracked];
  return {
    rows,
    untracked,
    insertions: all.reduce((n, r) => n + r.insertions, 0),
    deletions: all.reduce((n, r) => n + r.deletions, 0),
  };
}

/**
 * O que o botão Commit faz.
 *
 * Com algo preparado, commita o preparado (como o Diff). Sem nada preparado,
 * prepara só o que o git já rastreia — semântica de `git add -u`, incluindo o
 * caminho antigo de um rename. Untracked só entra se estiver em `chosen`, que
 * é a escolha explícita do usuário, nos dois modos.
 */
export function commitPlan(
  diff: SessionDiff | null,
  chosen: ReadonlySet<string> = new Set(),
): { mode: "staged" | "tracked" | "nothing"; stage: string[] } {
  if (!diff) return { mode: "nothing", stage: [] };
  const isUntracked = new Set(diff.untracked);
  const picked = diff.untracked.filter((p) => chosen.has(p));
  if (diff.staged_files.length > 0) return { mode: "staged", stage: picked };
  const paths = new Set<string>();
  for (const file of diff.unstaged_files) {
    if (isUntracked.has(file.path)) continue;
    paths.add(file.path);
    if (file.old_path) paths.add(file.old_path);
  }
  for (const p of picked) paths.add(p);
  if (paths.size === 0) return { mode: "nothing", stage: [] };
  return { mode: "tracked", stage: [...paths] };
}

/**
 * Marcar um untracked. Arquivo que parece segredo só entra com confirmação
 * explícita (`confirmed`); desmarcar é sempre livre.
 */
export function toggleUntracked(
  chosen: ReadonlySet<string>,
  path: string,
  confirmed: boolean,
): { chosen: Set<string>; needsConfirm: boolean } {
  const next = new Set(chosen);
  if (next.has(path)) {
    next.delete(path);
    return { chosen: next, needsConfirm: false };
  }
  if (isSensitivePath(path) && !confirmed) {
    return { chosen: next, needsConfirm: true };
  }
  next.add(path);
  return { chosen: next, needsConfirm: false };
}

/** Escolha que não existe mais (arquivo commitado ou apagado) sai. */
export function pruneChosen(
  chosen: ReadonlySet<string>,
  diff: SessionDiff | null,
): Set<string> {
  const alive = new Set(diff?.untracked ?? []);
  return new Set([...chosen].filter((p) => alive.has(p)));
}

/** Linha "à frente / atrás" do remoto, como chave de i18n + contagens. */
export function upstreamLine(
  ahead: number | null | undefined,
  behind: number | null | undefined,
): { key: string; ahead: number; behind: number } {
  if (ahead == null || behind == null) {
    return { key: "gitPanelNoUpstream", ahead: 0, behind: 0 };
  }
  if (ahead === 0 && behind === 0) {
    return { key: "gitPanelUpToDate", ahead, behind };
  }
  if (behind === 0) return { key: "gitPanelAhead", ahead, behind };
  if (ahead === 0) return { key: "gitPanelBehind", ahead, behind };
  return { key: "gitPanelAheadBehind", ahead, behind };
}
