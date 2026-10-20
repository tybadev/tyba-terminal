import { describe, expect, test } from "bun:test";

import { visibleAsk } from "./asking";

const ask = (session_id: string) => ({
  session_id,
  body: `O Claude está pedindo algo · ${session_id}`,
});

describe("card de pedido dentro do app", () => {
  const known = new Set(["a", "b"]);

  test("mostra o mais antigo da lista do core fora da tela", () => {
    expect(visibleAsk([ask("a"), ask("b")], null, known)?.session_id).toBe("a");
  });

  test("a sessão em foco não ganha card", () => {
    expect(visibleAsk([ask("a"), ask("b")], "a", known)?.session_id).toBe("b");
    expect(visibleAsk([ask("a")], "a", known)).toBeNull();
  });

  test("sessão que já não existe não ganha card", () => {
    expect(visibleAsk([ask("x"), ask("b")], null, known)?.session_id).toBe("b");
  });
});
