import { beforeEach, describe, expect, it } from "bun:test";
import {
  clearToasts,
  dismissToast,
  pushToast,
  subscribeToasts,
  toastError,
  type ToastMessage,
} from "./toast";

describe("toast store", () => {
  beforeEach(() => {
    clearToasts();
  });

  it("entrega o estado atual assim que alguém assina", () => {
    pushToast({ title: "já estava aqui" });
    let seen: ToastMessage[] = [];
    const un = subscribeToasts((all) => {
      seen = all;
    });
    expect(seen).toHaveLength(1);
    un();
  });

  it("notifica cada assinante quando chega um toast", () => {
    let a = 0;
    let b = 0;
    const unA = subscribeToasts(() => {
      a += 1;
    });
    const unB = subscribeToasts(() => {
      b += 1;
    });
    pushToast({ title: "novo" });
    expect(a).toBe(2);
    expect(b).toBe(2);
    unA();
    unB();
  });

  it("para de notificar depois de cancelar a assinatura", () => {
    let calls = 0;
    const un = subscribeToasts(() => {
      calls += 1;
    });
    un();
    pushToast({ title: "ninguém ouve" });
    expect(calls).toBe(1);
  });

  it("dismiss remove só o alvo", () => {
    const first = pushToast({ title: "um" });
    pushToast({ title: "dois" });
    let seen: ToastMessage[] = [];
    const un = subscribeToasts((all) => {
      seen = all;
    });
    dismissToast(first);
    expect(seen.map((t) => t.title)).toEqual(["dois"]);
    un();
  });

  it("dismiss de id inexistente não notifica ninguém", () => {
    pushToast({ title: "um" });
    let calls = 0;
    const un = subscribeToasts(() => {
      calls += 1;
    });
    dismissToast("fantasma");
    expect(calls).toBe(1);
    un();
  });

  it("mantém a ordem de chegada", () => {
    pushToast({ title: "um" });
    pushToast({ title: "dois" });
    pushToast({ title: "três" });
    let seen: ToastMessage[] = [];
    const un = subscribeToasts((all) => {
      seen = all;
    });
    expect(seen.map((t) => t.title)).toEqual(["um", "dois", "três"]);
    un();
  });

  it("tone padrão é info", () => {
    pushToast({ title: "sem tom" });
    let seen: ToastMessage[] = [];
    const un = subscribeToasts((all) => {
      seen = all;
    });
    expect(seen[0].tone).toBe("info");
    un();
  });

  it("toastError serializa o detalhe e marca o tom", () => {
    toastError("falhou", new Error("boom"));
    let seen: ToastMessage[] = [];
    const un = subscribeToasts((all) => {
      seen = all;
    });
    expect(seen[0].tone).toBe("error");
    expect(seen[0].detail).toContain("boom");
    un();
  });

  it("toastError sem detalhe não inventa string", () => {
    toastError("falhou");
    let seen: ToastMessage[] = [];
    const un = subscribeToasts((all) => {
      seen = all;
    });
    expect(seen[0].detail).toBeUndefined();
    un();
  });

  it("substitui a lista em vez de mutar, para o React ver a mudança", () => {
    pushToast({ title: "um" });
    let first: ToastMessage[] = [];
    let second: ToastMessage[] = [];
    const un = subscribeToasts((all) => {
      first = second;
      second = all;
    });
    pushToast({ title: "dois" });
    expect(first).not.toBe(second);
    un();
  });
});
