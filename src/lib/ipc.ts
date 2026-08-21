// Camada IPC: espelho tipado dos commands do core Rust.
// Princípio #1 do CLAUDE.md: nenhuma lógica de sessão aqui —
// só serialização de intenções e subscrição de eventos.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { MenuSpec } from "./appMenu";

export type SessionId = string;

export type SessionKind =
  | { type: "shell" }
  | { type: "agent"; runner: "claude_code" | "codex" | { custom: string } }
  | { type: "ssh"; host_id: string }
  | { type: "container"; host_id: string | null; container_id: string };

export type AwaitingReason = "approval" | "reply";

export type SessionStatus =
  | { state: "running" }
  | { state: "awaiting_input"; hint: string | null; reason: AwaitingReason }
  | { state: "idle"; summary: string | null }
  | { state: "exited"; code: number }
  | { state: "failed"; reason: string };

export interface Worktree {
  path: string;
  branch: string;
  base_ref: string;
}

export type ConnectionState = "live" | "reconnecting" | "dropped";

export interface Session {
  id: SessionId;
  kind: SessionKind;
  title: string;
  repo_root: string | null;
  worktree: Worktree | null;
  status: SessionStatus;
  attention: boolean;
  created_at: string;
  connection?: ConnectionState;
}

export interface CreateSessionOpts {
  kind: SessionKind;
  title?: string;
  cwd?: string;
  cols: number;
  rows: number;
  worktree_task?: string;
  /** Agente na pasta de `cwd`, que já existe (review, conflitos), em vez de num
   * worktree novo. Sandbox e gate de aprovação continuam valendo. */
  attach_existing?: boolean;
  /** Id do shell escolhido no picker (ver `listShells`). Ausente = default do OS. */
  shell?: string;
}

/** Um shell disponível para abrir uma sessão (ver `listShells`). */
export interface ShellOption {
  id: string;
  label: string;
  program: string;
  args: string[];
}

export const listShells = () => invoke<ShellOption[]>("list_shells");

export interface SetupScriptInfo {
  path: string;
  content: string;
  hash: string;
  consent: boolean | null;
}

export interface WorktreeStatus {
  path: string;
  branch: string | null;
  dirty: boolean;
  ahead_of_head: number;
  managed: boolean;
  session_id: SessionId | null;
}

export interface WorktreeListing {
  root: string;
  worktrees: WorktreeStatus[];
}

export const worktreeList = (repoRoot: string) =>
  invoke<WorktreeListing>("worktree_list", { repoRoot });

export const worktreeRemove = (
  path: string,
  deleteBranch: boolean,
  force: boolean,
) => invoke<void>("worktree_remove", { path, deleteBranch, force });

export type FileStatus =
  | "added"
  | "modified"
  | "deleted"
  | "renamed"
  | "copied"
  | "type_changed"
  | "unmerged"
  | "other";

export interface FileDiff {
  path: string;
  old_path: string | null;
  status: FileStatus;
  insertions: number | null;
  deletions: number | null;
}

export interface CommitInfo {
  sha: string;
  subject: string;
  authored_at: string;
}

export interface SessionDiff {
  root: string;
  base_ref: string;
  commits: CommitInfo[];
  files: FileDiff[];
  staged_files: FileDiff[];
  unstaged_files: FileDiff[];
}

export type DiffScope = "committed" | "staged" | "unstaged";

export type ConflictOperation = "merge" | "rebase" | "cherry_pick";

export interface ConflictFile {
  path: string;
  kind: string;
}

export interface ConflictState {
  root: string;
  operation: ConflictOperation;
  ours: string | null;
  theirs: string | null;
  files: ConflictFile[];
}

export const sessionConflicts = (id: SessionId) =>
  invoke<ConflictState | null>("session_conflicts", { id });

export const sessionConflictChoose = (
  id: SessionId,
  path: string,
  side: "ours" | "theirs",
) => invoke<void>("session_conflict_choose", { id, path, side });

export const sessionConflictMarkResolved = (id: SessionId, path: string) =>
  invoke<void>("session_conflict_mark_resolved", { id, path });

export const sessionCheckout = (
  id: SessionId,
  branch: string,
  isRemote: boolean,
) => invoke<void>("session_checkout", { id, branch, isRemote });

export const suggestCommitMessage = (id: SessionId, agent: string) =>
  invoke<string>("suggest_commit_message", { id, agent });

export interface DiffLine {
  kind: "context" | "add" | "del";
  text: string;
}

export interface Hunk {
  old_start: number;
  old_lines: number;
  new_start: number;
  new_lines: number;
  header: string;
  lines: DiffLine[];
}

export interface FileHunks {
  binary: boolean;
  hunks: Hunk[];
}

export const openDiffTab = (id: SessionId) =>
  invoke<void>("open_diff_tab", { id });

export interface SessionGitStatus {
  root: string;
  branch: string | null;
  dirty: boolean;
}

export const sessionGitStatus = (id: SessionId) =>
  invoke<SessionGitStatus | null>("session_git_status", { id });

export const worktreeStage = (id: SessionId, paths: string[]) =>
  invoke<void>("worktree_stage", { id, paths });

export const worktreeUnstage = (id: SessionId, paths: string[]) =>
  invoke<void>("worktree_unstage", { id, paths });

export const worktreeDiscard = (id: SessionId, paths: string[]) =>
  invoke<void>("worktree_discard", { id, paths });

export const worktreeCommit = (id: SessionId, message: string) =>
  invoke<void>("worktree_commit", { id, message });

export const worktreePush = (id: SessionId) =>
  invoke<string>("worktree_push", { id });

export const openWorktreeFile = (
  id: SessionId,
  path: string,
  editor?: string,
) => invoke<void>("open_worktree_file", { id, path, editor: editor ?? null });

export const sessionDiff = (id: SessionId) =>
  invoke<SessionDiff>("session_diff", { id });

