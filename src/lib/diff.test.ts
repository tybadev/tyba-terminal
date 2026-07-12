import { describe, expect, test } from "bun:test";

import type { FileDiff, FileHunks, Hunk } from "./ipc";
import {
  buildReviewPrompt,
  buildRows,
  defaultCollapsed,
  fileKeyOf,
  isGeneratedPath,
  isSectionOpen,
  langOfPath,
  numberHunk,
  splitPairs,
  toggleSection,
} from "./diff";

const hunk: Hunk = {
  old_start: 10,
  old_lines: 3,
  new_start: 10,
  new_lines: 4,
  header: "@@ -10,3 +10,4 @@",
  lines: [
    { kind: "context", text: "a" },
    { kind: "del", text: "velha" },
    { kind: "add", text: "nova" },
    { kind: "add", text: "extra" },
    { kind: "context", text: "z" },
  ],
};

const file = (path: string): FileDiff => ({
  path,
  old_path: null,
  status: "modified",
  insertions: 1,
  deletions: 1,
});

describe("isGeneratedPath", () => {
  test("lockfiles e minificados são gerados", () => {
    expect(isGeneratedPath("bun.lockb")).toBe(true);
    expect(isGeneratedPath("sub/Cargo.lock")).toBe(true);
    expect(isGeneratedPath("app.min.js")).toBe(true);
    expect(isGeneratedPath("src/main.rs")).toBe(false);
    expect(isGeneratedPath("unlock.rs")).toBe(false);
  });
});

describe("numberHunk", () => {
  test("numera velho e novo respeitando add/del", () => {
    const rows = numberHunk(hunk);
    expect(rows[0]).toMatchObject({ oldNo: 10, newNo: 10 });
    expect(rows[1]).toMatchObject({ oldNo: 11, newNo: null });
    expect(rows[2]).toMatchObject({ oldNo: null, newNo: 11 });
    expect(rows[3]).toMatchObject({ oldNo: null, newNo: 12 });
    expect(rows[4]).toMatchObject({ oldNo: 12, newNo: 13 });
  });
});

describe("splitPairs", () => {
  test("alinha del com add lado a lado e contexto espelhado", () => {
    const pairs = splitPairs(hunk);
    expect(pairs[0].left).toMatchObject({ kind: "context", no: 10 });
    expect(pairs[0].right).toMatchObject({ kind: "context", no: 10 });
    expect(pairs[1].left).toMatchObject({ kind: "del", text: "velha" });
    expect(pairs[1].right).toMatchObject({ kind: "add", text: "nova" });
    expect(pairs[2].left).toBeNull();
    expect(pairs[2].right).toMatchObject({ kind: "add", text: "extra" });
  });
});

describe("defaultCollapsed", () => {
  test("gerado colapsa; diff gigante colapsa; comum não", () => {
    expect(defaultCollapsed(file("bun.lockb"), undefined)).toBe(true);
    const big: FileHunks = {
      binary: false,
      hunks: [
        {
          ...hunk,
          lines: Array.from({ length: 900 }, () => ({
            kind: "add" as const,
            text: "x",
          })),
        },
      ],
    };
    expect(defaultCollapsed(file("grande.rs"), big)).toBe(true);
    expect(defaultCollapsed(file("normal.rs"), { binary: false, hunks: [hunk] })).toBe(
      false,
    );
  });
});

describe("buildRows", () => {
  test("seções, arquivo colapsado sem linhas, expandido com hunks", () => {
    const hunks: FileHunks = { binary: false, hunks: [hunk] };
    const rows = buildRows({
      committed: [file("src/a.rs"), file("bun.lockb")],
      staged: [file("pronto.txt")],
      unstaged: [file("wip.txt")],
      hunksByKey: { [fileKeyOf("committed", "src/a.rs")]: hunks },
      collapsedByKey: {},
      mode: "unified",
    });
    const kinds = rows.map((r) => r.kind);
    expect(kinds[0]).toBe("section");
    expect(kinds).toContain("hunk");
    expect(rows.filter((r) => r.kind === "line")).toHaveLength(5);
    const lock = rows.find(
      (r) => r.kind === "file" && r.file.path === "bun.lockb",
    );
    expect(lock).toMatchObject({ collapsed: true, generated: true });
    const sections = rows.filter((r) => r.kind === "section");
    expect(sections.map((s) => s.scope)).toEqual([
      "committed",
      "staged",
      "unstaged",
    ]);
  });

  test("modo split gera pares", () => {
    const hunks: FileHunks = { binary: false, hunks: [hunk] };
    const rows = buildRows({
      committed: [file("src/a.rs")],
      staged: [],
      unstaged: [],
      hunksByKey: { [fileKeyOf("committed", "src/a.rs")]: hunks },
      collapsedByKey: {},
      mode: "split",
    });
    expect(rows.filter((r) => r.kind === "split").length).toBeGreaterThan(0);
    expect(rows.filter((r) => r.kind === "line")).toHaveLength(0);
  });

  test("expandido sem hunks carregados marca loading", () => {
    const rows = buildRows({
      committed: [file("src/a.rs")],
      staged: [],
      unstaged: [],
      hunksByKey: {},
      collapsedByKey: { [fileKeyOf("committed", "src/a.rs")]: false },
      mode: "unified",
    });
    expect(rows.some((r) => r.kind === "empty" && r.label === "loading")).toBe(
      true,
    );
  });
});

describe("isSectionOpen / toggleSection", () => {
  test("seção sem entrada no estado começa aberta", () => {
    expect(isSectionOpen({}, "staged")).toBe(true);
  });

  test("toggle fecha uma seção aberta por padrão", () => {
    const next = toggleSection({}, "staged");
    expect(isSectionOpen(next, "staged")).toBe(false);
  });

  test("toggle reabre uma seção fechada", () => {
    const closed = toggleSection({}, "unstaged");
    const reopened = toggleSection(closed, "unstaged");
    expect(isSectionOpen(reopened, "unstaged")).toBe(true);
  });

  test("toggle de uma seção não mexe nas outras", () => {
    const next = toggleSection({}, "committed");
    expect(isSectionOpen(next, "committed")).toBe(false);
    expect(isSectionOpen(next, "staged")).toBe(true);
  });
});

describe("langOfPath", () => {
  test("mapeia extensões conhecidas e devolve null pro resto", () => {
    expect(langOfPath("src/main.rs")).toBe("rust");
    expect(langOfPath("App.tsx")).toBe("tsx");
    expect(langOfPath("LICENSE")).toBeNull();
  });
});

describe("buildReviewPrompt", () => {
  test("formata arquivo:linha com trecho e instrução final", () => {
    const prompt = buildReviewPrompt("abc1234", [
      {
        path: "src/main.rs",
        oldNo: null,
        newNo: 12,
        excerpt: "let total = soma(1, 2) * 2;",
        text: "extrai esse 2 pra uma constante",
      },
      {
        path: "src/velho.rs",
        oldNo: 40,
        newNo: null,
        excerpt: "fn antiga()",
        text: "essa função não podia ter sumido",
      },
    ]);
    expect(prompt).toContain("base abc1234..HEAD");
    expect(prompt).toContain("src/main.rs:12");
    expect(prompt).toContain("> let total = soma(1, 2) * 2;");
    expect(prompt).toContain("linha removida 40");
    expect(prompt).toContain("2 comentário(s)");
  });
});
