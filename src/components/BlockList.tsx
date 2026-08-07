import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  ArrowUUpLeft,
  Check,
  ClipboardText,
  Copy,
  MarkdownLogo,
} from "@phosphor-icons/react";

import {
  blockMarkdown,
  blockOutput,
  duration,
  failed,
  shortPath,
} from "../lib/blockText";
import { writeClipboardText } from "../lib/clipboard";
import type { Block, BlockColor, LogicalLine, StyleRun } from "../lib/ipc";
import { toastError } from "../lib/toast";
import { ansiColor, onTerminalThemeChange } from "../theme";
import type { PaneRectStyle } from "./TerminalView";

/**
 * Cor indexada é resolvida na RENDERIZAÇÃO, não na captura: o bloco guarda o
 * índice para acompanhar troca de tema. `Default` devolve `undefined` e herda a
 * cor do container.
 */
function cssColor(color: BlockColor): string | undefined {
  if (color.kind === "rgb") {
    const [r, g, b] = color.value;
    return `rgb(${r} ${g} ${b})`;
  }
  if (color.kind === "idx") return ansiColor(color.value);
  return undefined;
}

function runStyle(run: StyleRun): React.CSSProperties {
  return {
    color: cssColor(run.fg),
    backgroundColor: cssColor(run.bg),
    fontWeight: run.bold ? 600 : undefined,
    fontStyle: run.italic ? "italic" : undefined,
    textDecoration: run.underline ? "underline" : undefined,
  };
}

/**
 * Uma linha lógica vira spans. Nunca HTML: os runs são dados, e montar string
 * de markup a partir de saída de comando é injeção esperando acontecer.
 */
function Line({ line }: { line: LogicalLine }) {
  if (line.runs.length === 0) {
    return <div className="whitespace-pre-wrap break-words">{line.text}</div>;
  }
  const parts: React.ReactNode[] = [];
  let cursor = 0;
  line.runs.forEach((run, i) => {
    if (run.start > cursor) {
      parts.push(<span key={`p${i}`}>{line.text.slice(cursor, run.start)}</span>);
    }
    parts.push(
      <span key={`r${i}`} style={runStyle(run)}>
        {line.text.slice(run.start, run.end)}
      </span>,
    );
    cursor = run.end;
  });
  if (cursor < line.text.length) {
    parts.push(<span key="tail">{line.text.slice(cursor)}</span>);
  }
  return <div className="whitespace-pre-wrap break-words">{parts}</div>;
}

/** Quanto tempo o ✓ fica no lugar do ícone depois de copiar. */
const COPIED_MS = 1200;

type ActionId = "command" | "output" | "markdown";

/**
 * Copiar comando, copiar saída, copiar como markdown, devolver para a linha.
 *
 * Tudo sai do modelo do bloco — ver `lib/blockText`. Ler do DOM entregaria o
 * corpo cortado em `BODY_LIMIT` linhas, e a saída sem os espaços que o
 * `white-space` desenha mas o `textContent` não tem.
 */
