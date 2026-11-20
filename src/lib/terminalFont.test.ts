import { describe, expect, test } from "bun:test";

import { parseTerminalFont, terminalFontFamily } from "./terminalFont";

describe("terminalFont", () => {
  test("valor desconhecido ou ausente cai na JetBrains Mono", () => {
    expect(parseTerminalFont("system")).toBe("system");
    expect(parseTerminalFont("jetbrains")).toBe("jetbrains");
    expect(parseTerminalFont("geist")).toBe("jetbrains");
    expect(parseTerminalFont(undefined)).toBe("jetbrains");
  });

  test("cada escolha tem a sua pilha, com mono no fim", () => {
    expect(terminalFontFamily("jetbrains").startsWith('"JetBrains Mono"')).toBe(true);
    expect(terminalFontFamily("system").startsWith("ui-monospace")).toBe(true);
    expect(terminalFontFamily("system").endsWith("monospace")).toBe(true);
  });
});
