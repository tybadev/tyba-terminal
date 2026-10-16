import { describe, expect, test } from "bun:test";

import type { SubagentRun } from "./ipc";
import {
  SUB_COLS,
  SUB_GAP,
  SUB_H,
  SUB_ROUNDS_H,
  SUB_TOP,
  SUB_W,
  blockOrigin,
  blockOrigins,
  blockSize,
  formatSubagentTime,
  gridPosition,
  groupRounds,
  isSubagentNodeId,
  roundTally,
  pillPosition,
  roundsAt,
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
  test("pílulas numa coluna abaixo do agente", () => {
    const top = 30 + 260 + SUB_TOP;
    expect(SUB_COLS).toBe(1);
    expect(gridPosition(parent, 0)).toEqual({ x: 30, y: top });
    expect(gridPosition(parent, 2)).toEqual({ x: 30, y: top + 2 * (SUB_H + SUB_GAP) });
  });
  test("a linha da rodada anterior fica depois da última pílula", () => {
    const top = 30 + 260 + SUB_TOP;
    expect(roundsPosition(parent, 3).y).toBe(top + 3 * (SUB_H + SUB_GAP));
    expect(roundsPosition(parent, 0).y).toBe(top + SUB_H + SUB_GAP);
  });
});

describe("bloco de subagentes sem cobrir outro nó (nos-8)", () => {
  const parent = { position: { x: 30, y: 40 }, width: 560, height: 300 };
  const top = 40 + 300 + SUB_TOP;

  test("sem ninguém embaixo: logo abaixo do pai", () => {
    expect(blockOrigin(parent, blockSize(3, false), [])).toEqual({ x: 30, y: top });
  });

  test("Dev server logo abaixo do Claude: o bloco vai para o lado dele", () => {
    const dev = { x: 30, y: 430, w: 380, h: 300 };
    const at = blockOrigin(parent, blockSize(3, true), [dev]);
    expect(at.y).toBe(top);
    expect(at.x).toBeGreaterThanOrEqual(dev.x + dev.w);
  });

  test("arrastando: o bloco segue o pai sem refazer o desvio; no fim, refaz", () => {
    const claude = { id: "claude", box: parent, size: blockSize(3, false) };
    const dev = {
      id: "dev",
      box: { position: { x: 30, y: 430 }, width: 380, height: 300 },
      size: null,
    };
    const first = blockOrigins([claude, dev], null, false);
    const before = first.origins.get("claude")!;
    expect(before.x).toBeGreaterThanOrEqual(30 + 380);

    // O dono arrasta o Dev server para longe: no meio do arrasto o bloco
    // fica onde estava (o desvio não é refeito a cada quadro).
    const away = { ...dev, box: { ...dev.box, position: { x: 3000, y: 3000 } } };
    const during = blockOrigins([claude, away], first.offsets, true);
    expect(during.origins.get("claude")).toEqual(before);

    // Arrastando o próprio Claude, o bloco vai junto, pelo mesmo deslocamento.
    const moved = { ...claude, box: { ...parent, position: { x: 130, y: 90 } } };
    const follow = blockOrigins([moved, dev], first.offsets, true);
    expect(follow.origins.get("claude")).toEqual({ x: before.x + 100, y: before.y + 50 });

    // Soltou: o desvio é refeito com o Dev server longe — logo abaixo do pai.
    const after = blockOrigins([claude, away], during.offsets, false);
    expect(after.origins.get("claude")).toEqual({ x: 30, y: top });
  });

  test("tamanho conta as pílulas e a linha da rodada anterior", () => {
    expect(blockSize(2, false)).toEqual({ w: SUB_W, h: 2 * (SUB_H + SUB_GAP) });
    expect(blockSize(2, true).h).toBe(2 * (SUB_H + SUB_GAP) + SUB_ROUNDS_H);
  });

  test("pílulas e rodada anterior a partir do canto do bloco", () => {
    expect(pillPosition({ x: 5, y: 10 }, 1)).toEqual({ x: 5, y: 10 + SUB_H + SUB_GAP });
    expect(roundsAt({ x: 5, y: 10 }, 2)).toEqual({ x: 5, y: 10 + 2 * (SUB_H + SUB_GAP) });
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
