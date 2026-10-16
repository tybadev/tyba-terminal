import { describe, expect, test } from "bun:test";

import {
  NEW_IN_HANDLE,
  ORCHESTRATOR_HANDLE,
  createdByOf,
  createdFlow,
  ephemeralCreated,
  importItems,
  newNode,
  type Created,
  type Graph,
  type Run,
} from "./orbit";

// Fixtures sintéticas: nada aqui veio de sessão real.
const created = (id: string, patch: Partial<Created> = {}): Created => ({
  node: newNode(
    id,
    id,
    { type: "process", command: "bun run dev", output: "log", file: null, dir: "/srv/api" },
    { x: 0, y: 0 },
  ),
  createdBy: "o",
  createdAt: 1_000,
  edges: [],
  kept: false,
  ...patch,
});

const run = (status: Run["status"], list: Created[]): Run => ({
  id: "r",
  graphId: "g",
  startedAt: 0,
  finishedAt: null,
  status,
  nodes: {},
  created: list,
});

describe("orquestrador: nós criados", () => {
  test("só o run rodando desenha os criados, e só os ainda não mantidos", () => {
    const list = [created("orq-1"), created("orq-2", { kept: true })];
    expect(ephemeralCreated(run("running", list)).map((c) => c.node.id)).toEqual(["orq-1"]);
    // ■ Parar ou fim do run: somem.
    expect(ephemeralCreated(run("stopped", list))).toEqual([]);
    expect(ephemeralCreated(run("done", list))).toEqual([]);
    expect(ephemeralCreated(null)).toEqual([]);
    // Run gravado antes do orquestrador não tem `created`.
    expect(ephemeralCreated({ ...run("running", []), created: undefined })).toEqual([]);
  });

  test("a prévia de importação mostra o agente sem o antigo orquestrar", () => {
    const graph: Graph = {
      id: "g",
      name: "subir",
      dir: "/tmp",
      variables: {},
      edges: [],
      nodes: [
        newNode("a", "Claude", { type: "agent", runner: "claude", role: "", prompt: "x", dir: "" }, {
          x: 0,
          y: 0,
        }),
      ],
    };
    const items = importItems({ graph, integrations: [] });
    expect(items[0]).toEqual({ type: "agent", node: graph.nodes[0], runner: "claude", role: "", prompt: "x" });
  });
});

describe("orquestrador: marca e ligação dos criados no canvas", () => {
  const orchestrator = newNode(
    "o",
    "Claude",
    { type: "agent", runner: "claude", role: "", prompt: "", dir: "/srv" },
    { x: 0, y: 0 },
  );
  const agent = (id: string, patch: Partial<Created> = {}) =>
    created(id, {
      node: newNode(id, id, { type: "agent", runner: "claude", role: "", prompt: "ls", dir: "/srv" }, { x: 0, y: 0 }),
      ...patch,
    });

  test("run rodando com criados: viram nós efêmeros com a tracejada do criador", () => {
    const r = run("running", [agent("orq-1"), agent("orq-2"), agent("orq-3")]);
    const flow = createdFlow(r, [orchestrator]);
    expect(flow.nodes.map((n) => n.id)).toEqual(["orq-1", "orq-2", "orq-3"]);
    expect(flow.byEdges).toEqual(
      ["orq-1", "orq-2", "orq-3"].map((id) => ({
        id: `${id}:by`,
        source: "o",
        target: id,
        sourceHandle: ORCHESTRATOR_HANDLE,
        targetHandle: NEW_IN_HANDLE,
      })),
    );
    expect([...createdByOf(r).entries()]).toEqual([
      ["orq-1", "o"],
      ["orq-2", "o"],
      ["orq-3", "o"],
    ]);
  });

  // O caso do teste do dono: os 3 agentes foram mantidos e o run fechou.
  test("mantidos (já no grafo) seguem marcados e ligados com o run à vista, mesmo fechado", () => {
    const list = ["orq-1", "orq-2", "orq-3"].map((id) => agent(id, { kept: true }));
    const r = run("done", list);
    const flow = createdFlow(r, [orchestrator, ...list.map((c) => c.node)]);
    expect(flow.nodes).toEqual([]);
    expect(flow.byEdges.map((e) => [e.source, e.target])).toEqual([
      ["o", "orq-1"],
      ["o", "orq-2"],
      ["o", "orq-3"],
    ]);
    expect(createdByOf(r).has("orq-2")).toBe(true);
  });

  test("sem tracejada quando o criador não está no grafo ou o criado não está desenhado", () => {
    // O criador saiu do grafo (o dono o apagou): não há de onde sair.
    expect(createdFlow(run("running", [agent("orq-1")]), []).byEdges).toEqual([]);
    // Descartado (não mantido) com o run fechado: não existe mais no canvas.
    expect(createdFlow(run("stopped", [agent("orq-1")]), [orchestrator]).byEdges).toEqual([]);
  });

  test("ligação url do efêmero só com a origem desenhada", () => {
    const browser = created("orq-2", {
      node: newNode("orq-2", "code", { type: "browser", url: "" }, { x: 0, y: 0 }),
      edges: [{ id: "orq-2-url", from: "orq-1", to: "orq-2", kind: "url" }],
    });
    expect(createdFlow(run("running", [browser]), [orchestrator]).edges).toEqual([]);
    const both = createdFlow(run("running", [created("orq-1"), browser]), [orchestrator]);
    expect(both.edges.map((e) => [e.source, e.target, e.sourceHandle])).toEqual([["orq-1", "orq-2", "out"]]);
  });
});
