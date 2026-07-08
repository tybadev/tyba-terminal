import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  FolderOpen,
  MagnifyingGlass,
  Plus,
  SidebarSimple,
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
  DropdownMenuLabel,
  DropdownMenuSeparator,
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
import { CommandPalette } from "./components/CommandPalette";
import { NewSessionPrompt } from "./components/NewSessionPrompt";
import { SettingsView, type SidebarTogglePref } from "./components/SettingsView";
import { TabBar } from "./components/TabBar";
import { TerminalView } from "./components/TerminalView";
import {
  activateTab,
  activateWorkspace,
  closePane,
  closeTab,
  closeWorkspace,
  createSession,
  createTab,
  createWorkspace,
  getPref,
  layoutState,
  listSessions,
  onLayoutChanged,
  paneSession,
  setPref,
  type LayoutState,
  type Session,
} from "./lib/ipc";

const EMPTY_LAYOUT: LayoutState = { workspaces: [], active_workspace: null };
const TOGGLE_PREF_KEY = "pref.sidebar_toggle";

type SidebarMode = "open" | "rail" | "hidden";

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="rounded-[4px] border border-tyba-border-strong bg-tyba-raised px-1 py-px font-mono text-[10px] text-tyba-text-muted">
      {children}
    </kbd>
  );
}

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
        {shortcut && <Kbd>{shortcut}</Kbd>}
      </TooltipContent>
    </Tooltip>
  );
}

const basename = (dir: string) => dir.split("/").filter(Boolean).pop() ?? dir;

