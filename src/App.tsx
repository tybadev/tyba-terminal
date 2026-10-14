import type { ElementType } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CaretDown,
  CaretRight,
  Check,
  DotsThree,
  FolderOpen,
  GearSix,
  GitBranch,
  GitDiff,
  HardDrives,
  Keyboard,
  MagnifyingGlass,
  Plus,
  Prohibit,
  SidebarSimple,
  SquaresFour,
  Stack,
  TerminalWindow,
  User,
  X,
} from "@phosphor-icons/react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { getThemeMode, onThemeModeChange, setThemeMode, type ThemeMode } from "./theme";
import { NotificationsInbox } from "./components/NotificationsInbox";
import { NotificationToaster } from "./components/NotificationToaster";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { WindowControls, WindowResizeEdges } from "./components/WindowChrome";
import { UpdateToast } from "./components/UpdateToast";
import { IS_MAC } from "./lib/platform";
import { AgentIcon } from "./components/icons/AgentIcon";
import { AgentsBoard } from "./components/AgentsBoard";
import { AgentsQueue } from "./components/AgentsQueue";
import { AgentsSidebar } from "./components/AgentsSidebar";
import { ClaudeIcon } from "./components/icons/ClaudeIcon";
import { OpenAIIcon } from "./components/icons/OpenAIIcon";
import { Clock } from "./components/Clock";
import { CommandPalette } from "./components/CommandPalette";
import { ConfirmHost } from "./components/ConfirmHost";
import { ToastHost } from "./components/ToastHost";
import { requestConfirm } from "./lib/confirm";
import { translateError } from "./lib/errors";
import {
  agentBinaryName,
  noticeKey,
  showShellAgentNotice,
  showUnjailedNotice,
  ungatedReason,
} from "./lib/shellAgentNotice";
import { resumeCandidates, showAgentResumeInvite } from "./lib/agentResume";
import {
  paneRunningAgent,
  tabRunningAgent,
  workspaceRunningAgent,
} from "./lib/closeGuard";
import { pushToast, toastError } from "./lib/toast";
import {
  hostingIntroToastInput,
  shouldShowHostingIntroToast,
} from "./lib/hostingIntroToast";
import {
  sandboxWarningTitleKey,
  sandboxWarningToastInput,
} from "./lib/sandboxWarning";
import {
  authAlertMessageKey,
  authAlertToastInput,
  withRuntimeAuthAlert,
  withoutAuthAlert,
  withoutRecoveredAuthAlerts,
} from "./lib/authAlert";
import { agentOpenUrlToastInput } from "./lib/agentOpenUrlToast";
import { agentAlertToast } from "./lib/agentUsage";
import {
  LaunchConfigDialog,
  type LaunchConfigDraftState,
} from "./components/LaunchConfigDialog";
import { ShortcutsPanel } from "./components/ShortcutsPanel";
import { ConnectionsView } from "./components/ConnectionsView";
import {
  HostFormDialog,
  type HostDialogState,
} from "./components/HostFormDialog";
import { knownHostsRemoveCommand } from "./lib/canoFailure";
import { HostPicker } from "./components/HostPicker";
import {
  BroadcastBar,
  BroadcastConfirmDialog,
  type BroadcastTarget,
} from "./components/BroadcastBar";
import { ptyExitEndsSession } from "./lib/sessionExit";
import {
  SESSION_DOT_CLASS,
  SESSION_DOT_LABEL_KEY,
  sessionDot,
} from "./lib/sessionCard";
import { matchSshHost } from "./lib/sshCommand";
import { DockerIcon } from "./components/icons/DockerIcon";
import { TybaMark } from "./components/icons/TybaMark";
import { NewSessionPrompt } from "./components/NewSessionPrompt";
import { WorktreeCreateDialog } from "./components/WorktreeCreateDialog";
import { WorktreesView } from "./components/WorktreesView";
import { StatsView } from "./components/StatsView";
import { DiffView } from "./components/DiffView";
import { TunnelsView } from "./components/TunnelsView";
import { FilesScreen, closeScreenFile } from "./components/FilesScreen";
import { GitHistoryView } from "./components/GitHistoryView";
import { DockerScreen } from "./components/DockerScreen";
import { HistoryPanel } from "./components/HistoryPanel";
import { SidePanelBar, type PanelTab } from "./components/SidePanelBar";
import {
  escClosesPanel,
  litPanel,
  panelAriaLabel,
  panelClick,
  panelIndicator,
  parseSideView,
  visiblePanels,
  type PanelKind,
} from "./lib/sidePanel";
import { ContainerLogsView } from "./components/ContainerLogsView";
import { ContainerShellBanner, ContainerStrip } from "./components/ContainerStrip";
import { ContainerFilesPanel } from "./components/ContainerFilesPanel";
import {
  baseName,
  containerFilesTarget,
  dirtyEditorsOf,
  isEditorDirty,
  openDiffTab as openDiffContentTab,
  openEditorTab,
  openHistoryTab,
  parseContentView,
  setEditorDirty,
} from "./lib/contentTabs";
import { AgentsPanel } from "./components/AgentsPanel";
import { PrListView } from "./components/PrListView";
import { GitScreen } from "./components/GitScreen";
import { activeFile, allScreenEditors, closeTarget, screenOf, sideSlotView } from "./lib/screens";
import { openGitScreen } from "./lib/gitScreen";
import { PrDetailView } from "./components/PrDetailView";
import { SubagentViewer } from "./components/SubagentViewer";
import { ForgePanel } from "./components/ForgePanel";
import { PasteConfirmDialog } from "./components/PasteConfirmDialog";
import { SnippetArgsDialog } from "./components/SnippetArgsDialog";
import { DiffStat } from "./components/DiffStat";
import { SessionHoverCard } from "./components/SessionHoverCard";
import { PromptDialog } from "./components/PromptDialog";
import { TerminalSearch } from "./components/TerminalSearch";
import {
  SettingsView,
  type DetailsPref,
  type SidebarTogglePref,
  type StartupMode,
} from "./components/SettingsView";
import { TabBar } from "./components/TabBar";
import {
  requestTerminalRelayout,
  FONT_SIZE_EVENT,
  getDefaultFontSize,
  setDefaultFontSize,
  TERMINAL_CELL_WIDTH,
  TERMINAL_LINE_HEIGHT,
  TerminalView,
} from "./components/TerminalView";
import {
  activateTab,
  activateWorkspace,
  closeAgentViewers,
  closePane,
  closeSideView,
  closeTab,
  closeWorkspace,
  createSession,
  createTab,
  createWorkspace,
  disposeSession,
  dockerAvailable,
  dockerListContainers,
  ackDriftWarning,
  bootGate,
  bootSnapshot,
  onAgentOpenUrl,
  onAgentSandboxWarning,
  onAgentAuthAlert,
  onAgentAlert,
  type AuthAlertKind,
  onAppReady,
  onBootFailed,
  focusPane,
  layoutState,
  leafSessions,
  listApprovals,
  listSessions,
  newWindow,
  applyLaunchConfig,
  deleteLaunchConfig,
  launchConfigSeed,
  listLaunchConfigs,
  type LaunchConfig,
  type LaunchConfigId,
  onAnyAgentReady,
  onApprovalRequested,
  onApprovalResolved,
  onLaunchConfigPrefill,
  onLayoutChanged,
  onSessionCommand,
  onSessionCwd,
  onSessionIntegration,
  onSessionRemoteChips,
  onSessionStatus,
  onSessionTransport,
  remoteAgentWithoutJail,
  sessionIntegration as fetchSessionIntegration,
  sessionRemoteChips,
  type Integration,
  type RemoteChips,
  type SessionTransport,
  filesSearch,
  type FileSearchResult,
  backTab,
  openFilesPanel,
  openTunnelsPanel,
  openAgentsPanel,
  openGitPanel,
  openDockerPanel,
  openHistoryPanel,
  openViewTab,
  toggleAgentQueue,
  paneSession,
  listSubagents,
  focusSubagent,
  onSubagentsChanged,
  detectedAgent,
  killShellAgent,
  canResumeAgentSession,
  resumeAgentSession,
  resumeShellConversation,
  dismissShellResumeOffer,
  type AgentRunner,
  onAgentDetected,
  type AgentDetectedPayload,
  type ShimRoute,
  type NoJailReason,
  type DetectedAgent,
  type SubagentSnapshot,
  renameWorkspace,
  onRepoChanged,
  onRepoReconciled,
  repoSnapshots as fetchRepoSnapshots,
  sessionBracketedPaste,
  sessionCwd,
  sessionMarkSeen,
  sessionGitStatus,
  setAgentMatchPattern,
  submitRichInput,
  setPref,
  setSideViewExpanded,
  setSideViewRatio,
  setSplitRatio,
  setWorkspaceColor,
  setWorkspaceGroup,
  splitPane,
  type ApprovalRequest,
  type LayoutState,
  type Loaded,
  type RepoSnapshot,
  type Session,
  type Tab,
  type SessionCommand,
  type ConflictState,
  type SessionCwd,
  type SessionGitStatus,
  type SessionId,
  type SessionKind,
  type SplitKind,
  type Workspace,
  type Host,
  type HostGroup,
  broadcastWrite,
  broadcastSubmit,
  connectHost,
  connectHostGroup,
  reconnectSsh,
  listHosts,
  listHostGroups,
  onHostsChanged,
  appVersion,
  updateCheck,
  setAppMenu,
  onMenuAction,
  onBlockFinalized,
  onBlocksCleared,
  onSessionPromptMode,
  sessionBlocks,
  sessionLineEcho,
  sessionHookExpected,
  sessionShellBoot,
  sessionPromptMode,
  togglePromptMode,
  renderSnippet,
  snippetPlaceholders,
  type Block,
  type Snippet,
  type SnippetPlaceholder,
  updateDismiss,
  writeToSession,
  type UpdateStatus,
  type BootFailure,
} from "./lib/ipc";
import { bootFailureTitleKey } from "./lib/bootFailure";
import { hiddenApprovalIds as computeHiddenApprovalIds } from "./lib/notifications";
import {
  panelRequestsBySession,
  panelVisibleApprovalIds,
  withoutResolved,
} from "./lib/approvalPanel";
import { basename } from "@/lib/utils";
import { buildConflictPrompt } from "./lib/conflicts";
import {
  connectingHostIds,
  isFinishedStatus,
  mergeSessionUpdate,
} from "./lib/sessionStatus";
import {
  AGENTS_PANEL_LINGER_MS,
  agentsPanelActionableIds,
  agentsPanelRunConcluded,
  agentsPanelSession,
  agentsPanelUngated,
  deadAgentsPanels,
  mainColumn,
  showAgentsButton,
  subagentInlineApprovals,
  trackPanelRun,
  type PanelRunEntry,
} from "./lib/agentsPanel";
import { usePresence } from "./lib/usePresence";
import {
  SPINNER_FRAMES,
  SPINNER_INTERVAL_MS,
  windowTitle,
} from "./lib/windowTitle";
import { gitIconTone } from "./lib/headerGit";
import {
  buildAgentSessionOpts,
  runnerFromCommand,
  type AgentRunnerId,
} from "./lib/agentSession";
import { scheduleAgentReadyPrompt } from "./lib/agentReady";
import {
  mergeLoaded,
  nextBootPoll,
  parseStartupMode,
  shouldPromptNewSession,
  SPLASH_DONE_EVENT,
} from "./lib/startup";
import {
  compactPath,
  resolveWorkspaceCwd,
  workspaceMatchDir,
} from "./lib/workspaceCwd";
import { findSessionLocation } from "./lib/sessionLocation";
import {
  agentQueueVisibleApprovalIds,
  boardOrder,
  buildRows as buildAgentRows,
  nextAttention,
  wantsAttention,
  agentForWorkspace,
  type SessionPlace,
} from "./lib/agentsBoard";
import {
  computeRects,
  findAncestorSplit,
  type DividerRect,
} from "./lib/panes";
import {
  DEFAULT_TOOLBAR,
  parseToolbarPref,
  snapshotForDir,
  toolbarBranchChip,
  type ToolbarPref,
} from "./lib/repoSnapshots";
import { Toolbar } from "./components/Toolbar";
import {
  LIVE_DELAY_MS,
  termRect,
  usedFraction,
} from "./components/ActiveBlock";
import { SessionBlocks } from "./components/SessionBlocks";
import { sessionBlocksData } from "./lib/sessionBlocks";

/**
 * Intervalo da consulta ao modo do tty, em ms.
 *
 * Só roda com comando em execução. Curto o bastante para acompanhar a troca de
 * canônico para raw no meio de um mesmo comando (o menu do `npm create` abre
 * segundos depois do `Ok to proceed?`), e longo o bastante para não pesar num
 * core que já disputa CPU com os agentes.
 */
const LINE_ECHO_POLL_MS = 200;

/** Piso do painel lateral. Os 240px são a largura fixa da coluna da árvore
 *  (`FilesPanel`, `w-[240px] shrink-0`, que por ser `shrink-0` não cede); o
 *  resto é o mínimo para o conteúdo ao lado dela não virar uma fresta. Abaixo
 *  disso o painel fica menor que o próprio conteúdo e a árvore transborda. */
const SIDE_MIN_PX = 360;
/** Largura da superfície de agentes (árvore + detalhe inline). */
const AGENTS_PANEL_WIDTH_PX = 380;
/** Piso do terminal. Sem ele o arrasto para o outro lado engole a área de
 *  trabalho, que é o problema simétrico e igualmente fácil de provocar. */
const MAIN_MIN_PX = 320;
/**
 * A área de conteúdo de um painel, nas coordenadas DELA: o painel inteiro.
 *
 * Terminal e blocos moram dentro de um invólucro por painel que já desconta a
 * linha do TYBA, então medem em % desse invólucro e não da área de painéis.
 * Objeto único e fixo: o `rect` derivado dele compara igual entre renders
 * (`sameRect`), e o fit só roda quando o invólucro muda de tamanho de verdade.
 */
const PANE_BOX = { left: 0, top: 0, width: 100, height: 100 } as const;
import { handlerCache, withEntry } from "./lib/perSession";
import { useStableCallback } from "./lib/stableProps";
import { mergeBlockHistory } from "./lib/blockHistory";
import { blocksMarkdown, wipesTheScreen } from "./lib/blockText";
import {
  inTextField,
  modeFor,
  pickedBlocks,
  selectBlock,
  type BlockSelection,
} from "./lib/blockSelection";
import { CommandLine } from "./components/CommandLine";
import { RichInput } from "./components/RichInput";
import {
  DEFAULT_RICH_INPUT,
  RICH_INPUT_PREF_KEY,
  parseRichInputPref,
  richInputVisibility,
  shouldShowRichInput,
  type RichInputPref,
} from "./lib/richInput";
import {
  captureState,
  comboOf,
  tabBackRoute,
  DEFAULT_BINDINGS,
  KEY_ACTIONS,
  formatCombo,
  parseBindings,
  BINDINGS_PREF_KEY,
  isTerminalAction,
  isPaneResizeChord,
  isTabDigitChord,
  type Bindings,
  type KeyAction,
} from "./lib/keys";
import { type PaletteMode } from "./lib/paletteMode";
import {
  buildMenuSpec,
  isMenuExtraId,
  type MenuExtraId,
} from "./lib/appMenu";
import {
  paneLine,
  type PaneLine,
  programName,
  swallowsArrow,
  PROMPT_MODE_PREF_KEY,
  promptModeEnabled,
} from "./lib/commandLine";
import {
  INITIAL_TRANSPORT,
  isIntegratedSession,
  nextTransport,
  paneNotice,
  remoteAgentNotice,
  sidebarChips,
  toolbarChips,
} from "./lib/remoteSession";
import { changelogUrl } from "./lib/changelog";
import { docsUrl, REPO_URL } from "./lib/links";
import {
  flattenPaste,
  openExternalUrl,
  readClipboardText,
  routePaste,
  sanitizePaste,
  writeClipboardText,
} from "./lib/clipboard";
import {
  getTerm,
  isTermFocused,
  suppressNativePaste,
  type TerminalPasteDetail,
} from "./lib/termRegistry";
import { HoverCard, HoverCardTrigger } from "@/components/ui/hover-card";
import { Shortcut } from "@/components/ui/kbd";
import type { ShellBoot } from "./lib/loading";

const EMPTY_LAYOUT: LayoutState = { workspaces: [], active_workspace: null };
type MenuParts = {
  Item: ElementType;
  Separator: ElementType;
  Sub: ElementType;
  SubTrigger: ElementType;
  SubContent: ElementType;
};

const DROPDOWN_MENU_PARTS: MenuParts = {
  Item: DropdownMenuItem,
  Separator: DropdownMenuSeparator,
  Sub: DropdownMenuSub,
  SubTrigger: DropdownMenuSubTrigger,
  SubContent: DropdownMenuSubContent,
};

const CONTEXT_MENU_PARTS: MenuParts = {
  Item: ContextMenuItem,
  Separator: ContextMenuSeparator,
  Sub: ContextMenuSub,
  SubTrigger: ContextMenuSubTrigger,
  SubContent: ContextMenuSubContent,
};

const TOGGLE_PREF_KEY = "pref.sidebar_toggle";
const DETAILS_PREF_KEY = "pref.sidebar_details";
const DETAILS_OVERRIDES_KEY = "pref.session_details";
const TOOLBAR_PREF_KEY = "pref.toolbar";
const WORKTREE_DEFAULT_KEY = "pref.worktree_default";
const EDITOR_PREF_KEY = "pref.editor";
const REVIEW_AGENT_KEY = "pref.review_agent";
const DEFAULT_REVIEW_AGENT = "claude";
const ACCOUNT_NAME_KEY = "pref.account_name";
const FONT_SIZE_KEY = "pref.code.font_size";
const SHOW_CONTAINERS_KEY = "pref.code.show_containers";
const GIT_STATUS_KEY = "pref.git_status";
const SHELL_INTEGRATION_KEY = "pref.shell_integration";
const STARTUP_KEY = "pref.startup";

function runnerLabel(kind: SessionKind): string | null {
  if (kind.type !== "agent") return null;
  if (kind.runner === "claude_code") return "claude";
  if (kind.runner === "codex") return "codex";
  return kind.runner.custom;
}

function isConfigWorkspace(w: Workspace): boolean {
  return w.tabs.length > 0 && w.tabs.every((t) => t.view === "settings");
}

function isConnectionsWorkspace(w: Workspace): boolean {
  return w.tabs.length > 0 && w.tabs.every((t) => t.view === "connections");
}

function isWorktreesWorkspace(w: Workspace): boolean {
  return w.tabs.length > 0 && w.tabs.every((t) => t.view === "workspace");
}

function isStatsWorkspace(w: Workspace): boolean {
  return w.tabs.length > 0 && w.tabs.every((t) => t.view === "stats");
}

function isAgentBoardWorkspace(w: Workspace): boolean {
  return w.tabs.length > 0 && w.tabs.every((t) => t.view === "agent-board");
}

const AGENT_COMMAND = /^\s*(?:\S*\/)?(claude|codex|gemini)\b/;

function agentFromCommand(command: string | null): string | null {
  if (!command) return null;
  return AGENT_COMMAND.exec(command)?.[1] ?? null;
}

function agentGlyph(label: string, size = 16): React.ReactNode {
  if (label === "claude")
    return (
      <ClaudeIcon
        size={size}
        className="shrink-0"
        style={{ color: "#d97757" }}
      />
    );
  if (label === "codex")
    return <OpenAIIcon size={size} className="shrink-0 text-tyba-text" />;
  return <AgentIcon size={size} className="shrink-0" />;
}

const SESSION_COLORS = [
  "green",
  "amber",
  "magenta",
  "violet",
  "blue",
  "cyan",
  "red",
];

const copyText = (text: string) => {
  void navigator.clipboard?.writeText(text).catch(() => {});
};

type SidebarMode = "open" | "rail" | "hidden";

const SIDEBAR_WIDTH: Record<SidebarMode, number> = {
  open: 224,
  rail: 44,
  hidden: 0,
};

