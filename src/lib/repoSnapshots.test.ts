import { describe, expect, test } from "bun:test";

import type { RepoSnapshot } from "./ipc";
import {
  DEFAULT_TOOLBAR,
  parseToolbarPref,
  snapshotForDir,
} from "./repoSnapshots";

const snap = (root: string): RepoSnapshot => ({
  root,
  branch: "main",
  status: null,
  ahead: null,
  behind: null,
});

describe("snapshotForDir", () => {
  test("exact root matches", () => {
    const s = { "/home/u/repo": snap("/home/u/repo") };
    expect(snapshotForDir(s, "/home/u/repo")?.root).toBe("/home/u/repo");
  });

  test("subdirectory matches its root", () => {
    const s = { "/home/u/repo": snap("/home/u/repo") };
    expect(snapshotForDir(s, "/home/u/repo/src/deep")?.root).toBe(
      "/home/u/repo",
    );
  });

  test("sibling with shared prefix does not match", () => {
    const s = { "/home/u/repo": snap("/home/u/repo") };
    expect(snapshotForDir(s, "/home/u/repo-other")).toBeUndefined();
  });

  test("nested roots pick the longest", () => {
    const s = {
      "/home/u/outer": snap("/home/u/outer"),
      "/home/u/outer/inner": snap("/home/u/outer/inner"),
    };
    expect(snapshotForDir(s, "/home/u/outer/inner/src")?.root).toBe(
      "/home/u/outer/inner",
    );
  });

  test("unrelated dir matches nothing", () => {
    const s = { "/home/u/repo": snap("/home/u/repo") };
    expect(snapshotForDir(s, "/tmp/elsewhere")).toBeUndefined();
  });
});

describe("parseToolbarPref", () => {
  test("null and garbage fall back to default", () => {
    expect(parseToolbarPref(null)).toEqual(DEFAULT_TOOLBAR);
    expect(parseToolbarPref("not json")).toEqual(DEFAULT_TOOLBAR);
    expect(parseToolbarPref("42")).toEqual(DEFAULT_TOOLBAR);
  });

  test("newer version than supported falls back to default", () => {
    expect(parseToolbarPref('{"version": 2, "enabled": false}')).toEqual(
      DEFAULT_TOOLBAR,
    );
  });

  test("unknown chip ids are dropped", () => {
    const pref = parseToolbarPref(
      '{"version":1,"enabled":true,"left":["diffCount","voiceInput","hack"],"right":["clock"],"hidden":[]}',
    );
    expect(pref.left).toEqual(["diffCount"]);
    expect(pref.right).toEqual(["clock"]);
  });

  test("missing chip arrays fall back to the default sides", () => {
    const pref = parseToolbarPref('{"version":1,"enabled":true}');
    expect(pref.left).toEqual(DEFAULT_TOOLBAR.left);
    expect(pref.right).toEqual(DEFAULT_TOOLBAR.right);
  });

  test("an explicit empty array is respected", () => {
    const pref = parseToolbarPref(
      '{"version":1,"enabled":true,"left":[],"right":["clock"],"hidden":[]}',
    );
    expect(pref.left).toEqual([]);
    expect(pref.right).toEqual(["clock"]);
  });

  test("enabled false is preserved", () => {
    const pref = parseToolbarPref(
      '{"version":1,"enabled":false,"left":[],"right":[],"hidden":[]}',
    );
    expect(pref.enabled).toBe(false);
  });
});
