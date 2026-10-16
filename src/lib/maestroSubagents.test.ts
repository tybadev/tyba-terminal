import { describe, expect, test } from "bun:test";

import type { SubagentRun } from "./ipc";
import {
  SUB_GAP,
  SUB_H,
  SUB_TOP,
  SUB_W,
  formatSubagentTime,
  gridPosition,
  groupRounds,
  isSubagentNodeId,
  roundTally,
  roundsPosition,
  subagentElapsed,
  subagentKey,
  subagentLine,
  subagentNodeId,
  subagentState,
} from "./maestroSubagents";

// Fixtures sintéticas: nada aqui veio de sessão real.
const sub = (patch: Partial<SubagentRun> = {}): SubagentRun => ({
  agent_id: "a1",
  agent_type: "general-purpose",
  description: "listar a raiz",
  status: "running",
  started_at_ms: 1000,
  ended_at_ms: null,
  summary: null,
  interrupted: false,
  round: 1,
  ...patch,
});

describe("subagentState", () => {
  test("iniciando e rodando contam como rodando", () => {
    expect(subagentState(sub({ status: "starting" }))).toBe("running");
    expect(subagentState(sub())).toBe("running");
  });
  test("pronto, e parou quando interrompido ou bloqueado", () => {
    expect(subagentState(sub({ status: "done" }))).toBe("done");
    expect(subagentState(sub({ status: "done", interrupted: true }))).toBe("stopped");
    expect(subagentState(sub({ status: "done", blocked: true }))).toBe("stopped");
  });
});

describe("groupRounds", () => {
  test("sem subagente não há rodada", () => {
    expect(groupRounds([])).toBeNull();
  });
  test("separa a rodada mais nova, na ordem de nascimento", () => {
    const g = groupRounds([
      sub({ agent_id: "c", round: 2, started_at_ms: 30 }),
      sub({ agent_id: "x", round: 1 }),
      sub({ agent_id: "b", round: 2, started_at_ms: 20 }),
      sub({ agent_id: "y", round: 1 }),
    ]);
    expect(g?.round).toBe(2);
    expect(g?.current.map((r) => r.agent_id)).toEqual(["b", "c"]);
    expect(g?.previous).toBe(2);
  });
});

test("roundTally conta rodando, prontos e parados", () => {
  expect(
    roundTally([
      sub(),
      sub({ status: "starting" }),
      sub({ status: "done" }),
      sub({ status: "done", interrupted: true }),
    ]),
  ).toEqual({ running: 2, done: 1, stopped: 1 });
});

describe("grade", () => {
  const parent = { position: { x: 30, y: 30 }, width: 600, height: 260 };
  test("três colunas abaixo do agente", () => {
    const top = 30 + 260 + SUB_TOP;
    expect(gridPosition(parent, 0)).toEqual({ x: 30, y: top });
    expect(gridPosition(parent, 2)).toEqual({ x: 30 + 2 * (SUB_W + SUB_GAP), y: top });
    expect(gridPosition(parent, 3)).toEqual({ x: 30, y: top + SUB_H + SUB_GAP });
  });
  test("a linha da rodada anterior fica depois da última linha", () => {
    const top = 30 + 260 + SUB_TOP;
    expect(roundsPosition(parent, 6).y).toBe(top + 2 * (SUB_H + SUB_GAP));
    expect(roundsPosition(parent, 4).y).toBe(top + 2 * (SUB_H + SUB_GAP));
    expect(roundsPosition(parent, 0).y).toBe(top + SUB_H + SUB_GAP);
  });
});

describe("subagentLine", () => {
  test("rodando mostra a ferramenta e o argumento", () => {
    expect(subagentLine(sub({ activity_tool: "Bash", activity_detail: "sleep 10" }))).toEqual({
      kind: "activity",
      text: "Bash · sleep 10",
    });
    expect(subagentLine(sub({ activity_tool: "Read", activity_detail: null }))).toEqual({
      kind: "activity",
      text: "Read",
    });
    expect(subagentLine(sub())).toBeNull();
  });
  test("pronto mostra o resumo; parado sem resumo diz que parou", () => {
    expect(subagentLine(sub({ status: "done", summary: " 28 itens " }))).toEqual({
      kind: "summary",
      text: "28 itens",
    });
    expect(subagentLine(sub({ status: "done", interrupted: true }))).toEqual({
      kind: "interrupted",
    });
  });
  test("pronto sem resumo mostra a última ação, ou que terminou", () => {
    const at_ms = 2000;
    expect(
      subagentLine(
        sub({
          status: "done",
          summary: "  ",
          recent_activity: [
            { tool: "Grep", detail: "TODO", at_ms },
            { tool: "Read", detail: "a.ts", at_ms: 1500 },
          ],
        }),
      ),
    ).toEqual({ kind: "activity", text: "Grep · TODO" });
    expect(
      subagentLine(sub({ status: "done", activity_tool: "Bash", activity_detail: "ls" })),
    ).toEqual({ kind: "activity", text: "Bash · ls" });
    expect(subagentLine(sub({ status: "done", recent_activity: [] }))).toEqual({
      kind: "finished",
    });
    expect(subagentLine(sub({ status: "done" }))).toEqual({ kind: "finished" });
  });
});

test("ids efêmeros são reconhecíveis e a chave vale sem agent_id", () => {
  const key = subagentKey(sub({ agent_id: null, started_at_ms: 5, description: "d" }));
  expect(key).toBe("start-5-d");
  expect(isSubagentNodeId(subagentNodeId("n1", key))).toBe(true);
  expect(isSubagentNodeId("n1")).toBe(false);
});

describe("tempo", () => {
  test("decorrido até agora ou até o fim", () => {
    expect(subagentElapsed(sub(), 13_500)).toBe(12_500);
    expect(subagentElapsed(sub({ ended_at_ms: 11_400 }), 99_999)).toBe(10_400);
  });
  test("formato: inteiro rodando, uma casa pronto, minutos acima de 60s", () => {
    expect(formatSubagentTime(12_500, false, "pt-BR")).toBe("12s");
    expect(formatSubagentTime(10_400, true, "pt-BR")).toBe("10,4s");
    expect(formatSubagentTime(10_400, true, "en")).toBe("10.4s");
    expect(formatSubagentTime(190_000, true, "pt-BR")).toBe("3m10s");
  });
});
