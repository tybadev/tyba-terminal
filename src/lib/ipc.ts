// Camada IPC: espelho tipado dos commands do core Rust.
// Princípio #1 do CLAUDE.md: nenhuma lógica de sessão aqui —
// só serialização de intenções e subscrição de eventos.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type SessionId = string;

export type SessionKind =
  | { type: "shell" }
  | { type: "agent"; runner: "claude_code" | "codex" | { custom: string } };

export type SessionStatus =
  | { state: "running" }
  | { state: "awaiting_input"; hint: string | null }
  | { state: "idle" }
  | { state: "exited"; code: number }
  | { state: "failed"; reason: string };

export interface Session {
  id: SessionId;
  kind: SessionKind;
  title: string;
  repo_root: string | null;
  worktree: unknown | null;
  status: SessionStatus;
  created_at: string;
}

export interface CreateSessionOpts {
  kind: SessionKind;
  title?: string;
  cwd?: string;
  cols: number;
  rows: number;
}

// --- base64 <-> bytes (chunks de PTY podem quebrar UTF-8 no meio) ---

const decodeBase64 = (data: string): Uint8Array =>
  Uint8Array.from(atob(data), (c) => c.charCodeAt(0));

const encodeBase64 = (data: string): string => {
  const bytes = new TextEncoder().encode(data);
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
};

export const createSession = (opts: CreateSessionOpts) =>
  invoke<Session>("create_session", { opts });

export const writeToSession = (id: SessionId, data: string) =>
  invoke<void>("write_to_session", { id, data: encodeBase64(data) });

export const sessionScrollback = (id: SessionId): Promise<Uint8Array> =>
  invoke<string>("session_scrollback", { id }).then(decodeBase64);

export const resizeSession = (id: SessionId, cols: number, rows: number) =>
  invoke<void>("resize_session", { id, cols, rows });

export const listSessions = () => invoke<Session[]>("list_sessions");

export const disposeSession = (id: SessionId) =>
  invoke<void>("dispose_session", { id });

export type RiskLevel = "green" | "yellow" | "red";
export type ApprovalDecision = "approved" | "denied";

export interface ApprovalRequest {
  id: number;
  session_id: SessionId;
  command: string;
  cwd: string | null;
  risk: RiskLevel;
  context: string | null;
  requested_at_ms: number;
}

export const requestApproval = (
  sessionId: SessionId,
  command: string,
  cwd?: string,
  context?: string,
) =>
  invoke<ApprovalRequest>("request_approval", {
    sessionId,
    command,
    cwd: cwd ?? null,
    context: context ?? null,
  });

export const listApprovals = () =>
  invoke<ApprovalRequest[]>("list_approvals");

export const resolveApproval = (id: number, decision: ApprovalDecision) =>
  invoke<void>("resolve_approval", { id, decision });

export const onApprovalRequested = (
  handler: (request: ApprovalRequest) => void,
): Promise<UnlistenFn> =>
  listen<ApprovalRequest>("approvals://requested", (e) => handler(e.payload));

export const onApprovalResolved = (
  handler: (resolved: { id: number; decision: ApprovalDecision }) => void,
): Promise<UnlistenFn> =>
  listen<{ id: number; decision: ApprovalDecision }>(
    "approvals://resolved",
    (e) => handler(e.payload),
  );

export type TabId = string;
export type PaneId = string;
export type SplitKind = "h" | "v";

export type PaneNode =
  | { type: "leaf"; id: PaneId; session_id: SessionId }
  | {
      type: "split";
      id: PaneId;
      split: SplitKind;
      ratio: number;
      first: PaneNode;
      second: PaneNode;
    };

export interface Tab {
  id: TabId;
  title: string | null;
  active_pane: PaneId;
  root: PaneNode;
  created_at: string;
}

export interface LayoutState {
  tabs: Tab[];
  active_tab: TabId | null;
}

export const layoutState = () => invoke<LayoutState>("layout_state");

export const createTab = (sessionId: SessionId) =>
  invoke<TabId>("create_tab", { sessionId });

export const closeTab = (id: TabId) => invoke<void>("close_tab", { id });

export const activateTab = (id: TabId) => invoke<void>("activate_tab", { id });

export const moveTab = (id: TabId, to: number) =>
  invoke<void>("move_tab", { id, to });

export const openSessionInTab = (sessionId: SessionId) =>
  invoke<TabId>("open_session_in_tab", { sessionId });

export const splitPane = (
  paneId: PaneId,
  kind: SplitKind,
  sessionId: SessionId,
) => invoke<PaneId>("split_pane", { paneId, kind, sessionId });

export const closePane = (paneId: PaneId) =>
  invoke<void>("close_pane", { paneId });

export const focusPane = (paneId: PaneId) =>
  invoke<void>("focus_pane", { paneId });

export const setSplitRatio = (paneId: PaneId, ratio: number) =>
  invoke<void>("set_split_ratio", { paneId, ratio });

export const onLayoutChanged = (
  handler: (state: LayoutState) => void,
): Promise<UnlistenFn> =>
  listen<LayoutState>("layout://changed", (e) => handler(e.payload));

export function paneSession(node: PaneNode, pane: PaneId): SessionId | null {
  if (node.type === "leaf") return node.id === pane ? node.session_id : null;
  return paneSession(node.first, pane) ?? paneSession(node.second, pane);
}

export function leafSessions(node: PaneNode): SessionId[] {
  if (node.type === "leaf") return [node.session_id];
  return [...leafSessions(node.first), ...leafSessions(node.second)];
}

export type ThemeMode = "dark" | "light" | "system";
export type ThemeBase = "dark" | "light";

export interface TerminalPalette {
  background: string;
  foreground: string;
  cursor: string;
  cursorAccent?: string;
  selectionBackground?: string;
  ansi: string[];
}

export interface Theme {
  id: string;
  name: string;
  base: ThemeBase;
  builtin: boolean;
  ui: Record<string, string>;
  terminal: TerminalPalette;
}

export interface ThemeState {
  mode: ThemeMode;
  dark: Theme;
  light: Theme;
}

export const listThemes = () => invoke<Theme[]>("list_themes");

export const getThemeState = () => invoke<ThemeState>("get_theme_state");

export const setThemeModeCmd = (mode: ThemeMode) =>
  invoke<void>("set_theme_mode", { mode });

export const setThemeSlot = (base: ThemeBase, id: string) =>
  invoke<void>("set_theme_slot", { base, id });

export const importThemeCmd = (path: string) =>
  invoke<Theme>("import_theme", { path });

export const onThemeChanged = (
  handler: (state: ThemeState) => void,
): Promise<UnlistenFn> =>
  listen<ThemeState>("theme://changed", (e) => handler(e.payload));

export const onPtyOutput = (
  id: SessionId,
  handler: (bytes: Uint8Array) => void,
): Promise<UnlistenFn> =>
  listen<{ data: string }>(`pty://output/${id}`, (e) =>
    handler(decodeBase64(e.payload.data)),
  );

export const onPtyExit = (
  id: SessionId,
  handler: () => void,
): Promise<UnlistenFn> => listen(`pty://exit/${id}`, () => handler());

export const onSessionStatus = (
  id: SessionId,
  handler: (session: Session) => void,
): Promise<UnlistenFn> =>
  listen<Session>(`session://status/${id}`, (e) => handler(e.payload));
