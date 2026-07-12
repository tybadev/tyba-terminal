import { describe, expect, test } from "bun:test";

import type { ForgeStatus, PullRequest } from "./ipc";
import {
  shouldShowPrIcon,
  sortPullRequestsByNumberDesc,
  toPrStatus,
} from "./prPanel";

const status = (over: Partial<ForgeStatus> = {}): ForgeStatus => ({
  kind: "github",
  cli: "gh",
  installed: true,
  authenticated: true,
  web_create_url: null,
  ...over,
});

const pr = (over: Partial<PullRequest> = {}): PullRequest => ({
  number: 1,
  title: "t",
  url: "https://github.com/tybadev/tyba-terminal/pull/1",
  state: "open",
  checks: [],
  ...over,
});

describe("shouldShowPrIcon", () => {
  test("shows when the repo has a detected forge", () => {
    expect(shouldShowPrIcon(status())).toBe(true);
  });

  test("hides when there is no forge (null)", () => {
    expect(shouldShowPrIcon(null)).toBe(false);
  });

  test("hides while the status has not loaded yet (undefined)", () => {
    expect(shouldShowPrIcon(undefined)).toBe(false);
  });

  test("shows even if the cli is not installed/authenticated yet — that's surfaced in the panel", () => {
    expect(shouldShowPrIcon(status({ installed: false, authenticated: false }))).toBe(
      true,
    );
  });
});

describe("sortPullRequestsByNumberDesc", () => {
  test("orders the most recent PR first", () => {
    const result = sortPullRequestsByNumberDesc([
      pr({ number: 3 }),
      pr({ number: 10 }),
      pr({ number: 1 }),
    ]);
    expect(result.map((p) => p.number)).toEqual([10, 3, 1]);
  });

  test("does not mutate the input array", () => {
    const input = [pr({ number: 1 }), pr({ number: 2 })];
    const result = sortPullRequestsByNumberDesc(input);
    expect(result).not.toBe(input);
    expect(input.map((p) => p.number)).toEqual([1, 2]);
  });

  test("handles an empty list", () => {
    expect(sortPullRequestsByNumberDesc([])).toEqual([]);
  });
});

describe("toPrStatus", () => {
  test("passes through known states", () => {
    expect(toPrStatus("draft")).toBe("draft");
    expect(toPrStatus("open")).toBe("open");
    expect(toPrStatus("merged")).toBe("merged");
    expect(toPrStatus("closed")).toBe("closed");
  });

  test("falls back to open for an unknown/forge-specific state", () => {
    expect(toPrStatus("locked")).toBe("open");
    expect(toPrStatus("")).toBe("open");
  });
});
