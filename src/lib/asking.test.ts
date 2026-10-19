import { describe, expect, test } from "bun:test";

import type { Session, SessionStatus } from "./ipc";
import { nextAsk, pruneAsks, recordAsk, type Ask } from "./asking";

const asking: SessionStatus = {
  state: "awaiting_input",
  hint: null,
  reason: "native_prompt",
};

const session = (id: string, status: SessionStatus): Session =>
  ({
    id,
    kind: { type: "shell" },
    title: id,
    repo_root: null,
    worktree: null,
    status,
    attention: false,
    created_at: "",
  }) as Session;

describe("pedido de permissão dentro do app", () => {
  test("mostra o mais antigo fora da tela; a sessão em foco não ganha card", () => {
    let asks: Map<string, Ask> = new Map();
    asks = recordAsk(asks, "a", "O Claude está pedindo algo · api · main", 1);
    asks = recordAsk(asks, "b", "O Claude está pedindo algo · web · x", 2);
    const sessions = [session("a", asking), session("b", asking)];
    expect(nextAsk(asks, sessions, null)?.sessionId).toBe("a");
    expect(nextAsk(asks, sessions, "a")?.sessionId).toBe("b");
    expect(nextAsk(asks, [session("a", asking)], "a")).toBeNull();
  });

  test("um pendente por sessão: o pedido de B não apaga o de A", () => {
    let asks: Map<string, Ask> = new Map();
    asks = recordAsk(asks, "a", "A1", 1);
    asks = recordAsk(asks, "b", "B1", 2);
    asks = recordAsk(asks, "a", "A2", 3);
    expect(asks.size).toBe(2);
    expect(asks.get("a")).toEqual({ body: "A2", at: 1 });
  });

  test("some quando a sessão sai do prompt nativo", () => {
    const asks = recordAsk(new Map(), "a", "A", 1);
    const working = [session("a", { state: "running" })];
    expect(nextAsk(asks, working, null)).toBeNull();
    expect(pruneAsks(asks, working).size).toBe(0);
    const still = [session("a", asking)];
    expect(pruneAsks(asks, still)).toBe(asks);
  });

  test("aprovação do gate (card do TYBA) não é pedido do prompt nativo", () => {
    const asks = recordAsk(new Map(), "a", "A", 1);
    const gate = [
      session("a", { state: "awaiting_input", hint: null, reason: "approval" }),
    ];
    expect(nextAsk(asks, gate, null)).toBeNull();
  });
});
