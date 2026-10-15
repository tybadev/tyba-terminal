import { useEffect, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import {
  elapsedSeconds,
  isSlow,
  progressOf,
  splitEmphasis,
  type LoadOp,
} from "@/lib/loading";

/**
 * Um bloco do esqueleto. Véu do texto do tema (6%), então vale nos 18 temas,
 * claro e escuro; o brilho para em `prefers-reduced-motion` (ver `styles.css`).
 */
export function Skeleton({
  w,
  h = 10,
  r = 4,
  className,
  style,
}: {
  w: string;
  h?: number;
  r?: number;
  className?: string;
  style?: CSSProperties;
}) {
  return (
    <span
      aria-hidden
      className={cn("tyba-skeleton", className)}
      style={{ width: w, height: h, borderRadius: r, ...style }}
    />
  );
}

/** A forma final do que vai chegar. Mesmas medidas do mockup `Carregamento`. */
export type SkeletonShape = "list" | "tree" | "prs" | "diff" | "lines";

const LIST_W = ["46%", "62%", "38%", "54%"];
const TREE: [number, string][] = [
  [0, "40%"],
  [1, "30%"],
  [2, "52%"],
  [2, "44%"],
  [1, "36%"],
  [1, "48%"],
];
const PR_W = ["70%", "56%", "64%"];
const DIFF_W = ["50%", "42%", "58%"];
const LINES_W = ["92%", "76%", "84%", "58%", "70%"];

export function LoadingSkeleton({ shape, rows }: { shape: SkeletonShape; rows?: number }) {
  const n = (fallback: number) => Math.max(1, rows ?? fallback);
  switch (shape) {
    case "list":
      return (
        <>
          {Array.from({ length: n(4) }, (_, i) => (
            <div key={i} className="flex items-center gap-2.5">
              <Skeleton w="8px" h={8} />
              <Skeleton w={LIST_W[i % LIST_W.length]} />
              <span className="flex-1" />
              <Skeleton w="38px" h={8} />
            </div>
          ))}
        </>
      );
    case "tree":
      return (
        <>
          {Array.from({ length: n(TREE.length) }, (_, i) => {
            const [depth, w] = TREE[i % TREE.length];
            return (
              <div key={i} className="flex items-center gap-2" style={{ paddingLeft: depth * 14 }}>
                <Skeleton w="12px" h={12} r={3} />
                <Skeleton w={w} />
              </div>
            );
          })}
        </>
      );
    case "prs":
      return (
        <>
          {Array.from({ length: n(3) }, (_, i) => (
            <div key={i} className="flex items-center gap-2.5">
              <Skeleton w="18px" h={18} r={9} />
              <div className="flex flex-1 flex-col gap-[5px]">
                <Skeleton w={PR_W[i % PR_W.length]} />
                <Skeleton w="30%" h={8} />
              </div>
              <Skeleton w="44px" h={16} r={8} />
            </div>
          ))}
        </>
      );
    case "diff":
      return (
        <>
          {Array.from({ length: n(3) }, (_, i) => (
            <div key={i} className="flex items-center gap-2">
              <Skeleton w="10px" h={10} r={2} />
              <Skeleton w={DIFF_W[i % DIFF_W.length]} />
              <span className="flex-1" />
              <Skeleton w="28px" h={8} />
            </div>
          ))}
          <div className="mt-1.5 flex flex-col gap-1">
            <Skeleton w="100%" h={9} r={2} style={{ opacity: 0.6 }} />
            <Skeleton w="100%" h={9} r={2} style={{ opacity: 0.6 }} />
            <Skeleton w="82%" h={9} r={2} style={{ opacity: 0.6 }} />
          </div>
        </>
      );
    case "lines":
      return (
        <>
          {Array.from({ length: n(5) }, (_, i) => (
            <Skeleton key={i} w={LINES_W[i % LINES_W.length]} h={9} r={2} />
          ))}
        </>
      );
  }
}

/** Relógio que só anda enquanto `active` — para medir a demora de quem espera. */
export function useNow(active: boolean, intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(timer);
  }, [active, intervalMs]);
  return now;
}

/** O texto da etapa com os fatos (caminho, host, sha) acesos. */
export function StepText({ text, em = [] }: { text: string; em?: readonly string[] }) {
  return (
    <>
      {splitEmphasis(text, em).map((part, i) =>
        part.em ? (
          <span key={i} className="text-tyba-text">
            {part.text}
          </span>
        ) : (
          part.text
        ),
      )}
    </>
  );
}

