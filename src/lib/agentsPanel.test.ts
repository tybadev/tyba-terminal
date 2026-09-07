import { describe, expect, test } from "bun:test";

import type {
  Session,
  SessionKind,
  SessionStatus,
  SubagentRun,
  Workspace,
} from "./ipc";
import {
  agentsPanelRunConcluded,
  agentsPanelSession,
  agentsPanelUngated,
  deadAgentsPanels,
  orchestratorVisual,
  showAgentsButton,
  trackPanelRun,
  type PanelRunEntry,
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
  test("shell com claude detectado, sem hosting → badge sem gate", () => {
    expect(agentsPanelUngated({ type: "shell" }, false)).toBe(true);
  });

  test("shell com claude hospedado pelo shim v2 (hosting) → sem badge, o gate está de pé (tech-spec §7)", () => {
    expect(agentsPanelUngated({ type: "shell" }, true)).toBe(false);
  });

  test("sessão de agente é gerenciada → sem badge, com ou sem hosting", () => {
    expect(
      agentsPanelUngated({ type: "agent", runner: "claude_code" }, false),
    ).toBe(false);
    expect(
      agentsPanelUngated({ type: "agent", runner: "claude_code" }, true),
    ).toBe(false);
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

describe("agentsPanelRunConcluded", () => {
  const agentKind: SessionKind = { type: "agent", runner: "claude_code" };
  const shellKind: SessionKind = { type: "shell" };
  const idle: SessionStatus = { state: "idle", summary: "resumo" };

  test("gerenciada: turno encerrado com todos os subagentes Done conclui", () => {
    expect(agentsPanelRunConcluded(agentKind, idle, [sub("done")])).toBe(true);
    expect(
      agentsPanelRunConcluded(agentKind, { state: "exited", code: 0 }, [
        sub("done"),
      ]),
    ).toBe(true);
  });

  test("gerenciada: turno ainda rodando não conclui, mesmo com subagentes Done", () => {
    expect(
      agentsPanelRunConcluded(agentKind, { state: "running" }, [sub("done")]),
    ).toBe(false);
    expect(
      agentsPanelRunConcluded(
        agentKind,
        { state: "awaiting_input", hint: null, reason: "approval" },
        [sub("done")],
      ),
    ).toBe(false);
  });

  test("subagente ativo nunca conclui", () => {
    expect(agentsPanelRunConcluded(agentKind, idle, [sub("running")])).toBe(
      false,
    );
    expect(
      agentsPanelRunConcluded(shellKind, { state: "running" }, [
        sub("done"),
        sub("starting"),
      ]),
    ).toBe(false);
  });

  test("painel sem rodada (zero subagentes) nunca conclui", () => {
    expect(agentsPanelRunConcluded(agentKind, idle, [])).toBe(false);
    expect(agentsPanelRunConcluded(shellKind, { state: "running" }, [])).toBe(
      false,
    );
  });

  test("shell: todos Done conclui independente do status da sessão", () => {
    expect(
      agentsPanelRunConcluded(shellKind, { state: "running" }, [sub("done")]),
    ).toBe(true);
  });
});

describe("trackPanelRun", () => {
  test("painel abrindo no meio da rodada arma e fecha na conclusão", () => {
    const opened = trackPanelRun(undefined, "s1", false);
    expect(opened.entry).toEqual({ session: "s1", armed: true });
    expect(opened.action).toBe("cancel");

    const running = trackPanelRun(opened.entry, "s1", false);
    expect(running.action).toBe("cancel");

    const concluded = trackPanelRun(running.entry, "s1", true);
    expect(concluded.action).toBe("schedule");
    expect(concluded.entry.armed).toBe(false);

    const after = trackPanelRun(concluded.entry, "s1", true);
    expect(after.action).toBe("none");
  });

  test("painel reaberto depois da conclusão não agenda fechamento", () => {
    const reopened = trackPanelRun(undefined, "s1", true);
    expect(reopened.entry).toEqual({ session: "s1", armed: false });
    expect(reopened.action).toBe("cancel");
    expect(trackPanelRun(reopened.entry, "s1", true).action).toBe("none");
  });

  test("rodada nova depois da conclusão re-arma e fecha de novo no fim", () => {
    const handled: PanelRunEntry = { session: "s1", armed: false };
    const rearmed = trackPanelRun(handled, "s1", false);
    expect(rearmed.entry.armed).toBe(true);
    expect(trackPanelRun(rearmed.entry, "s1", true).action).toBe("schedule");
  });

  test("painel trocando de sessão recomeça o rastreio", () => {
    const s1: PanelRunEntry = { session: "s1", armed: false };
    const swapped = trackPanelRun(s1, "s2", false);
    expect(swapped.entry).toEqual({ session: "s2", armed: true });
    expect(swapped.action).toBe("cancel");
  });
});
