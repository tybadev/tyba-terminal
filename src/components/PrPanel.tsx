import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowSquareOut, GithubLogo, GitlabLogo } from "@phosphor-icons/react";

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
import {
  forgePrList,
  forgeStatus,
  type ForgeStatus,
  type PullRequest,
  type SessionId,
} from "../lib/ipc";
import {
  overallChecksTone,
  type CheckTone,
} from "../lib/forge";
import {
  shouldShowPrIcon,
  sortPullRequestsByNumberDesc,
  toPrStatus,
} from "../lib/prPanel";
import { translateError } from "../lib/errors";
import { openExternalUrl } from "../lib/clipboard";

const TONE_DOT_CLASS: Record<CheckTone, string> = {
  success: "bg-tyba-green",
  failure: "bg-tyba-red",
  pending: "bg-tyba-amber",
};

export function PrPanel({
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
  const [prs, setPrs] = useState<PullRequest[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setStatus(undefined);
    if (!sessionId || !repoRoot) {
      setStatus(null);
      return;
    }
    let cancelled = false;
    void forgeStatus(sessionId)
      .then((next) => {
        if (!cancelled) setStatus(next);
      })
      .catch(() => {
        if (!cancelled) setStatus(null);
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, repoRoot]);

  const loadPrs = useCallback(() => {
    if (!sessionId) return;
    setLoading(true);
    setError(null);
    forgePrList(sessionId)
      .then((list) => setPrs(sortPullRequestsByNumberDesc(list)))
      .catch((e) => setError(translateError(e, t)))
      .finally(() => setLoading(false));
  }, [sessionId, t]);

  useEffect(() => {
    if (!open) return;
    setPrs(null);
    setError(null);
    loadPrs();
  }, [open, loadPrs]);

  if (!shouldShowPrIcon(status)) return null;

  const isMr = status?.kind === "gitlab";
  const title = t(isMr ? "headerPrPanelTitleMr" : "headerPrPanelTitle");

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
              {isMr ? <GitlabLogo size={16} /> : <GithubLogo size={16} />}
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent side="bottom">{title}</TooltipContent>
      </Tooltip>
      <PopoverContent align="end" className="w-80">
        <div className="tyba-label flex items-center justify-between px-2 py-1.5">
          {title}
        </div>
        <div
          role="separator"
          aria-orientation="horizontal"
          className="-mx-1 my-1 h-px bg-tyba-border"
        />
        {loading && (
          <div className="px-2 py-5 text-center text-xs text-tyba-text-faint">
            {t(isMr ? "headerPrPanelLoadingMr" : "headerPrPanelLoading")}
          </div>
        )}
        {!loading && error && (
          <div className="px-2 py-5 text-center text-xs text-tyba-red">
            {error}
          </div>
        )}
        {!loading && !error && prs && prs.length === 0 && (
          <div className="px-2 py-5 text-center text-xs text-tyba-text-faint">
            {t(isMr ? "headerPrPanelEmptyMr" : "headerPrPanelEmpty")}
          </div>
        )}
        {!loading && !error && prs && prs.length > 0 && (
          <div className="flex max-h-80 flex-col gap-1 overflow-y-auto p-1">
            {prs.map((pr) => {
              const tone = overallChecksTone(pr.checks);
              return (
                <button
                  key={pr.number}
                  type="button"
                  onClick={() => void openExternalUrl(pr.url)}
                  className="flex items-center gap-2 rounded-md p-2 text-left text-xs hover:bg-tyba-text/[.04]"
                >
                  <PrStatusIcon
                    status={toPrStatus(pr.state)}
                    size={14}
                    className="shrink-0"
                  />
                  <span className="min-w-0 flex-1 truncate">{pr.title}</span>
                  <span className="shrink-0 font-mono text-tyba-text-faint">
                    #{pr.number}
                  </span>
                  {tone && (
                    <span
                      className={`shrink-0 size-1.5 rounded-full ${TONE_DOT_CLASS[tone]}`}
                      aria-label={t(`prCheckTone_${tone}`)}
                    />
                  )}
                  <ArrowSquareOut
                    size={11}
                    className="shrink-0 text-tyba-text-faint"
                  />
                </button>
              );
            })}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}