export const sessionDiffHunks = (
  id: SessionId,
  path: string,
  scope: DiffScope,
  oldPath?: string | null,
) =>
  invoke<FileHunks>("session_diff_hunks", {
    id,
    path,
    scope,
    oldPath: oldPath ?? null,
  });

export interface BranchInfo {
  name: string;
  is_current: boolean;
  is_remote: boolean;
  subject: string;
  committed_at: string;
}

export interface BranchList {
  branches: BranchInfo[];
  truncated: number;
}

export const sessionBranches = (id: SessionId) =>
  invoke<BranchList>("session_branches", { id });

export const sessionFetch = (id: SessionId) =>
  invoke<void>("session_fetch", { id });

export const sessionBranchDiff = (id: SessionId, branch: string) =>
  invoke<SessionDiff>("session_branch_diff", { id, branch });

export const sessionBranchHunks = (
  id: SessionId,
  branch: string,
  path: string,
  oldPath?: string | null,
) =>
  invoke<FileHunks>("session_branch_hunks", {
    id,
    branch,
    path,
    oldPath: oldPath ?? null,
  });

export type MergeStrategy = "squash" | "merge_commit";

export interface MergePreview {
  base_branch: string;
  source_branch: string;
  commits: number;
  files_changed: number;
  conflicts: string[];
  base_dirty: boolean;
}

export const worktreeMergePreview = (id: SessionId) =>
  invoke<MergePreview>("worktree_merge_preview", { id });

export const worktreeMergeMaterialize = (id: SessionId) =>
  invoke<void>("worktree_merge_materialize", { id });

export const worktreeMergeIntoBase = (
  id: SessionId,
  strategy: MergeStrategy,
  message?: string,
) =>
  invoke<string>("worktree_merge_into_base", {
    id,
    strategy,
    message: message?.trim() ? message.trim() : null,
  });

export type ForgeKind = "github" | "gitlab";

export interface ForgeStatus {
  kind: ForgeKind;
  cli: string;
  installed: boolean;
  authenticated: boolean;
  web_create_url: string | null;
}

export interface CheckRun {
  name: string;
  status: string;
  conclusion: string | null;
}

export interface PullRequest {
  number: number;
  title: string;
  url: string;
  state: string;
  checks: CheckRun[];
}

export interface ForgeReviewComment {
  id: string;
  author: string;
  body: string;
  path: string | null;
  line: number | null;
  url: string | null;
}

export const forgeStatus = (id: SessionId) =>
  invoke<ForgeStatus | null>("forge_status", { id });

export const forgePrForSession = (id: SessionId) =>
  invoke<PullRequest | null>("forge_pr_for_session", { id });

export const forgePrList = (id: SessionId) =>
  invoke<PullRequest[]>("forge_pr_list", { id });

export interface WorkflowRun {
  id: number;
  name: string;
  status: string;
  conclusion: string | null;
  url: string;
  head_branch: string;
  event: string;
  created_at: string;
}

export interface WorkflowJob {
  name: string;
  status: string;
  conclusion: string | null;
  url: string | null;
}

/**
 * `null` = a forja não sabe olhar CI (GitLab hoje). `[]` = não há run.
 * `fresh: false` aceita o cache do core — é o que faz o painel abrir instantâneo
 * sem gastar um processo `gh` por troca de sessão.
 */
export const forgeWorkflowRuns = (id: SessionId, fresh: boolean) =>
  invoke<WorkflowRun[] | null>("forge_workflow_runs", { id, fresh });

export const forgeWorkflowJobs = (id: SessionId, runId: number) =>
  invoke<WorkflowJob[]>("forge_workflow_jobs", { id, runId });

export const forgePrComments = (id: SessionId, number: number) =>
  invoke<ForgeReviewComment[]>("forge_pr_comments", { id, number });

export const forgeCreatePr = (id: SessionId, title: string, body: string) =>
  invoke<PullRequest>("forge_create_pr", { id, title, body });

export const worktreeSetupScript = (repoRoot: string) =>
  invoke<SetupScriptInfo | null>("worktree_setup_script", { repoRoot });

export const worktreeSetSetupConsent = (
  repoRoot: string,
  hash: string,
  allow: boolean,
) => invoke<void>("worktree_set_setup_consent", { repoRoot, hash, allow });

export interface AgentRepoConfig {
  hash: string;
  default_agent: string | null;
  env_allow: string[];
  consent: boolean | null;
}

export const agentRepoConfig = (repoRoot: string) =>
  invoke<AgentRepoConfig | null>("agent_repo_config", { repoRoot });

export const agentBinaryAvailable = (runner: "claude_code" | "codex") =>
  invoke<boolean>("agent_binary_available", { runner });

export const setAgentConfigConsent = (
  repoRoot: string,
  hash: string,
  allowed: boolean,
) => invoke<void>("set_agent_config_consent", { repoRoot, hash, allowed });

export const onAnyAgentReady = (
  handler: (id: SessionId) => void,
): Promise<UnlistenFn> =>
  listen<{ session_id: SessionId }>("agent://ready", (e) =>
    handler(e.payload.session_id),
  );

export type AgentRunner = "claude_code" | "codex" | { custom: string };

export interface DetectedAgent {
  pid: number;
  start_ms: number;
  kind: AgentRunner;
}

export const detectedAgent = (sessionId: SessionId) =>
  invoke<DetectedAgent | null>("detected_agent", { sessionId });

export const killShellAgent = (sessionId: SessionId) =>
  invoke<string>("kill_shell_agent", { sessionId });

export const onAgentDetected = (
  handler: (p: {
    session_id: SessionId;
    detected: DetectedAgent | null;
  }) => void,
): Promise<UnlistenFn> =>
  listen<{ session_id: SessionId; detected: DetectedAgent | null }>(
    "agent-detected://changed",
    (e) => handler(e.payload),
  );

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

