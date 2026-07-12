import { describe, expect, test } from "bun:test";

import type { SessionGitStatus } from "./ipc";
import { shouldShowGitIcon } from "./headerGit";

const status = (over: Partial<SessionGitStatus> = {}): SessionGitStatus => ({
  root: "/repo",
  branch: "main",
  dirty: true,
  ...over,
});

describe("shouldShowGitIcon", () => {
  test("shows when the session is a git repo with changes", () => {
    expect(shouldShowGitIcon(status({ dirty: true }))).toBe(true);
  });

  test("hides when the repo is clean", () => {
    expect(shouldShowGitIcon(status({ dirty: false }))).toBe(false);
  });

  test("hides when the session is not a git repo (null)", () => {
    expect(shouldShowGitIcon(null)).toBe(false);
  });

  test("hides while the status has not loaded yet (undefined)", () => {
    expect(shouldShowGitIcon(undefined)).toBe(false);
  });
});
