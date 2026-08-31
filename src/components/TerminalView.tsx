import { useEffect, useRef, useState } from "react";
import {
  ArrowCounterClockwise,
  CircleNotch,
  ShieldSlash,
} from "@phosphor-icons/react";
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
import { hiddenFraction, sameRect, usedRowsFromLastLine } from "../lib/liveSeam";
import { isArrowKey } from "../lib/commandLine";
import { getTerminalTheme, onTerminalThemeChange } from "../theme";

export const RELAYOUT_EVENT = "tyba:relayout";

/**
 * Padding vertical da caixa do terminal, em px.
 *
 * Mora aqui, e não nas classes, porque o recorte da faixa ao vivo precisa
 * descontar exatamente este valor: as linhas ocupam a caixa MENOS ele, e
 * recortar pela altura cheia parte a última linha ao meio. Duas fontes de
 * verdade para o mesmo número foi como esse bug nasceu.
 */
/**
 * Recuo lateral do bloco em execução, em px.
 *
 * Sem ele o bloco ativo vai de ponta a ponta do painel enquanto os cartões
 * parados respeitam a margem da lista — o em execução fica encostado nas
 * bordas e destoa de todos os outros.
 *
 Encolhe a CAIXA, e o fundo do painel aparece atrás — é ele que dá a margem,
 * do mesmo jeito que a lista dá a dos cartões. Virar padding em vez de margem
 * faria o terminal pintar até a borda do painel e a área fora da moldura
 * mostraria a cor dele, como se o bloco não terminasse ali.
 *
 * Entra na largura, então mexe no número de colunas. É um ajuste de LAYOUT,
 * constante durante a sessão: a regra que não pode cair é o PTY não ser
 * redimensionado POR COMANDO, e essa continua de pé.
 */
export const LIVE_INSET_X_PX = 8;
export const LIVE_PAD_TOP_PX = 8;
export const LIVE_PAD_BOTTOM_PX = 12;
export const LIVE_PAD_Y_PX = LIVE_PAD_TOP_PX + LIVE_PAD_BOTTOM_PX;
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

/**
 * Entrelinha do terminal — e a dos blocos tem de ser a MESMA.
 *
 * O corpo do bloco é a mesma saída que estava no terminal um instante antes.
 * Métrica diferente faz o texto mudar de tamanho ao virar cartão, que é a
 * emenda voltando por outro caminho.
 */
export const TERMINAL_LINE_HEIGHT = 1.35;

/**
 * Largura de uma célula em fração do tamanho da fonte, só como ponto de partida.
 *
 * Vale enquanto o terminal daquela sessão não mediu a sua — 0,6 é o avanço da
 * JetBrains Mono, a fonte padrão. Serve para a primeira estimativa não nascer
 * sem largura nenhuma; o valor medido chega no primeiro render e manda.
 */
export const TERMINAL_CELL_WIDTH = 0.6;

/**
 * Muda o tamanho da fonte do terminal e ANUNCIA.
 *
 * O anúncio faz parte da troca: os blocos usam esta mesma métrica, e quem
 * apenas guardasse o valor deixaria o corpo dos cartões num tamanho e o
 * terminal noutro. Foi o que aconteceu com a preferência lida no boot, que
 * mudava o número sem avisar ninguém.
 */
export function setDefaultFontSize(size: number) {
  if (size < 10 || size > 20 || size === defaultFontSize) return;
  defaultFontSize = size;
  window.dispatchEvent(new CustomEvent(FONT_SIZE_EVENT, { detail: size }));
}

/**
 * O tamanho de fonte do terminal, que é preferência do dono.
 *
 * Os blocos leem daqui em vez de fixar um número: com o corpo do cartão preso
 * em 13px, aumentar a fonte do terminal fazia a saída encolher ao virar bloco.
 */