// --- SSH: gestor de conexões (Host / Host Group) ---

export interface Host {
  id: string;
  alias: string;
  hostname: string;
  port: number | null;
  username: string | null;
  identity_file: string | null;
  proxy_jump: string | null;
  group_id: string | null;
  color: string | null;
  notes: string | null;
  position: number;
  tunnels: Tunnel[];
  created_at: string;
  last_connected_at: string | null;
}

export type TunnelKind = "local" | "remote" | "dynamic";

export interface Tunnel {
  kind: TunnelKind;
  listen_port: number;
  listen_host: string | null;
  target_host: string | null;
  target_port: number | null;
}

export type TunnelState =
  | { state: "opening" }
  | { state: "live" }
  | { state: "error"; detail: string };

export interface SessionTunnel extends Tunnel {
  id: string;
  session_id: SessionId;
  state: TunnelState;
  created_at: string;
}

export const listSessionTunnels = (sessionId: SessionId) =>
  invoke<SessionTunnel[]>("list_session_tunnels", { sessionId });

export const openSessionTunnel = (
  sessionId: SessionId,
  tunnel: Tunnel,
  confirmed: boolean,
) =>
  invoke<SessionTunnel>("open_session_tunnel", { sessionId, tunnel, confirmed });

export const closeSessionTunnel = (sessionId: SessionId, tunnelId: string) =>
  invoke<void>("close_session_tunnel", { sessionId, tunnelId });

export const openTunnelsPanel = (id: SessionId) =>
  invoke<void>("open_tunnels_panel", { id });

export const onSessionTunnels = (
  handler: (p: { session_id: SessionId; tunnels: SessionTunnel[] }) => void,
): Promise<UnlistenFn> =>
  listen<{ session_id: SessionId; tunnels: SessionTunnel[] }>(
    "session-tunnels",
    (e) => handler(e.payload),
  );

export interface HostGroup {
  id: string;
  name: string;
  color: string | null;
  notes: string | null;
  position: number;
  created_at: string;
}

export interface HostInput {
  alias: string;
  hostname: string;
  port?: number | null;
  username?: string | null;
  identity_file?: string | null;
  proxy_jump?: string | null;
  group_id?: string | null;
  color?: string | null;
  notes?: string | null;
  tunnels?: Tunnel[];
}

export interface HostGroupInput {
  name: string;
  color?: string | null;
  notes?: string | null;
}

export const listHosts = () => invoke<Host[]>("list_hosts");
export const listHostGroups = () => invoke<HostGroup[]>("list_host_groups");
export const createHost = (input: HostInput, confirmed = false) =>
  invoke<Host>("create_host", { input, confirmed });
export const updateHost = (host: Host, confirmed = false) =>
  invoke<Host>("update_host", { host, confirmed });
export const deleteHost = (id: string) => invoke<void>("delete_host", { id });
export const createHostGroup = (input: HostGroupInput) =>
  invoke<HostGroup>("create_host_group", { input });
export const updateHostGroup = (group: HostGroup) =>
  invoke<HostGroup>("update_host_group", { group });
export const deleteHostGroup = (id: string) =>
  invoke<void>("delete_host_group", { id });

// --- Broadcast: uma rajada para N SSH Sessions vivas ---

export type BroadcastVerdict =
  | { outcome: "sent"; targets: number }
  | {
      outcome: "needs_confirmation";
      risk: "green" | "yellow" | "red";
      command: string;
      targets: number;
    };

/** Tecla crua espelhada: nada executa sem Enter. */
export const broadcastWrite = (ids: SessionId[], data: string) =>
  invoke<void>("broadcast_write", { ids, data: encodeBase64(data) });

/** Enter da rajada. O core recusa vermelho sem `confirmed` — a regra não vive
 * aqui, o webview não consegue burlar. */
export const broadcastSubmit = (
  ids: SessionId[],
  command: string,
  confirmed: boolean,
) => invoke<BroadcastVerdict>("broadcast_submit", { ids, command, confirmed });

/** Abre o grupo inteiro como panes de um workspace só — é o que dá sentido
 * visual ao broadcast (digitar uma vez, conferir as N saídas lado a lado). */
export const connectHostGroup = (
  hostIds: string[],
  name: string,
  color: string | null,
  group: string | null,
) =>
  invoke<Session[]>("connect_host_group", { hostIds, name, color, group });

/** Conecta a um Host abrindo uma SSH Session (reusa create_session). */
export const connectHost = (hostId: string, cols: number, rows: number) =>
  createSession({ kind: { type: "ssh", host_id: hostId }, cols, rows });

export const reconnectSsh = (id: SessionId) =>
  invoke<void>("reconnect_ssh", { id });

export const resizeSession = (id: SessionId, cols: number, rows: number) =>
  invoke<void>("resize_session", { id, cols, rows });

/**
 * Resposta que separa "carregou e está vazio" de "ainda não carregou".
 *
 * O boot do core roda numa thread, e comando que chega antes dela terminar
 * devolve `ready: false`. Sem essa distinção a UI leria a lista vazia do meio
 * do boot como estado final e desenharia "nenhuma sessão" por cima de vinte
 * sessões que estão voltando.
 */
export interface Loaded<T> {
  ready: boolean;
  value: T;
}

/** O core terminou de carregar. Quem leu `ready: false` reconsulta aqui. */
export const EVENT_APP_READY = "app://ready";

export const listSessions = () =>
  invoke<Loaded<Session[]>>("list_sessions");

export const sessionMarkSeen = (id: SessionId) =>
  invoke<void>("session_mark_seen", { id });

export const disposeSession = (id: SessionId) =>
  invoke<void>("dispose_session", { id });

export type RiskLevel = "green" | "yellow" | "red";
export type ApprovalDecision = "approved" | "denied" | "approved_always";

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

