import { describe, expect, test } from "bun:test";

import type { Loaded } from "./ipc";
import {
  mergeLoaded,
  nextBootPoll,
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

describe("nextBootPoll", () => {
  test("para no instante em que o core responde pronto", () => {
    expect(nextBootPoll({ ready: true, elapsedMs: 0 })).toBeNull();
    expect(nextBootPoll({ ready: true, elapsedMs: 600_000 })).toBeNull();
  });

  test("o intervalo afrouxa conforme o boot demora", () => {
    expect(nextBootPoll({ ready: false, elapsedMs: 0 })).toBe(150);
    expect(nextBootPoll({ ready: false, elapsedMs: 1_999 })).toBe(150);
    expect(nextBootPoll({ ready: false, elapsedMs: 2_000 })).toBe(1_000);
    expect(nextBootPoll({ ready: false, elapsedMs: 29_999 })).toBe(1_000);
    expect(nextBootPoll({ ready: false, elapsedMs: 30_000 })).toBe(5_000);
  });

  test("não desiste: o diálogo de permissão do macOS espera clique humano", () => {
    // Um teto aqui recriaria o bug que este poll existe para fechar: quem
    // desistiu não tem como voltar, e a janela fica vazia para sempre.
    expect(nextBootPoll({ ready: false, elapsedMs: 10 * 60_000 })).toBe(5_000);
  });

  test("evento perdido e snapshot `ready: false`: quem preenche é o poll", () => {
    // `listen()` do Tauri é assíncrono: entre pedir o registro e o listener
    // existir de fato há uma janela, e o `app://ready` emitido dentro dela se
    // perde — não há reenvio. Some a isso um `boot_snapshot` que leu `ready`
    // antes do `mark_ready` do core, que é o caso para o qual a ordem de
    // leitura do core foi escolhida, e o front fica sem nenhuma notícia: era a
    // janela vazia até uma chamada não relacionada acontecer.
    const coreReadyAtMs = 900; // boot lento, mas dentro do normal
    let clock = 0;
    let state = { ready: false, sessions: [] as string[] };

    const accept = (response: Loaded<string[]>) => {
      const update = mergeLoaded(state.ready, response);
      state = { ready: update.ready, sessions: update.value ?? state.sessions };
    };

    // A resposta do boot chegou primeiro e leu `ready: false`.
    accept({ ready: false, value: [] });
    expect(state.ready).toBe(false);

    // O `app://ready` se perdeu: ninguém mais avisa. Só o poll roda.
    let ticks = 0;
    for (;;) {
      const delay = nextBootPoll({ ready: state.ready, elapsedMs: clock });
      if (delay === null) break;
      if (++ticks > 100) throw new Error("o poll não convergiu");
      clock += delay;
      accept({ ready: clock >= coreReadyAtMs, value: ["restaurada"] });
    }

    expect(state.ready).toBe(true);
    expect(state.sessions).toEqual(["restaurada"]);
    // E preenche logo depois de o core ficar pronto, não muito depois.
    expect(clock).toBeLessThanOrEqual(coreReadyAtMs + 150);
  });
});
