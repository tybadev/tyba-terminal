import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { MagnifyingGlass } from "@phosphor-icons/react";

import { PANEL_SECTION, RAISED_ROW, SidePanelHeader } from "./SidePanelBar";
import { formatDuration } from "../lib/blockLayout";
import { writeClipboardText } from "../lib/clipboard";
import { translateError } from "../lib/errors";
import { runOutcome, runTime, shortCwd } from "../lib/historyPanel";
import { historyRuns, onSessionCommand, type HistoryRun, type Session } from "../lib/ipc";

const LIMIT = 200;

/**
 * Painel Histórico: as execuções que o shell da sessão reportou (OSC 633),
 * uma por linha, mais recente primeiro. Clicar põe o comando na linha de
 * comando do painel — nunca executa. A paleta (⌘R) segue existindo à parte.
 */
export function HistoryPanel({
  session,
  repoRoot,
  onInsert,
  onClose,
}: {
  session: Session;
  repoRoot: string | null;
  onInsert: (command: string) => void;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [query, setQuery] = useState("");
  const [repo, setRepo] = useState(false);
  const [runs, setRuns] = useState<HistoryRun[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState<number | null>(null);

  const refresh = useCallback(() => {
    historyRuns(session.id, repo, query, LIMIT)
      .then((r) => {
        setRuns(r);
        setError(null);
      })
      .catch((e) => setError(translateError(e, t)));
  }, [session.id, repo, query, t]);

  useEffect(() => {
    const timer = window.setTimeout(refresh, 120);
    return () => window.clearTimeout(timer);
  }, [refresh]);

  // Comando novo terminou na sessão → entra no topo sem esperar.
  useEffect(() => {
    let un: (() => void) | null = null;
    let cancelled = false;
    // A gravação é assíncrona (fila do core): um respiro antes de reler.
    void onSessionCommand(session.id, () => {
      window.setTimeout(refresh, 400);
    }).then((f) => {
      if (cancelled) f();
      else un = f;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, [session.id, refresh]);

  const now = Date.now();

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <SidePanelHeader kind="history" title={t("sidePanelHistory")} onClose={onClose} />

      <div className={`${PANEL_SECTION} flex flex-col gap-2 px-4 py-2.5`}>
        <label className="flex items-center gap-2 rounded-[6px] border border-tyba-border bg-tyba-sunken px-2.5 py-1.5 text-[12px] text-tyba-text-faint">
          <MagnifyingGlass size={13} className="shrink-0" />
          <input
            type="text"
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && query) {
                e.preventDefault();
                setQuery("");
              }
            }}
            placeholder={t("historyPanelSearch")}
            aria-label={t("historyPanelSearch")}
            className="min-w-0 flex-1 bg-transparent font-mono text-tyba-text outline-none placeholder:text-tyba-text-faint"
          />
        </label>
        {repoRoot && (
          <div role="group" className="flex gap-1 text-[11px]">
            {([false, true] as const).map((r) => (
              <button
                key={String(r)}
                type="button"
                aria-pressed={repo === r}
                onClick={() => setRepo(r)}
                className={`rounded-[5px] px-2 py-0.5 outline-none focus-visible:ring-2 focus-visible:ring-tyba-cyan ${
                  repo === r ? `${RAISED_ROW} text-tyba-text` : "text-tyba-text-faint hover:text-tyba-text"
                }`}
              >
                {r ? t("historyScopeRepo") : t("historyScopeSession")}
              </button>
            ))}
          </div>
        )}
      </div>

      <ul className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
        {error && (
          <li role="alert" className="px-2 py-2 text-[12px] text-tyba-red">
            {error}
          </li>
        )}
        {runs?.length === 0 && (
          <li className="px-2 py-2 text-[12px] text-tyba-text-faint">
            {query ? t("historyPanelNoMatch") : t("historyEmpty")}
          </li>
        )}
        {runs?.map((run, i) => {
          const outcome = runOutcome(run.exit_code);
          const duration =
            run.duration_ms != null
              ? formatDuration(run.duration_ms, i18n.language)
              : null;
          return (
            <li key={`${run.started_at_ms}:${i}`} className="group relative">
              <button
                type="button"
                onClick={() => onInsert(run.command)}
                title={t("historyPanelInsert")}
                className="flex w-full flex-col gap-1 rounded-[6px] px-2.5 py-2 text-left outline-none hover:bg-tyba-raised hover:shadow-[var(--tyba-row-raised)] focus-visible:ring-2 focus-visible:ring-tyba-cyan"
              >
                <span className="flex w-full items-start gap-2 font-mono text-[12.5px]">
                  <span aria-hidden className="shrink-0 text-tyba-green">❯</span>
                  <span className="min-w-0 flex-1 truncate pr-14 text-tyba-text">
                    {run.command}
                  </span>
                </span>
                <span className="flex w-full items-center gap-2 pl-4 text-[11px] text-tyba-text-faint">
                  <span className="min-w-0 truncate" title={run.cwd ?? undefined}>
                    {shortCwd(run.cwd, repoRoot)}
                  </span>
                  <span className="flex-1" />
                  {outcome.kind === "ok" && (
                    <span className="shrink-0 text-tyba-green" aria-label={t("historyPanelOk")}>
                      ✓
                    </span>
                  )}
                  {outcome.kind === "failed" && (
                    <span className="shrink-0 text-tyba-red">
                      ✗ {outcome.code}
                    </span>
                  )}
                  {duration && <span className="shrink-0">{duration}</span>}
                  <span className="shrink-0">{runTime(run.started_at_ms, now, i18n.language)}</span>
                </span>
              </button>
              <button
                type="button"
                onClick={() =>
                  void writeClipboardText(run.command).then(() => {
                    setCopied(i);
                    window.setTimeout(() => setCopied((c) => (c === i ? null : c)), 1500);
                  })
                }
                className="absolute right-2 top-1.5 rounded-[5px] border border-tyba-border px-2 py-0.5 text-[11px] text-tyba-text-muted opacity-0 outline-none hover:text-tyba-text focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-tyba-cyan group-hover:opacity-100"
              >
                {copied === i ? t("historyPanelCopied") : t("historyPanelCopy")}
              </button>
            </li>
          );
        })}
      </ul>

      <p className="shrink-0 px-4 pb-3 pt-1 text-[11px] text-tyba-text-faint">
        {t("historyPanelNote")}
      </p>
    </div>
  );
}