export const resolveApproval = (
  id: number,
  decision: ApprovalDecision,
  feedback?: string,
) =>
  invoke<void>("resolve_approval", {
    id,
    decision,
    feedback: feedback?.trim() ? feedback.trim() : null,
  });

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
  | { type: "agentviewer"; id: PaneId; session_id: SessionId }
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
  name_locked: boolean;
  repo_root: string | null;
  color: string | null;
  group: string | null;
  kind: WorkspaceKind;
  launch_config_id: LaunchConfigId | null;
  active_tab: TabId | null;
  tabs: Tab[];
  side_view: string | null;
  side_ratio: number;
  side_expanded: boolean;
  created_at: string;
}

export interface LayoutState {
  workspaces: Workspace[];
  active_workspace: WorkspaceId | null;
}

export const layoutState = () => invoke<Loaded<LayoutState>>("layout_state");

/**
 * Tudo que a primeira pintura precisa, numa chamada.
 *
 * Eram 18 `invoke` no mount, 16 deles `get_pref`. Cada comando síncrono do
 * Tauri roda na main thread do macOS — a mesma que desenha o webview —, então
 * "paralelo" no JS vira fila ali. Um comando só tira 18 travessias do caminho
 * crítico da abertura.
 */
export interface BootSnapshot {
  ready: boolean;
  prefs: Record<string, string>;
  sessions: Session[];
  layout: LayoutState;
}

export const bootSnapshot = () => invoke<BootSnapshot>("boot_snapshot");

export type LaunchConfigId = string;
export type SlotId = string;

export type SlotNode =
  | { type: "leaf"; id: PaneId; slot_id: SlotId }
  | {
      type: "split";
      id: PaneId;
      split: SplitKind;
      ratio: number;
      first: SlotNode;
      second: SlotNode;
    };

export interface LaunchSlot {
  id: SlotId;
  name: string;
  kind: SessionKind;
  cwd_rel: string | null;
  isolate: boolean;
  initial_prompt: string | null;
}

export interface LaunchConfigTab {
  id: TabId;
  title: string | null;
  root: SlotNode;
}

export interface LaunchConfig {
  id: LaunchConfigId;
  name: string;
  slug: string;
  repo_root: string;
  source: "local";
  slots: LaunchSlot[];
  tabs: LaunchConfigTab[];
  created_at: string;
  updated_at: string;
}

export interface LaunchConfigDraft {
  name: string;
  repo_root: string;
  slots: LaunchSlot[];
  tabs: LaunchConfigTab[];
}

export interface SavedLaunchConfig {
  id: LaunchConfigId;
  slug: string;
  secret_warnings: string[];
}

export interface SlotFailure {
  slot: string;
  message: string;
}

export interface AppliedLaunchConfig {
  workspace_id: WorkspaceId;
  reused: boolean;
  failures: SlotFailure[];
}

export interface SnapshotSeed {
  name: string;
  repo_root: string;
  slots: LaunchSlot[];
  tabs: LaunchConfigTab[];
}

export const listLaunchConfigs = () =>
  invoke<LaunchConfig[]>("list_launch_configs");

export const applyLaunchConfig = (
  id: LaunchConfigId,
  cols: number,
  rows: number,
  clean?: boolean,
) =>
  invoke<AppliedLaunchConfig>("apply_launch_config", {
    id,
    clean: clean ?? null,
    cols,
    rows,
  });

export const saveLaunchConfig = (
  draft: LaunchConfigDraft,
  id?: LaunchConfigId,
) => invoke<SavedLaunchConfig>("save_launch_config", { id: id ?? null, draft });

export const deleteLaunchConfig = (id: LaunchConfigId) =>
  invoke<void>("delete_launch_config", { id });

export const launchConfigSeed = (workspaceId?: WorkspaceId) =>
  invoke<SnapshotSeed>("launch_config_seed", {
    workspaceId: workspaceId ?? null,
  });

export const onLaunchConfigPrefill = (
  handler: (payload: { session_id: SessionId; prompt: string }) => void,
): Promise<UnlistenFn> =>
  listen<{ session_id: SessionId; prompt: string }>(
    "launch-config://prefill",
    (e) => handler(e.payload),
  );

/** Identidade junto na criação: renomear/colorir/agrupar depois faz a sidebar
 * piscar entre os estados intermediários. */
export interface WorkspaceTag {
  lock_name?: boolean;
  color?: string | null;
  group?: string | null;
}

export const createWorkspace = (
  name: string,
  repoRoot: string | null,
  sessionId: SessionId,
  tag?: WorkspaceTag,
) =>
  invoke<WorkspaceId>("create_workspace", {
    name,
    repoRoot,
    sessionId,
    tag: tag ?? null,
  });

export const tagWorkspace = (id: WorkspaceId, name: string, tag: WorkspaceTag) =>
  invoke<void>("tag_workspace", { id, name, tag });

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

export interface EditorInfo {
  id: string;
  name: string;
  path: string;
  terminal: boolean;
}

export const listEditors = () => invoke<EditorInfo[]>("list_editors");

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

export const closeSideView = (workspaceId: WorkspaceId) =>
  invoke<void>("close_side_view", { workspaceId });

export const setSideViewExpanded = (workspaceId: WorkspaceId, expanded: boolean) =>
  invoke<void>("set_side_view_expanded", { workspaceId, expanded });

export const setSideViewRatio = (
  workspaceId: WorkspaceId,
  ratio: number,
  commit = true,
) => invoke<void>("set_side_view_ratio", { workspaceId, ratio, commit });

/** O core terminou de carregar. Quem leu `ready: false` reconsulta aqui. */
export const onAppReady = (handler: () => void): Promise<UnlistenFn> =>
  listen(EVENT_APP_READY, () => handler());

