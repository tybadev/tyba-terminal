import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowRight,
  Check,
  DotsThree,
  FolderOpen,
  GearSix,
  GitBranch,
  Keyboard,
  MagnifyingGlass,
  Plus,
  Prohibit,
  Robot,
  SidebarSimple,
  SquaresFour,
  TerminalWindow,
  User,
  X,
} from "@phosphor-icons/react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

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
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { getThemeMode, onThemeModeChange, setThemeMode, type ThemeMode } from "./theme";
import { ApprovalsInbox } from "./components/ApprovalsInbox";
import {
  Notification,
  NotificationLink,
  NotificationTitle,
} from "@/components/ui/notification";
import { Clock } from "./components/Clock";
import { CommandPalette } from "./components/CommandPalette";
import { ShortcutsPanel } from "./components/ShortcutsPanel";
import { ContainersView } from "./components/ContainersView";
import { DockerIcon } from "./components/icons/DockerIcon";
import { NewSessionPrompt } from "./components/NewSessionPrompt";
import { PromptDialog } from "./components/PromptDialog";
import {
  SettingsView,
  type DetailsPref,
  type SidebarTogglePref,
} from "./components/SettingsView";
import { TabBar } from "./components/TabBar";
import {
  requestTerminalRelayout,
  setDefaultFontSize,
  TerminalView,
} from "./components/TerminalView";
import {
  activateTab,
  activateWorkspace,
  closePane,
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
  onApprovalRequested,
  onApprovalResolved,
  onLayoutChanged,
  openViewTab,
  paneSession,
  renameWorkspace,
  repoBranch,
  setPref,
  setSplitRatio,
  setWorkspaceColor,
  setWorkspaceGroup,
  splitPane,
  type ApprovalRequest,
  type LayoutState,
  type Session,
  type SessionKind,
  type SplitKind,
  type Workspace,
} from "./lib/ipc";
import { basename } from "@/lib/utils";
import {
  computeRects,
  findAncestorSplit,
  type DividerRect,
} from "./lib/panes";
import {
  captureState,
  comboOf,
  DEFAULT_BINDINGS,
  parseBindings,
  BINDINGS_PREF_KEY,
  type Bindings,
  type KeyAction,
} from "./lib/keys";
import { Shortcut } from "@/components/ui/kbd";

const EMPTY_LAYOUT: LayoutState = { workspaces: [], active_workspace: null };
const TOGGLE_PREF_KEY = "pref.sidebar_toggle";
const DETAILS_PREF_KEY = "pref.sidebar_details";
const DETAILS_OVERRIDES_KEY = "pref.session_details";
const ACCOUNT_NAME_KEY = "pref.account_name";
const FONT_SIZE_KEY = "pref.code.font_size";
const SHOW_CONTAINERS_KEY = "pref.code.show_containers";

function runnerLabel(kind: SessionKind): string | null {
  if (kind.type !== "agent") return null;
  if (kind.runner === "claude_code") return "claude";
  if (kind.runner === "codex") return "codex";
  return kind.runner.custom;
}

function isConfigWorkspace(w: Workspace): boolean {
  return w.tabs.length > 0 && w.tabs.every((t) => t.view === "settings");
}

