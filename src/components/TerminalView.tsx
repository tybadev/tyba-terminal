// TerminalView: xterm.js <-> PTY do core, com fit e resize.
// O xterm é imperativo; o React só gerencia o ciclo de vida.

import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

import {
  onPtyExit,
  onPtyOutput,
  resizeSession,
  writeToSession,
  type SessionId,
} from "../lib/ipc";

// Tema TYBA: canvas quase-preto da identidade, gradiente fica na marca —
// o terminal em si é disciplinado (cores ANSI legíveis, sem ruído).
const TYBA_THEME = {
  background: "#0a0a0c",
  foreground: "#e6e6ea",
  cursor: "#8b5cf6",
  cursorAccent: "#0a0a0c",
  selectionBackground: "#8b5cf640",
  black: "#1a1a1f",
  red: "#f472b6",
  green: "#a3e635",
  yellow: "#fbbf24",
  blue: "#60a5fa",
  magenta: "#a78bfa",
  cyan: "#2dd4bf",
  white: "#e6e6ea",
  brightBlack: "#52525b",
  brightRed: "#fb7185",
  brightGreen: "#bef264",
  brightYellow: "#fcd34d",
  brightBlue: "#93c5fd",
  brightMagenta: "#c4b5fd",
  brightCyan: "#5eead4",
  brightWhite: "#fafafa",
};

interface Props {
  sessionId: SessionId;
  active: boolean;
  onExit?: () => void;
}

export function TerminalView({ sessionId, active, onExit }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const term = new Terminal({
      theme: TYBA_THEME,
      fontFamily:
        '"JetBrains Mono", "Fira Code", ui-monospace, SFMono-Regular, Menlo, monospace',
      fontSize: 13,
      lineHeight: 1.35,
      cursorBlink: true,
      allowProposedApi: true,
      scrollback: 10_000,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    fit.fit();

    termRef.current = term;
    fitRef.current = fit;

    // teclado -> PTY
    const dataSub = term.onData((data) => {
      void writeToSession(sessionId, data);
    });

    // PTY -> tela (chunks já chegam batched do core)
    const unlisteners: Array<() => void> = [];
    void onPtyOutput(sessionId, (bytes) => term.write(bytes)).then((un) =>
      unlisteners.push(un),
    );
    void onPtyExit(sessionId, () => {
      term.write("\r\n\x1b[2m[sessão encerrada]\x1b[0m\r\n");
      onExit?.();
    }).then((un) => unlisteners.push(un));

    // resize: observa o container e propaga cols/rows pro PTY
    const ro = new ResizeObserver(() => {
      fit.fit();
      void resizeSession(sessionId, term.cols, term.rows);
    });
    ro.observe(el);
    void resizeSession(sessionId, term.cols, term.rows);

    return () => {
      ro.disconnect();
      dataSub.dispose();
      unlisteners.forEach((un) => un());
      term.dispose();
      termRef.current = null;
    };
    // sessionId é estável por instância do componente (key no pai)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  // foco ao ativar a tab
  useEffect(() => {
    if (active) {
      termRef.current?.focus();
      fitRef.current?.fit();
    }
  }, [active]);

  return (
    <div
      ref={containerRef}
      className="h-full w-full overflow-hidden bg-[#0a0a0c] p-2"
      style={{ display: active ? "block" : "none" }}
    />
  );
}
