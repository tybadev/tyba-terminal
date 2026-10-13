import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { MagnifyingGlass } from "@phosphor-icons/react";

import {
  dockerLogsStart,
  exportDownload,
  exportRedacted,
  onDockerLogs,
  type LogLine,
} from "@/lib/ipc";
import {
  appendCapped,
  clockOf,
  countMatches,
  exportLine,
  highlightParts,
  levelTotals,
  matchesQuery,
  ownTimestamp,
  TIME_WINDOWS,
  visibleLogs,
  type LevelChip,
  type TimeWindow,
} from "@/lib/containerLogs";
import { containerName, openEditorTab } from "@/lib/contentTabs";
import { writeClipboardText } from "@/lib/clipboard";
import { pushToast, toastError } from "@/lib/toast";

interface Props {
  containerId: string;
  /** Muda a cada "Reiniciar": religa o `docker logs -f`. */
  restartNonce: number;
}

const ROW_PX = 24; // mockup: 20px de linha + 2px em cima e embaixo

const LEVEL_CLASS: Record<string, string> = {
  error: "text-tyba-red",
  warn: "text-tyba-amber",
  info: "text-tyba-text-faint",
  debug: "text-tyba-text-faint",
};

/** Logs do container como lista: hora, nível, filtro que acende, links de
 *  stack trace para o editor. O parse já veio do core. */
