import { describe, expect, test } from "bun:test";

import type { GraphRow, LogRow, RefLabel } from "./ipc";
import {
  appendPage,
  edgePath,
  entriesOf,
  graphColor,
  graphColumnWidth,
  historyKey,
  laneX,
  MAX_DRAWN_LANES,
  chipLabel,
  chipTone,
  cleanFilterText,
  initials,
  relativeTime,
  secondsSince,
  shouldLoadMore,
} from "./gitHistory";

const NOW = Date.parse("2026-10-12T15:00:00Z");
const ago = (ms: number) => new Date(NOW - ms).toISOString();

const row = (sha: string): LogRow => ({
  sha,
  parents: [],
  author: "a",
  authored_at: "2026-10-12T15:00:00Z",
  refs: [],
  subject: sha,
});

const ref = (name: string, kind: RefLabel["kind"], is_head = false): RefLabel => ({
  name,
  kind,
  is_head,
});

describe("relativeTime", () => {
  test("escala de agora a data", () => {
    expect(relativeTime(ago(10_000), NOW)).toEqual({ unit: "now" });
    expect(relativeTime(ago(4 * 60_000), NOW)).toEqual({ unit: "min", count: 4 });
    expect(relativeTime(ago(90 * 60_000), NOW)).toEqual({ unit: "hour", count: 1 });
    expect(relativeTime(ago(30 * 3600_000), NOW)).toEqual({ unit: "yesterday" });
    expect(relativeTime(ago(3 * 86400_000), NOW)).toEqual({ unit: "days", count: 3 });
    expect(relativeTime(ago(30 * 86400_000), NOW).unit).toBe("date");
  });

  test("data do futuro (relógio torto) e lixo não quebram", () => {
    expect(relativeTime(new Date(NOW + 60_000).toISOString(), NOW)).toEqual({ unit: "now" });
    expect(relativeTime("lixo", NOW)).toEqual({ unit: "now" });
  });
});

describe("chips de ref", () => {
  test("cada tipo tem seu tom; agente é âmbar mesmo sendo HEAD", () => {
    expect(chipTone(ref("main", "branch", true))).toBe("head");
    expect(chipTone(ref("HEAD", "head", true))).toBe("head");
    expect(chipTone(ref("feat/x", "branch"))).toBe("local");
    expect(chipTone(ref("origin/main", "remote"))).toBe("remote");
    expect(chipTone(ref("v1", "tag"))).toBe("tag");
    expect(chipTone(ref("tyba/fix-x", "agent", true))).toBe("agent");
  });

  test("HEAD aponta para a branch; HEAD destacado é só HEAD", () => {
    expect(chipLabel(ref("main", "branch", true))).toBe("HEAD → main");
    expect(chipLabel(ref("HEAD", "head", true))).toBe("HEAD");
    expect(chipLabel(ref("feat/x", "branch"))).toBe("feat/x");
  });
});

test("iniciais do autor", () => {
  expect(initials("Guilherme Teodoro dos Santos")).toBe("GS");
  expect(initials("ana")).toBe("AN");
  expect(initials("  ")).toBe("?");
});

const entry = (sha: string) => ({ row: row(sha), graph: null });

test("página nova entra no fim sem repetir commit", () => {
  const merged = appendPage([entry("a"), entry("b")], [entry("b"), entry("c")]);
  expect(merged.map((e) => e.row.sha)).toEqual(["a", "b", "c"]);
});

