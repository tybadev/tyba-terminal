// TYBA — shell da aplicação: header geral + sidebar de sessões + terminal.
// Direção visual: "o gradiente é luz" — linha viva marca a sessão ativa,
// itens flat (terminal, não web), raios contidos. Tokens em src/styles.css.
// Atalhos: ⌘K paleta · ⌘B painel (aberto → ícones → oculto) ·
// ⌘T nova sessão · ⌘W fechar.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CaretDown,
  CaretRight,
  FolderOpen,
  Plus,
  SidebarSimple,
  TerminalWindow,
  User,
  X,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { LANGUAGES, setLanguage, type LanguageCode } from "./i18n";
import {
  getThemeMode,
  onThemeModeChange,
  setThemeMode,
  type ThemeMode,
} from "./theme";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ApprovalsInbox } from "./components/ApprovalsInbox";
import { CommandPalette } from "./components/CommandPalette";
import { TabBar } from "./components/TabBar";
import { TerminalView } from "./components/TerminalView";
import {
  activateTab,
  closePane,
  closeTab,
  createSession,
  createTab,
  disposeSession,
  layoutState,
  leafSessions,
  listSessions,
  onLayoutChanged,
  openSessionInTab,
  paneSession,
  type LayoutState,
  type Session,
  type SessionId,
} from "./lib/ipc";
import tybaMark from "./assets/tyba-mark.svg";

const EMPTY_LAYOUT: LayoutState = { tabs: [], active_tab: null };

interface SessionGroup {
  key: string;
  label: string | null;
  sessions: Session[];
}

function groupSessions(sessions: Session[]): SessionGroup[] {
  const map = new Map<string, Session[]>();
  for (const s of sessions) {
    const key = s.repo_root ?? "";
    map.set(key, [...(map.get(key) ?? []), s]);
  }
  return [...map.entries()]
    .sort(([a], [b]) =>
      a === "" ? 1 : b === "" ? -1 : a.localeCompare(b),
    )
    .map(([key, list]) => ({
      key: key || "loose",
      label: key ? (key.split("/").pop() ?? key) : null,
      sessions: list,
    }));
}

type SidebarMode = "open" | "rail" | "hidden";

const NEXT_MODE: Record<SidebarMode, SidebarMode> = {
  open: "rail",
  rail: "hidden",
  hidden: "open",
};

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

