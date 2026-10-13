import { leafSessions, paneSession, type SessionCwd, type Workspace } from "./ipc";

export function compactPath(dir: string): string {
  const home = dir.replace(/^\/Users\/[^/]+/, "~");
  const parts = home.split("/").filter(Boolean);
  if (home.length <= 34 || parts.length <= 3) return home;
  return `…/${parts.slice(-2).join("/")}`;
}

/**
 * O nome curto de um diretório, para a aba: só a pasta ("poc-git"), como no
 * mockup. O caminho inteiro vai para o tooltip. A home vira `~` e a raiz `/`,
 * porque o nome da pasta ali ("guilherme", "") não diz onde se está.
 */
export function dirLabel(dir: string): string {
  const trimmed = dir.replace(/[/\\]+$/, "");
  if (trimmed === "" || trimmed === "~") return trimmed || "/";
  if (/^\/(Users|home)\/[^/]+$/.test(trimmed)) return "~";
  return trimmed.slice(Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\")) + 1);
}

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
