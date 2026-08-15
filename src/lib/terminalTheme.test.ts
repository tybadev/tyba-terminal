import { describe, expect, it } from "bun:test";

import type { TerminalPalette } from "./ipc";
import { isCssColor, terminalTheme } from "./terminalTheme";

/** `mono-dark`, o par que expôs a costura: sunken `#0f0d0d`, terminal `#151313`. */
const MONO_DARK: TerminalPalette = {
  background: "#151313",
  foreground: "#eeffff",
  cursor: "#ffd867",
  cursorAccent: "#151313",
  selectionBackground: "#ffd8674d",
  ansi: [
    "#151313", "#ff6d67", "#c3e88d", "#ffd867",
    "#82aaff", "#c792ea", "#89ddff", "#c7c7c7",
    "#5c595f", "#ff6d67", "#c3e88d", "#ffd867",
    "#82aaff", "#c792ea", "#89ddff", "#ffffff",
  ],
};

describe("terminalTheme", () => {
  it("o fundo do xterm é o token de UI, não o da paleta", () => {
    expect(terminalTheme(MONO_DARK, "#0f0d0d").background).toBe("#0f0d0d");
  });

  it("o cursorAccent acompanha o fundo novo quando era igual ao antigo", () => {
    expect(terminalTheme(MONO_DARK, "#0f0d0d").cursorAccent).toBe("#0f0d0d");
  });

  // Quem escolheu um accent diferente do fundo fez de propósito.
  it("cursorAccent próprio é preservado", () => {
    const palette = { ...MONO_DARK, cursorAccent: "#000000" };
    expect(terminalTheme(palette, "#0f0d0d").cursorAccent).toBe("#000000");
  });

  it("o resto da paleta continua sendo do tema", () => {
    const theme = terminalTheme(MONO_DARK, "#0f0d0d");
    expect(theme.foreground).toBe("#eeffff");
    expect(theme.cursor).toBe("#ffd867");
    expect(theme.green).toBe("#c3e88d");
    expect(theme.brightWhite).toBe("#ffffff");
    expect(theme.selectionBackground).toBe("#ffd8674d");
  });

  // O terminal nunca pode ficar sem fundo: sem token resolvido, vale o de antes.
  it("sem token resolvido cai no fundo da paleta", () => {
    expect(terminalTheme(MONO_DARK, null).background).toBe("#151313");
    expect(terminalTheme(MONO_DARK, "").background).toBe("#151313");
    expect(terminalTheme(MONO_DARK, "   ").background).toBe("#151313");
  });

  // Custom property guarda texto, e tema importado é arquivo de terceiro.
  it("valor que não é cor não vira fundo", () => {
    expect(terminalTheme(MONO_DARK, "url(http://x/y.png)").background).toBe(
      "#151313",
    );
    expect(terminalTheme(MONO_DARK, "var(--outra)").background).toBe("#151313");
  });

  it("aceita as notações de cor que o CSS entrega", () => {
    expect(terminalTheme(MONO_DARK, " #0f0d0d ").background).toBe("#0f0d0d");
    expect(terminalTheme(MONO_DARK, "rgb(15 13 13)").background).toBe(
      "rgb(15 13 13)",
    );
  });
});

describe("isCssColor", () => {
  it("aceita hex de 3 a 8 dígitos, rgb/rgba e hsl/hsla", () => {
    expect(isCssColor("#abc")).toBe(true);
    expect(isCssColor("#0f0d0dff")).toBe(true);
    expect(isCssColor("rgba(0,0,0,.5)")).toBe(true);
    expect(isCssColor("hsl(120 50% 10%)")).toBe(true);
  });

  it("recusa o que não é cor", () => {
    expect(isCssColor("")).toBe(false);
    expect(isCssColor("black")).toBe(false);
    expect(isCssColor("url(x)")).toBe(false);
  });
});
