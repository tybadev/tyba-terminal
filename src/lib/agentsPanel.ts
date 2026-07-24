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

export const agentsPanelUngated = (kind: SessionKind): boolean =>
  kind.type !== "agent";

const anyActive = (subagents: SubagentRun[]): boolean =>
  subagents.some((s) => s.status === "running" || s.status === "starting");

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
