// TYBA — shell da aplicação: barra de tabs + terminais.
// Fase 2 do roadmap traz sidebar/inbox; por ora, tabs horizontais.

import { useCallback, useEffect, useRef, useState } from "react";

import { TerminalView } from "./components/TerminalView";
import {
  createSession,
  disposeSession,
  type Session,
  type SessionId,
} from "./lib/ipc";

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
    <div className="flex h-screen flex-col bg-[#0a0a0c] text-zinc-200">
      <header
        data-tauri-drag-region
        className="flex h-10 shrink-0 items-center gap-2 border-b border-zinc-800/80 px-3"
      >
        <span className="brand select-none text-sm font-semibold tracking-[0.2em]">
          TYBA
        </span>
        <div className="ml-3 flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
          {sessions.map((s) => (
            <button
              key={s.id}
              onClick={() => setActiveId(s.id)}
              className={`group flex shrink-0 items-center gap-2 rounded-md px-2.5 py-1 text-xs transition-colors ${
                s.id === activeId
                  ? "bg-zinc-800 text-zinc-100"
                  : "text-zinc-500 hover:bg-zinc-900 hover:text-zinc-300"
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
                className="rounded px-1 text-zinc-600 opacity-0 transition-opacity hover:text-zinc-200 group-hover:opacity-100"
              >
                ×
              </span>
            </button>
          ))}
        </div>
        <button
          onClick={() => void newShell()}
          aria-label="Nova sessão"
          className="shrink-0 rounded-md px-2 py-1 text-sm text-zinc-500 transition-colors hover:bg-zinc-900 hover:text-zinc-200"
        >
          +
        </button>
      </header>

      <main className="relative min-h-0 flex-1">
        {sessions.length === 0 ? (
          <div className="flex h-full items-center justify-center text-sm text-zinc-600">
            Nenhuma sessão. Crie uma com{" "}
            <kbd className="mx-1 rounded bg-zinc-900 px-1.5 py-0.5">+</kbd>
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
