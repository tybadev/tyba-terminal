import { describe, expect, it } from "bun:test";

import { STRIP_HYSTERESIS, STRIP_TOP_LEVEL, pickStripLevel } from "./orbitStrip";

// Larguras por degrau, já com o vão de 12 (ordem de grandeza do quadro).
const NEEDS = [1460, 1096, 967, 834, 647];

describe("pickStripLevel", () => {
  it("fica inteira quando a faixa toda cabe", () => {
    expect(pickStripLevel(NEEDS, 1500, 0)).toBe(0);
    expect(pickStripLevel(NEEDS, 1460, 0)).toBe(0);
  });

  it("usa o menor degrau que cabe", () => {
    expect(pickStripLevel(NEEDS, 1459, 0)).toBe(1);
    expect(pickStripLevel(NEEDS, 1176, 0)).toBe(1);
    expect(pickStripLevel(NEEDS, 976, 0)).toBe(2);
    expect(pickStripLevel(NEEDS, 876, 0)).toBe(3);
    expect(pickStripLevel(NEEDS, 696, 0)).toBe(4);
  });

  it("abaixo do último degrau fica no último (a faixa quebra)", () => {
    expect(pickStripLevel(NEEDS, 400, 2)).toBe(STRIP_TOP_LEVEL);
    expect(pickStripLevel(NEEDS, 400, 4)).toBe(STRIP_TOP_LEVEL);
  });

  it("só volta um degrau com a folga", () => {
    expect(pickStripLevel(NEEDS, 1460, 1)).toBe(1);
    expect(pickStripLevel(NEEDS, 1460 + STRIP_HYSTERESIS - 1, 1)).toBe(1);
    expect(pickStripLevel(NEEDS, 1460 + STRIP_HYSTERESIS, 1)).toBe(0);
  });

  it("a folga não segura quem já não cabe", () => {
    expect(pickStripLevel(NEEDS, 1095, 1)).toBe(2);
  });

  it("volta vários degraus de uma vez quando sobra espaço", () => {
    expect(pickStripLevel(NEEDS, 2000, 4)).toBe(0);
    expect(pickStripLevel(NEEDS, 1000, 4)).toBe(2);
  });

  it("pede para medir o degrau que ainda não tem largura", () => {
    expect(pickStripLevel([], 1000, 0)).toBe(0);
    expect(pickStripLevel([undefined, 1096], 2000, 1)).toBe(0);
    expect(pickStripLevel([1460], 1000, 0)).toBe(1);
    expect(pickStripLevel([1460, 1096, undefined, 834], 900, 3)).toBe(2);
  });
});
