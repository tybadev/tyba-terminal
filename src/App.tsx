// TYBA — shell da aplicação: header geral + sidebar de sessões + terminal.
// Direção visual: "o gradiente é luz" — linha viva marca a sessão ativa,
// sidebar de vidro sobre canvas com aurora. Tokens em src/styles.css.

import { useCallback, useEffect, useRef, useState } from "react";
import {
  Bell,
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
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { TerminalView } from "./components/TerminalView";
import {
  createSession,
  disposeSession,
  type Session,
  type SessionId,
} from "./lib/ipc";
import tybaMark from "./assets/tyba-mark.svg";

function IconAction({
  label,
  onClick,
  children,
}: {
  label: string;
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
          className="size-7 text-tyba-text-muted hover:text-tyba-text"
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}

export default function App() {
  const [sessions, setSessions] = useState<Session[]>([]);
  const [activeId, setActiveId] = useState<SessionId | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const booted = useRef(false);

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

  return (
    <TooltipProvider delayDuration={400}>
      <div className="tyba-aurora flex h-screen flex-col text-tyba-text">
        {/* ---------- Header geral (arrastável; semáforos em overlay) ---------- */}
        <header
          data-tauri-drag-region
          className="tyba-glass flex h-11 shrink-0 items-center gap-1 border-b border-tyba-border pl-20 pr-3"
        >
          <IconAction
            label={sidebarOpen ? "Recolher painel" : "Expandir painel"}
            onClick={() => setSidebarOpen((v) => !v)}
          >
            <SidebarSimple size={18} />
          </IconAction>

          <span className="ml-1 flex select-none items-center gap-2.5">
            <img src={tybaMark} alt="" className="h-5 w-5" />
            <span className="text-[13px] font-bold tracking-[0.2em]">
              TYBA
            </span>
          </span>

          <div className="flex-1" data-tauri-drag-region />

          <IconAction label="Abrir pasta do projeto">
            <FolderOpen size={18} />
          </IconAction>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                aria-label="Notificações"
                className="size-7 text-tyba-text-muted hover:text-tyba-text"
              >
                <Bell size={18} />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-72">
              <DropdownMenuLabel className="tyba-label">
                Notificações
              </DropdownMenuLabel>
              <div className="px-2 py-6 text-center text-xs text-tyba-text-faint">
                Tudo em dia. Aprovações de sessões de agente chegam aqui.
              </div>
            </DropdownMenuContent>
          </DropdownMenu>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                aria-label="Conta"
                className="ml-1 rounded-full p-[1.5px]"
                style={{ background: "var(--tyba-gradient)" }}
              >
                <span className="flex size-6 items-center justify-center rounded-full bg-tyba-raised text-tyba-text-muted">
                  <User size={13} weight="bold" />
                </span>
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-48">
              <DropdownMenuLabel>Conta local</DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuItem disabled>Configurações</DropdownMenuItem>
              <DropdownMenuItem disabled>Sobre o TYBA</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </header>

        {/* ---------- Sidebar + conteúdo ---------- */}
        <div className="flex min-h-0 flex-1">
          <aside
            className={`tyba-glass flex shrink-0 flex-col border-r border-tyba-border transition-all duration-200 ${
              sidebarOpen ? "w-56" : "w-12"
            }`}
          >
            <span
              className={`tyba-label px-4 pt-4 ${sidebarOpen ? "" : "sr-only"}`}
            >
              Sessões
            </span>
            <nav
              className={`flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2.5 pb-3 ${
                sidebarOpen ? "mt-2" : "mt-3"
              }`}
            >
              {sessions.map((s) => {
                const isActive = s.id === activeId;
                return (
                  <button
                    key={s.id}
                    onClick={() => setActiveId(s.id)}
                    title={sidebarOpen ? undefined : s.title}
                    className={`group relative flex h-9 shrink-0 items-center gap-2.5 rounded-md text-[13px] transition-colors ${
                      sidebarOpen ? "px-2.5" : "justify-center px-0"
                    } ${
                      isActive
                        ? "bg-white/[.04] text-tyba-text"
                        : "text-tyba-text-faint hover:bg-tyba-surface hover:text-tyba-text-muted"
                    }`}
                  >
                    {/* linha viva: a sessão ativa está acesa */}
                    {isActive && (
                      <span
                        className="absolute left-0 top-2 bottom-2 w-0.5 rounded-full"
                        style={{ background: "var(--tyba-gradient-soft)" }}
                      />
                    )}
                    <TerminalWindow
                      size={18}
                      className={
                        isActive
                          ? "shrink-0 text-tyba-green [filter:drop-shadow(0_0_6px_rgba(124,197,68,.55))]"
                          : "shrink-0"
                      }
                    />
                    {sidebarOpen && (
                      <>
                        <span className="min-w-0 flex-1 truncate text-left">
                          {s.title}
                        </span>
                        <span
                          role="button"
                          aria-label="Fechar sessão"
                          onClick={(e) => {
                            e.stopPropagation();
                            void closeSession(s.id);
                          }}
                          className="rounded text-tyba-text-faint opacity-0 transition-opacity hover:text-tyba-text group-hover:opacity-100"
                        >
                          <X size={12} weight="bold" />
                        </span>
                      </>
                    )}
                  </button>
                );
              })}
              <Button
                variant="ghost"
                onClick={() => void newShell()}
                aria-label="Nova sessão"
                className={`mt-1 h-9 shrink-0 gap-2.5 text-[13px] font-normal text-tyba-text-faint hover:text-tyba-text ${
                  sidebarOpen ? "justify-start px-2.5" : "justify-center px-0"
                }`}
              >
                <Plus size={16} />
                {sidebarOpen && "Nova sessão"}
              </Button>
            </nav>
          </aside>

          <main className="relative min-h-0 min-w-0 flex-1">
            {sessions.length === 0 ? (
              <div className="flex h-full flex-col items-center justify-center gap-4">
                <img src={tybaMark} alt="" className="h-12 w-12 opacity-90" />
                <p className="text-sm text-tyba-text-faint">
                  Nenhuma sessão aberta.
                </p>
                <Button onClick={() => void newShell()}>
                  <Plus size={16} weight="bold" />
                  Nova sessão
                </Button>
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
