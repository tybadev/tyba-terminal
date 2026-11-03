import { describe, expect, test } from "bun:test";

import { clipBounds, copyNodes, pasteClip } from "./graphClipboard";
import { newNode, type OrbitEdge } from "./orbit";

// Fixtures sintéticas: nada aqui veio de sessão real.
const proc = (id: string, x: number, y: number) =>
  newNode(
    id,
    id,
    { type: "process", command: "bun dev", output: "log", file: null, dir: "/srv" },
    { x, y },
  );

const nodes = [proc("a", 0, 0), proc("b", 700, 100), proc("c", 0, 600)];
const edges: OrbitEdge[] = [
  { id: "ab", from: "a", to: "b", kind: "log" },
  { id: "cb", from: "c", to: "b", kind: "log" },
];

describe("copiar e colar nós (canvas-9)", () => {
  test("copia os escolhidos e só as ligações entre eles", () => {
    const clip = copyNodes(nodes, edges, new Set(["a", "b"]));
    expect(clip.nodes.map((n) => n.id)).toEqual(["a", "b"]);
    expect(clip.edges.map((e) => e.id)).toEqual(["ab"]);
  });

  test("a caixa envolve os nós copiados", () => {
    const clip = copyNodes(nodes, edges, new Set(["a", "b"]));
    expect(clipBounds(clip)).toEqual({ x: 0, y: 0, w: 1260, h: 540 });
    expect(clipBounds({ nodes: [], edges: [] })).toBeNull();
  });

  test("colar dá ids novos, mantém a forma e religa entre os novos", () => {
    let n = 0;
    const pasted = pasteClip(
      copyNodes(nodes, edges, new Set(["a", "b"])),
      { x: 100, y: 50 },
      () => `n${++n}`,
    );
    expect(pasted.nodes.map((x) => [x.id, x.position])).toEqual([
      ["n1", { x: 100, y: 50 }],
      ["n2", { x: 800, y: 150 }],
    ]);
    expect(pasted.edges).toEqual([{ id: "n3", from: "n1", to: "n2", kind: "log" }]);
    // O original não muda.
    expect(nodes[0].position).toEqual({ x: 0, y: 0 });
  });

  test("a cópia é independente do original", () => {
    const pasted = pasteClip(copyNodes(nodes, edges, new Set(["a"])), { x: 0, y: 0 }, () => "z");
    if (pasted.nodes[0].kind.type === "process") pasted.nodes[0].kind.command = "outro";
    expect(nodes[0].kind.type === "process" && nodes[0].kind.command).toBe("bun dev");
  });
});
