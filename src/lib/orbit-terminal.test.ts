import { describe, expect, test } from "bun:test";

import {
  createdFlow,
  hostedAgentOf,
  importItems,
  isLiveRun,
  isOrchestrator,
  liveRunNodes,
  newNode,
  nodeControls,
  nodeSession,
  shownSession,
  terminalLive,
  terminalRun,
  terminalTitle,
  type Graph,
  type NodeRun,
  type Run,
} from "./orbit";

// Fixtures sintéticas (nó Terminal, O1): nada aqui veio de sessão real.
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

const termKind = { type: "terminal", dir: "/srv/api" } as const;

describe("nó Terminal: sessão ao vivo", () => {
  const live = run(
    {
      t: nodeRun({ status: "running", sessionId: "shell-1" }),
      c: nodeRun({ status: "running", sessionId: "s-c" }),
    },
    { terminal: "t" },
  );

  test("o Terminal aberto mostra o shell dele", () => {
    expect(nodeSession(termKind, live, "t", true)).toBe("shell-1");
    expect(terminalLive(live, "t")).toBe(true);
  });

  test("fechado (run concluído), sem terminal e sem 'ao vivo'", () => {
    const closed = run({ t: nodeRun({ status: "done", sessionId: "shell-1" }) }, {
      terminal: "t",
      status: "done",
    });
    expect(nodeSession(termKind, closed, "t", true)).toBeNull();
    expect(terminalLive(closed, "t")).toBe(false);
    expect(isLiveRun(closed)).toBe(false);
  });

  test("o run do ▶ não é sessão ao vivo", () => {
    const played = run({ t: nodeRun({ status: "running", sessionId: "x" }) }, { terminal: null });
    expect(isLiveRun(played)).toBe(false);
    expect(terminalLive(played, "t")).toBe(false);
    expect(isLiveRun(null)).toBe(false);
  });

  test("só o nó que segura a sessão está ao vivo", () => {
    expect(terminalLive(live, "c")).toBe(false);
    expect(isLiveRun(live)).toBe(true);
  });

  test("o ⋯ do Terminal não oferece parar/reiniciar (o caminho é o Fechar)", () => {
    expect(nodeControls(termKind, live, "t", true)).toEqual({
      stop: false,
      restart: false,
      rerun: false,
    });
  });

  test("a faixa lista o Terminal primeiro e depois o que ele criou, só os da sessão", () => {
    const graph = {
      nodes: [
        newNode("p", "p", { type: "note", markdown: "" }, { x: 0, y: 0 }),
        newNode("c", "c", { type: "browser", url: "" }, { x: 0, y: 0 }),
        newNode("t", "t", termKind, { x: 0, y: 0 }),
      ],
    };
    expect(liveRunNodes(graph, live)).toEqual(["t", "c"]);
  });
});

describe("terminalTitle", () => {
  test("pelo nome da pasta", () => {
    expect(terminalTitle("/srv/api")).toBe("Terminal · api");
    expect(terminalTitle("/srv/api/")).toBe("Terminal · api");
    expect(terminalTitle("C:\\work\\web")).toBe("Terminal · web");
  });
  test("sem pasta, só Terminal", () => {
    expect(terminalTitle("")).toBe("Terminal");
  });
});

describe("Terminal aberto com outro run à vista", () => {
  const held = run({ t: nodeRun({ status: "running", sessionId: "shell-1" }) }, { id: "live", terminal: "t" });
  // Testar uma integração abre run novo e ele vai para a vista.
  const tested = run(
    { i: nodeRun({ status: "running", sessionId: "s-i" }) },
    { id: "test", terminal: null },
  );
  const runs = [tested, held];

  test("a sessão ao vivo vem do run que segura o Terminal, não do à vista", () => {
    expect(terminalRun(runs, "t")?.id).toBe("live");
    expect(shownSession(termKind, tested, runs, "t", true)).toBe("shell-1");
    // Mesmo com o run à vista sendo o velho (live = false).
    expect(shownSession(termKind, tested, runs, "t", false)).toBe("shell-1");
  });

  test("sem sessão ao vivo que o segure, fechado", () => {
    const closed = { ...held, status: "done" as const };
    expect(terminalRun([tested, closed], "t")).toBeNull();
    expect(shownSession(termKind, held, [tested, closed], "t", true)).toBeNull();
    expect(terminalRun(runs, "i")).toBeNull();
  });

  test("os outros nós seguem o run à vista", () => {
    const proc = { type: "process", command: "ls", output: "log", file: null, dir: "" } as const;
    expect(shownSession(proc, tested, runs, "i", true)).toBe("s-i");
    expect(shownSession(proc, held, runs, "i", true)).toBeNull();
  });
});

describe("Terminal como orquestrador", () => {
  test("agente e Terminal criam; os outros não", () => {
    expect(isOrchestrator(termKind)).toBe(true);
    expect(isOrchestrator({ type: "agent" })).toBe(true);
    expect(isOrchestrator({ type: "process" })).toBe(false);
    expect(isOrchestrator(undefined)).toBe(false);
  });

  test("o criado pelo claude digitado no Terminal ganha a tracejada saindo do Terminal", () => {
    const term = newNode("t", "Terminal · api", termKind, { x: 0, y: 0 });
    const child = newNode("n1", "web", { type: "browser", url: "" }, { x: 0, y: 0 });
    const r = run({}, {
      terminal: "t",
      created: [{ node: child, createdBy: "t", createdAt: 1, edges: [], kept: false }],
    });
    expect(createdFlow(r, [term]).byEdges.map((e) => [e.source, e.target, e.sourceHandle])).toEqual([
      ["t", "n1", "orchestrator"],
    ]);
  });
});

describe("importação com Terminal", () => {
  test("o Terminal entra na prévia com a pasta dele", () => {
    const node = newNode("t", "Terminal · api", termKind, { x: 0, y: 0 });
    const graph: Graph = { id: "g", name: "g", dir: "/tmp", variables: {}, edges: [], nodes: [node] };
    expect(importItems({ graph, integrations: [] })).toEqual([{ type: "terminal", node, dir: "/srv/api" }]);
  });
});

describe("hostedAgentOf", () => {
  const live = (patch: Partial<NodeRun>) =>
    run({ t: nodeRun({ status: "running", sessionId: "shell-1", ...patch }) }, { id: "live", terminal: "t" });

  test("o agente com as ferramentas do Orbit, do run que segura o Terminal", () => {
    expect(hostedAgentOf([live({ hostedAgent: "claude" })], "t")).toBe("claude");
    expect(hostedAgentOf([live({ hostedAgent: "codex" })], "t")).toBe("codex");
  });

  test("sem agente, sem o campo, ou Terminal fechado: null", () => {
    expect(hostedAgentOf([live({ hostedAgent: null })], "t")).toBeNull();
    expect(hostedAgentOf([live({})], "t")).toBeNull();
    const closed = { ...live({ hostedAgent: "claude" }), status: "done" as const };
    expect(hostedAgentOf([closed], "t")).toBeNull();
    expect(hostedAgentOf([live({ hostedAgent: "claude" })], "outro")).toBeNull();
  });
});