export function getDefaultFontSize(): number {
  return defaultFontSize;
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
  /**
   * A linha do TYBA é a dona do teclado agora.
   *
   * O xterm precisa virar somente-leitura: senão são DUAS entradas para a mesma
   * linha, e o que for digitado aqui vai direto ao shell sem passar pela caixa,
   * pelo histórico nem pela confirmação de paste.
   */
  readOnly?: boolean;
  /** Clique sem seleção devolve o foco para a linha. */
  onReclaimFocus?: () => void;
  onSplit?: (kind: "v" | "h") => void;
  /** Agente cru detectado no shell (F2 do detectar-agente-no-shell). */
  agentNotice?: { binary: string } | null;
  onReopenManaged?: () => void;
  onDismissNotice?: () => void;
  /**
   * Convite de retomar a conversa nativa do agente numa sessão que morreu com o
   * app anterior. Só o core decide se ele aparece — ver `canResumeAgentSession`.
   *
   * O par do `agentNotice`, no outro lado da vida da sessão: aquele avisa de
   * agente vivo fora do gate, este oferece religar um agente morto. Por isso
   * este só existe com `exited`, e os dois nunca dividem a tela.
   */
  resumeNotice?: { binary: string } | null;
  onResumeAgent?: () => void;
  onDismissResume?: () => void;
  /**
   * Quanto da tela a saída do comando em curso ocupa, de 0 a 1.
   *
   * Recorta a faixa ao vivo na altura da saída real, para o cartão do bloco
   * nascer onde ela estava. `undefined` mostra o terminal inteiro.
   *
   * Nunca vira tamanho: entra como `transform` e `clip-path`, que ficam de fora
   * do layout e por isso não acordam o `ResizeObserver` que redimensionaria o
   * PTY. Trocar isto por altura é reintroduzir o `vim` reabrindo torto.
   */
  liveUsed?: number;
  /** Mede a saída em curso para {@link Props.liveUsed} — ver `usedFraction`. */
  onLiveRows?: (usedRows: number, totalRows: number, scrolled: boolean) => void;
  /**
   * Altura real de uma linha desenhada, em px.
   *
   * O corpo dos blocos usa este valor: calcular a partir de `lineHeight` dá
   * outro número, porque o xterm multiplica pela altura do glifo e o CSS pelo
   * `font-size`.
   */
  onLineHeight?: (px: number) => void;
  /**
   * Largura real de uma célula, em px — a mesma medida, no outro eixo.
   *
   * A estimativa de altura de um bloco precisa saber quantos caracteres cabem
   * na linha, porque o corpo do cartão quebra o texto. Medida pelo mesmo motivo
   * da altura: o avanço do glifo numa Nerd Font não é o que uma conta a partir
   * do `font-size` daria.
   */
  onCellWidth?: (px: number) => void;
  /**
   * Segurar as setas em vez de mandá-las ao PTY. Ver `swallowsArrow`.
   *
   * Vale só enquanto o tty entrega linhas: ali a seta não serve ao programa e
   * ainda é ecoada, virando `^[[A` na saída que o bloco grava no disco.
   */
  swallowArrows?: boolean;
}

