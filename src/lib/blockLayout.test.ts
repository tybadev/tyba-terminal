import { describe, expect, it } from "bun:test";

import { cwdChange, formatDuration } from "./blockLayout";

describe("cwdChange", () => {
  it("anuncia o primeiro diretório conhecido", () => {
    expect(cwdChange(null, "/a/b")).toBe("/a/b");
  });

  it("cala quando o diretório é o mesmo", () => {
    expect(cwdChange("/a/b", "/a/b")).toBeNull();
    expect(cwdChange("/a/b/", "/a/b")).toBeNull();
  });

  it("anuncia quando o diretório muda", () => {
    expect(cwdChange("/a/b", "/a/c")).toBe("/a/c");
  });

  it("não anuncia diretório desconhecido", () => {
    expect(cwdChange("/a/b", null)).toBeNull();
    expect(cwdChange(null, null)).toBeNull();
  });

  it("a raiz não vira string vazia", () => {
    expect(cwdChange("/a", "/")).toBe("/");
    expect(cwdChange("/", "/")).toBeNull();
  });
});

describe("formatDuration", () => {
  it("abaixo de 10s tem um decimal, no separador do idioma", () => {
    expect(formatDuration(400, "pt-BR")).toBe("0,4s");
    expect(formatDuration(400, "en")).toBe("0.4s");
    expect(formatDuration(9_940, "pt-BR")).toBe("9,9s");
  });

  it("nunca escreve 0,0s", () => {
    expect(formatDuration(0, "pt-BR")).toBe("0,1s");
    expect(formatDuration(20, "pt-BR")).toBe("0,1s");
  });

  it("de 10s a 1 min, segundos inteiros", () => {
    expect(formatDuration(12_300)).toBe("12s");
    expect(formatDuration(59_999)).toBe("59s");
  });

  it("minutos com segundos em dois dígitos", () => {
    expect(formatDuration(62_000)).toBe("1m02s");
    expect(formatDuration(252_000)).toBe("4m12s");
  });

  it("horas com minutos em dois dígitos", () => {
    expect(formatDuration(3_900_000)).toBe("1h05m");
  });

  it("whole força segundos inteiros no contador ao vivo", () => {
    expect(formatDuration(3_400, "pt-BR", { whole: true })).toBe("3s");
  });

  it("valor inválido não vira duração", () => {
    expect(formatDuration(-1)).toBeNull();
    expect(formatDuration(Number.NaN)).toBeNull();
  });
});
