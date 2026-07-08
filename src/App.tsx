// TYBA — shell da aplicação: sidebar de sessões + terminal.
// Direção visual: "o gradiente é luz" — linha viva marca a sessão ativa,
// sidebar de vidro sobre canvas com aurora. Tokens em src/styles.css.

import { useCallback, useEffect, useRef, useState } from "react";
import { Plus, TerminalWindow, X } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { TerminalView } from "./components/TerminalView";
import {
  createSession,
  disposeSession,
  type Session,
  type SessionId,
} from "./lib/ipc";
import tybaMark from "./assets/tyba-mark.svg";

export default function App() {
  const [sessions, setSessions] = useState<Session[]>([]);
  const [activeId, setActiveId] = useState<SessionId | null>(null);
  const booted = useRef(false);

  const active = sessions.find((s) => s.id === activeId) ?? null;

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
    <div className="tyba-aurora flex h-screen text-tyba-text">
      <aside className="tyba-glass flex w-56 shrink-0 flex-col border-r border-tyba-border">
        {/* topo arrastável; altura cobre os semáforos do macOS (overlay) */}
        <div data-tauri-drag-region className="h-9 shrink-0" />
        <div
          data-tauri-drag-region
          className="flex select-none items-center gap-2.5 px-4 pb-5"
        >
          <img src={tybaMark} alt="" className="h-6 w-6" />
          <span className="text-sm font-bold tracking-[0.2em]">TYBA</span>
        </div>

        <span className="tyba-label px-4">Sessões</span>
        <nav className="mt-2 flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-3 pb-3">
          {sessions.map((s) => {
            const isActive = s.id === activeId;
            return (
              <button
                key={s.id}
                onClick={() => setActiveId(s.id)}
                className={`group relative flex h-9 shrink-0 items-center gap-2.5 rounded-md px-2.5 text-[13px] transition-colors ${
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
              </button>
            );
          })}
          <Button
            variant="ghost"
            onClick={() => void newShell()}
            className="mt-1 h-9 shrink-0 justify-start gap-2.5 px-2.5 text-[13px] font-normal text-tyba-text-faint hover:text-tyba-text"
          >
            <Plus size={16} />
            Nova sessão
          </Button>
        </nav>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        {/* barra do conteúdo: arrastável, mostra a sessão ativa */}
        <div
          data-tauri-drag-region
          className="flex h-9 shrink-0 select-none items-center border-b border-tyba-border px-4"
        >
          {active && (
            <span className="font-mono text-xs text-tyba-text-muted">
              {active.title}
            </span>
          )}
        </div>

        <main className="relative min-h-0 flex-1">
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
  );
}
