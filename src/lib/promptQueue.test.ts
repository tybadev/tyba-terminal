import { describe, expect, test } from "bun:test";

import { queueIndicator, reorderIds, slotAt } from "./promptQueue";

const running = { state: "running" } as const;
const one = [{ id: "a", text: "x" }];

describe("queueIndicator", () => {
  test("sem pausa: verde, próximo é o nº 1", () => {
    expect(queueIndicator({ prompts: one, paused: null }, running)).toEqual({
      tone: "send",
      label: "promptQueueSends",
      detail: { key: "promptQueueNext", params: { n: 1 } },
    });
  });

  test("sem pausa e vazia: sem próximo", () => {
    expect(queueIndicator({ prompts: [], paused: null }, running).detail).toBeNull();
  });

  test("aprovação: âmbar com o pedido do status", () => {
    const s = {
      state: "awaiting_input",
      hint: "o claude pede para rodar cargo test",
      reason: "approval",
    } as const;
    expect(queueIndicator({ prompts: one, paused: "approval" }, s)).toEqual({
      tone: "approval",
      label: "promptQueuePausedApproval",
      detail: { text: "o claude pede para rodar cargo test" },
    });
  });

  test("aprovação sem hint: sem porquê", () => {
    expect(
      queueIndicator({ prompts: one, paused: "approval" }, running).detail,
    ).toBeNull();
  });

  test("falha: vermelho com o resumo do status", () => {
    const s = { state: "idle", summary: "limite de uso" } as const;
    expect(queueIndicator({ prompts: one, paused: "failed" }, s)).toEqual({
      tone: "failed",
      label: "promptQueuePausedFailed",
      detail: { text: "limite de uso" },
    });
  });

  test("o resto mapeia tom e rótulo", () => {
    const cases = [
      ["send_failed", "failed", "promptQueuePausedSendFailed"],
      ["interrupted", "stopped", "promptQueuePausedInterrupted"],
      ["user", "stopped", "promptQueuePausedUser"],
      ["agent_ended", "stopped", "promptQueuePausedAgentEnded"],
      ["session_ended", "stopped", "promptQueuePausedSessionEnded"],
    ] as const;
    for (const [paused, tone, label] of cases) {
      expect(queueIndicator({ prompts: one, paused }, running)).toEqual({
        tone,
        label,
        detail: null,
      });
    }
  });
});

describe("reorderIds", () => {
  const ids = ["a", "b", "c", "d"];

  test("o 3º para a posição 2 (quadro 4A)", () => {
    expect(reorderIds(ids, 2, 1)).toEqual(["a", "c", "b", "d"]);
  });

  test("para baixo desconta o próprio item", () => {
    expect(reorderIds(ids, 0, 3)).toEqual(["b", "c", "a", "d"]);
    expect(reorderIds(ids, 0, 4)).toEqual(["b", "c", "d", "a"]);
  });

  test("solto onde estava: nada muda", () => {
    expect(reorderIds(ids, 1, 1)).toBeNull();
    expect(reorderIds(ids, 1, 2)).toBeNull();
  });

  test("fora dos limites: nada", () => {
    expect(reorderIds(ids, 4, 0)).toBeNull();
    expect(reorderIds(ids, 0, 5)).toBeNull();
  });
});

describe("slotAt", () => {
  const mids = [16, 48, 80];
  test("acima de todos, entre e abaixo", () => {
    expect(slotAt(0, mids)).toBe(0);
    expect(slotAt(30, mids)).toBe(1);
    expect(slotAt(100, mids)).toBe(3);
  });
});
