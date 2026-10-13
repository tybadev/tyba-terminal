import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  CaretDown,
  DownloadSimple,
  GitCommit,
  MagnifyingGlass,
  X,
} from "@phosphor-icons/react";

import { GitIcon } from "./icons/GitIcon";
import { translateError } from "../lib/errors";
import { requestConfirm } from "../lib/confirm";
import {
  HISTORY_PAGE,
  appendPage,
  chipLabel,
  chipTone,
  cleanFilterText,
  edgePath,
  entriesOf,
  graphColor,
  graphColumnWidth,
  initials,
  laneX,
  type HistoryEntry,
  relativeTime,
  secondsSince,
  shortSha,
  historyKey,
  shouldLoadMore,
  type ChipTone,
} from "../lib/gitHistory";
import {
  onRepoChanged,
  repoCommitAction,
  repoCommitGraphWrite,
  repoLogStats,
  repoCommitDetail,
  repoCommitHunks,
  repoLog,
  sessionBranches,
  sessionFetch,
  sessionGitStatus,
  type BranchInfo,
  type CommitAction,
  type CommitDetail,
  type FileDiff,
  type FileHunks,
  type GraphRow,
  type LogCursor,
  type LogFilter,
  type LogStats,
  type RefLabel,
  type Session,
} from "../lib/ipc";

const ROW_PX = 30;

const CHIP: Record<ChipTone, string> = {
  head: "bg-tyba-blue-tint text-tyba-blue",
  // No escuro o texto das refs locais/remotas é o acento clareado (mockup
  // GitHistorico: #a78bfa e #8fa9f5); no claro, o acento cheio.
  local: "bg-tyba-violet-tint text-tyba-violet dark:text-[color-mix(in_srgb,var(--tyba-violet)_70%,var(--tyba-text))]",
  remote: "border border-tyba-blue/35 text-tyba-blue leading-[15px] dark:text-[color-mix(in_srgb,var(--tyba-blue)_60%,var(--tyba-text))]",
  tag: "border border-tyba-border-strong text-tyba-text-muted leading-[15px]",
  agent: "bg-tyba-amber-tint text-tyba-amber",
};

const STATUS_LETTER: Record<FileDiff["status"], { letter: string; cls: string }> = {
  added: { letter: "A", cls: "text-tyba-green" },
  modified: { letter: "M", cls: "text-tyba-amber" },
  deleted: { letter: "D", cls: "text-tyba-red" },
  renamed: { letter: "R", cls: "text-tyba-amber" },
  copied: { letter: "C", cls: "text-tyba-green" },
  type_changed: { letter: "T", cls: "text-tyba-amber" },
  unmerged: { letter: "U", cls: "text-tyba-red" },
  other: { letter: "?", cls: "text-tyba-text-faint" },
};

/** Botão da barra de ferramentas (`.tb` do mockup). */
const TB =
  "flex h-[26px] items-center gap-1.5 rounded-[6px] bg-tyba-raised px-2.5 text-[12px] text-tyba-text-muted shadow-[var(--tyba-raised-ring)] outline-none hover:text-tyba-text focus-visible:ring-2 focus-visible:ring-tyba-blue disabled:opacity-50";

function Chip({ refLabel }: { refLabel: RefLabel }) {
  return (
    <span
      className={`shrink-0 rounded-[4px] px-1.5 text-[10.5px] leading-[17px] ${CHIP[chipTone(refLabel)]}`}
    >
      {chipLabel(refLabel)}
    </span>
  );
}

/** A coluna do grafo de UMA linha. As linhas da lista são contíguas, então
 *  a base de uma encosta no topo da seguinte e os fios se emendam. */
function GraphCell({
  graph,
  width,
  head,
  selected,
}: {
  graph: GraphRow;
  width: number;
  head: boolean;
  selected: boolean;
}) {
  const cx = laneX(graph.lane);
  const cy = ROW_PX / 2;
  const fill = graphColor(graph.color);
  return (
    <svg
      width={width}
      height={ROW_PX}
      viewBox={`0 0 ${width} ${ROW_PX}`}
      aria-hidden="true"
      className="shrink-0 overflow-hidden"
    >
      <g fill="none" strokeWidth={2} strokeLinecap="round">
        {graph.edges.map((e, i) => (
          <path key={i} d={edgePath(e, graph.lane, ROW_PX)} stroke={graphColor(e.color)} />
        ))}
      </g>
      <circle
        cx={cx}
        cy={cy}
        r={head ? 5.5 : 4.5}
        fill={fill}
        stroke={selected ? "var(--tyba-text)" : "var(--tyba-sunken)"}
        strokeWidth={selected ? 1.5 : 2}
      />
      {head && <circle cx={cx} cy={cy} r={2} fill="var(--tyba-sunken)" />}
    </svg>
  );
}

