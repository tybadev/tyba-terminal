// TYBA — shell da aplicação: barra de tabs + terminais.
// Fase 2 do roadmap traz sidebar/inbox; por ora, tabs horizontais.
// Direção visual: "o gradiente é luz" — linha viva marca a tab ativa,
// aurora no canvas, header de vidro. Tokens em src/styles.css.

import { useCallback, useEffect, useRef, useState } from "react";
import { Plus, X } from "@phosphor-icons/react";

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
    <div className="tyba-aurora flex h-screen flex-col text-tyba-text">
      {/* pl-20: espaço dos semáforos do macOS (title bar em overlay) */}
      <header
        data-tauri-drag-region
        className="tyba-glass flex h-10 shrink-0 items-center gap-2 border-b border-tyba-border pl-20 pr-3"
      >
        <span className="flex select-none items-center gap-2">
          <img src={tybaMark} alt="" className="h-5 w-5" />
          <span className="text-sm font-bold tracking-[0.2em]">TYBA</span>
        </span>
        <div className="ml-3 flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
          {sessions.map((s) => (
            <button
              key={s.id}
              onClick={() => setActiveId(s.id)}
              className={`group relative flex shrink-0 items-center gap-2 rounded-md px-2.5 py-1 text-xs transition-colors ${
                s.id === activeId
                  ? "bg-white/[.04] text-tyba-text"
                  : "text-tyba-text-faint hover:bg-tyba-surface hover:text-tyba-text-muted"
              }`}
            >
              {s.title}
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
              {s.id === activeId && (
                <span className="tyba-flow-line absolute inset-x-2 -bottom-px" />
              )}
            </button>
          ))}
        </div>
        <Button
          variant="ghost"
          size="icon"
          onClick={() => void newShell()}
          aria-label="Nova sessão"
          className="size-7 shrink-0 text-tyba-text-faint hover:text-tyba-text"
        >
          <Plus size={16} />
        </Button>
      </header>

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
  );
}
