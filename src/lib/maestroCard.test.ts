import { describe, expect, test } from "bun:test";

import type { ApprovalRequest } from "./ipc";
import type { NodeRun } from "./maestro";
import { nodeApproval, ringState, visibleCardApprovalIds } from "./maestroCard";

// Fixtures sintéticas: nada aqui veio de sessão real.
const nodeRun = (patch: Partial<NodeRun>): NodeRun => ({
  status: "running",
  sessionId: "s1",
  startedAt: 1,
  finishedAt: null,
  exitCode: null,
  outputs: {},
  reason: null,
  ...patch,
});

const request = (id: number, session: string, at: number): ApprovalRequest => ({
  id,
  session_id: session,
  command: "rm -rf dist",
  cwd: null,
  risk: "red",
  risk_reason: null,
  context: null,
  requested_at_ms: at,
});

describe("ringState", () => {
  test("cada estado do nó vira um anel", () => {
    expect(ringState(undefined, false)).toBe("idle");
    expect(ringState(nodeRun({ status: "running" }), false)).toBe("working");
    expect(ringState(nodeRun({ status: "running", url: "http://localhost:3000" }), false)).toBe(
      "live",
    );
    expect(ringState(nodeRun({ status: "waiting" }), false)).toBe("waiting");
    expect(ringState(nodeRun({ status: "failed" }), false)).toBe("failed");
    expect(ringState(nodeRun({ status: "done" }), false)).toBe("done");
    expect(ringState(nodeRun({ status: "stopped" }), false)).toBe("idle");
    expect(ringState(nodeRun({ status: "skipped" }), false)).toBe("idle");
  });

  test("precisar do dono vence o resto", () => {
    expect(ringState(nodeRun({ status: "running" }), true)).toBe("owner");
    expect(ringState(undefined, true)).toBe("owner");
  });
});

describe("nodeApproval", () => {
  const list = [
    request(7, "s2", 50),
    request(5, "s1", 90),
    request(3, "s1", 40),
    request(4, "s1", 40),
  ];

  test("o mais antigo da sessão do nó, desempate pelo id", () => {
    expect(nodeApproval("s1", list)?.id).toBe(3);
    expect(nodeApproval("s2", list)?.id).toBe(7);
  });

  test("sem sessão ou sem pedido, nenhum", () => {
    expect(nodeApproval(null, list)).toBeNull();
    expect(nodeApproval("s9", list)).toBeNull();
  });
});

describe("visibleCardApprovalIds: o toast só sai com a cápsula à vista", () => {
  const approvals = new Map([
    ["a", request(1, "s1", 1)],
    ["b", request(2, "s2", 1)],
  ]);
  const at = (x: number, y: number) => ({ x, y, w: 400, h: 300 });
  const pane = { w: 1000, h: 700 };
  const view = { x: 0, y: 0, zoom: 1 };
  const ids = (p: Partial<Parameters<typeof visibleCardApprovalIds>[0]>) =>
    [
      ...visibleCardApprovalIds({
        cards: [
          { id: "a", rect: at(20, 20) },
          { id: "b", rect: at(1200, 20) },
        ],
        approvals,
        view,
        pane,
        foreground: true,
        ...p,
      }),
    ].sort();

  test("cápsula na tela esconde o toast; a do nó fora da tela não", () => {
    expect(ids({})).toEqual([1]);
  });

  test("o pan traz o nó para a tela", () => {
    expect(ids({ view: { x: -900, y: 0, zoom: 1 } })).toEqual([2]);
  });

  test("cápsula cortada na borda: o toast fica", () => {
    expect(ids({ view: { x: 0, y: -30, zoom: 1 } })).toEqual([]);
  });

  test("tela de Serviços por cima (canvas fora do primeiro plano): nenhuma", () => {
    expect(ids({ foreground: false })).toEqual([]);
  });

  test("zoom longe demais para ler a cápsula: nenhuma", () => {
    expect(ids({ view: { x: 0, y: 0, zoom: 0.3 } })).toEqual([]);
    // A 0,5 o nó de x 1200 também cabe na tela.
    expect(ids({ view: { x: 0, y: 0, zoom: 0.5 } })).toEqual([1, 2]);
  });

  test("área ainda sem medida: nenhuma", () => {
    expect(ids({ pane: { w: 0, h: 0 } })).toEqual([]);
  });
});
