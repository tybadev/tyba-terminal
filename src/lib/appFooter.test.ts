import { describe, expect, it } from "bun:test";

import {
  DRAWER_MAX,
  DRAWER_MIN,
  FOOTER_STEP,
  FOOTER_TOP_STEP,
  drawerBox,
  footerKeeps,
} from "./appFooter";
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

describe("drawerBox", () => {
  const base = { footerTop: 734, viewportHeight: 760 };

  it("R3: pane largo, 430 com a borda direita 8 dentro do pane e 6 acima do rodapé", () => {
    // janela 1000, lateral 224, pane 776
    const box = drawerBox({ ...base, paneLeft: 224, paneRight: 1000 });
    expect(box).toEqual({ left: 1000 - 8 - 430, width: 430, bottom: 26 + 6, compact: false });
  });

  it("R4: pane de 296 com detalhe: 360, compacta, cresce por cima da lateral", () => {
    // janela 900, lateral 224, pane 296, detalhe 380 à direita
    const box = drawerBox({ ...base, paneLeft: 224, paneRight: 520 });
    expect(box.width).toBe(DRAWER_MIN);
    expect(box.compact).toBe(true);
    expect(box.left + box.width).toBe(520 - 8); // não cobre o detalhe
    expect(box.left).toBeLessThan(224); // por cima da lateral
  });

  it("entre 360 e 430 segue o pane, e abaixo de 430 compacta", () => {
    const box = drawerBox({ ...base, paneLeft: 0, paneRight: 400 });
    expect(box.width).toBe(384);
    expect(box.compact).toBe(true);
    expect(drawerBox({ ...base, paneLeft: 0, paneRight: 2000 }).width).toBe(DRAWER_MAX);
  });

  it("nunca abaixo de 360; sem espaço encosta a 8 da borda esquerda da janela", () => {
    const box = drawerBox({ ...base, paneLeft: 0, paneRight: 300 });
    expect(box.width).toBe(DRAWER_MIN);
    expect(box.left).toBe(8);
  });

  it("split: ancora no pane ativo, o da direita", () => {
    const box = drawerBox({ ...base, paneLeft: 868, paneRight: 1512 });
    expect(box.left + box.width).toBe(1504);
  });
});