function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs]);
  return now;
}

function useDebounced<T>(value: T, ms: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(value), ms);
    return () => window.clearTimeout(timer);
  }, [value, ms]);
  return debounced;
}

function useRelative() {
  const { t, i18n } = useTranslation();
  return useCallback(
    (iso: string, now: number) => {
      const rel = relativeTime(iso, now);
      switch (rel.unit) {
        case "now":
          return t("gitLogAgoNow");
        case "min":
          return t("gitLogAgoMin", { count: rel.count });
        case "hour":
          return t("gitLogAgoHour", { count: rel.count });
        case "yesterday":
          return t("gitLogYesterday");
        case "days":
          return t("gitLogAgoDays", { count: rel.count });
        case "date":
          return rel.date.toLocaleDateString(i18n.language, {
            day: "numeric",
            month: "short",
            year: "numeric",
          });
      }
    },
    [t, i18n.language],
  );
}

/** Campo de filtro que aparece no lugar do botão (Autor ▾ / Caminho ▾). */
function FilterField({
  label,
  placeholder,
  value,
  onChange,
}: {
  label: string;
  placeholder: string;
  value: string;
  onChange: (v: string) => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  if (!open && !value) {
    return (
      <button type="button" className={TB} onClick={() => setOpen(true)}>
        {label}
        <CaretDown size={10} className="text-tyba-text-faint" />
      </button>
    );
  }
  return (
    <label className="flex h-[26px] items-center gap-1.5 rounded-[6px] bg-tyba-sunken px-2.5 text-[12px] text-tyba-text-faint shadow-[inset_0_0_0_1px_var(--tyba-border-field)]">
      {label}
      <input
        autoFocus={open && !value}
        type="text"
        value={value}
        placeholder={placeholder}
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        autoComplete="off"
        onChange={(e) => onChange(e.target.value)}
        onBlur={() => !value && setOpen(false)}
        className="w-28 bg-transparent text-tyba-text outline-none placeholder:text-tyba-text-faint"
      />
      <button
        type="button"
        aria-label={t("gitLogClearFilter", { label })}
        onClick={() => {
          onChange("");
          setOpen(false);
        }}
        className="text-tyba-text-faint hover:text-tyba-text"
      >
        <X size={11} />
      </button>
    </label>
  );
}

/**
 * Aba "git · histórico" (mockup GitHistorico): log paginado do core em lotes
 * de 200, lista virtualizada (só as linhas visíveis existem no DOM) e o
 * detalhe do commit selecionado à direita. Nada de estado de git aqui: a UI
 * guarda as linhas recebidas e devolve o cursor opaco para pedir mais.
 */
export function GitHistoryView({ session }: { session: Session }) {
  const { t, i18n } = useTranslation();
  const relative = useRelative();
  const now = useNow(5000);

  const [refName, setRefName] = useState<string | null>(null);
  const [allBranches, setAllBranches] = useState(false);
  const [query, setQuery] = useState("");
  const [author, setAuthor] = useState("");
  const [path, setPath] = useState("");
  const debouncedQuery = useDebounced(query, 250);
  const debouncedAuthor = useDebounced(author, 250);
  const debouncedPath = useDebounced(path, 250);

  const filter: LogFilter = useMemo(
    () => ({
      refName,
      allBranches,
      query: cleanFilterText(debouncedQuery),
      author: cleanFilterText(debouncedAuthor),
      path: cleanFilterText(debouncedPath),
    }),
    [refName, allBranches, debouncedQuery, debouncedAuthor, debouncedPath],
  );

  const [rows, setRows] = useState<HistoryEntry[]>([]);
  const [stats, setStats] = useState<LogStats | null>(null);
  const [writingGraph, setWritingGraph] = useState(false);
  const [cursor, setCursor] = useState<LogCursor | null>(null);
  const [filtered, setFiltered] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loadedAt, setLoadedAt] = useState<number | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [branches, setBranches] = useState<BranchInfo[]>([]);
  const [fetching, setFetching] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  /** Sobe depois de fetch/ação: branches e total mudaram. */
  const [refsVersion, setRefsVersion] = useState(0);
  /** Descarta resposta de uma leitura que já foi substituída por outra. */
  const generation = useRef(0);
  const loadingRef = useRef(false);

  const loadFirst = useCallback(() => {
    const gen = ++generation.current;
    loadingRef.current = true;
    setLoading(true);
    repoLog(session.id, filter, null, HISTORY_PAGE)
      .then((page) => {
        if (gen !== generation.current) return;
        setRows(entriesOf(page));
        setCursor(page.cursor);
        setFiltered(page.filtered);
        setError(null);
        setLoadedAt(Date.now());
      })
      .catch((e) => {
        if (gen === generation.current) setError(translateError(e, t));
      })
      .finally(() => {
        if (gen !== generation.current) return;
        loadingRef.current = false;
        setLoading(false);
      });
  }, [session.id, filter, t]);

  const loadMore = useCallback(() => {
    if (!cursor || loadingRef.current) return;
    const gen = generation.current;
    loadingRef.current = true;
    setLoading(true);
    repoLog(session.id, filter, cursor, HISTORY_PAGE)
      .then((page) => {
        if (gen !== generation.current) return;
        setRows((prev) => appendPage(prev, entriesOf(page)));
        setCursor(page.cursor);
      })
      .catch((e) => {
        if (gen === generation.current) setError(translateError(e, t));
      })
      .finally(() => {
        if (gen !== generation.current) return;
        loadingRef.current = false;
        setLoading(false);
      });
  }, [session.id, filter, cursor, t]);

  useEffect(() => {
    loadFirst();
  }, [loadFirst]);

  // Total e commit-graph à parte: o `rev-list --count` anda o histórico todo
  // e a primeira página não espera por ele.
  const refreshStats = useCallback(() => {
    let alive = true;
    repoLogStats(session.id, filter)
      .then((s) => alive && setStats(s))
      .catch(() => alive && setStats(null));
    return () => {
      alive = false;
    };
  }, [session.id, filter]);
  useEffect(() => refreshStats(), [refreshStats, refsVersion]);

  const writeCommitGraph = async () => {
    const ok = await requestConfirm({
      title: t("gitLogCommitGraphTitle"),
      detail: t("gitLogCommitGraphDetail"),
      confirmLabel: t("gitLogCommitGraphAction"),
    });
    if (!ok) return;
    setWritingGraph(true);
    try {
      await repoCommitGraphWrite(session.id);
      setNotice(t("gitLogCommitGraphDone"));
      refreshStats();
    } catch (e) {
      setNotice(translateError(e, t));
    } finally {
      setWritingGraph(false);
    }
  };

  useEffect(() => {
    sessionBranches(session.id)
      .then((list) => setBranches(list.branches))
      .catch(() => setBranches([]));
  }, [session.id, refsVersion]);

  // Commit/checkout/fetch mudam o histórico: relê a primeira página — mas só
  // se ninguém rolou além dela, para não arrancar a lista debaixo do dedo.
  // O evento vale para qualquer repo aberto: só o desta sessão interessa.
  const pagesLoaded = useRef(0);
  pagesLoaded.current = Math.ceil(rows.length / HISTORY_PAGE);
  const rootRef = useRef<string | null>(null);
  useEffect(() => {
    sessionGitStatus(session.id)
      .then((s) => {
        rootRef.current = s?.root ?? null;
      })
      .catch(() => {});
  }, [session.id]);
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void onRepoChanged((snap) => {
      if (rootRef.current && snap.root !== rootRef.current) return;
      if (pagesLoaded.current <= 1 && !loadingRef.current) loadFirst();
    }).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [loadFirst]);

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_PX,
    overscan: 12,
  });
  const items = virtualizer.getVirtualItems();
  const lastVisible = items.length > 0 ? items[items.length - 1].index : -1;
  useEffect(() => {
    if (
      shouldLoadMore({
        lastVisible,
        count: rows.length,
        hasMore: cursor !== null,
        loading,
      })
    ) {
      loadMore();
    }
  }, [lastVisible, rows.length, cursor, loading, loadMore]);

  const fetchNow = async () => {
    setFetching(true);
    setNotice(null);
    try {
      await sessionFetch(session.id);
      setRefsVersion((v) => v + 1);
      loadFirst();
    } catch (e) {
      setNotice(translateError(e, t));
    } finally {
      setFetching(false);
    }
  };

  // Teclado: ↑/↓ movem o foco (e o detalhe, se aberto), Enter abre, `c`
  // copia o hash. O foco é por sha: sobrevive a página nova no fim.
  const [focused, setFocused] = useState<string | null>(null);
  const focusIdx = focused ? rows.findIndex((e) => e.row.sha === focused) : -1;
  const onListKey = (e: React.KeyboardEvent) => {
    const action = historyKey(e, focusIdx, rows.length);
    if (!action) return;
    e.preventDefault();
    if (action.kind === "move") {
      const sha = rows[action.index].row.sha;
      setFocused(sha);
      if (selected) setSelected(sha);
      virtualizer.scrollToIndex(action.index, { align: "auto" });
    } else if (action.kind === "open" && focused) {
      setSelected(focused);
    } else if (action.kind === "copy" && focused) {
      void navigator.clipboard
        ?.writeText(focused)
        .then(() => setNotice(t("gitLogHashCopied", { sha: shortSha(focused) })))
        .catch(() => {});
    }
  };

  const currentBranch = branches.find((b) => b.is_current)?.name ?? null;
  const showGraph = !filtered && rows.length > 0;
  const graphWidth = useMemo(() => graphColumnWidth(rows), [rows]);
  const ago = secondsSince(loadedAt, now);
  const hasFilter = !!(filter.query || filter.author || filter.path);

  return (
    <div className="absolute inset-0 flex flex-col bg-tyba-sunken font-mono text-[13px] text-tyba-text">
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-tyba-border bg-[var(--tyba-panel-bg)] px-3.5 text-[12px] text-tyba-text-muted">
        <label className={`${TB} relative text-tyba-blue`}>
          <GitIcon size={12} />
          <select
            aria-label={t("gitLogBranch")}
            value={refName ?? ""}
            disabled={allBranches}
            onChange={(e) => setRefName(e.target.value || null)}
            className="max-w-[180px] cursor-pointer appearance-none truncate bg-transparent pr-3 text-tyba-blue outline-none"
          >
            <option value="">{currentBranch ?? "HEAD"}</option>
            {branches
              .filter((b) => !b.is_current)
              .map((b) => (
                <option key={b.name} value={b.name}>
                  {b.name}
                </option>
              ))}
          </select>
          <CaretDown size={10} className="pointer-events-none absolute right-2 text-tyba-text-faint" />
        </label>
        <button
          type="button"
          aria-pressed={allBranches}
          className={TB}
          onClick={() => setAllBranches((v) => !v)}
        >
          <span
            className={`relative inline-block h-3 w-[22px] rounded-full shadow-[inset_0_0_0_1px_var(--tyba-border-strong)] ${
              allBranches ? "bg-tyba-blue-tint" : "bg-tyba-overlay"
            }`}
          >
            <span
              className={`absolute top-0.5 h-2 w-2 rounded-full transition-[left] ${
                allBranches ? "left-3 bg-tyba-blue" : "left-0.5 bg-tyba-text-faint"
              }`}
            />
          </span>
          {t("gitLogAllBranches")}
        </button>
        <label className="flex h-[26px] w-[260px] items-center gap-1.5 rounded-[6px] bg-tyba-sunken px-2.5 shadow-[inset_0_0_0_1px_var(--tyba-border)]">
          <MagnifyingGlass size={12} className="shrink-0 text-tyba-text-faint" />
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("gitLogSearch")}
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            autoComplete="off"
            className="min-w-0 flex-1 bg-transparent text-[12px] text-tyba-text outline-none placeholder:text-tyba-text-faint"
          />
        </label>
        <FilterField
          label={t("gitLogAuthor")}
          placeholder={t("gitLogAuthorPlaceholder")}
          value={author}
          onChange={setAuthor}
        />
        <FilterField
          label={t("gitLogPath")}
          placeholder={t("gitLogPathPlaceholder")}
          value={path}
          onChange={setPath}
        />
        <span className="flex-1" />
        {notice && (
          <span role="alert" className="max-w-[260px] truncate text-[11px] text-tyba-red" title={notice}>
            {notice}
          </span>
        )}
        {ago !== null && (
          <span className="text-[11px] text-tyba-text-faint">
            {t("gitLogUpdated", { count: ago })}
          </span>
        )}
        <button type="button" className={TB} disabled={fetching} onClick={() => void fetchNow()}>
          <DownloadSimple size={12} />
          {fetching ? t("gitLogFetching") : t("gitLogFetch")}
        </button>
      </div>

      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex h-7 shrink-0 items-center gap-3 border-b border-tyba-border-soft px-3.5 text-[11px] text-tyba-text-faint">
            {showGraph && (
              <span className="-ml-3.5 shrink-0 pl-3.5" style={{ width: graphWidth }}>
                {t("gitLogColGraph")}
              </span>
            )}
            <span className="min-w-0 flex-1">{t("gitLogColMessage")}</span>
            <span className="w-[140px]">{t("gitLogColAuthor")}</span>
            <span className="w-[78px] text-right">{t("gitLogColDate")}</span>
            <span className="w-[62px] text-right">{t("gitLogColCommit")}</span>
          </div>
          <div
            ref={scrollRef}
            className="relative min-h-0 flex-1 overflow-y-auto outline-none"
            role="listbox"
            tabIndex={0}
            aria-label={t("tabGitHistory")}
            aria-activedescendant={focused ? `gitlog-${focused}` : undefined}
            onKeyDown={onListKey}
          >
            <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
              {items.map((item) => {
                const { row, graph } = rows[item.index];
                const isSel = row.sha === selected;
                const isFocus = row.sha === focused;
                return (
                  <div
                    key={row.sha}
                    id={`gitlog-${row.sha}`}
                    role="option"
                    aria-selected={isSel}
                    onClick={() => {
                      setFocused(row.sha);
                      setSelected(isSel ? null : row.sha);
                    }}
                    className={`absolute left-0 right-0 flex cursor-default items-center gap-3 pr-3.5 text-[12.5px] ${
                      showGraph ? "" : "pl-3.5"
                    } ${
                      isSel
                        ? "bg-tyba-raised shadow-[var(--tyba-edge),inset_2px_0_0_var(--tyba-blue)]"
                        : "hover:bg-tyba-raised/60"
                    } ${isFocus && !isSel ? "shadow-[inset_2px_0_0_var(--tyba-border-strong)]" : ""}`}
                    style={{ top: item.start, height: ROW_PX }}
                  >
                    {showGraph &&
                      (graph ? (
                        <GraphCell
                          graph={graph}
                          width={graphWidth}
                          head={row.refs.some((r) => r.is_head)}
                          selected={isSel}
                        />
                      ) : (
                        <span className="shrink-0" style={{ width: graphWidth }} />
                      ))}
                    <span className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden whitespace-nowrap">
                      {row.refs.map((r) => (
                        <Chip key={`${r.kind}:${r.name}`} refLabel={r} />
                      ))}
                      <span
                        className={`truncate ${row.parents.length > 1 ? "text-tyba-text-muted" : ""}`}
                        title={row.subject}
                      >
                        {row.subject}
                      </span>
                    </span>
                    <span
                      className={`w-[140px] truncate text-[12px] ${
                        isSel ? "text-tyba-text" : "text-tyba-text-muted"
                      }`}
                    >
                      {row.author}
                    </span>
                    <span
                      className={`w-[78px] text-right text-[11px] ${
                        isSel ? "text-tyba-text-muted" : "text-tyba-text-faint"
                      }`}
                    >
                      {relative(row.authored_at, now)}
                    </span>
                    <span
                      className={`w-[62px] text-right text-[11px] ${
                        isSel ? "text-tyba-text-muted" : "text-tyba-text-faint"
                      }`}
                    >
                      {shortSha(row.sha)}
                    </span>
                  </div>
                );
              })}
            </div>
            {!loading && rows.length === 0 && (
              <div className="p-6 text-center text-[12px] text-tyba-text-faint">
                {error ?? (hasFilter ? t("gitLogEmptyFiltered") : t("gitLogEmpty"))}
              </div>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-2 border-t border-tyba-border-soft px-3.5 py-2.5 text-[11px] text-tyba-text-faint">
            {loading && (
              <>
                <span className="inline-block h-3 w-3 animate-spin rounded-full border-2 border-tyba-border-strong border-t-tyba-blue" />
                <span className="text-tyba-text-muted">
                  {t("gitLogLoadingMore", { count: HISTORY_PAGE })}
                </span>
              </>
            )}
            <span>
              {stats?.total != null && !filtered
                ? t("gitLogTotal", {
                    count: rows.length,
                    total: stats.total.toLocaleString(i18n.language),
                  })
                : t("gitLogShowing", { count: rows.length })}
              {" · "}
              {cursor ? t("gitLogOnDemand") : t("gitLogEnd")}
            </span>
            {filtered && <span>· {t("gitLogFilteredNoGraph")}</span>}
            <span className="flex-1" />
            {stats?.suggest_commit_graph && (
              <button
                type="button"
                disabled={writingGraph}
                onClick={() => void writeCommitGraph()}
                className="text-tyba-text-faint underline decoration-dotted underline-offset-2 hover:text-tyba-text-muted disabled:opacity-50"
              >
                {writingGraph ? t("gitLogCommitGraphRunning") : t("gitLogCommitGraphHint")}
              </button>
            )}
            {error && rows.length > 0 && (
              <span role="alert" className="truncate text-tyba-red">
                {error}
              </span>
            )}
          </div>
        </div>

        {selected && (
          <CommitDetailPanel
            key={selected}
            session={session}
            sha={selected}
            onClose={() => setSelected(null)}
            onPickParent={setSelected}
            onChanged={(msg) => {
              setNotice(msg);
              setRefsVersion((v) => v + 1);
              loadFirst();
            }}
          />
        )}
      </div>
    </div>
  );
}

