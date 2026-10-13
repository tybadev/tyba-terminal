import { describe, expect, test } from "bun:test";

import { runOutcome, runTime, shortCwd } from "./historyPanel";

describe("historyPanel", () => {
  test("exit code vira ✓, ✗ código ou desconhecido", () => {
    expect(runOutcome(0)).toEqual({ kind: "ok" });
    expect(runOutcome(127)).toEqual({ kind: "failed", code: 127 });
    expect(runOutcome(null)).toEqual({ kind: "unknown" });
    expect(runOutcome(undefined)).toEqual({ kind: "unknown" });
  });

  test("hora: só HH:MM no mesmo dia, com data em outro", () => {
    const now = new Date(2026, 9, 12, 15, 30).getTime();
    expect(runTime(new Date(2026, 9, 12, 9, 5).getTime(), now, "pt-BR")).toBe("09:05");
    expect(runTime(new Date(2026, 9, 11, 23, 59).getTime(), now, "pt-BR")).toBe(
      "11/10 23:59",
    );
  });

  test("cwd curto relativo ao repo, ou os dois últimos níveis", () => {
    expect(shortCwd("/u/g/poc-git/src", "/u/g/poc-git")).toBe("poc-git/src");
    expect(shortCwd("/u/g/poc-git", "/u/g/poc-git/")).toBe("poc-git");
    expect(shortCwd("/u/g/poc-git-2/x", "/u/g/poc-git")).toBe("…/poc-git-2/x");
    expect(shortCwd("/tmp", null)).toBe("/tmp");
    expect(shortCwd(null)).toBe("");
  });
});
