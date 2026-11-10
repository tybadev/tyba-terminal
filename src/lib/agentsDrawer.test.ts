import { describe, expect, test } from "bun:test";

import type { ApprovalRequest, SubagentRun } from "./ipc";
import {
  drawerPill,
  drawerTab,
  pendingSubagentApprovals,
  queueGoesToAgents,
  roundProgress,
} from "./agentsDrawer";

const run = (
  id: string,
  status: SubagentRun["status"],
  round = 1,
  over: Partial<SubagentRun> = {},
): SubagentRun => ({
  agent_id: id,
  agent_type: "general-purpose",
  description: id,
  status,
  started_at_ms: 0,
  ended_at_ms: status === "done" ? 1 : null,
  summary: null,
  round,
  interrupted: false,
  ...over,
});

const approval = (
  id: number,
  agent: string | null,
  session = "s1",
): ApprovalRequest => ({
  id,
  session_id: session,
  command: "bun run test:fixtures --update",
  cwd: null,
  risk: "yellow",
  risk_reason: null,
  context: null,
  requested_at_ms: id,
  agent_id: agent,
});

describe("roundProgress", () => {
  test("conta concluídos / total só da rodada atual", () => {
    const subs = [
      run("a", "done", 1),
      run("b", "done", 1),
      run("c", "running", 2),
      run("d", "done", 2),
      run("e", "starting", 2),
    ];
    expect(roundProgress(subs)).toEqual({
      done: 1,
      total: 3,
      running: true,
      interrupted: 0,
    });
  });

  test("interrompido não conta como concluído (4/5 · 1 interrompido)", () => {
    const subs = [
      run("a", "done"),
      run("b", "done"),
      run("c", "done"),
      run("d", "done"),
      run("e", "done", 1, { interrupted: true }),
    ];
    expect(roundProgress(subs)).toEqual({
      done: 4,
      total: 5,
      running: false,
      interrupted: 1,
    });
  });

  test("rodada assentada: algum terminou e nenhum roda (starting não segura)", () => {
    const subs = [run("a", "done"), run("ghost", "starting")];
    expect(roundProgress(subs)?.running).toBe(false);
    expect(roundProgress([run("a", "starting")])?.running).toBe(true);
  });

  test("sem subagentes não há contagem", () => {
    expect(roundProgress([])).toBeNull();
  });
});

describe("pendingSubagentApprovals", () => {
  test("conta os pedidos dos subagentes desta sessão, vermelho incluso", () => {
    const subs = [run("a", "running"), run("b", "running")];
    const approvals = [
      approval(1, "a"),
      { ...approval(2, "b"), risk: "red" as const },
      approval(3, null), // agente principal
      approval(4, "a", "s2"), // outra sessão
      approval(5, "zzz"), // não é subagente daqui
    ];
    expect(pendingSubagentApprovals("s1", subs, approvals)).toBe(2);
  });
});

describe("drawerPill", () => {
  const subs = [run("a", "done"), run("b", "running"), run("c", "done")];

  test("só fila: sessão sem subagentes", () => {
    expect(drawerPill([], 0, false)).toBeNull();
  });

  test("rodada em curso", () => {
    expect(drawerPill(subs, 0, false)).toEqual({
      done: 2,
      total: 3,
      state: { kind: "run" },
    });
  });

  test("aprovação pendente pede atenção, acima de rodando", () => {
    expect(drawerPill(subs, 2, false)?.state).toEqual({
      kind: "attn",
      count: 2,
    });
  });

  test("rodada terminou: ✓ até o dono abrir", () => {
    const done = [run("a", "done"), run("b", "done")];
    expect(drawerPill(done, 0, false)).toEqual({
      done: 2,
      total: 2,
      state: { kind: "ok" },
    });
    expect(drawerPill(done, 0, true)).toEqual({
      done: 2,
      total: 2,
      state: null,
    });
  });

  test("terminou com interrupção mostra o vermelho no lugar do ✓", () => {
    const failed = [run("a", "done"), run("b", "done", 1, { interrupted: true })];
    expect(drawerPill(failed, 0, false)?.state).toEqual({
      kind: "fail",
      count: 1,
    });
    expect(drawerPill(failed, 0, true)?.state).toBeNull();
  });
});

describe("drawerTab", () => {
  const base = { owner: null, hasSubagents: true, pending: 0, running: false };

  test("sem subagentes não há aba de agentes: é a fila", () => {
    expect(drawerTab({ ...base, hasSubagents: false, owner: "agents" })).toBe(
      "queue",
    );
  });

  test("aprovação pendente abre em Agentes", () => {
    expect(drawerTab({ ...base, pending: 1 })).toBe("agents");
  });

  test("Agentes durante a rodada, Fila fora dela", () => {
    expect(drawerTab({ ...base, running: true })).toBe("agents");
    expect(drawerTab(base)).toBe("queue");
  });

  test("a escolha do dono vale (até recolher, quando o core a zera)", () => {
    expect(drawerTab({ ...base, running: true, owner: "queue" })).toBe("queue");
    expect(drawerTab({ ...base, pending: 2, owner: "queue" })).toBe("queue");
    expect(drawerTab({ ...base, owner: "agents" })).toBe("agents");
  });
});

describe("queueGoesToAgents", () => {
  test("fila pausada por aprovação de subagente aponta para Agentes", () => {
    expect(queueGoesToAgents("approval", 1)).toBe(true);
  });

  test("aprovação do agente principal, ou outra pausa, não aponta", () => {
    expect(queueGoesToAgents("approval", 0)).toBe(false);
    expect(queueGoesToAgents("user", 1)).toBe(false);
    expect(queueGoesToAgents(null, 1)).toBe(false);
  });
});
