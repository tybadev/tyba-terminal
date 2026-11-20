import { describe, expect, test } from "bun:test";

import { DEFAULT_UI_SCALE, parseUiScale } from "./uiScale";

describe("parseUiScale", () => {
  test("aceita só as três escalas", () => {
    expect(parseUiScale("100")).toBe(100);
    expect(parseUiScale("110")).toBe(110);
    expect(parseUiScale("125")).toBe(125);
  });

  test("ausente ou fora da lista vale o padrão de 100%", () => {
    expect(DEFAULT_UI_SCALE).toBe(100);
    for (const v of [null, undefined, "", "115", "150", "abc"]) {
      expect(parseUiScale(v)).toBe(100);
    }
  });
});
