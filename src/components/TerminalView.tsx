import { useEffect, useRef, useState } from "react";
import { CircleNotch, ShieldSlash } from "@phosphor-icons/react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
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
  attachSession,
  detachSession,
  writeToSession,
  type SessionId,
  type ConnectionState,
} from "../lib/ipc";
import {
  nativePasteSuppressed,
  registerTerm,
  unregisterTerm,
} from "../lib/termRegistry";
import {
  createTerminalLinkProvider,
  hasOpenModifier,
} from "../lib/terminalLinks";
import { IS_MAC } from "../lib/platform";
import { getTerminalTheme, onTerminalThemeChange } from "../theme";

export const RELAYOUT_EVENT = "tyba:relayout";
export const FONT_SIZE_EVENT = "tyba:font-size";

const EXIT_BANNER_SETTLE_MS = 120;

const IS_WINDOWS = navigator.platform.toUpperCase().includes("WIN");

// O ConPTY passou a emitir as sequências de wrap corretas no build 21376 do
// Win11 (microsoft/terminal#405). Informar `backend`+`buildNumber` ao xterm
// desliga o reflow duplo que embaralha o terminal no resize: em vez de recalcular
// a quebra de linha por conta própria (heurística "última coluna não-branca"), o
// xterm passa a confiar nos marcadores do ConPTY — que no core roda com
// PSEUDOCONSOLE_RESIZE_QUIRK. Como a jaula ConPTY é alvo exclusivo do Win11, o
// piso 21376 é sempre satisfeito.
const WINDOWS_PTY: { backend: "conpty"; buildNumber: number } = {
  backend: "conpty",
  buildNumber: 21376,
};

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
  exited?: boolean;
  /**
   * O PTY morrer não encerra esta sessão — o core reata. Quem vive no host
   * (SSH) some da tela quando o cano cai e volta segundos depois; dizer
   * "sessão encerrada" aí é o oposto do que aconteceu, justo no instante em
   * que o dono teme ter perdido trabalho.
   */
  reattaches?: boolean;
  /** Sessão SSH: o handshake demora e a tela fica preta até o servidor falar. */
  connecting?: boolean;
  connection?: ConnectionState;
  onReconnect?: () => void;
  /** Devolve true quando a rajada consumiu a tecla (não vai para este PTY). */
  onBroadcastInput?: (data: string) => boolean;
  onExit?: () => void;
  onFocus?: () => void;
  onPaste?: (sessionId: SessionId, text: string) => void;
  onSearch?: () => void;
  /** `alternate` = vim/htop/less: a tela é do programa, o teclado também. */
  onAltScreen?: (alt: boolean) => void;
  onSplit?: (kind: "v" | "h") => void;
  /** Agente cru detectado no shell (F2 do detectar-agente-no-shell). */
  agentNotice?: { binary: string } | null;
  onReopenManaged?: () => void;
  onDismissNotice?: () => void;
}

