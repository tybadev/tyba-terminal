import { describe, expect, test } from "bun:test";

import {
  MIN_CONTRAST,
  contrastRatio,
  ensureContrast,
  parseRgb,
  transparentOf,
  type Rgb,
} from "./contrast";

const WHITE: Rgb = [255, 255, 255];

describe("parseRgb", () => {
  test("hex curto, longo e com alfa; rgb() com vírgula ou espaço", () => {
    expect(parseRgb("#fff")).toEqual([255, 255, 255]);
    expect(parseRgb("#F8F8F2")).toEqual([248, 248, 242]);
    expect(parseRgb("#11223380")).toEqual([17, 34, 51]);
    expect(parseRgb("rgb(1 2 3)")).toEqual([1, 2, 3]);
    expect(parseRgb("rgba(4, 5, 6, 0.5)")).toEqual([4, 5, 6]);
    expect(parseRgb("hsl(0 0% 0%)")).toBeNull();
  });
});

describe("ensureContrast", () => {
  test("#F8F8F2 (texto do Monokai) sobre branco vira legível e escurece", () => {
    const out = ensureContrast([248, 248, 242], WHITE);
    expect(out).not.toBeNull();
    expect(contrastRatio(out!, WHITE)).toBeGreaterThanOrEqual(MIN_CONTRAST);
    expect(out![0]).toBeLessThan(248);
  });

  test("amarelo do Monokai (#E6DB74) sobre branco mantém o matiz", () => {
    const out = ensureContrast([0xe6, 0xdb, 0x74], WHITE)!;
    expect(contrastRatio(out, WHITE)).toBeGreaterThanOrEqual(MIN_CONTRAST);
    // continua amarelo: vermelho e verde acima do azul, na mesma ordem
    expect(out[0]).toBeGreaterThan(out[2]);
    expect(out[1]).toBeGreaterThan(out[2]);
    expect(out[0]).toBeGreaterThanOrEqual(out[1]);
  });

  test("cor já legível não muda", () => {
    expect(ensureContrast([0x33, 0x33, 0x33], WHITE)).toBeNull();
    expect(ensureContrast([0xf8, 0xf8, 0xf2], [0x27, 0x28, 0x22])).toBeNull();
  });

  test("tema escuro: cor escura é clareada", () => {
    const bg: Rgb = [0x1e, 0x1f, 0x29];
    const out = ensureContrast([0x30, 0x30, 0x60], bg)!;
    expect(contrastRatio(out, bg)).toBeGreaterThanOrEqual(MIN_CONTRAST);
    expect(out[2]).toBeGreaterThan(0x60);
  });
});

describe("transparentOf", () => {
  test("carrega o RGB do papel com alfa zero", () => {
    expect(transparentOf("#ffffff")).toBe("rgba(255, 255, 255, 0)");
    expect(transparentOf("#1e1f29")).toBe("rgba(30, 31, 41, 0)");
  });

  test("cor que não sabe ler cai no transparente preto de antes", () => {
    expect(transparentOf(undefined)).toBe("rgba(0, 0, 0, 0)");
    expect(transparentOf("oklch(0.5 0 0)")).toBe("rgba(0, 0, 0, 0)");
  });
});
