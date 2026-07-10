import { describe, expect, test } from "bun:test";

import {
  DEFAULT_TOOLBAR,
  parseToolbarPref,
  type ToolbarPref,
} from "./repoSnapshots";
import { dropTarget, moveChip, zoneOf } from "./toolbarLayout";

const pref: ToolbarPref = {
  version: 1,
  enabled: true,
  left: ["diffCount"],
  right: ["aheadBehind", "branch", "cwd"],
  hidden: ["clock"],
};

describe("zoneOf", () => {
  test("encontra a zona de cada chip", () => {
    expect(zoneOf(pref, "diffCount")).toBe("left");
    expect(zoneOf(pref, "cwd")).toBe("right");
    expect(zoneOf(pref, "clock")).toBe("hidden");
  });
});

describe("moveChip", () => {
  test("reordena dentro da mesma zona", () => {
    const next = moveChip(pref, "cwd", "right", 0);
    expect(next.right).toEqual(["cwd", "aheadBehind", "branch"]);
    expect(next.left).toEqual(pref.left);
  });

  test("move entre zonas na posição pedida", () => {
    const next = moveChip(pref, "clock", "right", 1);
    expect(next.right).toEqual(["aheadBehind", "clock", "branch", "cwd"]);
    expect(next.hidden).toEqual([]);
  });

  test("índice além do fim entra no fim", () => {
    const next = moveChip(pref, "diffCount", "right", 99);
    expect(next.right).toEqual(["aheadBehind", "branch", "cwd", "diffCount"]);
    expect(next.left).toEqual([]);
  });

  test("movimento que não muda nada devolve a mesma referência", () => {
    expect(moveChip(pref, "aheadBehind", "right", 0)).toBe(pref);
  });

  test("não muta o pref original", () => {
    moveChip(pref, "clock", "left", 0);
    expect(pref.left).toEqual(["diffCount"]);
    expect(pref.hidden).toEqual(["clock"]);
  });

  test("mantém os demais campos do pref", () => {
    const next = moveChip(DEFAULT_TOOLBAR, "clock", "hidden", 0);
    expect(next.version).toBe(DEFAULT_TOOLBAR.version);
    expect(next.enabled).toBe(DEFAULT_TOOLBAR.enabled);
  });
});

describe("dropTarget", () => {
  test("soltar sobre uma zona aponta pro fim dela", () => {
    expect(dropTarget(pref, "right")).toEqual({ zone: "right", index: 3 });
  });

  test("soltar sobre um chip aponta pra posição dele", () => {
    expect(dropTarget(pref, "branch")).toEqual({ zone: "right", index: 1 });
  });

  test("id desconhecido não tem alvo", () => {
    expect(dropTarget(pref, "ghost")).toBeNull();
  });
});

describe("parseToolbarPref", () => {
  test("chip duplicado entre zonas fica só na primeira", () => {
    const parsed = parseToolbarPref(
      JSON.stringify({
        version: 1,
        enabled: true,
        left: ["clock"],
        right: ["branch"],
        hidden: ["clock", "branch"],
      }),
    );
    expect(parsed.left).toEqual(["clock"]);
    expect(parsed.right).toEqual(["branch"]);
    expect(parsed.hidden).toEqual([]);
  });
});
