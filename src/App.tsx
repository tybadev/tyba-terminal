// TYBA — shell da aplicação: header geral + sidebar de sessões + terminal.
// Direção visual: "o gradiente é luz" — linha viva marca a sessão ativa,
// itens flat (terminal, não web), raios contidos. Tokens em src/styles.css.
// Atalhos: ⌘K paleta · ⌘B painel (aberto → ícones → oculto) ·
// ⌘T nova sessão · ⌘W fechar.

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
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
import { getThemeMode, setThemeMode, type ThemeMode } from "./theme";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ApprovalsInbox } from "./components/ApprovalsInbox";
import { CommandPalette } from "./components/CommandPalette";
import { TerminalView } from "./components/TerminalView";
import {
  createSession,
  disposeSession,
  type Session,
  type SessionId,
} from "./lib/ipc";
import tybaMark from "./assets/tyba-mark.svg";

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
  const [activeId, setActiveId] = useState<SessionId | null>(null);
  const [sidebar, setSidebar] = useState<SidebarMode>("open");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [theme, setTheme] = useState<ThemeMode>(getThemeMode);
  const booted = useRef(false);

  const changeTheme = useCallback((next: ThemeMode) => {
    setThemeMode(next);
    setTheme(next);
  }, []);

  const newShell = useCallback(async () => {
    const session = await createSession({
      kind: { type: "shell" },
      cols: 100,
      rows: 30,
    });
    setSessions((prev) => [...prev, session]);
    setActiveId(session.id);
  }, []);

  const closeSession = useCallback(
    async (id: SessionId) => {
      await disposeSession(id);
      setSessions((prev) => {
        const next = prev.filter((s) => s.id !== id);
        setActiveId((cur) =>
          cur === id ? (next.at(-1)?.id ?? null) : cur,
        );
        return next;
      });
    },
    [],
  );

  // primeira sessão ao abrir
  useEffect(() => {
    if (booted.current) return;
    booted.current = true;
    void newShell();
  }, [newShell]);

  // atalhos globais (capture: funcionam mesmo com o xterm focado)
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
        if (activeId) void closeSession(activeId);
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [newShell, closeSession, activeId]);

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
        onCloseActive={() => {
          if (activeId) void closeSession(activeId);
        }}
        onTogglePanel={() => setSidebar((m) => NEXT_MODE[m])}
        onGoToSession={setActiveId}
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
                {sessions.map((s) => {
                  const isActive = s.id === activeId;
                  return (
                    <button
                      key={s.id}
                      onClick={() => setActiveId(s.id)}
                      title={open ? undefined : s.title}
                      className={`group relative flex h-8 shrink-0 items-center gap-2 rounded-[4px] text-[13px] transition-colors ${
                        open ? "px-2" : "justify-center px-0"
                      } ${
                        isActive
                          ? "text-tyba-text"
                          : "text-tyba-text-faint hover:bg-white/[.03] hover:text-tyba-text-muted"
                      }`}
                    >
                      {/* linha viva: a sessão ativa está acesa — sem card */}
                      {isActive && (
                        <span
                          className="absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-full"
                          style={{ background: "var(--tyba-gradient-soft)" }}
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
                          <span
                            role="button"
                            aria-label={t("closeSession")}
                            onClick={(e) => {
                              e.stopPropagation();
                              void closeSession(s.id);
                            }}
                            className="rounded-[3px] text-tyba-text-faint opacity-0 transition-opacity hover:text-tyba-text group-hover:opacity-100"
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

          <main className="relative min-h-0 min-w-0 flex-1">
            {sessions.length === 0 ? (
              <div className="flex h-full flex-col items-center justify-center gap-4">
                <img src={tybaMark} alt="" className="h-12 w-12 opacity-90" />
                <p className="text-sm text-tyba-text-faint">
                  {t("noSessions")}
                </p>
                <Button onClick={() => void newShell()}>
                  <Plus size={16} weight="bold" />
                  {t("newSession")}
                </Button>
                <p className="flex items-center gap-1.5 text-xs text-tyba-text-faint">
                  <Kbd>⌘K</Kbd> {t("hintPalette")} · <Kbd>⌘T</Kbd>{" "}
                  {t("hintNewSession")} · <Kbd>⌘B</Kbd> {t("hintPanel")}
                </p>
              </div>
            ) : (
              sessions.map((s) => (
                <TerminalView
                  key={s.id}
                  sessionId={s.id}
                  active={s.id === activeId}
                  onExit={() => void closeSession(s.id)}
                />
              ))
            )}
          </main>
        </div>
      </div>
    </TooltipProvider>
  );
}
