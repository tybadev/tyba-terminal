import { leafSessions, paneSession, type SessionCwd, type Workspace } from "./ipc";

export function resolveWorkspaceCwd(
  w: Workspace,
  sessionCwds: Record<string, SessionCwd>,
): SessionCwd | null {
  const tab = w.tabs.find((tb) => tb.id === w.active_tab) ?? w.tabs[0] ?? null;
  if (!tab?.root) return null;

  const focused = tab.active_pane
    ? paneSession(tab.root, tab.active_pane)
    : null;
  if (focused && sessionCwds[focused]) return sessionCwds[focused];

  for (const sid of leafSessions(tab.root)) {
    if (sessionCwds[sid]) return sessionCwds[sid];
  }
  return null;
}

export function workspaceMatchDir(
  w: Workspace,
  sessionCwds: Record<string, SessionCwd>,
): string | null {
  return resolveWorkspaceCwd(w, sessionCwds)?.canonical ?? w.repo_root ?? null;
}
