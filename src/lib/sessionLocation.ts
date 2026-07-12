import { leafSessions, type SessionId, type Workspace } from "./ipc";

export interface SessionLocation {
  workspaceId: string;
  tabId: string;
}

export function findSessionLocation(
  workspaces: Workspace[],
  sessionId: SessionId,
): SessionLocation | null {
  for (const w of workspaces) {
    for (const tab of w.tabs) {
      if (tab.root && leafSessions(tab.root).includes(sessionId)) {
        return { workspaceId: w.id, tabId: tab.id };
      }
    }
  }
  return null;
}
