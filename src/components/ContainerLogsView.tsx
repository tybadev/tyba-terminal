import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  Check,
  Clock,
  Copy,
  DownloadSimple,
  MagnifyingGlass,
  Warning,
  WarningCircle,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { FilterChip } from "@/components/ui/filter-chip";
import { Input } from "@/components/ui/input";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

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
  /** Host SSH (alias) do container; ausente = docker local. */
  host?: string | null;
  /** Muda a cada "Reiniciar": religa o `docker logs -f`. */
  restartNonce: number;
}

const ROW_PX = 22; // quadro Docker: linha de log de 22px

/** Logs do container como lista: hora, nível, filtro que acende, links de
 *  stack trace para o editor. O parse já veio do core. */
export function ContainerLogsView({ containerId, host = null, restartNonce }: Props) {
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
    void dockerLogsStart(containerId, host)
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
  }, [containerId, host, restartNonce]);

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

  return (
    <div className="absolute inset-0 flex min-h-0 min-w-0 flex-col">
      {/* Quadro Docker: a linha de filtros fica no respiro de 12px do bloco
          (sem fio próprio); o fio é o topo do log. */}
      <div className="flex shrink-0 flex-wrap items-center gap-x-2.5 gap-y-2 px-5 pt-3 pb-3">
        <Input
          icon={<MagnifyingGlass size={16} />}
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label={t("logsFilter")}
          placeholder={t("logsFilter")}
          className="w-[240px] shrink-0"
          trail={
            query.trim() ? (
              <span className="shrink-0 text-[12px]">
                {t("logsMatchCount", { matches, total: visible.length })}
              </span>
            ) : undefined
          }
        />
        <FilterChip active={chip === "all"} onClick={() => setChip("all")} label={t("logsAll")} />
        <FilterChip
          active={chip === "errors"}
          onClick={() => setChip("errors")}
          icon={<WarningCircle />}
          label={t("logsErrors")}
        />
        <FilterChip
          active={chip === "warnings"}
          onClick={() => setChip("warnings")}
          icon={<Warning />}
          label={t("logsWarnings")}
        />
        {/* Período: chip com relógio (quadro), aberto como o Select do app. */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <FilterChip
              aria-label={t("logsWindow")}
              icon={<Clock />}
              label={t(`logsWindow_${window_}`)}
            />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="max-h-64">
            {TIME_WINDOWS.map((w) => (
              <DropdownMenuItem
                key={w}
                onSelect={() => {
                  setWindow(w as TimeWindow);
                  setNow(Date.now());
                }}
                className="justify-between gap-3 text-xs"
              >
                {t(`logsWindow_${w}`)}
                {w === window_ && <Check size={12} weight="bold" className="text-primary" />}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
        <FilterChip active={showTime} aria-pressed={showTime} onClick={() => setShowTime((v) => !v)}>
          {t("logsTime")}
        </FilterChip>
        <FilterChip active={wrap} aria-pressed={wrap} onClick={() => setWrap((v) => !v)}>
          {t("logsWrap")}
        </FilterChip>
        <FilterChip
          aria-pressed={follow}
          onClick={() => setFollow((v) => !v)}
          className={follow && !ended ? "text-tyba-green hover:text-tyba-green" : undefined}
        >
          <span
            aria-hidden
            className={`size-1.5 rounded-full ${
              follow && !ended
                ? "tyba-pulse bg-tyba-green shadow-[var(--tyba-glow-green)]"
                : "bg-tyba-text-faint"
            }`}
          />
          {ended ? t("logsEnded") : follow ? t("logsFollowing") : t("logsPaused")}
        </FilterChip>
        <Button variant="ghost" size="sm" onClick={() => void copy()}>
          <Copy />
          {t("logsCopy")}
        </Button>
        <Button variant="ghost" size="sm" onClick={() => void download()}>
          <DownloadSimple />
          {t("logsDownload")}
        </Button>
      </div>

      {/* O fio do topo do log fica dentro do respiro de 20px (`.q-dk-log`). */}
      <div aria-hidden className="mx-5 h-px shrink-0 bg-[var(--tyba-line)]" />
      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="min-h-0 flex-1 overflow-auto py-1.5 font-mono text-[12px]"
      >
        {error && <div className="px-5 py-2 text-tyba-red">{error}</div>}
        {!error && visible.length === 0 && (
          <div className="px-5 py-2 text-tyba-text-faint">{t("logsEmpty")}</div>
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
            // Quadro Docker: o erro só tinge o texto (sem faixa nem coluna de
            // nível); só o filtro que acende ganha fundo.
            const rowTone = hit ? "bg-tyba-blue/[.06]" : "";
            return (
              <div
                key={line.seq}
                data-index={item.index}
                ref={virtualizer.measureElement}
                className={`absolute left-0 flex items-start gap-3 px-5 leading-[22px] ${
                  wrap ? "right-0" : "w-max min-w-full whitespace-pre"
                } ${rowTone}`}
                style={{ top: item.start, minHeight: ROW_PX }}
              >
                {showTime && (
                  <span className="w-[72px] shrink-0 font-medium text-tyba-text-faint">
                    {line.continuation ? "" : (own?.clock ?? clockOf(line.ts_ms))}
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

      <div className="flex shrink-0 items-center gap-4 overflow-hidden whitespace-nowrap px-5 pt-3 pb-3.5 text-[12px] leading-4 text-tyba-text-muted">
        <span className="shrink-0">
          {t("logsLines", { count: lines.length })} ·{" "}
          <span className={totals.errors > 0 ? "font-bold text-tyba-red" : ""}>
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
