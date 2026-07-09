import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

import i18n from "../i18n";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  openExternalUrl,
  readClipboardText,
  writeClipboardText,
} from "../lib/clipboard";
import {
  onPtyExit,
  onPtyOutput,
  resizeSession,
  sessionScrollback,
  writeToSession,
  type SessionId,
} from "../lib/ipc";
import {
  nativePasteSuppressed,
  registerTerm,
  unregisterTerm,
} from "../lib/termRegistry";
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
  onPaste?: (sessionId: SessionId, text: string) => void;
  onSearch?: () => void;
  onSplit?: (kind: "v" | "h") => void;
}

export function TerminalView({
  sessionId,
  visible,
  focused,
  framed,
  rect,
  onExit,
  onFocus,
  onPaste,
  onSearch,
  onSplit,
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const webglRef = useRef<WebglAddon | null>(null);
  const onFocusRef = useRef(onFocus);
  onFocusRef.current = onFocus;
  const onPasteRef = useRef(onPaste);
  onPasteRef.current = onPaste;
  const [menuHasSelection, setMenuHasSelection] = useState(false);

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
      rightClickSelectsWord: true,
      macOptionClickForcesSelection: true,
      macOptionIsMeta: false,
      linkHandler: {
        activate: (_event, uri) => {
          void openExternalUrl(uri);
        },
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);

    const unicode11 = new Unicode11Addon();
    term.loadAddon(unicode11);
    term.unicode.activeVersion = "11";

    const search = new SearchAddon();
    term.loadAddon(search);

    const webLinks = new WebLinksAddon((_event, uri) => {
      void openExternalUrl(uri);
    });
    term.loadAddon(webLinks);

    term.open(el);
    fit.fit();

    registerTerm(sessionId, { term, search });

    el.style.backgroundColor = theme.background ?? "";
    const offTheme = onTerminalThemeChange((next) => {
      term.options.theme = next;
      el.style.backgroundColor = next.background ?? "";
    });

    termRef.current = term;
    fitRef.current = fit;

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
      const buffer = term.buffer.active;
      const wasAtBottom = buffer.viewportY === buffer.baseY;
      fit.fit();
      if (term.cols !== lastCols || term.rows !== lastRows) {
        const rowsChanged = term.rows !== lastRows;
        lastCols = term.cols;
        lastRows = term.rows;
        if (wasAtBottom && rowsChanged) term.scrollToBottom();
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
    const onNativePaste = (e: ClipboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (nativePasteSuppressed()) return;
      const text = e.clipboardData?.getData("text") ?? "";
      if (text) onPasteRef.current?.(sessionId, text);
    };
    window.addEventListener(RELAYOUT_EVENT, onRelayout);
    window.addEventListener(FONT_SIZE_EVENT, onFontSize);
    el.addEventListener("mousedown", onMouseDown);
    el.addEventListener("paste", onNativePaste, true);
    void resizeSession(sessionId, term.cols, term.rows).catch(() => {});

    return () => {
      if (timer !== null) window.clearTimeout(timer);
      ro.disconnect();
      window.removeEventListener(RELAYOUT_EVENT, onRelayout);
      window.removeEventListener(FONT_SIZE_EVENT, onFontSize);
      el.removeEventListener("mousedown", onMouseDown);
      el.removeEventListener("paste", onNativePaste, true);
      offTheme();
      dataSub.dispose();
      unlisteners.forEach((un) => un());
      unregisterTerm(sessionId);
      disposeWebgl(webglRef.current);
      webglRef.current = null;
      term.dispose();
      termRef.current = null;
    };
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

  const frameClass = framed ? "border border-tyba-border" : "";
  const frameStyle: React.CSSProperties =
    framed && focused
      ? {
          borderColor: "color-mix(in srgb, var(--tyba-green) 45%, transparent)",
          boxShadow:
            "0 0 0 1px color-mix(in srgb, var(--tyba-green) 25%, transparent), 0 0 14px -2px var(--tyba-glow-green, rgba(124,197,68,.4))",
        }
      : {};

  const selection = () => termRef.current?.getSelection() ?? "";

  const copySelection = (asMarkdown: boolean) => {
    const text = selection();
    if (!text) return;
    const payload = asMarkdown ? `\`\`\`\n${text}\n\`\`\`` : text;
    void writeClipboardText(payload).catch(() => {});
  };

  const pasteFromMenu = () => {
    void readClipboardText()
      .then((text) => {
        if (text) onPasteRef.current?.(sessionId, text);
      })
      .catch(() => {});
  };

  return (
    <ContextMenu
      onOpenChange={(o) => {
        if (o) setMenuHasSelection(termRef.current?.hasSelection() ?? false);
      }}
    >
      <ContextMenuTrigger asChild disabled={!visible}>
        <div
          ref={containerRef}
          className={`overflow-hidden rounded-[4px] bg-tyba-sunken px-2 pb-3 pt-2 ${frameClass}`}
          style={
            visible && rect
              ? {
                  position: "absolute",
                  left: `${rect.left}%`,
                  top: `${rect.top}%`,
                  width: `${rect.width}%`,
                  height: `${rect.height}%`,
                  ...frameStyle,
                }
              : { display: "none" }
          }
        />
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem
          disabled={!menuHasSelection}
          onSelect={() => copySelection(false)}
        >
          {i18n.t("copySelection")}
        </ContextMenuItem>
        <ContextMenuItem
          disabled={!menuHasSelection}
          onSelect={() => copySelection(true)}
        >
          {i18n.t("copyAsMarkdown")}
        </ContextMenuItem>
        <ContextMenuItem onSelect={pasteFromMenu}>
          {i18n.t("pasteClipboard")}
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={() => onSearch?.()}>
          {i18n.t("searchTerminal")}
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={() => onSplit?.("v")}>
          {i18n.t("splitRight")}
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => onSplit?.("h")}>
          {i18n.t("splitDown")}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}
