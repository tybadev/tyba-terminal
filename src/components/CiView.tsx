import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useRelative } from "./GitHistoryView";
import { ForgeLoading } from "./ForgeLoading";
import { ciFailurePrompt, errorTarget, isErrorLine } from "../lib/ci";
import { openExternalUrl, writeClipboardText } from "../lib/clipboard";
import { requestConfirm } from "../lib/confirm";
import { translateError } from "../lib/errors";
import { hasRunningWork, jobTone, runTone, sortRuns, type CheckTone } from "../lib/forge";
import { pageVisible } from "../lib/prs";
import {
  forgeRunLog,
  forgeRunRerun,
  forgeWorkflowJobs,
  forgeWorkflowRuns,
  type RunLog,
  type Session,
  type WorkflowJob,
  type WorkflowRun,
} from "../lib/ipc";

const TONE_TEXT: Record<CheckTone, string> = {
  success: "text-tyba-green",
  failure: "text-tyba-red",
  pending: "text-tyba-amber",
};
const TONE_DOT: Record<CheckTone, string> = {
  success: "bg-tyba-green-fill",
  failure: "bg-tyba-red-fill",
  pending: "bg-tyba-amber-fill",
};
const STATUS_KEY: Record<CheckTone, string> = {
  success: "ciRunPassed",
  failure: "ciRunFailed",
  pending: "ciRunRunning",
};
const MARK: Record<CheckTone, string> = { success: "✓", failure: "✗", pending: "●" };

const BTN =
  "shrink-0 whitespace-nowrap rounded-[6px] border border-tyba-border-strong px-2.5 py-1 text-[11.5px] text-tyba-text-muted hover:text-tyba-text disabled:opacity-50";

/** Só corre com a seção aberta E com run em andamento (cada tick é um `gh`). */
const REFRESH_MS = 15000;

