import { describe, expect, test } from "bun:test";

import type { ApprovalRequest } from "./ipc";
import type { NodeRun } from "./orbit";
import {
  agentDetailRows,
  agentName,
  approvalLine,
  diffSummary,
  failureFooter,
  showsEndLog,
  nodeApproval,
  ringState,
  visibleCardApprovalIds,
} from "./orbitCard";

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

  test("agente parado numa execução assentada fica pronto, não gira", () => {
    const live = nodeRun({ status: "running", orchestrating: true });
    expect(ringState(live, false)).toBe("working");
    expect(ringState(live, false, true)).toBe("done");
    // No primeiro turno (ainda não segura o run) e com pedido do dono, não.
    expect(ringState(nodeRun({ status: "running" }), false, true)).toBe("working");
    expect(ringState(live, true, true)).toBe("owner");
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

describe("cards no quadro OrbitCardsD", () => {
  const agent = (runner: "claude" | "codex", role: string) =>
    ({ type: "agent", runner, role, prompt: "", dir: "/p" }) as const;
  const process = { type: "process", command: "bun test", output: "log", file: null, dir: "/p" } as const;

  test("grade do ⋯ do agente: agente | pasta / papel | diff / contexto | tempo", () => {
    const rows = agentDetailRows({
      runner: "claude",
      role: "desenvolvedor",
      dir: "muriki-web",
      task: "escrever os testes",
      telemetry: { model: "Opus 5.5", contextPct: 12.6, linesAdded: 42, linesRemoved: 7 },
    });
    expect(rows.slice(0, 6)).toEqual([
      { key: "orbitDetailAgent", value: "Claude · Opus 5.5" },
      { key: "orbitDetailDir", value: "muriki-web" },
      { key: "orbitRole", value: "desenvolvedor" },
      { key: "orbitDetailDiff", value: "+42 −7" },
      { key: "orbitDetailContext", value: "13%" },
      { key: "orbitDetailTime", value: null },
    ]);
  });

  test("sem o que o core reporta, o agente sai sozinho e diff/contexto vazios", () => {
    const rows = agentDetailRows({
      runner: "codex",
      role: "",
      dir: "",
      task: "",
      telemetry: undefined,
    });
    expect(rows.map((r) => r.value).slice(0, 5)).toEqual(["Codex", "", "", "", ""]);
  });

  test("nome do modelo: o id do Claude perde o prefixo", () => {
    expect(agentName("claude", "claude-opus-5-5")).toBe("Claude · opus-5-5");
    expect(agentName("claude", null)).toBe("Claude");
    expect(diffSummary({ linesAdded: 3, linesRemoved: null })).toBe("+3 −0");
  });

  test("esperando você: 'Codex · revisor · pede há 40s'", () => {
    expect(approvalLine(agent("codex", "revisor"), 1_000, 41_000)).toEqual({
      who: "Codex · revisor",
      ago: "40s",
    });
    expect(approvalLine(agent("claude", " "), 0, 0).who).toBe("Claude");
  });

  test("processo que falhou: código de saída e 'Ver log inteiro'", () => {
    const failed = nodeRun({ status: "failed", exitCode: 1 });
    expect(failureFooter(process, failed)).toEqual({ exitCode: 1, reason: "", log: true });
    expect(failureFooter(agent("claude", ""), failed)?.log).toBe(false);
    expect(failureFooter(process, nodeRun({ status: "done" }))).toBeNull();
  });
});

describe("showsEndLog", () => {
  const process = {
    type: "process",
    command: "python3 -m http.server 8765",
    output: "log",
    file: null,
    dir: "",
  } as const;
  test("processo que rodou e acabou mostra o log; o resto não", () => {
    expect(showsEndLog(process, nodeRun({ status: "failed", exitCode: 1 }))).toBe(true);
    expect(showsEndLog(process, nodeRun({ status: "done", exitCode: 0 }))).toBe(true);
    expect(showsEndLog(process, nodeRun({ status: "stopped" }))).toBe(true);
    expect(showsEndLog(process, nodeRun({ status: "failed", sessionId: null }))).toBe(false);
    expect(showsEndLog(process, nodeRun({ status: "running" }))).toBe(false);
    expect(showsEndLog(process, nodeRun({ status: "skipped", sessionId: null }))).toBe(false);
    expect(showsEndLog(process, undefined)).toBe(false);
  });
});
