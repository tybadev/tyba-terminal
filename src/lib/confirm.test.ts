import { beforeEach, describe, expect, it } from "bun:test";
import {
  answerConfirm,
  requestConfirm,
  resetConfirms,
  subscribeConfirm,
  type ConfirmRequest,
} from "./confirm";

describe("confirm store", () => {
  beforeEach(() => {
    resetConfirms();
  });

  it("resolve true quando confirmado", async () => {
    let seen: ConfirmRequest | null = null;
    const un = subscribeConfirm((c) => {
      seen = c;
    });
    const answer = requestConfirm({ title: "apagar?" });
    answerConfirm(seen!.id, true);
    expect(await answer).toBe(true);
    un();
  });

  it("resolve false quando recusado", async () => {
    let seen: ConfirmRequest | null = null;
    const un = subscribeConfirm((c) => {
      seen = c;
    });
    const answer = requestConfirm({ title: "apagar?" });
    answerConfirm(seen!.id, false);
    expect(await answer).toBe(false);
    un();
  });

  it("enfileira o segundo pedido em vez de descartá-lo", async () => {
    const seen: (ConfirmRequest | null)[] = [];
    const un = subscribeConfirm((c) => {
      seen.push(c);
    });
    const first = requestConfirm({ title: "primeiro" });
    const second = requestConfirm({ title: "segundo" });

    const firstId = seen[seen.length - 1]!.id;
    answerConfirm(firstId, true);
    expect(await first).toBe(true);

    const secondShown = seen[seen.length - 1];
    expect(secondShown?.title).toBe("segundo");
    answerConfirm(secondShown!.id, false);
    expect(await second).toBe(false);
    un();
  });

  it("responder um id que não está na frente não faz nada", async () => {
    let seen: ConfirmRequest | null = null;
    const un = subscribeConfirm((c) => {
      seen = c;
    });
    const answer = requestConfirm({ title: "único" });
    answerConfirm("fantasma", true);
    expect(seen).not.toBeNull();
    answerConfirm(seen!.id, true);
    expect(await answer).toBe(true);
    un();
  });

  it("não vaza o resolve para quem assina", () => {
    const seen: (ConfirmRequest | null)[] = [];
    const un = subscribeConfirm((c) => {
      seen.push(c);
    });
    void requestConfirm({ title: "x" });
    const shown = seen[seen.length - 1];
    expect(shown).not.toBeNull();
    expect(Object.keys(shown!)).not.toContain("resolve");
    un();
  });

  it("reset resolve tudo como false", async () => {
    const first = requestConfirm({ title: "um" });
    const second = requestConfirm({ title: "dois" });
    resetConfirms();
    expect(await first).toBe(false);
    expect(await second).toBe(false);
  });

  it("entrega null quando não há nada pendente", () => {
    const seen: (ConfirmRequest | null)[] = [];
    const un = subscribeConfirm((c) => {
      seen.push(c);
    });
    expect(seen[0]).toBeNull();
    un();
  });

  it("preserva destructive e confirmLabel", () => {
    const seen: (ConfirmRequest | null)[] = [];
    const un = subscribeConfirm((c) => {
      seen.push(c);
    });
    void requestConfirm({
      title: "apagar",
      destructive: true,
      confirmLabel: "Apagar",
    });
    const shown = seen[seen.length - 1];
    expect(shown?.destructive).toBe(true);
    expect(shown?.confirmLabel).toBe("Apagar");
    un();
  });
});
