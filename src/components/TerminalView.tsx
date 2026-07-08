import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

import i18n from "../i18n";

import {
  onPtyExit,
  onPtyOutput,
  resizeSession,
  sessionScrollback,
  writeToSession,
  type SessionId,
} from "../lib/ipc";
import { getTerminalTheme, onTerminalThemeChange } from "../theme";

function loadWebgl(term: Terminal, onLost: () => void): WebglAddon | null {
  try {
    const webgl = new WebglAddon();
    webgl.onContextLoss(() => {
      disposeWebgl(webgl);
      onLost();
    });
    term.loadAddon(webgl);
    return webgl;
  } catch {
    return null;
  }
}

function disposeWebgl(webgl: WebglAddon | null) {
  try {
    webgl?.dispose();
  } catch {
    return;
  }
}

interface Props {
  sessionId: SessionId;
  active: boolean;
  onExit?: () => void;
}

export function TerminalView({ sessionId, active, onExit }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const webglRef = useRef<WebglAddon | null>(null);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const theme = getTerminalTheme();
    const term = new Terminal({
      theme,
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

    el.style.backgroundColor = theme.background ?? "";
    const offTheme = onTerminalThemeChange((next) => {
      term.options.theme = next;
      el.style.backgroundColor = next.background ?? "";
    });

    termRef.current = term;
    fitRef.current = fit;

    // teclado -> PTY
    const dataSub = term.onData((data) => {
      void writeToSession(sessionId, data).catch(() => {});
    });

    const unlisteners: Array<() => void> = [];
    void sessionScrollback(sessionId)
      .then((snapshot) => {
        if (snapshot.length) term.write(snapshot);
      })
      .catch(() => {})
      .then(() => onPtyOutput(sessionId, (bytes) => term.write(bytes)))
      .then((un) => unlisteners.push(un));
    void onPtyExit(sessionId, () => {
      term.write(`\r\n\x1b[2m${i18n.t("sessionEnded")}\x1b[0m\r\n`);
      onExit?.();
    }).then((un) => unlisteners.push(un));

    // resize: observa o container e propaga cols/rows pro PTY
    const ro = new ResizeObserver(() => {
      fit.fit();
      void resizeSession(sessionId, term.cols, term.rows).catch(() => {});
    });
    ro.observe(el);
    void resizeSession(sessionId, term.cols, term.rows).catch(() => {});

    return () => {
      ro.disconnect();
      offTheme();
      dataSub.dispose();
      unlisteners.forEach((un) => un());
      disposeWebgl(webglRef.current);
      webglRef.current = null;
      term.dispose();
      termRef.current = null;
    };
    // sessionId é estável por instância do componente (key no pai)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    if (active) {
      term.focus();
      fitRef.current?.fit();
      if (!webglRef.current) {
        webglRef.current = loadWebgl(term, () => {
          webglRef.current = null;
        });
      }
    } else if (webglRef.current) {
      disposeWebgl(webglRef.current);
      webglRef.current = null;
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
