import { describe, expect, it } from "bun:test";

import { formatCount, formatDuration, formatPercent } from "./stats";

describe("formatDuration", () => {
  it("escolhe a unidade pela grandeza", () => {
    expect(formatDuration(400, "en")).toBe("400 ms");
    expect(formatDuration(1500, "en")).toBe("1.5 s");
    expect(formatDuration(90_000, "en")).toBe("1.5 min");
    expect(formatDuration(5_400_000, "en")).toBe("1.5 h");
  });

  // Sem decisão humana no período não é "decidiu em zero": o travessão diz que
  // não há amostra, o `0 ms` diria que houve e foi instantânea.
  it("ausência de dado vira travessão, e zero continua sendo zero", () => {
    expect(formatDuration(null, "en")).toBe("—");
    expect(formatDuration(0, "en")).toBe("0 ms");
  });

  it("valor impossível não vira texto quebrado", () => {
    expect(formatDuration(Number.NaN, "en")).toBe("—");
    expect(formatDuration(-1, "en")).toBe("—");
  });

  it("respeita o separador decimal do idioma", () => {
    expect(formatDuration(1500, "pt-BR")).toBe("1,5 s");
  });
});

describe("formatPercent", () => {
  it("mantém uma casa e nunca mostra NaN", () => {
    expect(formatPercent(66.7, "en")).toBe("66.7%");
    expect(formatPercent(100, "en")).toBe("100%");
    expect(formatPercent(Number.NaN, "en")).toBe("0%");
  });

  it("respeita o separador decimal do idioma", () => {
    expect(formatPercent(66.7, "pt-BR")).toBe("66,7%");
  });
});

describe("formatCount", () => {
  it("usa o separador de milhar do idioma", () => {
    expect(formatCount(1234, "en")).toBe("1,234");
    expect(formatCount(1234, "pt-BR")).toBe("1.234");
    expect(formatCount(Number.NaN, "en")).toBe("0");
  });
});
