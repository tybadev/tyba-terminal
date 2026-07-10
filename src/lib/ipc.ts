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

export const attachSession = (id: SessionId) =>
  invoke<void>("attach_session", { id });

export const detachSession = (id: SessionId) =>
  invoke<void>("detach_session", { id });

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
  view: string | null;
  active_pane: PaneId | null;
  root: PaneNode | null;
  created_at: string;
}

export type WorkspaceId = string;
export type WorkspaceKind = "user" | "docker";

export interface Workspace {
  id: WorkspaceId;
  name: string;
  repo_root: string | null;
  color: string | null;
  group: string | null;
  kind: WorkspaceKind;
  active_tab: TabId | null;
  tabs: Tab[];
  created_at: string;
}

export interface LayoutState {
  workspaces: Workspace[];
  active_workspace: WorkspaceId | null;
}

export const layoutState = () => invoke<LayoutState>("layout_state");

export const createWorkspace = (
  name: string,
  repoRoot: string | null,
  sessionId: SessionId,
) => invoke<WorkspaceId>("create_workspace", { name, repoRoot, sessionId });

export const closeWorkspace = (id: WorkspaceId) =>
  invoke<void>("close_workspace", { id });

export const activateWorkspace = (id: WorkspaceId) =>
  invoke<void>("activate_workspace", { id });

export const renameWorkspace = (id: WorkspaceId, name: string) =>
  invoke<void>("rename_workspace", { id, name });

export const setWorkspaceColor = (id: WorkspaceId, color: string | null) =>
  invoke<void>("set_workspace_color", { id, color });

export const setWorkspaceGroup = (id: WorkspaceId, group: string | null) =>
  invoke<void>("set_workspace_group", { id, group });

export interface RepoStatus {
  dirty: boolean;
  changed: number;
  insertions: number;
  deletions: number;
}

export const newWindow = () => invoke<void>("new_window");

export const createTab = (sessionId: SessionId, workspaceId?: WorkspaceId) =>
  invoke<TabId>("create_tab", {
    sessionId,
    workspaceId: workspaceId ?? null,
  });

export const getPref = (key: string) =>
  invoke<string | null>("get_pref", { key });

export const setPref = (key: string, value: string) =>
  invoke<void>("set_pref", { key, value });

export const closeTab = (id: TabId) => invoke<void>("close_tab", { id });

export const activateTab = (id: TabId) => invoke<void>("activate_tab", { id });

export const moveTab = (id: TabId, to: number) =>
  invoke<void>("move_tab", { id, to });

export const openSessionInTab = (sessionId: SessionId) =>
  invoke<void>("open_session_in_tab", { sessionId });

export const splitPane = (
  paneId: PaneId,
  kind: SplitKind,
  sessionId: SessionId,
) => invoke<PaneId>("split_pane", { paneId, kind, sessionId });

export const closePane = (paneId: PaneId) =>
  invoke<void>("close_pane", { paneId });

export const focusPane = (paneId: PaneId) =>
  invoke<void>("focus_pane", { paneId });

export const setSplitRatio = (paneId: PaneId, ratio: number, commit = true) =>
  invoke<void>("set_split_ratio", { paneId, ratio, commit });

export const onLayoutChanged = (
  handler: (state: LayoutState) => void,
): Promise<UnlistenFn> =>
  listen<LayoutState>("layout://changed", (e) => handler(e.payload));

export interface RepoSnapshot {
  ahead: number | null;
  behind: number | null;
  root: string;
  branch: string | null;
  status: RepoStatus | null;
}

export const repoSnapshots = () =>
  invoke<RepoSnapshot[]>("repo_snapshots");

export const sessionCwd = (id: SessionId) =>
  invoke<string | null>("session_cwd", { id });

export const onRepoReconciled = (
  handler: (snapshots: RepoSnapshot[]) => void,
): Promise<UnlistenFn> =>
  listen<RepoSnapshot[]>("repo://reconciled", (e) => handler(e.payload));

