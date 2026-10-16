import { describe, expect, test } from "bun:test";

import {
  EMPTY_HISTORY,
  HISTORY_LIMIT,
  graphShot,
  keepInHistory,
  record,
  redo,
  removedNodes,
  restoreShot,
  undo,
} from "./graphHistory";
import { newNode, toFlow } from "./maestro";

// Fixtures sintéticas: nada aqui veio de sessão real.
const note = (id: string, x = 0) => newNode(id, id, { type: "note", markdown: "" }, { x, y: 0 });
const shotOf = (...ids: string[]) => {
  const flow = toFlow({
    id: "g",
    name: "g",
    dir: "",
    variables: {},
    nodes: ids.map((id) => note(id)),
    edges: [],
  });
  return graphShot(flow.nodes, flow.edges);
};

describe("pilha de retratos (canvas-5)", () => {
  test("⌘Z volta ao anterior e ⇧⌘Z refaz", () => {
    const a = shotOf("a");
    const ab = shotOf("a", "b");
    const h = record(EMPTY_HISTORY, a);
    const back = undo(h, ab)!;
    expect(back.shot).toBe(a);
    expect(back.history).toEqual({ past: [], future: [ab] });
    const again = redo(back.history, a)!;
    expect(again.shot).toBe(ab);
    expect(again.history).toEqual({ past: [a], future: [] });
  });

  test("nada a desfazer ou refazer: null", () => {
    expect(undo(EMPTY_HISTORY, "x")).toBeNull();
    expect(redo(EMPTY_HISTORY, "x")).toBeNull();
  });

  test("edição nova depois de desfazer perde o refazer", () => {
    const h = { past: ["a"], future: ["c"] };
    expect(record(h, "b")).toEqual({ past: ["a", "b"], future: [] });
  });

  test("o mesmo retrato seguido não entra duas vezes", () => {
    expect(record({ past: ["a"], future: [] }, "a").past).toEqual(["a"]);
  });

  test("a pilha tem teto", () => {
    let h = EMPTY_HISTORY;
    for (let i = 0; i < HISTORY_LIMIT + 10; i++) h = record(h, String(i));
    expect(h.past.length).toBe(HISTORY_LIMIT);
    expect(h.past[0]).toBe("10");
  });
});

describe("retrato do grafo", () => {
  test("ida e volta preserva nós, posição, tamanho e ligações", () => {
    const flow = toFlow({
      id: "g",
      name: "g",
      dir: "",
      variables: {},
      nodes: [note("a", 10), note("b", 500)],
      edges: [{ id: "e", from: "a", to: "b", kind: "context" }],
    });
    const back = restoreShot(graphShot(flow.nodes, flow.edges));
    expect(back.nodes.map((n) => [n.id, n.position.x, n.width])).toEqual([
      ["a", 10, 360],
      ["b", 500, 360],
    ]);
    expect(back.edges.map((e) => [e.source, e.target, e.data.kind])).toEqual([
      ["a", "b", "context"],
    ]);
  });

  test("seleção e arrasto não mudam o retrato", () => {
    const flow = toFlow({
      id: "g",
      name: "g",
      dir: "",
      variables: {},
      nodes: [note("a")],
      edges: [],
    });
    const selected = flow.nodes.map((n) => ({ ...n, selected: true, dragging: true }));
    expect(graphShot(selected, flow.edges)).toBe(graphShot(flow.nodes, flow.edges));
  });

  test("removedNodes acha o que foi apagado", () => {
    expect(removedNodes(shotOf("a", "b", "c"), shotOf("b"))).toEqual(["a", "c"]);
    expect(removedNodes(shotOf("a"), shotOf("a", "b"))).toEqual([]);
  });
});

describe("Manter no grafo fora do ⌘Z", () => {
  const graphOf = (ids: string[], edges: { id: string; from: string; to: string }[] = []) => {
    const flow = toFlow({
      id: "g",
      name: "g",
      dir: "",
      variables: {},
      nodes: ids.map((id) => note(id)),
      edges: edges.map((e) => ({ ...e, kind: "context" as const })),
    });
    return graphShot(flow.nodes, flow.edges);
  };
  const ids = (shot: string) => restoreShot(shot).nodes.map((n) => n.id);

  test("desfazer depois do Manter não tira do grafo o nó que o core marcou kept", () => {
    // O dono pôs "b" (passo do ⌘Z); depois manteve "k", criado pelo orquestrador.
    let h = record(EMPTY_HISTORY, graphOf(["a"]));
    const afterKeep = graphOf(["a", "b", "k"], [{ id: "e", from: "k", to: "a" }]);
    h = keepInHistory(h, afterKeep, new Set(["k"]));
    const back = undo(h, afterKeep)!;
    // Volta só o passo do dono: "b" sai, "k" e a ligação dele ficam.
    expect(ids(back.shot)).toEqual(["a", "k"]);
    expect(restoreShot(back.shot).edges.map((e) => e.id)).toEqual(["e"]);
    expect(undo(back.history, back.shot)).toBeNull();
  });

  test("o refazer também leva o mantido; ligação sem ponta no retrato não entra", () => {
    const h = { past: [], future: [graphOf(["b"])] };
    const shot = graphOf(["a", "k"], [{ id: "e", from: "k", to: "a" }]);
    const next = keepInHistory(h, shot, new Set(["k"]));
    expect(ids(next.future[0])).toEqual(["b", "k"]);
    expect(restoreShot(next.future[0]).edges).toEqual([]);
  });
});
