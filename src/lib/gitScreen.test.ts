import { describe, expect, test } from "bun:test";

import {
  gitPlaceOf,
  hasTurnDiff,
  railSections,
  resolvePlace,
  setGitPlace,
  slideOf,
} from "./gitScreen";
import type { TurnDigest } from "./ipc";

describe("railSections", () => {
  test("Revisar só com worktree", () => {
    expect(railSections(false)).toEqual(["changes", "history", "prs", "ci"]);
    expect(railSections(true)).toEqual(["changes", "review", "history", "prs", "ci"]);
  });
  test("Turno só enquanto o último turno tiver diff", () => {
    expect(railSections(false, true)).toEqual(["changes", "turn", "history", "prs", "ci"]);
    expect(railSections(true, true)).toEqual([
      "changes",
      "turn",
      "review",
      "history",
      "prs",
      "ci",
    ]);
  });
});

test("hasTurnDiff: turno medido que mexeu em arquivo", () => {
  const turn = (over: Partial<TurnDigest>) =>
    ({ last_turn: { diff: "ok", files_total: 2, ...over } as TurnDigest });
  expect(hasTurnDiff(turn({}))).toBe(true);
  expect(hasTurnDiff(turn({ files_total: 0 }))).toBe(false);
  expect(hasTurnDiff(turn({ diff: "unavailable" }))).toBe(false);
  expect(hasTurnDiff(turn({ diff: "no_repo" }))).toBe(false);
  expect(hasTurnDiff({ last_turn: null })).toBe(false);
  expect(hasTurnDiff(null)).toBe(false);
});

describe("slideOf", () => {
  test("descer no trilho entra de baixo, subir entra de cima", () => {
    expect(slideOf({ section: "changes", pr: null }, { section: "ci", pr: null })).toBe("up");
    expect(slideOf({ section: "ci", pr: null }, { section: "history", pr: null })).toBe("down");
  });
  test("abrir PR desliza para o lado, voltar desliza de volta", () => {
    expect(slideOf({ section: "prs", pr: null }, { section: "prs", pr: 6 })).toBe("left");
    expect(slideOf({ section: "prs", pr: 6 }, { section: "prs", pr: null })).toBe("right");
  });
});

test("resolvePlace cai em Alterações quando a seção não existe", () => {
  expect(resolvePlace({ section: "review", pr: null }, false)).toEqual({ section: "changes", pr: null });
  expect(resolvePlace({ section: "prs", pr: 3 }, false)).toEqual({ section: "prs", pr: 3 });
  // O turno seguinte fechou sem diff: a seção Turno some.
  expect(resolvePlace({ section: "turn", pr: null }, false, false)).toEqual({
    section: "changes",
    pr: null,
  });
  expect(resolvePlace({ section: "turn", pr: null, path: "a.rs" }, false, true)).toEqual({
    section: "turn",
    pr: null,
    path: "a.rs",
  });
});

test("cada sessão lembra onde estava", () => {
  setGitPlace("s1", { section: "ci", pr: null });
  expect(gitPlaceOf("s1")).toEqual({ section: "ci", pr: null });
  expect(gitPlaceOf("s2")).toEqual({ section: "changes", pr: null });
});
