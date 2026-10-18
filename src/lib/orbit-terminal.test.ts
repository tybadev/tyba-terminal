import { describe, expect, test } from "bun:test";

import {
  isLiveRun,
  liveRunNodes,
  newNode,
  nodeControls,
  nodeSession,
  terminalLive,
  terminalTitle,
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
