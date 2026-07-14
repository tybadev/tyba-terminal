import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CaretDown,
  CaretRight,
  GithubLogo,
  GitlabLogo,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { PrStatusIcon } from "@/components/icons/PrStatusIcon";
import { openExternalUrl } from "@/lib/clipboard";
import { translateError } from "@/lib/errors";
import {
  checkTone,
  hasRunningWork,
  jobTone,
  runTone,
  sortRuns,
  type CheckTone,
} from "@/lib/forge";
import {
  forgePrList,
  forgeStatus,
  forgeWorkflowJobs,
  forgeWorkflowRuns,
  type ForgeStatus,
  type PullRequest,
  type SessionId,
  type WorkflowJob,
  type WorkflowRun,
} from "@/lib/ipc";
import {
  shouldShowPrIcon,
  sortPullRequestsByNumberDesc,
  toPrStatus,
} from "@/lib/prPanel";

/**
 * Só corre com o painel aberto E com run em andamento. O core divide máquina com
 * os agentes: cada tick é um processo `gh` na rede, e um poll de fundo custaria
 * isso por sessão aberta — para mostrar o que ninguém está olhando.
 */
const REFRESH_MS = 15000;

type Tab = "ci" | "prs";

const TONE_DOT_CLASS: Record<CheckTone, string> = {
  success: "bg-tyba-green",
  failure: "bg-tyba-red",
  pending: "bg-tyba-amber",
};

function ToneDot({ tone }: { tone: CheckTone }) {
  return (
    <span
      aria-hidden="true"
      className={`size-1.5 shrink-0 rounded-full ${TONE_DOT_CLASS[tone]} ${
        tone === "pending" ? "motion-safe:animate-pulse" : ""
      }`}
    />
  );
}

function Message({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-2 py-5 text-center text-xs text-tyba-text-faint">
      {children}
    </div>
  );
}

function Expander({
  expanded,
  label,
  onToggle,
}: {
  expanded: boolean;
  label: string;
  onToggle: () => void;
}) {
  const Caret = expanded ? CaretDown : CaretRight;
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={expanded}
      aria-label={label}
      className="flex size-5 shrink-0 items-center justify-center rounded-md text-tyba-text-faint hover:bg-tyba-text/[.04] hover:text-tyba-text"
    >
      <Caret size={11} />
    </button>
  );
}