export default function App() {
  const { t } = useTranslation();
  const [sessions, setSessions] = useState<Session[]>([]);
  const [layout, setLayout] = useState<LayoutState>(EMPTY_LAYOUT);
  const [sidebar, setSidebar] = useState<SidebarMode>("open");
  const [togglePref, setTogglePref] = useState<SidebarTogglePref>("hidden");
  const [sessionQuery, setSessionQuery] = useState("");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [newSessionOpen, setNewSessionOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [theme, setTheme] = useState<ThemeMode>(getThemeMode);
  const booted = useRef(false);

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
      activeTab ? paneSession(activeTab.root, activeTab.active_pane) : null,
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

  const newSession = useCallback(
    async (cwd: string | null, name: string) => {
      const session = await createSession({
        kind: { type: "shell" },
        cwd: cwd ?? undefined,
        cols: 100,
        rows: 30,
      });
      setSessions((prev) => [...prev, session]);
      await createWorkspace(name, cwd, session.id);
    },
    [],
  );

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
    await createTab(session.id, activeWorkspace.id);
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
    await closePane(activeTab.active_pane);
    await refreshSessions();
  }, [activeTab, refreshSessions]);

  const closeTabAndRefresh = useCallback(
    async (id: string) => {
      await closeTab(id);
      await refreshSessions();
    },
    [refreshSessions],
  );

  const changeTogglePref = useCallback((value: SidebarTogglePref) => {
    setTogglePref(value);
    void setPref(TOGGLE_PREF_KEY, value).catch(() => {});
  }, []);

  const toggleSidebar = useCallback(() => {
    setSidebar((current) => (current === "open" ? togglePref : "open"));
  }, [togglePref]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void (async () => {
      const un = await onLayoutChanged((state) => {
        if (!cancelled) setLayout(state);
      });
      if (cancelled) {
        un();
        return;
      }
      unlisten = un;
      const [existing, currentLayout, pref] = await Promise.all([
        listSessions().catch(() => [] as Session[]),
        layoutState().catch(() => EMPTY_LAYOUT),
        getPref(TOGGLE_PREF_KEY).catch(() => null),
      ]);
      if (cancelled) return;
      setSessions(existing);
      setLayout(currentLayout);
      if (pref === "rail" || pref === "hidden") setTogglePref(pref);
      if (currentLayout.workspaces.length === 0 && !booted.current) {
        booted.current = true;
        setNewSessionOpen(true);
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || e.repeat || e.shiftKey || e.altKey || e.ctrlKey) return;
      const k = e.key.toLowerCase();
      if (k === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      } else if (k === "b") {
        e.preventDefault();
        toggleSidebar();
      } else if (k === "t") {
        e.preventDefault();
        void newTab();
      } else if (k === "o") {
        e.preventDefault();
        void openProjectFolder();
      } else if (k === "w") {
        e.preventDefault();
        void closeActivePane();
      } else if (k >= "1" && k <= "9") {
        const target = activeWorkspace?.tabs[Number(k) - 1];
        if (target) {
          e.preventDefault();
          void activateTab(target.id);
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [newTab, closeActivePane, toggleSidebar, openProjectFolder, activeWorkspace]);

  const open = sidebar === "open";

  return (
    <TooltipProvider delayDuration={400}>
      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        workspaces={layout.workspaces}
        activeWorkspace={layout.active_workspace}
        theme={theme}
        onChangeTheme={changeTheme}
        onNewSession={() => setNewSessionOpen(true)}
        onNewTab={() => void newTab()}
        onCloseActive={() => void closeActivePane()}
        onOpenSettings={() => setSettingsOpen(true)}
        onTogglePanel={toggleSidebar}
        onGoToWorkspace={(id) => void activateWorkspace(id)}
      />
      <NewSessionPrompt
        open={newSessionOpen}
        onOpenChange={setNewSessionOpen}
        onCreate={(cwd, name) => void newSession(cwd, name)}
      />
      <div className="tyba-aurora flex h-screen flex-col text-tyba-text">
        <header
          data-tauri-drag-region
          className="tyba-glass flex h-9 shrink-0 items-center gap-1 border-b border-tyba-border pl-20 pr-2.5"
        >
          <IconAction
            label={t("panelToggle")}
            shortcut="⌘B"
            onClick={toggleSidebar}
          >
            <SidebarSimple size={16} />
          </IconAction>

          <IconAction
            label={t("commandPalette")}
            shortcut="⌘K"
            onClick={() => setPaletteOpen(true)}
          >
            <MagnifyingGlass size={16} />
          </IconAction>

          <div className="h-full flex-1" data-tauri-drag-region />

          <IconAction
            label={t("openProjectFolder")}
            shortcut="⌘O"
            onClick={() => void openProjectFolder()}
          >
            <FolderOpen size={16} />
          </IconAction>

          <ApprovalsInbox sessions={sessions} />

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
              className="w-52 border-tyba-border-strong bg-tyba-overlay shadow-lg"
            >
              <DropdownMenuLabel className="text-xs">
                {t("localAccount")}
              </DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                className="text-xs"
                onSelect={() => setSettingsOpen(true)}
              >
                {t("settings")}
              </DropdownMenuItem>
              <DropdownMenuItem disabled className="text-xs">
                {t("about")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </header>

        <div className="flex min-h-0 flex-1">
          {settingsOpen ? (
            <SettingsView
              onClose={() => setSettingsOpen(false)}
              togglePref={togglePref}
              onTogglePrefChange={changeTogglePref}
            />
          ) : (
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
                    {workspaces.map((w) => {
                      const isActive = w.id === layout.active_workspace;
                      return (
                        <button
                          key={w.id}
                          onClick={() => void activateWorkspace(w.id)}
                          title={open ? (w.repo_root ?? undefined) : w.name}
                          className={`group relative flex h-8 shrink-0 items-center gap-2 rounded-[4px] text-[13px] transition-colors ${
                            open ? "px-2" : "justify-center px-0"
                          } ${
                            isActive
                              ? "text-tyba-text"
                              : "text-tyba-text-faint hover:bg-white/[.03] hover:text-tyba-text-muted"
                          }`}
                        >
                          {isActive && (
                            <span
                              className="absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-full"
                              style={{
                                background: "var(--tyba-gradient-soft)",
                              }}
                            />
                          )}
                          <TerminalWindow
                            size={16}
                            className={
                              isActive
                                ? "shrink-0 text-tyba-green [filter:drop-shadow(0_0_6px_rgba(124,197,68,.55))]"
                                : "shrink-0"
                            }
                          />
                          {open && (
                            <>
                              <span className="min-w-0 flex-1 truncate text-left">
                                {w.name}
                              </span>
                              <span className="font-mono text-[10px] text-tyba-text-faint">
                                {w.tabs.length > 0 ? w.tabs.length : ""}
                              </span>
                              <span
                                role="button"
                                aria-label={t("killSession")}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  void killWorkspace(w.id);
                                }}
                                className="rounded-[3px] text-tyba-text-faint opacity-0 transition-opacity hover:text-tyba-red group-hover:opacity-100"
                              >
                                <X size={11} weight="bold" />
                              </span>
                            </>
                          )}
                        </button>
                      );
                    })}
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
                {activeWorkspace && activeWorkspace.tabs.length > 0 && (
                  <TabBar
                    tabs={activeWorkspace.tabs}
                    activeTab={activeWorkspace.active_tab}
                    sessions={sessions}
                    onActivate={(id) => void activateTab(id)}
                    onClose={(id) => void closeTabAndRefresh(id)}
                    onNew={() => void newTab()}
                  />
                )}
                <div className="relative min-h-0 flex-1">
                  {sessions.map((s) => (
                    <TerminalView
                      key={s.id}
                      sessionId={s.id}
                      active={s.id === activeId}
                      onExit={() => void refreshSessions()}
                    />
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
                            <Kbd>⌘T</Kbd>
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
                            <Kbd>⌘T</Kbd>
                          </button>
                        )}
                        <button
                          onClick={() => setPaletteOpen(true)}
                          className="flex h-8 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] text-tyba-text-muted transition-colors hover:bg-white/[.04] hover:text-tyba-text"
                        >
                          <MagnifyingGlass size={14} />
                          <span className="flex-1 text-left">
                            {t("commandPalette")}
                          </span>
                          <Kbd>⌘K</Kbd>
                        </button>
                        <button
                          onClick={toggleSidebar}
                          className="flex h-8 items-center gap-2.5 rounded-[4px] px-2.5 text-[13px] text-tyba-text-muted transition-colors hover:bg-white/[.04] hover:text-tyba-text"
                        >
                          <SidebarSimple size={14} />
                          <span className="flex-1 text-left">
                            {t("togglePanel")}
                          </span>
                          <Kbd>⌘B</Kbd>
                        </button>
                      </div>
                    </div>
                  )}
                </div>
              </main>
            </>
          )}
        </div>
      </div>
    </TooltipProvider>
  );
}
