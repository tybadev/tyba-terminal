import { describe, expect, it } from "bun:test";

import { nextPaletteMode, PALETTE_MODES } from "./paletteMode";

describe("nextPaletteMode", () => {
  it("cicla para frente actions → files → sessions → actions", () => {
    expect(nextPaletteMode("actions")).toBe("files");
    expect(nextPaletteMode("files")).toBe("sessions");
    expect(nextPaletteMode("sessions")).toBe("actions");
  });

  it("cicla para trás com Shift+Tab", () => {
    expect(nextPaletteMode("actions", -1)).toBe("sessions");
    expect(nextPaletteMode("sessions", -1)).toBe("files");
    expect(nextPaletteMode("files", -1)).toBe("actions");
  });

  it("mantém os três modos na ordem esperada", () => {
    expect(PALETTE_MODES).toEqual(["actions", "files", "sessions"]);
  });
});