function BlockActions({
  block,
  onInject,
  always,
}: {
  block: Block;
  onInject?: (text: string) => void;
  /** Sem hover para revelar: é o caso do header preso, que não recebe hover. */
  always?: boolean;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState<ActionId | null>(null);
  const timer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );

  const copy = useCallback(
    (id: ActionId, text: string) => {
      void writeClipboardText(text)
        .then(() => {
          setCopied(id);
          if (timer.current !== null) window.clearTimeout(timer.current);
          timer.current = window.setTimeout(() => setCopied(null), COPIED_MS);
        })
        .catch((error) => toastError(t("blockCopyFailed"), error));
    },
    [t],
  );

  const hasOutput = block.lines.length > 0;
  const items: Array<{
    id: ActionId | "inject";
    label: string;
    icon: typeof Copy;
    run: () => void;
    show: boolean;
  }> = [
    {
      id: "command",
      label: t("blockCopyCommand"),
      icon: Copy,
      run: () => copy("command", block.command),
      show: block.command.length > 0,
    },
    {
      id: "output",
      label: t("blockCopyOutput"),
      icon: ClipboardText,
      run: () => copy("output", blockOutput(block)),
      show: hasOutput,
    },
    {
      id: "markdown",
      label: t("blockCopyMarkdown"),
      icon: MarkdownLogo,
      run: () => copy("markdown", blockMarkdown(block)),
      show: block.command.length > 0 || hasOutput,
    },
    {
      id: "inject",
      label: t("blockReuse"),
      icon: ArrowUUpLeft,
      // Reinsere na linha e para por aí: quem aperta Enter é o dono.
      run: () => onInject?.(block.command),
      show: Boolean(onInject) && block.command.length > 0,
    },
  ];

  return (
    <div
      className={`pointer-events-auto flex shrink-0 items-center gap-0.5 transition-opacity ${
        always
          ? "opacity-100"
          : "opacity-0 focus-within:opacity-100 group-hover:opacity-100"
      }`}
    >
      {items
        .filter((item) => item.show)
        .map((item) => {
          const Icon = copied === item.id ? Check : item.icon;
          return (
            <button
              key={item.id}
              type="button"
              title={item.label}
              aria-label={item.label}
              // Clicar numa ação não é clicar no bloco: sem isto, copiar a
              // saída marcaria o cartão de quebra.
              onClick={(event) => {
                event.stopPropagation();
                item.run();
              }}
              // Caixa de tamanho fixo, e menor que a linha do comando: botão
              // que cresce o header desalinha a estimativa de altura do
              // virtualizador e o bloco pula quando a medição real chega.
              className="flex size-[18px] items-center justify-center rounded-[3px] text-tyba-text-faint transition-colors hover:bg-tyba-text/[.06] hover:text-tyba-text focus-visible:bg-tyba-text/[.06] focus-visible:text-tyba-text"
            >
              <Icon size={13} weight={copied === item.id ? "bold" : "regular"} />
            </button>
          );
        })}
    </div>
  );
}

function BlockHeader({
  block,
  pinned,
  onInject,
}: {
  block: Block;
  pinned?: boolean;
  onInject?: (text: string) => void;
}) {
  const broke = failed(block.exitCode);
  const took = duration(block);
  const where = shortPath(block.cwd);
  return (
    <div
      // O preso é `pointer-events-none`: ele fica POR CIMA da lista, e uma
      // faixa opaca de 27px que captura a roda do mouse é uma faixa onde a
      // lista não rola. Só as ações reativam — senão as do bloco do topo, que
      // é o header que ele cobre, ficariam inalcançáveis.
      className={`group flex items-center gap-2 px-2.5 py-1 ${
        pinned
          ? "pointer-events-none rounded-[4px] border border-tyba-border bg-tyba-surface shadow-md"
          : "border-b border-tyba-border/60"
      }`}
    >
      <span className={`shrink-0 ${broke ? "text-tyba-red" : "text-tyba-green"}`}>
        ❯
      </span>
      <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-tyba-text">
        {block.command}
      </span>
      {/* Onde rodou. Encurtado porque o começo do caminho empurraria o comando
          para fora da linha; o inteiro fica no title. */}
      {where && (
        <span
          title={block.cwd ?? undefined}
          className="max-w-[38%] shrink-0 truncate font-mono text-[10px] text-tyba-text-faint"
        >
          {where}
        </span>
      )}
      {broke && (
        <span className="shrink-0 font-mono text-[10px] text-tyba-red">
          {block.exitCode}
        </span>
      )}
      {took && (
        <span className="shrink-0 font-mono text-[10px] tabular-nums text-tyba-text-faint">
          {took}
        </span>
      )}
      <BlockActions block={block} onInject={onInject} always={pinned} />
    </div>
  );
}

/**
 * Teto de linhas desenhadas de uma vez.
 *
 * O virtualizador virtualiza BLOCOS, não as linhas dentro de um: um bloco no
 * teto de 10 mil linhas viraria 10 mil nós de DOM num item só e travaria o
 * painel. Ninguém lê 10 mil linhas rolando — quem precisa, expande.
 */
const BODY_LIMIT = 200;

/**
 * Barra à esquerda do cartão marcado.
 *
 * `box-shadow` e não borda: borda muda a altura medida, e altura que muda faz o
 * virtualizador reposicionar a lista embaixo do ponteiro no meio de um
 * shift-clique.
 */
