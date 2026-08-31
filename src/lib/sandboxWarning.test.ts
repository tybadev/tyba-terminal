import { describe, expect, it } from "bun:test";

import type { SandboxWarningKind } from "./ipc";
import { sandboxWarningTitleKey } from "./sandboxWarning";

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
