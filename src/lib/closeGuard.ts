import {
  leafSessions,
  type PaneId,
  type PaneNode,
  type Session,
  type SessionId,
  type Tab,
  type Workspace,
} from "./ipc";

export function isRunningAgent(session: Session | undefined): boolean {
  return (
    session !== undefined &&
    session.kind.type === "agent" &&
    (session.status.state === "running" ||
      session.status.state === "awaiting_input")
  );
}

function terminalPaneSession(
  node: PaneNode,
  pane: PaneId,
): SessionId | null {
  if (node.type === "leaf") return node.id === pane ? node.session_id : null;
  if (node.type === "agentviewer") return null;
  return (
    terminalPaneSession(node.first, pane) ??
    terminalPaneSession(node.second, pane)
  );
}

export function runningAgentAmong(
  ids: Iterable<SessionId>,
  sessionById: Map<SessionId, Session>,
): Session | null {
  for (const id of ids) {
    const session = sessionById.get(id);
    if (isRunningAgent(session)) return session ?? null;
  }
  return null;
}

export function paneRunningAgent(
  root: PaneNode,
  paneId: PaneId,
  sessionById: Map<SessionId, Session>,
): Session | null {
  const sid = terminalPaneSession(root, paneId);
  return sid ? runningAgentAmong([sid], sessionById) : null;
}

export function tabRunningAgent(
  tab: Tab,
  sessionById: Map<SessionId, Session>,
): Session | null {
  return tab.root
    ? runningAgentAmong(leafSessions(tab.root), sessionById)
    : null;
}

export function workspaceRunningAgent(
  workspace: Workspace,
  sessionById: Map<SessionId, Session>,
): Session | null {
  for (const tab of workspace.tabs) {
    const running = tabRunningAgent(tab, sessionById);
    if (running) return running;
  }
  return null;
}
