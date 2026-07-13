import { describe, expect, test } from "bun:test";

import type { SessionGitStatus } from "./ipc";
import { gitIconTone } from "./headerGit";

const status = (over: Partial<SessionGitStatus> = {}): SessionGitStatus => ({
  root: "/repo",
  branch: "main",
  dirty: true,
  ...over,
});

describe("gitIconTone", () => {
  test("dirty repo gets the dirty tone", () => {
    expect(gitIconTone(status({ dirty: true }))).toBe("dirty");
  });

  test("clean repo still shows the icon, with the clean tone", () => {
    expect(gitIconTone(status({ dirty: false }))).toBe("clean");
  });

  test("hides when the session is not a git repo (null)", () => {
    expect(gitIconTone(null)).toBeNull();
  });

  test("hides while the status has not loaded yet (undefined)", () => {
    expect(gitIconTone(undefined)).toBeNull();
  });
});
