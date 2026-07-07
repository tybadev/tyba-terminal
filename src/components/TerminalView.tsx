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

// Tema TYBA derivado dos tokens do design system (src/styles.css / --tyba-*):
// canvas quase-preto, cursor no verde do terminal, ANSI na paleta da marca.
// Hexes duplicados aqui porque o xterm não lê CSS custom properties.
const TYBA_THEME = {
  background: "#070709", // --tyba-sunken: terminal mais fundo que o canvas
  foreground: "#f2f2f5", // --tyba-text
  cursor: "#7cc544", // --tyba-green: terminal é verde na identidade
  cursorAccent: "#070709",
  selectionBackground: "#7cc5444d",
  black: "#1a1a20", // --tyba-raised
  red: "#f0503c", // --tyba-red
  green: "#7cc544", // --tyba-green
  yellow: "#f5a93b", // --tyba-amber
  blue: "#4c7df0", // --tyba-blue
  magenta: "#ec4899", // --tyba-magenta
  cyan: "#2dd4bf", // --tyba-cyan
  white: "#f2f2f5", // --tyba-text
  brightBlack: "#6a6a76", // --tyba-text-faint
  brightRed: "#f4715f",
  brightGreen: "#a8e05f", // --tyba-lime
  brightYellow: "#f5c93b",
  brightBlue: "#7da3f5",
  brightMagenta: "#a78bfa", // --tyba-violet clareado
  brightCyan: "#5eead4",
  brightWhite: "#ffffff",
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
      className="h-full w-full overflow-hidden bg-tyba-sunken p-2"
      style={{ display: active ? "block" : "none" }}
    />
  );
}