function duration(start: string | null, end: string | null): string {
  if (!start || !end) return "";
  const s = Math.round((new Date(end).getTime() - new Date(start).getTime()) / 1000);
  if (!Number.isFinite(s) || s < 0) return "";
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m${String(s % 60).padStart(2, "0")}s`;
}

/**
 * Seção CI da tela de Git: runs (desta branch ou de todas), jobs e passos do
 * run escolhido, e o log do job com a linha do erro realçada. Rodar de novo é
 * vermelho: confirmação a cada vez.
 */
export function CiView({
  session,
  branch,
  onSendToAgent,
}: {
  session: Session;
  branch: string | null;
  onSendToAgent: (prompt: string) => Promise<void>;
}) {
  const { t } = useTranslation();
  const relative = useRelative();
  const [scope, setScope] = useState<"branch" | "all">(branch ? "branch" : "all");
  const [runs, setRuns] = useState<WorkflowRun[] | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [runId, setRunId] = useState<number | null>(null);
  const [jobs, setJobs] = useState<WorkflowJob[] | null>(null);
  const [jobId, setJobId] = useState<number | null>(null);
  const [log, setLog] = useState<RunLog | null>(null);
  const [logError, setLogError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const logRef = useRef<HTMLDivElement>(null);

  const load = useCallback(
    (fresh: boolean) => {
      forgeWorkflowRuns(session.id, fresh)
        .then((r) => {
          setRuns(r);
          setError(null);
        })
        .catch((e) => setError(translateError(e, t)));
    },
    [session.id, t],
  );

  useEffect(() => load(true), [load]);
  useEffect(() => {
    if (!runs || !hasRunningWork(runs)) return;
    // GET condicional no core: run que não mudou não gasta limite. Janela
    // escondida não pergunta nada.
    const timer = window.setInterval(() => {
      if (pageVisible()) load(true);
    }, REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [runs, load]);

  const shown = useMemo(
    () => sortRuns((runs ?? []).filter((r) => scope === "all" || !branch || r.head_branch === branch)),
    [runs, scope, branch],
  );
  const run = shown.find((r) => r.id === runId) ?? shown[0] ?? null;

  // Jobs e log são rebuscados quando o run escolhido muda de estado (fila →
  // rodando → concluído) e depois do rerun; só a troca de run limpa a tela.
  const runKey = run ? `${run.id}:${run.status}:${run.conclusion ?? ""}` : null;
  const [jobsTick, setJobsTick] = useState(0);
  const [runsAt] = useState(() => Date.now());
  const [jobsAt, setJobsAt] = useState(() => Date.now());
  const [logAt, setLogAt] = useState(() => Date.now());
  const jobsRunId = useRef<number | null>(null);
  useEffect(() => {
    const changedRun = jobsRunId.current !== (run?.id ?? null);
    jobsRunId.current = run?.id ?? null;
    if (changedRun) {
      setJobs(null);
      setJobId(null);
      setJobsAt(Date.now());
    }
    if (!run) return;
    let alive = true;
    forgeWorkflowJobs(session.id, run.id)
      .then((j) => {
        if (!alive) return;
        setJobs(j);
        setJobId((prev) => {
          if (!changedRun && prev != null && j.some((x) => x.id === prev)) return prev;
          return (j.find((x) => x.conclusion === "failure") ?? j[0])?.id ?? null;
        });
      })
      .catch(() => alive && changedRun && setJobs([]));
    return () => {
      alive = false;
    };
  }, [session.id, runKey, jobsTick]); // eslint-disable-line react-hooks/exhaustive-deps

  const job = jobs?.find((j) => j.id === jobId) ?? null;

  const jobKey = job ? `${job.id}:${job.status}:${job.conclusion ?? ""}` : null;
  const logJobId = useRef<number | null>(null);
  useEffect(() => {
    const changedJob = logJobId.current !== (job?.id ?? null);
    logJobId.current = job?.id ?? null;
    if (changedJob) {
      setLog(null);
      setLogError(null);
      setLogAt(Date.now());
    }
    if (!job) return;
    let alive = true;
    forgeRunLog(session.id, job.id)
      .then((l) => {
        if (!alive) return;
        setLog(l);
        setLogError(null);
      })
      .catch((e) => alive && setLogError(translateError(e, t)));
    return () => {
      alive = false;
    };
  }, [session.id, jobKey, t]); // eslint-disable-line react-hooks/exhaustive-deps

  const lines = useMemo(() => (log ? log.text.split("\n") : []), [log]);

  const goToError = () => {
    const i = errorTarget(lines, log?.error_line);
    if (i < 0) return;
    logRef.current?.querySelector(`[data-line="${i}"]`)?.scrollIntoView({ block: "center" });
  };

  const rerun = async () => {
    if (!run) return;
    // `--failed` só quando o run falhou; o texto da confirmação diz qual dos dois.
    const failedOnly = run.conclusion === "failure";
    const ok = await requestConfirm({
      title: t("ciRerunConfirm", { name: run.name, branch: run.head_branch }),
      detail: t(failedOnly ? "ciRerunConfirmDetailFailed" : "ciRerunConfirmDetail"),
      confirmLabel: t("ciRerun"),
    });
    if (!ok) return;
    try {
      await forgeRunRerun(session.id, run.id, failedOnly);
      setNote(t("ciRerunStarted"));
      load(true);
      setJobsTick((n) => n + 1);
    } catch (e) {
      setNote(translateError(e, t));
    }
  };

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-2.5 tyba-divide-b px-[18px] py-2 text-[12px]">
        <div
          role="tablist"
          className="flex shrink-0 gap-0.5 rounded-[7px] bg-tyba-sunken p-0.5 shadow-[inset_0_0_0_1px_var(--tyba-border)]"
        >
          {(["branch", "all"] as const).map((key) => (
            <button
              key={key}
              type="button"
              role="tab"
              aria-selected={scope === key}
              disabled={key === "branch" && !branch}
              onClick={() => setScope(key)}
              className={`whitespace-nowrap rounded-[5px] px-2.5 py-1 ${
                scope === key
                  ? "bg-tyba-raised text-tyba-text shadow-[var(--tyba-raised-ring)]"
                  : "text-tyba-text-muted hover:text-tyba-text"
              }`}
            >
              {t(key === "branch" ? "ciScopeBranch" : "ciScopeAll")}
            </button>
          ))}
        </div>
        <span className="min-w-0 flex-1 truncate text-right text-[11px] text-tyba-text-faint">
          {note ?? t("ciAutoRefresh")}
        </span>
      </div>

      {error && <p className="p-6 text-[12px] text-tyba-red">{error}</p>}
      {runs === undefined && !error && (
        <ForgeLoading step={t("loadForgeRuns")} startedAt={runsAt} rows={4} className="min-h-0 flex-1" />
      )}
      {runs === null && <p className="p-6 text-[12px] text-tyba-text-faint">{t("ciUnsupported")}</p>}
      {runs && shown.length === 0 && <p className="p-6 text-[12px] text-tyba-text-faint">{t("ciEmpty")}</p>}

      {run && (
        <div className="flex min-h-0 flex-1">
          <ul className="flex w-[340px] shrink-0 flex-col gap-0.5 overflow-y-auto tyba-divide-r p-2">
            {shown.map((r) => {
              const tone = runTone(r);
              const on = r.id === run.id;
              return (
                <li key={r.id}>
                  <button
                    type="button"
                    onClick={() => setRunId(r.id)}
                    className={`flex w-full flex-col gap-0.5 rounded-[6px] px-2.5 py-2 text-left ${
                      on ? "bg-tyba-raised shadow-[var(--tyba-raised-ring)]" : "hover:bg-tyba-raised/60"
                    }`}
                  >
                    <span className="flex min-w-0 items-center gap-2 text-[12.5px]">
                      <span className={`size-1.5 shrink-0 rounded-full ${TONE_DOT[tone]}`} />
                      <span className="min-w-0 flex-1 truncate text-tyba-text">
                        {r.name}
                        <span className="text-[11px] text-tyba-text-faint"> · {r.event}</span>
                      </span>
                      <span className={`shrink-0 text-[11px] ${TONE_TEXT[tone]}`}>{t(STATUS_KEY[tone])}</span>
                    </span>
                    <span className="truncate pl-3.5 text-[11px] text-tyba-text-faint">
                      <span className="font-mono text-tyba-blue">{r.head_branch}</span> ·{" "}
                      {relative(r.created_at, Date.now())}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>

          <div className="flex w-[260px] shrink-0 flex-col gap-1 overflow-y-auto tyba-divide-r p-3">
            <span className="tyba-label px-1.5 pb-1.5">{t("ciJobs")}</span>
            {jobs === null && (
              <ForgeLoading
                step={t("loadForgeJobs")}
                startedAt={jobsAt}
                rows={3}
                className="-mx-1.5"
              />
            )}
            {jobs?.map((j) => {
              const tone = jobTone(j);
              return (
                <button
                  key={j.id}
                  type="button"
                  onClick={() => setJobId(j.id)}
                  className={`flex items-center gap-2 rounded-[6px] px-1.5 py-1.5 text-left text-[12.5px] ${
                    j.id === jobId ? "bg-tyba-raised shadow-[var(--tyba-raised-ring)]" : "hover:bg-tyba-raised/60"
                  }`}
                >
                  <span className={`w-3 shrink-0 ${TONE_TEXT[tone]}`}>{MARK[tone]}</span>
                  <span className="min-w-0 flex-1 truncate">{j.name}</span>
                  <span className="shrink-0 text-[11px] text-tyba-text-faint">
                    {duration(j.started_at, j.completed_at)}
                  </span>
                </button>
              );
            })}
            {job && job.steps.length > 0 && (
              <>
                <span className="tyba-label px-1.5 pb-1 pt-3.5">{t("ciSteps")}</span>
                {job.steps.map((s, i) => {
                  const tone = runTone(s);
                  return (
                    <span
                      key={`${i}:${s.name}`}
                      className={`flex gap-2 px-1.5 text-[12px] ${
                        tone === "failure" ? "text-tyba-text" : "text-tyba-text-muted"
                      }`}
                    >
                      <span className={`w-3 shrink-0 ${TONE_TEXT[tone]}`}>{MARK[tone]}</span>
                      <span className="min-w-0 truncate">{s.name}</span>
                    </span>
                  );
                })}
              </>
            )}
          </div>

          <div className="flex min-w-0 flex-1 flex-col bg-tyba-sunken">
            <div className="flex shrink-0 items-center gap-2.5 tyba-divide-b px-4 py-2 text-[12px]">
              <span className="min-w-0 truncate text-tyba-text">{log?.failed_step ?? job?.name ?? ""}</span>
              {log?.failed_step && <span className="shrink-0 text-[11px] text-tyba-red">{t("ciStepFailed")}</span>}
              <span className="flex-1" />
              <button type="button" className={BTN} disabled={!log} onClick={goToError}>
                {t("ciGoToError")}
              </button>
              <button
                type="button"
                className={BTN}
                disabled={!log}
                onClick={() => log && void writeClipboardText(log.text)}
              >
                {t("ciCopyLog")}
              </button>
            </div>
            <div ref={logRef} className="min-h-0 flex-1 overflow-auto py-2 font-mono text-[12px] leading-5">
              {logError && <p className="px-4 text-tyba-red">{logError}</p>}
              {!log && !logError && job && (
                <ForgeLoading
                  step={t("loadForgeLog", { job: job.name })}
                  em={[job.name]}
                  startedAt={logAt}
                  shape="lines"
                  rows={10}
                  className="-my-2"
                />
              )}
              {log?.truncated && <p className="px-4 pb-1 text-[11px] text-tyba-text-faint">{t("ciLogTruncated")}</p>}
              {lines.map((line, i) => (
                <div
                  key={i}
                  data-line={i}
                  className={`flex whitespace-pre ${
                    isErrorLine(line) || i === log?.error_line
                      ? "bg-tyba-diff-del-bg text-tyba-diff-del-text"
                      : "text-tyba-text-muted"
                  }`}
                >
                  <span className="w-11 shrink-0 select-none pr-3 text-right text-tyba-text-ghost">{i + 1}</span>
                  <span className="pr-4">{line}</span>
                </div>
              ))}
            </div>
            <div className="flex shrink-0 flex-wrap items-center gap-2 tyba-divide-t bg-[var(--tyba-panel-bg)] px-4 py-2.5">
              {/* Só em sessão de agente: numa de shell o prompt cairia no shell. */}
              {session.kind.type === "agent" && log && job && job.conclusion === "failure" && (
                <button
                  type="button"
                  onClick={() =>
                    void onSendToAgent(ciFailurePrompt(run, job, log.failed_step, log.text)).then(() =>
                      setNote(t("ciSentToAgent")),
                    )
                  }
                  className="shrink-0 whitespace-nowrap rounded-[6px] bg-tyba-violet px-3 py-1.5 text-[12px] text-white hover:bg-tyba-violet/90"
                >
                  {t("ciSendToAgent")}
                </button>
              )}
              <span className="flex-1" />
              <button type="button" className={BTN} onClick={() => void openExternalUrl(run.url)}>
                GitHub ↗
              </button>
              <button
                type="button"
                className={`${BTN} text-tyba-text`}
                disabled={run.status !== "completed"}
                onClick={() => void rerun()}
              >
                {t("ciRerun")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
