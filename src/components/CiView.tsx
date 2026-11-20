import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowClockwise, Copy, GithubLogo } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { SegmentedControl } from "@/components/ui/segmented";

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
  // Sem escolha, abre o último run concluído (o que tem log e resultado; o
  // quadro abre no que falhou com um rodando acima). Só run na fila: o primeiro.
  const run =
    shown.find((r) => r.id === runId) ?? shown.find((r) => r.status === "completed") ?? shown[0] ?? null;

  // Jobs e log são rebuscados quando o run escolhido muda de estado (fila →
  // rodando → concluído) e depois do rerun; só a troca de run limpa a tela.
  const runKey = run ? `${run.id}:${run.status}:${run.conclusion ?? ""}` : null;
  const [jobsTick, setJobsTick] = useState(0);
  const [runsAt, setRunsAt] = useState(() => Date.now());
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
      <div className="flex h-[46px] shrink-0 items-center gap-3 shadow-[inset_0_-1px_0_var(--tyba-line)] px-[18px] text-[12px]">
        <SegmentedControl
          tone="tint"
          aria-label={t("ciRunsScope")}
          value={scope}
          onChange={setScope}
          options={(["branch", "all"] as const).map((key) => ({
            value: key,
            label: t(key === "branch" ? "ciScopeBranch" : "ciScopeAll"),
            disabled: key === "branch" && !branch,
          }))}
        />
        <span className="min-w-0 flex-1 truncate text-right text-[12px] text-tyba-text-faint">
          {note ?? t("ciAutoRefresh")}
        </span>
      </div>

      {/* Falha de refresh com runs na tela fica no aviso; a primeira carga
          falha dentro do carregamento, com "Tentar de novo". */}
      {error && runs !== undefined && <p className="p-6 text-[12px] text-tyba-red">{error}</p>}
      {runs === undefined && (
        <ForgeLoading
          step={t("loadForgeRuns")}
          startedAt={runsAt}
          error={error}
          onRetry={() => {
            setError(null);
            setRunsAt(Date.now());
            load(true);
          }}
          rows={4}
          className="min-h-0 flex-1"
        />
      )}
      {runs === null && <p className="p-6 text-[12px] text-tyba-text-faint">{t("ciUnsupported")}</p>}
      {runs && shown.length === 0 && <p className="p-6 text-[12px] text-tyba-text-faint">{t("ciEmpty")}</p>}

      {run && (
        <div className="flex min-h-0 flex-1">
          <ul className="flex w-[357px] shrink-0 flex-col gap-0.5 overflow-y-auto shadow-[inset_-1px_0_0_var(--tyba-line)] p-2">
            {shown.map((r) => {
              const tone = runTone(r);
              const on = r.id === run.id;
              return (
                <li key={r.id}>
                  <button
                    type="button"
                    onClick={() => setRunId(r.id)}
                    className={`flex w-full flex-col gap-[3px] rounded-[10px] px-2.5 py-2.5 text-left leading-[normal] ${
                      on ? "tyba-pill-on" : "hover:bg-tyba-raised/60"
                    }`}
                  >
                    <span className="flex min-w-0 items-center gap-2 text-[13px] font-semibold">
                      <span className={`size-[7px] shrink-0 rounded-full ${TONE_DOT[tone]}`} />
                      <span className="min-w-0 truncate text-tyba-text">{r.name}</span>
                      <span className="min-w-0 truncate text-[11.5px] font-medium text-tyba-text-faint">· {r.event}</span>
                      <span className={`ml-auto shrink-0 text-[11.5px] ${TONE_TEXT[tone]}`}>{t(STATUS_KEY[tone])}</span>
                    </span>
                    <span className="truncate pl-[15px] text-[11.5px] text-tyba-text-faint">
                      <span className="font-mono text-tyba-blue">{r.head_branch}</span> ·{" "}
                      {relative(r.created_at, Date.now())}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>

          <div className="flex w-[281px] shrink-0 flex-col gap-0.5 overflow-y-auto shadow-[inset_-1px_0_0_var(--tyba-line)] px-2.5 py-3">
            <span className="tyba-label px-2 pb-1.5 pt-1 font-semibold">{t("ciJobs")}</span>
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
                  className={`flex h-8 shrink-0 items-center gap-2 rounded-[9px] px-2.5 text-left text-[13px] ${
                    j.id === jobId ? "tyba-pill-on" : "hover:bg-tyba-raised/60"
                  }`}
                >
                  <span className={`w-3 shrink-0 text-center font-bold ${TONE_TEXT[tone]}`}>{MARK[tone]}</span>
                  <span className="min-w-0 flex-1 truncate">{j.name}</span>
                  <span className="shrink-0 font-mono text-[11px] text-tyba-text-faint">
                    {duration(j.started_at, j.completed_at)}
                  </span>
                </button>
              );
            })}
            {job && job.steps.length > 0 && (
              <>
                <span className="tyba-label px-2 pb-1.5 pt-4 font-semibold">{t("ciSteps")}</span>
                {job.steps.map((s, i) => {
                  const tone = runTone(s);
                  return (
                    <span
                      key={`${i}:${s.name}`}
                      className={`flex h-[26px] shrink-0 items-center gap-2 px-2.5 text-[12.5px] ${
                        tone === "failure" ? "font-semibold text-tyba-text" : "text-tyba-text-muted"
                      }`}
                    >
                      <span className={`w-3 shrink-0 text-center font-bold ${TONE_TEXT[tone]}`}>{MARK[tone]}</span>
                      <span className="min-w-0 truncate">{s.name}</span>
                    </span>
                  );
                })}
              </>
            )}
          </div>

          <div className="flex min-w-0 flex-1 flex-col">
            <div className="flex h-11 shrink-0 items-center gap-2.5 shadow-[inset_0_-1px_0_var(--tyba-line)] px-4 text-[13px] font-semibold">
              <span className="min-w-0 truncate text-tyba-text">{log?.failed_step ?? job?.name ?? ""}</span>
              {log?.failed_step && <span className="shrink-0 text-[11.5px] text-tyba-red">{t("ciStepFailed")}</span>}
              <span className="flex-1" />
              <Button variant="ghost" size="sm" disabled={!log} onClick={goToError}>
                {t("ciGoToError")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                disabled={!log}
                onClick={() => log && void writeClipboardText(log.text)}
              >
                <Copy />
                {t("ciCopyLog")}
              </Button>
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
                  className={`grid grid-cols-[44px_1fr] gap-3.5 whitespace-pre ${
                    isErrorLine(line) || i === log?.error_line
                      ? "bg-[var(--tyba-danger-soft,var(--tyba-diff-del-bg))] text-[var(--tyba-mk-danger,var(--tyba-diff-del-text))]"
                      : "text-tyba-text-muted"
                  }`}
                >
                  <span className="select-none text-right text-tyba-text-faint opacity-60">{i + 1}</span>
                  <span className="pr-4">{line}</span>
                </div>
              ))}
            </div>
            <div className="flex min-h-[52px] shrink-0 flex-wrap items-center gap-2 shadow-[inset_0_1px_0_var(--tyba-line)] px-4 py-2.5">
              {/* Só em sessão de agente: numa de shell o prompt cairia no shell. */}
              {session.kind.type === "agent" && log && job && job.conclusion === "failure" && (
                <Button
                  size="sm"
                  onClick={() =>
                    void onSendToAgent(ciFailurePrompt(run, job, log.failed_step, log.text)).then(() =>
                      setNote(t("ciSentToAgent")),
                    )
                  }
                >
                  {t("ciSendToAgent")}
                </Button>
              )}
              <span className="flex-1" />
              <Button variant="ghost" size="sm" onClick={() => void openExternalUrl(run.url)}>
                <GithubLogo />
                GitHub ↗
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={run.status !== "completed"}
                onClick={() => void rerun()}
              >
                <ArrowClockwise />
                {t("ciRerun")}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
