import { describe, expect, test } from "bun:test";

import { staleTerminals, terminalStaleVars, toggleDrawer, type NodeRun, type Run } from "./orbit";

// Fixtures sintéticas (quadro GavetasOrbit): nada aqui veio de sessão real.
const nodeRun = (patch: Partial<NodeRun> = {}): NodeRun => ({
  status: "running",
  sessionId: "shell",
  startedAt: 1,
  finishedAt: null,
  exitCode: null,
  outputs: {},
  reason: null,
  ...patch,
});

const terminal = (id: string, node: string, staleVars: string[], patch: Partial<Run> = {}): Run => ({
  id,
  graphId: "g",
  startedAt: 0,
  finishedAt: null,
  status: "running",
  nodes: { [node]: nodeRun() },
  terminal: node,
  staleVars,
  ...patch,
});

describe("gavetas: uma por vez", () => {
  test("abrir troca a de cima; o mesmo botão fecha", () => {
    expect(toggleDrawer(null, "vars")).toEqual({ kind: "vars" });
    expect(toggleDrawer({ kind: "vars" }, "ints")).toEqual({ kind: "ints" });
    expect(toggleDrawer({ kind: "ints" }, "roles")).toEqual({ kind: "roles", start: {} });
    expect(toggleDrawer({ kind: "roles", start: {} }, "roles")).toBeNull();
    expect(toggleDrawer({ kind: "vars" }, "vars")).toBeNull();
  });
});

describe("aviso de reaplicar (O2)", () => {
  test("sem Terminal ao vivo com variável mudada, sem aviso", () => {
    expect(staleTerminals([])).toEqual({ nodes: [], vars: [] });
    expect(staleTerminals([terminal("r1", "t", [])])).toEqual({ nodes: [], vars: [] });
  });

  test("um chip por nó afetado e as variáveis que mudaram, sem repetir", () => {
    const runs = [terminal("r1", "dev", ["API_URL"]), terminal("r2", "logs", ["API_URL", "MODELO"])];
    expect(staleTerminals(runs)).toEqual({ nodes: ["dev", "logs"], vars: ["API_URL", "MODELO"] });
    expect(terminalStaleVars(runs, "logs")).toEqual(["API_URL", "MODELO"]);
  });

  test("Terminal fechado (parado para reabrir) sai do aviso", () => {
    const closed = terminal("r1", "dev", ["API_URL"], { status: "stopped" });
    expect(staleTerminals([closed])).toEqual({ nodes: [], vars: [] });
    expect(terminalStaleVars([closed], "dev")).toEqual([]);
  });

  test("run do ▶ não conta", () => {
    const played = terminal("r1", "dev", ["API_URL"], { terminal: null });
    expect(staleTerminals([played])).toEqual({ nodes: [], vars: [] });
  });
});
