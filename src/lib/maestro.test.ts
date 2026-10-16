import { describe, expect, test } from "bun:test";

import {
  accepts,
  connectionProblem,
  emptyGraph,
  fromFlow,
  inHandle,
  integrationsById,
  kindOfOutHandle,
  newNode,
  outHandle,
  produces,
  toFlow,
  type Graph,
  type Integration,
  type MaestroNode,
} from "./maestro";

// Fixtures sintéticas: nada aqui veio de sessão real.
const claude = newNode(
  "a",
  "Claude · API",
  { type: "agent", runner: "claude", role: "implementador", prompt: "GET /saude" },
  { x: 40, y: 40 },
);
const tests = newNode(
  "t",
  "Testes",
  { type: "process", command: "bun test", output: "json", file: null },
  { x: 540, y: 230 },
);
const note = newNode("n", "Decisões", { type: "note", markdown: "- UTC" }, { x: 540, y: 560 });
const lint = newNode("i", "Lint", { type: "integration", integrationId: "lint" }, { x: 1040, y: 120 });
const manualHook = newNode(
  "m",
  "Avisar",
  { type: "integration", integrationId: "avisar" },
  { x: 1040, y: 400 },
);

const integrations = integrationsById([
  {
    id: "lint",
    name: "Lint",
    script: "eslint .",
    trigger: { type: "onInput", kind: "diff" },
    output: "log",
    file: null,
  },
  {
    id: "avisar",
    name: "Avisar",
    script: "echo pronto",
    trigger: { type: "manual" },
    output: "json",
    file: null,
  },
] satisfies Integration[]);

const nodes: MaestroNode[] = [claude, tests, note, lint, manualHook];

const graph: Graph = {
  id: "g1",
  name: "rota-saude",
  dir: "/tmp/rota-saude",
  variables: { porta: "5173" },
  nodes,
  edges: [
    { id: "e1", from: "a", to: "t", kind: "diff" },
    { id: "e2", from: "n", to: "a", kind: "file" },
  ],
};

describe("produces / accepts", () => {
  test("seguem a tabela de rules", () => {
    expect(produces(claude.kind, integrations)).toEqual(["context", "diff"]);
    expect(produces(tests.kind, integrations)).toEqual(["json"]);
    expect(produces(note.kind, integrations)).toEqual(["context", "file"]);
    expect(produces(lint.kind, integrations)).toEqual(["log"]);
    expect(accepts(note.kind, integrations)).toEqual([]);
    expect(accepts(claude.kind, integrations)).toHaveLength(5);
    expect(accepts(lint.kind, integrations)).toEqual(["diff"]);
    expect(accepts(manualHook.kind, integrations)).toHaveLength(5);
  });

  test("integração apagada não produz nem aceita nada", () => {
    const orphan = { type: "integration", integrationId: "sumiu" } as const;
    expect(produces(orphan, integrations)).toEqual([]);
    expect(accepts(orphan, integrations)).toEqual([]);
  });
});

describe("connectionProblem", () => {
  const check = (from: string, to: string, kind: Parameters<typeof connectionProblem>[5]) =>
    connectionProblem(nodes, graph.edges, integrations, from, to, kind);

  test("ligação válida", () => {
    expect(check("a", "i", "diff")).toBeNull();
    expect(check("t", "m", "json")).toBeNull();
  });

  test("nota não recebe nada", () => {
    expect(check("a", "n", "diff")).toBe("notAccepted");
  });

  test("tipo que a origem não produz", () => {
    expect(check("t", "i", "diff")).toBe("notProduced");
    expect(check("n", "t", "diff")).toBe("notProduced");
  });

  test("integração por gatilho só aceita o tipo do gatilho", () => {
    expect(check("a", "i", "context")).toBe("notAccepted");
  });

  test("ciclo, laço e repetida recusados", () => {
    // t → a fecharia a → t → a.
    expect(check("t", "a", "json")).toBe("cycle");
    expect(check("a", "a", "context")).toBe("self");
    expect(check("a", "t", "diff")).toBe("duplicate");
  });

  test("nó inexistente", () => {
    expect(check("x", "t", "log")).toBe("unknownNode");
  });
});

describe("toFlow / fromFlow", () => {
  test("round-trip preserva o grafo", () => {
    const flow = toFlow(graph);
    expect(fromFlow(graph, flow.nodes, flow.edges)).toEqual(graph);
  });

  test("portas: saída por tipo, entrada por ligação", () => {
    const { nodes: fn, edges: fe } = toFlow(graph);
    expect(fn[0]).toMatchObject({
      id: "a",
      type: "agent",
      position: { x: 40, y: 40 },
      width: 360,
      height: 300,
    });
    expect(fe[0]).toMatchObject({
      source: "a",
      target: "t",
      sourceHandle: outHandle("diff"),
      targetHandle: inHandle("e1"),
      data: { kind: "diff" },
    });
  });

  test("posição e tamanho saem do canvas", () => {
    const flow = toFlow(graph);
    const moved = flow.nodes.map((n) =>
      n.id === "a" ? { ...n, position: { x: 10, y: 20 }, width: 500, height: 420 } : n,
    );
    const back = fromFlow(graph, moved, flow.edges);
    expect(back.nodes[0].position).toEqual({ x: 10, y: 20 });
    expect(back.nodes[0].size).toEqual({ w: 500, h: 420 });
  });

  test("ligação sem data tira o tipo da porta de saída", () => {
    const back = fromFlow(graph, toFlow(graph).nodes, [
      { id: "e9", source: "a", target: "i", sourceHandle: outHandle("diff") },
      { id: "e10", source: "a", target: "i", sourceHandle: "lixo" },
    ]);
    expect(back.edges).toEqual([{ id: "e9", from: "a", to: "i", kind: "diff" }]);
  });

  test("kindOfOutHandle", () => {
    expect(kindOfOutHandle("out:json")).toBe("json");
    expect(kindOfOutHandle("out:nada")).toBeNull();
    expect(kindOfOutHandle(inHandle("e1"))).toBeNull();
    expect(kindOfOutHandle(null)).toBeNull();
  });
});

test("emptyGraph usa o nome da pasta", () => {
  expect(emptyGraph("g", "/Users/x/rota-saude/")).toMatchObject({
    name: "rota-saude",
    dir: "/Users/x/rota-saude/",
    nodes: [],
    edges: [],
  });
});
