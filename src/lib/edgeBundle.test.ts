import { describe, expect, test } from "bun:test";

import { bundleEdges, unbundleEdgeChanges } from "./edgeBundle";
import { fromFlow, newNode, toFlow, type EdgeKind } from "./orbit";

// Fixtures sintéticas: nada aqui veio de sessão real.
const agent = (id: string) =>
  newNode(id, id, { type: "agent", runner: "claude", role: "", prompt: "", dir: "/p" }, { x: 0, y: 0 });

const edge = (id: string, from: string, to: string, kind: EdgeKind) => ({ id, from, to, kind });

/** Grafo salvo antes da porta única: A→B de contexto e de diff, e A→C. */
const saved = toFlow({
  id: "g",
  name: "g",
  dir: "",
  variables: {},
  nodes: [agent("a"), agent("b"), agent("c")],
  edges: [edge("e1", "a", "b", "context"), edge("e2", "a", "c", "context"), edge("e3", "a", "b", "diff")],
});

describe("ligações do mesmo par viram uma no canvas", () => {
  test("A→B de contexto e de diff: uma ligação com os dois tipos, na ordem do modelo", () => {
    const { edges, members } = bundleEdges(saved.edges);
    expect(edges.map((e) => [e.source, e.target])).toEqual([
      ["a", "b"],
      ["a", "c"],
    ]);
    expect(edges[0].data?.bundle).toEqual([
      { id: "e1", kind: "context" },
      { id: "e3", kind: "diff" },
    ]);
    expect(edges[1]).toBe(saved.edges[1]);
    expect(members.get(edges[0].id)).toEqual(["e1", "e3"]);
  });

  test("o modelo salvo não perde nenhuma (o canvas desenha, o state guarda)", () => {
    bundleEdges(saved.edges);
    const graph = fromFlow({ id: "g", name: "g", dir: "", variables: {} }, saved.nodes, saved.edges);
    expect(graph.edges.map((e) => e.id)).toEqual(["e1", "e2", "e3"]);
  });

  test("sem par repetido, a mesma lista", () => {
    const one = saved.edges.filter((e) => e.id !== "e3");
    expect(bundleEdges(one).edges).toBe(one);
  });

  test("apagar a juntada apaga cada uma; selecionar seleciona cada uma", () => {
    const { edges, members } = bundleEdges(saved.edges);
    const id = edges[0].id;
    expect(
      unbundleEdgeChanges(
        [
          { type: "remove", id },
          { type: "select", id: "e2", selected: true },
        ],
        members,
      ),
    ).toEqual([
      { type: "remove", id: "e1" },
      { type: "remove", id: "e3" },
      { type: "select", id: "e2", selected: true },
    ]);
    expect(unbundleEdgeChanges([{ type: "select", id, selected: true }], members)).toEqual([
      { type: "select", id: "e1", selected: true },
      { type: "select", id: "e3", selected: true },
    ]);
  });

  test("ligação sem tipo (subagente) passa como está", () => {
    const sub = { id: "s", source: "a", target: "b" };
    const { edges } = bundleEdges([...saved.edges, sub]);
    expect(edges.at(-1)).toBe(sub);
  });
});
