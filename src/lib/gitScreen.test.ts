import { describe, expect, test } from "bun:test";

import { gitPlaceOf, railSections, resolvePlace, setGitPlace, slideOf } from "./gitScreen";

describe("railSections", () => {
  test("Revisar só com worktree", () => {
    expect(railSections(false)).toEqual(["changes", "history", "prs", "ci"]);
    expect(railSections(true)).toEqual(["changes", "review", "history", "prs", "ci"]);
  });
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
});

test("cada sessão lembra onde estava", () => {
  setGitPlace("s1", { section: "ci", pr: null });
  expect(gitPlaceOf("s1")).toEqual({ section: "ci", pr: null });
  expect(gitPlaceOf("s2")).toEqual({ section: "changes", pr: null });
});
