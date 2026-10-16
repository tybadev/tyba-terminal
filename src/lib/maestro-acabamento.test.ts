import { describe, expect, test } from "bun:test";

import {
  agentSessions,
  findSessionNode,
  integrationsById,
  invalidEdges,
  newNode,
  nodeSession,
  nodeUnderPointer,
  routeNodeChanges,
  runCreated,
  type Created,
  type Integration,
  type MaestroEdge,
  type NodeRun,
  type Run,
} from "./maestro";

// Fixtures sintéticas (Fase 1 do acabamento): nada aqui veio de sessão real.
const nodeRun = (patch: Partial<NodeRun> = {}): NodeRun => ({
  status: "waiting",
  sessionId: null,
  startedAt: null,
  finishedAt: null,
  exitCode: null,
  outputs: {},
  reason: null,
  ...patch,
});

const run = (nodes: Record<string, NodeRun>, patch: Partial<Run> = {}): Run => ({
  id: "r1",
  graphId: "g",
  startedAt: 0,
  finishedAt: null,
  status: "running",
  nodes,
  ...patch,
});

const agentKind = { type: "agent", runner: "claude", role: "", prompt: "", dir: "/srv" } as const;
const agent = (id: string) => newNode(id, id, { ...agentKind }, { x: 0, y: 0 });
const proc = (id: string, output: "log" | "json" = "log") =>
  newNode(id, id, { type: "process", command: "bun dev", output, file: null, dir: "/srv" }, { x: 0, y: 0 });
const browser = (id: string) => newNode(id, id, { type: "browser", url: "" }, { x: 0, y: 0 });

describe("nos-4: agente morto não aparece como terminal", () => {
  const done = run(
    { a: nodeRun({ status: "done", sessionId: "s-a" }) },
    { status: "done", liveAgents: [] },
  );

  test("agente done cuja sessão saiu não tem terminal (volta o prompt)", () => {
    expect(nodeSession(agentKind, done, "a", true)).toBeNull();
    // Run gravado sem `liveAgents` (app reiniciado): também não.
    expect(nodeSession(agentKind, { ...done, liveAgents: undefined }, "a", true)).toBeNull();
  });

  test("agente done com a sessão viva no core segue com o terminal", () => {
    expect(nodeSession(agentKind, { ...done, liveAgents: ["s-a"] }, "a", true)).toBe("s-a");
    // Run antigo à vista: os terminais vivos são do mais novo.
    expect(nodeSession(agentKind, { ...done, liveAgents: ["s-a"] }, "a", false)).toBeNull();
  });

  test("nó rodando mostra o terminal", () => {
    const r = run({ p: nodeRun({ status: "running", sessionId: "s-p" }) });
    expect(nodeSession(proc("p").kind, r, "p", false)).toBe("s-p");
    expect(nodeSession(proc("p").kind, r, "x", true)).toBeNull();
  });
});

describe("nos-7: subagentes de agente criado viram nós", () => {
  const created = (node: Created["node"]): Created => ({
    node,
    createdBy: "o",
    createdAt: 1,
    edges: [],
    kept: false,
  });

  test("o agente que o orquestrador criou entra nas sessões que dão filhos", () => {
    const r = run(
      {
        o: nodeRun({ status: "running", sessionId: "s-o" }),
        c: nodeRun({ status: "running", sessionId: "s-c" }),
      },
      { created: [created(agent("c"))] },
    );
    const sessions = agentSessions([agent("o")], r, true);
    expect([...sessions]).toEqual([
      ["o", "s-o"],
      ["c", "s-c"],
    ]);
  });

  test("sem run, nada", () => {
    expect(agentSessions([agent("o")], null, true).size).toBe(0);
  });
});

