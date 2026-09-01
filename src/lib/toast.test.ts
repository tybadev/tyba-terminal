import { beforeEach, describe, expect, it } from "bun:test";
import {
  clearToasts,
  dismissToast,
  pushToast,
  subscribeToasts,
  toastDuration,
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

  // Review r1 (v0.6.2), MAJOR: o ack do alarme de deriva depende de saber
  // quando o dono realmente viu/fechou o toast -- `onDismiss` é o gancho.
  it("dismiss chama onDismiss só do toast fechado", () => {
    let fired = 0;
    const id = pushToast({ title: "alvo", onDismiss: () => (fired += 1) });
    pushToast({ title: "outro" });
    dismissToast(id);
    expect(fired).toBe(1);
  });

  it("dismiss de id inexistente não chama onDismiss de ninguém", () => {
    let fired = 0;
    pushToast({ title: "alvo", onDismiss: () => (fired += 1) });
    dismissToast("fantasma");
    expect(fired).toBe(0);
  });

  it("toast sem onDismiss dispensa normalmente, sem estourar", () => {
    const id = pushToast({ title: "sem callback" });
    expect(() => dismissToast(id)).not.toThrow();
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

  it("carrega a action até o assinante, quando informada", () => {
    const run = () => {};
    pushToast({ title: "login", action: { label: "Abrir no navegador", run } });
    let seen: ToastMessage[] = [];
    const un = subscribeToasts((all) => {
      seen = all;
    });
    expect(seen[0].action?.label).toBe("Abrir no navegador");
    expect(seen[0].action?.run).toBe(run);
    un();
  });

  it("sem action, o campo fica ausente", () => {
    pushToast({ title: "sem botão" });
    let seen: ToastMessage[] = [];
    const un = subscribeToasts((all) => {
      seen = all;
    });
    expect(seen[0].action).toBeUndefined();
    un();
  });

  it("toast sem action expõe duração finita — auto-dismiss (item 0 do contrato)", () => {
    expect(Number.isFinite(toastDuration({ action: undefined }))).toBe(true);
  });

  it("toast com action expõe duração infinita — não some sozinho antes do clique", () => {
    expect(toastDuration({ action: { label: "x", run: () => {} } })).toBe(
      Infinity,
    );
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
