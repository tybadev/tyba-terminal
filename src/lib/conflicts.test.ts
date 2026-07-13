import { describe, expect, test } from "bun:test";

import type { ConflictState } from "./ipc";
import { buildConflictPrompt } from "./conflicts";

const state = (over: Partial<ConflictState> = {}): ConflictState => ({
  root: "/repo",
  operation: "merge",
  ours: "main",
  theirs: "feature",
  files: [
    { path: "src/a.ts", kind: "UU" },
    { path: "docs/um doc.md", kind: "AA" },
  ],
  ...over,
});

describe("buildConflictPrompt", () => {
  test("merge lists the files, the sides and stops before the commit", () => {
    const prompt = buildConflictPrompt(state());
    expect(prompt).toContain("merge em andamento (main ← feature)");
    expect(prompt).toContain("2 arquivo(s)");
    expect(prompt).toContain("- src/a.ts (UU)");
    expect(prompt).toContain("- docs/um doc.md (AA)");
    expect(prompt).toContain("não conclua o commit do merge");
    expect(prompt).not.toContain("--continue");
  });

  test("rebase instructs git rebase --continue instead of stopping", () => {
    const prompt = buildConflictPrompt(state({ operation: "rebase" }));
    expect(prompt).toContain("rebase em andamento");
    expect(prompt).toContain("git rebase --continue");
    expect(prompt).not.toContain("não conclua o commit do merge");
  });

  test("cherry-pick instructs git cherry-pick --continue", () => {
    const prompt = buildConflictPrompt(state({ operation: "cherry_pick" }));
    expect(prompt).toContain("cherry-pick em andamento");
    expect(prompt).toContain("git cherry-pick --continue");
  });

  test("omits the sides when a label is missing", () => {
    const prompt = buildConflictPrompt(state({ theirs: null }));
    expect(prompt).toContain("merge em andamento com");
    expect(prompt).not.toContain("(main");
  });
});