function compactPath(dir: string): string {
  const home = dir.replace(/^\/Users\/[^/]+/, "~");
  const parts = home.split("/").filter(Boolean);
  if (home.length <= 34 || parts.length <= 3) return home;
  return `…/${parts.slice(-2).join("/")}`;
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
  const { t } = useTranslation();
  const [sessions, setSessions] = useState<Session[]>([]);
  const [layout, setLayout] = useState<LayoutState>(EMPTY_LAYOUT);
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
  const [paletteMode, setPaletteMode] = useState<"actions" | "sessions">(
    "actions",
  );
  const [newSessionOpen, setNewSessionOpen] = useState(false);
  const [branches, setBranches] = useState<Record<string, string>>({});
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

  const openPalette = useCallback((mode: "actions" | "sessions") => {
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

  const sessionIdsRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    sessionIdsRef.current = new Set(sessions.map((s) => s.id));
  }, [sessions]);

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

  const repoRootsKey = useMemo(() => {
    const roots = new Set<string>();
    for (const w of layout.workspaces) {
      if (w.repo_root) roots.add(w.repo_root);
    }
    return [...roots].sort().join("\n");
  }, [layout.workspaces]);

  useEffect(() => {
    let cancelled = false;
    const roots = repoRootsKey ? repoRootsKey.split("\n") : [];
    void Promise.all(
      roots.map(
        async (root) => [root, await repoBranch(root).catch(() => null)] as const,
      ),
    ).then((entries) => {
      if (cancelled) return;
      const next: Record<string, string> = {};
      for (const [root, branch] of entries) {
        if (branch) next[root] = branch;
      }
      setBranches(next);
    });
    return () => {
      cancelled = true;
    };
  }, [repoRootsKey]);

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

  const splitActive = useCallback(
    async (kind: SplitKind) => {
      if (!activeWorkspace || !activeTab?.active_pane) return;
      const session = await createSession({
        kind: { type: "shell" },
        cwd: activeWorkspace.repo_root ?? undefined,
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
    [activeWorkspace, activeTab],
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
    async (cwd: string | null, name: string, group?: string | null) => {
      const session = await createSession({
        kind: { type: "shell" },
        cwd: cwd ?? undefined,
        cols: 100,
        rows: 30,
      });
      setSessions((prev) => [...prev, session]);
      try {
        const workspaceId = await createWorkspace(name, cwd, session.id);
        if (group) await setWorkspaceGroup(workspaceId, group);
      } catch {
        void disposeSession(session.id).catch(() => {});
      }
    },
    [],
  );

  const newSessionInGroup = useCallback((group: string) => {
    setPendingGroup(group);
    setNewSessionOpen(true);
  }, []);

  const newTab = useCallback(async () => {
    if (!activeWorkspace) {
      setNewSessionOpen(true);
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
    } catch {
      void disposeSession(session.id).catch(() => {});
    }
  }, [activeWorkspace]);

  const openProjectFolder = useCallback(async () => {
    const dir = await openFileDialog({ directory: true, multiple: false });
    if (typeof dir !== "string") return;
    void setPref("pref.last_session_dir", dir).catch(() => {});
    await newSession(dir, basename(dir));
  }, [newSession]);

  const killWorkspace = useCallback(
    async (id: string) => {
      await closeWorkspace(id);
      await refreshSessions();
    },
    [refreshSessions],
  );

  const closeActivePane = useCallback(async () => {
    if (!activeTab) return;
    if (activeTab.active_pane) {
      await closePane(activeTab.active_pane);
    } else {
      await closeTab(activeTab.id);
    }
    await refreshSessions();
  }, [activeTab, refreshSessions]);

  const closeTabAndRefresh = useCallback(
    async (id: string) => {
      await closeTab(id);
      await refreshSessions();
    },
    [refreshSessions],
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
      ]);
      if (cancelled) return;
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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (captureState.active) return;
      const combo = comboOf(e);
      if (!combo) return;
      const action = (Object.keys(bindings) as KeyAction[]).find(
        (a) => bindings[a] === combo,
      );
      if (action) {
        e.preventDefault();
        if (e.repeat) return;
        if (action === "paletteActions") {
          openPalette("actions");
        } else if (action === "paletteSessions") {
          openPalette("sessions");
        } else if (action === "panel") {
          toggleSidebar();
        } else if (action === "newTab") {
          void newTab();
        } else if (action === "closePane") {
          void closeActivePane();
        } else if (action === "openFolder") {
          void openProjectFolder();
        } else if (action === "newSession") {
          setNewSessionOpen(true);
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
        }
        return;
      }
      if (e.metaKey && e.ctrlKey && !e.shiftKey && !e.altKey) {
        const key = e.key.toLowerCase();
        if (key === "arrowleft" || key === "arrowright") {
          e.preventDefault();
          resizeActivePane("v", key === "arrowright" ? 0.05 : -0.05);
          return;
        }
        if (key === "arrowup" || key === "arrowdown") {
          e.preventDefault();
          resizeActivePane("h", key === "arrowdown" ? 0.05 : -0.05);
          return;
        }
      }
      if (
        e.metaKey &&
        !e.repeat &&
        !e.shiftKey &&
        !e.altKey &&
        !e.ctrlKey &&
        e.key >= "1" &&
        e.key <= "9"
      ) {
        const target = activeWorkspace?.tabs[Number(e.key) - 1];
        if (target) {
          e.preventDefault();
          void activateTab(target.id);
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [
    bindings,
    newTab,
    closeActivePane,
    toggleSidebar,
    openProjectFolder,
    cycleWorkspace,
    cycleTab,
    focusPaneInDirection,
    toggleSettings,
    splitActive,
    cyclePane,
    resizeActivePane,
    activeWorkspace,
  ]);

  const open = sidebar === "open";

  const renderWorkspace = (w: Workspace) => {
    const isActive = w.id === layout.active_workspace;
    const isConfig = isConfigWorkspace(w);
    const showDetails = open && detailsFor(w.id) && !isConfig;
    const agent = showDetails ? workspaceAgent(w) : null;
    const branch = w.repo_root ? branches[w.repo_root] : undefined;
    return (
      <button
        key={w.id}
        onClick={() => void activateWorkspace(w.id)}
        title={open ? (w.repo_root ?? undefined) : w.name}
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
          showDetails ? "h-12" : "h-8"
        } ${open ? "px-2" : "justify-center px-0"} ${
          isActive
            ? `text-tyba-text ${w.color ? "" : "bg-white/[.05]"}`
            : "text-tyba-text-faint hover:bg-white/[.03] hover:text-tyba-text-muted"
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
        {isConfig ? (
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
        {open && (
          <>
            <span className="flex min-w-0 flex-1 flex-col items-start gap-0.5">
              <span className="w-full truncate text-left leading-none">
                {isConfig ? t("settings") : w.name}
              </span>
              {showDetails && (
                <span className="flex w-full items-center gap-1.5">
                  <span className="min-w-0 truncate font-mono text-[10px] leading-none text-tyba-text-faint">
                    {w.repo_root ? compactPath(w.repo_root) : "~"}
                  </span>
                  {branch && (
                    <span className="flex shrink-0 items-center gap-0.5 font-mono text-[10px] leading-none text-tyba-text-faint">
                      <GitBranch size={9} />
                      {branch}
                    </span>
                  )}
                  {agent && (
                    <span className="flex shrink-0 items-center gap-1 rounded-[3px] bg-tyba-violet-tint px-1 py-px font-mono text-[9px] leading-none text-tyba-violet">
                      <Robot size={9} weight="bold" />
                      {agent}
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
            <DropdownMenu>
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
              <DropdownMenuContent
                align="start"
                className="w-52"
              >
                <DropdownMenuItem
                  className="text-xs"
                  onSelect={() => setPrompt({ kind: "rename", ws: w })}
                >
                  {t("renameSession")}
                </DropdownMenuItem>
                <DropdownMenuSub>
                  <DropdownMenuSubTrigger className="text-xs">
                    {t("groupSession")}
                  </DropdownMenuSubTrigger>
                  <DropdownMenuSubContent className="w-44">
                    {groupNames.map((name) => (
                      <DropdownMenuItem
                        key={name}
                        className="text-xs"
                        onSelect={() =>
                          void setWorkspaceGroup(
                            w.id,
                            w.group === name ? null : name,
                          )
                        }
                      >
                        <span className="min-w-0 flex-1 truncate">{name}</span>
                        {w.group === name && (
                          <Check
                            size={11}
                            weight="bold"
                            className="text-tyba-green"
                          />
                        )}
                      </DropdownMenuItem>
                    ))}
                    {groupNames.length > 0 && <DropdownMenuSeparator />}
                    <DropdownMenuItem
                      className="text-xs"
                      onSelect={() => setPrompt({ kind: "group", ws: w })}
                    >
                      <Plus size={11} weight="bold" />
                      {t("newGroup")}
                    </DropdownMenuItem>
                  </DropdownMenuSubContent>
                </DropdownMenuSub>
                {w.group && (
                  <DropdownMenuItem
                    className="text-xs"
                    onSelect={() => void setWorkspaceGroup(w.id, null)}
                  >
                    {t("removeFromGroup")}
                  </DropdownMenuItem>
                )}
                <DropdownMenuSeparator />
                {branch && (
                  <DropdownMenuItem
                    className="text-xs"
                    onSelect={() => copyText(branch)}
                  >
                    {t("copyBranch")}
                  </DropdownMenuItem>
                )}
                {w.repo_root && (
                  <DropdownMenuItem
                    className="text-xs"
                    onSelect={() => copyText(w.repo_root as string)}
                  >
                    {t("copyDir")}
                  </DropdownMenuItem>
                )}
                {(branch || w.repo_root) && <DropdownMenuSeparator />}
                <div className="flex items-center gap-1.5 px-2 py-1.5">
                  <button
                    aria-label={t("noColor")}
                    onClick={() => void setWorkspaceColor(w.id, null)}
                    className={`flex size-4 items-center justify-center rounded-full border text-tyba-text-faint ${
                      !w.color
                        ? "border-tyba-text-muted"
                        : "border-tyba-border-strong"
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
                        w.color === c
                          ? "border-tyba-text"
                          : "border-transparent"
                      }`}
                      style={{ background: `var(--tyba-${c})` }}
                    />
                  ))}
                </div>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  className="text-xs"
                  onSelect={() => toggleWorkspaceDetails(w.id)}
                >
                  {detailsFor(w.id) ? t("detailsHide") : t("detailsShow")}
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  className="text-xs text-tyba-red focus:text-tyba-red"
                  onSelect={() => void killWorkspace(w.id)}
                >
                  <X size={12} weight="bold" />
                  {t("killSession")}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
            )}
          </>
        )}
      </button>
    );
  };

  return (
    <TooltipProvider delayDuration={400}>
      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        mode={paletteMode}
        onModeChange={setPaletteMode}
        workspaces={layout.workspaces}
        activeWorkspace={layout.active_workspace}
        bindings={bindings}
        theme={theme}
        onChangeTheme={changeTheme}
        onNewSession={() => setNewSessionOpen(true)}
        onNewTab={() => void newTab()}
        onCloseActive={() => void closeActivePane()}
        onOpenSettings={() => void openViewTab("settings").catch(() => {})}
        onTogglePanel={toggleSidebar}
        onGoToWorkspace={(id) => void activateWorkspace(id)}
      />
      <NewSessionPrompt
        open={newSessionOpen}
        onOpenChange={(open) => {
          setNewSessionOpen(open);
          if (!open) setPendingGroup(null);
        }}
        onCreate={(cwd, name) => void newSession(cwd, name, pendingGroup)}
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
        <header
          data-tauri-drag-region
          className="tyba-glass relative flex h-9 shrink-0 items-center gap-1 border-b border-tyba-border pl-20 pr-2.5"
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
            <div className="pointer-events-auto flex items-center gap-0.5 rounded-[5px] border border-tyba-border bg-white/[.02] p-0.5">
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    aria-label={t("terminalView")}
                    aria-pressed
                    className="flex h-6 items-center gap-1.5 rounded-[3px] bg-white/[.06] px-2 text-[11px] text-tyba-text"
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
                    disabled
                    aria-label={t("workspaceView")}
                    className="flex h-6 items-center gap-1.5 rounded-[3px] px-2 text-[11px] text-tyba-text-faint disabled:cursor-not-allowed"
                  >
                    <SquaresFour size={14} />
                    {t("workspaceView")}
                  </button>
                </TooltipTrigger>
                <TooltipContent side="bottom" className="flex items-center gap-2">
                  {t("workspaceView")}
                  <span className="rounded-full bg-tyba-violet-tint px-1.5 py-px text-[9px] font-medium text-tyba-violet">
                    {t("comingSoon")}
                  </span>
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
                      ? "bg-white/[.06] text-tyba-text"
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

          <IconAction
            label={t("openProjectFolder")}
            shortcut={bindings.openFolder}
            onClick={() => void openProjectFolder()}
          >
            <FolderOpen size={16} />
          </IconAction>

          <ApprovalsInbox
            sessions={sessions}
            approvals={approvals}
            open={inboxOpen}
            onOpenChange={setInboxOpen}
          />

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
        </header>

        {approvals.length > 0 && (
          <Notification
            variant="warning"
            className="rounded-none border-x-0 border-t-0 px-3 py-1.5 text-xs"
          >
            <NotificationTitle>
              {t("approvalsWaiting", { count: approvals.length })}
            </NotificationTitle>
            <NotificationLink asChild>
              <button type="button" onClick={() => setInboxOpen(true)}>
                {t("reviewApprovals")}
                <ArrowRight
                  aria-hidden="true"
                  size={14}
                  className="-mt-0.5 ms-1 inline-flex opacity-60 transition-transform motion-safe:group-hover:translate-x-0.5"
                />
              </button>
            </NotificationLink>
          </Notification>
        )}

        <div className="flex min-h-0 flex-1">
          <>
            {sidebar !== "hidden" && (
                <aside
                  className={`tyba-glass flex shrink-0 flex-col ${
                    open ? "w-56" : "w-11"
                  }`}
                >
                  {open && (
                    <label className="mx-2 mt-3 flex h-7 items-center gap-1.5 rounded-[4px] bg-white/[.03] px-2 focus-within:bg-white/[.05]">
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
                      <div className="mb-1 flex flex-col gap-px rounded-[6px] border border-tyba-border/70 bg-white/[.015] p-1">
                        {open && (
                          <span className="flex items-center gap-2 px-1.5 pt-1 pb-1.5">
                            <span className="text-[10px] font-medium uppercase tracking-[0.14em] text-tyba-text-faint">
                              {t("systemGroup")}
                            </span>
                            <span className="h-px min-w-0 flex-1 bg-tyba-border" />
                          </span>
                        )}
                        <button
                          onClick={() =>
                            void openViewTab("settings").catch(() => {})
                          }
                          title={open ? undefined : t("settings")}
                          className={`group relative flex h-8 shrink-0 items-center gap-2 rounded-[4px] text-[13px] transition-colors ${
                            open ? "px-2" : "justify-center px-0"
                          } ${
                            activeTab?.view === "settings"
                              ? "bg-white/[.05] text-tyba-text"
                              : "text-tyba-text-faint hover:bg-white/[.03] hover:text-tyba-text-muted"
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
                      </div>
                    )}
                    {groupedWorkspaces.groups.map(([name, list]) => (
                      <div
                        key={name}
                        className="mb-1 flex flex-col gap-px rounded-[6px] border border-tyba-border/70 bg-white/[.015] p-1"
                      >
                        <span className="flex items-center gap-2 px-1.5 pt-1 pb-1.5">
                          <span className="text-[10px] font-medium uppercase tracking-[0.14em] text-tyba-text-faint">
                            {name}
                          </span>
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
                        {list.map(renderWorkspace)}
                      </div>
                    ))}
                    {groupedWorkspaces.loose
                      .filter((w) => !isConfigWorkspace(w))
                      .map(renderWorkspace)}
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          variant="ghost"
                          onClick={() => setNewSessionOpen(true)}
                          aria-label={t("newSession")}
                          className={`mt-0.5 h-8 shrink-0 gap-2 rounded-[4px] text-[13px] font-normal text-tyba-text-faint hover:bg-white/[.03] hover:text-tyba-text ${
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
                  </nav>
                </aside>
              )}

              <main className="flex min-h-0 min-w-0 flex-1 flex-col">
                {activeWorkspace &&
                  activeWorkspace.tabs.length > 0 &&
                  activeTab?.view !== "settings" && (
                  <TabBar
                    tabs={activeWorkspace.tabs}
                    activeTab={activeWorkspace.active_tab}
                    sessions={sessions}
                    onActivate={(id) => void activateTab(id)}
                    onClose={(id) => void closeTabAndRefresh(id)}
                    onNew={() => void newTab()}
                  />
                )}
                <div
                  ref={paneAreaRef}
                  className="relative min-h-0 flex-1 overflow-hidden"
                >
                  {activeTab?.view === "containers" && (
                    <div className="absolute inset-0 flex">
                      <ContainersView
                        onAvailableChange={setDockerUp}
                        onRunningChange={setDockerRunning}
                      />
                    </div>
                  )}
                  {activeTab?.view === "settings" && (
                    <div className="absolute inset-0 flex">
                      <SettingsView
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
                      />
                    </div>
                  )}
                  {sessions.map((s) => {
                    const paneRect =
                      paneLayout?.panes.find((p) => p.session === s.id) ??
                      null;
                    return (
                      <TerminalView
                        key={s.id}
                        sessionId={s.id}
                        visible={paneRect !== null}
                        focused={s.id === activeId}
                        framed={(paneLayout?.panes.length ?? 0) > 1}
                        rect={
                          paneRect
                            ? {
                                left: paneRect.x,
                                top: paneRect.y,
                                width: paneRect.w,
                                height: paneRect.h,
                              }
                            : null
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
                        className={`rounded-full bg-tyba-border-strong transition-colors hover:bg-tyba-green/70 ${
                          d.kind === "v" ? "h-full w-px" : "h-px w-full"
                        }`}
                      />
                    </div>
                  ))}
                  {!activeTab && (
                    <div className="absolute inset-0 flex flex-col items-center justify-center gap-5">
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
                            onClick={() => setNewSessionOpen(true)}
                            className="flex h-8 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] text-tyba-text-muted transition-colors hover:bg-white/[.04] hover:text-tyba-text"
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
                            className="flex h-8 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] text-tyba-text-muted transition-colors hover:bg-white/[.04] hover:text-tyba-text"
                          >
                            <Plus size={14} className="text-tyba-green" />
                            <span className="flex-1 text-left">
                              {t("newTab")}
                            </span>
                            <Shortcut combo={bindings.newTab} />
                          </button>
                        )}
                        <button
                          onClick={() => openPalette("actions")}
                          className="flex h-8 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] text-tyba-text-muted transition-colors hover:bg-white/[.04] hover:text-tyba-text"
                        >
                          <MagnifyingGlass size={14} />
                          <span className="flex-1 text-left">
                            {t("commandPalette")}
                          </span>
                          <Shortcut combo={bindings.paletteActions} />
                        </button>
                        <button
                          onClick={toggleSidebar}
                          className="flex h-8 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] text-tyba-text-muted transition-colors hover:bg-white/[.04] hover:text-tyba-text"
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