describe("canvas-2: ligação que deixou de valer", () => {
  const ints = integrationsById([]);
  const edge = (id: string, from: string, to: string, kind: MaestroEdge["kind"]): MaestroEdge => ({
    id,
    from,
    to,
    kind,
  });

  test("trocar a saída do processo de log para json invalida a ligação log", () => {
    const edges = [edge("e1", "p", "a", "log"), edge("e2", "p", "b", "url")];
    const before = [proc("p", "log"), agent("a"), browser("b")];
    expect(invalidEdges(before, edges, ints)).toEqual([]);
    const after = [proc("p", "json"), agent("a"), browser("b")];
    expect(invalidEdges(after, edges, ints).map((e) => e.id)).toEqual(["e1"]);
  });

  test("integração salva com outro gatilho invalida a entrada que não aceita mais", () => {
    const integration: Integration = {
      id: "i",
      name: "Lint",
      script: "eslint .",
      trigger: { type: "onInput", kind: "diff" },
      output: "log",
      file: null,
    };
    const node = newNode("n", "Lint", { type: "integration", integrationId: "i", dir: "/srv" }, { x: 0, y: 0 });
    const edges = [edge("e1", "a", "n", "diff")];
    const nodes = [agent("a"), node];
    expect(invalidEdges(nodes, edges, integrationsById([integration]))).toEqual([]);
    const changed = { ...integration, trigger: { type: "onInput", kind: "log" } as const };
    expect(invalidEdges(nodes, edges, integrationsById([changed])).map((e) => e.id)).toEqual(["e1"]);
  });
});

describe("canvas-1: soltar no corpo do nó", () => {
  const el = (id: string | null) => ({
    closest: (sel: string) =>
      sel === ".react-flow__node" && id !== null ? { getAttribute: (n: string) => (n === "data-id" ? id : null) } : null,
  });

  test("o nó sob o ponteiro sai do data-id do invólucro do xyflow", () => {
    expect(nodeUnderPointer(el("b"))).toBe("b");
  });

  test("fora de nó (canvas vazio) ou sem elemento: nenhum", () => {
    expect(nodeUnderPointer(el(null))).toBeNull();
    expect(nodeUnderPointer(null)).toBeNull();
  });
});

describe("nos-1: criados respondem (mexer sim, apagar não)", () => {
  const created = new Set(["c"]);
  const isEphemeral = (id: string) => id.startsWith("sub:");

  test("arrastar, redimensionar e selecionar o criado vão para a vista dele", () => {
    const changes = [
      { type: "position", id: "c", position: { x: 10, y: 20 } },
      { type: "dimensions", id: "c" },
      { type: "select", id: "c", selected: true },
      { type: "remove", id: "c" },
      { type: "position", id: "g", position: { x: 1, y: 1 } },
      { type: "select", id: "sub:1", selected: true },
    ];
    const { graph, created: mine } = routeNodeChanges(changes, created, isEphemeral);
    expect(mine.map((c) => c.type)).toEqual(["position", "dimensions", "select"]);
    expect(graph.map((c) => ("id" in c ? c.id : null))).toEqual(["g"]);
  });

  test("mudança sem id (add) é do grafo", () => {
    const add = { type: "add", item: {} };
    expect(routeNodeChanges([add], created, isEphemeral).graph).toEqual([add]);
  });
});

describe("orquestrador-9: Manter vale depois do fim", () => {
  const c: Created = { node: proc("c"), createdBy: "o", createdAt: 1, edges: [], kept: false };

  test("o painel lista os criados do run terminado ou parado", () => {
    expect(runCreated(run({}, { status: "done", created: [c] }))).toEqual([c]);
    expect(runCreated(run({}, { status: "stopped", created: [c] }))).toEqual([c]);
    expect(runCreated(null)).toEqual([]);
  });
});

describe("processo-7: ir ao nó de uma sessão", () => {
  test("acha a execução e o nó (inclusive criado) pela sessão", () => {
    const runs = [
      run({ a: nodeRun({ status: "running", sessionId: "s-a" }) }, { id: "r2" }),
      run({ c: nodeRun({ status: "done", sessionId: "s-c" }) }, { id: "r1" }),
    ];
    expect(findSessionNode(runs, "s-c")).toEqual({ runId: "r1", nodeId: "c" });
    expect(findSessionNode(runs, "s-x")).toBeNull();
  });
});