function IconAction({
  label,
  shortcut,
  onClick,
  children,
}: {
  label: string;
  shortcut?: string;
  onClick?: () => void;
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          onClick={onClick}
          aria-label={label}
          className="h-7 w-[30px] rounded-[6px] text-tyba-text-faint hover:text-tyba-text"
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom" className="flex items-center gap-2">
        {label}
        {shortcut && <Shortcut combo={shortcut} />}
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * Shim v2, Track C (tech-spec §7): o que `detectedBySession` guarda por
 * sessão. `hosting`/`jailed` chegam junto do `detected` (evento ou probe sob
 * demanda, mesma forma — ver `AgentDetectedPayload`) e nunca são persistidos
 * à parte: são derivados a cada leitura, e viajam sempre COM a identidade do
 * processo a que pertencem.
 */
type DetectedAgentState = DetectedAgent & {
  hosting: boolean;
  jailed: boolean;
  shim: ShimRoute | null;
  noJail: NoJailReason | null;
};

const withHostingState = (p: AgentDetectedPayload): DetectedAgentState | null =>
  p.detected
    ? {
        ...p.detected,
        hosting: p.hosting,
        jailed: p.jailed,
        shim: p.shim ?? null,
        noJail: p.no_jail ?? null,
      }
    : null;

/** Shim v2 (tech-spec §9): flag local de "já mostrou o toast de introdução",
 * uma vez por usuário. Durabilidade local — pior caso de perder o flag
 * (reinstall, limpar dados do app) é mostrar o toast de novo, nunca um gate
 * a menos: não é decisão de segurança. */
const HOSTING_INTRO_TOAST_SEEN_KEY = "tyba.shim-v2-hosting-intro-seen";

export default function App() {
  const { t, i18n } = useTranslation();
  const [sessions, setSessions] = useState<Session[]>([]);
  const [layout, setLayout] = useState<LayoutState>(EMPTY_LAYOUT);
  const [subagentsBySession, setSubagentsBySession] = useState<
    Map<SessionId, SubagentSnapshot>
  >(() => new Map());
  const [detectedBySession, setDetectedBySession] = useState<
    Map<SessionId, DetectedAgentState>
  >(() => new Map());
  // F2/F3 do detectar-agente-no-shell: "Ignorar" esconde o aviso só pra
  // aquela instância de processo (pid+start) — um agente novo re-avisa.
  const [dismissedShellNotices, setDismissedShellNotices] = useState<
    Map<SessionId, string>
  >(() => new Map());
  // Veredito do core sobre retomar a conversa nativa de uma sessão de agente
  // morta. `undefined` = ainda não perguntado; o convite só nasce do `true`.
  const [resumableAgents, setResumableAgents] = useState<
    Map<SessionId, boolean>
  >(() => new Map());
  const [dismissedResumeInvites, setDismissedResumeInvites] = useState<
    Set<SessionId>
  >(() => new Set());
  // Entrega C, F2/F3: a faixa de auth de RUNTIME (fase preflight vira toast,
  // não entra aqui -- ver o listener de `agent://auth-alert`). Some da tela
  // por dois caminhos: dispensa manual (`dismissAuthAlert`) ou a sessão
  // voltar a `running` -- o mesmo "recovery" que zera o dedupe no core.
  const [runtimeAuthAlerts, setRuntimeAuthAlerts] = useState<
    Map<SessionId, AuthAlertKind>
  >(() => new Map());
  // Quantas vezes o core re-subiu um processo NESTE mesmo id de sessão. Entra na
  // `key` do TerminalView: sem isso o xterm continuaria montado sobre um PTY que
  // não existe mais, sem reatar no novo e sem nunca receber saída.
  const [paneEpochs, setPaneEpochs] = useState<Map<SessionId, number>>(
    () => new Map(),
  );
  const [sidebar, setSidebar] = useState<SidebarMode>("open");
  const [togglePref, setTogglePref] = useState<SidebarTogglePref>("hidden");
  const [detailsPref, setDetailsPref] = useState<DetailsPref>("on");
  const [detailOverrides, setDetailOverrides] = useState<
    Record<string, DetailsPref>
  >({});
  const [bindings, setBindings] = useState<Bindings>(DEFAULT_BINDINGS);
  const [accountName, setAccountName] = useState("");
  const [sessionQuery, setSessionQuery] = useState("");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [paletteMode, setPaletteMode] = useState<PaletteMode>("actions");
  const [newSessionOpen, setNewSessionOpen] = useState(false);
  const [newSessionIsolate, setNewSessionIsolate] = useState(false);
  const [worktreeDir, setWorktreeDir] = useState<string | null>(null);
  const [worktreeDefault, setWorktreeDefault] = useState(false);
  const [launchConfigs, setLaunchConfigs] = useState<LaunchConfig[]>([]);
  const [launchDraft, setLaunchDraft] =
    useState<LaunchConfigDraftState | null>(null);
  const agentReadyCancels = useRef<Map<string, () => void>>(new Map());
  const [agentReadyWarnings, setAgentReadyWarnings] = useState<
    Record<SessionId, boolean>
  >({});

  const [launchPrefills, setLaunchPrefills] = useState<
    Record<SessionId, string>
  >({});

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    void onLaunchConfigPrefill(({ session_id, prompt }) => {
      setLaunchPrefills((prev) => ({ ...prev, [session_id]: prompt }));
      setRichInputOpened((prev) => new Set(prev).add(session_id));
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const refreshLaunchConfigs = useCallback(() => {
    void listLaunchConfigs()
      .then(setLaunchConfigs)
      .catch(() => setLaunchConfigs([]));
  }, []);

  useEffect(() => {
    if (!paletteOpen) return;
    refreshLaunchConfigs();
  }, [paletteOpen, refreshLaunchConfigs]);

  const newLaunchConfig = useCallback(() => {
    const slotId = crypto.randomUUID();
    setLaunchDraft({
      name: "",
      repoRoot: "",
      slots: [
        {
          id: slotId,
          name: "slot-1",
          kind: { type: "shell" },
          cwd_rel: null,
          isolate: false,
          initial_prompt: null,
        },
      ],
      tabs: [
        {
          id: crypto.randomUUID(),
          title: null,
          root: { type: "leaf", id: crypto.randomUUID(), slot_id: slotId },
        },
      ],
    });
  }, []);

  const editLaunchConfig = useCallback(
    (id: LaunchConfigId) => {
      const config = launchConfigs.find((c) => c.id === id);
      if (!config) return;
      setLaunchDraft({
        id: config.id,
        name: config.name,
        repoRoot: config.repo_root,
        slots: config.slots,
        tabs: config.tabs,
      });
    },
    [launchConfigs],
  );

  const removeLaunchConfig = useCallback(
    async (id: LaunchConfigId) => {
      const config = launchConfigs.find((c) => c.id === id);
      const confirmed = await requestConfirm({
        title: t("launchDeleteConfirm", { name: config?.name ?? "" }),
        confirmLabel: t("launchDelete"),
        destructive: true,
      });
      if (!confirmed) return;
      try {
        await deleteLaunchConfig(id);
        await refreshLaunchConfigs();
      } catch (e) {
        toastError(t("launchDeleteFailed"), e);
      }
    },
    [launchConfigs, refreshLaunchConfigs, t],
  );

  const saveWorkspaceAsLaunchConfig = useCallback(async () => {
    try {
      const seed = await launchConfigSeed();
      setLaunchDraft({
        name: seed.name,
        repoRoot: seed.repo_root,
        slots: seed.slots,
        tabs: seed.tabs,
      });
    } catch (e) {
      toastError(t("launchNeedsRepo"), e);
    }
  }, [t]);

  const applyLaunchConfigById = useCallback(
    async (id: LaunchConfigId) => {
      const config = launchConfigs.find((c) => c.id === id);
      try {
        const applied = await applyLaunchConfig(id, 100, 30);
        if (applied.failures.length > 0) {
          pushToast({
            tone: "warning",
            title: t("launchAppliedWithFailures", {
              name: config?.name ?? "",
              count: applied.failures.length,
            }),
            detail: applied.failures
              .map((f) => `${f.slot}: ${f.message}`)
              .join("; "),
          });
        }
      } catch (e) {
        toastError(t("launchApplyFailed"), e);
      }
    },
    [launchConfigs, t],
  );

  const openNewSession = useCallback(
    (isolate?: boolean) => {
      setNewSessionIsolate(isolate ?? worktreeDefault);
      setNewSessionOpen(true);
    },
    [worktreeDefault],
  );
  const [searchOpen, setSearchOpen] = useState(false);
  const [altScreens, setAltScreens] = useState<Record<string, boolean>>({});
  const [promptModes, setPromptModes] = useState<Record<string, boolean>>({});
  // O listener de comando é assinado uma vez por sessão; sem ref ele leria o
  // modo do primeiro render para sempre.
  const promptModesRef = useRef<Record<string, boolean>>({});
  promptModesRef.current = promptModes;
  const [promptModePref, setPromptModePref] = useState(false);
  /**
   * O que foi digitado e não enviado, por sessão.
   *
   * Cada painel tem a própria linha, e trocar o foco não a remonta. Mas ela
   * remonta quando o painel sai da tela (troca de aba) e ao voltar de um app
   * de tela cheia, e o rascunho ia junto: meio comando escrito, uma ida a
   * outra aba, e ele sumia sem aviso. Nunca vai para o disco.
   *
   * `ref` e não estado: ninguém lê isto durante o render, só a próxima
   * montagem da caixa. Como estado, cada tecla digitada re-renderizaria o
   * `App` inteiro para guardar um valor que ninguém está olhando.
   */
  const drafts = useRef<Record<string, string>>({});
  // Estável de propósito: como seta inline, ela seria uma função nova a cada
  // render do `App`, e o efeito que guarda o rascunho re-rodaria junto —
  // trabalho por tecla digitada para escrever o mesmo valor de novo.
  const rememberDraft = useCallback((id: SessionId, text: string) => {
    drafts.current[id] = text;
  }, []);
  // As linhas já enviadas de um comando em continuação (`ls \`), por sessão e
  // pelo mesmo motivo do rascunho: sobreviver à remontagem da caixa. Só em
  // memória, como o rascunho.
  const pendings = useRef<Record<string, string[]>>({});
  const rememberPending = useCallback((id: SessionId, lines: string[]) => {
    pendings.current[id] = lines;
  }, []);
  /**
   * Sessões que subiram com o hook do TYBA injetado.
   *
   * Separado de `promptModes` porque as duas perguntas são diferentes:
   * `promptModes` é o que o shell RELATOU, e ausente ali significa "ainda não
   * relatou". Aqui é se ele algum dia vai relatar — a integração só existe
   * para `bash` e `zsh`. Sem essa distinção, um `fish` fica para sempre com a
   * linha do TYBA na tela dizendo "Carregando o shell…".
   */
  const [hookExpected, setHookExpected] = useState<Record<string, boolean>>({});
  /** O shell e o rc de cada sessão local subindo: a etapa da linha em espera. */
  const [shellBoots, setShellBoots] = useState<Record<string, ShellBoot | null>>({});
  const [blocks, setBlocks] = useState<Record<string, Block[]>>({});
  // Lido por handler de clique e de tecla; num deles é `window`, e reassinar o
  // listener a cada bloco que chega seria trabalho por nada.
  const blocksRef = useRef<Record<string, Block[]>>({});
  blocksRef.current = blocks;
  const [snippetPrompt, setSnippetPrompt] = useState<{
    snippet: Snippet;
    placeholders: SnippetPlaceholder[];
    /** O painel que pediu o snippet. Ver `pickSnippet`. */
    sessionId: SessionId;
  } | null>(null);
  const [pastePrompt, setPastePrompt] = useState<TerminalPasteDetail | null>(
    null,
  );
  const [repoSnapshots, setRepoSnapshots] = useState<
    Record<string, RepoSnapshot>
  >({});
  const [toolbarPref, setToolbarPref] = useState<ToolbarPref>(DEFAULT_TOOLBAR);
  const [richInputPref, setRichInputPref] =
    useState<RichInputPref>(DEFAULT_RICH_INPUT);
  const [richInputOpened, setRichInputOpened] = useState<Set<string>>(
    () => new Set(),
  );
  const [richInputDismissed, setRichInputDismissed] = useState<Set<string>>(
    () => new Set(),
  );
  const [richInputFocusNonce, setRichInputFocusNonce] = useState(0);
  const [richInputRegexInvalid, setRichInputRegexInvalid] = useState(false);
  const [editorPref, setEditorPref] = useState("");
  const [reviewAgent, setReviewAgent] = useState(DEFAULT_REVIEW_AGENT);
  const richInputFocused = useRef(false);
  const richInputAutoOpened = useRef<Set<string>>(new Set());
  const dismissedCommand = useRef<Record<string, string | null>>({});
  const [showGitStatus, setShowGitStatus] = useState(true);
  const [shellIntegration, setShellIntegration] = useState(true);
  const [startup, setStartup] = useState<StartupMode>("resume");
  const [menuWorkspace, setMenuWorkspace] = useState<string | null>(null);
  const [prompt, setPrompt] = useState<{
    kind: "rename" | "group";
    ws: Workspace;
  } | null>(null);
  const [pendingGroup, setPendingGroup] = useState<string | null>(null);
  const [showContainers, setShowContainers] = useState(false);
  const [dockerUp, setDockerUp] = useState(true);
  const [dockerRunning, setDockerRunning] = useState(0);
  const [theme, setTheme] = useState<ThemeMode>(getThemeMode);
  const [approvals, setApprovals] = useState<ApprovalRequest[]>([]);
  const [inboxOpen, setInboxOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [version, setVersion] = useState("");
  const [update, setUpdate] = useState<UpdateStatus | null>(null);

  useEffect(() => {
    void appVersion()
      .then(setVersion)
      .catch(() => {});
    void updateCheck()
      .then(setUpdate)
      .catch(() => {});
  }, []);
  const booted = useRef(false);

  // A decisão nasce em dois pontos — no snapshot, quando ele já vem pronto, e
  // no `app://ready`, quando não vinha — e vale uma vez só. O ref é o que
  // garante a unicidade; `shouldPromptNewSession` é quem decide.
  const promptNewSessionIfEmpty = useCallback(
    (ready: boolean, workspaces: number) => {
      const prompted = booted.current;
      if (!shouldPromptNewSession({ ready, workspaces, prompted })) return;
      booted.current = true;
      setNewSessionOpen(true);
    },
    [],
  );

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const track = (p: Promise<() => void>) => {
      void p.then((un) => (disposed ? un() : unlisteners.push(un)));
    };
    // Listeners primeiro: eventos que chegam antes do snapshot viram deltas
    // sobre ele (união por id no requested; filtro no resolved), em vez de
    // serem sobrescritos por listApprovals().
    track(
      onApprovalRequested((request) =>
        setApprovals((prev) =>
          prev.some((p) => p.id === request.id) ? prev : [...prev, request],
        ),
      ),
    );
    track(
      onApprovalResolved(({ id }) =>
        setApprovals((prev) => withoutResolved(prev, id)),
      ),
    );
    listApprovals()
      .then((pending) => {
        if (disposed) return;
        setApprovals((prev) => {
          const seen = new Set(prev.map((p) => p.id));
          return [...prev, ...pending.filter((p) => !seen.has(p.id))];
        });
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, []);

  const openPalette = useCallback((mode: PaletteMode) => {
    setPaletteMode(mode);
    setPaletteOpen(true);
  }, []);

  const changeTheme = useCallback((next: ThemeMode) => {
    setThemeMode(next);
    setTheme(next);
  }, []);

  useEffect(() => onThemeModeChange(setTheme), []);

  const activeWorkspace = useMemo(
    () =>
      layout.workspaces.find((w) => w.id === layout.active_workspace) ?? null,
    [layout],
  );

  const activeTab = useMemo(
    () =>
      activeWorkspace?.tabs.find((tab) => tab.id === activeWorkspace.active_tab) ??
      null,
    [activeWorkspace],
  );

  const activeId = useMemo(
    () =>
      activeTab?.root && activeTab.active_pane
        ? paneSession(activeTab.root, activeTab.active_pane)
        : null,
    [activeTab],
  );

  // Sessão dona dos arquivos do que está em foco: o pane ativo; numa aba de
  // editor/diff, a dona dela; numa aba de logs, o primeiro terminal do
  // workspace. É o alvo do ⌘⇧E e da busca de arquivos da paleta.
  const fileSessionId = useMemo(() => {
    if (activeId) return activeId;
    const content = parseContentView(activeTab?.view);
    if (content && content.kind !== "container-logs") return content.session;
    for (const tab of activeWorkspace?.tabs ?? []) {
      if (tab.root) return leafSessions(tab.root)[0] ?? null;
    }
    return null;
  }, [activeId, activeTab, activeWorkspace]);

  const searchFilesForPalette = useCallback(
    (query: string): Promise<FileSearchResult> =>
      fileSessionId
        ? filesSearch(fileSessionId, query)
        : Promise.resolve({ paths: [], truncated: false }),
    [fileSessionId],
  );

  const openFileFromFinder = useCallback(
    (rel: string) => {
      if (!fileSessionId) return;
      void openEditorTab(rel, undefined, { session: fileSessionId }).catch((e) =>
        toastError(t("editorOpenError"), e),
      );
    },
    [fileSessionId, t],
  );

  /**
   * O core terminou de carregar?
   *
   * `false` não é "vazio", é "ainda não sei". O boot do core roda numa thread
   * e um comando que chega antes dela terminar devolve listas vazias — tratar
   * isso como estado final desenha "nenhuma sessão" por cima das que estão
   * voltando.
   */
  const [bootReady, setBootReady] = useState(false);
  // Espelho de `bootReady` para quem lê de dentro de um `await`: a closure de
  // uma resposta que demorou guarda o valor de quando a chamada saiu, e o boot
  // pode ter terminado no meio do caminho.
  const bootReadyRef = useRef(false);
  const markReady = useCallback(() => {
    bootReadyRef.current = true;
    setBootReady(true);
  }, []);

  /**
   * Porta única das respostas do core que carregam `ready`.
   *
   * Devolve o dado quando ele vale como estado, e `null` quando a resposta é do
   * meio do boot — aí quem aplicaria estaria apagando o que está voltando. As
   * duas invariantes (o `ready` não regride; a lista só se aplica quando
   * `ready`) vivem em `mergeLoaded`, e não em cada chamador.
   */
  const acceptLoaded = useCallback(
    <T,>(response: Loaded<T> | null | undefined): T | null => {
      const update = mergeLoaded(bootReadyRef.current, response);
      if (update.ready) markReady();
      return update.value;
    },
    [markReady],
  );

  // O splash espera por isto. Ver `main.tsx`.
  useEffect(() => {
    if (bootReady) window.dispatchEvent(new Event(SPLASH_DONE_EVENT));
  }, [bootReady]);

  const paneLayout = useMemo(
    () => (activeTab?.root ? computeRects(activeTab.root) : null),
    [activeTab],
  );

  /**
   * As sessões que ocupam um painel em ALGUMA aba, de qualquer workspace.
   *
   * Só elas ganham um `xterm.js`. Sem este corte, o `sessions.map` do render
   * montava um terminal por sessão da LISTA — e a lista guarda sessão morta: no
   * modo `Resume`, que é o padrão, agente e container encerrados nunca são
   * esquecidos. Medido num banco real: 22 sessões, todas `exited`, zero painéis
   * — 22 instâncias de xterm construídas no boot, nenhuma visível, cada uma com
   * seu `attach_session`, seus listeners e seu `ResizeObserver`. E como a
   * tabela só cresce, o app ficava mais lento a cada uso.
   *
   * > [!warning] Em QUALQUER aba, não na ativa. `paneLayout` acima só conhece a
   * > aba ativa; filtrar por ele desmontaria os terminais das outras abas a
   * > cada troca, e o xterm remontado volta com a tela que o core reenvia, não
   * > com o scrollback que o webview tinha. Trocaria um problema por outro.
   */
  const pannedSessions = useMemo(() => {
    const ids = new Set<string>();
    for (const workspace of layout.workspaces) {
      for (const tab of workspace.tabs) {
        if (!tab.root) continue;
        for (const pane of computeRects(tab.root).panes) ids.add(pane.session);
      }
    }
    return ids;
  }, [layout.workspaces]);

  const workspaces = useMemo(() => {
    const query = sessionQuery.trim().toLowerCase();
    if (!query) return layout.workspaces;
    return layout.workspaces.filter((w) =>
      `${w.name} ${w.repo_root ?? ""}`.toLowerCase().includes(query),
    );
  }, [layout.workspaces, sessionQuery]);

  /** Recarrega a lista de sessões. Devolve se o core respondeu pronto. */
  const refreshSessions = useCallback(async () => {
    const all = await listSessions().catch(() => null);
    const loaded = acceptLoaded(all);
    if (loaded) setSessions(loaded);
    return loaded !== null;
  }, [acceptLoaded]);

  /** O layout também veio vazio do meio do boot; reconsultar é a outra metade. */
  const refreshLayout = useCallback(() => {
    void layoutState()
      .then((next) => {
        const loaded = acceptLoaded(next);
        if (!loaded) return;
        setLayout(loaded);
        promptNewSessionIfEmpty(true, loaded.workspaces.length);
      })
      .catch(() => {});
  }, [acceptLoaded, promptNewSessionIfEmpty]);

  /**
   * As 16 preferências, aplicadas de uma vez — e recuperáveis.
   *
   * Antes cada uma era um `getPref` com `.catch` próprio, então uma falha
   * custava UMA configuração. Trocar por uma chamada só tirou 18 travessias da
   * main thread do caminho crítico, mas transformou a falha em tudo-ou-nada: se
   * o `boot_snapshot` rejeitar — e ele rejeita quando o `store.prefs()` falha,
   * por exemplo com `SQLITE_BUSY` depois do `busy_timeout` enquanto a thread de
   * boot martela o mesmo mutex —, TODA preferência cai no padrão pela vida da
   * janela: fonte, toolbar, atalhos, modo prompt, editor.
   *
   * O `ref` é o que permite recuperar sem estragar: o poll já volta a perguntar
   * enquanto o boot não terminou, então ele reaplica se a primeira tentativa não
   * trouxe nada — e uma vez aplicadas, uma resposta atrasada não pisa por cima
   * do que o dono mudou nas Configurações nesse meio-tempo.
   */
  const prefsApplied = useRef(false);
  const applyPrefsRef = useRef<(p: Record<string, string>) => void>(() => {});
  /**
   * Repesca as preferências quando a primeira tentativa não as trouxe.
   *
   * Mora aqui, e não dentro de um dos caminhos, porque **os dois** chegam ao
   * pronto e nenhum é garantido: o `app://ready` costuma ganhar do primeiro
   * tick do poll — o boot normal termina em ~80ms —, e o poll é a única via
   * quando o evento se perde na janela do `listen()`. Instalada num caminho só,
   * a rede fica inalcançável justamente no caso comum.
   */
  const recoverPrefs = useCallback(() => {
    if (prefsApplied.current) return;
    void bootSnapshot()
      .then((full) => applyPrefsRef.current(full.prefs))
      .catch(() => {});
  }, []);
  const applyPrefs = useCallback((prefs: Record<string, string>) => {
    if (prefsApplied.current) return;
    prefsApplied.current = true;
    const pref = (key: string) => prefs[key] ?? null;
    const togglePrefRaw = pref(TOGGLE_PREF_KEY);
    const detailsRaw = pref(DETAILS_PREF_KEY);
    const overridesRaw = pref(DETAILS_OVERRIDES_KEY);
    const nameRaw = pref(ACCOUNT_NAME_KEY);
    const bindingsRaw = pref(BINDINGS_PREF_KEY);
    const fontRaw = pref(FONT_SIZE_KEY);
    const containersRaw = pref(SHOW_CONTAINERS_KEY);
    const gitStatusRaw = pref(GIT_STATUS_KEY);
    const shellIntegrationRaw = pref(SHELL_INTEGRATION_KEY);
    const startupRaw = pref(STARTUP_KEY);
    const toolbarRaw = pref(TOOLBAR_PREF_KEY);
    const richInputRaw = pref(RICH_INPUT_PREF_KEY);
    const editorRaw = pref(EDITOR_PREF_KEY);
    const worktreeDefaultRaw = pref(WORKTREE_DEFAULT_KEY);
    const reviewAgentRaw = pref(REVIEW_AGENT_KEY);
    const promptModeRaw = pref(PROMPT_MODE_PREF_KEY);
    setPromptModePref(promptModeEnabled(promptModeRaw));
    if (togglePrefRaw === "rail" || togglePrefRaw === "hidden") {
      setTogglePref(togglePrefRaw);
    }
    if (detailsRaw === "on" || detailsRaw === "off") setDetailsPref(detailsRaw);
    if (overridesRaw) {
      try {
        setDetailOverrides(
          JSON.parse(overridesRaw) as Record<string, DetailsPref>,
        );
      } catch {
        setDetailOverrides({});
      }
    }
    if (nameRaw) setAccountName(nameRaw);
    setBindings(parseBindings(bindingsRaw));
    setShowContainers(containersRaw === "on");
    setShowGitStatus(gitStatusRaw !== "off");
    setShellIntegration(shellIntegrationRaw !== "off");
    setStartup(parseStartupMode(startupRaw));
    setToolbarPref(parseToolbarPref(toolbarRaw));
    if (editorRaw) setEditorPref(editorRaw);
    setWorktreeDefault(worktreeDefaultRaw === "on");
    if (reviewAgentRaw) setReviewAgent(reviewAgentRaw);
    const richInput = parseRichInputPref(richInputRaw);
    setRichInputPref(richInput);
    if (richInput.agentRegex) {
      void setAgentMatchPattern(richInput.agentRegex)
        .then((accepted) => setRichInputRegexInvalid(!accepted))
        .catch(() => setRichInputRegexInvalid(true));
    }
    const fontSize = Number(fontRaw);
    if (fontSize >= 10 && fontSize <= 20) setDefaultFontSize(fontSize);
  }, []);
  applyPrefsRef.current = applyPrefs;

  // A thread de boot morreu, e o vazio na tela é falha e não ausência de dado.
  //
  // O core abre o portão mesmo em pânico — senão todo comando de escrita
  // pagaria o timeout de espera e a falha viraria lentidão —, e logo em seguida
  // manda o `app://ready`. Sem consumir ESTE evento, um boot que morreu fica
  // indistinguível de um app que simplesmente não tem sessão, e o usuário
  // conclui que perdeu o trabalho em vez de que algo quebrou.
  //
  // O aviso é o mesmo para as duas vias — o evento, que é o caminho rápido, e o
  // campo `bootFailure` do snapshot, que é a rede quando o evento se perde. Um
  // `ref` guarda o que já foi avisado: as duas podem chegar, e avisar duas
  // vezes da mesma falha é ruído.
  const bootFailureSeen = useRef(false);
  const reportBootFailure = useCallback(
    (failure: BootFailure) => {
      if (bootFailureSeen.current) return;
      bootFailureSeen.current = true;
      // O título vem do `kind`, não da mensagem — ver `bootFailureTitleKey`,
      // que é onde mora o porquê e o que quebra se aparecer origem nova.
      toastError(t(bootFailureTitleKey(failure.kind)), failure.message);
    },
    [t],
  );
  useEffect(() => {
    const unlisten = onBootFailed(reportBootFailure);
    return () => {
      void unlisten.then((off) => off());
    };
  }, [reportBootFailure]);

  // Entrega B (§5.3): o toast é a superfície PRIMÁRIA do 1-clique — com
  // $BROWSER setado, o fallback na tela do próprio Claude só aparece depois
  // de 3s (M8). O clique é a única coisa que abre o navegador (item 26 do
  // contrato) — nunca acontece sozinho.
  useEffect(() => {
    const unlisten = onAgentOpenUrl((payload) => {
      pushToast(agentOpenUrlToastInput(payload, t));
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [t]);

  // Aviso sem ação (ao contrário do toast acima) -- cada SandboxWarningKind
  // vira um toast warning com o texto do §6 (item 35 do contrato).
  //
  // Review r1 (v0.6.2), MAJOR: pro `FilhoDesconhecidoEmClaude` (alarme de
  // deriva), `onDismiss` manda o ack pro core (`ackDriftWarning`) SÓ quando
  // este toast é de fato dispensado -- nunca na mera chegada do evento. É o
  // que garante que o dedupe durável (`drift.warned_unclassified_children`
  // no core) só marca "visto" depois que o dono realmente viu: se o evento
  // se perder antes de virar toast (sem listener no instante do emit), o
  // `onDismiss` nunca roda, o ack nunca acontece, e o core volta a oferecer
  // o mesmo nome no próximo spawn de sessão.
  //
  // Review de segurança r2 (v0.6.2), MAJOR: e SE o toast chegar a aparecer,
  // não pode sumir sozinho -- nenhum `SandboxWarningKind` tem `action`, e
  // sem `sticky` o auto-dismiss de ~9s (`toastDuration`) dispararia o
  // `onDismiss` (e o ack durável) sem o dono ter visto nada. A montagem
  // inteira do `ToastInput` (sticky + onDismiss condicional) mora em
  // `sandboxWarningToastInput` -- testável sem i18n/IPC reais.
  useEffect(() => {
    const unlisten = onAgentSandboxWarning((payload) => {
      const title = t(sandboxWarningTitleKey(payload.kind), {
        detail: payload.detail ?? "",
      });
      pushToast(
        sandboxWarningToastInput(payload, title, (names) => {
          void ackDriftWarning(names).catch(() => {});
        }),
      );
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [t]);

  // Entrega C: `agent://auth-alert` roteia por `phase` (F1/F2 do contrato de
  // cobertura). Preflight roda ANTES de o agente falar -- a sessão ainda nem
  // tem prompt na tela, então vira toast (sticky, sem ação: a URL de login
  // da Entrega B chega depois, por `agent://open-url`, e não é este evento).
  // Runtime é a faixa por sessão (F2/F8) -- o `useEffect` seguinte, que já
  // observa `sessions`, cuida do "some" por recovery.
  useEffect(() => {
    const unlisten = onAgentAuthAlert((payload) => {
      if (payload.phase === "preflight") {
        pushToast(
          authAlertToastInput(
            t(authAlertMessageKey(payload.kind, payload.phase)),
          ),
        );
        return;
      }
      setRuntimeAuthAlerts((prev) =>
        withRuntimeAuthAlert(prev, payload.session_id, payload.kind),
      );
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [t]);

  // `agent://alert` (proposta 4 da ia-v2): falha de turno e limite de uso
  // viram toast. O aviso do sistema, fora de foco e conforme a preferência,
  // o core já mandou — aqui é só a superfície dentro do app. O título da
  // sessão vem de um ref para o listener não ser refeito a cada update.
  const alertSessionsRef = useRef(sessions);
  alertSessionsRef.current = sessions;
  useEffect(() => {
    const unlisten = onAgentAlert((payload) => {
      const title =
        alertSessionsRef.current.find((s) => s.id === payload.session_id)?.title ?? null;
      pushToast(agentAlertToast(payload, title, i18n.language, t));
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [t, i18n.language]);

  // F3: metade "recovery" do dismiss -- a mesma sessão que carrega a faixa
  // volta a `running` (o dono rodou `/login` e o agente seguiu). `sessions`
  // já chega atualizado por `onSessionStatus` (ver o efeito que popula esse
  // state), então este efeito só precisa reagir à mudança, nunca escutar
  // IPC por conta própria.
  useEffect(() => {
    if (runtimeAuthAlerts.size === 0) return;
    setRuntimeAuthAlerts((prev) => withoutRecoveredAuthAlerts(prev, sessions));
  }, [sessions, runtimeAuthAlerts.size]);

  // O core avisa quando a thread de boot termina. Só então o que veio vazio
  // vira estado — antes disso é transitório.
  //
  // > [!warning] O aviso pode não chegar. `listen()` do Tauri é assíncrono:
  // > entre pedir o registro e o listener existir de fato há uma janela, e o
  // > `app://ready` emitido dentro dela se perde — o core não reenvia. Por isso
  // > o poll ao lado do listener: ele não depende da entrega de nada. Os dois
  // > caminhos param juntos, porque marcar `bootReady` muda a dependência e
  // > desmonta este efeito; e se rodarem em paralelo por um instante, nenhum
  // > desfaz o outro — `mergeLoaded` não deixa o `ready` regredir nem aplica
  // > lista de resposta que não veio pronta.
  useEffect(() => {
    if (bootReady) return;
    let stopped = false;
    let timer: number | undefined;
    const startedAt = Date.now();

    const unlisten = onAppReady(() => {
      markReady();
      void refreshSessions();
      refreshLayout();
      // O evento normalmente ganha do primeiro tick, e `markReady` desmonta
      // este efeito — então a repesca tem de sair daqui também, senão ela só
      // funciona no caso raro em que o evento se perde.
      recoverPrefs();
    });

    const tick = async () => {
      // `boot_snapshot` e não `list_sessions`: os dois são `async` no core,
      // então nenhum roda na main thread do macOS — a que desenha o webview —,
      // mas só este traz o `bootFailure`. E o evento que carregaria essa
      // notícia sofre a mesma corrida de entrega que o `app://ready`, com o
      // agravante de a thread de boot começar dentro do `.setup()`, antes de o
      // webview existir. Sem o campo aqui, um pânico cedo não chegaria nunca.
      //
      // De quebra, uma pergunta em vez de duas: sessões e layout vêm juntos.
      // `boot_gate` e não `boot_snapshot`: os dois são `async` no core, mas o
      // snapshot completo roda `store.prefs()` a cada chamada, tomando o mesmo
      // mutex que a thread de boot usa. O poll estava disputando o lock com a
      // coisa que ele espera terminar — e descartando o resultado, porque o
      // `mergeLoaded` recusa payload que não veio pronto.
      const gate = await bootGate().catch(() => null);
      if (gate?.bootFailure) reportBootFailure(gate.bootFailure);
      const loaded = acceptLoaded(
        gate?.loaded ? { ready: gate.ready, value: gate.loaded } : null,
      );
      if (loaded) {
        setSessions(loaded.sessions);
        setLayout(loaded.layout);
        promptNewSessionIfEmpty(true, loaded.layout.workspaces.length);
        // As prefs não vêm no portão, e o mount pode não tê-las trazido — a
        // chamada dele rejeita inteira quando o `store.prefs()` falha.
        recoverPrefs();
      }
      // O `stopped` só é consultado no fim, de propósito: o próprio sucesso
      // deste tick sobe o `bootReady` e desmonta o efeito, e desistir por causa
      // disso deixaria layout e falha sem aplicar.
      schedule(loaded !== null);
    };
    const schedule = (ready: boolean) => {
      const delay = nextBootPoll({ ready, elapsedMs: Date.now() - startedAt });
      if (delay === null || stopped) return;
      timer = window.setTimeout(() => void tick(), delay);
    };
    schedule(false);

    return () => {
      stopped = true;
      window.clearTimeout(timer);
      void unlisten.then((off) => off());
    };
  }, [bootReady, markReady, refreshSessions, refreshLayout, recoverPrefs]);

  const sessionById = useMemo(
    () => new Map(sessions.map((s) => [s.id, s])),
    [sessions],
  );

  const [collapsedGroups, setCollapsedGroups] = useState<
    Record<string, boolean>
  >({});
  const [sshHosts, setSshHosts] = useState<Host[]>([]);
  const [hostGroups, setHostGroups] = useState<HostGroup[]>([]);
  const [hostPickerOpen, setHostPickerOpen] = useState(false);
  // Uma falha de conexão pode vir de Host criado depois da última leitura;
  // o cartão precisa dele para o "Editar host" e o `ssh-keygen -R`.
  const failedSshSessions = useMemo(
    () =>
      sessions
        .filter((s) => s.kind.type === "ssh" && s.connection === "failed")
        .map((s) => s.id)
        .sort()
        .join("\n"),
    [sessions],
  );
  const reloadHosts = useCallback(() => {
    void listHosts()
      .then(setSshHosts)
      .catch(() => {});
    void listHostGroups()
      .then(setHostGroups)
      .catch(() => {});
  }, []);
  useEffect(() => {
    reloadHosts();
  }, [reloadHosts, hostPickerOpen, failedSshSessions]);
  // A tela de conexões grava por conta própria; sem o evento, a cópia daqui
  // (subtítulo `user@host` da barra lateral) fica com o valor antigo.
  useEffect(() => {
    const unlisten = onHostsChanged(reloadHosts);
    return () => {
      void unlisten.then((off) => off());
    };
  }, [reloadHosts]);
  const [hostEditor, setHostEditor] = useState<HostDialogState>(null);
  const editSshHost = useCallback(
    (hostId: string) => {
      const known = sshHosts.find((h) => h.id === hostId);
      if (known) {
        setHostEditor({ mode: "edit", host: known });
        return;
      }
      void listHosts()
        .then((hosts) => {
          setSshHosts(hosts);
          const host = hosts.find((h) => h.id === hostId);
          if (host) setHostEditor({ mode: "edit", host });
        })
        .catch((e) =>
          toastError(t("connectionsActionFailed"), translateError(e, t)),
        );
    },
    [sshHosts, t],
  );
  const knownHostsCommandFor = useCallback(
    (hostId: string): string | null => {
      const host = sshHosts.find((h) => h.id === hostId);
      return host ? knownHostsRemoveCommand(host.hostname, host.port) : null;
    },
    [sshHosts],
  );

  const sideView = activeWorkspace?.side_view ?? null;
  const sideTarget = useMemo(
    () =>
      sideView?.startsWith("diff:")
        ? (sessionById.get(sideView.slice(5)) ?? null)
        : null,
    [sideView, sessionById],
  );
  const tunnelsTarget = useMemo(
    () =>
      sideView?.startsWith("tunnels:")
        ? (sessionById.get(sideView.slice(8)) ?? null)
        : null,
    [sideView, sessionById],
  );
  const filesTarget = useMemo(
    () =>
      sideView?.startsWith("files:")
        ? (sessionById.get(sideView.slice(6)) ?? null)
        : null,
    [sideView, sessionById],
  );
  const gitPanelTarget = useMemo(() => {
    const view = parseSideView(sideView);
    return view?.kind === "git" && view.session
      ? (sessionById.get(view.session) ?? null)
      : null;
  }, [sideView, sessionById]);
  const dockerPanelTarget = useMemo(() => {
    const view = parseSideView(sideView);
    return view?.kind === "docker" && view.session
      ? (sessionById.get(view.session) ?? null)
      : null;
  }, [sideView, sessionById]);
  const historyPanelTarget = useMemo(() => {
    const view = parseSideView(sideView);
    return view?.kind === "history" && view.session
      ? (sessionById.get(view.session) ?? null)
      : null;
  }, [sideView, sessionById]);
  // Docker numa sessão SSH fala com o servidor dela (alias no ssh_config).
  const dockerPanelHost = useMemo(() => {
    const kind = dockerPanelTarget?.kind;
    if (kind?.type !== "ssh") return null;
    return sshHosts.find((h) => h.id === kind.host_id)?.alias ?? null;
  }, [dockerPanelTarget, sshHosts]);
  const tunnelsHostAlias = useMemo(() => {
    const kind = tunnelsTarget?.kind;
    if (kind?.type !== "ssh") return "";
    return sshHosts.find((h) => h.id === kind.host_id)?.alias ?? "";
  }, [tunnelsTarget, sshHosts]);
  const sideExpanded = Boolean(sideView && activeWorkspace?.side_expanded);
  const sideRatio = activeWorkspace?.side_ratio ?? 0.5;

  const reducedMotion = useMemo(
    () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
    [],
  );
  // Tela cheia de Git: `git:<id>` no slot vira a tela por cima da área
  // principal, não um painel de 380px. Fica montada durante a saída.
  const gitMotion = usePresence(gitPanelTarget !== null, reducedMotion ? 0 : 260);
  const lastGitSessionRef = useRef<Session | null>(null);
  if (gitPanelTarget) lastGitSessionRef.current = gitPanelTarget;
  const gitScreenSession = gitPanelTarget ?? (gitMotion.mounted ? lastGitSessionRef.current : null);
  // Tela cheia de Arquivos: mesma entrada/saída da de Git.
  const filesMotion = usePresence(filesTarget !== null, reducedMotion ? 0 : 260);
  const lastFilesSessionRef = useRef<Session | null>(null);
  if (filesTarget) lastFilesSessionRef.current = filesTarget;
  const filesScreenSession =
    filesTarget ?? (filesMotion.mounted ? lastFilesSessionRef.current : null);
  const dockerMotion = usePresence(dockerPanelTarget !== null, reducedMotion ? 0 : 260);
  const lastDockerSessionRef = useRef<Session | null>(null);
  if (dockerPanelTarget) lastDockerSessionRef.current = dockerPanelTarget;
  const dockerScreenSession =
    dockerPanelTarget ?? (dockerMotion.mounted ? lastDockerSessionRef.current : null);
  const agentsSideActive = Boolean(sideView?.startsWith("agents:"));
  const agentsMotion = usePresence(agentsSideActive, reducedMotion ? 0 : 150);
  const lastAgentsSessionRef = useRef<SessionId | null>(null);
  useEffect(() => {
    if (sideView?.startsWith("agents:")) {
      lastAgentsSessionRef.current = sideView.slice(7);
    }
  }, [sideView]);

  // O painel Agentes continua montado durante o fade de saída (agentsMotion):
  // renderSideView guarda a sessão que estava aberta para desenhar o último
  // quadro enquanto ela sai, sem que a coluna principal reflua na hora.
  // A superfície de agentes é uma coluna estreita de largura fixa (A5.3): o
  // terminal fica com o resto. Expandida, ocupa tudo como as outras views.
  const { sideVisible, renderSideView, agentsSideNarrow, slot: mainSlot } =
    mainColumn(
      // Git, Arquivos e Docker são telas cheias por cima, não coluna lateral.
      sideSlotView(sideView),
      agentsMotion.mounted,
      lastAgentsSessionRef.current,
      sideExpanded,
      sideRatio,
    );
  const agentsTarget = useMemo(() => {
    const sid = renderSideView?.startsWith("agents:")
      ? renderSideView.slice(7)
      : null;
    return sid ? (sessionById.get(sid) ?? null) : null;
  }, [renderSideView, sessionById]);

  // Sessão dona do painel Agentes morreu (exited/failed) ou sumiu → nada vivo
  // pra ver, fecha o side view.
  const closingAgentsPanels = useRef<Set<string>>(new Set());
  const seenAgentSessions = useRef<Set<string>>(new Set());
  useEffect(() => {
    for (const s of sessions) seenAgentSessions.current.add(s.id);
    const dead = new Set(
      deadAgentsPanels(layout.workspaces, sessions, seenAgentSessions.current),
    );
    for (const id of [...closingAgentsPanels.current]) {
      if (!dead.has(id)) closingAgentsPanels.current.delete(id);
    }
    for (const wsId of dead) {
      if (closingAgentsPanels.current.has(wsId)) continue;
      closingAgentsPanels.current.add(wsId);
      void closeSideView(wsId).catch(() =>
        closingAgentsPanels.current.delete(wsId),
      );
    }
  }, [layout.workspaces, sessions]);

  // Rodada de subagentes concluiu (todos Done e, em sessão gerenciada, turno
  // encerrado) → o painel mostra a conclusão por um instante e fecha sozinho.
  // Só no flanco de subida: painel reaberto DEPOIS da conclusão fica aberto —
  // o usuário o abriu pra rever, não é lixo de rodada.
  const layoutRef = useRef(layout);
  layoutRef.current = layout;
  const panelRuns = useRef<Map<string, PanelRunEntry>>(new Map());
  const panelCloseTimers = useRef<Map<string, number>>(new Map());
  useEffect(() => {
    const cancelTimer = (wsId: string) => {
      const timer = panelCloseTimers.current.get(wsId);
      if (timer != null) {
        window.clearTimeout(timer);
        panelCloseTimers.current.delete(wsId);
      }
    };
    const open = new Set<string>();
    for (const ws of layout.workspaces) {
      const sessionId = agentsPanelSession(ws.side_view);
      if (!sessionId) continue;
      const session = sessionById.get(sessionId);
      if (!session) continue;
      open.add(ws.id);
      const concluded = agentsPanelRunConcluded(
        session.kind,
        session.status,
        subagentsBySession.get(sessionId)?.subagents ?? [],
      );
      const { entry, action } = trackPanelRun(
        panelRuns.current.get(ws.id),
        sessionId,
        concluded,
      );
      panelRuns.current.set(ws.id, entry);
      if (action === "cancel") cancelTimer(ws.id);
      if (action !== "schedule" || panelCloseTimers.current.has(ws.id)) {
        continue;
      }
      const timer = window.setTimeout(() => {
        panelCloseTimers.current.delete(ws.id);
        const current = layoutRef.current.workspaces.find(
          (w) => w.id === ws.id,
        );
        if (current && agentsPanelSession(current.side_view) === sessionId) {
          void closeSideView(ws.id).catch(() => {});
          // Viewer e painel são uma feature: o fim da rodada fecha os dois —
          // o split do viewer não pode ficar órfão na tela.
          void closeAgentViewers(sessionId).catch(() => {});
        }
      }, AGENTS_PANEL_LINGER_MS);
      panelCloseTimers.current.set(ws.id, timer);
    }
    for (const wsId of [...panelRuns.current.keys()]) {
      if (open.has(wsId)) continue;
      panelRuns.current.delete(wsId);
      cancelTimer(wsId);
    }
  }, [layout.workspaces, sessionById, subagentsBySession]);

  // `mainSlot`, não só `sideView`: o painel Agentes fecha com fade e só libera
  // a largura quando desmonta — ver `mainColumn`. O ResizeObserver do terminal
  // sozinho não bastou para pegar essa volta à largura cheia.
  useEffect(() => {
    requestTerminalRelayout();
  }, [sideView, sideRatio, sideExpanded, mainSlot]);

  const mainAreaRef = useRef<HTMLDivElement>(null);
  const sideDragThrottle = useRef(0);
  const startSideDrag = useCallback(
    (e: React.PointerEvent) => {
      const wsId: string = activeWorkspace?.id ?? "";
      if (!wsId) return;
      e.preventDefault();
      const bounds = mainAreaRef.current?.getBoundingClientRect();
      if (!bounds || bounds.width === 0) return;
      // O limite tem que ser em PIXEL, não em fração. Com 10%/90% um painel
      // numa janela de 1512px podia ir a ~151px — e a coluna da árvore é
      // `w-[240px] shrink-0`, que não encolhe: o painel ficava menor que o
      // próprio conteúdo e a árvore transbordava por baixo do terminal.
      // A fração só existe porque é o que o core persiste; o clamp acontece
      // antes da conversão.
      const compute = (ev: PointerEvent) => {
        const min = Math.min(SIDE_MIN_PX, bounds.width / 2);
        const max = Math.max(min, bounds.width - MAIN_MIN_PX);
        const px = Math.min(max, Math.max(min, bounds.right - ev.clientX));
        return px / bounds.width;
      };
      const up = (ev: PointerEvent) => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        void setSideViewRatio(wsId, compute(ev), true).catch(() => {});
      };
      function move(ev: PointerEvent) {
        if (ev.buttons === 0) {
          up(ev);
          return;
        }
        const now = Date.now();
        if (now - sideDragThrottle.current < 80) return;
        sideDragThrottle.current = now;
        void setSideViewRatio(wsId, compute(ev), false).catch(() => {});
      }
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    },
    [activeWorkspace],
  );

  const sessionIdsRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    sessionIdsRef.current = new Set(sessions.map((s) => s.id));
  }, [sessions]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void onSubagentsChanged((p) => {
      if (cancelled) return;
      setSubagentsBySession((prev) => {
        const next = new Map(prev);
        next.set(p.session_id, { focused: p.focused, subagents: p.subagents });
        return next;
      });
    }).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const live = new Set(sessions.map((s) => s.id));
    setSubagentsBySession((prev) => {
      let changed = false;
      const next = new Map(prev);
      for (const key of [...next.keys()]) {
        if (!live.has(key)) {
          next.delete(key);
          changed = true;
        }
      }
      return changed ? next : prev;
    });
    for (const s of sessions) {
      if (s.kind.type !== "agent") continue;
      void listSubagents(s.id)
        .then((snap) => {
          if (snap.subagents.length === 0) return;
          setSubagentsBySession((prev) =>
            prev.has(s.id) ? prev : new Map(prev).set(s.id, snap),
          );
        })
        .catch(() => {});
    }
  }, [sessions]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void onAgentDetected((p) => {
      if (cancelled) return;
      setDetectedBySession((prev) => {
        const state = withHostingState(p);
        if (state) return new Map(prev).set(p.session_id, state);
        if (!prev.has(p.session_id)) return prev;
        const next = new Map(prev);
        next.delete(p.session_id);
        return next;
      });
    }).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const probedDetection = useRef<Set<string>>(new Set());
  useEffect(() => {
    const live = new Set(sessions.map((s) => s.id));
    setDetectedBySession((prev) => {
      let changed = false;
      const next = new Map(prev);
      for (const key of [...next.keys()]) {
        if (!live.has(key)) {
          next.delete(key);
          changed = true;
        }
      }
      return changed ? next : prev;
    });
    setDismissedShellNotices((prev) => {
      let changed = false;
      const next = new Map(prev);
      for (const key of [...next.keys()]) {
        if (!live.has(key)) {
          next.delete(key);
          changed = true;
        }
      }
      return changed ? next : prev;
    });
    for (const id of [...probedDetection.current]) {
      if (!live.has(id)) probedDetection.current.delete(id);
    }
    for (const s of sessions) {
      if (s.kind.type !== "shell" || probedDetection.current.has(s.id)) continue;
      probedDetection.current.add(s.id);
      void detectedAgent(s.id)
        .then((p) => {
          const state = p ? withHostingState(p) : null;
          if (!state) return;
          setDetectedBySession((prev) =>
            prev.has(s.id) ? prev : new Map(prev).set(s.id, state),
          );
        })
        .catch(() => probedDetection.current.delete(s.id));
    }
  }, [sessions]);

  // Shim v2, Track C (tech-spec §9): toast único, na primeira transição
  // hosting=false -> true que o app vê. Não é gatilho de abertura de sessão
  // (não colide com a ADR 2026-08-22) — dispara de `detectedBySession`, que
  // só muda quando o poll do core detecta ou perde um agente.
  useEffect(() => {
    const anyHosting = [...detectedBySession.values()].some((d) => d.hosting);
    const alreadySeen =
      localStorage.getItem(HOSTING_INTRO_TOAST_SEEN_KEY) === "1";
    if (!shouldShowHostingIntroToast(alreadySeen, anyHosting)) return;
    localStorage.setItem(HOSTING_INTRO_TOAST_SEEN_KEY, "1");
    pushToast(hostingIntroToastInput(t));
  }, [detectedBySession, t]);

  const worktreeRepoRoots = useMemo(() => {
    const roots = new Set<string>();
    for (const w of layout.workspaces) {
      if (w.repo_root) roots.add(w.repo_root);
    }
    return [...roots];
  }, [layout.workspaces]);

  const goToSession = useCallback(
    (sessionId: SessionId) => {
      const location = findSessionLocation(layout.workspaces, sessionId);
      if (!location) return;
      void activateWorkspace(location.workspaceId);
      void activateTab(location.tabId);
    },
    [layout.workspaces],
  );

  /**
   * Salto do quadro: além do workspace e da aba, foca o painel.
   *
   * O `goToSession` acima para na aba, e numa aba dividida isso deixa o cursor
   * no painel errado — o quadro já sabe qual é o painel, então não há motivo
   * para chegar perto e não chegar.
   */
  const jumpToAgent = useCallback(
    (sessionId: SessionId, place: SessionPlace) => {
      goToSession(sessionId);
      void focusPane(place.paneId).catch(() => {});
    },
    [goToSession],
  );

  // "Agentes" no menu lateral: com a sessão ativa tendo subagentes, abre (ou
  // fecha) a árvore dela — é o que o dono espera achar ali, e é o caminho de
  // volta depois de fechar o painel. Sem subagentes, segue sendo a fila.
  const openAgentsFromSidebar = useCallback(() => {
    const hasSubagents =
      activeId != null &&
      (subagentsBySession.get(activeId)?.subagents.length ?? 0) > 0;
    if (!hasSubagents || !activeWorkspace) {
      void toggleAgentQueue().catch(() => {});
      return;
    }
    if (sideView === `agents:${activeId}`) {
      void closeSideView(activeWorkspace.id).catch(() => {});
    } else {
      void openAgentsPanel(activeId).catch(() => {});
    }
  }, [activeId, activeWorkspace, sideView, subagentsBySession]);

  // O `hosting` do shim v2 (tech-spec §7), por sessão — projeção de
  // `detectedBySession` porque `agentsBoard.buildRows`/`AgentRow` só
  // precisam do booleano, nunca do `pid`/`start_ms`/`jailed` que o resto do
  // App usa. É o que decide o selo "sem gate" no quadro, na fila e na barra
  // lateral (ver `agentsBoard.rowShowsNoGate`): hospedado já está dentro do
  // gate, mesmo continuando `kind.type === "shell"`.
  const hostingBySession = useMemo(() => {
    const map = new Map<SessionId, boolean>();
    for (const [id, d] of detectedBySession) map.set(id, d.hosting);
    return map;
  }, [detectedBySession]);

  // Conta as duas seções do quadro: o agente sem gate também pede alguém, e o

  // Uma lista só, consumida pelo contador e pela seção da barra lateral. Duas
  // derivações independentes do mesmo estado dariam contador e lista que
  // discordam — e é exatamente o número em cima da lista que denuncia.
  const agentRows = useMemo(
    () => boardOrder(buildAgentRows(sessions, layout, hostingBySession)),
    [sessions, layout, hostingBySession],
  );
  const agentsWaiting = useMemo(
    () => agentRows.filter(wantsAttention).length,
    [agentRows],
  );

  // Reaproveita agentRows — não recalcula buildAgentRows/boardOrder — pela
  // mesma razão do comentário acima: uma segunda derivação divergiria do
  // que a fila realmente mostra. A fila colapsa pra um pedido por sessão;
  // esconder todo pendente enquanto ela está aberta deixaria o segundo
  // pedido de uma sessão sem NENHUM ponto de ação visível.
  const agentQueueVisibleIds = useMemo(
    () => agentQueueVisibleApprovalIds(agentRows, approvals),
    [agentRows, approvals],
  );
  // Painel vermelho dentro da sessão: um pedido por sessão (o mais antigo).
  // Os que um painel NA TELA mostra saem do toast — acionável em um lugar só.
  const panelRequests = useMemo(
    () => panelRequestsBySession(approvals),
    [approvals],
  );
  const sessionPanelIds = useMemo(
    () =>
      panelVisibleApprovalIds(
        approvals,
        new Set(paneLayout?.panes.map((p) => p.session) ?? []),
      ),
    [approvals, paneLayout],
  );
  // Painel de agentes aberto: os pedidos que ele deixa aprovar na linha do
  // subagente (não-vermelhos) saem do toast. A mesma derivação que o painel
  // usa para desenhar — `subagentInlineApprovals` —, para os dois concordarem.
  const agentsPanelIds = useMemo(() => {
    const panelSession = agentsPanelSession(sideView);
    if (!panelSession) return new Set<number>();
    return agentsPanelActionableIds(
      subagentInlineApprovals(
        panelSession,
        subagentsBySession.get(panelSession)?.subagents ?? [],
        approvals,
      ),
    );
  }, [sideView, subagentsBySession, approvals]);
  const approvalHiddenIds = useMemo(
    () =>
      computeHiddenApprovalIds({
        approvals,
        notificationsOpen: inboxOpen,
        agentQueueOpen: sideView === "agent-queue",
        agentQueueVisibleIds,
        sessionPanelIds,
        agentsPanelIds,
      }),
    [
      approvals,
      inboxOpen,
      sideView,
      agentQueueVisibleIds,
      sessionPanelIds,
      agentsPanelIds,
    ],
  );

  const goToNextAttention = useCallback(() => {
    const next = nextAttention(
      agentRows,
      activeId,
    );
    if (!next) return;
    jumpToAgent(next.session.id, next.place);
  }, [agentRows, activeId, jumpToAgent]);

  const typeIntoSession = useCallback(
    async (sid: SessionId, text: string, submit: boolean) => {
      let lastError: unknown = null;
      for (let attempt = 0; attempt < 5; attempt += 1) {
        try {
          await submitRichInput(sid, text, submit);
          return;
        } catch (e) {
          lastError = e;
          await new Promise((r) => setTimeout(r, 500));
        }
      }
      throw lastError;
    },
    [],
  );

  // Sessão de AGENTE na pasta que já existe — nunca uma sessão de shell com o
  // binário digitado dentro. Digitar `claude` num shell sobe o agente sem
  // sandbox, com o env inteiro do usuário e, sobretudo, sem os hooks: sem
  // PreToolUse não há gate, e um agente fora do inbox faz o que quiser.
  const spawnAgentSession = useCallback(
    async (
      title: string,
      cwd: string,
      runner?: AgentRunner,
      openedByGate = false,
      resumeConversation?: string,
    ) => {
      const fresh = await createSession({
        kind: { type: "agent", runner: runner ?? runnerFromCommand(reviewAgent) },
        cwd,
        title,
        attach_existing: true,
        cols: 100,
        rows: 30,
        opened_by_gate: openedByGate,
        resume_conversation: resumeConversation,
      });
      setSessions((prev) => [...prev, fresh]);
      try {
        await createWorkspace(title, cwd, fresh.id);
      } catch {
        void disposeSession(fresh.id).catch(() => {});
        throw new Error("não deu pra abrir a sessão no worktree");
      }
      // O prompt é multilinha e o submit_rich_input recusa multilinha sem
      // bracketed paste — esperar o TUI ligar o modo (DECSET 2004) é o
      // sinal exato de "composer pronto"; cold start do agente estoura
      // qualquer sleep fixo.
      for (let attempt = 0; attempt < 30; attempt += 1) {
        const bracketed = await sessionBracketedPaste(fresh.id).catch(
          () => false,
        );
        if (bracketed) break;
        await new Promise((r) => setTimeout(r, 500));
      }
      return fresh.id;
    },
    [reviewAgent],
  );

  const sendReviewToAgent = useCallback(
    async (target: Session, prompt: string) => {
      const wtPath = target.worktree?.path;
      let sid = target.id;
      const running = sessions.find(
        (s) =>
          s.id === target.id && s.status.state !== "exited" && s.status.state !== "failed",
      );
      if (!running && wtPath) {
        sid = await spawnAgentSession(target.title, wtPath);
      }
      await typeIntoSession(sid, prompt, false);
      goToSession(sid);
    },
    [sessions, goToSession, spawnAgentSession, typeIntoSession],
  );

  const dismissShellAgentNotice = useCallback(
    (sessionId: SessionId) => {
      const detected = detectedBySession.get(sessionId);
      if (!detected) return;
      setDismissedShellNotices((prev) =>
        new Map(prev).set(sessionId, noticeKey(detected)),
      );
    },
    [detectedBySession],
  );

  // Entrega C, F3: a outra metade do "some" -- dispensa manual. A metade de
  // recovery (sessão volta a `running`) mora no `useEffect` que observa
  // `sessions` logo abaixo, porque só ele já escuta status de sessão.
  const dismissRuntimeAuthAlert = useCallback((sessionId: SessionId) => {
    setRuntimeAuthAlerts((prev) => withoutAuthAlert(prev, sessionId));
  }, []);

  const reopeningShellAgents = useRef<Set<SessionId>>(new Set());
  const reopenShellAgentManaged = useCallback(
    async (sessionId: SessionId) => {
      const detected = detectedBySession.get(sessionId);
      if (!detected || reopeningShellAgents.current.has(sessionId)) return;
      reopeningShellAgents.current.add(sessionId);
      try {
        const binary = agentBinaryName(detected.kind);
        const proceed = await requestConfirm({
          title: t("shellAgentReopenTitle"),
          detail: t("shellAgentReopenDetail", { binary }),
          confirmLabel: t("shellAgentReopen"),
          destructive: true,
        });
        if (!proceed) return;
        let cwd: string;
        let conversationId: string | null;
        try {
          ({ cwd, conversation_id: conversationId } =
            await killShellAgent(sessionId));
        } catch (e) {
          toastError(t("shellAgentReopenFailed"), translateError(e, t));
          return;
        }
        try {
          const title = cwd.split("/").filter(Boolean).pop() ?? binary;
          // A aba que o TYBA abre aqui é a que ele mesmo se propõe a fechar
          // de novo quando o agente sair depois de trabalhar — ver
          // `opened_by_gate` no core. Review e resolução de conflitos NÃO
          // marcam isto: só este fluxo reabre por trás de uma confirmação
          // explícita ("gate") e é dono da aba que criou.
          // Com o id, a sessão gerenciada retoma a MESMA conversa (M1 do
          // claude-monitor) em vez de nascer vazia e deixar a original órfã.
          const sid = await spawnAgentSession(
            title,
            cwd,
            detected.kind,
            true,
            conversationId ?? undefined,
          );
          goToSession(sid);
        } catch (e) {
          toastError(t("shellAgentReopenSpawnFailed"), translateError(e, t));
        }
      } finally {
        reopeningShellAgents.current.delete(sessionId);
      }
    },
    [detectedBySession, spawnAgentSession, goToSession, t],
  );

  // Pergunta ao core, uma vez por sessão candidata, se dá pra retomar a conversa
  // nativa daquele agente. Quem decide é ele: só o core sabe se o id foi
  // capturado, se o binário continua no PATH e se a pasta sobreviveu. Enquanto
  // não responder `true`, nada aparece — convite que abre em erro é pior que
  // convite nenhum.
  const askedResume = useRef<Set<SessionId>>(new Set());
  useEffect(() => {
    let alive = true;
    for (const id of resumeCandidates(sessions)) {
      if (askedResume.current.has(id)) continue;
      askedResume.current.add(id);
      void canResumeAgentSession(id)
        .then((ok) => {
          if (alive) setResumableAgents((prev) => new Map(prev).set(id, ok));
        })
        .catch(() => {
          // Falha na pergunta é "não": silêncio, nunca convite.
          if (alive) setResumableAgents((prev) => new Map(prev).set(id, false));
        });
    }
    return () => {
      alive = false;
    };
  }, [sessions]);

  const resumingAgents = useRef<Set<SessionId>>(new Set());
  const resumeAgentConversation = useCallback(
    async (sessionId: SessionId) => {
      if (resumingAgents.current.has(sessionId)) return;
      resumingAgents.current.add(sessionId);
      try {
        const fresh = await resumeAgentSession(sessionId);
        // A sessão volta viva com o MESMO id — é a mesma conversa, e o pane já
        // aponta para ela. Escrita direta, e não pelo `session://status`: aquele
        // listener recusa de propósito a volta de "encerrada" para "rodando".
        setSessions((prev) =>
          prev.map((s) => (s.id === fresh.id ? fresh : s)),
        );
        // O PTY é outro, então a assinatura de saída do anterior morreu com ele.
        // Trocar a `key` remonta o TerminalView, que reata e começa com a tela
        // limpa em vez de escrever o agente novo embaixo do "sessão encerrada".
        setPaneEpochs((prev) =>
          new Map(prev).set(sessionId, (prev.get(sessionId) ?? 0) + 1),
        );
        askedResume.current.delete(sessionId);
        setResumableAgents((prev) => {
          const next = new Map(prev);
          next.delete(sessionId);
          return next;
        });
        goToSession(sessionId);
      } catch (e) {
        toastError(t("agentResumeFailed"), translateError(e, t));
      } finally {
        resumingAgents.current.delete(sessionId);
      }
    },
    [goToSession, t],
  );

  const dismissResumeInvite = useCallback((sessionId: SessionId) => {
    setDismissedResumeInvites((prev) => new Set(prev).add(sessionId));
  }, []);

  // Shell restaurado: o convite e a retomada são do core (ele digita o
  // `claude --resume` no próprio shell). Aqui só a intenção e o erro.
  const resumeShellClaude = useCallback(
    async (sessionId: SessionId) => {
      try {
        await resumeShellConversation(sessionId);
        goToSession(sessionId);
      } catch (e) {
        toastError(t("agentResumeFailed"), translateError(e, t));
      }
    },
    [goToSession, t],
  );

  // Sessão com agente vivo recebe o prompt direto; sessão plain (ou morta)
  // ganha uma sessão de agente nova apontada pro repo conflitado.
  const resolveConflictsWithAgent = useCallback(
    async (target: Session, state: ConflictState) => {
      const prompt = buildConflictPrompt(state);
      const alive = sessions.find(
        (s) =>
          s.id === target.id && s.status.state !== "exited" && s.status.state !== "failed",
      );
      const cmd = sessionCommandsRef.current[target.id];
      let sid = target.id;
      if (!alive || !cmd?.running || !cmd.agent_match) {
        sid = await spawnAgentSession(target.title, state.root);
      }
      await typeIntoSession(sid, prompt, false);
      goToSession(sid);
    },
    [sessions, goToSession, spawnAgentSession, typeIntoSession],
  );

  const sessionIds = useMemo(
    () =>
      sessions
        .map((s) => s.id)
        .sort()
        .join("\n"),
    [sessions],
  );

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    for (const id of sessionIds.split("\n").filter(Boolean)) {
      void onSessionStatus(id, (session) => {
        setSessions((prev) => {
          const current = prev.find((c) => c.id === id);
          if (!current) return prev;
          const merged = mergeSessionUpdate(current, session);
          if (!merged) return prev;
          return prev.map((c) => (c.id === id ? merged : c));
        });
      }).then((un) => (disposed ? un() : unlisteners.push(un)));
    }
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, [sessionIds]);

  const [sessionCommands, setSessionCommands] = useState<
    Record<string, SessionCommand>
  >({});
  const sessionCommandsRef = useRef<Record<string, SessionCommand>>({});
  useEffect(() => {
    sessionCommandsRef.current = sessionCommands;
  }, [sessionCommands]);
  const [sessionCwds, setSessionCwds] = useState<Record<string, SessionCwd>>(
    {},
  );
  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    for (const id of sessionIds.split("\n").filter(Boolean)) {
      void onSessionCommand(id, (payload) => {
        // A limpeza do terminal ao vivo NÃO mora aqui.
        //
        // Ela morava, e chegava adiantada: este evento vem por um canal e os
        // bytes por outro, então limpava-se uma tela onde o eco do comando
        // ainda não tinha chegado — e ele aparecia logo depois. Agora o próprio
        // fluxo de bytes traz a limpeza, no lugar exato do `133;C`.
        setSessionCommands((prev) => ({ ...prev, [id]: payload }));
      }).then((un) => (disposed ? un() : unlisteners.push(un)));
      void onSessionCwd(id, (payload) =>
        setSessionCwds((prev) =>
          prev[id]?.cwd === payload.cwd &&
          prev[id]?.canonical === payload.canonical
            ? prev
            : { ...prev, [id]: payload },
        ),
      ).then((un) => (disposed ? un() : unlisteners.push(un)));
      void sessionCwd(id)
        .then((cwd) => {
          if (disposed || !cwd) return;
          setSessionCwds((prev) => (prev[id] ? prev : { ...prev, [id]: cwd }));
        })
        .catch(() => {});
    }
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, [sessionIds]);

  // --- SSH Session integrada: o plano da sessão e os chips do servidor ---

  /** O que o core decidiu para cada SSH Session (`ssh://integration`). */
  const [integrations, setIntegrations] = useState<Record<string, Integration>>(
    {},
  );
  /**
   * Os chips do servidor. `null` é resposta: não há canal (regra 25), e o chip
   * some em vez de ficar carregando para sempre.
   */
  const [remoteChips, setRemoteChips] = useState<
    Record<string, RemoteChips | null>
  >({});
  /**
   * Por onde cada sessão fala, dito pelo core (`session://transport`).
   *
   * Vive aqui, e não no pane: o anúncio sai uma vez, e um pane que ainda não
   * montou (sessão sem painel em aba nenhuma) perderia o evento e montaria
   * depois achando que o transporte é cru.
   */
  const [transports, setTransports] = useState<
    Record<string, SessionTransport>
  >({});
  /** O comando remoto que o core confirmou ser um agente (regra 26). */
  const [remoteAgents, setRemoteAgents] = useState<Record<string, string>>({});
  // O que já foi PERGUNTADO, que não é o que foi confirmado: sem isto, um
  // comando longo que não é agente seria reperguntado a cada evento de comando
  // de qualquer sessão.
  const remoteAgentAsked = useRef<Record<string, string>>({});

  /**
   * Guarda o plano só quando ele muda de fato.
   *
   * O puxão inicial responde por sessão, e um objeto novo a cada resposta
   * remontaria os ouvintes de bloco (que dependem deste mapa) uma vez por
   * sessão SSH aberta.
   */
  const rememberIntegration = useCallback(
    (id: SessionId, integration: Integration) =>
      setIntegrations((prev) => {
        const old = prev[id];
        if (
          old?.state === integration.state &&
          old.reason === integration.reason &&
          (old.detail ?? null) === (integration.detail ?? null)
        ) {
          return prev;
        }
        return { ...prev, [id]: integration };
      }),
    [],
  );

  const sshSessionIds = sessions
    .filter((s) => s.kind.type === "ssh")
    .map((s) => s.id)
    .join("\n");

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    for (const id of sshSessionIds.split("\n").filter(Boolean)) {
      void onSessionIntegration(id, (integration) =>
        rememberIntegration(id, integration),
      ).then((un) => (disposed ? un() : unlisteners.push(un)));
      // Puxão além da assinatura: o evento sai no spawn, antes deste listener
      // existir, e sem ele o pane ficaria sem a linha de explicação.
      void fetchSessionIntegration(id)
        .then((integration) => {
          if (disposed || !integration) return;
          rememberIntegration(id, integration);
        })
        .catch(() => {});
      void onSessionRemoteChips(id, (chips) =>
        setRemoteChips((prev) => ({ ...prev, [id]: chips })),
      ).then((un) => (disposed ? un() : unlisteners.push(un)));
      void sessionRemoteChips(id)
        .then((chips) => {
          if (disposed) return;
          setRemoteChips((prev) => (prev[id] ? prev : { ...prev, [id]: chips }));
        })
        .catch(() => {});
    }
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, [sshSessionIds, rememberIntegration]);

  const sessionIdsForTransport = sessions.map((s) => s.id).join("\n");

  /**
   * Por onde cada sessão fala.
   *
   * De todas as sessões, não só as SSH: quem decide o transporte é o core, e a
   * tela não deduz "é de controle" a partir do tipo da sessão nem da
   * integração. O que muda com isto é quem responde às consultas do terminal —
   * ver `silenceReply`.
   */
  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    for (const id of sessionIdsForTransport.split("\n").filter(Boolean)) {
      void onSessionTransport(id, (announced) =>
        setTransports((prev) => {
          const next = nextTransport(prev[id] ?? INITIAL_TRANSPORT, announced);
          return prev[id] === next ? prev : { ...prev, [id]: next };
        }),
      ).then((un) => (disposed ? un() : unlisteners.push(un)));
    }
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, [sessionIdsForTransport]);

  /**
   * Regra 26: quem diz que o comando remoto é um agente é o core.
   *
   * Uma pergunta por comando que COMEÇA numa sessão SSH — o matcher vive lá,
   * junto do resto do app, e o front não reimplementa a lista de agentes.
   */
  useEffect(() => {
    let disposed = false;
    for (const id of sshSessionIds.split("\n").filter(Boolean)) {
      const command = sessionCommands[id];
      if (!command?.running || !command.command) continue;
      const asked = command.command;
      if (remoteAgentAsked.current[id] === asked) continue;
      remoteAgentAsked.current[id] = asked;
      void remoteAgentWithoutJail(id, asked)
        .then((isAgent) => {
          if (disposed || !isAgent) return;
          setRemoteAgents((prev) =>
            prev[id] === asked ? prev : { ...prev, [id]: asked },
          );
        })
        .catch(() => {});
    }
    return () => {
      disposed = true;
    };
  }, [sshSessionIds, sessionCommands]);

  useEffect(() => {
    const live = new Set(sessions.map((s) => s.id));
    const prune = <T,>(prev: Record<string, T>): Record<string, T> => {
      const stale = Object.keys(prev).filter((id) => !live.has(id));
      if (stale.length === 0) return prev;
      const next = { ...prev };
      for (const id of stale) delete next[id];
      return next;
    };
    setSessionCommands(prune);
    setSessionCwds(prune);
    setIntegrations(prune);
    setRemoteChips(prune);
    setRemoteAgents(prune);
    for (const id of Object.keys(remoteAgentAsked.current)) {
      if (!live.has(id)) delete remoteAgentAsked.current[id];
    }
  }, [sessions]);

  const [activeGitStatus, setActiveGitStatus] = useState<
    SessionGitStatus | null | undefined
  >(undefined);
  // O cwd da sessão ativa chega por evento (onSessionCwd) — usar como
  // dependência dispara o snapshot na hora do `cd`, em vez de esperar o
  // próximo tick do poll de 4s. O poll continua cobrindo mudanças que não
  // trocam de diretório (commit, stage) sem evento próprio.
  const activeCwdKey = activeId
    ? (sessionCwds[activeId]?.canonical ?? sessionCwds[activeId]?.cwd ?? null)
    : null;
  // O mesmo, para qualquer sessão: cada painel tem a própria linha, e o cwd do
  // rodapé dela e o escopo do histórico são os da sessão DELA.
  const cwdKeyOf = (id: SessionId) =>
    sessionCwds[id]?.canonical ?? sessionCwds[id]?.cwd ?? null;
  const statusSessionRef = useRef<SessionId | null>(null);
  useEffect(() => {
    if (!activeId) {
      setActiveGitStatus(null);
      statusSessionRef.current = null;
      return;
    }
    if (statusSessionRef.current !== activeId) {
      setActiveGitStatus(undefined);
      statusSessionRef.current = activeId;
    }
    let cancelled = false;
    const id = activeId;
    const check = () => {
      void sessionGitStatus(id)
        .then((status) => {
          if (!cancelled) setActiveGitStatus(status);
        })
        .catch(() => {
          if (!cancelled) setActiveGitStatus(null);
        });
    };
    check();
    const timer = window.setInterval(check, 4000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeId, activeCwdKey]);
  const gitTone = gitIconTone(activeGitStatus);

  const activeSession = activeId ? sessionById.get(activeId) : undefined;

  // Abas de conteúdo. Editores moram na tela de Arquivos e ficam montados em
  // TODOS os workspaces enquanto a aba interna existe: o buffer não salvo é do
  // CodeMirror.
  const activeContent = parseContentView(activeTab?.view);
  const screenEditors = useMemo(
    () => allScreenEditors(layout.workspaces),
    [layout.workspaces],
  );
  // Faixa de contexto: aba de logs estruturados, ou o shell (PTY) de um
  // container local em foco.
  const containerStrip: { id: string; current: "logs" | "shell" } | null =
    activeContent?.kind === "container-logs"
      ? { id: activeContent.containerId, current: "logs" }
      : activeSession?.kind.type === "container" && !activeSession.kind.host_id
        ? { id: activeSession.kind.container_id, current: "shell" }
        : null;
  const [logsRestart, setLogsRestart] = useState<Record<string, number>>({});
  const activeCommand = activeId ? sessionCommands[activeId] : undefined;
  const agentsButtonVisible =
    activeSession != null &&
    showAgentsButton(
      activeSession.kind,
      activeId != null && detectedBySession.has(activeId),
    );

  // Painel lateral único: os ícones do header são as abas dele, e qual está
  // aberta é o `side_view` do workspace no core — aqui só se deriva o que o
  // header mostra (regras em `lib/sidePanel`, testadas). A sessão dos painéis
  // é `fileSessionId`: numa aba de editor/diff/logs não há pane em foco, e os
  // ícones seguem valendo para a sessão dona dela.
  const activeRepoSnapshot = activeGitStatus
    ? snapshotForDir(repoSnapshots, activeGitStatus.root)
    : undefined;
  const panelIndicatorInput = {
    dockerUp,
    dockerRunning,
    gitDirty: activeGitStatus?.dirty ?? false,
    pendingApprovals: activeId
      ? approvals.filter((a) => a.session_id === activeId).length
      : 0,
  };
  const panelTabs: PanelTab[] = visiblePanels({
    hasSession: fileSessionId != null,
    isSsh: activeSession?.kind.type === "ssh",
    showDocker: showContainers,
    hasRepo: gitTone != null,
    agentsAvailable: agentsButtonVisible,
  }).map((kind) => {
    const label = panelAriaLabel(
      kind,
      {
        ...panelIndicatorInput,
        gitChanged: activeRepoSnapshot?.status?.changed ?? null,
      },
      t,
    );
    return {
      kind,
      label,
      tooltip: label,
      shortcut: kind === "files" ? bindings.files : undefined,
      indicator: panelIndicator(kind, panelIndicatorInput),
    };
  });
  const onPanelClick = (kind: PanelKind) => {
    if (!fileSessionId || !activeWorkspace) return;
    if (panelClick(kind, sideView, fileSessionId) === "close") {
      void closeSideView(activeWorkspace.id).catch(() => {});
      return;
    }
    const open = {
      history: openHistoryPanel,
      docker: openDockerPanel,
      tunnels: openTunnelsPanel,
      files: openFilesPanel,
      git: openGitPanel,
      // O Diff abre a tela de Git na seção Revisar, não um painel lateral.
      diff: openDiffContentTab,
      agents: openAgentsPanel,
    }[kind];
    void open(fileSessionId).catch(() => {});
  };

  useEffect(() => {
    const markSeen = () => {
      if (!document.hasFocus() || !activeId) return;
      if (sessionById.get(activeId)?.attention) {
        void sessionMarkSeen(activeId).catch(() => {});
      }
    };
    markSeen();
    window.addEventListener("focus", markSeen);
    return () => window.removeEventListener("focus", markSeen);
  }, [activeId, sessionById]);
  const richInputEligible =
    activeSession != null &&
    shouldShowRichInput(
      activeSession.kind,
      activeCommand?.agent_match ?? false,
      richInputPref,
    );
  const richInputVisible =
    activeSession != null &&
    activeId != null &&
    richInputVisibility({
      kind: activeSession.kind,
      command: activeCommand,
      pref: richInputPref,
      opened: richInputOpened.has(activeId),
      dismissed: richInputDismissed.has(activeId),
    });

  const openRichInput = useCallback((id: SessionId) => {
    setRichInputDismissed((prev) => {
      if (!prev.has(id)) return prev;
      const next = new Set(prev);
      next.delete(id);
      return next;
    });
    setRichInputOpened((prev) => (prev.has(id) ? prev : new Set(prev).add(id)));
    setRichInputFocusNonce((n) => n + 1);
  }, []);

  const closeRichInput = useCallback(
    (id: SessionId) => {
      setRichInputOpened((prev) => {
        if (!prev.has(id)) return prev;
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
      dismissedCommand.current[id] = sessionCommands[id]?.command ?? null;
      setRichInputDismissed((prev) =>
        prev.has(id) ? prev : new Set(prev).add(id),
      );
      richInputFocused.current = false;
      getTerm(id)?.term.focus();
    },
    [sessionCommands],
  );

  useEffect(() => {
    const live = new Set(sessions.map((s) => s.id));
    const prune = (prev: Set<string>) => {
      if ([...prev].every((id) => live.has(id))) return prev;
      return new Set([...prev].filter((id) => live.has(id)));
    };
    setRichInputOpened(prune);
    setRichInputDismissed(prune);
  }, [sessions]);

  useEffect(() => {
    if (!activeId) return;
    const current = activeCommand?.command ?? null;
    if (
      richInputDismissed.has(activeId) &&
      dismissedCommand.current[activeId] !== current
    ) {
      setRichInputDismissed((prev) => {
        const next = new Set(prev);
        next.delete(activeId);
        return next;
      });
    }
  }, [activeId, activeCommand?.command, richInputDismissed]);

  useEffect(() => {
    if (!richInputPref.autoOpenOnStart || !activeId || !richInputEligible) {
      return;
    }
    if (richInputAutoOpened.current.has(activeId)) return;
    richInputAutoOpened.current.add(activeId);
    openRichInput(activeId);
  }, [activeId, richInputEligible, richInputPref.autoOpenOnStart, openRichInput]);


  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void fetchRepoSnapshots()
      .then((all) => {
        if (cancelled) return;
        setRepoSnapshots((prev) => {
          const next = { ...prev };
          for (const snap of all) {
            if (!next[snap.root]) next[snap.root] = snap;
          }
          return next;
        });
      })
      .catch(() => {});
    void onRepoChanged((snapshot) => {
      setRepoSnapshots((prev) => ({ ...prev, [snapshot.root]: snapshot }));
    }).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    let unlistenReconciled: (() => void) | null = null;
    void onRepoReconciled((all) => {
      setRepoSnapshots(Object.fromEntries(all.map((snap) => [snap.root, snap])));
    }).then((un) => {
      if (cancelled) un();
      else unlistenReconciled = un;
    });
    return () => {
      cancelled = true;
      unlisten?.();
      unlistenReconciled?.();
    };
  }, []);

  const workspaceAgent = useCallback(
    (w: Workspace): string | null => {
      for (const tab of w.tabs) {
        if (!tab.root) continue;
        for (const sid of leafSessions(tab.root)) {
          const session = sessionById.get(sid);
          const label = session && runnerLabel(session.kind);
          if (label) return label;
        }
      }
      return null;
    },
    [sessionById],
  );

  const workspaceCommand = useCallback(
    (w: Workspace): string | null => {
      for (const tab of w.tabs) {
        if (!tab.root) continue;
        for (const sid of leafSessions(tab.root)) {
          const c = sessionCommands[sid];
          if (c?.running && c.command) return c.command;
        }
      }
      return null;
    },
    [sessionCommands],
  );

  // A regra mora em `lib/agentsBoard`, testada: gerenciado nunca perde para
  // observado, e o visual do observado não herda o `status` da sessão de shell
  // — que está sempre `running` e pintaria de azul um agente parado.
  const workspaceAgentStatus = useCallback(
    (w: Workspace) => agentForWorkspace(w, sessionById),
    [sessionById],
  );

  const titleBase =
    activeWorkspace &&
    !isConfigWorkspace(activeWorkspace) &&
    !isWorktreesWorkspace(activeWorkspace) &&
    !isConnectionsWorkspace(activeWorkspace) &&
    !isStatsWorkspace(activeWorkspace) &&
    !isAgentBoardWorkspace(activeWorkspace)
      ? activeWorkspace.name
      : "Tyba";
  const activeAgentRunning = activeWorkspace
    ? workspaceAgentStatus(activeWorkspace)?.session.status.state === "running"
    : false;
  const unseenAttention = useMemo(
    () => sessions.some((s) => s.attention),
    [sessions],
  );

  useEffect(() => {
    const win = getCurrentWindow();
    const reducedMotion = window.matchMedia(
      "(prefers-reduced-motion: reduce)",
    ).matches;
    const apply = (frame: number) => {
      void win
        .setTitle(
          windowTitle({
            base: titleBase,
            running: activeAgentRunning,
            attention: unseenAttention,
            frame,
            reducedMotion,
          }),
        )
        .catch(() => {});
    };
    apply(0);
    if (!activeAgentRunning || reducedMotion) return;
    let frame = 0;
    const timer = window.setInterval(() => {
      frame = (frame + 1) % SPINNER_FRAMES.length;
      apply(frame);
    }, SPINNER_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [titleBase, activeAgentRunning, unseenAttention]);

  const workspaceCwd = useCallback(
    (w: Workspace): string | null => resolveWorkspaceCwd(w, sessionCwds)?.cwd ?? null,
    [sessionCwds],
  );

  const workspaceGitDir = useCallback(
    (w: Workspace): string | null => workspaceMatchDir(w, sessionCwds),
    [sessionCwds],
  );

  // Ver `toolbarBranchChip`: a decisão de rótulo/ação/"não sei" mora lá, pura e
  // testada. Aqui só se junta o estado que a tela tem.
  const toolbarBranch = useMemo(() => {
    return toolbarBranchChip({
      session: activeSession
        ? {
            id: activeSession.id,
            worktree: activeSession.worktree?.path ?? null,
            alive: !isFinishedStatus(activeSession.status),
            cwd: sessionCwds[activeSession.id]?.canonical ?? null,
          }
        : null,
      workspaceDir: activeWorkspace ? workspaceGitDir(activeWorkspace) : null,
      snapshots: repoSnapshots,
    });
  }, [
    activeSession,
    activeWorkspace,
    repoSnapshots,
    sessionCwds,
    workspaceGitDir,
  ]);

  // De onde os chips da barra vêm (regra 23). Numa SSH Session, do SERVIDOR —
  // a decisão mora em `toolbarChips`, testada; aqui só se junta o estado.
  const activeChips = useMemo(
    () =>
      toolbarChips({
        remote: activeSession?.kind.type === "ssh",
        chips: activeId ? remoteChips[activeId] : undefined,
        local: {
          cwd: activeWorkspace ? workspaceCwd(activeWorkspace) : null,
          branch: toolbarBranch,
          snapshot: (() => {
            const dir = activeWorkspace ? workspaceGitDir(activeWorkspace) : null;
            return dir ? snapshotForDir(repoSnapshots, dir) : undefined;
          })(),
        },
      }),
    [
      activeId,
      activeSession,
      activeWorkspace,
      remoteChips,
      repoSnapshots,
      toolbarBranch,
      workspaceCwd,
      workspaceGitDir,
    ],
  );

  // Sessão SSH roda o `ssh` localmente: o cwd do processo fica no home e nunca
  // reflete o `cd` do outro lado. Mostrar caminho local seria mentira — o que
  // localiza o usuário é o destino.
  // A sessão SSH é um contexto, não uma janela solta: o que nasce dentro dela
  // (split, tab, "+" do grupo) herda a conexão em vez de cair num shell local.
  const workspaceSshHostId = useCallback(
    (w: Workspace | null | undefined): string | null => {
      if (!w) return null;
      for (const tab of w.tabs) {
        if (!tab.root) continue;
        for (const sid of leafSessions(tab.root)) {
          const kind = sessionById.get(sid)?.kind;
          if (kind?.type === "ssh") return kind.host_id;
          // Container do host: o `sh` de dentro não fala OSC 133, então o
          // vínculo vem do próprio kind — sem ele o split cai no shell local.
          if (kind?.type === "container" && kind.host_id) return kind.host_id;
          // Shell que rodou `ssh` à mão não É uma SSH Session — ele ESTÁ numa.
          // Derivar do comando vivo dá o mesmo contexto e se desfaz sozinho no
          // exit, em vez de deixar a sessão mentindo que é remota.
          const cmd = sessionCommands[sid];
          if (cmd?.running) {
            const host = matchSshHost(cmd.command, sshHosts);
            if (host) return host.id;
          }
        }
      }
      return null;
    },
    [sessionById, sessionCommands, sshHosts],
  );

  const workspaceSshHost = useCallback(
    (w: Workspace): string | null => {
      const id = workspaceSshHostId(w);
      if (!id) return null;
      const host = sshHosts.find((h) => h.id === id);
      if (!host) return null;
      return host.username ? `${host.username}@${host.hostname}` : host.hostname;
    },
    [workspaceSshHostId, sshHosts],
  );

  /**
   * A SSH Session que a linha da barra lateral descreve, quando há uma.
   *
   * A linha fala da aba ativa — é de lá que `resolveWorkspaceCwd` tira o cwd, e
   * portanto de lá que sairia o snapshot local. Achar uma SSH Session ali basta
   * para a fonte deixar de ser a máquina local (regra 23): errar para "remota"
   * só esconde um chip, e errar para "local" mostra a branch de outro
   * computador.
   *
   * `ssh` digitado à mão num shell local não entra (regra 27): aquela sessão É
   * local, e o repositório que ela mostra é mesmo o de cá — por isso a pergunta
   * é pelo `kind`, e não pelo Host como em `workspaceSshHostId`.
   */
  const workspaceRemoteSessionId = useCallback(
    (w: Workspace): SessionId | null => {
      const tab = w.tabs.find((tb) => tb.id === w.active_tab) ?? w.tabs[0];
      if (!tab?.root) return null;
      const focused = tab.active_pane
        ? paneSession(tab.root, tab.active_pane)
        : null;
      if (focused && sessionById.get(focused)?.kind.type === "ssh") {
        return focused;
      }
      for (const sid of leafSessions(tab.root)) {
        if (sessionById.get(sid)?.kind.type === "ssh") return sid;
      }
      return null;
    },
    [sessionById],
  );

  const detailsFor = useCallback(
    (id: string): boolean => (detailOverrides[id] ?? detailsPref) === "on",
    [detailOverrides, detailsPref],
  );

  const groupNames = useMemo(() => {
    const names = new Set<string>();
    for (const w of layout.workspaces) {
      if (w.group) names.add(w.group);
    }
    return [...names].sort((a, b) => a.localeCompare(b));
  }, [layout.workspaces]);

  const groupedWorkspaces = useMemo(() => {
    const groups = new Map<string, Workspace[]>();
    const loose: Workspace[] = [];
    for (const w of workspaces) {
      if (w.group) {
        groups.set(w.group, [...(groups.get(w.group) ?? []), w]);
      } else {
        loose.push(w);
      }
    }
    return {
      groups: [...groups.entries()].sort(([a], [b]) => a.localeCompare(b)),
      loose,
    };
  }, [workspaces]);

  const cycleWorkspace = useCallback(
    (dir: 1 | -1) => {
      const list = layout.workspaces;
      if (list.length === 0) return;
      const idx = list.findIndex((w) => w.id === layout.active_workspace);
      const next = list[(idx + dir + list.length) % list.length];
      if (next) void activateWorkspace(next.id);
    },
    [layout],
  );

  const connectGroup = useCallback(
    async (group: HostGroup | null, hosts: Host[]) => {
      if (hosts.length === 0) return;
      const opened = await connectHostGroup(
        hosts.map((h) => h.id),
        group?.name ?? hosts[0].alias,
        group?.color ?? null,
        group?.name ?? null,
      );
      setSessions((prev) => [...prev, ...opened]);
    },
    [],
  );

  // --- Broadcast: a rajada para as SSH Sessions vivas do Broadcast Set ---
  const [broadcastOn, setBroadcastOn] = useState(false);
  const [broadcastSet, setBroadcastSet] = useState<SessionId[]>([]);
  const [broadcastAsk, setBroadcastAsk] = useState<string | null>(null);
  // A linha digitada, para o core poder classificar no Enter. É reconstrução:
  // seta/tab/histórico não passam por aqui, então serve ao gate, não à verdade
  // do terminal — o que executa continua sendo o que está no prompt.
  const broadcastLine = useRef("");

  // Só os panes do workspace ativo: rajada em sessão que você não está vendo é
  // comando disparado no escuro — metade do valor do broadcast é conferir as
  // saídas lado a lado.
  const broadcastTargets = useMemo<BroadcastTarget[]>(() => {
    if (!activeTab?.root) return [];
    const out: BroadcastTarget[] = [];
    for (const sid of leafSessions(activeTab.root)) {
      const s = sessionById.get(sid);
      if (!s) continue;
      const kind = s.kind;
      if (kind.type !== "ssh") continue;
      if (isFinishedStatus(s.status)) continue;
      if (s.connection && s.connection !== "live") continue;
      const host = sshHosts.find((h) => h.id === kind.host_id);
      if (!host) continue;
      out.push({ sessionId: s.id, alias: host.alias, color: host.color });
    }
    return out;
  }, [activeTab, sessionById, sshHosts]);

  // Semeado pelo grupo, refinado pelo usuário: alvo que some (sessão morreu)
  // sai sozinho, senão a rajada miraria um pane fantasma.
  useEffect(() => {
    const live = new Set(broadcastTargets.map((t) => t.sessionId));
    setBroadcastSet((prev) => {
      const kept = prev.filter((id) => live.has(id));
      if (kept.length > 0) return kept.length === prev.length ? prev : kept;
      return broadcastTargets.map((t) => t.sessionId);
    });
  }, [broadcastTargets]);

  const submitBroadcast = useCallback(
    async (confirmed: boolean) => {
      const line = broadcastLine.current;
      const verdict = await broadcastSubmit(broadcastSet, line, confirmed);
      if (verdict.outcome === "needs_confirmation") {
        setBroadcastAsk(verdict.command);
        return;
      }
      broadcastLine.current = "";
      setBroadcastAsk(null);
    },
    [broadcastSet],
  );

  const handleBroadcastInput = useCallback(
    (data: string): boolean => {
      if (!broadcastOn || broadcastSet.length === 0) return false;
      if (data === "\r" || data === "\n") {
        void submitBroadcast(false).catch(() => {});
        return true;
      }
      if (data === "") {
        broadcastLine.current = broadcastLine.current.slice(0, -1);
      } else if (data === "") {
        broadcastLine.current = "";
      } else if (!data.startsWith("")) {
        broadcastLine.current += data;
      }
      void broadcastWrite(broadcastSet, data).catch((e) => {
        console.error("broadcast", e);
      });
      return true;
    },
    [broadcastOn, broadcastSet, submitBroadcast],
  );

  const splitActive = useCallback(
    async (kind: SplitKind) => {
      if (!activeWorkspace || !activeTab?.active_pane) return;
      const hostId = workspaceSshHostId(activeWorkspace);
      const session = await createSession({
        kind: hostId ? { type: "ssh", host_id: hostId } : { type: "shell" },
        cwd: hostId ? undefined : (activeWorkspace.repo_root ?? undefined),
        cols: 80,
        rows: 24,
      });
      setSessions((prev) => [...prev, session]);
      try {
        await splitPane(activeTab.active_pane as string, kind, session.id);
      } catch {
        void disposeSession(session.id).catch(() => {});
      }
    },
    [activeWorkspace, activeTab, workspaceSshHostId],
  );

  const cyclePane = useCallback(() => {
    if (!activeTab || !paneLayout || paneLayout.panes.length < 2) return;
    const idx = paneLayout.panes.findIndex(
      (p) => p.pane === activeTab.active_pane,
    );
    const next = paneLayout.panes[(idx + 1) % paneLayout.panes.length];
    if (next) void focusPane(next.pane);
  }, [activeTab, paneLayout]);

  const cycleTab = useCallback(
    (dir: 1 | -1) => {
      if (!activeWorkspace || activeWorkspace.tabs.length < 2) return;
      const idx = activeWorkspace.tabs.findIndex(
        (tab) => tab.id === activeWorkspace.active_tab,
      );
      const next =
        activeWorkspace.tabs[
          (idx + dir + activeWorkspace.tabs.length) %
            activeWorkspace.tabs.length
        ];
      if (next) void activateTab(next.id);
    },
    [activeWorkspace],
  );

  const focusPaneInDirection = useCallback(
    (dir: "left" | "right" | "up" | "down") => {
      if (!activeTab || !paneLayout || paneLayout.panes.length < 2) return;
      const current = paneLayout.panes.find(
        (p) => p.pane === activeTab.active_pane,
      );
      if (!current) return;
      const cx = current.x + current.w / 2;
      const cy = current.y + current.h / 2;
      let best: (typeof paneLayout.panes)[number] | null = null;
      let bestScore = Infinity;
      for (const p of paneLayout.panes) {
        if (p.pane === current.pane) continue;
        const dx = p.x + p.w / 2 - cx;
        const dy = p.y + p.h / 2 - cy;
        const forward =
          dir === "left" ? -dx : dir === "right" ? dx : dir === "up" ? -dy : dy;
        if (forward <= 0.5) continue;
        const lateral = dir === "left" || dir === "right" ? dy : dx;
        const score = forward + Math.abs(lateral) * 2;
        if (score < bestScore) {
          bestScore = score;
          best = p;
        }
      }
      if (best) void focusPane(best.pane);
    },
    [activeTab, paneLayout],
  );

  const resizeActivePane = useCallback(
    (kind: SplitKind, delta: number) => {
      if (!activeTab?.root || !activeTab.active_pane) return;
      const split = findAncestorSplit(
        activeTab.root,
        activeTab.active_pane,
        kind,
      );
      if (split) void setSplitRatio(split.id, split.ratio + delta);
    },
    [activeTab],
  );

  const paneAreaRef = useRef<HTMLDivElement>(null);
  const dragThrottle = useRef(0);

  const startDividerDrag = useCallback(
    (divider: DividerRect) => (e: React.PointerEvent) => {
      e.preventDefault();
      const area = paneAreaRef.current;
      if (!area) return;
      const bounds = area.getBoundingClientRect();
      const compute = (ev: PointerEvent) => {
        const pct =
          divider.kind === "v"
            ? ((ev.clientX - bounds.left) / bounds.width) * 100
            : ((ev.clientY - bounds.top) / bounds.height) * 100;
        return (pct - divider.start) / divider.length;
      };
      const move = (ev: PointerEvent) => {
        const now = Date.now();
        if (now - dragThrottle.current < 80) return;
        dragThrottle.current = now;
        void setSplitRatio(divider.split, compute(ev), false).catch(() => {});
      };
      const up = (ev: PointerEvent) => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        void setSplitRatio(divider.split, compute(ev), true).catch(() => {});
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    },
    [],
  );

  const newSession = useCallback(
    async (
      cwd: string | null,
      name: string,
      group?: string | null,
      worktreeTask?: string,
      shell?: string,
    ) => {
      const session = await createSession({
        kind: { type: "shell" },
        cwd: cwd ?? undefined,
        cols: 100,
        rows: 30,
        worktree_task: worktreeTask,
        shell,
      });
      setSessions((prev) => [...prev, session]);
      try {
        const workspaceId = await createWorkspace(
          name,
          session.worktree?.path ?? cwd,
          session.id,
        );
        if (group) await setWorkspaceGroup(workspaceId, group);
      } catch {
        void disposeSession(session.id).catch(() => {});
      }
    },
    [],
  );

  // Só a janela entre o clique e a resposta do core: dali em diante quem diz
  // "conectando" é o `connection` da sessão. Quem impede a segunda sessão é o
  // core (`connect_host`), não este estado.
  const [connectRequests, setConnectRequests] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const connectingHosts = useMemo(() => {
    const ids = connectingHostIds(sessions);
    for (const id of connectRequests) ids.add(id);
    return ids;
  }, [sessions, connectRequests]);

  const connectToHost = useCallback(async (host: Host) => {
    setConnectRequests((prev) => new Set(prev).add(host.id));
    try {
      const result = await connectHost(host.id);
      if (result.type === "opened") {
        const { session } = result;
        setSessions((prev) =>
          prev.some((s) => s.id === session.id) ? prev : [...prev, session],
        );
      }
    } catch (e) {
      console.error("connect_host", e);
    } finally {
      setConnectRequests((prev) => {
        const next = new Set(prev);
        next.delete(host.id);
        return next;
      });
    }
  }, []);

  const newAgentSession = useCallback(
    async (
      cwd: string,
      name: string,
      group: string | null | undefined,
      prompt: string,
      runner: AgentRunnerId = "claude_code",
    ) => {
      const readyEarly = new Set<string>();
      let onEarlyReady: ((id: string) => void) | null = null;
      let unlistenEarly: (() => void) | null = null;
      let earlyDisposed = false;
      void onAnyAgentReady((id) => {
        readyEarly.add(id);
        onEarlyReady?.(id);
      }).then((un) => {
        if (earlyDisposed) un();
        else unlistenEarly = un;
      });
      const disposeEarly = () => {
        earlyDisposed = true;
        unlistenEarly?.();
        unlistenEarly = null;
      };

      let session: Session;
      try {
        session = await createSession(
          buildAgentSessionOpts({ cwd, task: name, runner }),
        );
      } catch (e) {
        disposeEarly();
        throw e;
      }
      setSessions((prev) => [...prev, session]);
      try {
        const workspaceId = await createWorkspace(
          name,
          session.worktree?.path ?? cwd,
          session.id,
        );
        if (group) await setWorkspaceGroup(workspaceId, group);
      } catch {
        disposeEarly();
        void disposeSession(session.id).catch(() => {});
        return;
      }
      const trimmedPrompt = prompt.trim();
      if (!trimmedPrompt) {
        disposeEarly();
        return;
      }
      const cancel = scheduleAgentReadyPrompt({
        onReady: (handler) => {
          if (readyEarly.has(session.id)) handler();
          else
            onEarlyReady = (id) => {
              if (id === session.id) handler();
            };
          return disposeEarly;
        },
        paste: (submit) => {
          agentReadyCancels.current.delete(session.id);
          void typeIntoSession(session.id, trimmedPrompt, submit).catch(
            () => {},
          );
        },
        onTimeout: () => {
          agentReadyCancels.current.delete(session.id);
          setAgentReadyWarnings((prev) => ({ ...prev, [session.id]: true }));
        },
        setTimeout: (cb, ms) => window.setTimeout(cb, ms),
        clearTimeout: (h) => window.clearTimeout(h),
      });
      agentReadyCancels.current.set(session.id, () => {
        cancel();
        disposeEarly();
      });
    },
    [typeIntoSession],
  );

  useEffect(() => {
    for (const s of sessions) {
      if (!isFinishedStatus(s.status)) continue;
      const cancel = agentReadyCancels.current.get(s.id);
      if (cancel) {
        cancel();
        agentReadyCancels.current.delete(s.id);
      }
    }
  }, [sessions]);

  // Grupo de conexões é um grupo de SSH: o "+" abre outra sessão no mesmo host,
  // em vez de perguntar por uma pasta da máquina local.
  const groupSshHostId = useCallback(
    (group: string): string | null => {
      for (const w of layout.workspaces) {
        if (w.group !== group) continue;
        const hostId = workspaceSshHostId(w);
        if (hostId) return hostId;
      }
      return null;
    },
    [layout.workspaces, workspaceSshHostId],
  );

  const newSessionInGroup = useCallback(
    (group: string) => {
      // Grupo de conexões: o "+" pergunta qual host — o SSH abrange todas as
      // conexões, não só a última daquele grupo.
      if (groupSshHostId(group)) {
        setHostPickerOpen(true);
        return;
      }
      setPendingGroup(group);
      openNewSession();
    },
    [openNewSession, groupSshHostId],
  );

  const newTab = useCallback(async () => {
    if (!activeWorkspace) {
      openNewSession();
      return;
    }
    const hostId = workspaceSshHostId(activeWorkspace);
    const session = await createSession({
      kind: hostId ? { type: "ssh", host_id: hostId } : { type: "shell" },
      cwd: hostId ? undefined : (activeWorkspace.repo_root ?? undefined),
      cols: 100,
      rows: 30,
    });
    setSessions((prev) => [...prev, session]);
    try {
      await createTab(session.id, activeWorkspace.id);
    } catch {
      void disposeSession(session.id).catch(() => {});
    }
  }, [activeWorkspace, openNewSession, workspaceSshHostId]);

  // Sessão aberta pelo core para um PR ("Abrir no TYBA"): se já existia e
  // está numa aba, só vai até ela; senão ganha uma aba no workspace ativo. Nos
  // dois casos a tela de Git fecha, para o shell do PR aparecer. Falha ao criar
  // a aba sobe para quem chamou (o detalhe do PR mostra o erro); a sessão só é
  // descartada se acabou de nascer.
  const adoptOpenedSession = useCallback(
    async (opened: Session) => {
      if (!activeWorkspace) return;
      const known = sessionById.has(opened.id);
      if (known && findSessionLocation(layout.workspaces, opened.id)) {
        goToSession(opened.id);
      } else {
        if (!known) setSessions((prev) => [...prev, opened]);
        try {
          await createTab(opened.id, activeWorkspace.id);
        } catch (e) {
          if (!known) void disposeSession(opened.id).catch(() => {});
          throw e;
        }
      }
      void closeSideView(activeWorkspace.id).catch(() => {});
    },
    [activeWorkspace, sessionById, goToSession, layout.workspaces],
  );

  const runInTerminal = useCallback(
    async (command: string) => {
      if (!activeWorkspace) {
        openNewSession();
        return;
      }
      const session = await createSession({
        kind: { type: "shell" },
        cwd: activeWorkspace.repo_root ?? undefined,
        cols: 100,
        rows: 30,
      });
      setSessions((prev) => [...prev, session]);
      try {
        await createTab(session.id, activeWorkspace.id);
        window.setTimeout(() => {
          void writeToSession(session.id, `${command}\n`).catch(() => {});
        }, 400);
      } catch {
        void disposeSession(session.id).catch(() => {});
      }
    },
    [activeWorkspace, openNewSession],
  );

  const openProjectFolder = useCallback(async () => {
    const dir = await openFileDialog({ directory: true, multiple: false });
    if (typeof dir !== "string") return;
    void setPref("pref.last_session_dir", dir).catch(() => {});
    await newSession(dir, basename(dir));
  }, [newSession]);

  const confirmCloseWithRunningAgent = useCallback(
    async (running: Session | null): Promise<boolean> => {
      if (!running) return true;
      const proceed = await requestConfirm({
        title: t("closeAgentRunningTitle"),
        detail: t("closeAgentRunningDetail", { title: running.title }),
        confirmLabel: t("closeAnyway"),
        cancelLabel: t("showRunning"),
        destructive: true,
      });
      if (!proceed) goToSession(running.id);
      return proceed;
    },
    [t, goToSession],
  );

  const killWorkspace = useCallback(
    async (id: string) => {
      const ws = layout.workspaces.find((w) => w.id === id);
      const running = ws ? workspaceRunningAgent(ws, sessionById) : null;
      if (!(await confirmCloseWithRunningAgent(running))) return;
      await closeWorkspace(id);
      await refreshSessions();
    },
    [layout.workspaces, sessionById, confirmCloseWithRunningAgent, refreshSessions],
  );

  /** Editores com alteração não salva que fechar esta aba levaria junto: o
   *  próprio editor, ou os editores das sessões do terminal que fecha. */
  const confirmCloseDirtyEditors = useCallback(
    async (tab: Tab): Promise<boolean> => {
      const views = tab.root
        ? dirtyEditorsOf(leafSessions(tab.root))
        : isEditorDirty(tab.view)
          ? [tab.view as string]
          : [];
      if (views.length === 0) return true;
      const parsed = parseContentView(views[0]);
      const name = parsed?.kind === "editor" ? baseName(parsed.rel) : "";
      const ok = await requestConfirm({
        title: t("editorUnsavedTitle", { name }),
        detail: t("editorUnsavedDetail"),
        confirmLabel: t("editorUnsavedConfirm"),
        destructive: true,
      });
      if (ok) for (const view of views) setEditorDirty(view, false);
      return ok;
    },
    [t],
  );

  const closeActivePane = useCallback(async () => {
    // Tela cheia aberta: o pane fica escondido por baixo e não é o alvo.
    const screen = screenOf(sideView);
    if (screen && activeWorkspace) {
      const file =
        screen.kind === "files" ? activeFile(activeWorkspace.screens.files, screen.session) : null;
      if (closeTarget(screen.kind, file) === "file" && file) {
        await closeScreenFile(activeWorkspace.id, file);
      } else {
        await closeSideView(activeWorkspace.id).catch(() => {});
      }
      return;
    }
    if (!activeTab) return;
    if (!activeTab.active_pane && !(await confirmCloseDirtyEditors(activeTab))) return;
    if (activeTab.active_pane) {
      const running = activeTab.root
        ? paneRunningAgent(activeTab.root, activeTab.active_pane, sessionById)
        : null;
      if (!(await confirmCloseWithRunningAgent(running))) return;
      await closePane(activeTab.active_pane);
    } else {
      const running = tabRunningAgent(activeTab, sessionById);
      if (!(await confirmCloseWithRunningAgent(running))) return;
      await closeTab(activeTab.id);
    }
    await refreshSessions();
  }, [
    sideView,
    activeWorkspace,
    activeTab,
    sessionById,
    confirmCloseWithRunningAgent,
    confirmCloseDirtyEditors,
    refreshSessions,
  ]);

  const closeTabAndRefresh = useCallback(
    async (id: string) => {
      const tab = layout.workspaces
        .flatMap((w) => w.tabs)
        .find((tt) => tt.id === id);
      const running = tab ? tabRunningAgent(tab, sessionById) : null;
      if (!(await confirmCloseWithRunningAgent(running))) return;
      if (tab && !(await confirmCloseDirtyEditors(tab))) return;
      await closeTab(id);
      await refreshSessions();
    },
    [
      layout.workspaces,
      sessionById,
      confirmCloseWithRunningAgent,
      confirmCloseDirtyEditors,
      refreshSessions,
    ],
  );

  const toggleSettings = useCallback(() => {
    if (activeTab?.view === "settings") {
      void closeTabAndRefresh(activeTab.id);
    } else {
      void openViewTab("settings").catch(() => {});
    }
  }, [activeTab, closeTabAndRefresh]);

  const changeTogglePref = useCallback((value: SidebarTogglePref) => {
    setTogglePref(value);
    setSidebar((current) => (current === "open" ? current : value));
    void setPref(TOGGLE_PREF_KEY, value).catch(() => {});
  }, []);

  const changeDetailsPref = useCallback((value: DetailsPref) => {
    setDetailsPref(value);
    void setPref(DETAILS_PREF_KEY, value).catch(() => {});
  }, []);

  const toggleWorkspaceDetails = useCallback(
    (id: string) => {
      setDetailOverrides((prev) => {
        const globalOn = detailsPref === "on";
        const current = prev[id] ?? detailsPref;
        const next = { ...prev, [id]: current === "on" ? "off" : ("on" as DetailsPref) };
        if ((next[id] === "on") === globalOn) delete next[id];
        void setPref(DETAILS_OVERRIDES_KEY, JSON.stringify(next)).catch(
          () => {},
        );
        return next;
      });
    },
    [detailsPref],
  );

  const changeAccountName = useCallback((value: string) => {
    setAccountName(value);
    void setPref(ACCOUNT_NAME_KEY, value).catch(() => {});
  }, []);

  const changeBindings = useCallback((value: Bindings) => {
    setBindings(value);
    void setPref(BINDINGS_PREF_KEY, JSON.stringify(value)).catch(() => {});
  }, []);

  const changeShowContainers = useCallback((value: boolean) => {
    setShowContainers(value);
    void setPref(SHOW_CONTAINERS_KEY, value ? "on" : "off").catch(() => {});
  }, []);

  const changeShowGitStatus = useCallback((value: boolean) => {
    setShowGitStatus(value);
    void setPref(GIT_STATUS_KEY, value ? "on" : "off").catch(() => {});
  }, []);

  const changeWorktreeDefault = useCallback((value: boolean) => {
    setWorktreeDefault(value);
    void setPref(WORKTREE_DEFAULT_KEY, value ? "on" : "off").catch(() => {});
  }, []);

  const changeToolbarPref = useCallback((next: ToolbarPref) => {
    setToolbarPref(next);
    void setPref(TOOLBAR_PREF_KEY, JSON.stringify(next)).catch(() => {});
  }, []);

  const changeRichInputPref = useCallback(
    async (next: RichInputPref) => {
      if (next.agentRegex !== richInputPref.agentRegex) {
        const accepted = await setAgentMatchPattern(next.agentRegex).catch(
          () => false,
        );
        if (!accepted) {
          setRichInputRegexInvalid(true);
          return;
        }
      }
      setRichInputRegexInvalid(false);
      setRichInputPref(next);
      void setPref(RICH_INPUT_PREF_KEY, JSON.stringify(next)).catch(() => {});
    },
    [richInputPref.agentRegex],
  );

  const changeEditor = useCallback((value: string) => {
    setEditorPref(value);
    void setPref(EDITOR_PREF_KEY, value).catch(() => {});
  }, []);

  const changeReviewAgent = useCallback((value: string) => {
    setReviewAgent(value);
    void setPref(REVIEW_AGENT_KEY, value).catch(() => {});
  }, []);

  const changeShellIntegration = useCallback((value: boolean) => {
    setShellIntegration(value);
    void setPref(SHELL_INTEGRATION_KEY, value ? "on" : "off").catch(() => {});
  }, []);

  // Vale no próximo boot: quem lê a pref é o core, no startup.
  const changeStartup = useCallback((value: StartupMode) => {
    setStartup(value);
    void setPref(STARTUP_KEY, value).catch(() => {});
  }, []);

  const toggleSidebar = useCallback(() => {
    setSidebar((current) => (current === "open" ? togglePref : "open"));
  }, [togglePref]);

  useEffect(() => {
    requestTerminalRelayout();
  }, [sidebar]);

  useEffect(() => {
    if (!showContainers) return;
    let cancelled = false;
    const check = async () => {
      try {
        const ok = await dockerAvailable();
        if (cancelled) return;
        setDockerUp(ok);
        if (!ok) {
          setDockerRunning(0);
          return;
        }
        const list = await dockerListContainers(null, true);
        if (!cancelled) {
          setDockerRunning(list.filter((c) => c.state === "running").length);
        }
      } catch {
        if (!cancelled) {
          setDockerUp(false);
          setDockerRunning(0);
        }
      }
    };
    void check();
    const timer = window.setInterval(() => void check(), 30_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [showContainers]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void (async () => {
      const un = await onLayoutChanged((state) => {
        if (cancelled) return;
        setLayout(state);
        const known = sessionIdsRef.current;
        const hasUnknownSession = state.workspaces.some((w) =>
          w.tabs.some(
            (tab) =>
              tab.root !== null &&
              leafSessions(tab.root).some((id) => !known.has(id)),
          ),
        );
        if (hasUnknownSession) void refreshSessions();
      });
      if (cancelled) {
        un();
        return;
      }
      unlisten = un;
      // UMA chamada, e não dezoito.
      //
      // Eram 18 `invoke` no mount, 16 deles `get_pref`. Comando síncrono do
      // Tauri roda na main thread do macOS — a mesma que desenha o webview —,
      // então "paralelo" no JS vira fila ali, entremeada com a construção dos
      // terminais. O core devolve prefs, sessões e layout de uma vez.
      const snapshot = await bootSnapshot().catch(() => null);
      if (cancelled) return;
      // A falha do boot precisa ser lida AQUI também, e não só no poll.
      //
      // O poll só roda enquanto `bootReady` é falso. Quando a thread de boot
      // morre, o core abre o portão assim mesmo — senão todo comando de escrita
      // pagaria o timeout —, então este snapshot volta `ready: true` COM
      // `bootFailure`. Ler só no poll significa que `markReady()` desmonta o
      // efeito e cancela o timer antes do primeiro tick, e o aviso nunca sai:
      // um app sem sessão, sem layout e sem explicação, que é exatamente o
      // "vazio indistinguível de falha" que este campo existe para evitar.
      if (snapshot?.bootFailure) reportBootFailure(snapshot.bootFailure);
      // Só aplica se veio de fato: `{}` de uma chamada que rejeitou não é
      // "nenhuma preferência", é "não sei" — e gravar isso como estado deixaria
      // o dono com todos os padrões pela vida da janela. O poll reaplica.
      if (snapshot) applyPrefs(snapshot.prefs);
      // As listas passam pela thread de boot e só valem quando ela terminou —
      // vazio do meio do boot não é vazio. Ver `mergeLoaded`.
      const loaded = acceptLoaded(
        snapshot ? { ready: snapshot.ready, value: snapshot } : null,
      );
      if (loaded) {
        setSessions(loaded.sessions);
        setLayout(loaded.layout);
      }
      // Só o layout que valeu como estado decide o modal. Resposta atrasada não
      // decide nada: quem decidiu foi o handler do `app://ready`, com o layout
      // que ele mesmo reconsultou.
      promptNewSessionIfEmpty(
        loaded !== null,
        loaded?.layout.workspaces.length ?? 0,
      );
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [acceptLoaded, refreshSessions, promptNewSessionIfEmpty, applyPrefs]);

  // Texto que a linha do TYBA recebeu de fora — paste, histórico, snippet.
  // Declarado aqui, e não junto do resto da linha lá embaixo, porque o
  // `deliverPaste` precisa dele.
  //
  // Por SESSÃO: cada painel tem a própria linha, e o texto vai para a linha do
  // painel a que foi endereçado — nunca para "a linha", que com a tela dividida
  // seria a do painel que por acaso tem o foco.
  const [injected, setInjected] = useState<
    Record<string, { text: string; nonce: number }>
  >({});
  const injectLine = useCallback((sessionId: SessionId, text: string) => {
    setInjected((prev) => ({
      ...prev,
      [sessionId]: { text, nonce: Date.now() },
    }));
  }, []);
  // Qual linha está no comando do teclado, por sessão. Ref pelo mesmo motivo
  // que `promptModesRef` existe: as linhas só são calculadas bem abaixo, e uma
  // dependência de `useCallback` seria lida antes de existir.
  const ownsLineRef = useRef<Record<string, boolean>>({});

  const deliverPaste = useCallback(
    (sessionId: SessionId, raw: string) => {
      const entry = getTerm(sessionId);
      if (!entry) return;
      // A dona do teclado é a linha DESTA sessão. Colar num painel vizinho vai
      // para o que é dono do teclado lá — a linha dele ou o terminal dele.
      const route = routePaste({
        raw,
        ownsCommandLine: ownsLineRef.current[sessionId] ?? false,
        bracketed: entry.term.modes.bracketedPasteMode,
      });
      if (!route) return;
      if (route.to === "line") {
        injectLine(sessionId, route.text);
        return;
      }
      if (route.to === "terminal") {
        entry.term.paste(route.text);
        return;
      }
      setPastePrompt({ sessionId, text: route.text });
    },
    [injectLine],
  );

  const pasteFromClipboard = useCallback(async () => {
    if (!activeId) return;
    // O alvo é decidido ANTES de esperar a área de transferência: o foco pode
    // trocar de painel enquanto ela responde.
    const target = activeId;
    let text = "";
    try {
      text = await readClipboardText();
    } catch {
      return;
    }
    deliverPaste(target, text);
  }, [activeId, deliverPaste]);

  const confirmPaste = useCallback(
    (mode: "raw" | "single") => {
      if (!pastePrompt) return;
      const entry = getTerm(pastePrompt.sessionId);
      const text =
        mode === "single"
          ? flattenPaste(pastePrompt.text)
          : sanitizePaste(pastePrompt.text);
      // O comando pode ter terminado entre abrir o preview e confirmar, e aí a
      // linha do TYBA voltou a ser a dona do teclado: o `term.paste` seria
      // engolido e o texto confirmado sumiria sem aviso.
      if (ownsLineRef.current[pastePrompt.sessionId]) {
        injectLine(pastePrompt.sessionId, text);
        setPastePrompt(null);
        return;
      }
      entry?.term.paste(text);
      entry?.term.focus();
      setPastePrompt(null);
    },
    [pastePrompt, injectLine],
  );

  // Despacho único das ações: o teclado e o menu nativo do macOS entram por
  // aqui. Duas cópias desta lista divergiriam no primeiro atalho novo.
  const runAction = useCallback(
    (action: KeyAction) => {
      if (action === "paletteActions") {
        openPalette("actions");
      } else if (action === "paletteSessions") {
        openPalette("sessions");
      } else if (action === "paletteHistory") {
        openPalette("history");
      } else if (action === "paletteSnippets") {
        openPalette("snippets");
      } else if (action === "panel") {
        toggleSidebar();
      } else if (action === "files") {
        if (fileSessionId) void openFilesPanel(fileSessionId).catch(() => {});
      } else if (action === "clear") {
        // Manda `clear` para o shell em vez de limpar por conta própria: esse
        // caminho já existe e já leva os BLOCOS junto (`Action::Wipe`), que em
        // modo bloco é o que "limpar" significa. Um segundo mecanismo aqui
        // divergiria do primeiro no dia em que um dos dois mudasse.
        //
        // Com comando em execução não faz nada: `clear` ali não limparia
        // coisa alguma, viraria texto na entrada do programa em voo — e o
        // `y` que o usuário não digitou é exatamente o tipo de acidente que a
        // regra dos três estados existe para evitar.
        const alvo = activeId;
        if (alvo && !sessionCommandsRef.current[alvo]?.running) {
          void writeToSession(alvo, "clear\n").catch(() => {});
        }
      } else if (action === "filesFinder") {
        openPalette("files");
      } else if (action === "newTab") {
        void newTab();
      } else if (action === "closePane") {
        void closeActivePane();
      } else if (action === "openFolder") {
        void openProjectFolder();
      } else if (action === "newSession") {
        openNewSession();
      } else if (action === "newWorktreeSession") {
        openNewSession(true);
      } else if (action === "newWindow") {
        void newWindow().catch(() => {});
      } else if (action === "prevSession") {
        cycleWorkspace(-1);
      } else if (action === "nextSession") {
        cycleWorkspace(1);
      } else if (action === "nextAttentionSession") {
        void toggleAgentQueue().catch(() => {});
      } else if (action === "prevTab") {
        cycleTab(-1);
      } else if (action === "nextTab") {
        cycleTab(1);
      } else if (action === "tabBack") {
        void backTab().catch(() => {});
      } else if (action === "paneLeft") {
        focusPaneInDirection("left");
      } else if (action === "paneRight") {
        focusPaneInDirection("right");
      } else if (action === "paneUp") {
        focusPaneInDirection("up");
      } else if (action === "paneDown") {
        focusPaneInDirection("down");
      } else if (action === "settings") {
        toggleSettings();
      } else if (action === "splitRight") {
        void splitActive("v");
      } else if (action === "splitDown") {
        void splitActive("h");
      } else if (action === "nextPane") {
        cyclePane();
      } else if (action === "search") {
        // Só o menu chega aqui: pelo teclado o ⌘F passa pelo caminho de cima,
        // que exige o terminal em foco.
        if (getTerm(activeId)) setSearchOpen((v) => !v);
      } else if (action === "promptLine" && activeId) {
        // Alterna na sessão VIVA: a preferência só entra por env no spawn, e
        // pedir sessão nova para experimentar é atrito demais.
        void togglePromptMode(activeId).catch(() => {});
      } else if (action === "richInput" && activeId) {
        if (richInputFocused.current) {
          richInputFocused.current = false;
          getTerm(activeId)?.term.focus();
        } else {
          openRichInput(activeId);
        }
      }
    },
    [
      activeId,
      fileSessionId,
      openPalette,
      toggleSidebar,
      newTab,
      closeActivePane,
      openProjectFolder,
      newWindow,
      cycleWorkspace,
      cycleTab,
      focusPaneInDirection,
      toggleSettings,
      splitActive,
      cyclePane,
      openRichInput,
    ],
  );

  // Quem é dono do teclado agora. A regra vive em lib/commandLine.ts, testada:
  // é ela que impede a caixa de engolir a senha que o sudo está pedindo.
  //
  // Por PAINEL: cada sessão com painel tem a própria linha, e o estado dela —
  // visível, editável, dona do teclado — sai só do estado DAQUELA sessão. O
  // vizinho do `vim` continua no prompt; o vizinho de um comando rodando
  // continua aceitando texto. Ver `paneLine`.
  //
  // O shell só reporta o modo no primeiro prompt — depois de carregar rc,
  // framework e plugins. Até lá a linha aparece desabilitada em vez de a tela
  // ficar em branco sem lugar nenhum para digitar. A preferência só adianta a
  // linha onde o hook pode chegar (ver `lineVisible`).
  const paneLines = new Map<string, PaneLine>();
  for (const s of sessions) {
    if (!pannedSessions.has(s.id)) continue;
    paneLines.set(
      s.id,
      paneLine({
        kind: s.kind,
        sessionIntegration: integrations[s.id],
        reported: promptModes[s.id],
        promptModePref,
        hookExpected: hookExpected[s.id],
        // A rajada é interceptada no xterm; a linha do TYBA o deixa
        // somente-leitura. Ver `OwnerInput.broadcasting`.
        broadcasting:
          broadcastOn &&
          s.kind.type === "ssh" &&
          broadcastSet.includes(s.id),
        altScreen: altScreens[s.id] ?? false,
        command: sessionCommands[s.id],
      }),
    );
  }
  const ownsLines: Record<string, boolean> = {};
  for (const [id, line] of paneLines) ownsLines[id] = line.owns;
  ownsLineRef.current = ownsLines;
  // A sessão ativa ainda decide o que é da JANELA: a caixa de agente e o
  // histórico/snippet pedidos pela paleta.
  const ownsCommandLine = activeId ? (ownsLines[activeId] ?? false) : false;

  // Voltar do vim, o fim de um comando ou a troca de painel devolvem o foco à
  // linha — só à do painel ATIVO. Cada linha guarda o próprio contador; o
  // painel que terminou um comando sem estar em foco não rouba o teclado.
  const [lineFocusNonces, setLineFocusNonces] = useState<
    Record<string, number>
  >({});
  const focusLine = useCallback((id: SessionId) => {
    setLineFocusNonces((prev) => ({ ...prev, [id]: (prev[id] ?? 0) + 1 }));
  }, []);
  // Com a tela de Git aberta o teclado é dela (Esc, campos): nada de puxar o
  // foco de volta por baixo. Quando ela desmonta, o effect roda de novo e o
  // foco volta ao terminal.
  const gitScreenOpen =
    gitScreenSession !== null || filesScreenSession !== null || dockerScreenSession !== null;
  useEffect(() => {
    if (!activeId || gitScreenOpen) return;
    if (ownsCommandLine) {
      focusLine(activeId);
      return;
    }
    // Devolveu o teclado ao terminal (vim abriu, comando começou, modo
    // desligado): o foco vai junto, senão as teclas caem numa caixa que já não
    // é dona da linha.
    getTerm(activeId)?.term.focus();
  }, [ownsCommandLine, activeId, focusLine, gitScreenOpen]);

  // Quem responde se o PS1 saiu da tela é o SHELL, não o app: o hook relata por
  // `633;P` a cada prompt. Assumir pela preferência mentiria quando o hook não
  // subiu (shell sem integração, subshell, container).
  useEffect(() => {
    const ids = sessions.map((s) => s.id);
    // Rascunho de sessão que não existe mais não volta a ser lido por ninguém,
    // e o mapa é `ref`: sem isto ele só cresce enquanto o app estiver aberto.
    const vivos = new Set<string>(ids);
    for (const id of Object.keys(drafts.current)) {
      if (!vivos.has(id)) delete drafts.current[id];
    }
    for (const id of Object.keys(pendings.current)) {
      if (!vivos.has(id)) delete pendings.current[id];
    }
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    for (const id of ids) {
      void onSessionPromptMode(id, (on) =>
        setPromptModes((prev) =>
          prev[id] === on ? prev : { ...prev, [id]: on },
        ),
      ).then((un) => {
        if (disposed) un();
        else unlisteners.push(un);
      });
      // Consulta além de assinar: se o primeiro prompt chegou antes deste
      // listener existir, o evento se perdeu e a linha nunca apareceria.
      void sessionPromptMode(id)
        .then((on) => {
          if (!disposed && on) {
            setPromptModes((prev) => (prev[id] ? prev : { ...prev, [id]: on }));
          }
        })
        .catch(() => {});
      // Ao contrário do modo prompt, esta resposta é definitiva no instante do
      // spawn: não muda depois, então basta perguntar uma vez.
      // Também definitivo no spawn: o shell e o rc não mudam depois.
      void sessionShellBoot(id)
        .then((boot) => {
          if (!disposed) {
            setShellBoots((prev) => (id in prev ? prev : { ...prev, [id]: boot }));
          }
        })
        .catch(() => {});
      void sessionHookExpected(id)
        .then((expected) => {
          if (!disposed) {
            setHookExpected((prev) =>
              prev[id] === expected ? prev : { ...prev, [id]: expected },
            );
          }
        })
        .catch(() => {});
    }
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, [sessions]);

  // Histórico persistido + os que chegam agora. O bloco vem pronto do core: o
  // front não parseia saída, só desenha os spans.
  useEffect(() => {
    // A SSH Session integrada grava bloco pelo mesmo caminho do shell local
    // (regra 14) — é o mesmo `133;A/B/C/D`, vindo do servidor.
    const ids = sessions
      .filter((s) =>
        isIntegratedSession({ kind: s.kind, integration: integrations[s.id] }),
      )
      .map((s) => s.id);
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    for (const id of ids) {
      void sessionBlocks(id)
        .then((loaded) => {
          if (disposed || loaded.length === 0) return;
          setBlocks((prev) => {
            const merged = mergeBlockHistory(prev[id] ?? [], loaded);
            return merged === prev[id] ? prev : { ...prev, [id]: merged };
          });
        })
        .catch(() => {});
      void onBlockFinalized(id, (block) => {
        setBlocks((prev) => ({ ...prev, [id]: [...(prev[id] ?? []), block] }));
      }).then((un) => {
        if (disposed) un();
        else unlisteners.push(un);
      });
      void onBlocksCleared(id, () => {
        setBlocks((prev) => (prev[id]?.length ? { ...prev, [id]: [] } : prev));
        setBlockPick((prev) => (prev?.session === id ? null : prev));
      }).then((un) => {
        if (disposed) un();
        else unlisteners.push(un);
      });
    }
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, [sessions, integrations]);

  // Blocos marcados para copiar de uma vez. Estado de tela, não de sessão: é
  // qual cartão está aceso, e morre com a janela.
  const [blockPick, setBlockPick] = useState<{
    session: SessionId;
    selection: BlockSelection;
  } | null>(null);

  const pickBlock = useCallback(
    (session: SessionId, id: number, event: React.MouseEvent) => {
      // Arrastar texto dentro de um bloco termina em clique. Marcar o cartão
      // aqui apagaria a seleção que a pessoa acabou de fazer.
      //
      // Menos com shift: shift-clique é COMO o navegador estende seleção de
      // texto, então o guarda veria sempre texto selecionado e o intervalo de
      // blocos nunca aconteceria. Aqui shift é do bloco — e a seleção de texto
      // que ele deixou para trás vai embora junto.
      const text = window.getSelection();
      if (event.shiftKey) text?.removeAllRanges();
      else if (text && !text.isCollapsed) return;
      const order = (blocksRef.current[session] ?? []).map(
        (block) => block.id,
      );
      setBlockPick((prev) => {
        const current = prev?.session === session ? prev.selection : null;
        const next = selectBlock(current, order, id, modeFor(event));
        return next ? { session, selection: next } : null;
      });
    },
    [],
  );

  // A sessão é amarrada FORA do JSX, senão a prop nasce com identidade nova a
  // cada render e derruba o `memo` do cartão de bloco. Ver `handlerCache`.
  const pickerFor = useMemo(() => handlerCache(pickBlock), [pickBlock]);
  const clearPick = useCallback(() => setBlockPick(null), []);

  const markedBlocks = useMemo(
    () => (blockPick ? new Set(blockPick.selection.ids) : null),
    [blockPick],
  );

  // A faixa ao vivo só abre para comando que dura. Ver `LIVE_DELAY_MS`.
  const [liveSlow, setLiveSlow] = useState<Record<string, boolean>>({});
  const liveTimers = useRef<Record<string, number>>({});
  useEffect(() => {
    const timers = liveTimers.current;
    for (const [id, cmd] of Object.entries(sessionCommands)) {
      if (cmd?.running) {
        if (timers[id] === undefined) {
          timers[id] = window.setTimeout(() => {
            delete timers[id];
            setLiveSlow((prev) => (prev[id] ? prev : { ...prev, [id]: true }));
          }, LIVE_DELAY_MS);
        }
        continue;
      }
      if (timers[id] !== undefined) {
        window.clearTimeout(timers[id]);
        delete timers[id];
      }
      setLiveSlow((prev) => (prev[id] ? { ...prev, [id]: false } : prev));
    }
  }, [sessionCommands]);
  useEffect(() => {
    const timers = liveTimers.current;
    return () => {
      for (const timer of Object.values(timers)) window.clearTimeout(timer);
    };
  }, []);

  /**
   * A faixa ao vivo está aberta para esta sessão?
   *
   * Uma resposta só, usada pelo recorte do terminal e pelo layout dos blocos:
   * quando as duas contas divergem, o terminal fica recortado sem a lista
   * saber, e vira uma tira no rodapé do painel.
   */
  const liveOf = useCallback(
    (id: SessionId) =>
      Boolean(liveSlow[id]) &&
      !wipesTheScreen(sessionCommands[id]?.command ?? null),
    [liveSlow, sessionCommands],
  );

  // Quanto da tela a saída de cada sessão está ocupando — recorta a faixa ao
  // vivo na altura dela para o cartão nascer onde a saída estava.
  const [liveUsed, setLiveUsed] = useState<Record<string, number>>({});
  // Altura medida do header do bloco em execução. A lista encurta desse tanto —
  // o header é desenhado sobre o fim dela. Ver `BlockList.bottomInset`.
  //
  // Por SESSÃO, não por janela: com a tela dividida os headers são medidos em
  // painéis de larguras diferentes, e um valor único deixa o último a medir
  // sobrescrever o outro. Ver `lib/perSession`.
  const [activeHeaderPx, setActiveHeaderPx] = useState<Record<string, number>>(
    {},
  );

  // O corpo dos blocos acompanha a fonte do TERMINAL: é a mesma saída, e ela
  // mudava de tamanho ao virar cartão porque o corpo estava preso em 13px.
  //
  // Esta é preferência do dono e vale para a janela inteira — ao contrário das
  // medidas abaixo, que saem do layout de cada painel.
  const [termFontSize, setTermFontSize] = useState(getDefaultFontSize);
  // Altura real de uma linha do terminal, medida por ele. Ver `onLineHeight`.
  // Por sessão pelo mesmo motivo do header.
  const [termLineHeight, setTermLineHeight] = useState<Record<string, number>>(
    {},
  );
  // Enquanto o terminal de uma sessão não mediu, a estimativa sai da fonte —
  // é o que evita o primeiro render posicionar os blocos com altura chutada.
  const fallbackLineHeight = termFontSize * TERMINAL_LINE_HEIGHT;
  const reportLineHeight = useCallback((id: SessionId, px: number) => {
    setTermLineHeight((prev) => withEntry(prev, id, px));
  }, []);
  // Largura de uma célula, medida — a estimativa de altura do bloco precisa
  // saber quantos caracteres cabem na linha, porque o corpo do cartão quebra.
  const [termCellWidth, setTermCellWidth] = useState<Record<string, number>>({});
  const fallbackCellWidth = termFontSize * TERMINAL_CELL_WIDTH;
  const reportCellWidth = useCallback((id: SessionId, px: number) => {
    setTermCellWidth((prev) => withEntry(prev, id, px));
  }, []);
  const reportHeaderPx = useCallback((id: SessionId, px: number) => {
    setActiveHeaderPx((prev) => withEntry(prev, id, px));
  }, []);
  useEffect(() => {
    const sync = () => setTermFontSize(getDefaultFontSize());
    sync();
    window.addEventListener(FONT_SIZE_EVENT, sync);
    return () => window.removeEventListener(FONT_SIZE_EVENT, sync);
  }, []);

  // O tty entrega linhas ou teclas? Decide se a seta vai ao PTY ou morre no
  // caminho — ver `swallowsArrow`.
  //
  // Consultado em intervalo, e não uma vez por comando: o MESMO comando troca
  // de modo no meio. O `npm create` pergunta `Ok to proceed? (y)` em canônico e
  // vira raw ao abrir o menu; um valor lido no início mandaria a seta para o
  // lado errado da metade em diante.
  //
  // Só enquanto há comando rodando: fora disso a linha do TYBA já é dona do
  // teclado e a resposta não muda nada.
  const [lineEcho, setLineEcho] = useState<Record<string, boolean>>({});
  const runningIds = useMemo(
    () =>
      Object.entries(sessionCommands)
        .filter(([, cmd]) => cmd?.running)
        .map(([id]) => id)
        .sort()
        .join(","),
    [sessionCommands],
  );
  useEffect(() => {
    const ids = runningIds ? runningIds.split(",") : [];
    if (ids.length === 0) return;
    let alive = true;
    const poll = () => {
      for (const id of ids) {
        void sessionLineEcho(id)
          .then((on) => {
            if (!alive) return;
            setLineEcho((prev) => withEntry(prev, id, on));
          })
          .catch(() => {});
      }
    };
    poll();
    const timer = window.setInterval(poll, LINE_ECHO_POLL_MS);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [runningIds]);
  const reportLiveRows = useCallback(
    (id: SessionId, used: number, total: number, scrolled: boolean) => {
      const next = usedFraction(used, total, scrolled);
      setLiveUsed((prev) => withEntry(prev, id, next));
    },
    [],
  );
  const blockPickRef = useRef<typeof blockPick>(null);
  blockPickRef.current = blockPick;

  const historyScope = useMemo(
    () => ({
      cwd: activeCwdKey,
      repoRoot: activeSession?.repo_root ?? null,
      sessionId: activeSession?.id ?? null,
    }),
    [activeCwdKey, activeSession?.repo_root, activeSession?.id],
  );

  // Histórico e snippet entram na linha pelo MESMO caminho do paste: bracketed
  // paste detectado, control chars sanitizados e confirmação quando é multilinha.
  // Nada é executado — quem aperta Enter é o dono.
  //
  // Endereçado pela sessão que pediu, nunca por `activeId` lido depois: entre o
  // clique e a entrega o foco pode ter trocado de painel, e o texto iria parar
  // na linha errada.
  //
  // `useStableCallback` e não `useCallback`: isto desce como `onInject` para o
  // `SessionBlocks`, que é `memo`. As dependências naturais deste handler são
  // as que mais mudam no app — `ownsLines` a cada começo e fim de comando —,
  // então a lista daria identidade nova em toda fronteira de comando e
  // invalidaria a memoização de TODOS os painéis, inclusive os que nada tinham
  // a ver com o comando. Ver `lib/stableProps`.
  const injectInto = useStableCallback((sessionId: SessionId, text: string) => {
    if (!text) return;
    // Sem decidir de novo para onde o texto vai: quem decide é o
    // `deliverPaste`, e duas cópias da regra divergiriam — foi assim que o
    // "Colar" do menu de contexto ficou inerte enquanto histórico e snippet
    // funcionavam.
    deliverPaste(sessionId, text);
    if (!ownsLineRef.current[sessionId]) getTerm(sessionId)?.term.focus();
  });
  // A paleta de histórico é da janela: o alvo é o painel ativo no momento da
  // escolha.
  const injectIntoActive = useStableCallback((text: string) => {
    if (activeId) injectInto(activeId, text);
  });

  const pickSnippet = useCallback(
    (snippet: Snippet) => {
      // O alvo é o painel ativo NA ESCOLHA. Os placeholders e o render são
      // assíncronos, e o foco pode trocar de painel até o texto voltar.
      const target = activeId;
      if (!target) return;
      void snippetPlaceholders(snippet.command)
        .then((found) => {
          if (found.length === 0) {
            return renderSnippet(snippet.id, snippet.command, []).then(
              (text) => injectInto(target, text),
            );
          }
          setSnippetPrompt({ snippet, placeholders: found, sessionId: target });
        })
        .catch((error) => toastError(t("snippetsError"), error));
    },
    [activeId, injectInto, t],
  );

  const confirmSnippet = useCallback(
    (values: [string, string][]) => {
      const prompt = snippetPrompt;
      setSnippetPrompt(null);
      if (!prompt) return;
      void renderSnippet(prompt.snippet.id, prompt.snippet.command, values)
        .then((text) => injectInto(prompt.sessionId, text))
        .catch((error) => toastError(t("snippetsError"), error));
    },
    [snippetPrompt, injectInto, t],
  );

  const runMenuExtra = useCallback(
    (id: MenuExtraId) => {
      if (id === "menu:shortcuts") {
        setShortcutsOpen((open) => !open);
      } else if (id === "menu:checkUpdates") {
        // Abre as Configurações junto: sem versão nova o toast não aparece, e
        // um item de menu que não responde parece quebrado.
        void updateCheck().then(setUpdate).catch(() => {});
        void openViewTab("settings").catch(() => {});
      } else if (id === "menu:docs") {
        void openExternalUrl(docsUrl(i18n.language)).catch(() => {});
      } else if (id === "menu:changelog") {
        void openExternalUrl(changelogUrl(i18n.language)).catch(() => {});
      } else if (id === "menu:issues") {
        void openExternalUrl(`${REPO_URL}/issues/new`).catch(() => {});
      }
    },
    [i18n.language, openViewTab],
  );

  // O menu nativo espelha os atalhos e o idioma correntes: rebind e troca de
  // idioma remontam a barra.
  useEffect(() => {
    void setAppMenu(buildMenuSpec(t, bindings)).catch(() => {});
  }, [t, bindings, i18n.language]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void onMenuAction((id) => {
      if (isMenuExtraId(id)) {
        runMenuExtra(id);
        return;
      }
      if ((KEY_ACTIONS as string[]).includes(id)) runAction(id as KeyAction);
    }).then((un) => {
      if (disposed) un();
      else unlisten = un;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [runAction, runMenuExtra]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (captureState.active) return;
      // Esc só sai da seleção de blocos quando o teclado não é de um campo:
      // com o foco na caixa de comando, Esc é dela — fecha a sugestão.
      if (
        e.key === "Escape" &&
        blockPickRef.current &&
        !inTextField(document.activeElement)
      ) {
        e.preventDefault();
        e.stopPropagation();
        setBlockPick(null);
        return;
      }
      // Voltar a digitar encerra o modo. Sem isto a seleção fica acesa por trás
      // de um comando novo, porque com o foco na caixa o Esc é dela — e o
      // usuário não tem como saber que precisa clicar em algum lugar.
      if (
        blockPickRef.current &&
        e.key.length === 1 &&
        !e.metaKey &&
        !e.ctrlKey &&
        !e.altKey
      ) {
        setBlockPick(null);
      }
      const combo = comboOf(e);
      if (!combo) return;
      // ⌘[ é "voltar de aba" — menos dentro do editor de código, onde é do
      // CodeMirror (desindentar); lá o voltar é ⌘⌥←.
      const backRoute = tabBackRoute(
        combo,
        bindings,
        Boolean((document.activeElement as HTMLElement | null)?.closest?.(".cm-editor")),
      );
      if (backRoute === "native") return;
      if (backRoute === "back") {
        e.preventDefault();
        e.stopPropagation();
        if (!e.repeat) void backTab().catch(() => {});
        return;
      }
      const action = (Object.keys(bindings) as KeyAction[]).find(
        (a) => bindings[a] === combo,
      );
      // Blocos marcados fazem o ⌘C copiar os blocos. Perde para quem tem
      // seleção de texto de verdade: quem destacou algo quer aquilo, não o
      // cartão que ficou aceso de antes.
      if (action === "copy" && blockPickRef.current) {
        const pick = blockPickRef.current;
        const entry = getTerm(activeId);
        const text = window.getSelection();
        const busy =
          (text && !text.isCollapsed) ||
          (isTermFocused(entry) && entry?.term.hasSelection());
        if (pick.session === activeId && !busy) {
          e.preventDefault();
          e.stopPropagation();
          if (e.repeat) return;
          const chosen = pickedBlocks(
            pick.selection,
            blocksRef.current[activeId ?? ""] ?? [],
          );
          if (chosen.length > 0) {
            void writeClipboardText(blocksMarkdown(chosen)).catch(() => {});
          }
          return;
        }
      }
      if (action && isTerminalAction(action)) {
        const entry = getTerm(activeId);
        const focused = isTermFocused(entry);
        if (action === "search") {
          if (!entry || (!focused && !searchOpen)) return;
          e.preventDefault();
          e.stopPropagation();
          if (e.repeat) return;
          setSearchOpen((v) => !v);
          return;
        }
        if (!entry || !focused) return;
        if (action === "copy") {
          if (!entry.term.hasSelection()) return;
          e.preventDefault();
          e.stopPropagation();
          if (e.repeat) return;
          void writeClipboardText(entry.term.getSelection()).catch(() => {});
          return;
        }
        e.preventDefault();
        e.stopPropagation();
        if (e.repeat) return;
        if (action === "selectAll") {
          entry.term.selectAll();
        } else {
          suppressNativePaste();
          void pasteFromClipboard();
        }
        return;
      }
      if (action) {
        e.preventDefault();
        e.stopPropagation();
        if (e.repeat) return;
        runAction(action);
        return;
      }
      if (isPaneResizeChord(e)) {
        const key = e.key.toLowerCase();
        if (key === "arrowleft" || key === "arrowright") {
          e.preventDefault();
          e.stopPropagation();
          resizeActivePane("v", key === "arrowright" ? 0.05 : -0.05);
          return;
        }
        if (key === "arrowup" || key === "arrowdown") {
          e.preventDefault();
          e.stopPropagation();
          resizeActivePane("h", key === "arrowdown" ? 0.05 : -0.05);
          return;
        }
      }
      if (isTabDigitChord(e) && e.key >= "1" && e.key <= "9") {
        // Consome o chord SEMPRE (mesmo já na aba, ou aba inexistente), senão o
        // dígito vaza pro xterm e é digitado no shell.
        e.preventDefault();
        e.stopPropagation();
        if (e.repeat) return;
        const target = activeWorkspace?.tabs[Number(e.key) - 1];
        if (target) void activateTab(target.id);
        return;
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [
    activeId,
    searchOpen,
    pasteFromClipboard,
    bindings,
    runAction,
    resizeActivePane,
    activeWorkspace,
  ]);

  const open = sidebar === "open";

  const worktreeSessionOf = (w: Workspace): Session | null => {
    for (const tab of w.tabs) {
      if (!tab.root) continue;
      for (const sid of leafSessions(tab.root)) {
        const session = sessionById.get(sid);
        if (session?.worktree) return session;
      }
    }
    return null;
  };

  const renderWorkspaceMenuItems = (
    w: Workspace,
    branch: string | undefined,
    M: MenuParts,
  ) => (
    <>
      <M.Item
        className="text-xs"
        onSelect={() => setPrompt({ kind: "rename", ws: w })}
      >
        {t("renameSession")}
      </M.Item>
      {worktreeSessionOf(w) && (
        <M.Item
          className="text-xs"
          onSelect={() => {
            const target = worktreeSessionOf(w);
            if (target) void openDiffContentTab(target.id).catch(() => {});
          }}
        >
          {t("diffReviewAction")}
        </M.Item>
      )}
      <M.Sub>
        <M.SubTrigger className="text-xs">{t("groupSession")}</M.SubTrigger>
        <M.SubContent className="w-44">
          {groupNames.map((name) => (
            <M.Item
              key={name}
              className="text-xs"
              onSelect={() =>
                void setWorkspaceGroup(w.id, w.group === name ? null : name)
              }
            >
              <span className="min-w-0 flex-1 truncate">{name}</span>
              {w.group === name && (
                <Check size={11} weight="bold" className="text-tyba-green" />
              )}
            </M.Item>
          ))}
          {groupNames.length > 0 && <M.Separator />}
          <M.Item
            className="text-xs"
            onSelect={() => setPrompt({ kind: "group", ws: w })}
          >
            <Plus size={11} weight="bold" />
            {t("newGroup")}
          </M.Item>
        </M.SubContent>
      </M.Sub>
      {w.group && (
        <M.Item
          className="text-xs"
          onSelect={() => void setWorkspaceGroup(w.id, null)}
        >
          {t("removeFromGroup")}
        </M.Item>
      )}
      <M.Separator />
      {branch && (
        <M.Item className="text-xs" onSelect={() => copyText(branch)}>
          {t("copyBranch")}
        </M.Item>
      )}
      {w.repo_root && (
        <M.Item
          className="text-xs"
          onSelect={() => copyText(w.repo_root as string)}
        >
          {t("copyDir")}
        </M.Item>
      )}
      {(branch || w.repo_root) && <M.Separator />}
      <div className="flex items-center gap-1.5 px-2 py-1.5">
        <button
          aria-label={t("noColor")}
          onClick={() => void setWorkspaceColor(w.id, null)}
          className={`flex size-4 items-center justify-center rounded-full border text-tyba-text-faint ${
            !w.color ? "border-tyba-text-muted" : "border-tyba-border-strong"
          }`}
        >
          <Prohibit size={10} />
        </button>
        {SESSION_COLORS.map((c) => (
          <button
            key={c}
            aria-label={c}
            onClick={() => void setWorkspaceColor(w.id, c)}
            className={`size-4 rounded-full border ${
              w.color === c ? "border-tyba-text" : "border-transparent"
            }`}
            style={{ background: `var(--tyba-${c})` }}
          />
        ))}
      </div>
      <M.Separator />
      <M.Item
        className="text-xs"
        onSelect={() => toggleWorkspaceDetails(w.id)}
      >
        {detailsFor(w.id) ? t("detailsHide") : t("detailsShow")}
      </M.Item>
      <M.Separator />
      <M.Item
        className="text-xs text-tyba-red focus:text-tyba-red"
        onSelect={() => void killWorkspace(w.id)}
      >
        <X size={12} weight="bold" />
        {t("killSession")}
      </M.Item>
    </>
  );

  const renderWorkspace = (w: Workspace) => {
    const isActive = w.id === layout.active_workspace;
    const isConfig = isConfigWorkspace(w);
    const isWtView = isWorktreesWorkspace(w);
    const showDetails = open && detailsFor(w.id) && !isConfig && !isWtView;
    const gitDir = workspaceGitDir(w);
    const sshHost = workspaceSshHost(w);
    const displayDir = workspaceCwd(w) ?? w.repo_root;
    // Nome automático segue o cwd vivo (cd troca o contexto da sessão);
    // renomeou de propósito → o nome escolhido fica.
    const displayName = w.name_locked
      ? w.name
      : displayDir
        ? basename(displayDir)
        : w.name;
    const snapshot = gitDir ? snapshotForDir(repoSnapshots, gitDir) : undefined;
    // De onde branch e diff desta linha vêm (regra 23) — a decisão mora em
    // `sidebarChips`, testada; aqui só se junta o estado.
    const remoteSessionId =
      isConfig || isWtView ? null : workspaceRemoteSessionId(w);
    const chips = sidebarChips({
      remote: remoteSessionId !== null,
      showStatus: showGitStatus,
      chips: remoteSessionId ? remoteChips[remoteSessionId] : undefined,
      local: { branch: snapshot?.branch, status: snapshot?.status },
    });
    const branch = chips.branch ?? undefined;
    const gitStatus = chips.status;
    const runner = isConfig || isWtView ? null : workspaceAgent(w);
    const runningCmd = isConfig || isWtView ? null : workspaceCommand(w);
    const hoverAgent = runner ?? agentFromCommand(runningCmd);
    const agentStatus = isConfig || isWtView ? null : workspaceAgentStatus(w);
    const agentDetail = (() => {
      if (!agentStatus) return null;
      const label = t(agentStatus.visual.labelKey);
      // Observado não tem `hint` nem `reason`: o que a tela diz é o estado, e
      // mais nada. O nome do agente é o que orienta — sem ele a linha diria
      // "sem sinal" sem dizer de quem.
      if (agentStatus.observed) {
        return `${agentStatus.observed.agent} · ${label}`;
      }
      const status = agentStatus.session.status;
      if (status.state === "awaiting_input" && status.hint) {
        return `${label}: ${status.hint}`;
      }
      if (status.state === "failed") return `${label}: ${status.reason}`;
      if (status.state === "running") return runningCmd ?? label;
      return label;
    })();
    // Cartão do mockup Proposta no visual (ativa sobe, ponto de estado à
    // direita), com os dados de sempre: caminho/host, agente ou comando,
    // branch, contadores do git e número de abas — ativa ou não. A regra do
    // ponto mora em `lib/sessionCard`, testada.
    const workspaceSessions = new Set(
      w.tabs.flatMap((tab) => (tab.root ? leafSessions(tab.root) : [])),
    );
    const dot = sessionDot({
      approvals: approvals.filter((a) => workspaceSessions.has(a.session_id))
        .length,
      agentRank: agentStatus?.visual.rank ?? null,
      commandRunning: runningCmd != null,
    });
    const workspaceButton = (
      <button
        key={w.id}
        onClick={() => void activateWorkspace(w.id)}
        style={
          w.color
            ? {
                background: `color-mix(in srgb, var(--tyba-${w.color}) ${
                  isActive ? 14 : 8
                }%, transparent)`,
              }
            : undefined
        }
        // Ativa = o cartão que SOBE (mockup Proposta): `raised` com o contorno
        // do tema e o filamento vertical no meio da aresta esquerda.
        className={`group relative flex shrink-0 items-center gap-2 overflow-hidden rounded-[6px] text-[13px] transition-colors ${
          showDetails ? "h-[3.75rem]" : "h-[33px]"
        } ${open ? "px-2.5" : "justify-center px-0"} ${
          isActive
            ? `text-tyba-text ${w.color ? "" : "bg-tyba-raised shadow-[var(--tyba-raised-ring)]"}`
            : "text-tyba-text-muted hover:bg-tyba-text/[.03] hover:text-tyba-text"
        }`}
      >
        {isActive && (
          // O fio pega só o meio da aresta: de ponta a ponta pesava mais que o
          // próprio cartão.
          <span
            className="absolute left-0 top-1/2 h-6 w-0.5 -translate-y-1/2 rounded-r-full"
            style={{
              background: w.color
                ? `var(--tyba-${w.color})`
                : "var(--tyba-filament-v)",
            }}
          />
        )}
        <span className="relative shrink-0">
          {isWtView ? (
            <SquaresFour
              size={16}
              className={
                isActive
                  ? "shrink-0 text-tyba-text [filter:drop-shadow(0_0_6px_rgba(255,255,255,.2))]"
                  : "shrink-0"
              }
            />
          ) : isConfig ? (
            <GearSix
              size={16}
              className={
                isActive
                  ? "shrink-0 text-tyba-text [filter:drop-shadow(0_0_6px_rgba(255,255,255,.2))]"
                  : "shrink-0"
              }
            />
          ) : w.kind === "docker" ? (
            <DockerIcon
              size={16}
              style={w.color ? { color: `var(--tyba-${w.color})` } : undefined}
              className={
                isActive
                  ? `shrink-0 ${w.color ? "" : "text-tyba-cyan"} [filter:drop-shadow(0_0_6px_rgba(45,212,191,.35))]`
                  : "shrink-0"
              }
            />
          ) : hoverAgent ? (
            agentGlyph(hoverAgent)
          ) : (
            <TerminalWindow
              size={16}
              style={w.color ? { color: `var(--tyba-${w.color})` } : undefined}
              className={
                isActive
                  ? `shrink-0 ${w.color ? "" : "text-tyba-green"} [filter:drop-shadow(0_0_6px_rgba(124,197,68,.35))]`
                  : "shrink-0"
              }
            />
          )}
          {agentStatus && (
            <span
              aria-hidden
              title={t(agentStatus.visual.labelKey)}
              className={`absolute -right-0.5 -top-0.5 size-1.5 rounded-full ${agentStatus.visual.dotClass}`}
            />
          )}
        </span>
        {open && (
          <>
            <span className="flex min-w-0 flex-1 flex-col items-start gap-[3px]">
              <span
                className={`w-full truncate text-left leading-tight ${
                  isConfig || isWtView ? "" : "text-[13px]"
                }`}
              >
                {isConfig
                  ? t("settings")
                  : isWtView
                    ? t("workspaceView")
                    : displayName}
              </span>
              {showDetails && (
                <span className="w-full truncate text-left font-mono text-[10px] leading-none text-tyba-text-faint">
                  {sshHost
                    ? sshHost
                    : displayDir
                      ? compactPath(displayDir)
                      : "~"}
                </span>
              )}
              {showDetails && (
                <span className="flex w-full items-center gap-1.5">
                  {agentStatus && agentDetail ? (
                    <span className="flex min-w-0 items-center gap-1">
                      <span
                        className={`size-1 shrink-0 rounded-full ${agentStatus.visual.dotClass}`}
                      />
                      <span
                        className={`min-w-0 truncate font-mono text-[11px] leading-none ${agentStatus.visual.textClass}`}
                      >
                        {agentDetail}
                      </span>
                    </span>
                  ) : runningCmd ? (
                    <span className="min-w-0 truncate font-mono text-[11px] leading-none text-tyba-text-muted">
                      {runningCmd}
                    </span>
                  ) : null}
                  {/* A branch cede a linha quando há agente ou comando: os
                      dois lado a lado truncavam e nenhum ficava legível. */}
                  {branch && !agentDetail && !runningCmd && (
                    <span
                      title={branch}
                      className="flex min-w-0 items-center gap-0.5 font-mono text-[11px] leading-none text-tyba-text-faint"
                    >
                      <GitBranch size={10} className="shrink-0" />
                      <span className="truncate">{branch}</span>
                    </span>
                  )}
                  {(gitStatus?.dirty || chips.remoteChanged != null) && (
                    <span
                      title={
                        gitStatus?.dirty
                          ? t("gitChanges", { count: gitStatus.changed })
                          : t("toolbarRemoteDiff")
                      }
                      className="flex shrink-0 items-center gap-1"
                    >
                      <span className="size-1.5 shrink-0 rounded-full bg-tyba-amber-fill" />
                      {gitStatus?.dirty ? (
                        <DiffStat status={gitStatus} size="lg" />
                      ) : (
                        // O servidor responde só quantos arquivos mudaram
                        // (regra 24): o número sozinho é o que o `DiffStat`
                        // desenha quando não há linhas a mostrar.
                        <span className="font-mono text-[11px] leading-none text-tyba-amber">
                          {chips.remoteChanged}
                        </span>
                      )}
                    </span>
                  )}
                </span>
              )}
            </span>
            {!isConfig && !isWtView && w.tabs.length > 0 && (
              <span
                title={t("sessionPanes", { count: w.tabs.length })}
                className="shrink-0 font-mono text-[10px] text-tyba-text-faint"
              >
                {w.tabs.length}
              </span>
            )}
            {!isConfig && !isWtView && (
              <span
                role="img"
                aria-label={t(SESSION_DOT_LABEL_KEY[dot])}
                title={t(SESSION_DOT_LABEL_KEY[dot])}
                className={`size-[7px] shrink-0 rounded-full ${SESSION_DOT_CLASS[dot]} ${
                  menuWorkspace === w.id ? "hidden" : "group-hover:hidden"
                }`}
              />
            )}
            {!isConfig && (
            <DropdownMenu
              onOpenChange={(o) => setMenuWorkspace(o ? w.id : null)}
            >
              <DropdownMenuTrigger asChild>
                <span
                  role="button"
                  aria-label={t("sessionOptions")}
                  onClick={(e) => e.stopPropagation()}
                  className={`rounded-[3px] text-tyba-text-faint hover:text-tyba-text ${
                    menuWorkspace === w.id ? "" : "hidden group-hover:block"
                  }`}
                >
                  <DotsThree size={14} weight="bold" />
                </span>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="w-52">
                {renderWorkspaceMenuItems(w, branch, DROPDOWN_MENU_PARTS)}
              </DropdownMenuContent>
            </DropdownMenu>
            )}
          </>
        )}
      </button>
    );
    if (isConfig) return workspaceButton;
    const sideDiff =
      open && w.side_view?.startsWith("diff:") ? w.side_view : null;
    return (
      <div key={w.id} className="flex shrink-0 flex-col">
        <HoverCard>
          <ContextMenu
            onOpenChange={(o) => setMenuWorkspace(o ? w.id : null)}
          >
            <ContextMenuTrigger asChild>
              <HoverCardTrigger asChild>{workspaceButton}</HoverCardTrigger>
            </ContextMenuTrigger>
            <ContextMenuContent className="w-52">
              {renderWorkspaceMenuItems(w, branch, CONTEXT_MENU_PARTS)}
            </ContextMenuContent>
          </ContextMenu>
          {menuWorkspace !== w.id && (
            <SessionHoverCard
              name={displayName}
              path={sshHost ?? workspaceCwd(w) ?? w.repo_root}
              branch={branch}
              status={gitStatus}
              runner={hoverAgent}
              runnerIcon={hoverAgent ? agentGlyph(hoverAgent, 11) : null}
              runningCommand={runningCmd}
              agentVisual={agentStatus?.visual ?? null}
              tabs={w.tabs.length}
              group={w.group}
              color={w.color}
            />
          )}
        </HoverCard>
        {sideDiff && (
          <div className="group/diff flex h-7 shrink-0 items-center gap-2 rounded-[4px] pl-8 pr-2 text-[12px] text-tyba-text-faint transition-colors hover:bg-tyba-text/[.03] hover:text-tyba-text-muted">
            <button
              onClick={() => void activateWorkspace(w.id)}
              className="flex h-full min-w-0 flex-1 items-center gap-1.5"
            >
              <GitDiff size={13} className="shrink-0" />
              <span className="min-w-0 flex-1 truncate text-left">
                {t("diffPaneLabel", {
                  title: sessionById.get(sideDiff.slice(5))?.title ?? "?",
                })}
              </span>
            </button>
            <span
              role="button"
              aria-label={t("diffPaneClose")}
              onClick={() => void closeSideView(w.id).catch(() => {})}
              className="rounded-[3px] opacity-0 transition-opacity hover:text-tyba-text group-hover/diff:opacity-100"
            >
              <X size={11} weight="bold" />
            </span>
          </div>
        )}
      </div>
    );
  };

  return (
    <TooltipProvider delayDuration={400}>
      <ErrorBoundary region="notificações">
        <ToastHost />
        <ConfirmHost />
        <UpdateToast
          status={update}
          onDismiss={() => {
            if (!update) return;
            void updateDismiss(update.info.version).catch(() => {});
            setUpdate({ ...update, dismissed: true });
          }}
        />
        <NotificationToaster
          sessions={sessions}
          activeSessionId={activeId}
          agentReadyWarnings={agentReadyWarnings}
          hiddenApprovalIds={approvalHiddenIds}
          viewportRightPx={
            agentsSideNarrow ? AGENTS_PANEL_WIDTH_PX + 16 : undefined
          }
          onDismissAgentReady={(sessionId) =>
            setAgentReadyWarnings((prev) => {
              const next = { ...prev };
              delete next[sessionId];
              return next;
            })
          }
          onGoToSession={goToSession}
        />
      </ErrorBoundary>
      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        mode={paletteMode}
        onModeChange={setPaletteMode}
        searchFiles={searchFilesForPalette}
        onOpenFile={openFileFromFinder}
        workspaces={layout.workspaces}
        activeWorkspace={layout.active_workspace}
        bindings={bindings}
        theme={theme}
        onChangeTheme={changeTheme}
        onNewSession={() => openNewSession()}
        onNewWorktreeSession={() => openNewSession(true)}
        onNewTab={() => void newTab()}
        onCloseActive={() => void closeActivePane()}
        onOpenSettings={() => void openViewTab("settings").catch(() => {})}
        onOpenAgentsBoard={() =>
          void openViewTab("agent-board").catch(() => {})
        }
        onOpenStats={() => void openViewTab("stats").catch(() => {})}
        onTogglePanel={toggleSidebar}
        onOpenFiles={() => {
          if (fileSessionId) void openFilesPanel(fileSessionId).catch(() => {});
        }}
        onOpenGitHistory={
          activeId && activeGitStatus
            ? () => void openHistoryTab(activeId).catch(() => {})
            : undefined
        }
        onGoToWorkspace={(id) => void activateWorkspace(id)}
        launchConfigs={launchConfigs}
        onApplyLaunchConfig={applyLaunchConfigById}
        onNewLaunchConfig={newLaunchConfig}
        onSaveWorkspaceAsLaunchConfig={saveWorkspaceAsLaunchConfig}
        historyScope={historyScope}
        onPickHistory={injectIntoActive}
        onPickSnippet={pickSnippet}
      />
      <LaunchConfigDialog
        draft={launchDraft}
        onClose={() => setLaunchDraft(null)}
        onSaved={refreshLaunchConfigs}
      />
      <HostFormDialog
        state={hostEditor}
        groups={hostGroups}
        onClose={() => setHostEditor(null)}
        onSaved={(saved) => {
          setHostEditor(null);
          setSshHosts((prev) => prev.map((h) => (h.id === saved.id ? saved : h)));
        }}
      />
      <PasteConfirmDialog
        text={pastePrompt?.text ?? null}
        onCancel={() => setPastePrompt(null)}
        onConfirm={confirmPaste}
      />
      {snippetPrompt && (
        <SnippetArgsDialog
          snippet={snippetPrompt.snippet}
          placeholders={snippetPrompt.placeholders}
          onCancel={() => setSnippetPrompt(null)}
          onConfirm={confirmSnippet}
        />
      )}
      <HostPicker
        open={hostPickerOpen}
        onOpenChange={setHostPickerOpen}
        onPick={(host) => void connectToHost(host)}
      />
      <BroadcastConfirmDialog
        open={broadcastAsk !== null}
        command={broadcastAsk ?? ""}
        targets={broadcastSet.length}
        onConfirm={() => void submitBroadcast(true).catch(() => {})}
        onCancel={() => setBroadcastAsk(null)}
      />
      <NewSessionPrompt
        onConnectHost={(host) => void connectToHost(host)}
        open={newSessionOpen}
        onOpenChange={(open) => {
          setNewSessionOpen(open);
          if (open) setNewSessionIsolate(worktreeDefault);
          if (!open) setPendingGroup(null);
        }}
        isolate={newSessionIsolate}
        onIsolateChange={setNewSessionIsolate}
        onCreate={(cwd, name, isolate, shell) => {
          if (isolate && cwd) {
            setWorktreeDir(cwd);
            return;
          }
          void newSession(cwd, name, pendingGroup, undefined, shell ?? undefined);
        }}
      />
      <WorktreeCreateDialog
        dir={worktreeDir}
        onClose={() => {
          setWorktreeDir(null);
          setPendingGroup(null);
        }}
        onCreate={async (task, agent) => {
          const dir = worktreeDir;
          if (dir) {
            if (agent) {
              await newAgentSession(
                dir,
                task,
                pendingGroup,
                agent.prompt,
                agent.runner,
              );
            } else {
              await newSession(dir, task, pendingGroup, task);
            }
          }
          setWorktreeDir(null);
          setPendingGroup(null);
        }}
      />
      <PromptDialog
        open={prompt !== null}
        onOpenChange={(o) => {
          if (!o) setPrompt(null);
        }}
        title={prompt?.kind === "group" ? t("groupSession") : t("renameSession")}
        placeholder={
          prompt?.kind === "group" ? t("groupPlaceholder") : undefined
        }
        initial={
          prompt?.kind === "group"
            ? (prompt.ws.group ?? "")
            : (prompt?.ws.name ?? "")
        }
        onSubmit={(value) => {
          if (!prompt) return;
          if (prompt.kind === "rename") {
            if (value) void renameWorkspace(prompt.ws.id, value);
          } else {
            void setWorkspaceGroup(prompt.ws.id, value || null);
          }
          setPrompt(null);
        }}
      />
      <div className="tyba-aurora flex h-screen flex-col text-tyba-text">
        <WindowResizeEdges />
        <header
          data-tauri-drag-region
          // Sem vidro: o header é transparente sobre o canvas, e é por ele que a
          // aurora tripla aparece no topo da janela (mockup Proposta/PropostaClaro).
          className={`tyba-lift-b flex h-[38px] shrink-0 items-center gap-1 pr-2.5 ${
            IS_MAC ? "pl-20" : "pl-2.5"
          }`}
        >
          {/* Marca + wordmark. Sem eventos: o clique cai no header, que
              arrasta a janela. */}
          <span className="pointer-events-none mr-2 flex shrink-0 items-center gap-2">
            <TybaMark size={18} />
            <span className="font-sans text-[13px] font-bold tracking-[0.02em] text-tyba-text">
              TYBA
            </span>
          </span>

          <IconAction
            label={t("panelToggle")}
            shortcut={bindings.panel}
            onClick={toggleSidebar}
          >
            <SidebarSimple size={16} />
          </IconAction>

          <IconAction
            label={t("commandPalette")}
            shortcut={bindings.paletteActions}
            onClick={() => openPalette("actions")}
          >
            <MagnifyingGlass size={16} />
          </IconAction>

          <div className="pointer-events-none absolute inset-x-0 top-0 flex h-[38px] items-center justify-center">
            <div className="pointer-events-auto flex items-center gap-0.5 rounded-[8px] bg-[var(--tyba-segment-bg)] p-[3px] shadow-[var(--tyba-edge)]">
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    aria-label={t("terminalView")}
                    aria-pressed
                    className="flex h-6 items-center gap-1.5 rounded-[6px] bg-tyba-raised px-3 text-[13px] text-tyba-text shadow-[var(--tyba-row-raised)]"
                  >
                    <TerminalWindow size={14} aria-hidden />
                    {t("terminalView")}
                  </button>
                </TooltipTrigger>
                <TooltipContent side="bottom">
                  {t("terminalView")}
                </TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    aria-label={`${t("workspaceView")} — ${t("comingSoon")}`}
                    aria-disabled
                    disabled
                    className="flex h-6 cursor-not-allowed items-center gap-1.5 rounded-[6px] px-3 text-[13px] text-tyba-text-faint"
                  >
                    <Stack size={14} aria-hidden />
                    {t("workspaceView")}
                  </button>
                </TooltipTrigger>
                <TooltipContent side="bottom">
                  {t("comingSoon")}
                </TooltipContent>
              </Tooltip>
            </div>
          </div>

          <div className="h-full flex-1" data-tauri-drag-region />

          <Clock />

          <SidePanelBar
            tabs={panelTabs}
            lit={
              activeContent?.kind === "session-diff"
                ? "diff"
                : litPanel(sideView, fileSessionId)
            }
            onClick={onPanelClick}
            onEscape={() => {
              if (activeWorkspace) {
                void closeSideView(activeWorkspace.id).catch(() => {});
              }
            }}
          />

          <ErrorBoundary region="forge">
            <ForgePanel
              sessionId={activeId}
              repoRoot={activeGitStatus?.root ?? null}
            />
          </ErrorBoundary>

          {/* A pasta do header abre a tela de Arquivos da sessão (pedido do dono,
              2026-10-14); "Abrir pasta do projeto" fica no menu, na paleta e no
              atalho. */}
          <IconAction
            label={t("filesPanel")}
            shortcut={bindings.files}
            onClick={() => {
              if (fileSessionId) void openFilesPanel(fileSessionId).catch(() => {});
            }}
          >
            <FolderOpen size={16} />
          </IconAction>

          <ErrorBoundary region="notificações">
            <NotificationsInbox
              sessions={sessions}
              approvals={approvals}
              open={inboxOpen}
              onOpenChange={setInboxOpen}
              onGoToSession={goToSession}
            />
          </ErrorBoundary>

          {/* Divisória antes da conta (mockup): 1px × 18px, a linha fina. */}
          <span aria-hidden className="mx-1.5 h-[18px] w-px bg-tyba-border" />

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                aria-label={t("account")}
                className="ml-1 rounded-full p-px"
                style={{ background: "var(--tyba-gradient)" }}
              >
                <span className="flex size-5 items-center justify-center rounded-full bg-tyba-raised text-tyba-text-muted">
                  <User size={11} weight="bold" />
                </span>
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              align="end"
              className="w-56"
            >
              <DropdownMenuLabel className="flex min-w-0 flex-col">
                <span className="truncate text-xs font-medium text-tyba-text">
                  {accountName || t("localAccount")}
                </span>
                {accountName && (
                  <span className="truncate text-[11px] font-normal text-tyba-text-faint">
                    {t("localAccount")}
                  </span>
                )}
              </DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                <DropdownMenuItem
                  className="text-xs"
                  onSelect={() => void openViewTab("settings").catch(() => {})}
                >
                  <GearSix size={16} className="opacity-60" />
                  {t("settings")}
                </DropdownMenuItem>
                <DropdownMenuItem
                  className="text-xs"
                  onSelect={() => openPalette("actions")}
                >
                  <MagnifyingGlass size={16} className="opacity-60" />
                  {t("commandPalette")}
                </DropdownMenuItem>
                <DropdownMenuItem
                  className="text-xs"
                  onSelect={() => setShortcutsOpen(true)}
                >
                  <Keyboard size={16} className="opacity-60" />
                  {t("shortcuts")}
                </DropdownMenuItem>
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>

          <WindowControls />
        </header>

        <div className="flex min-h-0 flex-1">
          <>
            {sidebar !== "hidden" && (
                <aside
                  // `tyba-divide-r`: a sidebar nunca teve borda direita. Ela se
                  // separava do terminal por um degrau de fundo — que existe no
                  // `github-dark` e some no `tyba-dark`, onde `surface` e
                  // `sunken` distam 5/255. E acima do terminal ela encosta na
                  // faixa de abas, que é `surface` também: mesma cor, zero
                  // separação, em TODO tema.
                  className="tyba-lift-r tyba-glass flex shrink-0 flex-col"
                  style={{ width: SIDEBAR_WIDTH[sidebar] }}
                >
                  {open && (
                    <label className="mx-2 mt-3 flex h-[31px] items-center gap-1.5 rounded-[6px] bg-tyba-raised px-2.5 shadow-[var(--tyba-raised-ring)] focus-within:shadow-[inset_0_0_0_1px_var(--tyba-border-strong)]">
                      <MagnifyingGlass
                        size={12}
                        className="shrink-0 text-tyba-text-faint"
                      />
                      <input
                        spellCheck={false}
                        autoCapitalize="off"
                        autoCorrect="off"
                        value={sessionQuery}
                        onChange={(e) => setSessionQuery(e.target.value)}
                        placeholder={t("searchSessions")}
                        className="w-full bg-transparent text-[13px] text-tyba-text outline-none placeholder:text-tyba-text-faint"
                      />
                    </label>
                  )}
                  <nav
                    className="tyba-scroll-fade mt-3 flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto px-2 pb-2"
                  >
                    {layout.workspaces.some(isConfigWorkspace) && (
                      <div className="mb-1 flex flex-col gap-px rounded-[6px] border border-tyba-border/70 bg-tyba-text/[.015] p-1">
                        {open && (
                          <span className="flex items-center gap-2 px-1.5 pt-1 pb-1.5">
                            <span className="text-[10px] font-medium uppercase tracking-[0.14em] text-tyba-text-faint">
                              {t("systemGroup")}
                            </span>
                            <span className="h-px min-w-0 flex-1 bg-tyba-border" />
                          </span>
                        )}
                        <Tooltip>
                        <TooltipTrigger asChild>
                        <button
                          onClick={() =>
                            void openViewTab("settings").catch(() => {})
                          }
                          aria-label={t("settings")}
                          className={`group relative flex h-8 shrink-0 items-center gap-2 rounded-[4px] text-[13px] transition-colors ${
                            open ? "px-2" : "justify-center px-0"
                          } ${
                            activeTab?.view === "settings"
                              ? "bg-tyba-text/[.05] text-tyba-text"
                              : "text-tyba-text-faint hover:bg-tyba-text/[.03] hover:text-tyba-text-muted"
                          }`}
                        >
                          <GearSix size={16} className="shrink-0" />
                          {open && (
                            <span className="min-w-0 flex-1 truncate text-left">
                              {t("settings")}
                            </span>
                          )}
                        </button>
                        </TooltipTrigger>
                        {!open && (
                          <TooltipContent side="right">
                            {t("settings")}
                          </TooltipContent>
                        )}
                        </Tooltip>
                      </div>
                    )}
                    {layout.workspaces.some(isWorktreesWorkspace) && (
                      <div className="mb-1 flex flex-col gap-px rounded-[6px] border border-tyba-border/70 bg-tyba-text/[.015] p-1">
                        {open && (
                          <span className="flex items-center gap-2 px-1.5 pt-1 pb-1.5">
                            <span className="text-[10px] font-medium uppercase tracking-[0.14em] text-tyba-text-faint">
                              {t("codeGroup")}
                            </span>
                            <span className="h-px min-w-0 flex-1 bg-tyba-border" />
                          </span>
                        )}
                        <button
                          disabled
                          aria-disabled
                          aria-label={`${t("worktreesTitle")} — ${t("comingSoon")}`}
                          title={t("comingSoon")}
                          className={`group relative flex h-8 shrink-0 cursor-not-allowed items-center gap-2 rounded-[4px] text-[13px] text-tyba-text-faint/50 ${
                            open ? "px-2" : "justify-center px-0"
                          }`}
                        >
                          <SquaresFour size={16} className="shrink-0" />
                          {open && (
                            <>
                              <span className="min-w-0 flex-1 truncate text-left">
                                {t("worktreesTitle")}
                              </span>
                              <span className="shrink-0 rounded-[3px] bg-tyba-text/[.06] px-1.5 py-0.5 text-[9px] uppercase tracking-wider">
                                {t("comingSoon")}
                              </span>
                            </>
                          )}
                        </button>
                      </div>
                    )}
                    {groupedWorkspaces.groups.map(([name, list]) => (
                      <div
                        key={name}
                        className="mb-1 flex flex-col gap-px rounded-[6px] border border-tyba-border/70 bg-tyba-text/[.015] p-1"
                      >
                        <span className="flex items-center gap-2 px-1.5 pt-1 pb-1.5">
                          <button
                            aria-label={name}
                            aria-expanded={!collapsedGroups[name]}
                            onClick={() =>
                              setCollapsedGroups((prev) => ({
                                ...prev,
                                [name]: !prev[name],
                              }))
                            }
                            className="flex min-w-0 items-center gap-1 text-tyba-text-faint transition-colors hover:text-tyba-text"
                          >
                            {collapsedGroups[name] ? (
                              <CaretRight size={9} weight="bold" />
                            ) : (
                              <CaretDown size={9} weight="bold" />
                            )}
                            <span className="truncate text-[10px] font-medium uppercase tracking-[0.14em]">
                              {name}
                            </span>
                          </button>
                          <span className="h-px min-w-0 flex-1 bg-tyba-border" />
                          <span className="font-mono text-[9px] text-tyba-text-faint">
                            {list.length}
                          </span>
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <button
                                aria-label={t("addToGroup")}
                                onClick={() => newSessionInGroup(name)}
                                className="rounded-[3px] text-tyba-text-faint transition-colors hover:text-tyba-text"
                              >
                                <Plus size={11} weight="bold" />
                              </button>
                            </TooltipTrigger>
                            <TooltipContent side="bottom">
                              {t("addToGroup")}
                            </TooltipContent>
                          </Tooltip>
                        </span>
                        {!collapsedGroups[name] && list.map(renderWorkspace)}
                      </div>
                    ))}
                    {groupedWorkspaces.loose
                      .filter(
                        (w) =>
                          !isConfigWorkspace(w) &&
                          !isWorktreesWorkspace(w) &&
                          !isConnectionsWorkspace(w) &&
                          !isStatsWorkspace(w) &&
                          !isAgentBoardWorkspace(w),
                      )
                      .map(renderWorkspace)}
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          variant="ghost"
                          onClick={() => openNewSession()}
                          aria-label={t("newSession")}
                          className={`h-[33px] shrink-0 gap-2 rounded-[6px] text-[13px] font-normal text-tyba-text-faint hover:bg-tyba-text/[.03] hover:text-tyba-text ${
                            open ? "justify-start px-2.5" : "justify-center px-0"
                          }`}
                        >
                          {/* Aberta, só o texto (mockup Proposta); o ícone fica
                              para o trilho recolhido, onde não há texto. */}
                          {open ? t("newSession") : <Plus size={14} />}
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent
                        side={open ? "bottom" : "right"}
                        className="flex items-center gap-2"
                      >
                        {t("newSession")}
                        <span className="font-mono text-tyba-text-faint">
                          {formatCombo(bindings.newSession)}
                        </span>
                      </TooltipContent>
                    </Tooltip>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          variant="ghost"
                          onClick={() =>
                            void openViewTab("connections").catch(() => {})
                          }
                          aria-label={t("connectionsTitle")}
                          className={`h-[33px] shrink-0 gap-2 rounded-[6px] text-[13px] font-normal ${
                            open ? "justify-start px-2.5" : "justify-center px-0"
                          } ${
                            activeTab?.view === "connections"
                              ? "bg-tyba-text/[.05] text-tyba-text"
                              : "text-tyba-text-faint hover:bg-tyba-text/[.03] hover:text-tyba-text"
                          }`}
                        >
                          {open ? t("connectionsTitle") : <HardDrives size={14} />}
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent side={open ? "bottom" : "right"}>
                        {t("connectionsTitle")}
                      </TooltipContent>
                    </Tooltip>
                  </nav>
                  {/* Sem divisa: quem marca a fronteira de rolagem é o fade
                      no `nav` acima. A linha aqui ficava 21px acima da borda da
                      linha de comando — duas pilhas ancoradas embaixo, com
                      alturas que não têm por que coincidir, então qualquer
                      valor escolhido hoje quebra quando uma delas mudar.
                      O fade resolve por deleção, e informa mais: ele só existe
                      quando há conteúdo escondido acima. */}
                  <div className="flex shrink-0 flex-col px-2 pt-1 pb-2">
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          variant="ghost"
                          onClick={openAgentsFromSidebar}
                          aria-label={t("agentsQueue")}
                          className={`relative h-[33px] shrink-0 gap-2 rounded-[6px] text-[13px] font-normal ${
                            open ? "justify-start px-2.5" : "justify-center px-0"
                          } ${
                            activeWorkspace?.side_view === "agent-queue" ||
                            activeWorkspace?.side_view?.startsWith("agents:")
                              ? "bg-tyba-text/[.05] text-tyba-text"
                              : "text-tyba-text hover:bg-tyba-text/[.03]"
                          }`}
                        >
                          {open ? t("agentsQueue") : <AgentIcon size={14} />}
                          {/* Pílula do mockup: conta APROVAÇÕES pendentes. Sem
                              nenhuma, mas com agente pedindo alguém, cai no
                              número de antes — a atenção não some da tela. */}
                          {(approvals.length > 0 || agentsWaiting > 0) && (
                            <span
                              className={
                                open
                                  ? "ml-auto rounded-[4px] bg-tyba-amber-tint px-1.5 text-[11px] text-tyba-amber"
                                  : "absolute right-1 top-1 size-1.5 rounded-full bg-tyba-amber-fill"
                              }
                            >
                              {open
                                ? approvals.length > 0
                                  ? t("agentsApprovals", {
                                      count: approvals.length,
                                    })
                                  : agentsWaiting
                                : null}
                            </span>
                          )}
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent side={open ? "bottom" : "right"}>
                        {t("agentsQueue")}
                      </TooltipContent>
                    </Tooltip>
                    {agentRows.length > 0 && (
                      <div className="mt-0.5 max-h-[40vh] min-h-0 overflow-y-auto">
                        <AgentsSidebar
                          rows={agentRows}
                          open={open}
                          onSelect={(sessionId, place) => {
                            jumpToAgent(sessionId, place);
                            // A linha da sessão também reabre a árvore dela,
                            // se ela tem subagentes: fechar nunca é sem volta.
                            if (
                              (subagentsBySession.get(sessionId)?.subagents
                                .length ?? 0) > 0
                            ) {
                              void openAgentsPanel(sessionId).catch(() => {});
                            }
                          }}
                        />
                      </div>
                    )}
                  </div>
                </aside>
              )}

              <main ref={mainAreaRef} className="relative flex min-h-0 min-w-0 flex-1 overflow-hidden">
                {gitScreenSession && activeWorkspace && (
                  <GitScreen
                    key={gitScreenSession.id}
                    session={gitScreenSession}
                    // O repo da PRÓPRIA sessão da tela (worktree, senão o cwd
                    // dela), não o do painel em foco: branch, ↑↓, Pull, push e
                    // o "esta branch" do CI têm de bater com ela.
                    repoRoot={gitScreenSession.worktree?.path ?? cwdKeyOf(gitScreenSession.id)}
                    snapshots={repoSnapshots}
                    exiting={gitMotion.exiting}
                    editor={editorPref}
                    suggestAgent={reviewAgent.trim() === "codex" ? "codex" : "claude"}
                    onClose={() => void closeSideView(activeWorkspace.id).catch(() => {})}
                    onFocusSession={goToSession}
                    findSession={(id) => sessionById.get(id)}
                    onSendToAgent={sendReviewToAgent}
                    onResolveConflicts={resolveConflictsWithAgent}
                    onOpenSession={adoptOpenedSession}
                  />
                )}
                {dockerScreenSession && activeWorkspace && (
                  <DockerScreen
                    key={dockerScreenSession.id}
                    session={dockerScreenSession}
                    workspaceId={activeWorkspace.id}
                    state={activeWorkspace.screens.docker}
                    host={dockerPanelHost}
                    repoRoot={
                      dockerScreenSession.id === activeId
                        ? (activeGitStatus?.root ?? null)
                        : null
                    }
                    exiting={dockerMotion.exiting}
                    // Sessão que a lista ainda não tem acabou de nascer, não morreu.
                    exitedOf={(id) => {
                      const s = sessionById.get(id);
                      return s ? isFinishedStatus(s.status) : false;
                    }}
                    onSessionsChanged={() => void refreshSessions()}
                    onClose={() => void closeSideView(activeWorkspace.id).catch(() => {})}
                  />
                )}
                {(filesScreenSession || screenEditors.length > 0) && (
                  <FilesScreen
                    open={filesTarget !== null}
                    exiting={filesMotion.exiting}
                    workspace={activeWorkspace ?? null}
                    session={filesScreenSession}
                    editors={screenEditors}
                    findSession={(id) => sessionById.get(id)}
                    snapshot={
                      filesScreenSession
                        ? snapshotForDir(
                            repoSnapshots,
                            filesScreenSession.worktree?.path ?? cwdKeyOf(filesScreenSession.id),
                          )
                        : undefined
                    }
                    editorPref={editorPref}
                    finderShortcut={formatCombo(bindings.filesFinder)}
                    rootLabelOf={(owner, wsId) => {
                      const ws = layout.workspaces.find((w) => w.id === wsId);
                      const root = ws?.repo_root ?? owner.worktree?.path ?? null;
                      return root ? baseName(root.replace(/[/\\]+$/, "")) : (ws?.name ?? "");
                    }}
                    onClose={() =>
                      activeWorkspace && void closeSideView(activeWorkspace.id).catch(() => {})
                    }
                    onOpenDiff={(sid) => void openDiffContentTab(sid).catch(() => {})}
                    // O terminal novo nasce debaixo da tela: fecha ela para ele aparecer.
                    onRunInTerminal={(command) => {
                      if (activeWorkspace) void closeSideView(activeWorkspace.id).catch(() => {});
                      void runInTerminal(command);
                    }}
                  />
                )}
                <div
                  // Sem `px-2`, de propósito. O respiro morava AQUI, e o que
                  // ele abria era uma faixa de 8px do fundo do app entre a
                  // sidebar e o terminal — medido no `github-dark`: `#161b22`
                  // até x=223, `#0d1117` de 224 a 231, `#010409` daí em diante.
                  // Três superfícies em oito pixels, sem uma linha sequer. Um
                  // vazamento, não uma margem.
                  // O respiro dos cartões continua existindo: ele é do
                  // `BlockList`, que tem o seu próprio `px-2` por dentro.
                  className={`min-h-0 min-w-0 flex-col ${
                    sideVisible
                      ? sideExpanded
                        ? "hidden"
                        : agentsSideNarrow
                          ? "flex flex-1"
                          : "flex shrink-0"
                      : "flex flex-1"
                  }`}
                  style={
                    sideVisible && !sideExpanded && !agentsSideNarrow
                      ? // O `minWidth` não é redundante com o clamp do arrasto:
                        // a razão é persistida, então uma janela redimensionada
                        // para menos — ou um valor gravado antes deste conserto
                        // — reproduziria o transbordo sem ninguém arrastar nada.
                        {
                          width: `${(1 - sideRatio) * 100}%`,
                          minWidth: MAIN_MIN_PX,
                        }
                      : undefined
                  }
                >
                {activeWorkspace &&
                  activeWorkspace.tabs.length > 0 &&
                  activeTab?.view !== "settings" &&
                  activeTab?.view !== "connections" &&
                  activeTab?.view !== "stats" &&
                  activeTab?.view !== "agent-board" && (
                  <TabBar
                    tabs={activeWorkspace.tabs}
                    activeTab={activeWorkspace.active_tab}
                    sessions={sessions}
                    cwds={sessionCwds}
                    onActivate={(id) => void activateTab(id)}
                    onClose={(id) => void closeTabAndRefresh(id)}
                    onNew={() => void newTab()}
                  />
                )}
                {containerStrip && (
                  <ContainerStrip
                    key={containerStrip.id}
                    containerId={containerStrip.id}
                    current={containerStrip.current}
                    onRestarted={() =>
                      setLogsRestart((prev) => ({
                        ...prev,
                        [containerStrip.id]: (prev[containerStrip.id] ?? 0) + 1,
                      }))
                    }
                  />
                )}
                {broadcastTargets.length > 1 && (
                  <BroadcastBar
                    targets={broadcastTargets}
                    enabled={broadcastOn}
                    selected={broadcastSet}
                    onToggle={setBroadcastOn}
                    onSelectedChange={setBroadcastSet}
                  />
                )}
                <div
                  ref={paneAreaRef}
                  className="relative min-h-0 flex-1 overflow-hidden"
                >
                  {searchOpen && activeId && (
                    <TerminalSearch
                      sessionId={activeId}
                      onClose={() => setSearchOpen(false)}
                    />
                  )}
                  {activeContent?.kind === "container-logs" && (
                    <ContainerLogsView
                      key={activeContent.containerId}
                      containerId={activeContent.containerId}
                      restartNonce={logsRestart[activeContent.containerId] ?? 0}
                    />
                  )}
                  {activeContent?.kind === "session-diff" &&
                    (sessionById.get(activeContent.session) ? (
                      <div className="absolute inset-0 flex">
                        <DiffView
                          key={activeContent.session}
                          session={sessionById.get(activeContent.session) as Session}
                          editor={editorPref}
                          suggestAgent={
                            reviewAgent.trim() === "codex" ? "codex" : "claude"
                          }
                          expanded
                          asTab
                          onToggleExpand={() => {}}
                          onClose={() =>
                            activeTab && void closeTabAndRefresh(activeTab.id)
                          }
                          onSendToAgent={(prompt) =>
                            sendReviewToAgent(
                              sessionById.get(activeContent.session) as Session,
                              prompt,
                            )
                          }
                          onResolveConflicts={(state) =>
                            resolveConflictsWithAgent(
                              sessionById.get(activeContent.session) as Session,
                              state,
                            )
                          }
                        />
                      </div>
                    ) : (
                      <div className="absolute inset-0 flex items-center justify-center text-[12px] text-tyba-text-faint">
                        {t("diffSessionGone")}
                      </div>
                    ))}
                  {activeContent?.kind === "pr-list" &&
                    (sessionById.get(activeContent.session) ? (
                      <PrListView
                        key={activeContent.session}
                        session={sessionById.get(activeContent.session) as Session}
                        onFocusSession={goToSession}
                        onOpenPr={(n) =>
                          void openGitScreen(activeContent.session, "prs", n).catch(() => {})
                        }
                      />
                    ) : (
                      <div className="absolute inset-0 flex items-center justify-center text-[12px] text-tyba-text-faint">
                        {t("diffSessionGone")}
                      </div>
                    ))}
                  {activeContent?.kind === "pr" &&
                    (sessionById.get(activeContent.session) ? (
                      <PrDetailView
                        key={`${activeContent.session}:${activeContent.number}`}
                        session={sessionById.get(activeContent.session) as Session}
                        number={activeContent.number}
                        onForward={async (sid, prompt) => {
                          const target = sessionById.get(sid);
                          if (target) await sendReviewToAgent(target, prompt);
                        }}
                        onOpenSession={adoptOpenedSession}
                      />
                    ) : (
                      <div className="absolute inset-0 flex items-center justify-center text-[12px] text-tyba-text-faint">
                        {t("diffSessionGone")}
                      </div>
                    ))}
                  {activeContent?.kind === "git-log" &&
                    (sessionById.get(activeContent.session) ? (
                      <GitHistoryView
                        key={activeContent.session}
                        session={sessionById.get(activeContent.session) as Session}
                      />
                    ) : (
                      <div className="absolute inset-0 flex items-center justify-center text-[12px] text-tyba-text-faint">
                        {t("diffSessionGone")}
                      </div>
                    ))}
                  {activeTab?.view === "connections" && (
                    <div className="absolute inset-0 flex">
                      <ConnectionsView
                        onConnect={(host) => void connectToHost(host)}
                        connectingHostIds={connectingHosts}
                        onConnectGroup={(group, hosts) =>
                          void connectGroup(group, hosts)
                        }
                      />
                    </div>
                  )}
                  {activeTab?.view === "stats" && (
                    <div className="absolute inset-0 flex">
                      <StatsView />
                    </div>
                  )}
                  {activeTab?.view === "agent-board" && (
                    <div className="absolute inset-0 flex">
                      <AgentsBoard
                        sessions={sessions}
                        layout={layout}
                        activeSessionId={activeId}
                        hostingBySession={hostingBySession}
                        nextAttentionCombo={bindings.nextAttentionSession}
                        onJump={jumpToAgent}
                        onNextAttention={goToNextAttention}
                      />
                    </div>
                  )}
                  {activeTab?.view === "workspace" && (
                    <div className="absolute inset-0 flex">
                      <WorktreesView
                        repoRoots={worktreeRepoRoots}
                        newWorktreeSessionCombo={bindings.newWorktreeSession}
                        onOpenSession={(path, name) =>
                          void newSession(path, name)
                        }
                        onFocusSession={goToSession}
                        onReviewSession={(id) =>
                          void openDiffContentTab(id).catch(() => {})
                        }
                      />
                    </div>
                  )}
                  {activeTab?.view === "settings" && (
                    <div className="absolute inset-0 flex">
                      <SettingsView
                        launchConfigs={launchConfigs}
                        onApplyLaunchConfig={applyLaunchConfigById}
                        onEditLaunchConfig={editLaunchConfig}
                        onDeleteLaunchConfig={removeLaunchConfig}
                        onNewLaunchConfig={newLaunchConfig}
                        onRefreshLaunchConfigs={refreshLaunchConfigs}
                        version={version}
                        update={update}
                        togglePref={togglePref}
                        onTogglePrefChange={changeTogglePref}
                        detailsPref={detailsPref}
                        onDetailsPrefChange={changeDetailsPref}
                        bindings={bindings}
                        onBindingsChange={changeBindings}
                        accountName={accountName}
                        onAccountNameChange={changeAccountName}
                        showContainers={showContainers}
                        onShowContainersChange={changeShowContainers}
                        showGitStatus={showGitStatus}
                        onShowGitStatusChange={changeShowGitStatus}
                        shellIntegration={shellIntegration}
                        onShellIntegrationChange={changeShellIntegration}
                        startup={startup}
                        onStartupChange={changeStartup}
                        toolbarPref={toolbarPref}
                        onToolbarPrefChange={changeToolbarPref}
                        worktreeDefault={worktreeDefault}
                        onWorktreeDefaultChange={changeWorktreeDefault}
                        richInputPref={richInputPref}
                        onRichInputPrefChange={(next) => {
                          void changeRichInputPref(next);
                        }}
                        richInputRegexInvalid={richInputRegexInvalid}
                        editor={editorPref}
                        onEditorChange={changeEditor}
                        reviewAgent={reviewAgent}
                        onReviewAgentChange={changeReviewAgent}
                      />
                    </div>
                  )}
                  {/* O painel É a área de terminal, e pinta como tal.
                      Antes dos terminais no DOM, portanto atrás de todos.

                      Sem isto, tudo que não é coberto pelo terminal nem pela
                      lista mostra o fundo do app, mais claro: a faixa entre o
                      último cartão e o bloco em execução, e a margem em volta
                      dele. Cada pedaço aparece como um degrau de cor. */}
                  {paneLayout?.panes.map((p) => (
                    <div
                      key={`pane-bg-${p.pane}`}
                      className="pointer-events-none bg-tyba-sunken"
                      style={{
                        position: "absolute",
                        left: `${p.x}%`,
                        top: `${p.y}%`,
                        width: `${p.w}%`,
                        height: `${p.h}%`,
                      }}
                    />
                  ))}
                  {sessions.map((s) => {
                    // Sem painel em aba nenhuma, sem terminal. Ver
                    // `pannedSessions`: a lista de sessões guarda as mortas, e
                    // construir um xterm para cada uma era o grosso do custo
                    // de abertura.
                    if (!pannedSessions.has(s.id)) return null;
                    const paneRect =
                      paneLayout?.panes.find((p) => p.session === s.id) ??
                      null;
                    // O terminal e os blocos medem em % da ÁREA DE CONTEÚDO do
                    // painel — o painel menos a linha do TYBA, que mora no
                    // rodapé dele. Ver o invólucro abaixo.
                    const pane = paneRect ? PANE_BOX : null;
                    const line = paneLines.get(s.id);
                    // Em modo prompt o terminal ocupa uma faixa FIXA embaixo: a
                    // lista de blocos a cobre quando ocioso e a revela quando um
                    // comando roda, sem nunca redimensionar o PTY. Alt-screen é
                    // a exceção — `vim` precisa do painel inteiro.
                    const blocked =
                      (promptModes[s.id] ?? false) &&
                      !(altScreens[s.id] ?? false);
                    const terminalBox =
                      pane && blocked ? termRect(pane) : pane;
                    const detected = detectedBySession.get(s.id) ?? null;
                    const sshHostId =
                      s.kind.type === "ssh" ? s.kind.host_id : null;
                    const hosting = detected?.hosting ?? false;
                    const jailed = detected?.jailed ?? false;
                    const shim = detected?.shim ?? null;
                    const notice = showShellAgentNotice(
                      s.kind,
                      detected,
                      hosting,
                      dismissedShellNotices.get(s.id),
                      shim,
                    );
                    const ungated = ungatedReason(
                      s.kind,
                      detected,
                      hosting,
                      shim,
                    );
                    const unjailedNotice = showUnjailedNotice(
                      s.kind,
                      detected,
                      hosting,
                      jailed,
                    );
                    const resumeInvite = showAgentResumeInvite(
                      s,
                      resumableAgents.get(s.id),
                      dismissedResumeInvites.has(s.id),
                    );
                    // A faixa da sessão remota: a linha que explica por que ela
                    // é comum, ou o agente rodando no servidor sem jaula. Uma
                    // só — as duas ocupam o mesmo lugar do pane.
                    const remoteNotice = sshHostId
                      ? paneNotice({
                          integration: integrations[s.id],
                          agent: remoteAgentNotice({
                            command: sessionCommands[s.id],
                            confirmed: remoteAgents[s.id],
                          }),
                        })
                      : null;
                    return (
                      // O painel é uma coluna: a área de conteúdo (terminal e
                      // blocos) em cima, a linha do TYBA embaixo. A altura da
                      // linha sai da área de conteúdo DESTE painel e de nenhum
                      // outro — um resize por painel, e só quando a linha muda
                      // de altura, nunca por comando.
                      <div
                        key={`pane-${s.id}`}
                        className="flex flex-col"
                        style={
                          paneRect
                            ? {
                                position: "absolute",
                                left: `${paneRect.x}%`,
                                top: `${paneRect.y}%`,
                                width: `${paneRect.w}%`,
                                height: `${paneRect.h}%`,
                              }
                            : { display: "none" }
                        }
                      >
                        {s.kind.type === "container" && (
                          <ContainerShellBanner containerId={s.kind.container_id} />
                        )}
                        <div className="relative min-h-0 flex-1">
                          <TerminalView
                            key={`${s.id}:${paneEpochs.get(s.id) ?? 0}`}
                            sessionId={s.id}
                            transport={transports[s.id] ?? INITIAL_TRANSPORT}
                            agentNotice={
                              notice && detected
                                ? { binary: agentBinaryName(detected.kind) }
                                : null
                            }
                            onReopenManaged={() =>
                              void reopenShellAgentManaged(s.id)
                            }
                            onDismissNotice={() => dismissShellAgentNotice(s.id)}
                            ungatedNotice={
                              ungated && detected
                                ? {
                                    binary: agentBinaryName(detected.kind),
                                    reason: ungated,
                                  }
                                : null
                            }
                            unjailedNotice={
                              unjailedNotice && detected
                                ? {
                                    binary: agentBinaryName(detected.kind),
                                    reason: detected.noJail,
                                  }
                                : null
                            }
                            remoteNotice={remoteNotice}
                            resumeNotice={
                              resumeInvite && s.kind.type === "agent"
                                ? { binary: agentBinaryName(s.kind.runner) }
                                : null
                            }
                            onResumeAgent={() =>
                              void resumeAgentConversation(s.id)
                            }
                            onDismissResume={() => dismissResumeInvite(s.id)}
                            shellResumeNotice={
                              s.kind.type === "shell" &&
                              Boolean(s.shell_resume_offer)
                            }
                            onResumeShell={() => void resumeShellClaude(s.id)}
                            onDismissShellResume={() =>
                              void dismissShellResumeOffer(s.id).catch(() => {})
                            }
                            authAlert={
                              runtimeAuthAlerts.has(s.id)
                                ? { kind: runtimeAuthAlerts.get(s.id)! }
                                : null
                            }
                            onDismissAuthAlert={() =>
                              dismissRuntimeAuthAlert(s.id)
                            }
                            approvalRequest={panelRequests.get(s.id) ?? null}
                            onPaste={deliverPaste}
                            onSearch={() => setSearchOpen(true)}
                            readOnly={line?.owns ?? false}
                            onReclaimFocus={() => focusLine(s.id)}
                            onAltScreen={(alt) =>
                              setAltScreens((prev) =>
                                prev[s.id] === alt
                                  ? prev
                                  : { ...prev, [s.id]: alt },
                              )
                            }
                            onSplit={(kind) => void splitActive(kind)}
                            visible={paneRect !== null}
                            focused={s.id === activeId}
                            reattaches={!ptyExitEndsSession(s.kind)}
                            connection={
                              s.kind.type === "ssh" ? s.connection : undefined
                            }
                            connectionFailure={
                              s.kind.type === "ssh" ? s.connection_failure : null
                            }
                            knownHostsCommand={
                              sshHostId ? knownHostsCommandFor(sshHostId) : null
                            }
                            onReconnect={
                              s.kind.type === "ssh"
                                ? () =>
                                    void reconnectSsh(s.id).catch((e) =>
                                      toastError(
                                        t("connectionsActionFailed"),
                                        translateError(e, t),
                                      ),
                                    )
                                : undefined
                            }
                            onEditHost={
                              sshHostId ? () => editSshHost(sshHostId) : undefined
                            }
                            onBroadcastInput={
                              broadcastOn && s.kind.type === "ssh"
                                ? handleBroadcastInput
                                : undefined
                            }
                            exited={isFinishedStatus(s.status)}
                            rect={terminalBox}
                            // Só com a faixa ABERTA. `liveUsed` guarda a última
                            // medida e não se apaga quando o comando termina: sem
                            // esta guarda o recorte fica valendo para sempre, e o
                            // terminal vira uma tira no rodapé assim que a lista
                            // deixa de cobri-lo — num split, o painel sem blocos.
                            liveUsed={
                              blocked && liveOf(s.id) ? liveUsed[s.id] : undefined
                            }
                            swallowArrows={swallowsArrow({
                              running: Boolean(sessionCommands[s.id]?.running),
                              lineEcho: lineEcho[s.id] ?? false,
                              altScreen: altScreens[s.id] ?? false,
                            })}
                            bindings={bindings}
                            // Quem lê o teclado neste painel é um composer de
                            // agente: a sessão gerenciada, ou o `claude` cru que
                            // o probe achou rodando no shell. É o que libera
                            // Shift/Ctrl/Option+Enter como quebra de linha.
                            agentTui={s.kind.type === "agent" || detected !== null}
                            onLineHeight={(px) => reportLineHeight(s.id, px)}
                            onCellWidth={(px) => reportCellWidth(s.id, px)}
                            onLiveRows={
                              blocked
                                ? (used, total, scrolled) =>
                                    reportLiveRows(s.id, used, total, scrolled)
                                : undefined
                            }
                            onFocus={
                              paneRect
                                ? () => void focusPane(paneRect.pane)
                                : undefined
                            }
                            onExit={() => void refreshSessions()}
                          />
                          {/* Nada de objeto ou arrow montados aqui
                              dentro: o corpo do painel é `memo`, e prop com
                              identidade nova a cada quadro faz a comparação
                              rasa falhar sempre. Ver `SessionBlocks`. */}
                          {blocked && paneRect && (
                            <SessionBlocks
                              key={`blocks-${s.id}`}
                              {...sessionBlocksData({
                                session: s,
                                pane: {
                                  pane: paneRect.pane,
                                  x: PANE_BOX.left,
                                  y: PANE_BOX.top,
                                  w: PANE_BOX.width,
                                  h: PANE_BOX.height,
                                },
                                blocks: blocks[s.id],
                                // `clear` não abre a faixa: ele não tem saída para
                                // mostrar, e meio painel preto que aparece para logo
                                // esvaziar tudo é um solavanco em cima de um comando
                                // cujo ponto é sumir com as coisas.
                                live: liveOf(s.id),
                                // A lista cede à faixa só a altura que a saída usa de
                                // fato. Sem isto ela larga metade do painel para um
                                // terminal que costuma estar em boa parte vazio, e o
                                // cartão nasce longe de onde a saída estava.
                                used: liveUsed[s.id],
                                headerPx: activeHeaderPx[s.id],
                                fontSizePx: termFontSize,
                                lineHeightPx:
                                  termLineHeight[s.id] ?? fallbackLineHeight,
                                cellWidthPx:
                                  termCellWidth[s.id] ?? fallbackCellWidth,
                                cwd: sessionCwds[s.id],
                                active: s.id === activeId,
                                command: sessionCommands[s.id]?.command,
                                commandStartedAtMs:
                                  sessionCommands[s.id]?.started_at_ms,
                                marked:
                                  blockPick?.session === s.id
                                    ? markedBlocks
                                    : null,
                                copyCombo: formatCombo(bindings.copy),
                              })}
                              onInject={injectInto}
                              onFocusPane={focusPane}
                              onPick={pickerFor(s.id)}
                              onClearPick={clearPick}
                              onHeaderPx={reportHeaderPx}
                            />
                          )}
                        </div>
                        {paneRect && line?.visible && (
                          // Clicar (ou focar) a linha de outro painel passa o
                          // foco do CORE para ele antes de qualquer tecla: é o
                          // core que decide o painel ativo, e o atalho, a
                          // paleta e a caixa de agente seguem essa resposta.
                          <div
                            className="shrink-0"
                            onPointerDownCapture={() => {
                              if (s.id !== activeId) {
                                void focusPane(paneRect.pane).catch(() => {});
                              }
                            }}
                            onFocusCapture={() => {
                              if (s.id !== activeId) {
                                void focusPane(paneRect.pane).catch(() => {});
                              }
                            }}
                          >
                            {/* `key` só pela sessão: trocar o foco NÃO remonta
                                a linha. Texto, caret, linhas pendentes e menu
                                de cada painel ficam onde estavam. */}
                            <CommandLine
                              key={`${s.id}:line`}
                              sessionId={s.id}
                              cwd={cwdKeyOf(s.id)}
                              program={programName(
                                sessionCommands[s.id]?.command,
                              )}
                              // A branch DESTA sessão, pela mesma regra do
                              // chip da barra (sem emprestar de outro repo).
                              // SSH fica sem: o snapshot é local.
                              branch={(() => {
                                if (s.kind.type === "ssh") return null;
                                const chip = toolbarBranchChip({
                                  session: {
                                    id: s.id,
                                    worktree: s.worktree?.path ?? null,
                                    alive: !isFinishedStatus(s.status),
                                    cwd: sessionCwds[s.id]?.canonical ?? null,
                                  },
                                  workspaceDir: null,
                                  snapshots: repoSnapshots,
                                });
                                return chip?.state === "known"
                                  ? chip.label
                                  : null;
                              })()}
                              scope={{
                                cwd: cwdKeyOf(s.id),
                                repoRoot: s.repo_root ?? null,
                              }}
                              focusOnMount={s.id === activeId && line.owns}
                              focusNonce={lineFocusNonces[s.id] ?? 0}
                              state={line.state}
                              boot={shellBoots[s.id] ?? null}
                              bootHost={
                                s.kind.type === "ssh"
                                  ? (sshHosts.find(
                                      (h) =>
                                        s.kind.type === "ssh" &&
                                        h.id === s.kind.host_id,
                                    )?.alias ?? null)
                                  : null
                              }
                              inject={injected[s.id] ?? null}
                              draft={drafts.current[s.id] ?? ""}
                              onDraftChange={rememberDraft}
                              pending={pendings.current[s.id]}
                              onPendingChange={rememberPending}
                            />
                          </div>
                        )}
                      </div>
                    );
                  })}
                  {/* Onde o foco está: o filamento de luz na aresta de cima
                      do painel ativo — a mesma linha viva (estática) da aba
                      ativa. "Estática = selecionado" é a regra da marca.

                      Já foi um contorno verde em volta do painel inteiro.
                      Num terminal BLACKOUT o retângulo aceso era o elemento
                      mais forte da tela, competindo com o que roda; o
                      filamento diz "é aqui" sem cercar.

                      Só o painel ATIVO ganha marca, e só com mais de um: o
                      divisor e o degrau de fundo já separam os outros.
                      `zIndex` acima da lista de blocos (`z-10`), que cobre
                      quase todo o painel. */}
                  {(paneLayout?.panes.length ?? 0) > 1 &&
                    paneLayout?.panes
                      .filter((p) => p.session === activeId)
                      .map((p) => (
                        <div
                          key={`pane-focus-${p.pane}`}
                          aria-hidden
                          className="tyba-flow-line tyba-flow-line--thick pointer-events-none"
                          style={{
                            position: "absolute",
                            left: `${p.x}%`,
                            top: `${p.y}%`,
                            width: `${p.w}%`,
                            zIndex: 30,
                          }}
                        />
                      ))}
                  {paneLayout?.agentViewers.map((v) => {
                    const owner = sessionById.get(v.session);
                    return (
                      <SubagentViewer
                        key={v.pane}
                        sessionId={v.session}
                        snapshot={subagentsBySession.get(v.session) ?? null}
                        sessionEnded={
                          owner ? isFinishedStatus(owner.status) : true
                        }
                        rect={{
                          left: v.x,
                          top: v.y,
                          width: v.w,
                          height: v.h,
                        }}
                        onClose={() => void closePane(v.pane).catch(() => {})}
                        onFocus={() => void focusPane(v.pane).catch(() => {})}
                      />
                    );
                  })}
                  {paneLayout?.dividers.map((d) => (
                    <div
                      key={d.split}
                      onPointerDown={startDividerDrag(d)}
                      className={`absolute z-10 flex items-center justify-center ${
                        d.kind === "v"
                          ? "w-[7px] -translate-x-1/2 cursor-col-resize"
                          : "h-[7px] -translate-y-1/2 cursor-row-resize"
                      }`}
                      style={
                        d.kind === "v"
                          ? {
                              left: `${d.at}%`,
                              top: `${d.crossStart}%`,
                              height: `${d.crossLength}%`,
                            }
                          : {
                              top: `${d.at}%`,
                              left: `${d.crossStart}%`,
                              width: `${d.crossLength}%`,
                            }
                      }
                    >
                      <span
                        // Mesmo peso do divisor do painel lateral: são a mesma
                        // peça de linguagem visual e destoar entre elas se lê
                        // como defeito.
                        className={`rounded-full bg-tyba-border-strong/60 transition-colors hover:bg-tyba-green/70 ${
                          d.kind === "v" ? "h-full w-px" : "h-px w-full"
                        }`}
                      />
                    </div>
                  ))}
                  {!activeTab && (
                    <div
                      className="absolute inset-0 flex flex-col items-center justify-center gap-5"
                      style={{
                        transform: `translateX(${-SIDEBAR_WIDTH[sidebar] / 2}px)`,
                      }}
                    >
                      <TerminalWindow
                        size={36}
                        className="text-tyba-text-faint"
                      />
                      <p className="text-sm text-tyba-text-faint">
                        {layout.workspaces.length === 0
                          ? t("noSessions")
                          : t("noTabs")}
                      </p>
                      <div className="flex w-64 flex-col gap-px">
                        {layout.workspaces.length === 0 ? (
                          <button
                            onClick={() => openNewSession()}
                            className="flex h-8 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] text-tyba-text-muted transition-colors hover:bg-tyba-text/[.04] hover:text-tyba-text"
                          >
                            <Plus size={14} className="text-tyba-green" />
                            <span className="flex-1 text-left">
                              {t("newSession")}
                            </span>
                            <Shortcut combo={bindings.newTab} />
                          </button>
                        ) : (
                          <button
                            onClick={() => void newTab()}
                            className="flex h-8 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] text-tyba-text-muted transition-colors hover:bg-tyba-text/[.04] hover:text-tyba-text"
                          >
                            <Plus size={14} className="text-tyba-green" />
                            <span className="flex-1 text-left">
                              {t("newTab")}
                            </span>
                            <Shortcut combo={bindings.newTab} />
                          </button>
                        )}
                        <button
                          onClick={() =>
                            sshHosts.length > 0
                              ? setHostPickerOpen(true)
                              : void openViewTab("connections").catch(() => {})
                          }
                          className="flex h-8 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] text-tyba-text-muted transition-colors hover:bg-tyba-text/[.04] hover:text-tyba-text"
                        >
                          <HardDrives size={14} className="text-tyba-cyan" />
                          <span className="flex-1 text-left">
                            {sshHosts.length > 0
                              ? t("connectSsh")
                              : t("connectionsTitle")}
                          </span>
                        </button>
                        <button
                          onClick={() => openPalette("actions")}
                          className="flex h-8 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] text-tyba-text-muted transition-colors hover:bg-tyba-text/[.04] hover:text-tyba-text"
                        >
                          <MagnifyingGlass size={14} />
                          <span className="flex-1 text-left">
                            {t("commandPalette")}
                          </span>
                          <Shortcut combo={bindings.paletteActions} />
                        </button>
                        <button
                          onClick={toggleSidebar}
                          className="flex h-8 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] text-tyba-text-muted transition-colors hover:bg-tyba-text/[.04] hover:text-tyba-text"
                        >
                          <SidebarSimple size={14} />
                          <span className="flex-1 text-left">
                            {t("togglePanel")}
                          </span>
                          <Shortcut combo={bindings.panel} />
                        </button>
                      </div>
                    </div>
                  )}
                </div>
                {activeSession && richInputVisible && !ownsCommandLine && (
                  <RichInput
                    key={activeSession.id}
                    sessionId={activeSession.id}
                    pref={richInputPref}
                    focusNonce={richInputFocusNonce}
                    openedExplicitly={richInputOpened.has(activeSession.id)}
                    prefill={launchPrefills[activeSession.id] ?? null}
                    onFocusChange={(focused) => {
                      richInputFocused.current = focused;
                    }}
                    onClose={() => closeRichInput(activeSession.id)}
                  />
                )}
                </div>
                {sideVisible && activeWorkspace && (
                  <>
                    {!sideExpanded && !agentsSideNarrow && (
                      <div
                        onPointerDown={startSideDrag}
                        className="z-10 flex w-[7px] shrink-0 cursor-col-resize items-stretch justify-center"
                      >
                        {/* Sem linha em repouso, de propósito: quem separa as
                            duas regiões é o degrau de fundo (o painel é
                            `surface`, o terminal é `sunken`), não um traço.
                            Enquanto a linha existia ela era a ÚNICA separação
                            da metade de baixo pra baixo — medido, os dois lados
                            davam rgb(22,19,19) idênticos ali —, e por isso
                            precisava do peso de régua para se sustentar.
                            A linha volta no hover, que é quando ela vira
                            affordance de arrastar em vez de moldura. */}
                        <span className="w-px bg-transparent transition-colors hover:bg-tyba-green/70" />
                      </div>
                    )}
                    <div
                      // `bg-tyba-surface` explícito é o que faz a separação
                      // existir: sem ele o degrau para o terminal só aparecia
                      // na metade de cima e sumia na de baixo, onde os dois
                      // lados mediam a mesma cor. Fundo constante é o que
                      // permite a linha do divisor recuar para o hover.
                      // Mockups Painel*: no escuro `#0a0a0b`, no claro o vidro
                      // a 62%, sempre com a linha fina à esquerda.
                      style={
                        sideExpanded
                          ? undefined
                          : agentsSideNarrow
                            ? { width: AGENTS_PANEL_WIDTH_PX }
                            : { minWidth: SIDE_MIN_PX }
                      }
                      className={`flex min-h-0 min-w-0 ${
                        agentsSideNarrow
                          ? "shrink-0 bg-[var(--tyba-panel-bg)] tyba-divide-l"
                          : "flex-1 bg-[var(--tyba-panel-bg)] tyba-divide-l"
                      }${
                        renderSideView?.startsWith("agents:")
                          ? agentsMotion.exiting
                            ? " tyba-panel-exit"
                            : " motion-safe:animate-tyba-panel-in"
                          : ""
                      }`}
                      role="complementary"
                      aria-label={t("sidePanelNav")}
                      onKeyDown={(e) => {
                        const target = e.target as HTMLElement;
                        // Evento de portal (popover, select) borbulha pela
                        // árvore React mas não está no DOM do painel: o Esc
                        // é dele, não do painel.
                        if (!e.currentTarget.contains(target)) return;
                        if (
                          escClosesPanel({
                            key: e.key,
                            defaultPrevented: e.defaultPrevented,
                            targetEditable:
                              target.isContentEditable ||
                              target.tagName === "INPUT" ||
                              target.tagName === "TEXTAREA" ||
                              target.tagName === "SELECT",
                          })
                        ) {
                          void closeSideView(activeWorkspace.id).catch(
                            () => {},
                          );
                        }
                      }}
                    >
                      {renderSideView?.startsWith("history:") ? (
                        historyPanelTarget ? (
                          <HistoryPanel
                            key={historyPanelTarget.id}
                            session={historyPanelTarget}
                            repoRoot={
                              historyPanelTarget.id === activeId
                                ? (activeGitStatus?.root ?? null)
                                : null
                            }
                            onInsert={(command) =>
                              injectInto(historyPanelTarget.id, command)
                            }
                            onClose={() =>
                              void closeSideView(activeWorkspace.id).catch(
                                () => {},
                              )
                            }
                          />
                        ) : (
                          <div className="flex flex-1 items-center justify-center text-[12px] text-tyba-text-faint">
                            {t("diffSessionGone")}
                          </div>
                        )
                      ) : renderSideView === "agent-queue" ? (
                        <AgentsQueue
                          sessions={sessions}
                          layout={layout}
                          approvals={approvals}
                          hostingBySession={hostingBySession}
                          onJump={jumpToAgent}
                          onReviewDiff={(id) =>
                            void openDiffContentTab(id).catch(() => {})
                          }
                          onClose={() =>
                            void closeSideView(activeWorkspace.id).catch(
                              () => {},
                            )
                          }
                        />
                      ) : renderSideView?.startsWith("agents:") ? (
                        agentsTarget ? (
                          <AgentsPanel
                            key={agentsTarget.id}
                            session={agentsTarget}
                            snapshot={
                              subagentsBySession.get(agentsTarget.id) ?? null
                            }
                            approvals={approvals}
                            ungated={agentsPanelUngated(
                              agentsTarget.kind,
                              hostingBySession.get(agentsTarget.id) ?? false,
                            )}
                            orchestratorTitle={
                              agentsTarget.kind.type === "shell"
                                ? (() => {
                                    const d = detectedBySession.get(
                                      agentsTarget.id,
                                    );
                                    return d ? agentBinaryName(d.kind) : null;
                                  })()
                                : null
                            }
                            runner={
                              agentsTarget.kind.type === "agent"
                                ? agentsTarget.kind.runner
                                : (detectedBySession.get(agentsTarget.id)
                                    ?.kind ?? null)
                            }
                            expanded={sideExpanded}
                            onToggleExpand={() =>
                              void setSideViewExpanded(
                                activeWorkspace.id,
                                !sideExpanded,
                              ).catch(() => {})
                            }
                            onClose={() =>
                              void closeSideView(activeWorkspace.id).catch(
                                () => {},
                              )
                            }
                            onSelect={(agentId) => {
                              // O detalhe abre inline no próprio painel (A5.3):
                              // nada de split de viewer ao lado.
                              void focusSubagent(
                                agentsTarget.id,
                                agentId,
                              ).catch(() => {});
                            }}
                          />
                        ) : (
                          <div className="flex flex-1 items-center justify-center text-[12px] text-tyba-text-faint">
                            {t("agentsSessionGone")}
                          </div>
                        )
                      ) : renderSideView?.startsWith("tunnels:") ? (
                        tunnelsTarget ? (
                          <TunnelsView
                            key={tunnelsTarget.id}
                            session={tunnelsTarget}
                            hostAlias={tunnelsHostAlias}
                            expanded={sideExpanded}
                            onToggleExpand={() =>
                              void setSideViewExpanded(
                                activeWorkspace.id,
                                !sideExpanded,
                              ).catch(() => {})
                            }
                            onClose={() =>
                              void closeSideView(activeWorkspace.id).catch(
                                () => {},
                              )
                            }
                          />
                        ) : (
                          <div className="flex flex-1 items-center justify-center text-[12px] text-tyba-text-faint">
                            {t("tunnelsSessionGone")}
                          </div>
                        )
                      ) : containerFilesTarget(renderSideView) ? (
                        <ContainerFilesPanel
                          key={containerFilesTarget(renderSideView) as string}
                          containerId={containerFilesTarget(renderSideView) as string}
                          onClose={() =>
                            void closeSideView(activeWorkspace.id).catch(() => {})
                          }
                        />
                      ) : sideTarget ? (
                        <DiffView
                          key={sideTarget.id}
                          session={sideTarget}
                          editor={editorPref}
                          suggestAgent={
                            reviewAgent.trim() === "codex" ? "codex" : "claude"
                          }
                          expanded={sideExpanded}
                          onToggleExpand={() =>
                            void setSideViewExpanded(
                              activeWorkspace.id,
                              !sideExpanded,
                            ).catch(() => {})
                          }
                          onClose={() =>
                            void closeSideView(activeWorkspace.id).catch(
                              () => {},
                            )
                          }
                          onSendToAgent={(prompt) =>
                            sendReviewToAgent(sideTarget, prompt)
                          }
                          onResolveConflicts={(state) =>
                            resolveConflictsWithAgent(sideTarget, state)
                          }
                        />
                      ) : (
                        <div className="flex flex-1 items-center justify-center text-[12px] text-tyba-text-faint">
                          {t("diffSessionGone")}
                        </div>
                      )}
                    </div>
                  </>
                )}
              </main>

              <ShortcutsPanel
                open={shortcutsOpen}
                bindings={bindings}
                onClose={() => setShortcutsOpen(false)}
              />
          </>
        </div>
        {/* Rodapé da JANELA, não da coluna do terminal. Enquanto ele morava
            dentro da coluna, a linha dele parava no meio da tela — aqui é uma
            só, de moldura a moldura.
            `!sideExpanded` não é detalhe: com o painel em tela cheia a coluna
            do terminal ganha `hidden`, e antes o rodapé sumia junto por ser
            filho dela. Solto aqui ele sobreviveria — empilhando uma segunda
            barra debaixo do rodapé do próprio painel, e pior, o botão de
            entrada rica continuaria montando o `RichInput` DENTRO da coluna
            escondida: nada aparece e o foco entra num `display: none`. */}
        {activeTab && activeWorkspace && !(sideVisible && sideExpanded) && (
          <Toolbar
            pref={toolbarPref}
            cwd={activeChips.cwd}
            branch={activeChips.branch}
            snapshot={activeChips.snapshot}
            remoteChanged={activeChips.remoteChanged}
            hasWorktree={Boolean(
              activeSession?.worktree ??
                worktreeSessionOf(activeWorkspace)?.worktree,
            )}
            onOpenDiff={() => {
              const target = activeSession?.worktree
                ? activeSession
                : (worktreeSessionOf(activeWorkspace) ?? activeSession);
              if (target) void openDiffContentTab(target.id).catch(() => {});
            }}
            showRichInput={richInputEligible && !richInputVisible}
            richInputCombo={bindings.richInput}
            onOpenRichInput={() => {
              if (activeId) openRichInput(activeId);
            }}
          />
        )}
      </div>
    </TooltipProvider>
  );
}
