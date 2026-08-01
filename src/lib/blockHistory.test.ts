import { describe, expect, it } from "bun:test";

import { mergeBlockHistory } from "./blockHistory";
import type { Block } from "./ipc";

function block(id: number, command = `cmd${id}`): Block {
  return {
    id,
    sessionId: "s1",
    command,
    exitCode: 0,
    startedAtMs: 0,
    finishedAtMs: 0,
    lines: [],
    truncated: 0,
  };
}

const ids = (blocks: Block[]) => blocks.map((b) => b.id);

describe("mergeBlockHistory", () => {
  it("puts the history before what is already on screen", () => {
    expect(ids(mergeBlockHistory([block(9)], [block(1), block(2)]))).toEqual([
      1, 2, 9,
    ]);
  });

  it("keeps the history when a command finished while it was loading", () => {
    // O bug que isto fecha: o guarda antigo via a lista não-vazia e jogava o
    // histórico inteiro fora, apagando a sessão da tela.
    const merged = mergeBlockHistory([block(42)], [block(40), block(41)]);
    expect(ids(merged)).toEqual([40, 41, 42]);
  });

  it("does not duplicate a block that is in both", () => {
    expect(ids(mergeBlockHistory([block(2), block(3)], [block(1), block(2)])))
      .toEqual([1, 2, 3]);
  });

  it("keeps the on-screen copy when the id repeats", () => {
    // O bloco vivo é o que o usuário está olhando; trocá-lo pela cópia do disco
    // faria a tela piscar sem motivo.
    const merged = mergeBlockHistory([block(2, "vivo")], [block(2, "do disco")]);
    expect(merged).toHaveLength(1);
    expect(merged[0].command).toBe("vivo");
  });

  it("returns what is on screen when there is no history", () => {
    const live = [block(1)];
    expect(mergeBlockHistory(live, [])).toBe(live);
  });

  it("returns the history when the screen is empty", () => {
    const loaded = [block(1)];
    expect(mergeBlockHistory([], loaded)).toBe(loaded);
  });
});