export function TerminalView({
  sessionId,
  visible,
  focused,
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
  readOnly,
  onReclaimFocus,
  onSplit,
  agentNotice,
  onReopenManaged,
  onDismissNotice,
  resumeNotice,
  onResumeAgent,
  onDismissResume,
  liveUsed,
  onLiveRows,
  onLineHeight,
  onCellWidth,
  swallowArrows,
}: Props) {
  const [gotOutput, setGotOutput] = useState(false);
  // O onData é assinado uma vez no mount: sem ref, a rajada ficaria presa no
  // callback do primeiro render.
  const broadcastRef = useRef(onBroadcastInput);
  broadcastRef.current = onBroadcastInput;
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const webglRef = useRef<WebglAddon | null>(null);
  const onFocusRef = useRef(onFocus);
  onFocusRef.current = onFocus;
  const onPasteRef = useRef(onPaste);
  onPasteRef.current = onPaste;
  const onAltScreenRef = useRef(onAltScreen);
  onAltScreenRef.current = onAltScreen;
  const onReclaimFocusRef = useRef(onReclaimFocus);
  onReclaimFocusRef.current = onReclaimFocus;
  const readOnlyRef = useRef(readOnly);
  readOnlyRef.current = readOnly;
  const showExitBannerRef = useRef<(() => void) | null>(null);
  const reattachesRef = useRef(false);
  reattachesRef.current = Boolean(reattaches);
  const visibleRef = useRef(visible);
  visibleRef.current = visible;
  const syncWebglRef = useRef<(() => void) | null>(null);
  const onLiveRowsRef = useRef(onLiveRows);
  onLiveRowsRef.current = onLiveRows;
  const onLineHeightRef = useRef(onLineHeight);
  onLineHeightRef.current = onLineHeight;
  const onCellWidthRef = useRef(onCellWidth);
  onCellWidthRef.current = onCellWidth;
  const measureLineHeightRef = useRef<(() => void) | null>(null);
  // Único caminho que compara `cols`/`rows` contra o PTY e avisa
  // `resizeSession` — todo fit() cru passa por aqui, nunca chama
  // `fit.fit()` sozinho. Ver D4 na tech-spec.
  const refitRef = useRef<(() => void) | null>(null);
  // Última geometria (por VALOR) em que o efeito de `[visible, rect]`
  // realmente refez o fit. `rect` é objeto literal novo a cada render do
  // App — comparar por identidade faria o fit rodar por frame.
  const lastFitVisibleRef = useRef(false);
  const lastFitRectRef = useRef<PaneRectStyle | null>(null);
  // O handler de tecla é assinado uma vez no mount: sem ref, ficaria preso no
  // valor do primeiro render e a seta pararia de acompanhar o modo do tty.
  const swallowArrowsRef = useRef(false);
  swallowArrowsRef.current = Boolean(swallowArrows);
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
      // Por `refit()`, não `fit.fit()` cru: é o único jeito do PTY saber o
      // tamanho inicial sem esperar os ~80ms de debounce do ResizeObserver.
      // Ver D4 na tech-spec — todo fit avisa o PTY.
      refitRef.current?.();
      // Mede já na abertura: os blocos precisam da altura da linha desde o
      // primeiro render, e esperar o `onRender` deixaria a lista com a altura
      // provisória enquanto nenhum comando rodasse.
      measureLineHeightRef.current?.();
    });

    registerTerm(sessionId, { term, search });

    el.style.backgroundColor = theme.background ?? "";
    const offTheme = onTerminalThemeChange((next) => {
      term.options.theme = next;
      el.style.backgroundColor = next.background ?? "";
    });

    termRef.current = term;

    const bufferSub = term.buffer.onBufferChange((buffer) => {
      onAltScreenRef.current?.(buffer.type === "alternate");
    });

    // A seta morre aqui quando o tty está em modo linha — ver `swallowsArrow`.
    //
    // No evento de TECLADO, não no `onData`: ali a seta já virou `\x1b[A`, que
    // é indistinguível de um ESC vindo de paste ou de rajada. Barrar por bytes
    // engoliria escape legítimo de outra origem.
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown") return true;
      if (!isArrowKey(event.key)) return true;
      return !swallowArrowsRef.current;
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

    // O xterm mede o glifo UMA VEZ, no `term.open()`, e não tem hook de
    // font-load: se a JetBrains Mono embarcada ainda não tinha terminado de
    // carregar naquele instante, ele fica preso na métrica do fallback pelo
    // resto da sessão. Reatribuir a option força o `CharSizeService` interno
    // a remedir — mas o setter do xterm ignora valor IGUAL ao atual (não
    // dispara nada), por isso o vai-e-volta: o espaço extra é invisível pro
    // CSS (trailing whitespace num valor de `font-family` é ignorado), mas
    // muda a STRING o bastante pra passar no `!==` do xterm duas vezes. Ver
    // D2 na tech-spec.
    const forceRemeasure = () => {
      if (disposed || !opened) return;
      const family = term.options.fontFamily;
      term.options.fontFamily = `${family} `;
      term.options.fontFamily = family;
      term.clearTextureAtlas();
      refitRef.current?.();
    };
    void document.fonts.ready.then(forceRemeasure);
    // `ready` cobre o carregamento em curso na abertura; famílias que só
    // entram em uso depois (bold/italic do primeiro texto nesse peso, por
    // exemplo) dependem deste evento, que dispara a cada lote que termina.
    const onFontsLoadingDone = () => forceRemeasure();
    document.fonts.addEventListener("loadingdone", onFontsLoadingDone);

    // Sem escala fracionária (GNOME 1.25/1.5/1.75) ou ao arrastar a janela
    // entre monitores, `css.cell.height` muda até 0,8px/linha — ~1 linha
    // inteira a cada ~30 — e NADA acorda: o container não muda de tamanho
    // CSS, o `ResizeObserver` não dispara, e a última linha some sob o
    // `overflow-hidden`. `matchMedia("(resolution: <dpr>dppx)")` é o padrão
    // canônico pra ouvir DPR: a query só dispara UMA VEZ (fica falsa depois),
    // por isso se re-arma a cada disparo com o DPR atual. Ver D3 na tech-spec.
    let dprQuery: MediaQueryList | null = null;
    const armDprListener = () => {
      dprQuery = window.matchMedia(
        `(resolution: ${window.devicePixelRatio}dppx)`,
      );
      dprQuery.addEventListener("change", onDprChange);
    };
    const onDprChange = () => {
      dprQuery?.removeEventListener("change", onDprChange);
      forceRemeasure();
      armDprListener();
    };
    armDprListener();
    addUnlistener(() => {
      document.fonts.removeEventListener("loadingdone", onFontsLoadingDone);
      dprQuery?.removeEventListener("change", onDprChange);
    });

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

    // Mede a saída do comando em curso para recortar a faixa ao vivo na altura
    // dela. `cursorY` sozinho mente quando o programa reposiciona o cursor
    // ACIMA da última linha que escreveu — barra de progresso que volta pro
    // início da linha com `\r`, por exemplo. `baseY > 0` significa que a
    // saída passou da tela e não há o que recortar.
    //
    // Só reporta quando o número muda: `onRender` dispara a cada repintura, e
    // um evento por repintura atravessaria o React inteiro a cada linha de
    // saída — o core já disputa CPU com os agentes.
    let lastUsed = -1;
    let lastTotal = -1;
    let lastScrolled: boolean | null = null;
    // Índice (0-based) da última linha com texto, de baixo pra cima. Só
    // roda quando não está `scrolled` (aí a conta satura em 1 de qualquer
    // jeito) e no tick batched do `onRender` — nunca por byte.
    const lastNonEmptyRow = (): number => {
      const buffer = term.buffer.active;
      for (let y = term.rows - 1; y >= 0; y--) {
        const line = buffer.getLine(y);
        if (line && line.translateToString(true).length > 0) return y;
      }
      return -1;
    };
    const measureLive = () => {
      const report = onLiveRowsRef.current;
      if (!report) return;
      const buffer = term.buffer.active;
      const scrolled = buffer.baseY > 0;
      const used = scrolled
        ? buffer.cursorY + 1
        : usedRowsFromLastLine(buffer.cursorY, lastNonEmptyRow());
      if (used === lastUsed && term.rows === lastTotal && scrolled === lastScrolled) {
        return;
      }
      lastUsed = used;
      lastTotal = term.rows;
      lastScrolled = scrolled;
      report(used, term.rows, scrolled);
    };
    // A altura real de uma linha, medida do que o xterm desenhou.
    //
    // Não dá para calcular: o CSS multiplica `lineHeight` pelo `font-size`, o
    // xterm multiplica pela altura MEDIDA do glifo — numa Nerd Font, ~1,33x o
    // tamanho nominal. Mesmo 1.35 nos dois lados, alturas diferentes. O corpo
    // do bloco é a mesma saída que estava aqui e precisa da altura de verdade.
    let lastLineH = 0;
    let lastCellW = 0;
    const measureLineHeight = () => {
      // `.xterm-screen`, não `.xterm-rows`: com o renderer WebGL as linhas são
      // desenhadas num canvas e a camada de divs fica sem altura. A tela tem
      // sempre `rows * altura da célula`, com ou sem WebGL.
      const screen = term.element?.querySelector(".xterm-screen");
      if (!screen || term.rows <= 0) return;
      const box = screen.getBoundingClientRect();
      const h = box.height / term.rows;
      if (h > 0 && Math.abs(h - lastLineH) >= 0.05) {
        lastLineH = h;
        onLineHeightRef.current?.(h);
      }
      // A largura da célula sai da MESMA caixa: a estimativa de altura de um
      // bloco depende de quantos caracteres cabem na linha, e o corpo do cartão
      // usa esta fonte.
      if (term.cols <= 0) return;
      const w = box.width / term.cols;
      if (w <= 0 || Math.abs(w - lastCellW) < 0.05) return;
      lastCellW = w;
      onCellWidthRef.current?.(w);
    };
    measureLineHeightRef.current = measureLineHeight;
    const lineHeightMeter = term.onRender(measureLineHeight);
    addUnlistener(() => lineHeightMeter.dispose());

    const liveMeter = term.onRender(measureLive);
    addUnlistener(() => liveMeter.dispose());

    let lastCols = -1;
    let lastRows = -1;
    let timer: number | null = null;
    const refit = () => {
      timer = null;
      if (!opened) return; // o rAF de open ainda não rodou — nada a ajustar
      if (el.offsetWidth === 0 || el.offsetHeight === 0) return;
      const buffer = term.buffer.active;
      const wasAtBottom = buffer.viewportY === buffer.baseY;
      try {
        fit.fit();
      } catch {
        return; // dimensões ainda não prontas — o ResizeObserver refaz
      }
      if (term.cols !== lastCols || term.rows !== lastRows) {
        const rowsChanged = term.rows !== lastRows;
        lastCols = term.cols;
        lastRows = term.rows;
        if (wasAtBottom && rowsChanged) term.scrollToBottom();
        void resizeSession(sessionId, term.cols, term.rows).catch(() => {});
      }
    };
    // Único ponto de saída deste efeito para fora dele: o efeito de
    // `[visible, rect]`, abaixo, chama por aqui — nunca `fit.fit()` cru. Ver
    // D4 na tech-spec: todo fit avisa o PTY.
    refitRef.current = refit;
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
    // Clique para selecionar continua funcionando; clique "para posicionar o
    // cursor" devolve o foco a quem de fato edita a linha.
    const onMouseUp = () => {
      if (!readOnlyRef.current) return;
      if (term.hasSelection()) return;
      onReclaimFocusRef.current?.();
    };
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
    el.addEventListener("mouseup", onMouseUp);
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
    el.removeEventListener("mouseup", onMouseUp);
      el.removeEventListener("paste", onNativePaste, true);
      offTheme();
      linkProvider.dispose();
      bufferSub.dispose();
      dataSub.dispose();
      unlisteners.forEach((un) => un());
      unregisterTerm(sessionId);
      showExitBannerRef.current = null;
      syncWebglRef.current = null;
      refitRef.current = null;
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
    term.options.disableStdin = Boolean(readOnly);
  }, [readOnly]);

  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    if (focused && visible && !readOnly) term.focus();
    syncWebglRef.current?.();
  }, [focused, visible, readOnly]);

  // Ao ficar visível (troca de aba), o container sai de `display:none` e ganha
  // tamanho, mas o canvas do xterm ainda está nas dimensões antigas — a CSS o
  // estica até o ResizeObserver refazer o fit (com debounce de 80ms). Refaz o fit
  // e re-renderiza JÁ (no próximo frame) pra não aparecer o frame esticado.
  //
  // `rect` é objeto literal novo A CADA render do App, que re-renderiza a
  // ~60Hz durante um comando — sem o guard de igualdade por VALOR abaixo,
  // este efeito rodaria (fit + getComputedStyle síncrono + refresh) a cada
  // frame, não só quando o painel muda de geometria de verdade. Ver D5 na
  // tech-spec.
  useEffect(() => {
    if (!visible) {
      lastFitVisibleRef.current = false;
      return;
    }
    const term = termRef.current;
    const el = containerRef.current;
    if (!term || !el) return;
    const unchanged =
      lastFitVisibleRef.current === visible &&
      sameRect(lastFitRectRef.current, rect);
    if (unchanged) return;
    const raf = requestAnimationFrame(() => {
      if (el.offsetWidth === 0 || el.offsetHeight === 0) return;
      // Por `refit()`, não `fit.fit()` cru: fecha o mesmo caminho que avisa
      // o PTY. Ver D4 na tech-spec.
      refitRef.current?.();
      term.refresh(0, term.rows - 1);
      // Só marca "visto" DEPOIS do fit ter rodado de verdade — nunca antes
      // de agendar o rAF. O core flusha PTY a cada 8-16ms (princípio 3): dois
      // renders no mesmo frame cancelam o rAF do primeiro no cleanup do
      // segundo, e se a marca já tivesse sido gravada na hora de agendar, o
      // segundo render veria "unchanged" (mesmo valor já visto) e desistiria
      // sem reagendar — o fit nunca rodaria, e sem mudança de tamanho CSS
      // (só `top`) o ResizeObserver nem dispara pra corrigir depois.
      lastFitVisibleRef.current = visible;
      lastFitRectRef.current = rect;
    });
    return () => cancelAnimationFrame(raf);
  }, [visible, rect]);

  // A parte de baixo do terminal que fica escondida, em fração da altura dele.
  // A MESMA fração serve para descer o terminal e para cortá-lo: por isso a
  // parte visível cai exatamente onde a lista de blocos termina.
  const hidden = liveUsed === undefined ? 0 : hiddenFraction(liveUsed);
  // Padding vertical desta caixa, que as linhas NÃO ocupam. Sai de `calc`, com
  // `100%` sendo a altura do próprio elemento — assim a conta segue certa se o
  // painel mudar de tamanho, sem virar número mágico aqui. Ver `padSlackPx`.
  const padY = `${LIVE_PAD_Y_PX}px`;
  const liveClip: React.CSSProperties =
    hidden <= 0 || !rect
      ? {}
      : {
          // Desce por `top`, NÃO por `transform`.
          //
          // Os dois deixam o tamanho intacto — que é o que o `ResizeObserver`
          // observa e o que viraria `resizeSession` —, mas `transform` promove o
          // elemento a camada composta e o canvas WebGL do xterm passa a ser
          // rasterizado numa textura que ignora o pixel ratio da tela: o texto
          // ao vivo sai granulado ao lado dos cartões, mesma fonte e tudo.
          //
          // Para BAIXO porque a saída é escrita a partir do topo do terminal:
          // quem precisa encostar no fim do painel é a borda de cima.
          top: `calc(${rect.top}% + (${rect.height}% - ${padY}) * ${hidden})`,
          // E o corte é embaixo, onde estão as linhas que o comando não usou.
          clipPath: `inset(0 0 calc((100% - ${padY}) * ${hidden}) 0)`,
        };

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
    {resumeNotice && rect && visible && exited && (
      <div
        className="z-10 flex items-center gap-2 border-b border-tyba-cyan/30 bg-tyba-sunken px-2 py-1"
        style={{
          position: "absolute",
          left: `${rect.left}%`,
          top: `${rect.top}%`,
          width: `${rect.width}%`,
        }}
      >
        <ArrowCounterClockwise
          size={12}
          weight="bold"
          className="shrink-0 text-tyba-cyan"
        />
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-tyba-text-muted">
          {i18n.t("agentResumeNotice", { binary: resumeNotice.binary })}
        </span>
        {onResumeAgent && (
          <button
            type="button"
            onClick={onResumeAgent}
            className="shrink-0 rounded-[3px] border border-tyba-cyan/40 px-2 py-0.5 font-mono text-[10px] text-tyba-cyan hover:bg-tyba-cyan/10"
          >
            {i18n.t("agentResume")}
          </button>
        )}
        {onDismissResume && (
          <button
            type="button"
            onClick={onDismissResume}
            className="shrink-0 rounded-[3px] px-1.5 py-0.5 font-mono text-[10px] text-tyba-text-faint hover:text-tyba-text"
          >
            {i18n.t("agentResumeIgnore")}
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
          // Sem contorno próprio: quem emoldura é o PAINEL (ver `App`). A borda
          // aqui só cercava a caixa do terminal, que em modo prompt é meia
          // altura — o painel focado aparecia com moldura na metade de baixo.
          className="overflow-hidden rounded-[4px] bg-tyba-sunken px-2"
          style={
            visible && rect
              ? {
                  paddingTop: LIVE_PAD_TOP_PX,
                  paddingBottom: LIVE_PAD_BOTTOM_PX,
                  position: "absolute",
                  left: `calc(${rect.left}% + ${LIVE_INSET_X_PX}px)`,
                  top: `${rect.top}%`,
                  width: `calc(${rect.width}% - ${LIVE_INSET_X_PX * 2}px)`,
                  height: `${rect.height}%`,
                  // Recorte da faixa ao vivo. `height` acima é o tamanho que o
                  // `fit` mede e que vira `resizeSession` — ele NÃO entra nesta
                  // conta. O que se move aqui é só a posição: `top` empurra o
                  // terminal para que o topo dele encoste no fim da lista de
                  // blocos, e `clipPath` corta embaixo o que sobrou. Nenhum dos
                  // dois muda o TAMANHO, que é o que o `ResizeObserver` observa
                  // — por isso o PTY não sabe que algo mudou.
                  ...liveClip,
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