/**
 * A thread de boot do core morreu de pânico antes de terminar.
 *
 * Vem sempre seguido de `app://ready`: o portão abre de todo jeito, senão todo
 * comando de escrita pagaria o timeout de espera e a falha viraria lentidão. O
 * snapshot que se reconsultar depois disto estará incompleto — sem sessões, sem
 * layout, ou pela metade.
 *
 * É ESTE evento que diz que o vazio é falha, e não ausência de dado. Sem
 * consumi-lo, um boot que morreu é indistinguível de um app sem sessão nenhuma.
 */
export const onBootFailed = (
  handler: (message: string) => void,
): Promise<UnlistenFn> =>
  listen<{ message: string }>("app://boot-failed", (e) =>
    handler(e.payload.message),
  );

export const onLayoutChanged = (
  handler: (state: LayoutState) => void,
): Promise<UnlistenFn> =>
  listen<LayoutState>("layout://changed", (e) => handler(e.payload));

export type SubagentStatus = "starting" | "running" | "done";

export interface SubagentRun {
  agent_id: string | null;
  agent_type: string;
  description: string;
  status: SubagentStatus;
  started_at_ms: number;
  ended_at_ms: number | null;
  summary: string | null;
  interrupted: boolean;
}

export interface SubagentSnapshot {
  focused: string | null;
  subagents: SubagentRun[];
}

export interface SubagentsChanged {
  session_id: SessionId;
  focused: string | null;
  subagents: SubagentRun[];
}

export type TranscriptKind =
  | "assistant_text"
  | "tool_use"
  | "tool_result"
  | "user_text";

export interface TranscriptEntry {
  seq: number;
  kind: TranscriptKind;
  text: string;
  tool_name?: string | null;
  ts?: string | null;
}

export interface SubagentTranscript {
  entries: TranscriptEntry[];
  cursor: number;
  complete: boolean;
}

export interface SubagentTranscriptEvent {
  session_id: SessionId;
  agent_id: string;
  entries: TranscriptEntry[];
  cursor: number;
}

export const listSubagents = (sessionId: SessionId) =>
  invoke<SubagentSnapshot>("list_subagents", { sessionId });

export const focusSubagent = (sessionId: SessionId, agentId: string) =>
  invoke<SubagentSnapshot>("focus_subagent", { sessionId, agentId });

export const openAgentsPanel = (id: SessionId) =>
  invoke<void>("open_agents_panel", { id });

export const openSubagentViewer = (sessionId: SessionId) =>
  invoke<void>("open_subagent_viewer", { sessionId });

export const closeAgentViewers = (sessionId: SessionId) =>
  invoke<void>("close_agent_viewers", { sessionId });

export const subagentTranscript = (
  sessionId: SessionId,
  agentId: string,
  cursor: number | null,
) =>
  invoke<SubagentTranscript>("subagent_transcript", {
    sessionId,
    agentId,
    cursor,
  });

export const onSubagentsChanged = (
  handler: (p: SubagentsChanged) => void,
): Promise<UnlistenFn> =>
  listen<SubagentsChanged>("subagents://changed", (e) => handler(e.payload));

export const onSubagentTranscript = (
  handler: (p: SubagentTranscriptEvent) => void,
): Promise<UnlistenFn> =>
  listen<SubagentTranscriptEvent>("subagents://transcript", (e) =>
    handler(e.payload),
  );

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
  invoke<SessionCwd | null>("session_cwd", { id });

export const onRepoReconciled = (
  handler: (snapshots: RepoSnapshot[]) => void,
): Promise<UnlistenFn> =>
  listen<RepoSnapshot[]>("repo://reconciled", (e) => handler(e.payload));

export const onRepoChanged = (
  handler: (snapshot: RepoSnapshot) => void,
): Promise<UnlistenFn> =>
  listen<RepoSnapshot>("repo://changed", (e) => handler(e.payload));

export function paneSession(node: PaneNode, pane: PaneId): SessionId | null {
  if (node.type === "leaf" || node.type === "agentviewer")
    return node.id === pane ? node.session_id : null;
  return paneSession(node.first, pane) ?? paneSession(node.second, pane);
}

export function leafSessions(node: PaneNode): SessionId[] {
  if (node.type === "leaf") return [node.session_id];
  if (node.type === "agentviewer") return [];
  return [...leafSessions(node.first), ...leafSessions(node.second)];
}

export type PathStatus = "ok" | "missing" | "denied" | "unsupported";

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
  working_dir_status: PathStatus | null;
  compose_file_status: PathStatus | null;
}

export type ComposeOp = "up" | "down" | "restart";

/** `host` = alias SSH (materializado no ssh_config); ausente = máquina local. */
export const dockerAvailable = (host?: string | null) =>
  invoke<boolean>("docker_available", { host: host ?? null });

export const dockerListContainers = (
  repoRoot: string | null,
  all: boolean,
  host?: string | null,
) =>
  invoke<ContainerInfo[]>("docker_list_containers", {
    repoRoot,
    all,
    host: host ?? null,
  });

export const dockerOpenLogs = (containerId: string, host?: string | null) =>
  invoke<void>("docker_open_logs", { containerId, host: host ?? null });

export const dockerOpenShell = (containerId: string, host?: string | null) =>
  invoke<void>("docker_open_shell", { containerId, host: host ?? null });

export const dockerOpenDashboard = () =>
  invoke<void>("docker_open_dashboard");

export const openViewTab = (view: string) =>
  invoke<void>("open_view_tab", { view });

export const dockerRemoveContainer = (
  containerId: string,
  host?: string | null,
) => invoke<void>("docker_remove_container", { containerId, host: host ?? null });

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

export const setAppMenu = (spec: MenuSpec) =>
  invoke<void>("set_app_menu", { spec });

export const submitShellLine = (id: SessionId, text: string) =>
  invoke<void>("submit_shell_line", { id, text });

export const writeControl = (id: SessionId, bytes: string) =>
  invoke<void>("write_control", { id, bytes });

