import { useEffect, useReducer, useRef } from "react";
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

function BlockCard({ block }: { block: Block }) {
  const { t } = useTranslation();
  const broke = failed(block.exitCode);
  const took = duration(block);
  return (
    <div
      className={`mb-2 overflow-hidden rounded-[5px] border ${
        broke ? "border-tyba-red/40 bg-tyba-red/[.04]" : "border-tyba-border"
      }`}
    >
      <div className="flex items-center gap-2 border-b border-tyba-border/60 px-2.5 py-1">
        <span
          className={`shrink-0 ${broke ? "text-tyba-red" : "text-tyba-green"}`}
        >
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
      {block.lines.length > 0 && (
        <div className="px-2.5 py-1 font-mono text-[13px] leading-[1.35] text-tyba-text-muted">
          {block.lines.map((line, i) => (
            <Line key={i} line={line} />
          ))}
        </div>
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
    const body = Math.max(block.lines.length, 0) * LINE_PX;
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
