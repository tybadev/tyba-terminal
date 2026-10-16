import { describe, expect, test } from "bun:test";

import {
  EMPTY_RUN_VIEW,
  KEEP_RUNS,
  concurrentAgents,
  edgeDetail,
  formatDuration,
  newNode,
  nodeElapsed,
  nodeStatusLabel,
  runNumber,
  runProgress,
  runReducer,
  selectedRun,
  stoppableRuns,
  timelineSteps,
  type Graph,
  type OrbitEdge,
  type NodeRun,
  type Run,
} from "./orbit";

// Fixtures sintéticas: nada aqui veio de sessão real.
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

const run = (id: string, startedAt: number, nodes: Record<string, NodeRun>, patch: Partial<Run> = {}): Run => ({
  id,
  graphId: "g",
  startedAt,
  finishedAt: null,
  status: "running",
  nodes,
  ...patch,
});

const agent = (id: string, title: string) =>
  newNode(id, title, { type: "agent", runner: "claude", role: "", prompt: "", dir: "" }, { x: 0, y: 0 });
const proc = (id: string, title: string) =>
  newNode(id, title, { type: "process", command: "bun test", output: "json", file: null, dir: "" }, { x: 0, y: 0 });
const note = newNode("n", "Decisões", { type: "note", markdown: "- UTC" }, { x: 0, y: 0 });
const edge = (from: string, to: string, kind: OrbitEdge["kind"]): OrbitEdge => ({
  id: `${from}-${to}-${kind}`,
  from,
  to,
  kind,
});

// API → Testes → Codex; Dev server em paralelo com a API; nota alimenta o Codex.
const graph: Pick<Graph, "nodes" | "edges"> = {
  nodes: [agent("a", "Claude · API"), proc("d", "Dev server"), proc("t", "Testes"), agent("c", "Codex"), note],
  edges: [edge("a", "t", "diff"), edge("d", "t", "log"), edge("t", "c", "json"), edge("n", "c", "file")],
};

describe("stoppableRuns", () => {
  test("Parar alcança o run rodando e o run fechado com agente vivo", () => {
    const view = runReducer(EMPTY_RUN_VIEW, {
      type: "history",
      runs: [
        run("rodando", 300, { a: nodeRun({ status: "running" }) }),
        run("vivo", 200, { a: nodeRun({ status: "done" }) }, { status: "done", liveAgents: ["7"] }),
        run("morto", 100, { a: nodeRun({ status: "done" }) }, { status: "done", liveAgents: [] }),
      ],
    });
    expect(stoppableRuns(view).map((r) => r.id)).toEqual(["rodando", "vivo"]);
  });

  test("sem run rodando nem agente vivo, Parar fica desabilitado", () => {
    const view = runReducer(EMPTY_RUN_VIEW, {
      type: "history",
      runs: [run("r", 100, { a: nodeRun({ status: "done" }) }, { status: "done" })],
    });
    expect(stoppableRuns(view)).toEqual([]);
  });
});

describe("runReducer", () => {
  test("run novo entra no topo e vira o run à vista", () => {
    const r1 = run("r1", 100, { a: nodeRun() });
    const r2 = run("r2", 200, { a: nodeRun() });
    let view = runReducer(EMPTY_RUN_VIEW, { type: "update", run: r1 });
    view = runReducer(view, { type: "update", run: r2 });
    expect(view.runs.map((r) => r.id)).toEqual(["r2", "r1"]);
    expect(view.selectedId).toBe("r2");
  });

  test("atualização de um run conhecido não troca a vista do dono", () => {
    let view = runReducer(EMPTY_RUN_VIEW, {
      type: "history",
      runs: [run("r2", 200, { a: nodeRun() }), run("r1", 100, { a: nodeRun() })],
    });
    view = runReducer(view, { type: "select", runId: "r1" });
    view = runReducer(view, { type: "update", run: run("r2", 200, { a: nodeRun({ status: "running" }) }) });
    expect(view.selectedId).toBe("r1");
    expect(selectedRun(view)?.id).toBe("r1");
    expect(view.runs[0].nodes.a.status).toBe("running");
  });

  test("retrato mais velho que o guardado é ignorado", () => {
    const fresh = run("r", 100, { a: nodeRun({ status: "done", sessionId: "7" }) });
    const stale = run("r", 100, { a: nodeRun({ status: "running", sessionId: "7" }) });
    let view = runReducer(EMPTY_RUN_VIEW, { type: "update", run: fresh });
    view = runReducer(view, { type: "update", run: stale });
    expect(view.runs[0].nodes.a.status).toBe("done");
    expect(runProgress(fresh)).toBeGreaterThan(runProgress(stale));
  });

  test("sessão do nó chegando conta como progresso", () => {
    const before = run("r", 100, { a: nodeRun({ status: "running" }) });
    const after = run("r", 100, { a: nodeRun({ status: "running", sessionId: "9" }) });
    let view = runReducer(EMPTY_RUN_VIEW, { type: "update", run: before });
    view = runReducer(view, { type: "update", run: after });
    expect(view.runs[0].nodes.a.sessionId).toBe("9");
  });

  test("histórico não apaga o que já chegou por evento", () => {
    const live = run("r3", 300, { a: nodeRun({ status: "running" }) });
    let view = runReducer(EMPTY_RUN_VIEW, { type: "update", run: live });
    view = runReducer(view, {
      type: "history",
      runs: [run("r3", 300, { a: nodeRun() }), run("r1", 100, { a: nodeRun({ status: "done" }) }, { status: "done" })],
    });
    expect(view.runs.map((r) => r.id)).toEqual(["r3", "r1"]);
    expect(view.runs[0].nodes.a.status).toBe("running");
    expect(view.selectedId).toBe("r3");
  });

  test("guarda só as últimas e numera da mais antiga", () => {
    const many = Array.from({ length: KEEP_RUNS + 3 }, (_, i) => run(`r${i}`, i, {}));
    const view = runReducer(EMPTY_RUN_VIEW, { type: "history", runs: many });
    expect(view.runs).toHaveLength(KEEP_RUNS);
    expect(view.selectedId).toBe(`r${KEEP_RUNS + 2}`);
    expect(runNumber(view, `r${KEEP_RUNS + 2}`)).toBe(KEEP_RUNS);
    expect(runNumber(view, view.runs[KEEP_RUNS - 1].id)).toBe(1);
  });

  test("selecionar run desconhecido não muda nada; reset limpa", () => {
    const view = runReducer(EMPTY_RUN_VIEW, { type: "update", run: run("r", 1, {}) });
    expect(runReducer(view, { type: "select", runId: "x" })).toBe(view);
    expect(runReducer(view, { type: "reset" })).toEqual(EMPTY_RUN_VIEW);
  });
});

