import type { DiffLine, FileDiff, FileHunks, Hunk } from "./ipc";

export type DiffMode = "unified" | "split";

export type DiffScopeKey = "committed" | "staged" | "unstaged";

export interface FileKey {
  scope: DiffScopeKey;
  path: string;
}

export const fileKeyOf = (scope: DiffScopeKey, path: string): string =>
  `${scope}\u0000${path}`;

export const keyScope = (key: string): string =>
  key.slice(0, key.indexOf("\u0000"));

export const keyPath = (key: string): string =>
  key.slice(key.indexOf("\u0000") + 1);

const GENERATED_PATTERNS: RegExp[] = [
  /(^|\/)bun\.lockb?$/,
  /(^|\/)package-lock\.json$/,
  /(^|\/)pnpm-lock\.yaml$/,
  /(^|\/)yarn\.lock$/,
  /(^|\/)Cargo\.lock$/,
  /(^|\/)composer\.lock$/,
  /(^|\/)Gemfile\.lock$/,
  /(^|\/)poetry\.lock$/,
  /(^|\/)go\.sum$/,
  /\.min\.(js|css)$/,
  /(^|\/)dist\//,
  /(^|\/)node_modules\//,
  /\.snap$/,
];

export function isGeneratedPath(path: string): boolean {
  return GENERATED_PATTERNS.some((re) => re.test(path));
}

export const BIG_DIFF_LINES = 800;

export type Row =
  | { kind: "section"; scope: DiffScopeKey; count: number }
  | {
      kind: "file";
      key: string;
      scope: DiffScopeKey;
      file: FileDiff;
      collapsed: boolean;
      generated: boolean;
      loading: boolean;
      binary: boolean;
    }
  | { kind: "hunk"; key: string; header: string }
  | {
      kind: "line";
      key: string;
      path: string;
      hunkKey: string;
      idx: number;
      line: DiffLine;
      oldNo: number | null;
      newNo: number | null;
    }
  | {
      kind: "split";
      key: string;
      path: string;
      hunkKey: string;
      left: SplitSide | null;
      right: SplitSide | null;
    }
  | { kind: "empty"; key: string; label: "binary" | "loading" | "noChanges" };

interface UnifiedNumbered {
  line: DiffLine;
  idx: number;
  oldNo: number | null;
  newNo: number | null;
}

export function numberHunk(hunk: Hunk): UnifiedNumbered[] {
  let oldNo = hunk.old_start;
  let newNo = hunk.new_start;
  return hunk.lines.map((line, idx) => {
    const entry: UnifiedNumbered = {
      line,
      idx,
      oldNo: line.kind !== "add" ? oldNo : null,
      newNo: line.kind !== "del" ? newNo : null,
    };
    if (line.kind !== "add") oldNo += 1;
    if (line.kind !== "del") newNo += 1;
    return entry;
  });
}

export interface SplitSide {
  text: string;
  no: number;
  kind: "del" | "add" | "context";
  idx: number;
}

export interface SplitPair {
  left: SplitSide | null;
  right: SplitSide | null;
}

export function splitPairs(hunk: Hunk): SplitPair[] {
  const numbered = numberHunk(hunk);
  const pairs: SplitPair[] = [];
  let pendingDels: UnifiedNumbered[] = [];
  let pendingAdds: UnifiedNumbered[] = [];

  const flush = () => {
    const rows = Math.max(pendingDels.length, pendingAdds.length);
    for (let i = 0; i < rows; i += 1) {
      const del = pendingDels[i];
      const add = pendingAdds[i];
      pairs.push({
        left: del
          ? {
              text: del.line.text,
              no: del.oldNo as number,
              kind: "del",
              idx: del.idx,
            }
          : null,
        right: add
          ? {
              text: add.line.text,
              no: add.newNo as number,
              kind: "add",
              idx: add.idx,
            }
          : null,
      });
    }
    pendingDels = [];
    pendingAdds = [];
  };

  for (const entry of numbered) {
    if (entry.line.kind === "del") {
      pendingDels.push(entry);
    } else if (entry.line.kind === "add") {
      pendingAdds.push(entry);
    } else {
      flush();
      pairs.push({
        left: {
          text: entry.line.text,
          no: entry.oldNo as number,
          kind: "context",
          idx: entry.idx,
        },
        right: {
          text: entry.line.text,
          no: entry.newNo as number,
          kind: "context",
          idx: entry.idx,
        },
      });
    }
  }
  flush();
  return pairs;
}

