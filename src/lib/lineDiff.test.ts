import { describe, expect, it } from "bun:test";

import { hasLineDifferences, lineDiff } from "./lineDiff";

describe("lineDiff", () => {
  it("marca linhas iguais como equal", () => {
    const rows = lineDiff("a\nb", "a\nb");
    expect(rows.map((r) => r.kind)).toEqual(["equal", "equal"]);
    expect(hasLineDifferences(rows)).toBe(false);
  });

  it("aponta linha adicionada só no lado editado", () => {
    const rows = lineDiff("a\nc", "a\nb\nc");
    expect(rows).toEqual([
      { kind: "equal", text: "a" },
      { kind: "added", text: "b" },
      { kind: "equal", text: "c" },
    ]);
    expect(hasLineDifferences(rows)).toBe(true);
  });

  it("aponta linha removida só no disco", () => {
    const rows = lineDiff("a\nb\nc", "a\nc");
    expect(rows).toEqual([
      { kind: "equal", text: "a" },
      { kind: "removed", text: "b" },
      { kind: "equal", text: "c" },
    ]);
  });

  it("representa uma substituição como removed + added", () => {
    const rows = lineDiff("old", "new");
    expect(rows).toEqual([
      { kind: "removed", text: "old" },
      { kind: "added", text: "new" },
    ]);
  });
});
