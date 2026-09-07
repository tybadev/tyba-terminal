import type {
  Session,
  SessionId,
  SessionKind,
  SessionStatus,
  SubagentRun,
  Workspace,
  WorkspaceId,
} from "./ipc";
import { isFinishedStatus, statusVisual, type StatusVisual } from "./sessionStatus";

const AGENTS_VIEW_PREFIX = "agents:";

export const AGENTS_PANEL_LINGER_MS = 2_500;

export const agentsPanelSession = (sideView: string | null): SessionId | null => {
  if (!sideView?.startsWith(AGENTS_VIEW_PREFIX)) return null;
  return sideView.slice(AGENTS_VIEW_PREFIX.length);
};

export const deadAgentsPanels = (
  workspaces: Workspace[],
  sessions: Session[],
  seenSessions: Set<SessionId>,
): WorkspaceId[] => {
  const alive = new Map(sessions.map((s) => [s.id, s]));
  const out: WorkspaceId[] = [];
  for (const ws of workspaces) {
    const sessionId = agentsPanelSession(ws.side_view);
    if (!sessionId) continue;
    const session = alive.get(sessionId);
    if (session) {
      if (isFinishedStatus(session.status)) out.push(ws.id);
    } else if (seenSessions.has(sessionId)) {
      out.push(ws.id);
    }
  }
  return out;
};

export const showAgentsButton = (
  kind: SessionKind,
  detected: boolean,
): boolean => {
  if (kind.type === "agent") return true;
  if (kind.type === "shell") return detected;
  return false;
};

/**
 * Mostra o selo "sem gate" no painel de agentes?
 *
 * Shim v2 (tech-spec §7): `hosting` é o que decide, não só o `kind`. Um
 * `claude` hospedado pelo shim continua com `kind.type === "shell"` — o shim
 * mora dentro do shell, não vira sessão de agente —, mas já está dentro do
 * gate (princípios 1/4/6/10 do CLAUDE.md), jaulado ou não. Badge "sem gate"
 * é só para o caso `!hosting && detected`; hospedado é o sinal âmbar "sem
 * jaula" (`showUnjailedNotice`), nunca este.
 */
export const agentsPanelUngated = (
  kind: SessionKind,
  hosting: boolean,
): boolean => kind.type !== "agent" && !hosting;

const anyActive = (subagents: SubagentRun[]): boolean =>
  subagents.some((s) => s.status === "running" || s.status === "starting");

export const agentsPanelRunConcluded = (
  kind: SessionKind,
  status: SessionStatus,
  subagents: SubagentRun[],
): boolean => {
  if (subagents.length === 0 || anyActive(subagents)) return false;
  if (kind.type === "agent") {
    return (
      status.state === "idle" ||
      status.state === "exited" ||
      status.state === "failed"
    );
  }
  return true;
};

export type PanelRunEntry = { session: SessionId; armed: boolean };

export const trackPanelRun = (
  entry: PanelRunEntry | undefined,
  session: SessionId,
  concluded: boolean,
): { entry: PanelRunEntry; action: "cancel" | "schedule" | "none" } => {
  if (!entry || entry.session !== session) {
    return { entry: { session, armed: !concluded }, action: "cancel" };
  }
  if (!concluded) {
    return { entry: { session, armed: true }, action: "cancel" };
  }
  if (entry.armed) {
    return { entry: { session, armed: false }, action: "schedule" };
  }
  return { entry, action: "none" };
};

export const orchestratorVisual = (
  status: SessionStatus,
  attention: boolean,
  subagents: SubagentRun[],
): StatusVisual | null => {
  if (status.state === "failed" || status.state === "awaiting_input") {
    return statusVisual(status, attention);
  }
  if (anyActive(subagents)) {
    return statusVisual({ state: "running" }, attention);
  }
  if (subagents.length > 0 && status.state !== "exited") {
    return statusVisual({ state: "idle", summary: null }, true);
  }
  return statusVisual(status, attention);
};
