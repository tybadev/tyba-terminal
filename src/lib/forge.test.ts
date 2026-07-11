import { describe, expect, test } from "bun:test";

import type {
  CheckRun,
  ForgeReviewComment,
  ForgeStatus,
  MergePreview,
  PullRequest,
} from "./ipc";
import {
  buildForgeCommentPrompt,
  checkTone,
  forgeCliReady,
  mergeGate,
  overallChecksTone,
  primaryDeliveryAction,
} from "./forge";

const preview = (over: Partial<MergePreview> = {}): MergePreview => ({
  base_branch: "main",
  source_branch: "tyba/tarefa-1",
  commits: 2,
  files_changed: 3,
  conflicts: [],
  base_dirty: false,
  ...over,
});

describe("mergeGate", () => {
  test("clean preview with commits is enabled", () => {
    expect(mergeGate(preview())).toEqual({ enabled: true, reason: null });
  });

  test("dirty base blocks the merge", () => {
    expect(mergeGate(preview({ base_dirty: true }))).toEqual({
      enabled: false,
      reason: "base_dirty",
    });
  });

  test("conflicts block the merge even with a clean base", () => {
    expect(
      mergeGate(preview({ conflicts: ["f.txt"], base_dirty: false })),
    ).toEqual({ enabled: false, reason: "conflicts" });
  });

  test("nothing to merge blocks the merge", () => {
    expect(mergeGate(preview({ commits: 0 }))).toEqual({
      enabled: false,
      reason: "nothing_to_merge",
    });
  });

  test("dirty base takes priority over conflicts when both apply", () => {
    expect(
      mergeGate(preview({ base_dirty: true, conflicts: ["f.txt"] })),
    ).toEqual({ enabled: false, reason: "base_dirty" });
  });
});

describe("primaryDeliveryAction", () => {
  const status = (kind: "github" | "gitlab"): ForgeStatus => ({
    kind,
    cli: kind === "github" ? "gh" : "glab",
    installed: true,
    authenticated: true,
    web_create_url: null,
  });

  test("github forge opens a PR", () => {
    expect(primaryDeliveryAction(status("github"))).toBe("open_pr");
  });

  test("gitlab forge opens an MR", () => {
    expect(primaryDeliveryAction(status("gitlab"))).toBe("open_mr");
  });

  test("no forge detected falls back to merge only", () => {
    expect(primaryDeliveryAction(null)).toBe("merge_only");
  });
});

describe("forgeCliReady", () => {
  test("requires both installed and authenticated", () => {
    const base: ForgeStatus = {
      kind: "github",
      cli: "gh",
      installed: true,
      authenticated: true,
      web_create_url: null,
    };
    expect(forgeCliReady(base)).toBe(true);
    expect(forgeCliReady({ ...base, installed: false })).toBe(false);
    expect(forgeCliReady({ ...base, authenticated: false })).toBe(false);
  });
});

describe("checkTone / overallChecksTone", () => {
  const check = (over: Partial<CheckRun>): CheckRun => ({
    name: "build",
    status: "completed",
    conclusion: "success",
    ...over,
  });

  test("running check is pending", () => {
    expect(checkTone(check({ status: "in_progress", conclusion: null }))).toBe(
      "pending",
    );
  });

  test("completed with success conclusion is success", () => {
    expect(checkTone(check({}))).toBe("success");
  });

  test("completed with failure conclusion is failure", () => {
    expect(checkTone(check({ conclusion: "failure" }))).toBe("failure");
  });

  test("skipped/neutral count as success", () => {
    expect(checkTone(check({ conclusion: "skipped" }))).toBe("success");
    expect(checkTone(check({ conclusion: "neutral" }))).toBe("success");
  });

  test("overall tone is failure if any check failed", () => {
    expect(
      overallChecksTone([check({}), check({ conclusion: "failure" })]),
    ).toBe("failure");
  });

  test("overall tone is pending if any check is still running", () => {
    expect(
      overallChecksTone([check({}), check({ status: "queued", conclusion: null })]),
    ).toBe("pending");
  });

  test("overall tone is success when every check passed", () => {
    expect(overallChecksTone([check({}), check({})])).toBe("success");
  });

  test("no checks yields no overall tone", () => {
    expect(overallChecksTone([])).toBe(null);
  });
});

describe("buildForgeCommentPrompt", () => {
  const pr: PullRequest = {
    number: 42,
    title: "feat: entrega",
    url: "https://github.com/tybadev/tyba-terminal/pull/42",
    state: "open",
    checks: [],
  };

  const comment = (over: Partial<ForgeReviewComment>): ForgeReviewComment => ({
    id: "1",
    author: "reviewer",
    body: "ajusta isso aqui",
    path: "src/App.tsx",
    line: 12,
    url: null,
    ...over,
  });

  test("assembles a prompt from N selected comments with path:line", () => {
    const prompt = buildForgeCommentPrompt(pr, [
      comment({ id: "1" }),
      comment({ id: "2", path: "src/lib/forge.ts", line: 5, body: "renomeia" }),
    ]);
    expect(prompt).toContain("2 comentário(s)");
    expect(prompt).toContain("#42");
    expect(prompt).toContain(pr.url);
    expect(prompt).toContain("src/App.tsx:12");
    expect(prompt).toContain("src/lib/forge.ts:5");
    expect(prompt).toContain("@reviewer");
  });

  test("general comments without a path/line are labeled as such", () => {
    const prompt = buildForgeCommentPrompt(pr, [
      comment({ path: null, line: null, body: "comentário geral no PR" }),
    ]);
    expect(prompt).toContain("comentário geral");
    expect(prompt).not.toContain("null");
  });
});
