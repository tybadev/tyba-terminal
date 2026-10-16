import { describe, expect, test } from "bun:test";

import { canvasKey, enterTerminal, frameInUse, keyOwnedElsewhere } from "./graphKeys";

const key = (
  k: string,
  mods: Partial<Record<"metaKey" | "ctrlKey" | "shiftKey" | "altKey", boolean>> = {},
) => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  ...mods,
});

describe("canvasKey (canvas-9)", () => {
  test("mac: ⌘ com a letra", () => {
    expect(canvasKey(key("z", { metaKey: true }), true)).toBe("undo");
    expect(canvasKey(key("Z", { metaKey: true, shiftKey: true }), true)).toBe("redo");
    expect(canvasKey(key("d", { metaKey: true }), true)).toBe("duplicate");
    expect(canvasKey(key("c", { metaKey: true }), true)).toBe("copy");
    expect(canvasKey(key("v", { metaKey: true }), true)).toBe("paste");
    expect(canvasKey(key("a", { metaKey: true }), true)).toBe("selectAll");
    expect(canvasKey(key("0", { metaKey: true }), true)).toBe("zoomActual");
    expect(canvasKey(key("1", { metaKey: true }), true)).toBe("fit");
    expect(canvasKey(key("k", { metaKey: true }), true)).toBe("palette");
  });

  test("fora do mac é Ctrl; ⌘ não conta", () => {
    expect(canvasKey(key("z", { ctrlKey: true }), false)).toBe("undo");
    expect(canvasKey(key("z", { metaKey: true }), false)).toBeNull();
    expect(canvasKey(key("z", { ctrlKey: true }), true)).toBeNull();
  });

  test("/ sozinho abre a paleta; letra sem modificador não faz nada", () => {
    expect(canvasKey(key("/"), true)).toBe("palette");
    expect(canvasKey(key("d"), true)).toBeNull();
    expect(canvasKey(key("/", { shiftKey: true }), true)).toBeNull();
  });

  test("Enter sozinho foca o terminal do nó; com modificador não", () => {
    expect(canvasKey(key("Enter"), true)).toBe("focusTerminal");
    expect(canvasKey(key("Enter"), false)).toBe("focusTerminal");
    expect(canvasKey(key("Enter", { shiftKey: true }), true)).toBeNull();
    expect(canvasKey(key("Enter", { altKey: true }), true)).toBeNull();
    expect(canvasKey(key("Enter", { ctrlKey: true }), true)).toBeNull();
  });

  test("⌥ e ⇧ com outra letra não são do canvas", () => {
    expect(canvasKey(key("z", { metaKey: true, altKey: true }), true)).toBeNull();
    expect(canvasKey(key("d", { metaKey: true, shiftKey: true }), true)).toBeNull();
  });
});

describe("keyOwnedElsewhere", () => {
  const el = (tagName: string, inside: string | null = null, editable = false) => ({
    tagName,
    isContentEditable: editable,
    closest: (sel: string) => (inside && sel.includes(inside) ? {} : null),
  });

  test("campo, área de texto e editável são donos do teclado", () => {
    expect(keyOwnedElsewhere(el("INPUT"))).toBe(true);
    expect(keyOwnedElsewhere(el("TEXTAREA"))).toBe(true);
    expect(keyOwnedElsewhere(el("DIV", null, true))).toBe(true);
  });

  test("terminal do nó (.nokey) e diálogo também", () => {
    expect(keyOwnedElsewhere(el("DIV", ".nokey"))).toBe(true);
    expect(keyOwnedElsewhere(el("DIV", '[role="dialog"]'))).toBe(true);
  });

  test("a página do nó de navegador (iframe em foco) é dona: ⌘Z, ⌘D, Delete, / e ⌘K não são do canvas", () => {
    expect(keyOwnedElsewhere(el("IFRAME"))).toBe(true);
  });

  test("o canvas e o corpo da página não", () => {
    expect(keyOwnedElsewhere(el("DIV"))).toBe(false);
    expect(keyOwnedElsewhere(el("BODY"))).toBe(false);
    expect(keyOwnedElsewhere(null)).toBe(false);
  });
});

describe("enterTerminal", () => {
  const sessions: Record<string, string | null> = { a: "s-a", b: "s-b", n: null };
  const sessionOf = (id: string) => sessions[id] ?? null;

  test("um nó selecionado com sessão: o terminal dele", () => {
    expect(enterTerminal([{ id: "a" }], sessionOf)).toBe("s-a");
  });

  test("vários selecionados (⌘A), nenhum, ou nó sem sessão: nada recebe o foco", () => {
    expect(enterTerminal([{ id: "a" }, { id: "b" }], sessionOf)).toBeNull();
    expect(enterTerminal([], sessionOf)).toBeNull();
    expect(enterTerminal([{ id: "n" }], sessionOf)).toBeNull();
  });
});

describe("frameInUse (você está usando o navegador)", () => {
  test("o foco no iframe do nó é o dono usando a página", () => {
    const frame = { tagName: "IFRAME" };
    expect(frameInUse(frame, frame)).toBe(true);
  });

  test("foco em outro lugar, ou nó sem iframe, não", () => {
    const frame = { tagName: "IFRAME" };
    expect(frameInUse({ tagName: "BODY" }, frame)).toBe(false);
    expect(frameInUse({ tagName: "IFRAME" }, frame)).toBe(false);
    expect(frameInUse(null, null)).toBe(false);
    expect(frameInUse(undefined, null)).toBe(false);
  });
});