export interface CommandSuggestion {
  command: string;
  /** Nunca saiu com exit code 0: serve para o ghost text, não para a lista. */
  failed: boolean;
  kind: "history" | "snippet";
  label: string | null;
}

export const suggestCommands = (
  query: string,
  cwd: string | null,
  repoRoot: string | null,
) => invoke<CommandSuggestion[]>("suggest_commands", { query, cwd, repoRoot });

export const completePath = (cwd: string, token: string) =>
  invoke<string[]>("complete_path", { cwd, token });

export interface LineSuggestions {
  commands: CommandSuggestion[];
  paths: string[];
  arguments: string[];
}

export const suggestLine = (input: {
  query: string;
  cwd: string | null;
  repoRoot: string | null;
  pathToken: string | null;
  argPrefix: string | null;
  argToken: string | null;
}) => invoke<LineSuggestions>("suggest_line", input);

export const completeArgument = (prefix: string, token: string) =>
  invoke<string[]>("complete_argument", { prefix, token });

export type BlockColor =
  | { kind: "default" }
  | { kind: "idx"; value: number }
  | { kind: "rgb"; value: [number, number, number] };

export interface StyleRun {
  start: number;
  end: number;
  fg: BlockColor;
  bg: BlockColor;
  bold: boolean;
  italic: boolean;
  underline: boolean;
}

export interface LogicalLine {
  text: string;
  runs: StyleRun[];
}

export interface Block {
  id: number;
  sessionId: string;
  command: string;
  exitCode: number | null;
  startedAtMs: number;
  finishedAtMs: number;
  lines: LogicalLine[];
  truncated: number;
  /** Onde o comando rodou, do jeito que estava quando rodou. */
  cwd: string | null;
  /**
   * O comando pintou a tela alternada (`vim`, `bat`, `htop`). A saída não é
   * guardada — recortar a tela de um programa produz lixo —, mas o bloco existe
   * para o comando não sumir do registro.
   */
  altScreen: boolean;
}

export const sessionBlocks = (id: SessionId) =>
  invoke<Block[]>("session_blocks", { id });

export const onBlockFinalized = (
  id: SessionId,
  handler: (block: Block) => void,
): Promise<UnlistenFn> =>
  listen<Block>(`block://finalized/${id}`, (e) => handler(e.payload));

/** `clear`/`reset`: em modo bloco a tela é a lista, e ela some inteira. */
export const onBlocksCleared = (
  id: SessionId,
  handler: () => void,
): Promise<UnlistenFn> => listen(`block://cleared/${id}`, () => handler());

export const sessionPromptMode = (id: SessionId) =>
  invoke<boolean>("session_prompt_mode", { id });

/**
 * O tty está entregando linhas (eco ligado) ou teclas (raw)?
 *
 * Ligado, a seta não serve ao programa e ainda é ecoada — vira `^[[A` na saída
 * gravada do bloco. Desligado, quem lê tecla a tecla precisa dela.
 */
export const sessionLineEcho = (id: SessionId) =>
  invoke<boolean>("session_line_echo", { id });

export const togglePromptMode = (id: SessionId) =>
  invoke<void>("toggle_prompt_mode", { id });

export const onSessionPromptMode = (
  id: SessionId,
  handler: (on: boolean) => void,
): Promise<UnlistenFn> =>
  listen<{ prompt_mode: boolean }>(`session://prompt-mode/${id}`, (e) =>
    handler(e.payload.prompt_mode),
  );

export interface HistoryHit {
  command: string;
  cwd: string | null;
  uses: number;
  lastUsedAtMs: number;
  inCwd: boolean;
  inRepo: boolean;
}

export type SnippetSource = "local" | "repo";

export interface Snippet {
  id: string;
  name: string;
  command: string;
  description: string | null;
  tags: string[];
  source: SnippetSource;
}

export interface SnippetPlaceholder {
  name: string;
  default: string | null;
}

export const searchCommandHistory = (
  query: string,
  cwd: string | null,
  repoRoot: string | null,
  limit: number,
) =>
  invoke<HistoryHit[]>("search_command_history", {
    query,
    cwd,
    repoRoot,
    limit,
  });

export const clearCommandHistory = (repoRoot: string | null) =>
  invoke<void>("clear_command_history", { repoRoot });

export const setHistoryEnabled = (enabled: boolean) =>
  invoke<void>("set_history_enabled", { enabled });

export const listSnippets = (repoRoot: string | null) =>
  invoke<Snippet[]>("list_snippets", { repoRoot });

export const saveSnippet = (snippet: Snippet) =>
  invoke<void>("save_snippet", { snippet });

export const deleteSnippet = (id: string) =>
  invoke<void>("delete_snippet", { id });

export const snippetPlaceholders = (command: string) =>
  invoke<SnippetPlaceholder[]>("snippet_placeholders", { command });

export const renderSnippet = (
  id: string,
  command: string,
  values: [string, string][],
) => invoke<string>("render_snippet", { id, command, values });

export const onMenuAction = (
  handler: (action: string) => void,
): Promise<UnlistenFn> =>
  listen<string>("tyba:menu", (e) => handler(e.payload));

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
  /**
   * O shell está em prompt de continuação (`PS2`).
   *
   * `for`, `while`, `if`, `cat <<EOF`, aspas abertas: a última linha submetida
   * não fechou o comando. Nunca vem junto de `running: true` — são estados
   * diferentes do mesmo ciclo.
   */
  continuation: boolean;
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
  canonical: string;
}

export const onSessionCwd = (
  id: SessionId,
  handler: (payload: SessionCwd) => void,
): Promise<UnlistenFn> =>
  listen<SessionCwd>(`session://cwd/${id}`, (e) => handler(e.payload));

export interface UpdateInfo {
  version: string;
  url: string;
  published_at: string | null;
}

