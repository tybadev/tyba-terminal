import { describe, expect, test } from "bun:test";

import type { ApprovalRequest, SubagentRun } from "./ipc";
import {
  buildAgentTree,
  collapsedRows,
  formatElapsed,
  groupByRound,
  roundAge,
} from "./agentTree";

const SESSION = "s-1";

const run = (
  id: string | null,
  started: number,
  status: SubagentRun["status"],
  extra: Partial<SubagentRun> = {},
): SubagentRun => ({
  agent_id: id,
  agent_type: "general-purpose",
  description: `teste ${id}`,
  status,
  started_at_ms: started,
  ended_at_ms: status === "done" ? started + 10_000 : null,
  summary: null,
  interrupted: false,
  round: 1,
  ...extra,
});

const approval = (
  id: number,
  agentId: string | null,
  session = SESSION,
): ApprovalRequest => ({
  id,
  session_id: session,
  command: "sleep 10 && ls",
  cwd: null,
  risk: "yellow",
  risk_reason: null,
  context: null,
  requested_at_ms: 0,
  agent_id: agentId,
});

describe("buildAgentTree", () => {
  test("duas rodadas: as que rodam no topo, mais novas primeiro", () => {
    const tree = buildAgentTree(
      SESSION,
      [
        run("a1", 100, "done"),
        run("a2", 101, "done"),
        run("b1", 300, "done"),
        run("b2", 301, "running"),
        run("b3", 302, "running"),
      ],
      [],
      400,
    );
    expect(tree.rows.map((r) => r.key)).toEqual(["b3", "b2", "b1", "a2", "a1"]);
    expect(tree.active).toBe(2);
  });

  test("luz por estado", () => {
    const tree = buildAgentTree(
      SESSION,
      [
        run("r", 5, "running"),
        run(null, 4, "starting"),
        run("d", 3, "done"),
        run("f", 2, "done", { interrupted: true }),
      ],
      [],
      10,
    );
    const lights = Object.fromEntries(tree.rows.map((r) => [r.key, r.light]));
    expect(lights).toEqual({
      r: "running",
      "pending-1": "starting",
      d: "done",
      f: "failed",
    });
  });

  test("aprovação do subagente vira selo âmbar na linha dele", () => {
    const tree = buildAgentTree(
      SESSION,
      [run("a1", 1, "running"), run("a2", 2, "running")],
      [approval(1, "a1"), approval(2, "a1"), approval(3, null)],
      10,
    );
    const a1 = tree.rows.find((r) => r.key === "a1")!;
    expect(a1.pending).toBe(2);
    expect(a1.light).toBe("awaiting");
    expect(tree.rows.find((r) => r.key === "a2")!.pending).toBe(0);
    expect(tree.mainPending).toBe(1);
  });

  test("aprovação de outra sessão não conta; de subagente desconhecido cai no principal", () => {
    const tree = buildAgentTree(
      SESSION,
      [run("a1", 1, "running")],
      [approval(1, "a1", "outra"), approval(2, "zzz")],
      10,
    );
    expect(tree.rows[0].pending).toBe(0);
    expect(tree.mainPending).toBe(1);
  });

  test("tempo decorrido usa o fim quando concluído e o relógio quando roda", () => {
    const tree = buildAgentTree(
      SESSION,
      [run("r", 1_000, "running"), run("d", 0, "done")],
      [],
      61_000,
    );
    expect(tree.rows[0].elapsedMs).toBe(60_000);
    expect(tree.rows[1].elapsedMs).toBe(10_000);
    expect(formatElapsed(60_000)).toBe("1m 00s");
    expect(formatElapsed(13_400)).toBe("13s");
  });
});

describe("groupByRound", () => {
  const twoRounds = () =>
    buildAgentTree(
      SESSION,
      [
        run("a1", 100, "done", { description: "Teste 1: sleep 10 + ls" }),
        run("a2", 101, "done", { description: "Teste 2: sleep 10 + ls" }),
        run("b1", 300, "done", {
          round: 2,
          description: "Teste 1: sleep 10 + ls",
        }),
        run("b2", 301, "running", {
          round: 2,
          description: "Teste 2: sleep 10 + ls",
        }),
      ],
      [approval(1, "b2")],
      400,
    ).rows;

  test("rodada mais nova no topo, com contagens", () => {
    const rounds = groupByRound(twoRounds());
    expect(rounds.map((r) => r.round)).toEqual([2, 1]);
    expect(rounds[0].latest).toBe(true);
    expect(rounds[1].latest).toBe(false);
    expect(rounds[0].rows.map((r) => r.key)).toEqual(["b2", "b1"]);
    expect(rounds[0].counts.awaiting).toBe(1);
    expect(rounds[0].counts.done).toBe(1);
    expect(rounds[1].counts.done).toBe(2);
    expect(rounds[1].startedAtMs).toBe(100);
  });

  test("descrição repetida fica em rodadas diferentes, não duplicada", () => {
    const rounds = groupByRound(twoRounds());
    for (const r of rounds) {
      const names = r.rows.map((row) => row.run.description);
      expect(new Set(names).size).toBe(names.length);
    }
  });

  test("rodada recolhida ainda mostra quem roda", () => {
    const rows = buildAgentTree(
      SESSION,
      [
        run("a1", 100, "running"),
        run("a2", 101, "done"),
        run("b1", 300, "running", { round: 2 }),
      ],
      [],
      400,
    ).rows;
    const old = groupByRound(rows).find((r) => r.round === 1)!;
    expect(collapsedRows(old).map((r) => r.key)).toEqual(["a1"]);
  });

  test("idade da rodada", () => {
    expect(roundAge(0, 30_000)).toEqual({ unit: "now", count: 0 });
    expect(roundAge(0, 3 * 60_000 + 5_000)).toEqual({ unit: "min", count: 3 });
    expect(roundAge(0, 2 * 3_600_000)).toEqual({ unit: "h", count: 2 });
  });
});