/**
 * A barra na lateral do bloco marcado.
 *
 * `box-shadow` e não borda: borda muda a altura medida, e altura que muda faz o
 * virtualizador reposicionar a lista embaixo do ponteiro no meio de um
 * shift-clique.
 *
 * 2px, e não mais: com a borda do marcado já na cor de destaque, engrossar a
 * barra faz a lateral esquerda destoar do resto do contorno — 6px contra 2px
 * do topo lê como caixa torta. Mais destaque se pede à cor, não à espessura.
 */
const MARKED_BAR = "inset 2px 0 0 0 var(--tyba-primary)";

function BlockCard({
  block,
  fontSizePx,
  lineHeightPx,
  onInject,
  marked,
  onPick,
}: {
  block: Block;
  /** Tamanho da fonte do terminal — ver o corpo abaixo. */
  fontSizePx: number;
  /** Altura real de uma linha do terminal, medida. */
  lineHeightPx: number;
  onInject?: (text: string) => void;
  marked: boolean;
  onPick?: (event: React.MouseEvent) => void;
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const broke = failed(block.exitCode);
  const hidden = expanded ? 0 : Math.max(block.lines.length - BODY_LIMIT, 0);
  return (
    // `group` aqui além do header: revela as ações com o ponteiro em qualquer
    // lugar do cartão, não só na faixa de 27px de cima.
    <div
      onClick={onPick}
      // Shift-clique é o gesto do navegador para esticar seleção de texto.
      // Barrar aqui é o que deixa o shift ser do bloco, sem sujar a tela com
      // um trecho de texto realçado que ninguém pediu.
      onMouseDown={(event) => {
        if (event.shiftKey && onPick) event.preventDefault();
      }}
      style={marked ? { boxShadow: MARKED_BAR } : undefined}
      className={`group mb-2 overflow-hidden rounded-[5px] border ${
        marked
          ? "border-tyba-primary bg-tyba-green-tint"
          : broke
            ? "border-tyba-red/50 bg-tyba-red/[.07]"
            : "border-tyba-block-border"
      }`}
    >
      <BlockHeader block={block} onInject={onInject} />
      {/* Sem corpo, mas com motivo: um cartão vazio e mudo pareceria comando
          que não imprimiu nada. */}
      {block.altScreen && (
        <div className="px-2.5 py-1 font-mono text-[11px] italic text-tyba-text-faint">
          {t("blockAltScreen")}
        </div>
      )}
      {block.lines.length > 0 && (
        // Fonte e entrelinha do TERMINAL, não um tamanho próprio: este corpo é
        // a mesma saída que estava no terminal um instante antes, e o dono pode
        // ter mudado o tamanho da fonte. Preso em 13px, o texto encolhia ao
        // virar cartão.
        <div
          className="px-2.5 py-1 font-mono text-tyba-text-muted"
          style={{
            fontSize: `${fontSizePx}px`,
            lineHeight: `${lineHeightPx}px`,
          }}
        >
          {(expanded ? block.lines : block.lines.slice(0, BODY_LIMIT)).map(
            (line, i) => (
              <Line key={i} line={line} />
            ),
          )}
        </div>
      )}
      {hidden > 0 && (
        <button
          onClick={(event) => {
            event.stopPropagation();
            setExpanded(true);
          }}
          className="w-full border-t border-tyba-border/60 px-2.5 py-1 text-left font-mono text-[10px] text-tyba-text-faint hover:text-tyba-text"
        >
          {t("blockShowAll", { count: hidden })}
        </button>
      )}
      {block.truncated > 0 && (
        <div className="border-t border-tyba-border/60 px-2.5 py-1 font-mono text-[10px] text-tyba-amber">
          {t("blockTruncated", { count: block.truncated })}
        </div>
      )}
    </div>
  );
}

/** Métricas do cartão, para a estimativa nascer perto do valor medido. */
/// 13px × 1.35, a mesma métrica do xterm.

const HEADER_PX = 27;
/**
 * Respiro entre um bloco e o seguinte.
 *
 * Exportado porque o bloco EM EXECUÇÃO também precisa dele: sem isso ele é o
 * único da lista encostado no vizinho, e a diferença lê como defeito bem no
 * cartão para onde o olho vai.
 */
export const BLOCK_GAP_PX = 16;
/**
 * Folga para considerar a lista "no fim".
 *
 * Zero exigiria pixel exato: arredondamento de `scrollHeight` fracionário e uma
 * rolagem que parou a um fio do fim deixariam a lista de reancorar justamente
 * quando o dono achava que estava acompanhando a saída.
 */
const STICK_SLACK_PX = 4;

interface Props {
  blocks: Block[];
  rect: PaneRectStyle;
  framed: boolean;
  /**
   * Devolve o comando para a linha, sem executar. É o mesmo caminho do
   * histórico e do snippet — com a linha do TYBA no ar o terminal está
   * somente-leitura, e `term.paste` seria engolido em silêncio.
   *
   * Ausente quando o painel não é o ativo: a injeção tem um destino só, e
   * mandar o comando para a sessão errada é pior do que não ter o botão.
   */
  onInject?: (text: string) => void;
  /** Mexer nos blocos de um painel torna aquele painel o ativo. */
  onActivate?: () => void;
  /** Ids marcados para copiar de uma vez. */
  marked?: ReadonlySet<number>;
  /** Clique num cartão. O modificador vem no evento; a regra é do chamador. */
  onPick?: (id: number, event: React.MouseEvent) => void;
  /** Clique no fundo da lista — sai da seleção sem procurar onde clicar. */
  onClearPick?: () => void;
  /** Atalho de copiar, já formatado. Vem de fora porque é rebindável. */
  copyCombo?: string;
  /**
   * Altura, em px, do header do bloco em execução — que fica POR CIMA do fim
   * desta lista, não ao lado dela.
   *
   * Ele é desenhado no topo da faixa ao vivo com `translateY(-100%)`, ou seja,
   * ocupa os últimos pixels do retângulo da lista. Sem reservar esse espaço, o
   * último bloco passa por baixo do header e aparece com a linha final cortada
   * — a rolagem chega ao fim, o corte não é dela.
   *
   * Vem MEDIDO, nunca fixo: é a mesma conta que o header preso já errou uma
   * vez, número fixo contra altura real, e a diferença reaparece exatamente
   * como bloco cortado.
   */
  bottomInset?: number;
  /**
   * Tamanho da fonte do terminal, em px.
   *
   * O corpo do bloco usa a MESMA métrica do terminal: é a mesma saída, e o
   * dono pode ter mudado o tamanho. Ver `getDefaultFontSize`.
   */
  fontSizePx: number;
  /**
   * Altura real de uma linha do terminal, em px.
   *
   * Medida, nunca calculada: o xterm aplica a entrelinha sobre a altura do
   * glifo e o CSS sobre o `font-size`, então o mesmo 1.35 dá 21,5px de um lado
   * e 16,2px do outro. O corpo do bloco é a mesma saída — tem de ter a mesma
   * altura, ou o texto "pula" ao virar cartão.
   */
  lineHeightPx: number;
}

export function BlockList({
  blocks,
  rect,
  framed,
  bottomInset = 0,
  fontSizePx,
  lineHeightPx,
  onInject,
  onActivate,
  marked,
  onPick,
  onClearPick,
  copyCombo,
}: Props) {
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);
  // Cor indexada é resolvida na renderização: trocar de tema tem de repintar a
  // saída antiga junto, senão o bloco congela a paleta de quando foi capturado.
  const [, repaint] = useReducer((n: number) => n + 1, 0);
  useEffect(() => onTerminalThemeChange(() => repaint()), []);
  // A estimativa sai do número de linhas, não de um número fixo: com 80px
  // chutados, o primeiro layout põe o bloco no lugar errado e ele PULA quando a
  // medição real chega — que é o "aparece com delay e renderiza".
  const estimate = (index: number) => {
    const line = lineHeightPx;
    const block = blocks[index];
    if (!block) return line + HEADER_PX;
    const body = Math.min(block.lines.length, BODY_LIMIT) * line;
    const footer = block.truncated > 0 ? line : 0;
    return HEADER_PX + body + footer + BLOCK_GAP_PX;
  };

  const virtualizer = useVirtualizer({
    count: blocks.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: estimate,
    overscan: 6,
  });

  // Métrica nova = TODO cartão muda de altura, e o virtualizador precisa saber.
  //
  // Ele guarda a altura MEDIDA por índice, e `estimateSize` só vale para quem
  // ele ainda não mediu. Sem este reset, um cartão já medido conserva a altura
  // antiga enquanto o corpo passa a desenhar na nova — o bloco seguinte é
  // posicionado por cima do anterior e o texto de um vaza sobre o outro.
  //
  // Não era possível antes: o corpo do cartão tinha tamanho fixo, então a
  // métrica nunca mudava depois do mount e o cache nunca envelhecia. Ela virou
  // dinâmica para o cartão acompanhar a fonte do terminal — e a mudança que
  // consertou o texto pulando ao virar bloco abriu esta.
  //
  // O caminho normal passa por aqui sempre, não só quando o dono mexe na fonte:
  // a altura de linha nasce estimada da fonte e vira o valor medido do
  // `.xterm-screen` assim que o terminal desenha.
  useEffect(() => {
    virtualizer.measure();
  }, [fontSizePx, lineHeightPx, virtualizer]);

  const last = blocks.length - 1;
  useEffect(() => {
    if (last >= 0) virtualizer.scrollToIndex(last, { align: "end" });
  }, [last, virtualizer]);

  // Estava colado no fim quando o dono mexeu pela última vez?
  //
  // Medir isto na hora em que a altura muda não serve: o efeito roda depois do
  // render, quando a lista JÁ encolheu e o fim já escapou. O que vale é o
  // estado da última rolagem de verdade.
  const atBottomRef = useRef(true);

  // A altura da lista muda a cada linha de saída que chega — ela cede à faixa
  // ao vivo só o que a saída usa. Ancorada embaixo e sem reancorar a rolagem, o
  // último bloco aparece cortado no meio a cada linha nova.
  //
  // Só reancora quem já estava no fim: puxar de volta para baixo quem subiu
  // para ler um bloco antigo é pior do que o corte que isto conserta.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || last < 0 || !atBottomRef.current) return;
    // `scrollTop` na mão, e não `scrollToIndex`: o virtualizador posiciona os
    // itens a partir do topo, mas a lista é ancorada embaixo (`justify-end` com
    // `min-h-full`), então as posições dele não batem com a rolagem — é o mesmo
    // descolamento que o header preso já contorna logo abaixo.
    //
    // Em rAF porque o efeito roda antes de o WebKit refazer o layout com a
    // altura nova: lido agora, `scrollHeight` ainda é o de antes da mudança.
    const id = requestAnimationFrame(() => {
      el.scrollTop = el.scrollHeight;
    });
    return () => cancelAnimationFrame(id);
  }, [rect.height, bottomInset, last]);

  // Bloco mais alto que o painel some com o próprio header ao rolar, e o que
  // sobra na tela é uma parede de texto igual à de um terminal comum — some
  // justamente o "cada comando tem o seu recorte". O header do bloco que está
  // no topo da viewport fica preso ali.
  //
  // Preso por fora da lista, e não com `position: sticky`: o virtualizador
  // posiciona cada item com `transform`, e sticky dentro de um ancestral
  // transformado gruda no item, não na viewport.
  const [pinned, setPinned] = useState<number | null>(null);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const sync = () => {
      // Lista que cabe na tela está sempre no fim, por definição.
      atBottomRef.current =
        el.scrollHeight - el.scrollTop - el.clientHeight <= STICK_SLACK_PX;
      // Lista que cabe na tela não tem o que prender: ancorada embaixo, ela
      // começa deslocada do topo, e as posições que o virtualizador calcula
      // deixam de bater com a rolagem.
      if (el.scrollHeight <= el.clientHeight + 1) {
        setPinned(null);
        return;
      }
      const top = el.scrollTop;
      const items = virtualizer.getVirtualItems();
      const current = items.find(
        (item) => item.start <= top && item.end > top + HEADER_PX,
      );
      setPinned(current ? current.index : null);
    };
    sync();
    el.addEventListener("scroll", sync, { passive: true });
    return () => el.removeEventListener("scroll", sync);
  }, [virtualizer, blocks.length]);

  return (
    // Duas camadas: a lista que rola e, POR CIMA dela, o que fica parado.
    //
    // O header preso já morou dentro do scroll, seguro por `sticky` e uma
    // margem negativa que o tirava do fluxo. A conta nunca fecha: a margem é um
    // número fixo, a altura do header é medida, e a diferença aparece como a
    // primeira linha do bloco cortada e como scroll que não chega ao fim.
    // Sobreposição não é fluxo — então não fica no fluxo.
    <div
      className="pointer-events-none"
      style={{
        position: "absolute",
        left: `${rect.left}%`,
        top: `${rect.top}%`,
        width: `${rect.width}%`,
        // Encurtado pelo header do bloco em execução, que é desenhado sobre o
        // fim desta lista. Ver `bottomInset`.
        height: `calc(${rect.height}% - ${bottomInset}px)`,
      }}
    >
      <div
        ref={scrollRef}
        // Rolagem movida na mão, e não pela nativa.
        //
        // A nativa não chegava aqui com o ponteiro sobre os cartões, só sobre
        // a barra: esta lista é uma camada sobreposta ao terminal, e o WebKit
        // prende o gesto ao scroller que escolheu no primeiro evento — preso
        // no de trás, o de cima não recebe mais nada.
        onWheel={(event) => {
          const el = scrollRef.current;
          if (!el) return;
          const before = el.scrollTop;
          el.scrollTop = before + event.deltaY;
          // No topo e no fim o gesto passa adiante, senão a lista viraria um
          // buraco onde nada mais rola.
          if (el.scrollTop !== before) event.preventDefault();
        }}
        onMouseDown={onActivate}
        // Só o fundo: clique que veio de um cartão para aqui por bubbling já
        // decidiu o que fazer com a seleção.
        onClick={(event) => {
          if (event.target === event.currentTarget) onClearPick?.();
        }}
        // `overscroll-contain`: ao bater no topo, o WebKit passa o gesto para o
        // ancestral e TRAVA nele até a rolagem terminar — daí "rolei tudo para
        // cima e agora não desce mais, só clicando na barra". Conter o encadea-
        // mento mantém o gesto nesta lista.
        className={`pointer-events-auto h-full overflow-y-auto overscroll-contain rounded-[4px] bg-tyba-sunken px-2 pb-3 pt-2 ${
          framed ? "border border-tyba-border" : ""
        }`}
      >
        {/* Ancorado embaixo, como todo terminal: os blocos crescem de baixo
            para cima e o último fica colado na linha de comando. Ancorado em
            cima, poucos blocos deixam um vazio enorme logo acima do input —
            que é exatamente para onde o olho vai. */}
        <div className="flex min-h-full flex-col justify-end">
          <div
            style={{ height: virtualizer.getTotalSize(), position: "relative" }}
          >
          {virtualizer.getVirtualItems().map((item) => (
            <div
              key={blocks[item.index].id}
              ref={virtualizer.measureElement}
              data-index={item.index}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${item.start}px)`,
              }}
            >
              <BlockCard
                block={blocks[item.index]}
                fontSizePx={fontSizePx}
                lineHeightPx={lineHeightPx}
                onInject={onInject}
                marked={marked?.has(blocks[item.index].id) ?? false}
                onPick={
                  onPick
                    ? (event) => onPick(blocks[item.index].id, event)
                    : undefined
                }
                />
              </div>
            ))}
          </div>
        </div>
      </div>

      {pinned !== null && blocks[pinned] && (
        <div className="absolute inset-x-2 top-2 z-10">
          <BlockHeader block={blocks[pinned]} pinned onInject={onInject} />
        </div>
      )}

      {/* Marcar bloco é um modo, e modo que não se anuncia ninguém acha. */}
      {marked && marked.size > 0 && (
        <div className="absolute bottom-4 right-3 z-10">
          <span className="rounded-full border border-tyba-border bg-tyba-surface px-2 py-0.5 font-mono text-[10px] text-tyba-text-muted shadow-md">
            {t("blockPicked", { count: marked.size, combo: copyCombo ?? "" })}
          </span>
        </div>
      )}
    </div>
  );
}