function Hunks({ hunks }: { hunks: FileHunks | "loading" | string }) {
  const { t } = useTranslation();
  if (hunks === "loading") {
    return <div className="px-2 py-1 text-[11px] text-tyba-text-faint">…</div>;
  }
  if (typeof hunks === "string") {
    return <div className="px-2 py-1 text-[11px] text-tyba-red">{hunks}</div>;
  }
  if (hunks.binary) {
    return <div className="px-2 py-1 text-[11px] text-tyba-text-faint">{t("gitLogBinary")}</div>;
  }
  if (hunks.hunks.length === 0) {
    return <div className="px-2 py-1 text-[11px] text-tyba-text-faint">{t("gitLogNoHunks")}</div>;
  }
  return (
    <div className="my-1 overflow-x-auto rounded-[4px] bg-tyba-sunken text-[11px] leading-[17px]">
      {hunks.hunks.map((h, i) => (
        <div key={i}>
          <div className="px-2 text-tyba-text-faint">{h.header}</div>
          {h.lines.map((l, j) => (
            <div
              key={j}
              className={`whitespace-pre px-2 ${
                l.kind === "add"
                  ? "bg-tyba-green/[.07] text-tyba-text"
                  : l.kind === "del"
                    ? "bg-tyba-red/[.07] text-tyba-text"
                    : "text-tyba-text-muted"
              }`}
            >
              <span
                className={`mr-2 inline-block w-2 ${
                  l.kind === "add" ? "text-tyba-green" : l.kind === "del" ? "text-tyba-red" : ""
                }`}
              >
                {l.kind === "add" ? "+" : l.kind === "del" ? "−" : " "}
              </span>
              {l.text}
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

/** "Ver diff" abre no máximo estes arquivos de uma vez (cada um é um `git diff`). */
const EXPAND_ALL_MAX = 20;

function CommitDetailPanel({
  session,
  sha,
  onClose,
  onPickParent,
  onChanged,
}: {
  session: Session;
  sha: string;
  onClose: () => void;
  onPickParent: (sha: string) => void;
  onChanged: (message: string) => void;
}) {
  const { t, i18n } = useTranslation();
  const [detail, setDetail] = useState<CommitDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<Record<string, FileHunks | "loading" | string>>({});
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let alive = true;
    repoCommitDetail(session.id, sha)
      .then((d) => alive && setDetail(d))
      .catch((e) => alive && setError(translateError(e, t)));
    return () => {
      alive = false;
    };
  }, [session.id, sha, t]);

  const toggleFile = (f: FileDiff) => {
    if (open[f.path] !== undefined) {
      setOpen(({ [f.path]: _drop, ...rest }) => rest);
      return;
    }
    setOpen((prev) => ({ ...prev, [f.path]: "loading" }));
    repoCommitHunks(session.id, sha, f.path, f.old_path)
      .then((h) => setOpen((prev) => (prev[f.path] === undefined ? prev : { ...prev, [f.path]: h })))
      .catch((e) =>
        setOpen((prev) =>
          prev[f.path] === undefined ? prev : { ...prev, [f.path]: translateError(e, t) },
        ),
      );
  };

  const anyOpen = Object.keys(open).length > 0;
  const toggleAll = () => {
    if (!detail) return;
    if (anyOpen) {
      setOpen({});
      return;
    }
    for (const f of detail.files.slice(0, EXPAND_ALL_MAX)) toggleFile(f);
  };

  const run = async (action: CommitAction) => {
    setActionError(null);
    const status = await sessionGitStatus(session.id).catch(() => null);
    const dirty = status?.dirty ?? false;
    const label = t(ACTION_LABEL[action]);
    if (dirty) {
      const ok = await requestConfirm({
        title: t("gitLogDirtyTitle"),
        detail: t("gitLogDirtyDetail", { action: label, sha: shortSha(sha) }),
        confirmLabel: label,
      });
      if (!ok) return;
    }
    setBusy(true);
    try {
      await repoCommitAction(session.id, sha, action, dirty);
      onChanged(t(`gitLogDone_${action}`, { sha: shortSha(sha) }));
    } catch (e) {
      setActionError(translateError(e, t));
    } finally {
      setBusy(false);
    }
  };

  const copy = () => {
    void navigator.clipboard
      ?.writeText(sha)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  };

  const totals = useMemo(() => {
    let ins = 0;
    let del = 0;
    for (const f of detail?.files ?? []) {
      ins += f.insertions ?? 0;
      del += f.deletions ?? 0;
    }
    return { ins, del };
  }, [detail]);

  const date = detail
    ? new Date(detail.authored_at).toLocaleString(i18n.language, {
        day: "numeric",
        month: "short",
        year: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      })
    : "";

  return (
    <aside
      aria-label={`${t("gitLogCommit")} ${shortSha(sha)}`}
      className="flex w-[380px] min-w-0 shrink-0 flex-col border-l border-tyba-border bg-[var(--tyba-panel-bg)]"
    >
      <header className="flex shrink-0 items-center gap-2.5 border-b border-tyba-border px-4 py-3">
        <GitCommit size={16} className="shrink-0 text-tyba-blue" />
        <span className="flex-1">
          {t("gitLogCommit")} <span className="text-tyba-blue">{shortSha(sha)}</span>
        </span>
        <button
          type="button"
          onClick={onClose}
          aria-label={t("sidePanelClose")}
          className="text-tyba-text-faint hover:text-tyba-text"
        >
          <X size={13} />
        </button>
      </header>
      {error && <p className="p-4 text-[12px] text-tyba-red">{error}</p>}
      {detail && (
        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
          <div className="flex flex-col gap-2 border-b border-tyba-border-soft px-4 py-3.5">
            <div className="font-medium [overflow-wrap:anywhere]">{detail.subject}</div>
            {detail.body && (
              <div className="whitespace-pre-wrap text-[12px] leading-[1.55] text-tyba-text-muted [overflow-wrap:anywhere]">
                {detail.body}
              </div>
            )}
          </div>
          <div className="flex flex-col border-b border-tyba-border-soft px-4 py-2.5 text-[12px]">
            <div className="flex items-center py-1">
              <span className="w-16 shrink-0 text-tyba-text-faint">{t("gitLogDetailAuthor")}</span>
              <span className="flex min-w-0 flex-1 items-center gap-2" title={detail.author_email}>
                <span className="flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full bg-tyba-blue-tint text-[9px] text-tyba-blue">
                  {initials(detail.author_name)}
                </span>
                <span className="truncate">{detail.author_name}</span>
              </span>
            </div>
            <div className="flex py-1">
              <span className="w-16 shrink-0 text-tyba-text-faint">{t("gitLogDetailDate")}</span>
              <span className="text-tyba-text-muted">{date}</span>
            </div>
            <div className="flex py-1">
              <span className="w-16 shrink-0 text-tyba-text-faint">{t("gitLogDetailHash")}</span>
              <span className="break-all text-[11px] text-tyba-text-muted">{detail.sha}</span>
            </div>
            {detail.parents.length > 0 && (
              <div className="flex py-1">
                <span className="w-16 shrink-0 text-tyba-text-faint">{t("gitLogDetailParents")}</span>
                <span className="flex flex-wrap gap-2">
                  {detail.parents.map((p) => (
                    <button
                      key={p}
                      type="button"
                      onClick={() => onPickParent(p)}
                      className="text-tyba-violet hover:underline"
                    >
                      {shortSha(p)}
                    </button>
                  ))}
                </span>
              </div>
            )}
            {detail.refs.length > 0 && (
              <div className="flex py-1">
                <span className="w-16 shrink-0 text-tyba-text-faint">{t("gitLogDetailRefs")}</span>
                <span className="flex min-w-0 flex-wrap gap-1.5">
                  {detail.refs.map((r) => (
                    <Chip key={`${r.kind}:${r.name}`} refLabel={r} />
                  ))}
                </span>
              </div>
            )}
          </div>
          <div className="flex flex-1 flex-col gap-0.5 px-4 py-3">
            <div className="flex justify-between pb-1.5 text-[12px] text-tyba-text-muted">
              <span>{t("gitLogFilesChanged", { count: detail.files.length + detail.files_truncated })}</span>
              <span>
                <span className="text-tyba-green">+{totals.ins}</span>{" "}
                <span className="text-tyba-red">−{totals.del}</span>
              </span>
            </div>
            {detail.files.map((f) => {
              const st = STATUS_LETTER[f.status];
              return (
                <div key={f.path}>
                  <button
                    type="button"
                    aria-expanded={open[f.path] !== undefined}
                    onClick={() => toggleFile(f)}
                    className="flex w-full items-center justify-between gap-2 py-1 text-left text-[12px] hover:text-tyba-text"
                  >
                    <span className="flex min-w-0 items-center gap-2">
                      <span className={`shrink-0 ${st.cls}`}>{st.letter}</span>
                      <span className="truncate" title={f.old_path ? `${f.old_path} → ${f.path}` : f.path}>
                        {f.path}
                      </span>
                    </span>
                    <span className="shrink-0 text-[11px]">
                      {f.insertions === null ? (
                        <span className="text-tyba-text-faint">bin</span>
                      ) : (
                        <>
                          {f.insertions > 0 && <span className="text-tyba-green">+{f.insertions}</span>}
                          {f.insertions > 0 && (f.deletions ?? 0) > 0 && " "}
                          {(f.deletions ?? 0) > 0 && <span className="text-tyba-red">−{f.deletions}</span>}
                        </>
                      )}
                    </span>
                  </button>
                  {open[f.path] !== undefined && <Hunks hunks={open[f.path]} />}
                </div>
              );
            })}
            {detail.files_truncated > 0 && (
              <div className="py-1 text-[11px] text-tyba-text-faint">
                {t("gitLogFilesTruncated", { count: detail.files_truncated })}
              </div>
            )}
          </div>
          <div className="sticky bottom-0 flex flex-col gap-2 border-t border-tyba-border-soft bg-tyba-surface px-4 py-3">
            <button
              type="button"
              onClick={toggleAll}
              disabled={detail.files.length === 0}
              className="rounded-[6px] bg-tyba-blue px-3.5 py-[7px] text-[12px] text-white hover:bg-tyba-blue/90 disabled:opacity-50"
            >
              {anyOpen ? t("gitLogHideDiff") : t("gitLogViewDiff")}
            </button>
            <div className="grid grid-cols-2 gap-2">
              {(["checkout", "cherry_pick", "revert"] as const).map((action) => (
                <button
                  key={action}
                  type="button"
                  disabled={busy}
                  onClick={() => void run(action)}
                  className="rounded-[6px] border border-tyba-border-strong px-2.5 py-1.5 text-[12px] text-tyba-text-muted hover:text-tyba-text disabled:opacity-50"
                >
                  {t(ACTION_LABEL[action])}
                </button>
              ))}
              <button
                type="button"
                onClick={copy}
                className="rounded-[6px] border border-tyba-border-strong px-2.5 py-1.5 text-[12px] text-tyba-text-muted hover:text-tyba-text"
              >
                {copied ? t("gitLogCopied") : t("gitLogCopyHash")}
              </button>
            </div>
            {actionError && (
              <p role="alert" className="text-[11px] text-tyba-red">
                {actionError}
              </p>
            )}
            <p className="text-[11px] text-tyba-text-faint">{t("gitLogActionsNote")}</p>
          </div>
        </div>
      )}
    </aside>
  );
}

const ACTION_LABEL: Record<CommitAction, string> = {
  checkout: "gitLogCheckout",
  cherry_pick: "gitLogCherryPick",
  revert: "gitLogRevert",
};
