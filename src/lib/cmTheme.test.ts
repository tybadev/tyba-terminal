import { describe, expect, it } from "bun:test";

import { cmPalette, type SyntaxPalette } from "./cmTheme";

const ROLES: (keyof SyntaxPalette)[] = [
  "comment",
  "keyword",
  "control",
  "string",
  "function",
  "number",
  "type",
  "variable",
  "tag",
  "invalid",
];

describe("cmPalette", () => {
  it("cobre todos os papéis de token nos dois temas com cores válidas", () => {
    for (const dark of [true, false]) {
      const palette = cmPalette(dark);
      for (const role of ROLES) {
        expect(palette[role]).toMatch(/^#[0-9a-fA-F]{6,8}$/);
      }
    }
  });

  it("dark usa o mono-dark e light usa o vitesse — paletas distintas", () => {
    expect(cmPalette(true).keyword).toBe("#C792EA");
    expect(cmPalette(false).keyword).toBe("#4d9375");
    expect(cmPalette(true).string).not.toBe(cmPalette(false).string);
  });
});
