import { describe, expect, test } from "bun:test";

import { parseStartupMode } from "./startup";

describe("parseStartupMode", () => {
  test("reconhece os três modos", () => {
    expect(parseStartupMode("resume")).toBe("resume");
    expect(parseStartupMode("keep_layout")).toBe("keep_layout");
    expect(parseStartupMode("fresh")).toBe("fresh");
  });

  test("ausente ou inválido cai em resume, como no core", () => {
    expect(parseStartupMode(null)).toBe("resume");
    expect(parseStartupMode(undefined)).toBe("resume");
    expect(parseStartupMode("lixo")).toBe("resume");
  });
});