export function TerminalView({
  sessionId,
  visible,
  focused,
  framed,
  rect,
  exited,
  reattaches,
  connecting,
  connection,
  onReconnect,
  onBroadcastInput,
  onExit,
  onFocus,
  onPaste,
  onSearch,
  onAltScreen,
  onSplit,
  agentNotice,
  onReopenManaged,
  onDismissNotice,
}: Props) {
  const [gotOutput, setGotOutput] = useState(false);
  // O onData é assinado uma vez no mount: sem ref, a rajada ficaria presa no
  // callback do primeiro render.
  const broadcastRef = useRef(onBroadcastInput);
  broadcastRef.current = onBroadcastInput;
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const webglRef = useRef<WebglAddon | null>(null);
  const onFocusRef = useRef(onFocus);
  onFocusRef.current = onFocus;
  const onPasteRef = useRef(onPaste);
  onPasteRef.current = onPaste;
  const onAltScreenRef = useRef(onAltScreen);
  onAltScreenRef.current = onAltScreen;
  const showExitBannerRef = useRef<(() => void) | null>(null);
  const reattachesRef = useRef(false);
  reattachesRef.current = Boolean(reattaches);
  const visibleRef = useRef(visible);
  visibleRef.current = visible;
  const syncWebglRef = useRef<(() => void) | null>(null);
  const hoveredLinkRef = useRef<string | null>(null);
  const [menuHasSelection, setMenuHasSelection] = useState(false);
  const [menuMouseMode, setMenuMouseMode] = useState(false);
  const [menuLink, setMenuLink] = useState<string | null>(null);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const revealLink = (uri: string) => {
      hoveredLinkRef.current = uri;
      el.title = uri;
    };
    const clearLink = () => {
      hoveredLinkRef.current = null;
      if (el.title) el.removeAttribute("title");
    };
    const activateLink = (event: MouseEvent, uri: string) => {
      if (!hasOpenModifier(event, IS_MAC)) return;
      void openExternalUrl(uri);
    };

    const theme = getTerminalTheme();
    const term = new Terminal({
      theme,
      fontFamily:
        '"JetBrains Mono", "Symbols Nerd Font Mono", "Fira Code", ui-monospace, SFMono-Regular, Menlo, monospace',
      fontSize: defaultFontSize,
      lineHeight: 1.35,
      cursorBlink: true,
      allowProposedApi: true,
      scrollback: 10_000,
      rightClickSelectsWord: true,
      macOptionClickForcesSelection: true,
      macOptionIsMeta: false,
      ...(IS_WINDOWS ? { windowsPty: WINDOWS_PTY } : {}),
      linkHandler: {
        activate: (event, uri) => activateLink(event, uri),
        hover: (_event, uri) => revealLink(uri),
        leave: () => clearLink(),
        allowNonHttpProtocols: false,
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);

    const unicode11 = new Unicode11Addon();
    term.loadAddon(unicode11);
    term.unicode.activeVersion = "11";

    const search = new SearchAddon();
    term.loadAddon(search);

    const linkProvider = term.registerLinkProvider(
      createTerminalLinkProvider(term, {
        activate: activateLink,
        hover: (_event, uri) => revealLink(uri),
        leave: () => clearLink(),
      }),
    );

    let disposed = false;
    let opened = false;
    // O StrictMode (dev) monta → desmonta → remonta o efeito no mesmo tick. Se
    // `term.open()` rodar na montagem DESCARTÁVEL, o xterm agenda um `setTimeout`
    // interno de layout que dispara DEPOIS do dispose — e lê `dimensions` de um
    // renderer já morto (`RenderService._renderer.value` undefined → estoura em
    // `Viewport.syncScrollArea`), derrubando o terminal. Por isso adiamos o open
    // para um `requestAnimationFrame` e o cancelamos no cleanup: a montagem
    // descartável nunca chega a abrir o xterm. No Windows/WebView2 o timing bate
    // (por isso "nada, zero"); no mac não — mas o fix vale pros dois. O fit inicial
    // só roda se o elemento já tem tamanho; senão o ResizeObserver refaz.
    let openFrame = requestAnimationFrame(() => {
      openFrame = 0;
      if (disposed) return;
      term.open(el);
      opened = true;
      syncWebglRef.current?.();
      if (el.offsetWidth > 0 && el.offsetHeight > 0) {
        try {
          fit.fit();
        } catch {
          /* dimensões ainda não prontas — o ResizeObserver refaz o fit */
        }
      }
    });

    registerTerm(sessionId, { term, search });

    el.style.backgroundColor = theme.background ?? "";
    const offTheme = onTerminalThemeChange((next) => {
      term.options.theme = next;
      el.style.backgroundColor = next.background ?? "";
    });

    termRef.current = term;
    fitRef.current = fit;

    const bufferSub = term.buffer.onBufferChange((buffer) => {
      onAltScreenRef.current?.(buffer.type === "alternate");
    });

    const dataSub = term.onData((data) => {
      // Broadcast intercepta antes do PTY: a tecla vai para o conjunto inteiro,
      // e o Enter passa pelo core (que barra vermelho sem confirmação).
      if (broadcastRef.current?.(data)) return;
      void writeToSession(sessionId, data).catch(() => {});
    });

    void document.fonts.load('12px "Symbols Nerd Font Mono"').then((faces) => {
      if (!disposed && opened && faces.length > 0) term.clearTextureAtlas();
    });
    let holdsAttachment = false;
    const unlisteners: Array<() => void> = [];
    const addUnlistener = (un: () => void) => {
      if (disposed) un();
      else unlisteners.push(un);
    };
    const releaseAttachment = () => {
      if (!holdsAttachment) return;
      holdsAttachment = false;
      void detachSession(sessionId).catch(() => {});
    };

    const attached = (async () => {
      addUnlistener(
        await onPtyOutput(sessionId, (bytes) => {
          if (disposed) return;
          term.write(bytes);
          setGotOutput(true);
        }),
      );
      if (disposed) return;
      await attachSession(sessionId);
      holdsAttachment = true;
      if (disposed) releaseAttachment();
    })().catch(() => {});

    let exitBannerShown = false;
    const showExitBanner = () => {
      void attached.then(() => {
        if (disposed || exitBannerShown || reattachesRef.current) return;
        exitBannerShown = true;
        term.write(`\r\n\x1b[2m${i18n.t("sessionEnded")}\x1b[0m\r\n`);
      });
    };
    showExitBannerRef.current = showExitBanner;

    void onPtyExit(sessionId, () => {
      void attached.then(() => {
        if (disposed) return;
        showExitBanner();
        onExit?.();
      });
    }).then(addUnlistener);

    let lastCols = -1;
    let lastRows = -1;
    let timer: number | null = null;
    const refit = () => {
      timer = null;
      if (!opened) return; // o rAF de open ainda não rodou — nada a ajustar
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
    const syncWebgl = () => {
      if (disposed || !opened) return;
      if (visibleRef.current) {
        if (webglRef.current) return;
        webglRef.current = loadWebgl(term, () => {
          webglRef.current = null;
          refit();
        });
        if (webglRef.current) refit();
      } else if (webglRef.current) {
        disposeWebgl(webglRef.current);
        webglRef.current = null;
      }
    };
    syncWebglRef.current = syncWebgl;
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

    return () => {
      disposed = true;
      if (openFrame) cancelAnimationFrame(openFrame);
      releaseAttachment();
      if (timer !== null) window.clearTimeout(timer);
      ro.disconnect();
      window.removeEventListener(RELAYOUT_EVENT, onRelayout);
      window.removeEventListener(FONT_SIZE_EVENT, onFontSize);
      el.removeEventListener("mousedown", onMouseDown);
      el.removeEventListener("paste", onNativePaste, true);
      offTheme();
      linkProvider.dispose();
      bufferSub.dispose();
      dataSub.dispose();
      unlisteners.forEach((un) => un());
      unregisterTerm(sessionId);
      showExitBannerRef.current = null;
      syncWebglRef.current = null;
      disposeWebgl(webglRef.current);
      webglRef.current = null;
      term.dispose();
      termRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  useEffect(() => {
    if (!exited) return;
    const timer = window.setTimeout(
      () => showExitBannerRef.current?.(),
      EXIT_BANNER_SETTLE_MS,
    );
    return () => window.clearTimeout(timer);
  }, [exited, sessionId]);

  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    if (focused && visible) term.focus();
    syncWebglRef.current?.();
  }, [focused, visible]);

  // Ao ficar visível (troca de aba), o container sai de `display:none` e ganha
  // tamanho, mas o canvas do xterm ainda está nas dimensões antigas — a CSS o
  // estica até o ResizeObserver refazer o fit (com debounce de 80ms). Refaz o fit
  // e re-renderiza JÁ (no próximo frame) pra não aparecer o frame esticado.
  useEffect(() => {
    if (!visible) return;
    const term = termRef.current;
    const el = containerRef.current;
    if (!term || !el) return;
    const raf = requestAnimationFrame(() => {
      if (el.offsetWidth === 0 || el.offsetHeight === 0) return;
      try {
        fitRef.current?.fit();
      } catch {
        /* dimensões ainda não prontas — o ResizeObserver refaz */
      }
      term.refresh(0, term.rows - 1);
    });
    return () => cancelAnimationFrame(raf);
  }, [visible, rect]);

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

  const showConnecting = Boolean(connecting) && !gotOutput && visible && !!rect;
  const droppedPipe = connection === "dropped";
  const showPipe =
    (connection === "reconnecting" || droppedPipe) && visible && !!rect;

  return (
    <>
    {agentNotice && rect && visible && !exited && (
      <div
        className="z-10 flex items-center gap-2 border-b border-tyba-amber/30 bg-tyba-sunken px-2 py-1"
        style={{
          position: "absolute",
          left: `${rect.left}%`,
          top: `${rect.top}%`,
          width: `${rect.width}%`,
        }}
      >
        <ShieldSlash size={12} weight="fill" className="shrink-0 text-tyba-amber" />
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-tyba-text-muted">
          {i18n.t("shellAgentNotice", { binary: agentNotice.binary })}
        </span>
        {onReopenManaged && (
          <button
            type="button"
            onClick={onReopenManaged}
            className="shrink-0 rounded-[3px] border border-tyba-amber/40 px-2 py-0.5 font-mono text-[10px] text-tyba-amber hover:bg-tyba-amber/10"
          >
            {i18n.t("shellAgentReopen")}
          </button>
        )}
        {onDismissNotice && (
          <button
            type="button"
            onClick={onDismissNotice}
            className="shrink-0 rounded-[3px] px-1.5 py-0.5 font-mono text-[10px] text-tyba-text-faint hover:text-tyba-text"
          >
            {i18n.t("shellAgentIgnore")}
          </button>
        )}
      </div>
    )}
    {showPipe && rect && (
      <div
        className="z-10 flex flex-col items-center justify-center gap-2 rounded-[4px] bg-tyba-sunken/90"
        style={{
          position: "absolute",
          left: `${rect.left}%`,
          top: `${rect.top}%`,
          width: `${rect.width}%`,
          height: `${rect.height}%`,
        }}
      >
        {!droppedPipe && (
          <CircleNotch
            size={14}
            className="animate-spin text-tyba-text-faint"
            weight="bold"
          />
        )}
        <span className="font-mono text-[11px] text-tyba-text-faint">
          {i18n.t(droppedPipe ? "sshDropped" : "sshReconnecting")}
        </span>
        <span className="font-mono text-[10px] text-tyba-text-faint/70">
          {i18n.t("sshSessionAlive")}
        </span>
        {droppedPipe && onReconnect && (
          <button
            type="button"
            onClick={onReconnect}
            className="mt-1 rounded-[3px] border border-tyba-border px-2 py-1 font-mono text-[10px] text-tyba-text hover:bg-tyba-raised"
          >
            {i18n.t("sshReconnect")}
          </button>
        )}
      </div>
    )}
    {showConnecting && rect && (
      <div
        className="pointer-events-none z-10 flex items-center justify-center gap-2 rounded-[4px] bg-tyba-sunken"
        style={{
          position: "absolute",
          left: `${rect.left}%`,
          top: `${rect.top}%`,
          width: `${rect.width}%`,
          height: `${rect.height}%`,
        }}
      >
        <CircleNotch
          size={14}
          className="animate-spin text-tyba-text-faint"
          weight="bold"
        />
        <span className="font-mono text-[11px] text-tyba-text-faint">
          {i18n.t("sshConnecting")}
        </span>
      </div>
    )}
    <ContextMenu
      onOpenChange={(o) => {
        if (!o) return;
        const term = termRef.current;
        setMenuHasSelection(term?.hasSelection() ?? false);
        setMenuMouseMode(
          (term?.modes.mouseTrackingMode ?? "none") !== "none",
        );
        setMenuLink(hoveredLinkRef.current);
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
        {menuLink && (
          <>
            <ContextMenuItem
              onSelect={() =>
                void writeClipboardText(menuLink).catch(() => {})
              }
            >
              {i18n.t("copyLink")}
            </ContextMenuItem>
            <ContextMenuSeparator />
          </>
        )}
        {menuMouseMode && !menuHasSelection ? (
          <ContextMenuItem disabled>
            {i18n.t(IS_MAC ? "selectionHintMac" : "selectionHintOther")}
          </ContextMenuItem>
        ) : (
          <>
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
          </>
        )}
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
    </>
  );
}