export function hunkLineCount(hunks: FileHunks): number {
  return hunks.hunks.reduce((n, h) => n + h.lines.length, 0);
}

export interface BuildRowsInput {
  committed: FileDiff[];
  staged: FileDiff[];
  unstaged: FileDiff[];
  hunksByKey: Record<string, FileHunks | undefined>;
  collapsedByKey: Record<string, boolean | undefined>;
  mode: DiffMode;
}

export function defaultCollapsed(
  file: FileDiff,
  hunks: FileHunks | undefined,
): boolean {
  if (isGeneratedPath(file.path)) return true;
  if (hunks && hunkLineCount(hunks) > BIG_DIFF_LINES) return true;
  return false;
}

export function buildRows(input: BuildRowsInput): Row[] {
  const rows: Row[] = [];
  const sections: Array<{ scope: DiffScopeKey; files: FileDiff[] }> = [
    { scope: "committed", files: input.committed },
    { scope: "staged", files: input.staged },
    { scope: "unstaged", files: input.unstaged },
  ];

  for (const section of sections) {
    if (section.files.length === 0) continue;
    rows.push({
      kind: "section",
      scope: section.scope,
      count: section.files.length,
    });
    for (const file of section.files) {
      const key = fileKeyOf(section.scope, file.path);
      const hunks = input.hunksByKey[key];
      const generated = isGeneratedPath(file.path);
      const collapsed =
        input.collapsedByKey[key] ?? defaultCollapsed(file, hunks);
      rows.push({
        kind: "file",
        key,
        scope: section.scope,
        file,
        collapsed,
        generated,
        loading: !collapsed && hunks === undefined,
        binary: hunks?.binary ?? false,
      });
      if (collapsed) continue;
      if (hunks === undefined) {
        rows.push({ kind: "empty", key: `${key}:loading`, label: "loading" });
        continue;
      }
      if (hunks.binary) {
        rows.push({ kind: "empty", key: `${key}:binary`, label: "binary" });
        continue;
      }
      if (hunks.hunks.length === 0) {
        rows.push({ kind: "empty", key: `${key}:none`, label: "noChanges" });
        continue;
      }
      for (const [hi, hunk] of hunks.hunks.entries()) {
        const hunkKey = `${key}:h${hi}`;
        rows.push({ kind: "hunk", key: hunkKey, header: hunk.header });
        if (input.mode === "unified") {
          for (const [li, entry] of numberHunk(hunk).entries()) {
            rows.push({
              kind: "line",
              key: `${hunkKey}:l${li}`,
              path: file.path,
              hunkKey,
              idx: entry.idx,
              line: entry.line,
              oldNo: entry.oldNo,
              newNo: entry.newNo,
            });
          }
        } else {
          for (const [pi, pair] of splitPairs(hunk).entries()) {
            rows.push({
              kind: "split",
              key: `${hunkKey}:p${pi}`,
              path: file.path,
              hunkKey,
              left: pair.left,
              right: pair.right,
            });
          }
        }
      }
    }
  }
  return rows;
}

const LANG_BY_EXT: Record<string, string> = {
  ts: "typescript",
  tsx: "tsx",
  js: "javascript",
  jsx: "jsx",
  mjs: "javascript",
  cjs: "javascript",
  rs: "rust",
  json: "json",
  jsonc: "jsonc",
  css: "css",
  html: "html",
  md: "markdown",
  sh: "shellscript",
  bash: "shellscript",
  zsh: "shellscript",
  toml: "toml",
  yaml: "yaml",
  yml: "yaml",
  py: "python",
  go: "go",
  sql: "sql",
  swift: "swift",
};

export function langOfPath(path: string): string | null {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return LANG_BY_EXT[ext] ?? null;
}

export interface ReviewComment {
  path: string;
  oldNo: number | null;
  newNo: number | null;
  excerpt: string;
  text: string;
}

export function buildReviewPrompt(
  baseShort: string,
  comments: ReviewComment[],
): string {
  const blocks = comments.map((c) => {
    const where =
      c.newNo !== null
        ? `${c.path}:${c.newNo}`
        : `${c.path} (linha removida${c.oldNo !== null ? ` ${c.oldNo}` : ""})`;
    return `${where}\n> ${c.excerpt}\n${c.text}`;
  });
  return [
    `Revisei o diff deste worktree (base ${baseShort}..HEAD) e deixei ${comments.length} comentário(s):`,
    "",
    blocks.join("\n\n"),
    "",
    "Aplique as mudanças pedidas nos comentários, mantendo o resto como está.",
  ].join("\n");
}
