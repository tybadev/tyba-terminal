import { describe, expect, test } from "bun:test";

import type { ApprovalRequest } from "./ipc";
import type { NodeRun } from "./maestro";
import { nodeApproval, nodeApprovalIds, ringState } from "./maestroCard";

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

  test("ids mostrados nas cápsulas: um por sessão", () => {
    expect([...nodeApprovalIds(["s1", "s2", "s9"], list)].sort()).toEqual([3, 7]);
  });
});
