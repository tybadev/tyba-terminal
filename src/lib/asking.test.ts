import { describe, expect, test } from "bun:test";

import { askingText, visibleAsk } from "./asking";

const ask = (session_id: string) => ({
  session_id,
  agent: "Claude",
  project: "api" as string | null,
  branch: "feat/x" as string | null,
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

describe("texto do card", () => {
  const t = (key: string, params?: Record<string, string>) =>
    `${key}(${params?.agent ?? ""})`;

  test("agente, projeto e branch, separados por ·", () => {
    expect(askingText(ask("a"), t)).toBe("askingBody(Claude) · api · feat/x");
  });

  test("sem branch nem projeto, só o agente", () => {
    expect(askingText({ ...ask("a"), project: null, branch: null }, t)).toBe(
      "askingBody(Claude)",
    );
  });
});
