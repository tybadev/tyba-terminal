import { describe, expect, test } from "bun:test";

import {
  elapsedSeconds,
  formatSeconds,
  isSlow,
  progressOf,
  SLOW_AFTER_MS,
  shellBootStep,
  shortSha,
  slowAfterMs,
  splitEmphasis,
} from "./loading";

describe("limiar de demora por operação", () => {
  test("remoto nunca tem menos folga que local", () => {
    for (const [op, { local, remote }] of Object.entries(SLOW_AFTER_MS)) {
      expect(remote, op).toBeGreaterThanOrEqual(local);
      expect(local, op).toBeGreaterThan(0);
    }
  });

  test("docker local vira demora aos 5 s; no servidor, aos 10 s", () => {
    expect(slowAfterMs("docker")).toBe(5_000);
    expect(slowAfterMs("docker", true)).toBe(10_000);
    expect(isSlow(0, 4_999, "docker")).toBe(false);
    expect(isSlow(0, 5_000, "docker")).toBe(true);
    expect(isSlow(0, 9_000, "docker", true)).toBe(false);
    expect(isSlow(0, 10_000, "docker", true)).toBe(true);
  });

  test("sem início conhecido não há demora", () => {
    expect(isSlow(null, 1e12, "forge")).toBe(false);
    expect(isSlow(undefined, 1e12, "forge")).toBe(false);
    expect(isSlow(Number.NaN, 1e12, "forge")).toBe(false);
  });
});

describe("tempo decorrido", () => {
  test("segundos inteiros, relógio para trás vira zero", () => {
    expect(elapsedSeconds(1_000, 7_999)).toBe(6);
    expect(elapsedSeconds(5_000, 1_000)).toBe(0);
  });

  test("uma casa abaixo de 10 s, com a vírgula do idioma", () => {
    expect(formatSeconds(800, "pt-BR")).toBe("0,8 s");
    expect(formatSeconds(800, "en")).toBe("0.8 s");
    expect(formatSeconds(1_490, "pt-BR")).toBe("1,4 s");
    expect(formatSeconds(12_900, "pt-BR")).toBe("12 s");
    expect(formatSeconds(-5, "en")).toBe("0.0 s");
  });
});

describe("progresso só com total conhecido", () => {
  test("sem total, total zero ou inválido: nada de barra", () => {
    expect(progressOf(3, null)).toBeNull();
    expect(progressOf(3, undefined)).toBeNull();
    expect(progressOf(0, 0)).toBeNull();
    expect(progressOf(1, Number.POSITIVE_INFINITY)).toBeNull();
    expect(progressOf(null, 10)).toBeNull();
  });

  test("conta e percentual, presos ao total", () => {
    expect(progressOf(2, 3)).toEqual({ done: 2, total: 3, percent: 67 });
    expect(progressOf(400, 12_340)).toEqual({ done: 400, total: 12_340, percent: 3 });
    expect(progressOf(15, 10)).toEqual({ done: 10, total: 10, percent: 100 });
    expect(progressOf(-1, 10)).toEqual({ done: 0, total: 10, percent: 0 });
  });
});

describe("formatação da etapa", () => {
  test("sha curto", () => {
    expect(shortSha("a1b2c3d4e5f6")).toBe("a1b2c3d");
    expect(shortSha(" abc ")).toBe("abc");
  });

  test("acende os fatos citados, na ordem em que aparecem", () => {
    expect(splitEmphasis("no servidor api-prod: lendo /srv/app", ["/srv/app", "api-prod"])).toEqual([
      { text: "no servidor ", em: false },
      { text: "api-prod", em: true },
      { text: ": lendo ", em: false },
      { text: "/srv/app", em: true },
    ]);
  });

  test("valor ausente ou vazio não quebra o texto", () => {
    expect(splitEmphasis("comparando com a base", ["", "zzz"])).toEqual([
      { text: "comparando com a base", em: false },
    ]);
    expect(splitEmphasis("", ["x"])).toEqual([]);
  });

  test("um valor contido no outro: o mais longo ganha no mesmo ponto", () => {
    expect(splitEmphasis("lendo ~/a/b", ["~/a", "~/a/b"])).toEqual([
      { text: "lendo ", em: false },
      { text: "~/a/b", em: true },
    ]);
  });
});

describe("etapa do shell subindo", () => {
  const boot = { shell: "zsh", rc: "~/.zshrc", started_at_ms: 1 };

  test("local com rc conhecido: carregando o rc", () => {
    expect(shellBootStep(boot, null)).toEqual({ key: "shellBootRc", params: { rc: "~/.zshrc" } });
  });

  test("sem rc: o nome do shell", () => {
    expect(shellBootStep({ ...boot, rc: null }, null)).toEqual({
      key: "shellBootShell",
      params: { shell: "zsh" },
    });
  });

  test("core ainda não respondeu: abrindo o terminal", () => {
    expect(shellBootStep(null, null).key).toBe("shellBootOpening");
  });

  test("remoto diz o servidor, nunca um rc local", () => {
    expect(shellBootStep(boot, "api-prod")).toEqual({
      key: "shellBootRemote",
      params: { host: "api-prod" },
    });
  });
});
