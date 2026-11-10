import { describe, expect, it } from "bun:test";

import { FOOTER_STEP, FOOTER_TOP_STEP, footerKeeps } from "./appFooter";
import { STRIP_HYSTERESIS, pickStripLevel } from "./orbitStrip";

// Largura que o rodapé pede em cada degrau (0..7), da mais larga à mais curta.
const NEEDS = [1100, 1040, 975, 905, 830, 760, 640, 600];

describe("escada do rodapé", () => {
  it("fica inteiro quando cabe", () => {
    expect(pickStripLevel(NEEDS, 1200, 0, FOOTER_TOP_STEP)).toBe(0);
  });

  it("tira um degrau por vez, o menor que cabe", () => {
    expect(pickStripLevel(NEEDS, 1099, 0, FOOTER_TOP_STEP)).toBe(1);
    expect(pickStripLevel(NEEDS, 900, 0, FOOTER_TOP_STEP)).toBe(4);
    expect(pickStripLevel(NEEDS, 700, 0, FOOTER_TOP_STEP)).toBe(6);
  });

  it("abaixo de tudo fica no degrau 8 (nome da sessão só no ícone)", () => {
    expect(pickStripLevel(NEEDS, 500, 3, FOOTER_TOP_STEP)).toBe(8);
  });

  it("degrau ainda não medido é devolvido para medir antes de decidir", () => {
    expect(pickStripLevel([1100, 1040], 900, 1, FOOTER_TOP_STEP)).toBe(2);
  });

  it("só volta um degrau com a folga (histerese)", () => {
    expect(pickStripLevel(NEEDS, 905, 4, FOOTER_TOP_STEP)).toBe(4);
    expect(pickStripLevel(NEEDS, 905 + STRIP_HYSTERESIS - 1, 4, FOOTER_TOP_STEP)).toBe(4);
    expect(pickStripLevel(NEEDS, 905 + STRIP_HYSTERESIS, 4, FOOTER_TOP_STEP)).toBe(3);
  });

  it("ordem do quadro: atalhos, snippets, histórico, comandos, Rich Input, pílula, hora, sessão", () => {
    expect([
      FOOTER_STEP.shortcutsLabel,
      FOOTER_STEP.snippetsLabel,
      FOOTER_STEP.historyLabel,
      FOOTER_STEP.commandsLabel,
      FOOTER_STEP.richInputLabel,
      FOOTER_STEP.pillCompact,
      FOOTER_STEP.clock,
      FOOTER_STEP.cwdLabel,
    ]).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
  });

  it("a peça sai no próprio degrau e não antes", () => {
    expect(footerKeeps(0, FOOTER_STEP.shortcutsLabel)).toBe(true);
    expect(footerKeeps(1, FOOTER_STEP.shortcutsLabel)).toBe(false);
    expect(footerKeeps(5, FOOTER_STEP.pillCompact)).toBe(true);
    expect(footerKeeps(6, FOOTER_STEP.pillCompact)).toBe(false);
    expect(footerKeeps(8, FOOTER_STEP.cwdLabel)).toBe(false);
  });
});