export default function App() {
  const { t, i18n } = useTranslation();
  const [sessions, setSessions] = useState<Session[]>([]);
  const [layout, setLayout] = useState<LayoutState>(EMPTY_LAYOUT);
  const [sidebar, setSidebar] = useState<SidebarMode>("open");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [theme, setTheme] = useState<ThemeMode>(getThemeMode);
  const booted = useRef(false);

  const changeTheme = useCallback((next: ThemeMode) => {
    setThemeMode(next);
    setTheme(next);
  }, []);

  useEffect(() => onThemeModeChange(setTheme), []);

  const activeTab = useMemo(
    () => layout.tabs.find((tab) => tab.id === layout.active_tab) ?? null,
    [layout],
  );

  const activeId = useMemo(
    () =>
      activeTab ? paneSession(activeTab.root, activeTab.active_pane) : null,
    [activeTab],
  );

  const boundSessions = useMemo(
    () => new Set(layout.tabs.flatMap((tab) => leafSessions(tab.root))),
    [layout],
  );

  const groups = useMemo(() => groupSessions(sessions), [sessions]);

  const newShell = useCallback(async () => {
    const session = await createSession({
      kind: { type: "shell" },
      cols: 100,
      rows: 30,
    });
    setSessions((prev) => [...prev, session]);
    await createTab(session.id);
  }, []);

  const goToSession = useCallback((id: SessionId) => {
    void openSessionInTab(id);
  }, []);

  const killSession = useCallback(async (id: SessionId) => {
    await disposeSession(id);
    setSessions((prev) => prev.filter((s) => s.id !== id));
  }, []);

  const closeActivePane = useCallback(() => {
    if (activeTab) void closePane(activeTab.active_pane);
  }, [activeTab]);

  const toggleGroup = useCallback((key: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }, []);

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
      const [existing, currentLayout] = await Promise.all([
        listSessions(),
        layoutState(),
      ]);
      if (cancelled) return;
      setSessions(existing);
      setLayout(currentLayout);
      if (existing.length === 0 && !booted.current) {
        booted.current = true;
        void newShell();
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [newShell]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || e.repeat || e.shiftKey || e.altKey || e.ctrlKey) return;
      const k = e.key.toLowerCase();
      if (k === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      } else if (k === "b") {
        e.preventDefault();
        setSidebar((m) => NEXT_MODE[m]);
      } else if (k === "t") {
        e.preventDefault();
        void newShell();
      } else if (k === "w") {
        e.preventDefault();
        closeActivePane();
      } else if (k >= "1" && k <= "9") {
        const target = layout.tabs[Number(k) - 1];
        if (target) {
          e.preventDefault();
          void activateTab(target.id);
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [newShell, closeActivePane, layout.tabs]);

  const open = sidebar === "open";

  return (
    <TooltipProvider delayDuration={400}>
      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        sessions={sessions}
        activeId={activeId}
        theme={theme}
        onChangeTheme={changeTheme}
        onNewSession={() => void newShell()}
        onCloseActive={closeActivePane}
        onKillActive={() => {
          if (activeId) void killSession(activeId);
        }}
        onTogglePanel={() => setSidebar((m) => NEXT_MODE[m])}
        onGoToSession={goToSession}
      />
      <div className="tyba-aurora flex h-screen flex-col text-tyba-text">
        {/* ---------- Header geral: delicado, 36px ---------- */}
        <header
          data-tauri-drag-region
          className="tyba-glass flex h-9 shrink-0 items-center gap-1 border-b border-tyba-border pl-20 pr-2.5"
        >
          <IconAction
            label={t("panelToggle")}
            shortcut="⌘B"
            onClick={() => setSidebar((m) => NEXT_MODE[m])}
          >
            <SidebarSimple size={16} />
          </IconAction>

          <span className="ml-1.5 flex select-none items-center gap-2">
            <img src={tybaMark} alt="" className="h-4 w-4" />
            <span className="text-xs font-bold tracking-[0.2em]">TYBA</span>
          </span>

          <div className="h-full flex-1" data-tauri-drag-region />

          <IconAction label={t("openProjectFolder")} shortcut="⌘O">
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
              <DropdownMenuLabel className="tyba-label">
                {t("language")}
              </DropdownMenuLabel>
              <DropdownMenuRadioGroup
                value={i18n.language}
                onValueChange={(v) => setLanguage(v as LanguageCode)}
              >
                {LANGUAGES.map((lang) => (
                  <DropdownMenuRadioItem
                    key={lang.code}
                    value={lang.code}
                    className="text-xs"
                  >
                    {lang.label}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
              <DropdownMenuSeparator />
              <DropdownMenuLabel className="tyba-label">
                {t("theme")}
              </DropdownMenuLabel>
              <DropdownMenuRadioGroup
                value={theme}
                onValueChange={(v) => changeTheme(v as ThemeMode)}
              >
                <DropdownMenuRadioItem value="dark" className="text-xs">
                  {t("themeDark")}
                </DropdownMenuRadioItem>
                <DropdownMenuRadioItem value="light" className="text-xs">
                  {t("themeLight")}
                </DropdownMenuRadioItem>
                <DropdownMenuRadioItem value="system" className="text-xs">
                  {t("themeSystem")}
                </DropdownMenuRadioItem>
              </DropdownMenuRadioGroup>
              <DropdownMenuSeparator />
              <DropdownMenuItem disabled className="text-xs">
                {t("settings")}
              </DropdownMenuItem>
              <DropdownMenuItem disabled className="text-xs">
                {t("about")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </header>

        {/* ---------- Sidebar + conteúdo ---------- */}
        <div className="flex min-h-0 flex-1">
          {sidebar !== "hidden" && (
            <aside
              className={`tyba-glass flex shrink-0 flex-col border-r border-tyba-border ${
                open ? "w-56" : "w-11"
              }`}
            >
              {open && (
                <span className="tyba-label px-3.5 pt-3.5">{t("sessions")}</span>
              )}
              <nav
                className={`flex min-h-0 flex-1 flex-col gap-px overflow-y-auto px-2 pb-2 ${
                  open ? "mt-1.5" : "mt-2"
                }`}
              >
                {groups.map((group) => {
                  const isCollapsed = collapsed.has(group.key);
                  return (
                    <div key={group.key} className="flex flex-col gap-px">
                      {open && (
                        <button
                          onClick={() => toggleGroup(group.key)}
                          className="flex h-6 shrink-0 items-center gap-1 rounded-[4px] px-1.5 text-[11px] font-medium tracking-wide text-tyba-text-faint transition-colors hover:text-tyba-text-muted"
                        >
                          {isCollapsed ? (
                            <CaretRight size={9} weight="bold" />
                          ) : (
                            <CaretDown size={9} weight="bold" />
                          )}
                          <span className="truncate">
                            {group.label ?? t("looseSessions")}
                          </span>
                        </button>
                      )}
                      {(!open || !isCollapsed) &&
                        group.sessions.map((s) => {
                          const isActive = s.id === activeId;
                          const isBound = boundSessions.has(s.id);
                          return (
                            <button
                              key={s.id}
                              onClick={() => goToSession(s.id)}
                              title={open ? undefined : s.title}
                              className={`group relative flex h-8 shrink-0 items-center gap-2 rounded-[4px] text-[13px] transition-colors ${
                                open ? "px-2 pl-4" : "justify-center px-0"
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
                                    {s.title}
                                  </span>
                                  {isBound && !isActive && (
                                    <span className="size-1 shrink-0 rounded-full bg-tyba-green/60" />
                                  )}
                                  <span
                                    role="button"
                                    aria-label={t("killSession")}
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      void killSession(s.id);
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
                    </div>
                  );
                })}
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="ghost"
                      onClick={() => void newShell()}
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
                    <Kbd>⌘T</Kbd>
                  </TooltipContent>
                </Tooltip>
              </nav>
            </aside>
          )}

          <main className="flex min-h-0 min-w-0 flex-1 flex-col">
            {layout.tabs.length > 0 && (
              <TabBar
                tabs={layout.tabs}
                activeTab={layout.active_tab}
                sessions={sessions}
                onActivate={(id) => void activateTab(id)}
                onClose={(id) => void closeTab(id)}
                onNew={() => void newShell()}
              />
            )}
            <div className="relative min-h-0 flex-1">
              {sessions.map((s) => (
                <TerminalView
                  key={s.id}
                  sessionId={s.id}
                  active={s.id === activeId}
                  onExit={() => void killSession(s.id)}
                />
              ))}
              {!activeTab && (
                <div className="absolute inset-0 flex flex-col items-center justify-center gap-4">
                  <img
                    src={tybaMark}
                    alt=""
                    className="h-12 w-12 opacity-90"
                  />
                  <p className="text-sm text-tyba-text-faint">
                    {sessions.length === 0 ? t("noSessions") : t("noTabs")}
                  </p>
                  <Button onClick={() => void newShell()}>
                    <Plus size={16} weight="bold" />
                    {sessions.length === 0 ? t("newSession") : t("newTab")}
                  </Button>
                  <p className="flex items-center gap-1.5 text-xs text-tyba-text-faint">
                    <Kbd>⌘K</Kbd> {t("hintPalette")} · <Kbd>⌘T</Kbd>{" "}
                    {t("hintNewSession")} · <Kbd>⌘B</Kbd> {t("hintPanel")}
                  </p>
                </div>
              )}
            </div>
          </main>
        </div>
      </div>
    </TooltipProvider>
  );
}
