import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  SESSION_DOT_CLASS,
  SESSION_DOT_LABEL_KEY,
  sessionDot,
} from "./sessionCard";

const base = { approvals: 0, agentRank: null, commandRunning: false };

describe("sessionDot", () => {
  test("parada sem nada é cinza", () => {
    expect(sessionDot(base)).toBe("idle");
  });

  test("aprovação pendente é âmbar, mesmo com agente trabalhando", () => {
    expect(sessionDot({ ...base, approvals: 1, agentRank: 1 })).toBe("attention");
  });

  test("agente esperando, falhou ou terminou sem ser visto é âmbar", () => {
    for (const rank of [2, 3, 4]) {
      expect(sessionDot({ ...base, agentRank: rank })).toBe("attention");
    }
  });

  test("agente trabalhando ou comando rodando é verde", () => {
    expect(sessionDot({ ...base, agentRank: 1 })).toBe("running");
    expect(sessionDot({ ...base, commandRunning: true })).toBe("running");
  });

  test("agente sem sinal (rank 0) e nada rodando é cinza", () => {
    expect(sessionDot({ ...base, agentRank: 0 })).toBe("idle");
  });

  test("só o verde pulsa", () => {
    expect(SESSION_DOT_CLASS.running).toMatch(/tyba-pulse/);
    expect(SESSION_DOT_CLASS.attention).not.toMatch(/tyba-pulse/);
    expect(SESSION_DOT_CLASS.idle).not.toMatch(/tyba-pulse/);
    expect(SESSION_DOT_CLASS.attention).toMatch(/bg-tyba-amber/);
    expect(SESSION_DOT_CLASS.running).toMatch(/bg-tyba-green/);
  });
});

// O módulo de i18n lê `localStorage` ao carregar; sem DOM, a checagem é pelo
// texto do arquivo. O plural é do i18next (`_one`/`_other`): basta as duas
// formas existirem, com o singular certo, em cada idioma.
describe("rótulos existem em pt-BR e en", () => {
  const source = readFileSync(
    new URL("../i18n/index.ts", import.meta.url),
    "utf8",
  );
  const hits = (key: string) =>
    source.split(new RegExp(`\\n\\s+"?${key}"?:`)).length - 1;

  test("plural de painéis e aprovações, e rótulos do ponto", () => {
    for (const key of [
      "sessionPanes_one",
      "sessionPanes_other",
      "agentsApprovals_one",
      "agentsApprovals_other",
      ...Object.values(SESSION_DOT_LABEL_KEY),
    ]) {
      expect(hits(key)).toBe(2);
    }
  });

  test("singular e plural em pt-BR", () => {
    expect(source).toContain('sessionPanes_one: "{{count}} painel"');
    expect(source).toContain('sessionPanes_other: "{{count}} painéis"');
    expect(source).toContain('agentsApprovals_one: "{{count}} aprovação"');
    expect(source).toContain('agentsApprovals_other: "{{count}} aprovações"');
  });
});