export const onRepoChanged = (
  handler: (snapshot: RepoSnapshot) => void,
): Promise<UnlistenFn> =>
  listen<RepoSnapshot>("repo://changed", (e) => handler(e.payload));

export function paneSession(node: PaneNode, pane: PaneId): SessionId | null {
  if (node.type === "leaf") return node.id === pane ? node.session_id : null;
  return paneSession(node.first, pane) ?? paneSession(node.second, pane);
}

export function leafSessions(node: PaneNode): SessionId[] {
  if (node.type === "leaf") return [node.session_id];
  return [...leafSessions(node.first), ...leafSessions(node.second)];
}

export interface ContainerInfo {
  id: string;
  name: string;
  image: string;
  state: string;
  status: string;
  ports: string;
  compose_project: string | null;
  compose_working_dir: string | null;
  service: string | null;
  config_files: string | null;
}

export type ComposeOp = "up" | "down" | "restart";

export const dockerAvailable = () => invoke<boolean>("docker_available");

export const dockerListContainers = (repoRoot: string | null, all: boolean) =>
  invoke<ContainerInfo[]>("docker_list_containers", { repoRoot, all });

export const dockerOpenLogs = (containerId: string) =>
  invoke<void>("docker_open_logs", { containerId });

export const dockerOpenShell = (containerId: string) =>
  invoke<void>("docker_open_shell", { containerId });

export const dockerOpenDashboard = () =>
  invoke<void>("docker_open_dashboard");

export const openViewTab = (view: string) =>
  invoke<void>("open_view_tab", { view });

export const dockerRemoveContainer = (containerId: string) =>
  invoke<void>("docker_remove_container", { containerId });

export const dockerOpenDesktop = () =>
  invoke<void>("docker_open_desktop");

export const dockerComposeOp = (project: string, op: ComposeOp) =>
  invoke<void>("docker_compose_op", { project, op });

export const dockerOpenProject = (project: string) =>
  invoke<void>("docker_open_project", { project });

export const dockerOpenComposeFile = (project: string) =>
  invoke<void>("docker_open_compose_file", { project });

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

export interface SessionCommand {
  command: string | null;
  running: boolean;
  agent_match: boolean;
}

export const submitRichInput = (id: SessionId, text: string, submit: boolean) =>
  invoke<void>("submit_rich_input", { id, text, submit });

export const sessionBracketedPaste = (id: SessionId) =>
  invoke<boolean>("session_bracketed_paste", { id });

export const sessionRelPath = (id: SessionId, path: string) =>
  invoke<string>("session_rel_path", { id, path });

export const onSessionBracketedPaste = (
  id: SessionId,
  handler: (enabled: boolean) => void,
): Promise<UnlistenFn> =>
  listen<{ bracketed_paste: boolean }>(`session://bracketed/${id}`, (e) =>
    handler(e.payload.bracketed_paste),
  );

export const setAgentMatchPattern = (pattern: string) =>
  invoke<boolean>("set_agent_match_pattern", { pattern });

export const listWorktreeFiles = (
  id: SessionId,
  query: string,
  limit?: number,
) =>
  invoke<string[]>("list_worktree_files", {
    id,
    query,
    limit: limit ?? null,
  });

export const promptMentionsSensitive = (text: string) =>
  invoke<boolean>("prompt_mentions_sensitive", { text });

export const onSessionCommand = (
  id: SessionId,
  handler: (payload: SessionCommand) => void,
): Promise<UnlistenFn> =>
  listen<SessionCommand>(`session://command/${id}`, (e) => handler(e.payload));

export interface SessionCwd {
  cwd: string;
}

export const onSessionCwd = (
  id: SessionId,
  handler: (payload: SessionCwd) => void,
): Promise<UnlistenFn> =>
  listen<SessionCwd>(`session://cwd/${id}`, (e) => handler(e.payload));