export function ContainerLogsView({ containerId, restartNonce }: Props) {
  const { t } = useTranslation();
  const [lines, setLines] = useState<LogLine[]>([]);
  const [ended, setEnded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [chip, setChip] = useState<LevelChip>("all");
  const [window_, setWindow] = useState<TimeWindow>("all");
  const [showTime, setShowTime] = useState(true);
  // Quebrar é o padrão: linha cortada no meio esconde justo o fim da
  // mensagem. Sem quebra, a área rola na horizontal — nunca corta.
  const [wrap, setWrap] = useState(true);
  const [follow, setFollow] = useState(true);
  const [now, setNow] = useState(() => Date.now());
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let alive = true;
    setError(null);
    setEnded(false);
    // O listener entra antes do start: linha que chegue entre os dois não se
    // perde (o snapshot e o lote podem repetir — o `seq` desduplica).
    const un = onDockerLogs(containerId, (more, done) => {
      if (!alive) return;
      setLines((prev) => {
        const last = prev.length > 0 ? prev[prev.length - 1].seq : -1;
        return appendCapped(prev, more.filter((l) => l.seq > last));
      });
      if (done) setEnded(true);
    });
    void dockerLogsStart(containerId)
      .then((snap) => {
        if (!alive) return;
        setLines((prev) => {
          const first = prev.length > 0 ? prev[0].seq : Number.POSITIVE_INFINITY;
          return appendCapped(
            snap.lines.filter((l) => l.seq < first),
            prev,
          );
        });
      })
      .catch((e) => {
        if (alive) setError(String(e));
      });
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, [containerId, restartNonce]);

  // A janela de tempo é relativa ao agora: reavalia de tempos em tempos.
  useEffect(() => {
    if (window_ === "all") return;
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, [window_]);

  const visible = useMemo(
    () => visibleLogs(lines, chip, window_, now),
    [lines, chip, window_, now],
  );
  const matches = useMemo(() => countMatches(visible, query), [visible, query]);
  const totals = useMemo(() => levelTotals(lines), [lines]);
  // Coluna de nível só quando alguma linha visível tem nível conhecido.
  const hasLevels = useMemo(() => visible.some((l) => l.level !== null), [visible]);

  const virtualizer = useVirtualizer({
    count: visible.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_PX,
    overscan: 20,
  });

  useEffect(() => {
    if (follow && visible.length > 0) {
      virtualizer.scrollToIndex(visible.length - 1, { align: "end" });
    }
  }, [follow, visible.length, virtualizer]);

  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < ROW_PX * 2;
    setFollow(atBottom);
  }, []);

  const exportable = useCallback(
    () => visible.map((l) => exportLine(l, showTime)),
    [visible, showTime],
  );

  const copy = useCallback(async () => {
    try {
      const text = await exportRedacted(exportable());
      await writeClipboardText(text);
      pushToast({ title: t("logsCopied", { count: visible.length }) });
    } catch (e) {
      toastError(t("logsCopyError"), e);
    }
  }, [exportable, visible.length, t]);

  const download = useCallback(async () => {
    try {
      const path = await exportDownload(`${containerName(containerId)}-logs.txt`, exportable());
      pushToast({ title: t("logsDownloaded"), detail: path });
    } catch (e) {
      toastError(t("logsDownloadError"), e);
    }
  }, [containerId, exportable, t]);

  const chipClass = (on: boolean) =>
    `shrink-0 rounded-[5px] border px-[9px] py-[3px] text-[11px] ${
      on
        ? "border-tyba-text/20 text-tyba-text"
        : "border-tyba-border-field text-tyba-text-muted hover:text-tyba-text"
    }`;

  return (
    <div className="absolute inset-0 flex min-h-0 min-w-0 flex-col bg-tyba-sunken">
      <div className="flex shrink-0 flex-wrap items-center gap-x-2.5 gap-y-2 tyba-divide-b px-[18px] py-2.5">
        <label className="flex min-w-[180px] max-w-full flex-1 basis-[240px] items-center gap-2 rounded-[6px] border border-tyba-border-field bg-tyba-surface px-2.5 py-[5px] text-[12px] text-tyba-text-faint">
          <MagnifyingGlass size={13} />
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label={t("logsFilter")}
            placeholder={t("logsFilter")}
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            className="min-w-0 flex-1 bg-transparent text-tyba-text outline-none placeholder:text-tyba-text-faint"
          />
          {query.trim() && (
            <span className="shrink-0">
              {t("logsMatchCount", { matches, total: visible.length })}
            </span>
          )}
        </label>
        <button type="button" className={chipClass(chip === "all")} onClick={() => setChip("all")}>
          {t("logsAll")}
        </button>
        <button
          type="button"
          className={chipClass(chip === "errors")}
          onClick={() => setChip("errors")}
        >
          {t("logsErrors")}
        </button>
        <button
          type="button"
          className={chipClass(chip === "warnings")}
          onClick={() => setChip("warnings")}
        >
          {t("logsWarnings")}
        </button>
        <select
          value={window_}
          onChange={(e) => {
            setWindow(e.target.value as TimeWindow);
            setNow(Date.now());
          }}
          aria-label={t("logsWindow")}
          className="shrink-0 rounded-[5px] border border-tyba-border-field bg-tyba-surface px-1.5 py-0.5 text-[11px] text-tyba-text-muted"
        >
          {TIME_WINDOWS.map((w) => (
            <option key={w} value={w}>
              {t(`logsWindow_${w}`)}
            </option>
          ))}
        </select>
        <button
          type="button"
          className={chipClass(showTime)}
          aria-pressed={showTime}
          onClick={() => setShowTime((v) => !v)}
        >
          {t("logsTime")}
        </button>
        <button
          type="button"
          className={chipClass(wrap)}
          aria-pressed={wrap}
          onClick={() => setWrap((v) => !v)}
        >
          {t("logsWrap")}
        </button>
        <button
          type="button"
          aria-pressed={follow}
          onClick={() => setFollow((v) => !v)}
          className={`flex shrink-0 items-center gap-1.5 rounded-[5px] border px-2 py-0.5 text-[11px] ${
            follow && !ended
              ? "border-tyba-green/40 text-tyba-green"
              : "border-tyba-border-strong text-tyba-text-muted hover:text-tyba-text"
          }`}
        >
          <span
            aria-hidden
            className={`size-1.5 rounded-full ${
              follow && !ended
                ? "animate-pulse bg-tyba-green shadow-[var(--tyba-glow-green)]"
                : "bg-tyba-text-faint"
            }`}
          />
          {ended ? t("logsEnded") : follow ? t("logsFollowing") : t("logsPaused")}
        </button>
        <button type="button" className={chipClass(false)} onClick={() => void copy()}>
          {t("logsCopy")}
        </button>
        <button type="button" className={chipClass(false)} onClick={() => void download()}>
          {t("logsDownload")}
        </button>
      </div>

      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="min-h-0 flex-1 overflow-auto py-2 font-mono text-[12.5px]"
      >
        {error && <div className="px-4 py-2 text-tyba-red">{error}</div>}
        {!error && visible.length === 0 && (
          <div className="px-4 py-2 text-tyba-text-faint">{t("logsEmpty")}</div>
        )}
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((item) => {
            const line = visible[item.index];
            const hit = matchesQuery(line, query);
            // Com a hora ligada, a linha que já traz a própria hora mostra a
            // dela na coluna (e não a repete no texto); a do docker fica para
            // as que não trazem. Hora desligada: o texto vai cru.
            const own = showTime && !line.continuation ? ownTimestamp(line.text) : null;
            const text = own ? own.rest : line.text;
            const rowTone =
              line.level === "error" && !line.continuation
                ? "bg-tyba-red/[.08]"
                : hit
                  ? "bg-tyba-blue/[.06]"
                  : "";
            return (
              <div
                key={line.seq}
                data-index={item.index}
                ref={virtualizer.measureElement}
                className={`absolute left-0 flex items-start gap-3.5 px-[18px] py-0.5 leading-5 ${
                  wrap ? "right-0" : "w-max min-w-full whitespace-pre"
                } ${rowTone}`}
                style={{ top: item.start, minHeight: ROW_PX }}
              >
                {showTime && (
                  <span className="w-[64px] shrink-0 text-tyba-text-ghost">
                    {line.continuation ? "" : (own?.clock ?? clockOf(line.ts_ms))}
                  </span>
                )}
                {hasLevels && (
                  <span
                    className={`w-[44px] shrink-0 ${LEVEL_CLASS[line.level ?? ""] ?? "text-tyba-text-faint"}`}
                  >
                    {line.continuation ? "" : (line.level ?? "")}
                  </span>
                )}
                <span
                  className={`${wrap ? "min-w-0 flex-1 whitespace-pre-wrap [overflow-wrap:anywhere]" : ""} ${
                    // Mockup: o erro e o aviso tingem o texto; o filtro que
                    // acende traz o texto cheio.
                    line.continuation
                      ? "text-tyba-text-muted"
                      : line.level === "error"
                        ? "text-tyba-red"
                        : line.level === "warn"
                          ? "text-tyba-amber"
                          : hit
                            ? "text-tyba-text"
                            : "text-tyba-text-muted"
                  }`}
                >
                  {highlightParts(text, query).map((part, i) =>
                    part.hit ? (
                      <mark key={i} className="rounded-[2px] bg-tyba-amber/25 text-tyba-text">
                        {part.text}
                      </mark>
                    ) : (
                      <span key={i}>{part.text}</span>
                    ),
                  )}
                </span>
                {line.frame && (
                  <button
                    type="button"
                    disabled={!line.frame.host_path}
                    title={
                      line.frame.host_path
                        ? line.frame.host_path
                        : t("logsOpenInEditorUnmapped")
                    }
                    onClick={() => {
                      const frame = line.frame;
                      if (!frame?.host_path) return;
                      void openEditorTab(frame.host_path, frame.line).catch((e) =>
                        toastError(t("editorOpenError"), e),
                      );
                    }}
                    className="shrink-0 text-tyba-blue underline disabled:cursor-not-allowed disabled:text-tyba-text-faint disabled:no-underline"
                  >
                    {t("logsOpenInEditor")}
                  </button>
                )}
              </div>
            );
          })}
        </div>
      </div>

      <div className="flex h-[26px] shrink-0 items-center gap-4 overflow-hidden whitespace-nowrap border-t border-tyba-border bg-[var(--tyba-panel-bg)] px-[18px] text-[11px] text-tyba-text-faint">
        <span className="shrink-0">
          {t("logsLines", { count: lines.length })} ·{" "}
          <span className={totals.errors > 0 ? "text-tyba-red" : ""}>
            {t("editorErrors", { count: totals.errors })}
          </span>{" "}
          ·{" "}
          <span className={totals.warnings > 0 ? "text-tyba-amber" : ""}>
            {t("editorWarnings", { count: totals.warnings })}
          </span>
        </span>
        <span className="min-w-0 flex-1 truncate text-right" title={t("logsRedactNote")}>
          {t("logsRedactNote")}
        </span>
      </div>
    </div>
  );
}
