import { describe, expect, test } from "bun:test";

import type { Loaded } from "./ipc";
import {
  mergeLoaded,
  parseStartupMode,
  shouldPromptNewSession,
} from "./startup";

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

describe("mergeLoaded", () => {
  test("resposta pronta marca o boot e aplica o dado", () => {
    expect(mergeLoaded(false, { ready: true, value: ["a", "b"] })).toEqual({
      ready: true,
      value: ["a", "b"],
    });
  });

  test("resposta do meio do boot não aplica nada", () => {
    // Vazio de boot não é vazio: a lista veio assim por ainda não ter sido
    // lida. Aplicá-la apagaria da tela o que está voltando.
    expect(mergeLoaded(false, { ready: false, value: [] })).toEqual({
      ready: false,
      value: null,
    });
  });

  test("`ready` não regride", () => {
    expect(mergeLoaded(true, { ready: false, value: [] }).ready).toBe(true);
  });

  test("chamada que falhou não mexe no que já se sabe", () => {
    expect(mergeLoaded(true, null)).toEqual({ ready: true, value: null });
    expect(mergeLoaded(false, undefined)).toEqual({
      ready: false,
      value: null,
    });
  });

  test("a corrida: o evento chega primeiro, o snapshot velho depois", () => {
    // `app://ready` venceu a resposta de `boot_snapshot()`. O core marca ready,
    // emite o evento e só então termina de montar a resposta — que leu `ready`
    // ANTES do valor, de propósito, e por isso chega dizendo `false` com dado
    // que já era bom. Rebaixar o `ready` por causa dela apagava sessões e
    // layout, e `app://ready` não dispara de novo: a janela ficava vazia até
    // alguma chamada não relacionada acontecer.
    let state: { ready: boolean; value: string[] } = {
      ready: false,
      value: [],
    };
    const accept = (response: Loaded<string[]> | null) => {
      const update = mergeLoaded(state.ready, response);
      state = { ready: update.ready, value: update.value ?? state.value };
    };

    // 1. o handler do evento reconsulta e recebe o estado real
    accept({ ready: true, value: ["restaurada"] });
    // 2. a resposta velha do boot chega em seguida
    accept({ ready: false, value: [] });

    expect(state.ready).toBe(true);
    expect(state.value).toEqual(["restaurada"]);
  });
});
