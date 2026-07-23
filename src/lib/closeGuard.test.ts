import { describe, expect, test } from "bun:test";

import type {
  PaneNode,
  Session,
  SessionId,
  SessionKind,
  SessionStatus,
  Tab,
  Workspace,
} from "./ipc";
import {
  isRunningAgent,
  paneRunningAgent,
  tabRunningAgent,
  workspaceRunningAgent,
} from "./closeGuard";

const session = (
  id: SessionId,
  kind: SessionKind,
  status: SessionStatus,
): Session => ({
  id,
  kind,
  title: id,
  repo_root: null,
  worktree: null,
  status,
  attention: false,
  created_at: "",
});

const agentRunning = (id: SessionId) =>
  session(id, { type: "agent", runner: "claude_code" }, { state: "running" });
const agentIdle = (id: SessionId) =>
  session(id, { type: "agent", runner: "claude_code" }, {
    state: "idle",
    summary: null,
  });
const agentAwaiting = (id: SessionId) =>
  session(id, { type: "agent", runner: "claude_code" }, {
    state: "awaiting_input",
    hint: null,
    reason: "approval",
  });
const shellRunning = (id: SessionId) =>
  session(id, { type: "shell" }, { state: "running" });

const mapOf = (...list: Session[]) =>
  new Map<SessionId, Session>(list.map((s) => [s.id, s]));

const tab = (id: string, root: PaneNode | null): Tab => ({
  id,
  title: null,
  view: null,
  active_pane: null,
  root,
  created_at: "",
});

const workspace = (tabs: Tab[]): Workspace =>
  ({
    id: "w",
    name: "w",
    name_locked: false,
    repo_root: null,
    color: null,
    group: null,
    kind: "user",
    launch_config_id: null,
    active_tab: tabs[0]?.id ?? null,
    tabs,
    side_view: null,
    side_ratio: 0.5,
    side_expanded: false,
    created_at: "",
  }) as Workspace;

describe("isRunningAgent", () => {
  test("agente Running exige confirmação", () => {
    expect(isRunningAgent(agentRunning("a"))).toBe(true);
  });

  test("shell comum nunca exige, mesmo rodando", () => {
    expect(isRunningAgent(shellRunning("a"))).toBe(false);
  });

  test("agente Idle não exige", () => {
    expect(isRunningAgent(agentIdle("a"))).toBe(false);
  });

  test("agente aguardando input exige (vivo, esperando você)", () => {
    expect(isRunningAgent(agentAwaiting("a"))).toBe(true);
  });

  test("sessão ausente não exige", () => {
    expect(isRunningAgent(undefined)).toBe(false);
  });
});

describe("paneRunningAgent", () => {
  const split: PaneNode = {
    type: "split",
    id: "root",
    split: "v",
    ratio: 0.5,
    first: { type: "leaf", id: "p1", session_id: "a" },
    second: { type: "leaf", id: "p2", session_id: "b" },
  };

  test("devolve o agente quando o pane-alvo o contém", () => {
    const map = mapOf(agentRunning("a"), shellRunning("b"));
    expect(paneRunningAgent(split, "p1", map)?.id).toBe("a");
  });

  test("null quando o pane-alvo é shell, mesmo com agente no irmão", () => {
    const map = mapOf(agentRunning("a"), shellRunning("b"));
    expect(paneRunningAgent(split, "p2", map)).toBeNull();
  });

  test("pane de viewer de subagente não segura o fechamento", () => {
    const withViewer: PaneNode = {
      type: "split",
      id: "root",
      split: "v",
      ratio: 0.5,
      first: { type: "leaf", id: "p1", session_id: "a" },
      second: { type: "agentviewer", id: "v1", session_id: "a" },
    };
    const map = mapOf(agentRunning("a"));
    expect(paneRunningAgent(withViewer, "v1", map)).toBeNull();
    expect(paneRunningAgent(withViewer, "p1", map)?.id).toBe("a");
  });
});

describe("tabRunningAgent", () => {
  test("null para tab de shell", () => {
    const t = tab("t", { type: "leaf", id: "p1", session_id: "b" });
    expect(tabRunningAgent(t, mapOf(shellRunning("b")))).toBeNull();
  });

  test("devolve o agente Running em qualquer folha da tab", () => {
    const t = tab("t", {
      type: "split",
      id: "root",
      split: "h",
      ratio: 0.5,
      first: { type: "leaf", id: "p1", session_id: "b" },
      second: { type: "leaf", id: "p2", session_id: "a" },
    });
    expect(tabRunningAgent(t, mapOf(shellRunning("b"), agentRunning("a")))?.id).toBe(
      "a",
    );
  });

  test("tab sem root não exige", () => {
    expect(tabRunningAgent(tab("t", null), mapOf())).toBeNull();
  });
});

describe("workspaceRunningAgent", () => {
  test("exige quando ao menos uma tab tem agente Running", () => {
    const ws = workspace([
      tab("t1", { type: "leaf", id: "p1", session_id: "b" }),
      tab("t2", { type: "leaf", id: "p2", session_id: "a" }),
    ]);
    expect(workspaceRunningAgent(ws, mapOf(shellRunning("b"), agentRunning("a")))?.id).toBe(
      "a",
    );
  });

  test("não exige com só shells e agentes Idle", () => {
    const ws = workspace([
      tab("t1", { type: "leaf", id: "p1", session_id: "b" }),
      tab("t2", { type: "leaf", id: "p2", session_id: "a" }),
    ]);
    expect(workspaceRunningAgent(ws, mapOf(shellRunning("b"), agentIdle("a")))).toBeNull();
  });
});