describe("grafo", () => {
  const g = (width: number): GraphRow => ({ lane: 0, color: 1, edges: [], width });

  test("página casa cada commit com a linha de grafo dele; filtrada fica sem", () => {
    const page = { rows: [row("a"), row("b")], cursor: null, filtered: false, graph: [g(1), g(2)] };
    expect(entriesOf(page).map((e) => e.graph?.width)).toEqual([1, 2]);
    const filtered = { ...page, filtered: true, graph: [] };
    expect(entriesOf(filtered).every((e) => e.graph === null)).toBe(true);
  });

  test("cor 0 é do agente e as outras giram sem nunca cair nela", () => {
    expect(graphColor(0)).toBe("var(--tyba-amber)");
    expect(graphColor(1)).toBe("var(--tyba-blue)");
    expect(graphColor(8)).toBe(graphColor(1));
    for (let i = 1; i < 40; i++) expect(graphColor(i)).not.toBe(graphColor(0));
  });

  test("largura segue a linha mais larga, com teto", () => {
    expect(graphColumnWidth([])).toBe(28);
    expect(graphColumnWidth([{ row: row("a"), graph: g(3) }])).toBe(laneX(2) + 14);
    expect(graphColumnWidth([{ row: row("a"), graph: g(400) }])).toBe(
      laneX(MAX_DRAWN_LANES - 1) + 14,
    );
  });

  test("arestas: reta na própria coluna, curva para outra", () => {
    expect(edgePath({ kind: "pass", lane: 1, color: 1 }, 0, 30)).toBe("M30 0V30");
    expect(edgePath({ kind: "in", from: 0, color: 1 }, 0, 30)).toBe("M14 0V15");
    expect(edgePath({ kind: "out", to: 0, color: 1 }, 0, 30)).toBe("M14 15V30");
    const merge = edgePath({ kind: "out", to: 1, color: 2 }, 0, 30);
    expect(merge.startsWith("M14 15C")).toBe(true);
    expect(merge.endsWith("30 30")).toBe(true);
    const fork = edgePath({ kind: "in", from: 2, color: 2 }, 0, 30);
    expect(fork.startsWith("M46 0C")).toBe(true);
    expect(fork.endsWith("14 15")).toBe(true);
  });
});

describe("shouldLoadMore", () => {
  const base = { count: 200, hasMore: true, loading: false };
  test("só perto do fim, com mais página e nada em voo", () => {
    expect(shouldLoadMore({ ...base, lastVisible: 100 })).toBe(false);
    expect(shouldLoadMore({ ...base, lastVisible: 170 })).toBe(true);
    expect(shouldLoadMore({ ...base, lastVisible: 199, loading: true })).toBe(false);
    expect(shouldLoadMore({ ...base, lastVisible: 199, hasMore: false })).toBe(false);
  });
});

test("atualizado há Ns e texto de filtro", () => {
  expect(secondsSince(null, NOW)).toBeNull();
  expect(secondsSince(NOW - 12_400, NOW)).toBe(12);
  expect(cleanFilterText("  ")).toBeNull();
  expect(cleanFilterText(" ana ")).toBe("ana");
});

describe("historyKey", () => {
  test("↑/↓ andam presos nas pontas; sem foco, começam do topo", () => {
    expect(historyKey({ key: "ArrowDown" }, -1, 5)).toEqual({ kind: "move", index: 0 });
    expect(historyKey({ key: "ArrowDown" }, 2, 5)).toEqual({ kind: "move", index: 3 });
    expect(historyKey({ key: "ArrowDown" }, 4, 5)).toEqual({ kind: "move", index: 4 });
    expect(historyKey({ key: "ArrowUp" }, 0, 5)).toEqual({ kind: "move", index: 0 });
    expect(historyKey({ key: "ArrowUp" }, 3, 5)).toEqual({ kind: "move", index: 2 });
  });

  test("Enter abre e c copia só com uma linha em foco", () => {
    expect(historyKey({ key: "Enter" }, 1, 5)).toEqual({ kind: "open" });
    expect(historyKey({ key: "c" }, 1, 5)).toEqual({ kind: "copy" });
    expect(historyKey({ key: "Enter" }, -1, 5)).toBeNull();
    expect(historyKey({ key: "c" }, -1, 5)).toBeNull();
  });

  test("⌘C, lista vazia e outras teclas não são daqui", () => {
    expect(historyKey({ key: "c", metaKey: true }, 1, 5)).toBeNull();
    expect(historyKey({ key: "ArrowDown" }, -1, 0)).toBeNull();
    expect(historyKey({ key: "x" }, 1, 5)).toBeNull();
  });
});
