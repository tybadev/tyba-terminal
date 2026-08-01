import { useEffect, useReducer, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";

import type { Block, BlockColor, LogicalLine, StyleRun } from "../lib/ipc";
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

/// Ctrl+C (130) e SIGPIPE (141) não são falha: são o usuário interrompendo e um
/// pipe fechando. Pintar isso de vermelho treinaria o olho a ignorar vermelho.
function failed(exitCode: number | null): boolean {
  return exitCode !== null && exitCode !== 0 && exitCode !== 130 && exitCode !== 141;
}

function duration(block: Block): string | null {
  const ms = block.finishedAtMs - block.startedAtMs;
  if (ms < 1000) return null;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.round(ms / 60_000)}min`;
}

function BlockHeader({
  block,
  pinned,
}: {
  block: Block;
  pinned?: boolean;
}) {
  const broke = failed(block.exitCode);
  const took = duration(block);
  return (
    <div
      className={`flex items-center gap-2 px-2.5 py-1 ${
        pinned
          ? "rounded-[4px] border border-tyba-border bg-tyba-surface shadow-md"
          : "border-b border-tyba-border/60"
      }`}
    >
      <span className={`shrink-0 ${broke ? "text-tyba-red" : "text-tyba-green"}`}>
        ❯
      </span>
      <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-tyba-text">
        {block.command}
      </span>
      {took && (
        <span className="shrink-0 font-mono text-[10px] text-tyba-text-faint">
          {took}
        </span>
      )}
      {broke && (
        <span className="shrink-0 font-mono text-[10px] text-tyba-red">
          {block.exitCode}
        </span>
      )}
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

function BlockCard({ block }: { block: Block }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const broke = failed(block.exitCode);
  const hidden = expanded ? 0 : Math.max(block.lines.length - BODY_LIMIT, 0);
  return (
    <div
      className={`mb-2 overflow-hidden rounded-[5px] border ${
        broke
          ? "border-tyba-red/50 bg-tyba-red/[.07]"
          : "border-tyba-border"
      }`}
    >
      <BlockHeader block={block} />
      {block.lines.length > 0 && (
        <div className="px-2.5 py-1 font-mono text-[13px] leading-[1.35] text-tyba-text-muted">
          {(expanded ? block.lines : block.lines.slice(0, BODY_LIMIT)).map(
            (line, i) => (
              <Line key={i} line={line} />
            ),
          )}
        </div>
      )}
      {hidden > 0 && (
        <button
          onClick={() => setExpanded(true)}
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
const LINE_PX = 18;
const HEADER_PX = 27;
const BLOCK_GAP_PX = 16;

interface Props {
  blocks: Block[];
  rect: PaneRectStyle;
  framed: boolean;
}

export function BlockList({ blocks, rect, framed }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  // Cor indexada é resolvida na renderização: trocar de tema tem de repintar a
  // saída antiga junto, senão o bloco congela a paleta de quando foi capturado.
  const [, repaint] = useReducer((n: number) => n + 1, 0);
  useEffect(() => onTerminalThemeChange(() => repaint()), []);
  // A estimativa sai do número de linhas, não de um número fixo: com 80px
  // chutados, o primeiro layout põe o bloco no lugar errado e ele PULA quando a
  // medição real chega — que é o "aparece com delay e renderiza".
  const estimate = (index: number) => {
    const block = blocks[index];
    if (!block) return LINE_PX + HEADER_PX;
    const body = Math.min(block.lines.length, BODY_LIMIT) * LINE_PX;
    const footer = block.truncated > 0 ? LINE_PX : 0;
    return HEADER_PX + body + footer + BLOCK_GAP_PX;
  };

  const virtualizer = useVirtualizer({
    count: blocks.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: estimate,
    overscan: 6,
  });

  const last = blocks.length - 1;
  useEffect(() => {
    if (last >= 0) virtualizer.scrollToIndex(last, { align: "end" });
  }, [last, virtualizer]);

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
    <div
      ref={scrollRef}
      className={`overflow-y-auto rounded-[4px] bg-tyba-sunken px-2 pb-3 pt-2 ${
        framed ? "border border-tyba-border" : ""
      }`}
      style={{
        position: "absolute",
        left: `${rect.left}%`,
        top: `${rect.top}%`,
        width: `${rect.width}%`,
        height: `${rect.height}%`,
      }}
    >
      {pinned !== null && blocks[pinned] && (
        <div
          className="pointer-events-none sticky top-0 z-10 -mt-2"
          style={{ marginBottom: -(HEADER_PX + 8) }}
        >
          <BlockHeader block={blocks[pinned]} pinned />
        </div>
      )}
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
            <BlockCard block={blocks[item.index]} />
          </div>
        ))}
      </div>
    </div>
  );
}
