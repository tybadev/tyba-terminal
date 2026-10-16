import { describe, expect, test } from "bun:test";

import { canvasKey, keyOwnedElsewhere } from "./graphKeys";

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

  test("o canvas e o corpo da página não", () => {
    expect(keyOwnedElsewhere(el("DIV"))).toBe(false);
    expect(keyOwnedElsewhere(el("BODY"))).toBe(false);
    expect(keyOwnedElsewhere(null)).toBe(false);
  });
});
