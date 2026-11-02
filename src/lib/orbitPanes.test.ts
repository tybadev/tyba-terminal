import { describe, expect, test } from "bun:test";

import { gridAttention, loadWithRetry, orbitPanesToPrune, orbitPickKey } from "./orbitPanes";

describe("orbitPickKey", () => {
  const base = {
    key: "c",
    isCopy: false,
    pickSession: "node",
    attached: (id: string) => id === "node",
    inTextField: false,
    textSelected: false,
  };

  test("⌘C copia a marcação do nó", () => {
    expect(orbitPickKey({ ...base, isCopy: true })).toBe("copy");
  });

  test("seleção de texto de verdade ganha do ⌘C", () => {
    expect(orbitPickKey({ ...base, isCopy: true, textSelected: true })).toBeNull();
  });

  test("Esc limpa, menos dentro de campo de texto", () => {
    expect(orbitPickKey({ ...base, key: "Escape" })).toBe("clear");
    expect(orbitPickKey({ ...base, key: "Escape", inTextField: true })).toBeNull();
  });

  test("marcação da grade (ou nenhuma) segue bloqueada sob o Orbit", () => {
    expect(orbitPickKey({ ...base, isCopy: true, pickSession: "grid" })).toBeNull();
    expect(orbitPickKey({ ...base, key: "Escape", pickSession: null })).toBeNull();
  });

  test("outra tecla não é da marcação", () => {
    expect(orbitPickKey({ ...base, key: "w" })).toBeNull();
  });
});

describe("gridAttention", () => {
  const inLayout = (id: string) => id === "grid";
  test("só a sessão da grade acende o título", () => {
    expect(gridAttention([{ id: "node", attention: true }], inLayout)).toBe(false);
    expect(
      gridAttention(
        [
          { id: "node", attention: true },
          { id: "grid", attention: true },
        ],
        inLayout,
      ),
    ).toBe(true);
    expect(gridAttention([{ id: "grid", attention: false }], inLayout)).toBe(false);
  });
});

describe("orbitPanesToPrune", () => {
  const base = {
    detached: new Set(["closed", "alive", "back"]),
    attached: (id: string) => id === "back",
    inLayout: (id: string) => id === "grid",
  };

  test("sai a sessão de nó solta e fechada", () => {
    const sessions = [
      { id: "closed", finished: true },
      { id: "alive", finished: false },
      { id: "back", finished: true },
      { id: "grid", finished: true },
      { id: "other", finished: true },
    ];
    expect(orbitPanesToPrune({ ...base, sessions })).toEqual(["closed"]);
  });

  test("sessão fechada que está na grade fica", () => {
    expect(
      orbitPanesToPrune({
        ...base,
        detached: new Set(["grid"]),
        sessions: [{ id: "grid", finished: true }],
      }),
    ).toEqual([]);
  });
});

describe("loadWithRetry", () => {
  const noSleep = async () => {};

  test("primeira deu certo: uma leitura só", async () => {
    let calls = 0;
    const ok = await loadWithRetry(async () => (calls++, true), [10, 20], noSleep);
    expect(ok).toBe(true);
    expect(calls).toBe(1);
  });

  test("falha (ou ready:false) tenta de novo e para no limite", async () => {
    let calls = 0;
    const waits: number[] = [];
    const ok = await loadWithRetry(
      async () => (calls++, false),
      [10, 20],
      async (ms) => {
        waits.push(ms);
      },
    );
    expect(ok).toBe(false);
    expect(calls).toBe(3);
    expect(waits).toEqual([10, 20]);
  });

  test("rejeitar conta como falha; a seguinte que dá certo vale", async () => {
    let calls = 0;
    const ok = await loadWithRetry(
      async () => {
        calls++;
        if (calls === 1) throw new Error("boot");
        return true;
      },
      [10],
      noSleep,
    );
    expect(ok).toBe(true);
    expect(calls).toBe(2);
  });
});