describe("etiqueta da ligação", () => {
  test("sem execução não há segunda linha", () => {
    expect(edgeDetail("diff", undefined)).toBeNull();
  });

  test("segue o estado do nó de origem", () => {
    expect(edgeDetail("diff", nodeRun())?.key).toBe("orbitEdgeWaiting");
    expect(edgeDetail("log", nodeRun({ status: "running" }))?.key).toBe("orbitEdgeLive");
    expect(edgeDetail("json", nodeRun({ status: "failed" }))?.key).toBe("orbitEdgeSourceFailed");
    expect(edgeDetail("json", nodeRun({ status: "skipped" }))?.key).toBe("orbitEdgeSkipped");
    expect(edgeDetail("json", nodeRun({ status: "stopped" }))?.key).toBe("orbitEdgeStopped");
  });

  test("pronto com valor, vazio sem; arquivo mostra só o nome", () => {
    const done = nodeRun({
      status: "done",
      outputs: { diff: "/runs/r/a/out/diff.patch", file: "/runs/r/n/out/Decisões.md" },
    });
    expect(edgeDetail("diff", done)).toEqual({ key: "orbitEdgeReady" });
    expect(edgeDetail("file", done)).toEqual({
      key: "orbitEdgeReadyFile",
      params: { name: "Decisões.md" },
    });
    expect(edgeDetail("context", done)).toEqual({ key: "orbitEdgeEmpty" });
  });
});

describe("linha da execução", () => {
  test("passos agrupados por profundidade, nota fora", () => {
    expect(timelineSteps(graph)).toEqual([["a", "d"], ["t"], ["c"]]);
  });

  test("aguardando diz o tipo e o nó que falta", () => {
    const r = run("r", 0, {
      a: nodeRun({ status: "running" }),
      d: nodeRun({ status: "done" }),
      t: nodeRun(),
      c: nodeRun(),
      n: nodeRun({ status: "done" }),
    });
    expect(nodeStatusLabel(graph, r, "t")).toEqual({
      key: "orbitWaitingFor",
      params: { kind: "diff", node: "Claude · API" },
    });
    expect(nodeStatusLabel(graph, r, "a")).toEqual({ key: "orbitStatus_running" });
    expect(nodeStatusLabel(graph, { ...r, status: "stopped" }, "t")).toEqual({
      key: "orbitStatus_waiting",
    });
    expect(nodeStatusLabel(graph, r, "x")).toBeNull();
  });

  test("orquestrador que fechou o turno: orquestrando e já entrega as saídas", () => {
    const r = run("r", 0, {
      a: nodeRun({ status: "running", orchestrating: true, outputs: { diff: "/r/a/out/diff.patch" } }),
      d: nodeRun({ status: "done" }),
      t: nodeRun(),
      c: nodeRun(),
      n: nodeRun({ status: "done" }),
    });
    expect(nodeStatusLabel(graph, r, "a")).toEqual({ key: "orbitStatus_orchestrating" });
    expect(nodeStatusLabel(graph, r, "t")).toEqual({ key: "orbitStatus_waiting" });
    expect(edgeDetail("diff", r.nodes.a)).toEqual({ key: "orbitEdgeReady" });
    expect(edgeDetail("context", r.nodes.a)).toEqual({ key: "orbitEdgeEmpty" });
  });

  test("tempo do nó e formato", () => {
    expect(nodeElapsed(undefined, 10)).toBeNull();
    expect(nodeElapsed(nodeRun({ status: "running", startedAt: 1_000 }), 191_000)).toBe(190_000);
    expect(nodeElapsed(nodeRun({ status: "done", startedAt: 1_000, finishedAt: 43_000 }), 999_999)).toBe(42_000);
    expect(formatDuration(42_000)).toBe("42s");
    expect(formatDuration(190_000)).toBe("3m10s");
    expect(formatDuration(3_720_000)).toBe("1h02m");
  });
});

describe("agentes em paralelo", () => {
  test("agentes em sequência não avisam", () => {
    expect(concurrentAgents(graph)).toBeNull();
  });

  test("agentes em paralelo em pastas diferentes não avisam", () => {
    const a = agent("a", "A");
    const b = agent("b", "B");
    if (a.kind.type === "agent") a.kind = { ...a.kind, dir: "/x" };
    if (b.kind.type === "agent") b.kind = { ...b.kind, dir: "/y" };
    expect(concurrentAgents({ nodes: [a, b], edges: [] })).toBeNull();
  });

  test("dois agentes sem dependência entre si avisam", () => {
    const pair = concurrentAgents({ nodes: [agent("a", "A"), agent("b", "B")], edges: [] });
    expect(pair?.map((n) => n.id)).toEqual(["a", "b"]);
  });
});
