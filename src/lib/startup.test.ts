import { describe, expect, test } from "bun:test";

import { parseStartupMode, shouldPromptNewSession } from "./startup";

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

describe("shouldPromptNewSession", () => {
  test("layout conhecido e vazio abre o modal", () => {
    expect(
      shouldPromptNewSession({ ready: true, workspaces: 0, prompted: false }),
    ).toBe(true);
  });

  test("com workspace nenhum modal", () => {
    expect(
      shouldPromptNewSession({ ready: true, workspaces: 5, prompted: false }),
    ).toBe(false);
  });

  test("meio do boot não decide, mesmo com a lista vazia", () => {
    // O defeito: `boot_snapshot` responde antes de o core carregar o layout, e
    // os 5 workspaces salvos chegam como zero. O modal abria por cima deles.
    expect(
      shouldPromptNewSession({ ready: false, workspaces: 0, prompted: false }),
    ).toBe(false);
  });

  test("o `ready` que chega depois ainda decide", () => {
    // A outra metade do mesmo defeito: marcar a decisão como tomada no meio do
    // boot fazia o layout real chegar sem ninguém para reavaliar.
    expect(
      shouldPromptNewSession({ ready: true, workspaces: 0, prompted: false }),
    ).toBe(true);
  });

  test("uma vez por janela", () => {
    // O modal é boas-vindas, não lembrete: fechar sem criar nada e o core
    // reemitir `app://ready` não pode reabri-lo.
    expect(
      shouldPromptNewSession({ ready: true, workspaces: 0, prompted: true }),
    ).toBe(false);
  });
});