function Jobs({ sessionId, runId }: { sessionId: SessionId; runId: number }) {
  const { t } = useTranslation();
  const [jobs, setJobs] = useState<WorkflowJob[] | null>(null);

  useEffect(() => {
    let alive = true;
    void forgeWorkflowJobs(sessionId, runId)
      .then((j) => alive && setJobs(j))
      .catch(() => alive && setJobs([]));
    return () => {
      alive = false;
    };
  }, [sessionId, runId]);

  if (jobs === null) {
    return (
      <p className="py-1 pl-6 text-[11px] text-tyba-text-faint">
        {t("ciLoading")}
      </p>
    );
  }

  return (
    <ul className="flex flex-col">
      {jobs.map((job) => (
        <li key={job.name}>
          <button
            type="button"
            disabled={!job.url}
            onClick={() => job.url && void openExternalUrl(job.url)}
            className="flex w-full items-center gap-2 rounded-md py-1 pr-2 pl-6 text-left text-[11px] text-tyba-text-muted enabled:hover:bg-tyba-text/[.04] enabled:hover:text-tyba-text disabled:cursor-default"
          >
            <ToneDot tone={jobTone(job)} />
            <span className="min-w-0 flex-1 truncate">{job.name}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

function CiTab({ sessionId }: { sessionId: SessionId }) {
  const { t } = useTranslation();
  const [runs, setRuns] = useState<WorkflowRun[] | null>(null);
  const [supported, setSupported] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<number | null>(null);

  const load = useCallback(async () => {
    try {
      const result = await forgeWorkflowRuns(sessionId, true);
      // `null` = a forja não sabe olhar CI. Diferente de lista vazia, que é
      // "não há run": dizer "nenhum run" seria mentir sobre o GitLab.
      setSupported(result !== null);
      setRuns(result ?? []);
      setError(null);
    } catch (e) {
      setError(translateError(e, t));
      setRuns([]);
    }
  }, [sessionId, t]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!runs || !hasRunningWork(runs)) return;
    const timer = window.setInterval(() => void load(), REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [runs, load]);

  if (error) {
    return <div className="px-2 py-5 text-center text-xs text-tyba-red">{error}</div>;
  }
  if (runs === null) return <Message>{t("ciLoading")}</Message>;
  if (!supported) return <Message>{t("ciUnsupported")}</Message>;
  if (runs.length === 0) return <Message>{t("ciEmpty")}</Message>;

  return (
    <ul className="flex max-h-80 flex-col gap-0.5 overflow-y-auto p-1">
      {sortRuns(runs).map((run) => {
        const open = expanded === run.id;
        return (
          <li key={run.id}>
            <div className="flex items-center gap-1">
              <Expander
                expanded={open}
                label={t(open ? "ciCollapseRun" : "ciExpandRun")}
                onToggle={() => setExpanded(open ? null : run.id)}
              />
              <button
                type="button"
                onClick={() => void openExternalUrl(run.url)}
                className="flex min-w-0 flex-1 items-center gap-2 rounded-md p-1.5 text-left text-xs hover:bg-tyba-text/[.04]"
              >
                <ToneDot tone={runTone(run)} />
                <span className="min-w-0 truncate">{run.name}</span>
                <span className="ml-auto shrink-0 truncate font-mono text-[10px] text-tyba-text-faint">
                  {run.head_branch}
                </span>
              </button>
            </div>
            {open && <Jobs sessionId={sessionId} runId={run.id} />}
          </li>
        );
      })}
    </ul>
  );
}

function PrsTab({ sessionId, isMr }: { sessionId: SessionId; isMr: boolean }) {
  const { t } = useTranslation();
  const [prs, setPrs] = useState<PullRequest[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<number | null>(null);

  useEffect(() => {
    let alive = true;
    void forgePrList(sessionId)
      .then((list) => alive && setPrs(sortPullRequestsByNumberDesc(list)))
      .catch((e) => {
        if (!alive) return;
        setError(translateError(e, t));
        setPrs([]);
      });
    return () => {
      alive = false;
    };
  }, [sessionId, t]);

  if (error) {
    return <div className="px-2 py-5 text-center text-xs text-tyba-red">{error}</div>;
  }
  if (prs === null) return <Message>{t("ciLoading")}</Message>;
  if (prs.length === 0) {
    return (
      <Message>{t(isMr ? "headerPrPanelEmptyMr" : "headerPrPanelEmpty")}</Message>
    );
  }

  return (
    <ul className="flex max-h-80 flex-col gap-0.5 overflow-y-auto p-1">
      {prs.map((pr) => {
        const open = expanded === pr.number;
        return (
          <li key={pr.number}>
            <div className="flex items-center gap-1">
              <Expander
                expanded={open}
                label={t(open ? "ciCollapseRun" : "ciExpandRun")}
                onToggle={() => setExpanded(open ? null : pr.number)}
              />
              <button
                type="button"
                onClick={() => void openExternalUrl(pr.url)}
                className="flex min-w-0 flex-1 items-center gap-2 rounded-md p-1.5 text-left text-xs hover:bg-tyba-text/[.04]"
              >
                <PrStatusIcon
                  status={toPrStatus(pr.state)}
                  size={14}
                  className="shrink-0"
                />
                <span className="min-w-0 flex-1 truncate">{pr.title}</span>
                <span className="shrink-0 font-mono text-[10px] text-tyba-text-faint">
                  #{pr.number}
                </span>
              </button>
            </div>

            {/* O pontinho agregado escondia QUAL check quebrou — era a metade do
                dado que o core já buscava e a UI jogava fora. */}
            {open && (
              <ul className="flex flex-col">
                {pr.checks.length === 0 && (
                  <li className="py-1 pl-6 text-[11px] text-tyba-text-faint">
                    {t("ciEmpty")}
                  </li>
                )}
                {pr.checks.map((check) => (
                  <li
                    key={check.name}
                    className="flex items-center gap-2 py-1 pr-2 pl-6 text-[11px] text-tyba-text-muted"
                  >
                    <ToneDot tone={checkTone(check)} />
                    <span className="min-w-0 flex-1 truncate">{check.name}</span>
                  </li>
                ))}
              </ul>
            )}
          </li>
        );
      })}
    </ul>
  );
}

/**
 * CI e pull requests são o mesmo domínio — a forja. Dois ícones no header
 * separavam por implementação, não por assunto.
 */
export function ForgePanel({
  sessionId,
  repoRoot,
}: {
  sessionId: SessionId | null;
  repoRoot: string | null;
}) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<ForgeStatus | null | undefined>(
    undefined,
  );
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<Tab>("ci");

  useEffect(() => {
    setStatus(undefined);
    if (!sessionId || !repoRoot) {
      setStatus(null);
      return;
    }
    let alive = true;
    void forgeStatus(sessionId)
      .then((s) => alive && setStatus(s))
      .catch(() => alive && setStatus(null));
    return () => {
      alive = false;
    };
  }, [sessionId, repoRoot]);

  // Pré-carrega com o cache do core: o painel abre instantâneo em vez de piscar
  // "carregando", e trocar de aba de volta não gasta processo nenhum.
  useEffect(() => {
    if (!sessionId || !shouldShowPrIcon(status)) return;
    void forgeWorkflowRuns(sessionId, false).catch(() => {});
  }, [sessionId, status]);

  if (!sessionId || !shouldShowPrIcon(status)) return null;

  const isMr = status?.kind === "gitlab";
  const Logo = isMr ? GitlabLogo : GithubLogo;
  const title = t(isMr ? "headerMrPanelTitle" : "headerPrPanelTitle");

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label={title}
              className="size-6 rounded-[4px] text-tyba-text-muted hover:text-tyba-text"
            >
              <Logo size={16} />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent side="bottom">{title}</TooltipContent>
      </Tooltip>

      <PopoverContent align="end" className="w-80">
        <div className="flex items-center gap-0.5 p-0.5">
          {(["ci", "prs"] as Tab[]).map((value) => (
            <button
              key={value}
              type="button"
              onClick={() => setTab(value)}
              aria-pressed={tab === value}
              className={`flex h-6 flex-1 items-center justify-center rounded-[4px] text-[11px] transition-colors ${
                tab === value
                  ? "bg-tyba-text/[.06] text-tyba-text"
                  : "text-tyba-text-faint hover:text-tyba-text-muted"
              }`}
            >
              {value === "ci"
                ? t("ciPanelTitle")
                : t(isMr ? "headerMrPanelTitle" : "headerPrPanelTitle")}
            </button>
          ))}
        </div>

        <div role="separator" className="-mx-1 my-1 h-px bg-tyba-border" />

        {open && tab === "ci" && <CiTab sessionId={sessionId} />}
        {open && tab === "prs" && <PrsTab sessionId={sessionId} isMr={isMr} />}
      </PopoverContent>
    </Popover>
  );
}
