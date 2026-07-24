import { describe, expect, test } from "bun:test";

import type {
  Session,
  SessionKind,
  SessionStatus,
  SubagentRun,
  Workspace,
} from "./ipc";
import {
  agentsPanelSession,
  agentsPanelUngated,
  deadAgentsPanels,
  orchestratorVisual,
  showAgentsButton,
} from "./agentsPanel";

const workspace = (over: Partial<Workspace> = {}): Workspace =>
  ({
    id: "w1",
    name: "w",
    name_locked: false,
    repo_root: null,
    color: null,
    group: null,
    kind: "user",
    launch_config_id: null,
    active_tab: null,
    tabs: [],
    side_view: null,
    side_ratio: 0.5,
    side_expanded: false,
    created_at: "",
    ...over,
  }) as Workspace;

const session = (id: string, status: SessionStatus): Session =>
  ({
    id,
    kind: { type: "agent", runner: "claude_code" },
    title: id,
    repo_root: null,
    worktree: null,
    status,
    attention: false,
    created_at: "",
  }) as Session;

const sub = (status: SubagentRun["status"]): SubagentRun => ({
  agent_id: `a-${status}`,
  agent_type: "explorer",
  description: "",
  status,
  started_at_ms: 0,
  ended_at_ms: status === "done" ? 1 : null,
  summary: status === "done" ? "fez X" : null,
  interrupted: false,
});

describe("agentsPanelSession", () => {
  test("extrai o id só do side view de agentes", () => {
    expect(agentsPanelSession("agents:s-1")).toBe("s-1");
    expect(agentsPanelSession("diff:s-1")).toBeNull();
    expect(agentsPanelSession(null)).toBeNull();
  });
});

describe("deadAgentsPanels", () => {
  const seen = (...ids: string[]) => new Set(ids);

  test("fecha quando a sessão dona saiu (exited)", () => {
    const ws = workspace({ id: "w1", side_view: "agents:s-1" });
    const dead = deadAgentsPanels(
      [ws],
      [session("s-1", { state: "exited", code: 0 })],
      seen("s-1"),
    );
    expect(dead).toEqual(["w1"]);
  });

  test("fecha quando a sessão dona falhou", () => {
    const ws = workspace({ id: "w1", side_view: "agents:s-1" });
    const dead = deadAgentsPanels(
      [ws],
      [session("s-1", { state: "failed", reason: "x" })],
      seen("s-1"),
    );
    expect(dead).toEqual(["w1"]);
  });

  test("fecha quando a sessão dona (já vista) sumiu da lista", () => {
    const ws = workspace({ id: "w1", side_view: "agents:s-1" });
    expect(deadAgentsPanels([ws], [], seen("s-1"))).toEqual(["w1"]);
  });

  test("NÃO fecha sessão ausente que nunca foi vista (ainda carregando)", () => {
    const ws = workspace({ id: "w1", side_view: "agents:s-1" });
    expect(deadAgentsPanels([ws], [], seen())).toEqual([]);
  });

  test("NÃO fecha com a sessão viva (só subagentes terminaram)", () => {
    const ws = workspace({ id: "w1", side_view: "agents:s-1" });
    expect(
      deadAgentsPanels(
        [ws],
        [session("s-1", { state: "idle", summary: null })],
        seen("s-1"),
      ),
    ).toEqual([]);
    expect(
      deadAgentsPanels([ws], [session("s-1", { state: "running" })], seen("s-1")),
    ).toEqual([]);
  });

  test("ignora side views que não são de agentes", () => {
    const ws = workspace({ id: "w1", side_view: "diff:s-1" });
    expect(deadAgentsPanels([ws], [], seen("s-1"))).toEqual([]);
  });

  test("varre múltiplos workspaces", () => {
    const w1 = workspace({ id: "w1", side_view: "agents:s-1" });
    const w2 = workspace({ id: "w2", side_view: "agents:s-2" });
    const dead = deadAgentsPanels(
      [w1, w2],
      [
        session("s-1", { state: "running" }),
        session("s-2", { state: "exited", code: 1 }),
      ],
      seen("s-1", "s-2"),
    );
    expect(dead).toEqual(["w2"]);
  });
});

describe("showAgentsButton", () => {
  const shell: SessionKind = { type: "shell" };
  const agent: SessionKind = { type: "agent", runner: "claude_code" };
  const ssh: SessionKind = { type: "ssh", host_id: "h1" };

  test("shell com agente detectado → mostra o botão", () => {
    expect(showAgentsButton(shell, true)).toBe(true);
  });

  test("shell sem detecção → sem botão", () => {
    expect(showAgentsButton(shell, false)).toBe(false);
  });

  test("sessão de agente gerenciada → botão como hoje, independe de detecção", () => {
    expect(showAgentsButton(agent, false)).toBe(true);
    expect(showAgentsButton(agent, true)).toBe(true);
  });

  test("ssh nunca abre o painel de agentes, mesmo com detecção", () => {
    expect(showAgentsButton(ssh, true)).toBe(false);
  });
});

describe("agentsPanelUngated", () => {
  test("shell com claude detectado é não-gerenciada → badge sem gate", () => {
    expect(agentsPanelUngated({ type: "shell" })).toBe(true);
  });

  test("sessão de agente é gerenciada → sem badge", () => {
    expect(agentsPanelUngated({ type: "agent", runner: "claude_code" })).toBe(
      false,
    );
  });
});

describe("orchestratorVisual", () => {
  test("subagente ativo domina: em andamento (azul)", () => {
    const v = orchestratorVisual({ state: "running" }, false, [
      sub("running"),
      sub("done"),
    ]);
    expect(v?.labelKey).toBe("sessionInProgress");
    expect(v?.dotClass).toContain("bg-tyba-blue");
  });

  test("sessão rodando mas zero subagente ativo → concluído, nunca em andamento", () => {
    const v = orchestratorVisual({ state: "running" }, false, [sub("done")]);
    expect(v?.labelKey).toBe("sessionFinished");
    expect(v?.dotClass).toContain("bg-tyba-green");
  });

  test("idle com subagentes concluídos → concluído", () => {
    const v = orchestratorVisual({ state: "idle", summary: null }, false, [
      sub("done"),
    ]);
    expect(v?.labelKey).toBe("sessionFinished");
  });

  test("awaiting_input vence mesmo com subagente ativo (acionável)", () => {
    const v = orchestratorVisual(
      { state: "awaiting_input", hint: "git push", reason: "approval" },
      true,
      [sub("running")],
    );
    expect(v?.labelKey).toBe("sessionBlocked");
    expect(v?.dotClass).toContain("bg-tyba-amber");
  });

  test("failed vence tudo", () => {
    const v = orchestratorVisual({ state: "failed", reason: "x" }, false, [
      sub("running"),
    ]);
    expect(v?.labelKey).toBe("sessionFailed");
  });

  test("sem subagentes reflete o status real da sessão", () => {
    expect(
      orchestratorVisual({ state: "running" }, false, [])?.labelKey,
    ).toBe("sessionInProgress");
    expect(orchestratorVisual({ state: "idle", summary: null }, false, [])).toBeNull();
  });
});
