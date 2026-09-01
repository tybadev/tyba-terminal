import { describe, expect, it } from "bun:test";

import type { AgentSandboxWarningPayload, SandboxWarningKind } from "./ipc";
import {
  sandboxWarningTitleKey,
  sandboxWarningToastInput,
} from "./sandboxWarning";

const ALL_KINDS: SandboxWarningKind[] = [
  "CredencialPaiNaoEhRw",
  "CredencialSombreadaDepois",
  "CredencialHostNaoGrava",
  "HomeRoClaudeJsonNaoPersiste",
  "FilhoDesconhecidoEmClaude",
];

describe("sandboxWarningTitleKey", () => {
  it("cada kind tem uma chave própria, e nenhuma se repete", () => {
    // Item 35 do contrato de cobertura: "cada SandboxWarningKind vira toast
    // warning com o texto do §6" -- sem chave própria, dois kinds diferentes
    // cairiam no mesmo aviso e o dono não saberia qual dos dois aconteceu.
    const keys = ALL_KINDS.map(sandboxWarningTitleKey);
    expect(new Set(keys).size).toBe(ALL_KINDS.length);
  });

  it("mapeia cada kind pra uma chave sandboxWarning*", () => {
    for (const kind of ALL_KINDS) {
      expect(sandboxWarningTitleKey(kind)).toStartWith("sandboxWarning");
    }
  });

  it("credencial não bindada tem chave própria", () => {
    expect(sandboxWarningTitleKey("CredencialPaiNaoEhRw")).toBe(
      "sandboxWarningCredencialPaiNaoEhRw",
    );
  });
});

const payload = (
  overrides: Partial<AgentSandboxWarningPayload> = {},
): AgentSandboxWarningPayload => ({
  session_id: "session-1",
  kind: "CredencialPaiNaoEhRw",
  detail: null,
  names: null,
  ...overrides,
});

describe("sandboxWarningToastInput", () => {
  // Review de segurança r2 (v0.6.2), MAJOR: nenhum SandboxWarningKind tem
  // action, e sem `sticky` o toast auto-fecharia em ~9s -- num produto de
  // agente sem supervisão, isso silencia um alarme de segurança que o dono
  // nunca viu. `sticky` precisa valer pra TODO kind, não só deriva.
  for (const kind of ALL_KINDS) {
    it(`${kind}: sempre sticky (nunca auto-dismiss)`, () => {
      const input = sandboxWarningToastInput(
        payload({ kind }),
        "título",
        () => {},
      );
      expect(input.sticky).toBe(true);
    });
  }

  it("FilhoDesconhecidoEmClaude com nomes: onDismiss chama o ack com os nomes exatos", () => {
    const acked: string[][] = [];
    const input = sandboxWarningToastInput(
      payload({ kind: "FilhoDesconhecidoEmClaude", names: ["a", "b"] }),
      "título",
      (names) => acked.push(names),
    );
    input.onDismiss?.();
    expect(acked).toEqual([["a", "b"]]);
  });

  it("FilhoDesconhecidoEmClaude sem nomes: onDismiss ausente (nada pra ackar)", () => {
    const input = sandboxWarningToastInput(
      payload({ kind: "FilhoDesconhecidoEmClaude", names: null }),
      "título",
      () => {
        throw new Error("não devia ser chamado");
      },
    );
    expect(input.onDismiss).toBeUndefined();
  });

  it("outros kinds: onDismiss ausente -- names é só do alarme de deriva", () => {
    const input = sandboxWarningToastInput(
      payload({ kind: "CredencialHostNaoGrava" }),
      "título",
      () => {
        throw new Error("não devia ser chamado");
      },
    );
    expect(input.onDismiss).toBeUndefined();
  });
});
