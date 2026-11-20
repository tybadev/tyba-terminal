import { describe, expect, test } from "bun:test";

import { linkPair, talkTarget, toolKey } from "./toolbar";

const key = (k: string, mods: Partial<KeyboardEvent> = {}) => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  ...mods,
});

describe("toolKey", () => {
  test("a letra sozinha escolhe a ferramenta do quadro", () => {
    expect(toolKey(key("v"))).toBe("cursor");
    expect(toolKey(key("H"))).toBe("hand");
    expect(toolKey(key("a"))).toBe("agent");
    expect(toolKey(key("t"))).toBe("term");
    expect(toolKey(key("p"))).toBe("proc");
    expect(toolKey(key("n"))).toBe("note");
    expect(toolKey(key("b"))).toBe("web");
    expect(toolKey(key("l"))).toBe("link");
  });

  test("com modificador a tecla é de outro atalho (⌘V cola, ⌘A seleciona)", () => {
    expect(toolKey(key("v", { metaKey: true }))).toBeNull();
    expect(toolKey(key("a", { ctrlKey: true }))).toBeNull();
    expect(toolKey(key("t", { shiftKey: true }))).toBeNull();
    expect(toolKey(key("n", { altKey: true }))).toBeNull();
  });

  test("outras teclas não são ferramenta", () => {
    expect(toolKey(key("x"))).toBeNull();
    expect(toolKey(key("Enter"))).toBeNull();
    expect(toolKey(key("/"))).toBeNull();
  });
});

describe("linkPair", () => {
  const at = (id: string, x: number) => ({ id, position: { x } });

  test("dois selecionados ligam da esquerda para a direita", () => {
    expect(linkPair([at("b", 400), at("a", 10)])).toEqual({ from: "a", to: "b" });
    expect(linkPair([at("a", 10), at("b", 400)])).toEqual({ from: "a", to: "b" });
  });

  test("sem exatamente dois, nada a ligar", () => {
    expect(linkPair([])).toBeNull();
    expect(linkPair([at("a", 0)])).toBeNull();
    expect(linkPair([at("a", 0), at("b", 1), at("c", 2)])).toBeNull();
  });
});

describe("talkTarget", () => {
  const nodes = [
    { id: "nota", kind: { type: "note" } },
    { id: "term", kind: { type: "terminal", dir: "/srv/api" } },
    { id: "rev", kind: { type: "agent" } },
  ] as Parameters<typeof talkTarget>[0];

  test("o orquestrador que espera você vem primeiro", () => {
    expect(talkTarget(nodes, () => true, (id) => id === "rev")).toEqual({ id: "rev", waiting: true });
  });

  test("ninguém esperando: o primeiro com sessão", () => {
    expect(talkTarget(nodes, (n) => n.id !== "term", () => false)).toEqual({ id: "rev", waiting: false });
  });

  test("sem sessão viva, nada a abrir", () => {
    expect(talkTarget(nodes, () => false, () => false)).toBeNull();
  });
});