export interface LoadingAction {
  label: string;
  onClick: () => void;
}

export interface LoadingStateProps {
  /** A etapa real, já traduzida ("consultando o docker local"). */
  step: string;
  /** Fatos citados na etapa, acesos no texto. */
  em?: readonly string[];
  /** Quando o trabalho começou (epoch ms). Muda = recomeçou. */
  startedAt: number;
  op: LoadOp;
  remote?: boolean;
  /** Motivo provável da demora, mostrado só depois do limiar. */
  slowCause?: string;
  /** A ação da demora. Sem ela, "Tentar de novo" se houver `onRetry`. */
  slowAction?: LoadingAction;
  /** Falhou: o erro (já traduzido — o que o core devolveu) e "Tentar de novo". */
  error?: string | null;
  onRetry?: () => void;
  /** Só quando o core sabe o total. */
  done?: number | null;
  total?: number | null;
  /** `null` = só o filamento e a etapa, sem esqueleto (lugares pequenos). */
  shape?: SkeletonShape | null;
  rows?: number;
  className?: string;
}

const LINK = "text-tyba-blue hover:text-tyba-text";

/**
 * Carregamento sem "Carregando…": esqueleto na forma final, o filamento
 * correndo enquanto há trabalho, a etapa real e — passado o normal — o motivo
 * e uma ação. Falhou: o erro e "Tentar de novo", filamento apagado.
 */
export function LoadingState({
  step,
  em,
  startedAt,
  op,
  remote = false,
  slowCause,
  slowAction,
  error,
  onRetry,
  done,
  total,
  shape = "list",
  rows,
  className,
}: LoadingStateProps) {
  const { t, i18n } = useTranslation();
  const working = !error;
  const now = useNow(working);
  const slow = working && isSlow(startedAt, now, op, remote);
  const progress = progressOf(done, total);
  const fmt = (n: number) => n.toLocaleString(i18n.language);
  const action = slowAction ?? (onRetry ? { label: t("retry"), onClick: onRetry } : null);
  return (
    <div aria-busy={working} className={cn("flex min-h-0 flex-col overflow-hidden", className)}>
      {/* O filamento é o sinal de vida: só existe com trabalho de verdade. */}
      <div
        aria-hidden
        className={cn(
          "tyba-flow-line tyba-flow-line--thick shrink-0",
          working ? "tyba-flow-line--live" : "opacity-0",
        )}
      />
      {shape && working && (
        <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-hidden px-3.5 py-3">
          <LoadingSkeleton shape={shape} rows={rows} />
        </div>
      )}
      <div
        role="status"
        aria-live="polite"
        className={cn(
          "flex shrink-0 flex-col gap-1 px-3.5 py-2.5 text-[11px] leading-4 text-tyba-text-muted",
          shape && working && "border-t border-[var(--tyba-border-soft)]",
        )}
      >
        {error ? (
          <span className="text-tyba-red">
            {error}
            {onRetry && (
              <>
                {" "}
                <button type="button" onClick={onRetry} className={LINK}>
                  {t("retry")}
                </button>
              </>
            )}
          </span>
        ) : (
          <>
            <span className="min-w-0">
              <span
                aria-hidden
                className="tyba-pulse mr-2 inline-block size-1.5 rounded-full bg-tyba-cyan align-[1px]"
              />
              <StepText text={step} em={em} />
              {progress && (
                <>
                  {" · "}
                  <span className="text-tyba-text">
                    {t("loadingProgress", { done: fmt(progress.done), total: fmt(progress.total) })}
                  </span>
                </>
              )}
            </span>
            {progress && (
              <span aria-hidden className="tyba-load-bar">
                <span style={{ width: `${progress.percent}%` }} />
              </span>
            )}
            {slow && (
              <span className="text-tyba-amber">
                {t("loadingSlow", { s: elapsedSeconds(startedAt, now) })}
                {slowCause && ` ${slowCause}`}
                {action && (
                  <>
                    {" "}
                    <button type="button" onClick={action.onClick} className={LINK}>
                      {action.label}
                    </button>
                  </>
                )}
              </span>
            )}
          </>
        )}
      </div>
    </div>
  );
}
