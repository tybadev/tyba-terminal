import type { ElementType } from "react";
import { useCallback, useEffect, useMemo, useRef, useState,
  Fragment,
} from "react";
import { useTranslation } from "react-i18next";
import {
  CaretDown,
  CaretRight,
  Check,
  DotsThree,
  FolderOpen,
  GearSix,
  GitBranch,
  TreeStructure,
  TreeView,
  GitDiff,
  HardDrives,
  Keyboard,
  MagnifyingGlass,
  Plus,
  Prohibit,
  SidebarSimple,
  SquaresFour,
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
} from "./lib/shellAgentNotice";
import {
  paneRunningAgent,
  tabRunningAgent,
  workspaceRunningAgent,
} from "./lib/closeGuard";
import { pushToast, toastError } from "./lib/toast";
import {
  LaunchConfigDialog,
  type LaunchConfigDraftState,
} from "./components/LaunchConfigDialog";
import { ShortcutsPanel } from "./components/ShortcutsPanel";
import { ContainersView } from "./components/ContainersView";
import { ConnectionsView } from "./components/ConnectionsView";
import { HostPicker } from "./components/HostPicker";
import {
  BroadcastBar,
  BroadcastConfirmDialog,
  type BroadcastTarget,
} from "./components/BroadcastBar";
import { ptyExitEndsSession } from "./lib/sessionExit";
import { matchSshHost } from "./lib/sshCommand";
import { DockerIcon } from "./components/icons/DockerIcon";
import { NewSessionPrompt } from "./components/NewSessionPrompt";
import { WorktreeCreateDialog } from "./components/WorktreeCreateDialog";
import { WorktreesView } from "./components/WorktreesView";
import { DiffView } from "./components/DiffView";
import { TunnelsView } from "./components/TunnelsView";
import { FilesPanel } from "./components/FilesPanel";
import { AgentsPanel } from "./components/AgentsPanel";
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
  dockerOpenDashboard,
  getPref,
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
  onSessionStatus,
  filesSearch,
  type FileSearchResult,
  openDiffTab,
  openFilesPanel,
  openTunnelsPanel,
  openAgentsPanel,
  openSubagentViewer,
  openViewTab,
  paneSession,
  listSubagents,
  focusSubagent,
  onSubagentsChanged,
  detectedAgent,
  killShellAgent,
  type AgentRunner,
  onAgentDetected,
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
  type RepoSnapshot,
  type Session,
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
  connectHostGroup,
  reconnectSsh,
  listHosts,
  listHostGroups,
  tagWorkspace,
  appVersion,
  updateCheck,
  setAppMenu,
  onMenuAction,
  onBlockFinalized,
  onBlocksCleared,
  onSessionPromptMode,
  sessionBlocks,
  sessionLineEcho,
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
} from "./lib/ipc";
import { basename } from "@/lib/utils";
import { buildConflictPrompt } from "./lib/conflicts";
import {
  isFinishedStatus,
  sameSessionStatus,
  statusVisual,
  type StatusVisual,
} from "./lib/sessionStatus";
import {
  AGENTS_PANEL_LINGER_MS,
  agentsPanelRunConcluded,
  agentsPanelSession,
  agentsPanelUngated,
  deadAgentsPanels,
  showAgentsButton,
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
import { parseStartupMode } from "./lib/startup";
import {
  compactPath,
  resolveWorkspaceCwd,
  workspaceMatchDir,
} from "./lib/workspaceCwd";
import { findSessionLocation } from "./lib/sessionLocation";
import {
  computeRects,
  findAncestorSplit,
  type DividerRect,
} from "./lib/panes";
import {
  DEFAULT_TOOLBAR,
  parseToolbarPref,
  snapshotForDir,
  type ToolbarPref,
} from "./lib/repoSnapshots";
import { Toolbar } from "./components/Toolbar";
import {
  ActiveBlockFrame,
  ActiveBlockHeader,
  blocksRect,
  LIVE_DELAY_MS,
  liveRect,
  padSlackPx,
  termRect,
  usedFraction,
} from "./components/ActiveBlock";
import { LIVE_PAD_Y_PX } from "./components/TerminalView";

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
/** Piso do terminal. Sem ele o arrasto para o outro lado engole a área de
 *  trabalho, que é o problema simétrico e igualmente fácil de provocar. */
const MAIN_MIN_PX = 320;
import { BLOCK_GAP_PX, BlockList } from "./components/BlockList";
import { withEntry } from "./lib/perSession";
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
  keyboardOwner,
  swallowsArrow,
  lineState,
  PROMPT_MODE_PREF_KEY,
} from "./lib/commandLine";
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
          className="size-6 rounded-[4px] text-tyba-text-muted hover:text-tyba-text"
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

