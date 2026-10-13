import { describe, expect, test } from "bun:test";

import type { ApprovalRequest, SubagentRun } from "./ipc";
import {
  agentsPanelActionableIds,
  subagentInlineApprovals,
  subagentLine,
} from "./agentsPanel";
import { hiddenApprovalIds } from "./notifications";

const run = (over: Partial<SubagentRun> = {}): SubagentRun => ({
  agent_id: "a1",
  agent_type: "general-purpose",
  description: "Testes da API",
  status: "running",
  started_at_ms: 0,
  ended_at_ms: null,
  summary: null,
  interrupted: false,
  round: 1,
  ...over,
});

const approval = (over: Partial<ApprovalRequest> = {}): ApprovalRequest => ({
  id: 1,
  session_id: "s1",
  command: "curl -sI registry.npmjs.org",
  cwd: null,
  risk: "yellow",
  risk_reason: null,
  context: null,
  requested_at_ms: 100,
  agent_id: "a1",
  ...over,
});

describe("subagentLine", () => {
  test("rodando um comando: 'Rodando <comando>'", () => {
    expect(
      subagentLine(run({ activity_tool: "Bash", activity_detail: "bun test" })),
    ).toEqual({ kind: "run", detail: "bun test" });
  });

  test("outra ferramenta: nome e argumento", () => {
    expect(
      subagentLine(run({ activity_tool: "Read", activity_detail: "src/a.ts" })),
    ).toEqual({ kind: "tool", tool: "Read", detail: "src/a.ts" });
    expect(subagentLine(run({ activity_tool: "TodoWrite" }))).toEqual({
      kind: "tool",
      tool: "TodoWrite",
      detail: null,
    });
  });

  test("sem atividade conhecida, nada — não se inventa", () => {
    expect(subagentLine(run())).toBeNull();
  });

  test("terminado mostra a primeira linha do resultado, não a atividade", () => {
    expect(
      subagentLine(
        run({
          status: "done",
          summary: "\n  Resumiu 4 seções em 6 linhas\nmais detalhe",
          activity_tool: "Bash",
          activity_detail: "ls",
        }),
      ),
    ).toEqual({ kind: "result", text: "Resumiu 4 seções em 6 linhas" });
  });

  test("interrompido diz que foi interrompido", () => {
    expect(subagentLine(run({ status: "done", interrupted: true }))).toEqual({
      kind: "interrupted",
    });
  });
});

describe("subagentInlineApprovals", () => {
  test("o mais antigo pendente de cada subagente, só desta sessão", () => {
    const map = subagentInlineApprovals(
      "s1",
      [run(), run({ agent_id: "a2" })],
      [
        approval({ id: 3, requested_at_ms: 300 }),
        approval({ id: 2, requested_at_ms: 200 }),
        approval({ id: 4, agent_id: "a2" }),
        approval({ id: 5, session_id: "s2" }),
        approval({ id: 6, agent_id: null }),
        approval({ id: 7, agent_id: "desconhecido" }),
      ],
    );
    expect([...map.entries()].map(([k, v]) => [k, v.id])).toEqual([
      ["a1", 2],
      ["a2", 4],
    ]);
  });

  test("vermelho aparece na linha, mas não fica acionável no painel", () => {
    const map = subagentInlineApprovals(
      "s1",
      [run(), run({ agent_id: "a2" })],
      [approval({ id: 1 }), approval({ id: 2, agent_id: "a2", risk: "red" })],
    );
    expect(map.get("a2")?.id).toBe(2);
    expect(agentsPanelActionableIds(map)).toEqual(new Set([1]));
  });

  test("o que o painel deixa aprovar sai do toast — acionável em um lugar só", () => {
    const ids = hiddenApprovalIds({
      approvals: [approval({ id: 1 }), approval({ id: 9 })],
      notificationsOpen: false,
      agentQueueOpen: false,
      agentQueueVisibleIds: new Set(),
      agentsPanelIds: new Set([1]),
    });
    expect(ids).toEqual(new Set([1]));
  });
});