export interface UpdateStatus {
  info: UpdateInfo;
  dismissed: boolean;
}

export const appVersion = () => invoke<string>("app_version");

export interface BuildInfo {
  version: string;
  commit: string;
  commit_date: string;
  os: string;
  arch: string;
  webview: string;
}

export const appBuildInfo = () => invoke<BuildInfo>("app_build_info");

export const updateCheck = () => invoke<UpdateStatus | null>("update_check");

export const updateDismiss = (version: string) =>
  invoke<void>("update_dismiss", { version });

export type FilesKind = "agent_worktree" | "repo" | "outside_repo";

export interface FileEntry {
  name: string;
  rel_path: string;
  is_dir: boolean;
  is_symlink: boolean;
  gitignored: boolean;
}

export interface DirListing {
  dir: string;
  entries: FileEntry[];
  total: number;
  truncated: boolean;
}

export type FileDecoStatus =
  | "added"
  | "modified"
  | "deleted"
  | "renamed"
  | "untracked";

export interface FileDecoration {
  path: string;
  status: FileDecoStatus;
}

export type FileContent =
  | {
      kind: "text";
      text: string;
      offset: number;
      next_offset: number;
      total: number;
      truncated: boolean;
    }
  | { kind: "image"; data: string; mime: string; total: number }
  | { kind: "binary"; total: number; mime: string | null };

export interface FilesPanelInfo {
  root: string;
  kind: FilesKind;
  decorated: boolean;
  /** Raiz remota (sessão SSH) — o painel troca watcher por refresh sob demanda. */
  remote: boolean;
  /** Alias do host quando remoto; `null` no local. */
  host: string | null;
  /**
   * Raiz que a sessão resolveria agora, quando difere da ancorada; `null`
   * quando batem. Quem mede é o core — aqui só se escolhe entre o ícone mudo
   * e o rótulo. Sempre `null` no remoto, onde medir custaria um exec de git.
   */
  drifted_to: string | null;
}

export const openFilesPanel = (id: SessionId) =>
  invoke<string>("open_files_panel", { id });

export const filesPanelInfo = (id: SessionId) =>
  invoke<FilesPanelInfo>("files_panel_info", { id });

export const filesListDir = (id: SessionId, path: string) =>
  invoke<DirListing>("files_list_dir", { id, path });

export const filesRead = (id: SessionId, path: string, offset?: number) =>
  invoke<FileContent>("files_read", { id, path, offset: offset ?? null });

export const filesWatchDir = (id: SessionId, path: string) =>
  invoke<void>("files_watch_dir", { id, path });

export const filesUnwatchDir = (id: SessionId, path: string) =>
  invoke<void>("files_unwatch_dir", { id, path });

export const filesReanchor = (id: SessionId) =>
  invoke<string>("files_reanchor", { id });

/** Refresh sob demanda do painel remoto (substituto do watcher): invalida o
 * cache de listagem no core. No-op no local. */
export const filesRefresh = (id: SessionId) =>
  invoke<void>("files_refresh", { id });

export const filesOpenExternal = (
  id: SessionId,
  path: string,
  editor?: string,
) =>
  invoke<void>("files_open_external", { id, path, editor: editor ?? null });

export const filesDecorations = (id: SessionId) =>
  invoke<FileDecoration[]>("files_decorations", { id });

export const filesClose = (id: SessionId) =>
  invoke<void>("files_close", { id });

export const onFilesTree = (
  id: SessionId,
  handler: (dirs: string[]) => void,
): Promise<UnlistenFn> =>
  listen<{ dirs: string[] }>(`files://tree/${id}`, (e) =>
    handler(e.payload.dirs),
  );

export const onFilesDecorations = (
  id: SessionId,
  handler: (decorations: FileDecoration[]) => void,
): Promise<UnlistenFn> =>
  listen<{ decorations: FileDecoration[] }>(`files://decorations/${id}`, (e) =>
    handler(e.payload.decorations),
  );

export type GutterKind = "added" | "modified" | "deleted";

export interface GutterMarker {
  line: number;
  kind: GutterKind;
}

export type WriteResult =
  | { status: "written"; hash: string }
  | { status: "conflict"; disk_hash: string | null };

export interface EditContent {
  text: string;
  hash: string;
}

export const filesWrite = (
  id: SessionId,
  path: string,
  content: string,
  expectedHash: string,
) =>
  invoke<WriteResult>("files_write", {
    id,
    path,
    content,
    expectedHash,
  });

export const filesCreate = (id: SessionId, path: string, isDir: boolean) =>
  invoke<void>("files_create", { id, path, isDir });

export const filesRename = (id: SessionId, from: string, to: string) =>
  invoke<void>("files_rename", { id, from, to });

export const filesDelete = (id: SessionId, path: string) =>
  invoke<void>("files_delete", { id, path });

export interface FileSearchResult {
  paths: string[];
  truncated: boolean;
}

export const filesSearch = (id: SessionId, query: string, limit?: number) =>
  invoke<FileSearchResult>("files_search", { id, query, limit: limit ?? null });

export const filesFocus = (id: SessionId, path: string | null) =>
  invoke<GutterMarker[]>("files_focus", { id, path });

export const filesGutter = (id: SessionId, path: string) =>
  invoke<GutterMarker[]>("files_gutter", { id, path });

export const filesEditBegin = (id: SessionId, path: string) =>
  invoke<EditContent>("files_edit_begin", { id, path });

export const filesEditEnd = (id: SessionId) =>
  invoke<void>("files_edit_end", { id });

export const onFilesGutter = (
  id: SessionId,
  handler: (path: string, markers: GutterMarker[]) => void,
): Promise<UnlistenFn> =>
  listen<{ path: string; markers: GutterMarker[] }>(
    `files://gutter/${id}`,
    (e) => handler(e.payload.path, e.payload.markers),
  );