export default function App() {
  const { t, i18n } = useTranslation();
  const [sessions, setSessions] = useState<Session[]>([]);
  const [layout, setLayout] = useState<LayoutState>(EMPTY_LAYOUT);
  const [subagentsBySession, setSubagentsBySession] = useState<
    Map<SessionId, SubagentSnapshot>
  >(() => new Map());
  const [detectedBySession, setDetectedBySession] = useState<
    Map<SessionId, DetectedAgent>
  >(() => new Map());
  // F2/F3 do detectar-agente-no-shell: "Ignorar" esconde o aviso só pra
  // aquela instância de processo (pid+start) — um agente novo re-avisa.
  const [dismissedShellNotices, setDismissedShellNotices] = useState<
    Map<SessionId, string>
  >(() => new Map());
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
  const [fileOpenRequest, setFileOpenRequest] = useState<{
    id: string;
    path: string;
    nonce: number;
  } | null>(null);
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
  const [blocks, setBlocks] = useState<Record<string, Block[]>>({});
  // Lido por handler de clique e de tecla; num deles é `window`, e reassinar o
  // listener a cada bloco que chega seria trabalho por nada.
  const blocksRef = useRef<Record<string, Block[]>>({});
  blocksRef.current = blocks;
  const [snippetPrompt, setSnippetPrompt] = useState<{
    snippet: Snippet;
    placeholders: SnippetPlaceholder[];
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
  const [dockerRunning, setDockerRunning] = useState(false);
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
        setApprovals((prev) => prev.filter((p) => p.id !== id)),
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

  const searchFilesForPalette = useCallback(
    (query: string): Promise<FileSearchResult> =>
      activeId
        ? filesSearch(activeId, query)
        : Promise.resolve({ paths: [], truncated: false }),
    [activeId],
  );

  const openFileFromFinder = useCallback(
    (rel: string) => {
      if (!activeId) return;
      void openFilesPanel(activeId).catch(() => {});
      setFileOpenRequest({ id: activeId, path: rel, nonce: Date.now() });
    },
    [activeId],
  );

  const paneLayout = useMemo(
    () => (activeTab?.root ? computeRects(activeTab.root) : null),
    [activeTab],
  );

  const workspaces = useMemo(() => {
    const query = sessionQuery.trim().toLowerCase();
    if (!query) return layout.workspaces;
    return layout.workspaces.filter((w) =>
      `${w.name} ${w.repo_root ?? ""}`.toLowerCase().includes(query),
    );
  }, [layout.workspaces, sessionQuery]);

  const refreshSessions = useCallback(async () => {
    const all = await listSessions().catch(() => null);
    if (all) setSessions(all);
  }, []);

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
  useEffect(() => {
    void listHosts()
      .then(setSshHosts)
      .catch(() => {});
    void listHostGroups()
      .then(setHostGroups)
      .catch(() => {});
  }, [hostPickerOpen]);

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
  const sideVisible = Boolean(sideView) || agentsMotion.mounted;
  const renderSideView =
    sideView ??
    (agentsMotion.mounted && lastAgentsSessionRef.current
      ? `agents:${lastAgentsSessionRef.current}`
      : null);
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

  useEffect(() => {
    requestTerminalRelayout();
  }, [sideView, sideRatio, sideExpanded]);

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
        if (p.detected) return new Map(prev).set(p.session_id, p.detected);
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
        .then((d) => {
          if (!d) return;
          setDetectedBySession((prev) =>
            prev.has(s.id) ? prev : new Map(prev).set(s.id, d),
          );
        })
        .catch(() => probedDetection.current.delete(s.id));
    }
  }, [sessions]);

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
    async (title: string, cwd: string, runner?: AgentRunner) => {
      const fresh = await createSession({
        kind: { type: "agent", runner: runner ?? runnerFromCommand(reviewAgent) },
        cwd,
        title,
        attach_existing: true,
        cols: 100,
        rows: 30,
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
        try {
          cwd = await killShellAgent(sessionId);
        } catch (e) {
          toastError(t("shellAgentReopenFailed"), translateError(e, t));
          return;
        }
        try {
          const title = cwd.split("/").filter(Boolean).pop() ?? binary;
          const sid = await spawnAgentSession(title, cwd, detected.kind);
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
          if (
            !current ||
            (sameSessionStatus(current.status, session.status) &&
              current.attention === session.attention) ||
            (isFinishedStatus(current.status) &&
              !isFinishedStatus(session.status))
          ) {
            return prev;
          }
          return prev.map((c) =>
            c.id === id
              ? { ...c, status: session.status, attention: session.attention }
              : c,
          );
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
  const activeCommand = activeId ? sessionCommands[activeId] : undefined;
  const agentsButtonVisible =
    activeSession != null &&
    showAgentsButton(
      activeSession.kind,
      activeId != null && detectedBySession.has(activeId),
    );

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

  const workspaceAgentStatus = useCallback(
    (w: Workspace): { session: Session; visual: StatusVisual } | null => {
      let best: { session: Session; visual: StatusVisual } | null = null;
      for (const tab of w.tabs) {
        if (!tab.root) continue;
        for (const sid of leafSessions(tab.root)) {
          const session = sessionById.get(sid);
          if (!session || session.kind.type !== "agent") continue;
          const visual = statusVisual(session.status, session.attention);
          if (visual && (!best || visual.rank > best.visual.rank)) {
            best = { session, visual };
          }
        }
      }
      return best;
    },
    [sessionById],
  );

  const titleBase =
    activeWorkspace &&
    !isConfigWorkspace(activeWorkspace) &&
    !isWorktreesWorkspace(activeWorkspace) &&
    !isConnectionsWorkspace(activeWorkspace)
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

  // O painel Docker segue a conexão vigente. Abrir o painel troca o workspace
  // ativo (ele tem o seu), então o alvo é lembrado do último workspace de
  // trabalho — as views (docker/settings/conexões) não mexem no contexto.
  const [dockerSshHost, setDockerSshHost] = useState<string | null>(null);
  useEffect(() => {
    if (!activeWorkspace) return;
    if (
      activeWorkspace.kind === "docker" ||
      isConfigWorkspace(activeWorkspace) ||
      isConnectionsWorkspace(activeWorkspace) ||
      isWorktreesWorkspace(activeWorkspace)
    ) {
      return;
    }
    const id = workspaceSshHostId(activeWorkspace);
    setDockerSshHost(
      id ? (sshHosts.find((h) => h.id === id)?.alias ?? null) : null,
    );
  }, [activeWorkspace, workspaceSshHostId, sshHosts]);

  // O painel de containers é um só e o alvo dele muda: o workspace do Docker
  // segue o que está na tela, senão a sidebar diz "Docker" enquanto a lista
  // mostra a VPS — e um `sh` na máquina errada é caro.
  const tagDockerWorkspace = useCallback(
    (alias: string | null) => {
      const ws = layout.workspaces.find((w) => w.kind === "docker");
      if (!ws) return;
      const host = alias ? sshHosts.find((h) => h.alias === alias) : undefined;
      const group = host?.group_id
        ? (hostGroups.find((g) => g.id === host.group_id)?.name ?? null)
        : null;
      const name = host ? `${host.alias} · Docker` : "Docker";
      if (ws.name === name && ws.color === (host?.color ?? null)) return;
      void tagWorkspace(ws.id, name, {
        lock_name: true,
        color: host?.color ?? null,
        group,
      }).catch(() => {});
    },
    [layout.workspaces, sshHosts, hostGroups],
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

  const connectToHost = useCallback(async (host: Host) => {
    const session = await createSession({
      kind: { type: "ssh", host_id: host.id },
      cols: 100,
      rows: 30,
    });
    setSessions((prev) => [...prev, session]);
    try {
      const group = host.group_id
        ? ((await listHostGroups()).find((g) => g.id === host.group_id)?.name ??
          null)
        : null;
      await createWorkspace(host.alias, null, session.id, {
        lock_name: true,
        color: host.color,
        group,
      });
    } catch {
      void disposeSession(session.id).catch(() => {});
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

  const closeActivePane = useCallback(async () => {
    if (!activeTab) return;
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
  }, [activeTab, sessionById, confirmCloseWithRunningAgent, refreshSessions]);

  const closeTabAndRefresh = useCallback(
    async (id: string) => {
      const tab = layout.workspaces
        .flatMap((w) => w.tabs)
        .find((tt) => tt.id === id);
      const running = tab ? tabRunningAgent(tab, sessionById) : null;
      if (!(await confirmCloseWithRunningAgent(running))) return;
      await closeTab(id);
      await refreshSessions();
    },
    [layout.workspaces, sessionById, confirmCloseWithRunningAgent, refreshSessions],
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
          setDockerRunning(false);
          return;
        }
        const list = await dockerListContainers(null, true);
        if (!cancelled) {
          setDockerRunning(list.some((c) => c.state === "running"));
        }
      } catch {
        if (!cancelled) {
          setDockerUp(false);
          setDockerRunning(false);
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
      const [
        existing,
        currentLayout,
        togglePrefRaw,
        detailsRaw,
        overridesRaw,
        nameRaw,
        bindingsRaw,
        fontRaw,
        containersRaw,
        gitStatusRaw,
        shellIntegrationRaw,
        startupRaw,
        toolbarRaw,
        richInputRaw,
        editorRaw,
        worktreeDefaultRaw,
        reviewAgentRaw,
        promptModeRaw,
      ] = await Promise.all([
        listSessions().catch(() => [] as Session[]),
        layoutState().catch(() => EMPTY_LAYOUT),
        getPref(TOGGLE_PREF_KEY).catch(() => null),
        getPref(DETAILS_PREF_KEY).catch(() => null),
        getPref(DETAILS_OVERRIDES_KEY).catch(() => null),
        getPref(ACCOUNT_NAME_KEY).catch(() => null),
        getPref(BINDINGS_PREF_KEY).catch(() => null),
        getPref(FONT_SIZE_KEY).catch(() => null),
        getPref(SHOW_CONTAINERS_KEY).catch(() => null),
        getPref(GIT_STATUS_KEY).catch(() => null),
        getPref(SHELL_INTEGRATION_KEY).catch(() => null),
        getPref(STARTUP_KEY).catch(() => null),
        getPref(TOOLBAR_PREF_KEY).catch(() => null),
        getPref(RICH_INPUT_PREF_KEY).catch(() => null),
        getPref(EDITOR_PREF_KEY).catch(() => null),
        getPref(WORKTREE_DEFAULT_KEY).catch(() => null),
        getPref(REVIEW_AGENT_KEY).catch(() => null),
        getPref(PROMPT_MODE_PREF_KEY).catch(() => null),
      ]);
      if (cancelled) return;
      setPromptModePref(promptModeRaw === "on");
      setSessions(existing);
      setLayout(currentLayout);
      if (togglePrefRaw === "rail" || togglePrefRaw === "hidden") {
        setTogglePref(togglePrefRaw);
      }
      if (detailsRaw === "on" || detailsRaw === "off") {
        setDetailsPref(detailsRaw);
      }
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
      if (currentLayout.workspaces.length === 0 && !booted.current) {
        booted.current = true;
        setNewSessionOpen(true);
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refreshSessions]);

  // Texto que a linha do TYBA recebeu de fora — paste, histórico, snippet.
  // Declarado aqui, e não junto do resto da linha lá embaixo, porque o
  // `deliverPaste` precisa dele.
  const [injected, setInjected] = useState<{
    text: string;
    nonce: number;
  } | null>(null);
  // A linha do TYBA está no comando do teclado? Ref pelo mesmo motivo que
  // `promptModesRef` existe: `ownsCommandLine` só é calculado bem abaixo, e uma
  // dependência de `useCallback` seria lida antes de existir.
  const ownsCommandLineRef = useRef(false);

  const deliverPaste = useCallback(
    (sessionId: SessionId, raw: string) => {
      const entry = getTerm(sessionId);
      if (!entry) return;
      // A dona do teclado é a linha da sessão ATIVA. Colar num painel vizinho
      // continua indo para o terminal dele, que não está somente-leitura.
      const route = routePaste({
        raw,
        ownsCommandLine: sessionId === activeId && ownsCommandLineRef.current,
        bracketed: entry.term.modes.bracketedPasteMode,
      });
      if (!route) return;
      if (route.to === "line") {
        setInjected({ text: route.text, nonce: Date.now() });
        return;
      }
      if (route.to === "terminal") {
        entry.term.paste(route.text);
        return;
      }
      setPastePrompt({ sessionId, text: route.text });
    },
    [activeId],
  );

  const pasteFromClipboard = useCallback(async () => {
    if (!activeId) return;
    let text = "";
    try {
      text = await readClipboardText();
    } catch {
      return;
    }
    deliverPaste(activeId, text);
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
      if (
        pastePrompt.sessionId === activeId &&
        ownsCommandLineRef.current
      ) {
        setInjected({ text, nonce: Date.now() });
        setPastePrompt(null);
        return;
      }
      entry?.term.paste(text);
      entry?.term.focus();
      setPastePrompt(null);
    },
    [activeId, pastePrompt],
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
        if (activeId) void openFilesPanel(activeId).catch(() => {});
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
      } else if (action === "prevTab") {
        cycleTab(-1);
      } else if (action === "nextTab") {
        cycleTab(1);
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
  const promptMode = activeId ? (promptModes[activeId] ?? false) : false;
  // O shell só reporta o modo no primeiro prompt — depois de carregar rc,
  // framework e plugins. Até lá a linha aparece desabilitada em vez de a tela
  // ficar em branco sem lugar nenhum para digitar.
  // A linha aparece sempre que a sessão é um shell em modo prompt — inclusive
  // com comando rodando ou app de tela cheia aberto. Ela só muda de estado.
  const lineVisible =
    activeId != null &&
    activeSession?.kind.type === "shell" &&
    (promptMode || promptModePref);

  const commandLineState = lineState({
    reported: activeId ? promptModes[activeId] : undefined,
    promptMode,
    kind: activeSession?.kind,
    altScreen: activeId ? (altScreens[activeId] ?? false) : false,
    command: activeCommand,
    integrated: promptMode,
  });
  const ownsCommandLine =
    keyboardOwner({
      promptMode,
      kind: activeSession?.kind,
      altScreen: activeId ? (altScreens[activeId] ?? false) : false,
      command: activeCommand,
      integrated: promptMode,
    }) === "tybaLine";

  ownsCommandLineRef.current = ownsCommandLine;

  // Voltar do vim ou do fim de um comando devolve o foco para a linha.
  const [commandLineNonce, setCommandLineNonce] = useState(0);
  useEffect(() => {
    if (ownsCommandLine) {
      setCommandLineNonce((n) => n + 1);
      return;
    }
    // Devolveu o teclado ao terminal (vim abriu, comando começou, modo
    // desligado): o foco vai junto, senão as teclas caem numa caixa que já não
    // é dona da linha.
    if (activeId) getTerm(activeId)?.term.focus();
  }, [ownsCommandLine, activeId]);

  // Quem responde se o PS1 saiu da tela é o SHELL, não o app: o hook relata por
  // `633;P` a cada prompt. Assumir pela preferência mentiria quando o hook não
  // subiu (shell sem integração, subshell, container).
  useEffect(() => {
    const ids = sessions.map((s) => s.id);
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
    }
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
    };
  }, [sessions]);

  // Histórico persistido + os que chegam agora. O bloco vem pronto do core: o
  // front não parseia saída, só desenha os spans.
  useEffect(() => {
    const ids = sessions.filter((s) => s.kind.type === "shell").map((s) => s.id);
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
  }, [sessions]);

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
    }),
    [activeCwdKey, activeSession?.repo_root],
  );

  // Histórico e snippet entram na linha pelo MESMO caminho do paste: bracketed
  // paste detectado, control chars sanitizados e confirmação quando é multilinha.
  // Nada é executado — quem aperta Enter é o dono.
  const injectIntoActive = useCallback(
    (text: string) => {
      if (!activeId || !text) return;
      // Sem decidir de novo para onde o texto vai: quem decide é o
      // `deliverPaste`, e duas cópias da regra divergiriam — foi assim que o
      // "Colar" do menu de contexto ficou inerte enquanto histórico e snippet
      // funcionavam.
      deliverPaste(activeId, text);
      if (!ownsCommandLine) getTerm(activeId)?.term.focus();
    },
    [activeId, deliverPaste, ownsCommandLine],
  );

  const pickSnippet = useCallback(
    (snippet: Snippet) => {
      void snippetPlaceholders(snippet.command)
        .then((found) => {
          if (found.length === 0) {
            return renderSnippet(snippet.id, snippet.command, []).then(
              injectIntoActive,
            );
          }
          setSnippetPrompt({ snippet, placeholders: found });
        })
        .catch((error) => toastError(t("snippetsError"), error));
    },
    [injectIntoActive, t],
  );

  const confirmSnippet = useCallback(
    (values: [string, string][]) => {
      const prompt = snippetPrompt;
      setSnippetPrompt(null);
      if (!prompt) return;
      void renderSnippet(prompt.snippet.id, prompt.snippet.command, values)
        .then(injectIntoActive)
        .catch((error) => toastError(t("snippetsError"), error));
    },
    [snippetPrompt, injectIntoActive, t],
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
            if (target) void openDiffTab(target.id).catch(() => {});
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
    const branch = snapshot?.branch ?? undefined;
    const gitStatus = showGitStatus
      ? (snapshot?.status ?? undefined)
      : undefined;
    const runner = isConfig || isWtView ? null : workspaceAgent(w);
    const runningCmd = isConfig || isWtView ? null : workspaceCommand(w);
    const hoverAgent = runner ?? agentFromCommand(runningCmd);
    const agentStatus = isConfig || isWtView ? null : workspaceAgentStatus(w);
    const agentDetail = (() => {
      if (!agentStatus) return null;
      const status = agentStatus.session.status;
      const label = t(agentStatus.visual.labelKey);
      if (status.state === "awaiting_input" && status.hint) {
        return `${label}: ${status.hint}`;
      }
      if (status.state === "failed") return `${label}: ${status.reason}`;
      if (status.state === "running") return runningCmd ?? label;
      return label;
    })();
    const turnEndedUnseen =
      agentStatus?.session.attention === true &&
      agentStatus.session.status.state === "idle";
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
        className={`group relative flex shrink-0 items-center gap-2 rounded-[4px] text-[13px] transition-colors ${
          showDetails ? "h-[3.75rem]" : "h-8"
        } ${open ? "px-2" : "justify-center px-0"} ${
          isActive
            ? `text-tyba-text ${w.color ? "" : "bg-tyba-text/[.05]"}`
            : "text-tyba-text-faint hover:bg-tyba-text/[.03] hover:text-tyba-text-muted"
        }`}
      >
        {isActive && (
          <span
            className="absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-full"
            style={{
              background: w.color
                ? `var(--tyba-${w.color})`
                : "var(--tyba-gradient-soft)",
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
            <span className="flex min-w-0 flex-1 flex-col items-start gap-1">
              <span className="flex w-full items-center gap-1.5">
                <span className="min-w-0 flex-1 truncate text-left leading-none">
                  {isConfig
                    ? t("settings")
                    : isWtView
                      ? t("workspaceView")
                      : displayName}
                </span>
                {turnEndedUnseen && (
                  <span
                    aria-hidden
                    title={t("sessionFinished")}
                    className="size-1.5 shrink-0 rounded-full bg-tyba-green"
                  />
                )}
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
                        className={`min-w-0 truncate font-mono text-[10px] leading-none ${agentStatus.visual.textClass}`}
                      >
                        {agentDetail}
                      </span>
                    </span>
                  ) : runningCmd ? (
                    <span className="flex min-w-0 items-center gap-1">
                      <span className="size-1 shrink-0 rounded-full bg-tyba-green [box-shadow:var(--tyba-glow-green)] motion-safe:animate-pulse" />
                      <span className="min-w-0 truncate font-mono text-[10px] leading-none text-tyba-text-muted">
                        {runningCmd}
                      </span>
                    </span>
                  ) : null}
                  {branch && (
                    <span
                      title={branch}
                      className="flex min-w-0 items-center gap-0.5 font-mono text-[10px] leading-none text-tyba-text-faint"
                    >
                      <GitBranch size={9} className="shrink-0" />
                      <span className="truncate">{branch}</span>
                    </span>
                  )}
                  {gitStatus?.dirty && (
                    <span
                      title={t("gitChanges", { count: gitStatus.changed })}
                      className="flex shrink-0 items-center gap-1 rounded-[3px] bg-tyba-amber-tint px-1 py-px"
                    >
                      <span className="size-1 shrink-0 rounded-full bg-tyba-amber" />
                      <DiffStat status={gitStatus} />
                    </span>
                  )}
                </span>
              )}
            </span>
            {!isConfig && (
              <span className="font-mono text-[10px] text-tyba-text-faint">
                {w.tabs.length > 0 ? w.tabs.length : ""}
              </span>
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
                  className="rounded-[3px] text-tyba-text-faint opacity-0 transition-opacity hover:text-tyba-text group-hover:opacity-100"
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
              path={workspaceCwd(w) ?? w.repo_root}
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
        onTogglePanel={toggleSidebar}
        onOpenFiles={() => {
          if (activeId) void openFilesPanel(activeId).catch(() => {});
        }}
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
          className={`tyba-glass tyba-divide-b flex h-9 shrink-0 items-center gap-1 pr-2.5 ${
            IS_MAC ? "pl-20" : "pl-2.5"
          }`}
        >
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

          <div className="pointer-events-none absolute inset-x-0 top-0 flex h-9 items-center justify-center">
            <div className="pointer-events-auto flex items-center gap-0.5 rounded-[5px] border border-tyba-border bg-tyba-text/[.02] p-0.5">
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    aria-label={t("terminalView")}
                    aria-pressed
                    className="flex h-6 items-center gap-1.5 rounded-[3px] bg-tyba-text/[.06] px-2 text-[11px] text-tyba-text"
                  >
                    <TerminalWindow size={14} />
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
                    className="flex h-6 cursor-not-allowed items-center gap-1.5 rounded-[3px] px-2 text-[11px] text-tyba-text-faint/50"
                  >
                    <SquaresFour size={14} />
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

          {showContainers && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("containers")}
                  onClick={() => void dockerOpenDashboard().catch(() => {})}
                  className={`relative size-6 rounded-[4px] ${
                    dockerUp
                      ? "text-tyba-text-muted hover:text-tyba-text"
                      : "text-tyba-text-faint"
                  } ${
                    activeWorkspace?.kind === "docker"
                      ? "bg-tyba-text/[.06] text-tyba-text"
                      : ""
                  }`}
                >
                  <DockerIcon size={16} />
                  {!dockerUp ? (
                    <span className="absolute right-0.5 top-0.5 size-1.5 rounded-full bg-tyba-red [box-shadow:var(--tyba-glow-red)]" />
                  ) : (
                    dockerRunning && (
                      <span className="absolute right-0.5 top-0.5 size-1.5 rounded-full bg-tyba-green [box-shadow:var(--tyba-glow-green)]" />
                    )
                  )}
                </Button>
              </TooltipTrigger>
              <TooltipContent side="bottom">
                {dockerUp ? t("containers") : t("dockerUnavailable")}
              </TooltipContent>
            </Tooltip>
          )}

          {activeId && activeSession?.kind.type === "ssh" && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("tunnelsAction")}
                  onClick={() =>
                    void openTunnelsPanel(activeId).catch(() => {})
                  }
                  className="size-6 rounded-[4px] text-tyba-text-faint hover:text-tyba-text"
                >
                  <TreeStructure size={16} />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="bottom">{t("tunnelsAction")}</TooltipContent>
            </Tooltip>
          )}

          {activeId && agentsButtonVisible && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("agentsAction")}
                  onClick={() => {
                    if (!activeId || !activeWorkspace) return;
                    if (sideView === `agents:${activeId}`) {
                      void closeSideView(activeWorkspace.id).catch(() => {});
                    } else {
                      void openAgentsPanel(activeId).catch(() => {});
                    }
                  }}
                  className={`size-6 rounded-[4px] ${
                    sideView === `agents:${activeId}`
                      ? "bg-tyba-text/[.06] text-tyba-text"
                      : "text-tyba-text-faint hover:text-tyba-text"
                  }`}
                >
                  <TreeStructure size={16} />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="bottom">{t("agentsAction")}</TooltipContent>
            </Tooltip>
          )}

          {activeId && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("filesPanel")}
                  onClick={() => {
                    if (!activeId || !activeWorkspace) return;
                    if (sideView === `files:${activeId}`) {
                      void closeSideView(activeWorkspace.id).catch(() => {});
                    } else {
                      void openFilesPanel(activeId).catch(() => {});
                    }
                  }}
                  className={`size-6 rounded-[4px] ${
                    sideView === `files:${activeId}`
                      ? "bg-tyba-text/[.06] text-tyba-text"
                      : "text-tyba-text-faint hover:text-tyba-text"
                  }`}
                >
                  <TreeView size={16} />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="bottom">
                {`${t("filesPanel")} (${formatCombo(bindings.files)})`}
              </TooltipContent>
            </Tooltip>
          )}

          {gitTone && activeId && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={t("gitStatusIconLabel")}
                  onClick={() => void openDiffTab(activeId).catch(() => {})}
                  className={`size-6 rounded-[4px] ${
                    gitTone === "dirty"
                      ? "text-tyba-amber hover:text-tyba-amber/80"
                      : "text-tyba-text-faint hover:text-tyba-text"
                  }`}
                >
                  <GitBranch
                    size={16}
                    weight={gitTone === "dirty" ? "fill" : "regular"}
                  />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="bottom">
                {t("gitStatusIconLabel")}
              </TooltipContent>
            </Tooltip>
          )}

          <ErrorBoundary region="forge">
            <ForgePanel
              sessionId={activeId}
              repoRoot={activeGitStatus?.root ?? null}
            />
          </ErrorBoundary>

          <IconAction
            label={t("openProjectFolder")}
            shortcut={bindings.openFolder}
            onClick={() => void openProjectFolder()}
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
                  className="tyba-glass flex shrink-0 flex-col"
                  style={{ width: SIDEBAR_WIDTH[sidebar] }}
                >
                  {open && (
                    <label className="mx-2 mt-3 flex h-7 items-center gap-1.5 rounded-[4px] bg-tyba-text/[.03] px-2 focus-within:bg-tyba-text/[.05]">
                      <MagnifyingGlass
                        size={12}
                        className="shrink-0 text-tyba-text-faint"
                      />
                      <input
                        value={sessionQuery}
                        onChange={(e) => setSessionQuery(e.target.value)}
                        placeholder={t("searchSessions")}
                        className="w-full bg-transparent text-[12px] text-tyba-text outline-none placeholder:text-tyba-text-faint"
                      />
                    </label>
                  )}
                  <nav
                    className={`flex min-h-0 flex-1 flex-col gap-px overflow-y-auto px-2 pb-2 ${
                      open ? "mt-2" : "mt-3"
                    }`}
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
                          {activeTab?.view === "settings" && (
                            <span
                              className="absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-full"
                              style={{ background: "var(--tyba-gradient-soft)" }}
                            />
                          )}
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
                          !isConnectionsWorkspace(w),
                      )
                      .map(renderWorkspace)}
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          variant="ghost"
                          onClick={() => openNewSession()}
                          aria-label={t("newSession")}
                          className={`mt-0.5 h-8 shrink-0 gap-2 rounded-[4px] text-[13px] font-normal text-tyba-text-faint hover:bg-tyba-text/[.03] hover:text-tyba-text ${
                            open ? "justify-start px-2" : "justify-center px-0"
                          }`}
                        >
                          <Plus size={14} />
                          {open && t("newSession")}
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent
                        side={open ? "bottom" : "right"}
                        className="flex items-center gap-2"
                      >
                        {t("newSession")}
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
                          className={`mt-0.5 h-8 shrink-0 gap-2 rounded-[4px] text-[13px] font-normal ${
                            open ? "justify-start px-2" : "justify-center px-0"
                          } ${
                            activeTab?.view === "connections"
                              ? "bg-tyba-text/[.05] text-tyba-text"
                              : "text-tyba-text-faint hover:bg-tyba-text/[.03] hover:text-tyba-text"
                          }`}
                        >
                          <HardDrives size={14} />
                          {open && t("connectionsTitle")}
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent side={open ? "bottom" : "right"}>
                        {t("connectionsTitle")}
                      </TooltipContent>
                    </Tooltip>
                  </nav>
                </aside>
              )}

              <main ref={mainAreaRef} className="flex min-h-0 min-w-0 flex-1">
                <div
                  className={`min-h-0 min-w-0 flex-col ${
                    sideVisible
                      ? sideExpanded
                        ? "hidden"
                        : "flex shrink-0"
                      : "flex flex-1"
                  }`}
                  style={
                    sideVisible && !sideExpanded
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
                  activeTab?.view !== "connections" && (
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
                  {activeTab?.view === "containers" && (
                    <div className="absolute inset-0 flex">
                      <ContainersView
                        sshHost={dockerSshHost}
                        onTargetChange={tagDockerWorkspace}
                        onAvailableChange={setDockerUp}
                        onRunningChange={setDockerRunning}
                      />
                    </div>
                  )}
                  {activeTab?.view === "connections" && (
                    <div className="absolute inset-0 flex">
                      <ConnectionsView
                        onConnect={(host) => void connectToHost(host)}
                        onConnectGroup={(group, hosts) =>
                          void connectGroup(group, hosts)
                        }
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
                          void openDiffTab(id).catch(() => {})
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
                    const paneRect =
                      paneLayout?.panes.find((p) => p.session === s.id) ??
                      null;
                    const pane = paneRect
                      ? {
                          left: paneRect.x,
                          top: paneRect.y,
                          width: paneRect.w,
                          height: paneRect.h,
                        }
                      : null;
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
                    const notice = showShellAgentNotice(
                      s.kind,
                      detected,
                      dismissedShellNotices.get(s.id),
                    );
                    return (
                      <TerminalView
                        key={s.id}
                        sessionId={s.id}
                        agentNotice={
                          notice && detected
                            ? { binary: agentBinaryName(detected.kind) }
                            : null
                        }
                        onReopenManaged={() =>
                          void reopenShellAgentManaged(s.id)
                        }
                        onDismissNotice={() => dismissShellAgentNotice(s.id)}
                        onPaste={deliverPaste}
                        onSearch={() => setSearchOpen(true)}
                        readOnly={s.id === activeId && ownsCommandLine}
                        onReclaimFocus={() => setCommandLineNonce((n) => n + 1)}
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
                        connecting={s.kind.type === "ssh"}
                        reattaches={!ptyExitEndsSession(s.kind)}
                        connection={
                          s.kind.type === "ssh" ? s.connection : undefined
                        }
                        onReconnect={
                          s.kind.type === "ssh"
                            ? () => void reconnectSsh(s.id)
                            : undefined
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
                    );
                  })}
                  {sessions.map((s) => {
                    const paneRect =
                      paneLayout?.panes.find((p) => p.session === s.id) ?? null;
                    const list = blocks[s.id] ?? [];
                    const running = sessionCommands[s.id];
                    const blocked =
                      paneRect !== null &&
                      (promptModes[s.id] ?? false) &&
                      !(altScreens[s.id] ?? false);
                    if (!blocked || !paneRect) return null;
                    const pane = {
                      left: paneRect.x,
                      top: paneRect.y,
                      width: paneRect.w,
                      height: paneRect.h,
                    };
                    // `clear` não abre a faixa: ele não tem saída para mostrar,
                    // e meio painel preto que aparece para logo esvaziar tudo é
                    // um solavanco em cima de um comando cujo ponto é sumir com
                    // as coisas.
                    const live = liveOf(s.id);
                    // A lista cede à faixa só a altura que a saída usa de fato.
                    // Sem isto ela larga metade do painel para um terminal que
                    // costuma estar em boa parte vazio, e o cartão nasce longe
                    // de onde a saída estava.
                    const used = liveUsed[s.id] ?? 1;
                    // A saída sobe além do que a conta em % diz, porque o
                    // recorte desconta o padding do terminal. Lista, header e
                    // moldura acompanham pelo mesmo tanto. Ver `padSlackPx`.
                    const lift = padSlackPx(LIVE_PAD_Y_PX, used);
                    return (
                      <Fragment key={`blocks-${s.id}`}>
                        {/* Sem `list.length > 0`: a lista é o que COBRE o
                            terminal, e o terminal em modo prompt é meia altura
                            do painel. Escondida enquanto não houvesse bloco, o
                            painel recém-aberto mostrava a caixa do xterm no
                            rodapé e vazio em cima — o "abre já menor" do split.
                            Vazia ela é um scroller com o cartão-zero dentro. */}
                        <BlockList
                          blocks={list}
                          rect={blocksRect(pane, live, used)}
                          bottomInset={
                            live
                              ? (activeHeaderPx[s.id] ?? 0) + lift + BLOCK_GAP_PX
                              : 0
                          }
                          fontSizePx={termFontSize}
                          lineHeightPx={
                            termLineHeight[s.id] ?? fallbackLineHeight
                          }
                          cellWidthPx={termCellWidth[s.id] ?? fallbackCellWidth}
                          opened={{
                            cwd:
                              sessionCwds[s.id]?.cwd ??
                              sessionCwds[s.id]?.canonical ??
                              null,
                            atMs: Date.parse(s.created_at) || null,
                          }}
                          onInject={
                            s.id === activeId ? injectIntoActive : undefined
                          }
                          onActivate={
                            s.id === activeId
                              ? undefined
                              : () => void focusPane(paneRect.pane)
                          }
                          marked={
                            blockPick?.session === s.id
                              ? (markedBlocks ?? undefined)
                              : undefined
                          }
                          onPick={(id, event) => pickBlock(s.id, id, event)}
                          onClearPick={() => setBlockPick(null)}
                          copyCombo={formatCombo(bindings.copy)}
                        />
                        {live && (
                          <>
                            <ActiveBlockHeader
                              command={running?.command ?? ""}
                              rect={liveRect(pane, used)}
                              liftPx={lift}
                              onHeight={(px) => reportHeaderPx(s.id, px)}
                            />
                            <ActiveBlockFrame
                              rect={liveRect(pane, used)}
                              liftPx={lift}
                            />
                          </>
                        )}
                      </Fragment>
                    );
                  })}
                  {/* A moldura do painel — do PAINEL, não do que está dentro.
                      Depois de tudo no DOM, e `pointer-events-none`.

                      Antes, quem desenhava contorno eram o terminal e a lista,
                      cada um o seu. Em modo prompt o terminal é meia altura: o
                      painel focado aparecia com a moldura só da metade para
                      baixo, e com dois contornos concorrentes dentro da mesma
                      caixa. Como camada de cima ela também não some atrás de um
                      terminal opaco — que é o caso de todo painel de agente. */}
                  {(paneLayout?.panes.length ?? 0) > 1 &&
                    paneLayout?.panes.map((p) => (
                      <div
                        key={`pane-frame-${p.pane}`}
                        className="pointer-events-none rounded-[4px] border border-tyba-border"
                        style={{
                          position: "absolute",
                          left: `${p.x}%`,
                          top: `${p.y}%`,
                          width: `${p.w}%`,
                          height: `${p.h}%`,
                          ...(p.session === activeId
                            ? {
                                borderColor:
                                  "color-mix(in srgb, var(--tyba-green) 45%, transparent)",
                                boxShadow:
                                  "0 0 0 1px color-mix(in srgb, var(--tyba-green) 25%, transparent), 0 0 14px -2px var(--tyba-glow-green, rgba(124,197,68,.4))",
                              }
                            : {}),
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
                {activeSession && lineVisible && (
                  <CommandLine
                    key={`${activeSession.id}:line`}
                    sessionId={activeSession.id}
                    cwd={activeCwdKey}
                    branch={activeGitStatus?.branch ?? null}
                    scope={historyScope}
                    focusNonce={commandLineNonce}
                    state={commandLineState}
                    inject={injected}
                  />
                )}
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
                {activeTab && activeWorkspace && (
                  <Toolbar
                    pref={toolbarPref}
                    cwd={workspaceCwd(activeWorkspace)}
                    snapshot={(() => {
                      const dir = workspaceGitDir(activeWorkspace);
                      return dir
                        ? snapshotForDir(repoSnapshots, dir)
                        : undefined;
                    })()}
                    hasWorktree={Boolean(
                      activeSession?.worktree ??
                        worktreeSessionOf(activeWorkspace)?.worktree,
                    )}
                    onOpenDiff={() => {
                      const target = activeSession?.worktree
                        ? activeSession
                        : (worktreeSessionOf(activeWorkspace) ?? activeSession);
                      if (target) void openDiffTab(target.id).catch(() => {});
                    }}
                    showRichInput={richInputEligible && !richInputVisible}
                    richInputCombo={bindings.richInput}
                    onOpenRichInput={() => {
                      if (activeId) openRichInput(activeId);
                    }}
                  />
                )}
                </div>
                {sideVisible && activeWorkspace && (
                  <>
                    {!sideExpanded && (
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
                      style={sideExpanded ? undefined : { minWidth: SIDE_MIN_PX }}
                      className={`flex min-h-0 min-w-0 flex-1 bg-tyba-surface${
                        renderSideView?.startsWith("agents:")
                          ? agentsMotion.exiting
                            ? " tyba-panel-exit"
                            : " motion-safe:animate-tyba-panel-in"
                          : ""
                      }`}
                    >
                      {renderSideView?.startsWith("agents:") ? (
                        agentsTarget ? (
                          <AgentsPanel
                            key={agentsTarget.id}
                            session={agentsTarget}
                            snapshot={
                              subagentsBySession.get(agentsTarget.id) ?? null
                            }
                            ungated={agentsPanelUngated(agentsTarget.kind)}
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
                              void focusSubagent(
                                agentsTarget.id,
                                agentId,
                              ).catch(() => {});
                              void openSubagentViewer(agentsTarget.id).catch(
                                () => {},
                              );
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
                      ) : renderSideView?.startsWith("files:") ? (
                        filesTarget ? (
                          <FilesPanel
                            key={filesTarget.id}
                            session={filesTarget}
                            editor={editorPref}
                            expanded={sideExpanded}
                            openRequest={
                              fileOpenRequest &&
                              fileOpenRequest.id === filesTarget.id
                                ? {
                                    path: fileOpenRequest.path,
                                    nonce: fileOpenRequest.nonce,
                                  }
                                : null
                            }
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
                            onJumpToDiff={() =>
                              void openDiffTab(filesTarget.id).catch(() => {})
                            }
                            onRunInTerminal={runInTerminal}
                          />
                        ) : (
                          <div className="flex flex-1 items-center justify-center text-[12px] text-tyba-text-faint">
                            {t("filesSessionGone")}
                          </div>
                        )
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
      </div>
    </TooltipProvider>
  );
}
