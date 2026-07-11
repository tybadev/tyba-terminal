import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowClockwise,
  ArrowUp,
  Circle,
  FileMagnifyingGlass,
  GitBranch,
  Play,
  Trash,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  onRepoChanged,
  worktreeList,
  worktreeRemove,
  type SessionId,
  type WorktreeListing,
  type WorktreeStatus,
} from "../lib/ipc";
import { basename } from "@/lib/utils";

interface Props {
  repoRoots: string[];
  onOpenSession: (path: string, name: string) => void;
  onFocusSession: (sessionId: SessionId) => void;
  onReviewSession: (sessionId: SessionId) => void;
}

function WorktreeRow({
  wt,
  onOpenSession,
  onFocusSession,
  onReviewSession,
  onRemoved,
}: {
  wt: WorktreeStatus;
  onOpenSession: Props["onOpenSession"];
  onFocusSession: Props["onFocusSession"];
  onReviewSession: Props["onReviewSession"];
  onRemoved: () => void;
}) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const risky = wt.dirty || wt.ahead_of_head > 0;
  const name = wt.branch ?? basename(wt.path);

  const remove = async () => {
    if (!confirming) {
      setConfirming(true);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await worktreeRemove(wt.path, true, risky);
      onRemoved();
    } catch (e) {
      setError(String(e));
      setBusy(false);
      setConfirming(false);
    }
  };

  return (
    <div className="flex flex-col gap-1 px-4 py-2.5">
      <div className="flex items-center gap-3">
        <GitBranch size={14} className="shrink-0 text-tyba-text-muted" />
        <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-tyba-text">
          {name}
        </span>
        {wt.dirty && (
          <span
            title={t("worktreeDirty")}
            className="flex items-center gap-1 text-[11px] text-tyba-yellow"
          >
            <Circle size={7} weight="fill" />
            {t("worktreeDirty")}
          </span>
        )}
        {wt.ahead_of_head > 0 && (
          <span className="flex items-center gap-0.5 font-mono text-[11px] text-tyba-text-muted">
            <ArrowUp size={11} />
            {wt.ahead_of_head}
          </span>
        )}
        {wt.session_id && (
          <Button
            variant="ghost"
            size="sm"
            className="h-6 gap-1.5 px-2 text-[11px]"
            onClick={() => onReviewSession(wt.session_id as SessionId)}
          >
            <FileMagnifyingGlass size={12} />
            {t("worktreeReview")}
          </Button>
        )}
        {wt.session_id ? (
          <button
            onClick={() => onFocusSession(wt.session_id as SessionId)}
            className="flex items-center gap-1.5 rounded-[4px] px-1.5 py-0.5 text-[11px] text-tyba-green transition-colors hover:bg-white/[.06]"
          >
            <span className="size-1.5 rounded-full bg-tyba-green [box-shadow:var(--tyba-glow-green)]" />
            {t("worktreeSessionActive")}
          </button>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            className="h-6 gap-1.5 px-2 text-[11px]"
            onClick={() => onOpenSession(wt.path, name)}
          >
            <Play size={12} />
            {t("worktreeOpenSession")}
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          disabled={busy || wt.session_id !== null}
          onClick={() => void remove()}
          onBlur={() => setConfirming(false)}
          className={`h-6 gap-1.5 px-2 text-[11px] ${
            confirming
              ? "bg-tyba-red/15 text-tyba-red hover:bg-tyba-red/25"
              : "text-tyba-text-muted"
          }`}
        >
          <Trash size={12} />
          {confirming
            ? risky
              ? t("worktreeRemoveConfirmRisky")
              : t("worktreeRemoveConfirm")
            : t("worktreeRemove")}
        </Button>
      </div>
      <div className="flex items-center gap-2 pl-[26px]">
        <span className="truncate font-mono text-[10px] text-tyba-text-faint">
          {wt.path}
        </span>
        {!wt.managed && (
          <span className="shrink-0 rounded-full bg-white/[.06] px-1.5 text-[9px] text-tyba-text-faint">
            {t("worktreeExternal")}
          </span>
        )}
      </div>
      {error && <div className="pl-[26px] text-[11px] text-tyba-red">{error}</div>}
    </div>
  );
}

export function WorktreesView({
  repoRoots,
  onOpenSession,
  onFocusSession,
  onReviewSession,
}: Props) {
  const { t } = useTranslation();
  const [listings, setListings] = useState<Record<string, WorktreeListing>>({});
  const [loading, setLoading] = useState(true);
  const rootsRef = useRef(repoRoots);
  rootsRef.current = repoRoots;

  const reload = useCallback(async () => {
    const results = await Promise.all(
      rootsRef.current.map((root) => worktreeList(root).catch(() => null)),
    );
    const next: Record<string, WorktreeListing> = {};
    for (const listing of results) {
      if (listing) next[listing.root] = listing;
    }
    setListings(next);
    setLoading(false);
  }, []);

  useEffect(() => {
    void reload();
  }, [reload, repoRoots]);

  useEffect(() => {
    let disposed = false;
    let timer: number | null = null;
    const unlisten = onRepoChanged(() => {
      if (disposed) return;
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(() => void reload(), 500);
    });
    return () => {
      disposed = true;
      if (timer !== null) window.clearTimeout(timer);
      void unlisten.then((un) => un());
    };
  }, [reload]);

  const repos = Object.values(listings).sort((a, b) =>
    a.root.localeCompare(b.root),
  );

  return (
    <div className="flex-1 overflow-y-auto bg-tyba-bg">
      <div className="mx-auto w-full max-w-2xl px-6 py-8">
        <div className="flex items-baseline justify-between pb-1">
          <h1 className="text-[15px] text-tyba-text">{t("worktreesTitle")}</h1>
          <Button
            variant="ghost"
            size="sm"
            className="h-6 gap-1.5 px-2 text-[11px] text-tyba-text-muted"
            onClick={() => void reload()}
          >
            <ArrowClockwise size={12} />
            {t("worktreesRefresh")}
          </Button>
        </div>
        <p className="pb-6 text-[12px] text-tyba-text-faint">
          {t("worktreesHint")}
        </p>

        {!loading && repos.length === 0 && (
          <div className="rounded-[8px] border border-dashed border-tyba-border p-8 text-center text-[12px] text-tyba-text-faint">
            {t("worktreesEmpty")}
          </div>
        )}

        <div className="flex flex-col gap-6">
          {repos.map((repo) => (
            <section key={repo.root}>
              <div className="flex items-baseline gap-2 pb-2">
                <span className="shrink-0 whitespace-nowrap text-[13px] text-tyba-text">
                  {basename(repo.root)}
                </span>
                <span className="truncate font-mono text-[10px] text-tyba-text-faint">
                  {repo.root}
                </span>
              </div>
              {repo.worktrees.length === 0 ? (
                <div className="rounded-[8px] border border-tyba-border px-4 py-3 text-[12px] text-tyba-text-faint">
                  {t("worktreesNoneForRepo")}
                </div>
              ) : (
                <div className="divide-y divide-tyba-border overflow-hidden rounded-[8px] border border-tyba-border">
                  {repo.worktrees.map((wt) => (
                    <WorktreeRow
                      key={wt.path}
                      wt={wt}
                      onOpenSession={onOpenSession}
                      onFocusSession={onFocusSession}
                      onReviewSession={onReviewSession}
                      onRemoved={() => void reload()}
                    />
                  ))}
                </div>
              )}
            </section>
          ))}
        </div>
      </div>
    </div>
  );
}
