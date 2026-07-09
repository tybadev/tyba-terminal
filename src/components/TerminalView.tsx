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

export const RELAYOUT_EVENT = "tyba:relayout";
export const FONT_SIZE_EVENT = "tyba:font-size";

export function requestTerminalRelayout() {
  requestAnimationFrame(() => window.dispatchEvent(new Event(RELAYOUT_EVENT)));
}

let defaultFontSize = 13;

export function setDefaultFontSize(size: number) {
  if (size >= 10 && size <= 20) defaultFontSize = size;
}

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

export interface PaneRectStyle {
  left: number;
  top: number;
  width: number;
  height: number;
}

interface Props {
  sessionId: SessionId;
  visible: boolean;
  focused: boolean;
  framed: boolean;
  rect: PaneRectStyle | null;
  onExit?: () => void;
  onFocus?: () => void;
}

export function TerminalView({
  sessionId,
  visible,
  focused,
  framed,
  rect,
  onExit,
  onFocus,
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const webglRef = useRef<WebglAddon | null>(null);
  const onFocusRef = useRef(onFocus);
  onFocusRef.current = onFocus;

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const theme = getTerminalTheme();
    const term = new Terminal({
      theme,
      fontFamily:
        '"JetBrains Mono", "Fira Code", ui-monospace, SFMono-Regular, Menlo, monospace',
      fontSize: defaultFontSize,
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
    let attached = false;
    void onPtyOutput(sessionId, (bytes) => {
      if (attached) term.write(bytes);
    })
      .then((un) => {
        unlisteners.push(un);
        return sessionScrollback(sessionId);
      })
      .then((snapshot) => {
        if (snapshot.length) term.write(snapshot);
      })
      .catch(() => {})
      .then(() => {
        attached = true;
      });
    void onPtyExit(sessionId, () => {
      term.write(`\r\n\x1b[2m${i18n.t("sessionEnded")}\x1b[0m\r\n`);
      onExit?.();
    }).then((un) => unlisteners.push(un));

    let lastCols = term.cols;
    let lastRows = term.rows;
    let timer: number | null = null;
    const refit = () => {
      timer = null;
      if (el.offsetWidth === 0 || el.offsetHeight === 0) return;
      fit.fit();
      if (term.cols !== lastCols || term.rows !== lastRows) {
        lastCols = term.cols;
        lastRows = term.rows;
        term.scrollToBottom();
        void resizeSession(sessionId, term.cols, term.rows).catch(() => {});
      }
    };
    const schedule = () => {
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(refit, 80);
    };
    const ro = new ResizeObserver(schedule);
    ro.observe(el);
    const onRelayout = () => schedule();
    const onFontSize = (e: Event) => {
      const size = (e as CustomEvent<number>).detail;
      if (typeof size === "number" && size >= 10 && size <= 20) {
        term.options.fontSize = size;
        schedule();
      }
    };
    const onMouseDown = () => onFocusRef.current?.();
    window.addEventListener(RELAYOUT_EVENT, onRelayout);
    window.addEventListener(FONT_SIZE_EVENT, onFontSize);
    el.addEventListener("mousedown", onMouseDown);
    void resizeSession(sessionId, term.cols, term.rows).catch(() => {});

    return () => {
      if (timer !== null) window.clearTimeout(timer);
      ro.disconnect();
      window.removeEventListener(RELAYOUT_EVENT, onRelayout);
      window.removeEventListener(FONT_SIZE_EVENT, onFontSize);
      el.removeEventListener("mousedown", onMouseDown);
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
    if (focused && visible) {
      term.focus();
      if (!webglRef.current) {
        webglRef.current = loadWebgl(term, () => {
          webglRef.current = null;
        });
      }
    } else if (webglRef.current) {
      disposeWebgl(webglRef.current);
      webglRef.current = null;
    }
  }, [focused, visible]);

  const frameClass = framed
    ? focused
      ? "border border-tyba-border-strong"
      : "border border-tyba-border"
    : "";

  return (
    <div
      ref={containerRef}
      className={`overflow-hidden rounded-[4px] bg-tyba-sunken p-2 ${frameClass}`}
      style={
        visible && rect
          ? {
              position: "absolute",
              left: `${rect.left}%`,
              top: `${rect.top}%`,
              width: `${rect.width}%`,
              height: `${rect.height}%`,
            }
          : { display: "none" }
      }
    />
  );
}