export const onFilesConflict = (
  id: SessionId,
  handler: (path: string, diskHash: string | null) => void,
): Promise<UnlistenFn> =>
  listen<{ path: string; disk_hash: string | null }>(
    `files://conflict/${id}`,
    (e) => handler(e.payload.path, e.payload.disk_hash),
  );

export interface LspPosition {
  line: number;
  character: number;
}

export interface LspRange {
  start: LspPosition;
  end: LspPosition;
}

export interface LspContentChange {
  range?: LspRange;
  text: string;
}

export interface LspDiagnostic {
  range: LspRange;
  severity: number;
  message: string;
  source: string | null;
}

export interface LspCompletionItem {
  label: string;
  detail: string | null;
  kind: number | null;
  insert_text: string | null;
  sort_text: string | null;
  filter_text: string | null;
}

export interface LspHover {
  contents: string;
}

export interface LspLocation {
  path: string;
  in_root: boolean;
  line: number;
  character: number;
}

export interface LspSignatureInfo {
  label: string;
  documentation: string | null;
  parameters: string[];
}

export interface LspSignatureHelp {
  signatures: LspSignatureInfo[];
  active_signature: number;
  active_parameter: number;
}

export interface LspInstallHint {
  command: string;
  manager: string;
}

export interface LspManagedCard {
  server_id: string;
  label: string;
  version: string;
  size: number;
  source: string;
  platform: string;
  verified: boolean;
}

export type LspManagedProgress =
  | { phase: "pending" }
  | { phase: "downloading"; downloaded: number; total: number }
  | { phase: "verifying" }
  | { phase: "extracting" }
  | { phase: "ready" }
  | { phase: "error"; message: string };

export type LspManagedDecision = "accept" | "use_mine" | "refuse";

export type LspStatus =
  | { state: "unsupported" }
  | { state: "experimental"; server: string }
  | {
      state: "absent";
      server: string;
      install: LspInstallHint | null;
      alternatives: LspInstallHint[];
    }
  | { state: "available"; server: string }
  | { state: "starting"; server: string }
  | { state: "ready"; server: string; diagnostics: number; truncated: boolean }
  | { state: "crashed"; server: string; reason: string | null }
  | { state: "managed_offer"; server: string; card: LspManagedCard }
  | {
      state: "installing";
      server: string;
      server_id: string;
      progress: LspManagedProgress;
    };

export const lspStatus = (id: SessionId, path: string) =>
  invoke<LspStatus>("lsp_status", { id, path });

export const lspRetry = (id: SessionId, path: string) =>
  invoke<LspStatus>("lsp_retry", { id, path });

export const lspOpen = (
  id: SessionId,
  path: string,
  text: string,
  spawn: boolean,
) => invoke<LspStatus>("lsp_open", { id, path, text, spawn });

export const lspChange = (
  id: SessionId,
  path: string,
  changes: LspContentChange[],
) => invoke<void>("lsp_change", { id, path, changes });

export const lspDidSave = (id: SessionId, path: string) =>
  invoke<void>("lsp_did_save", { id, path });

export const lspCloseDoc = (id: SessionId, path: string) =>
  invoke<void>("lsp_close_doc", { id, path });

export const lspCompletion = (
  id: SessionId,
  path: string,
  line: number,
  character: number,
) =>
  invoke<LspCompletionItem[]>("lsp_completion", { id, path, line, character });

export const lspHover = (
  id: SessionId,
  path: string,
  line: number,
  character: number,
) => invoke<LspHover | null>("lsp_hover", { id, path, line, character });

export const lspDefinition = (
  id: SessionId,
  path: string,
  line: number,
  character: number,
) => invoke<LspLocation[]>("lsp_definition", { id, path, line, character });

export const lspSignature = (
  id: SessionId,
  path: string,
  line: number,
  character: number,
) =>
  invoke<LspSignatureHelp | null>("lsp_signature", { id, path, line, character });

export const lspOpenExternal = (path: string, editor?: string) =>
  invoke<void>("lsp_open_external", { path, editor: editor ?? null });

export const lspManagedRegistry = () =>
  invoke<LspManagedCard[]>("lsp_managed_registry", {});

export const lspManagedConsent = (
  id: SessionId,
  serverId: string,
  decision: LspManagedDecision,
  path: string,
) =>
  invoke<LspStatus>("lsp_managed_consent", {
    id,
    serverId,
    decision,
    path,
  });

export const lspManagedDownload = (
  id: SessionId,
  serverId: string,
  path: string,
) => invoke<LspStatus>("lsp_managed_download", { id, serverId, path });

export const lspManagedUseMine = (serverId: string) =>
  invoke<{ install: LspInstallHint | null; alternatives: LspInstallHint[] }>(
    "lsp_managed_use_mine",
    { serverId },
  );

export const lspManagedDownloadStatus = (serverId: string) =>
  invoke<LspManagedProgress | null>("lsp_managed_download_status", {
    serverId,
  });

export const onLspManagedProgress = (
  serverId: string,
  cb: (p: LspManagedProgress) => void,
) => listen<LspManagedProgress>(`lsp://managed/progress/${serverId}`, (e) =>
  cb(e.payload),
);

export const onLspManagedError = (
  serverId: string,
  cb: (p: LspManagedProgress) => void,
) =>
  listen<LspManagedProgress>(`lsp://managed/error/${serverId}`, (e) =>
    cb(e.payload),
  );

export const onLspDiagnostics = (
  id: SessionId,
  handler: (
    files: { path: string; diagnostics: LspDiagnostic[] }[],
    filesTruncated: boolean,
  ) => void,
): Promise<UnlistenFn> =>
  listen<{
    files: { path: string; diagnostics: LspDiagnostic[] }[];
    files_truncated: boolean;
  }>(`lsp://diagnostics/${id}`, (e) =>
    handler(e.payload.files, e.payload.files_truncated),
  );
