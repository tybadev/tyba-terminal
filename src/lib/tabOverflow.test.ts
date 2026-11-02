import { describe, expect, test } from "bun:test";

import { splitTabs } from "./tabOverflow";

const tabs = (n: number) => Array.from({ length: n }, (_, i) => `t${i + 1}`);

describe("splitTabs", () => {
  test("até o limite, todas aparecem", () => {
    expect(splitTabs(tabs(6), (t) => t === "t1")).toEqual({
      shown: tabs(6),
      hidden: [],
    });
  });

  test("passou do limite, ficam limite − 1 e o resto vai para a lista", () => {
    const { shown, hidden } = splitTabs(tabs(8), (t) => t === "t2");
    expect(shown).toEqual(["t1", "t2", "t3", "t4", "t5"]);
    expect(hidden).toEqual(["t6", "t7", "t8"]);
  });

  test("a ativa escondida toma o último lugar visível", () => {
    const { shown, hidden } = splitTabs(tabs(8), (t) => t === "t7");
    expect(shown).toEqual(["t1", "t2", "t3", "t4", "t7"]);
    expect(hidden).toEqual(["t5", "t6", "t8"]);
  });

  test("sem ativa, não quebra", () => {
    const { shown, hidden } = splitTabs(tabs(7), () => false);
    expect(shown).toHaveLength(5);
    expect(hidden).toEqual(["t6", "t7"]);
  });
});
