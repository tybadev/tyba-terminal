import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowRight,
  ArrowSquareOut,
  ChatTeardrop,
  Check,
  Clock,
  GitCommit,
  GitMerge,
  GitPullRequest,
  X,
} from "@phosphor-icons/react";

import { useRelative } from "./GitHistoryView";
import { MarkdownView } from "./MarkdownView";
import { getEffectiveBase } from "@/theme";
import { openExternalUrl } from "../lib/clipboard";
import { requestConfirm } from "../lib/confirm";
import { translateError } from "../lib/errors";
import {
  forgePrDetail,
  forgePrMerge,
  forgePrReview,
  onForgeChanged,
  type PrMergeStrategy,
  type PrReviewEvent,
  type PrCheck,
  type PrDetail,
  type PrReview,
  type PrThread,
  type PrThreadComment,
  type Session,
  type SessionId,
} from "../lib/ipc";
import {
  buildTimeline,
  checkDuration,
  checksSummary,
  forgeChangeHits,
  forwardPrompt,
  initials,
  pageVisible,
  rateResetTime,
  reviewerStates,
  timelineEventId,
  type TimelineEvent,
  type Tone,
} from "../lib/prs";

type Section = "conversation" | "commits" | "files" | "checks";

const TONE_TEXT: Record<Tone, string> = {
  green: "text-tyba-green",
  amber: "text-tyba-amber",
  red: "text-tyba-red",
  muted: "text-tyba-text-faint",
  violet: "text-tyba-violet",
};

const REVIEW_VERB: Record<PrReview["state"], { key: string; tone: string }> = {
  approved: { key: "prReviewedApproved", tone: "text-tyba-green" },
  changes_requested: { key: "prReviewedChanges", tone: "text-tyba-red" },
  commented: { key: "prReviewedCommented", tone: "text-tyba-text-muted" },
  dismissed: { key: "prReviewedDismissed", tone: "text-tyba-text-faint" },
  pending: { key: "prReviewedCommented", tone: "text-tyba-text-muted" },
};

const AVATAR_TONES = [
  "bg-tyba-cyan/[.18] text-tyba-cyan",
  "bg-tyba-violet/[.2] text-tyba-violet",
  "bg-tyba-blue/[.2] text-tyba-blue",
  "bg-tyba-green/[.18] text-tyba-green",
];

function avatarTone(name: string): string {
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return AVATAR_TONES[h % AVATAR_TONES.length];
}

function Avatar({ name }: { name: string }) {
  return (
    <span
      className={`flex size-5 shrink-0 items-center justify-center rounded-full text-[10px] ${avatarTone(name)}`}
    >
      {initials(name)}
    </span>
  );
}

function CheckMark({ status }: { status: PrCheck["status"] }) {
  if (status === "success") return <span className="w-3 text-tyba-green">✓</span>;
  if (status === "failure") return <span className="w-3 text-tyba-red">✗</span>;
  if (status === "pending") return <span className="w-3 text-tyba-amber">●</span>;
  return <span className="w-3 text-tyba-text-faint">–</span>;
}

/** Últimas linhas do trecho do diff da thread (o mockup mostra duas). */
function hunkTail(hunk: string, lines = 3): { text: string; kind: "add" | "del" | "ctx" }[] {
  return hunk
    .split("\n")
    .filter((l) => !l.startsWith("@@"))
    .slice(-lines)
    .map((text) => ({
      text,
      kind: text.startsWith("+") ? "add" : text.startsWith("-") ? "del" : "ctx",
    }));
}

/**
 * Aba "PR #N" (mockup GitPR). Leitura: título, estado, linha do tempo,
 * commits, arquivos, checks, revisores e condição de merge. Conteúdo do
 * GitHub é de terceiros: aparece marcado e só vira prompt quando o usuário
 * encaminha (`onForward`).
 */
export function PrDetailView({
  session,
  number,
  onForward,
}: {
  session: Session;
  number: number;
  /** Manda o prompt para a sessão de agente ligada ao PR (`pr.session`). */
  onForward: (session: SessionId, prompt: string) => Promise<void>;
}) {
  const { t } = useTranslation();
  const relative = useRelative();
  const [pr, setPr] = useState<PrDetail | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [section, setSection] = useState<Section>("conversation");
  const [forwarded, setForwarded] = useState<Set<string>>(() => new Set());
  const [now, setNow] = useState(() => Date.now());
  const [reviewBody, setReviewBody] = useState("");
  const [bodyOpen, setBodyOpen] = useState(false);
  const dark = getEffectiveBase() === "dark";
  const [reviewEvent, setReviewEvent] = useState<PrReviewEvent>("comment");
  const [strategy, setStrategy] = useState<PrMergeStrategy>("squash");
  const [acting, setActing] = useState<null | "review" | "merge">(null);
  // `from` decide onde o aviso aparece: o de revisão no formulário, o de merge
  // na seção Merge (que continua na tela depois que o PR vira "merged").
  const [actionNote, setActionNote] = useState<
    { text: string; error: boolean; from: "review" | "merge" } | null
  >(null);

  const load = useCallback(
    (fresh: boolean) => {
      forgePrDetail(session.id, number, fresh)
        .then((d) => {
          setPr(d);
          setError(null);
        })
        .catch((e) => setError(translateError(e, t)));
    },
    [session.id, number, t],
  );

  // Mesmo esquema da lista: o poll lê o cache, o core revalida em segundo
  // plano e avisa por `forge://changed` quando este PR mudou.
  useEffect(() => {
    load(false);
    const timer = window.setInterval(() => {
      if (pageVisible()) load(false);
    }, 60_000);
    return () => window.clearInterval(timer);
  }, [load]);

  useEffect(() => {
    const unlisten = onForgeChanged((change) => {
      if (forgeChangeHits(change, null, number)) load(false);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, [number, load]);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);

  const timeline = useMemo(() => (pr ? buildTimeline(pr) : []), [pr]);
  const reviewers = useMemo(() => (pr ? reviewerStates(pr) : []), [pr]);
  const checks = useMemo(() => checksSummary(pr?.checks ?? []), [pr]);

  if (error && !pr) {
    return <p className="absolute inset-0 p-6 text-[12px] text-tyba-red">{error}</p>;
  }
  if (pr === undefined) {
    return <p className="absolute inset-0 p-6 text-[12px] text-tyba-text-faint">{t("diffLoading")}</p>;
  }
  if (pr === null) {
    return (
      <p className="absolute inset-0 p-6 text-[12px] text-tyba-text-faint">{t("prListUnsupported")}</p>
    );
  }

  const mergeReady =
    pr.state === "open" && pr.mergeable !== "conflicting" && checks.tone !== "red";

  const submitReview = async () => {
    if (acting) return;
    const body = reviewBody.trim();
    const ok = await requestConfirm({
      title: t(`prReviewConfirm_${reviewEvent}`, { number: pr.number }),
      detail: body ? `${pr.title}\n\n${body}` : pr.title,
      confirmLabel: t(`prReviewEvent_${reviewEvent}`),
      destructive: reviewEvent === "request_changes",
    });
    if (!ok) return;
    setActing("review");
    setActionNote(null);
    try {
      await forgePrReview(session.id, pr.number, reviewEvent, body, pr.head_sha);
      setReviewBody("");
      setActionNote({ text: t("prReviewSent"), error: false, from: "review" });
      load(true);
    } catch (e) {
      setActionNote({ text: translateError(e, t), error: true, from: "review" });
      load(true);
    } finally {
      setActing(null);
    }
  };

  const merge = async () => {
    if (acting) return;
    const ok = await requestConfirm({
      title: t("prMergeConfirm", { number: pr.number, base: pr.base }),
      detail: `${pr.title}\n${pr.head} → ${pr.base} · ${t(
        strategy === "squash" ? "prMergeSquash" : strategy === "merge" ? "prMergeCommit" : "prMergeRebase",
      )} · ${pr.head_sha.slice(0, 7)}`,
      confirmLabel: t("prMergeAction"),
      destructive: true,
    });
    if (!ok) return;
    setActing("merge");
    setActionNote(null);
    try {
      await forgePrMerge(session.id, pr.number, strategy, pr.head_sha);
      setActionNote({ text: t("prMerged"), error: false, from: "merge" });
      load(true);
    } catch (e) {
      setActionNote({ text: translateError(e, t), error: true, from: "merge" });
      load(true);
    } finally {
      setActing(null);
    }
  };

  const target = pr.session?.agent ? pr.session.session : null;
  const forwardButton = (id: string, prompt: () => string) =>
    target && (
      <button
        type="button"
        disabled={forwarded.has(id)}
        onClick={() =>
          void onForward(target, prompt()).then(() =>
            setForwarded((prev) => new Set(prev).add(id)),
          )
        }
        className="flex shrink-0 items-center gap-1 rounded-[5px] border border-tyba-border-strong px-2 py-0.5 text-[11px] text-tyba-text-muted hover:text-tyba-text disabled:opacity-60"
      >
        {forwarded.has(id) ? <Check size={11} /> : <ArrowRight size={11} />}
        {forwarded.has(id) ? t("prForwarded") : t("prForward")}
      </button>
    );

  const externalTag = (
    <span className="shrink-0 rounded-[4px] bg-tyba-text/[.06] px-1.5 text-[10px] text-tyba-text-faint">
      {t("prExternal")}
    </span>
  );

  const reply = (c: PrThreadComment, i: number) => (
    <div key={i} className="flex gap-2 tyba-divide-t bg-tyba-text/[.015] px-2.5 py-2 text-[12px]">
      <Avatar name={c.author} />
      <div className="flex min-w-0 flex-col gap-0.5">
        <span>
          {c.author}{" "}
          <span className="text-tyba-text-faint">
            {t("prReplied")} · {relative(c.created_at, now)}
          </span>
        </span>
        <div className="min-w-0 text-tyba-text-muted">
          <MarkdownView text={c.body} dark={dark} compact />
        </div>
      </div>
    </div>
  );

  const threadCard = (thread: PrThread, id: string) => {
    const [first, ...rest] = thread.comments;
    return (
      <div className="flex flex-col overflow-hidden rounded-[8px] border border-tyba-border bg-tyba-raised/60">
        <div className="flex items-center gap-2 tyba-divide-b px-2.5 py-2 text-[12px]">
          <Avatar name={first.author} />
          <span>{first.author}</span>
          <span className="text-tyba-text-muted">{t("prReviewedCommented")}</span>
          <span className="text-tyba-text-faint">· {relative(first.created_at, now)}</span>
          <span className="flex-1" />
          {externalTag}
          {forwardButton(id, () =>
            forwardPrompt(pr, {
              author: first.author,
              verb: t("prReviewedCommented"),
              body: [first, ...rest].map((c) => `${c.author}: ${c.body}`).join("\n"),
              path: thread.path,
              line: thread.line,
            }),
          )}
        </div>
        <div className="px-2.5 pt-2 text-[11px] text-tyba-text-muted">
          {thread.path}
          {thread.line !== null && <span className="text-tyba-text-faint">:{thread.line}</span>}
        </div>
        {thread.diff_hunk && (
          <div className="mx-2.5 my-1.5 rounded-[6px] border border-tyba-border bg-tyba-sunken py-0.5 font-mono text-[12px] leading-[20px]">
            {hunkTail(thread.diff_hunk).map((l, i) => (
              <div
                key={i}
                className={`truncate whitespace-pre px-2.5 ${
                  l.kind === "add"
                    ? "bg-tyba-diff-add-bg text-tyba-diff-add-text"
                    : l.kind === "del"
                      ? "bg-tyba-diff-del-bg text-tyba-diff-del-text"
                      : "text-tyba-text-muted"
                }`}
              >
                {l.text}
              </div>
            ))}
          </div>
        )}
        <div className="min-w-0 px-2.5 pb-2.5 pt-0.5">
          <MarkdownView text={first.body} dark={dark} compact />
        </div>
        {rest.map(reply)}
      </div>
    );
  };

  const eventDot = (event: TimelineEvent) => {
    const base = "absolute -left-[26px] top-0.5 flex size-5 items-center justify-center rounded-full bg-tyba-sunken";
    if (event.kind === "commits") return <span className={`${base} text-tyba-text-muted`}><GitCommit size={12} /></span>;
    if (event.kind === "checks") {
      const tone = checksSummary(event.checks).tone;
      return (
        <span className={`${base} ${TONE_TEXT[tone]}`}>
          {tone === "red" ? <X size={11} weight="bold" /> : tone === "green" ? <Check size={11} weight="bold" /> : <Clock size={11} />}
        </span>
      );
    }
    if (event.kind === "review") {
      return (
        <span className={`${base} ${REVIEW_VERB[event.review.state].tone}`}>
          <ChatTeardrop size={12} />
        </span>
      );
    }
    if (event.kind === "opened") return <span className={`${base} text-tyba-green`}><GitPullRequest size={12} /></span>;
    return <span className={`${base} text-tyba-text-muted`}><ChatTeardrop size={12} /></span>;
  };

  const renderEvent = (event: TimelineEvent) => {
    const id = timelineEventId(event);
    let body: React.ReactNode = null;
    switch (event.kind) {
      case "opened":
        body = (
          <div className="flex flex-col gap-1.5">
            <div className="pt-0.5 text-[12px] text-tyba-text-muted">
              <span className="text-tyba-text">{event.author}</span> {t("prOpenedBy")}{" "}
              <span className="text-tyba-text-faint">· {relative(event.at, now)}</span>
            </div>
            {event.body.trim() && (
              <div className="relative rounded-[8px] border border-tyba-border bg-tyba-raised/60 px-3 py-2.5">
                <div className={bodyOpen ? "" : "max-h-[220px] overflow-hidden"}>
                  <MarkdownView text={event.body} dark={dark} compact />
                </div>
                {!bodyOpen && (
                  <div className="pointer-events-none absolute inset-x-0 bottom-9 h-10 bg-gradient-to-b from-transparent to-[var(--tyba-raised)]" />
                )}
                <button
                  type="button"
                  onClick={() => setBodyOpen((v) => !v)}
                  className="mt-1.5 text-[11px] text-tyba-text-muted hover:text-tyba-text"
                >
                  {bodyOpen ? t("prBodyLess") : t("prBodyMore")}
                </button>
              </div>
            )}
          </div>
        );
        break;
      case "commits": {
        // Autor real do commit sempre; a sessão do agente é complemento e só
        // aparece com sessão ligada ao PR.
        const author = event.commits[0].author;
        const agent = pr.session?.agent ?? null;
        body = (
          <div className="flex flex-col gap-1 pt-0.5 text-[12px]">
            <span className="text-tyba-text-muted">
              <span className="text-tyba-text">{author}</span>
              {agent && <span className="text-tyba-amber"> · {t("prAgentSession", { agent })}</span>}{" "}
              {t("prPushedCommits", { count: event.commits.length })}{" "}
              <span className="text-tyba-text-faint">· {relative(event.at, now)}</span>
            </span>
            {event.commits.map((c) => (
              <span key={c.sha} className="flex min-w-0 gap-2.5">
                <span className="shrink-0 font-mono text-tyba-blue">{c.sha.slice(0, 7)}</span>
                <span className="truncate">{c.subject}</span>
              </span>
            ))}
          </div>
        );
        break;
      }
      case "checks": {
        const workflow = event.checks.find((c) => c.workflow)?.workflow;
        body = (
          <div className="flex flex-col gap-1 pt-0.5 text-[12px]">
            <span className="text-tyba-text-muted">
              CI{" "}
              <span className="text-tyba-text-faint">
                · {workflow ? `${workflow} · ` : ""}
                {relative(event.at, now)}
              </span>
            </span>
            {event.checks.map((c, i) => (
              <span key={`${c.name}:${i}`} className="flex items-center gap-2.5">
                <CheckMark status={c.status} />
                <span className="min-w-[70px]">{c.name}</span>
                <span className="text-tyba-text-faint">{checkDuration(c) ?? ""}</span>
                {c.status === "failure" && c.url && (
                  <button
                    type="button"
                    onClick={() => void openExternalUrl(c.url as string)}
                    className="ml-auto rounded-[5px] border border-tyba-border-strong px-2 py-0.5 text-[11px] text-tyba-text-muted hover:text-tyba-text"
                  >
                    {t("prViewLog")}
                  </button>
                )}
              </span>
            ))}
          </div>
        );
        break;
      }
      case "review": {
        const verb = REVIEW_VERB[event.review.state];
        body = (
          <div className="flex flex-col overflow-hidden rounded-[8px] border border-tyba-border bg-tyba-raised/60">
            <div className="flex items-center gap-2 px-2.5 py-2 text-[12px]">
              <Avatar name={event.review.author} />
              <span>{event.review.author}</span>
              <span className={verb.tone}>{t(verb.key)}</span>
              <span className="text-tyba-text-faint">· {relative(event.at, now)}</span>
              <span className="flex-1" />
              {externalTag}
              {event.review.body.trim() &&
                forwardButton(id, () =>
                  forwardPrompt(pr, { author: event.review.author, verb: t(verb.key), body: event.review.body }),
                )}
            </div>
            {event.review.body.trim() && (
              <div className="min-w-0 px-2.5 pb-2.5 pl-[38px] text-tyba-text-muted">
                <MarkdownView text={event.review.body} dark={dark} compact />
              </div>
            )}
          </div>
        );
        break;
      }
      case "thread":
        body = threadCard(event.thread, id);
        break;
      case "comment":
        body = (
          <div className="flex flex-col overflow-hidden rounded-[8px] border border-tyba-border bg-tyba-raised/60">
            <div className="flex items-center gap-2 px-2.5 py-2 text-[12px]">
              <Avatar name={event.comment.author} />
              <span>{event.comment.author}</span>
              <span className="text-tyba-text-muted">{t("prReviewedCommented")}</span>
              <span className="text-tyba-text-faint">· {relative(event.at, now)}</span>
              <span className="flex-1" />
              {event.comment.author !== pr.author && externalTag}
              {forwardButton(id, () =>
                forwardPrompt(pr, { author: event.comment.author, verb: t("prReviewedCommented"), body: event.comment.body }),
              )}
            </div>
            <div className="min-w-0 px-2.5 pb-2.5 pl-[38px] text-tyba-text-muted">
              <MarkdownView text={event.comment.body} dark={dark} compact />
            </div>
          </div>
        );
        break;
    }
    return (
      <div key={id} className="relative">
        {eventDot(event)}
        {body}
      </div>
    );
  };

  const stateChip =
    pr.state === "merged"
      ? { key: "prStateMerged", cls: "bg-tyba-violet/[.12] text-tyba-violet", icon: <GitMerge size={11} /> }
      : pr.state === "closed"
        ? { key: "prStateClosed", cls: "bg-tyba-red/[.12] text-tyba-red", icon: <GitPullRequest size={11} /> }
        : pr.state === "draft"
          ? { key: "prStateDraft", cls: "bg-tyba-text/[.06] text-tyba-text-muted", icon: <GitPullRequest size={11} /> }
          : { key: "prStateOpen", cls: "bg-tyba-green/[.12] text-tyba-green", icon: <GitPullRequest size={11} /> };

  const approvedCount = reviewers.filter((r) => r.state === "approved").length;
  const tabs: { key: Section; label: string; count?: number; tone?: string }[] = [
    { key: "conversation", label: "prTabConversation" },
    { key: "commits", label: "prTabCommits", count: pr.commits.length },
    { key: "files", label: "prTabFiles", count: pr.changed_files },
    { key: "checks", label: "prTabChecks", count: checks.total, tone: TONE_TEXT[checks.tone] },
  ];

  return (
    <div className="absolute inset-0 flex min-w-0 flex-col bg-tyba-sunken">
      <div className="flex flex-col gap-2.5 px-[22px] pb-2.5 pt-3.5">
        <div className="flex items-baseline gap-2.5">
          <h1 className="min-w-0 truncate text-[17px] font-medium text-tyba-text" title={pr.title}>
            {pr.title}
          </h1>
          <span className="shrink-0 text-[17px] text-tyba-text-faint">#{pr.number}</span>
          <span className="flex-1" />
          <button
            type="button"
            onClick={() => void openExternalUrl(pr.url)}
            className="flex shrink-0 items-center gap-1.5 text-[12px] text-tyba-text-muted hover:text-tyba-text"
          >
            {t("prOpenInGithub")}
            <ArrowSquareOut size={12} />
          </button>
        </div>
        <div className="flex min-w-0 flex-wrap items-center gap-2.5 text-[12px] text-tyba-text-muted">
          <span className={`flex items-center gap-1.5 rounded-[4px] px-1.5 py-px ${stateChip.cls}`}>
            {stateChip.icon}
            {t(stateChip.key)}
          </span>
          <span className="font-mono">
            <span className="text-tyba-blue">{pr.head}</span> → <span className="text-tyba-blue">{pr.base}</span>
          </span>
          <span className="text-tyba-text-ghost">·</span>
          <span className="flex items-center gap-1.5">
            <Avatar name={pr.author} />
            <span className="text-tyba-text">{pr.author}</span>
            {t("prOpenedAgo", { ago: relative(pr.created_at, now) })}
          </span>
          {pr.session?.agent && (
            <span className="rounded-[4px] bg-tyba-amber/[.12] px-1.5 py-px text-tyba-amber">
              {t("prOpenedBySession", { agent: pr.session.agent })}
            </span>
          )}
          <span className="flex-1" />
          <span>
            <span className="text-tyba-green">+{pr.additions}</span>{" "}
            <span className="text-tyba-red">−{pr.deletions}</span> ·{" "}
            {t("diffFilesSummary", { count: pr.changed_files })} ·{" "}
            {t("prCommitsCount", { count: pr.commits.length })}
          </span>
        </div>
        {pr.rate_limit && (
          <p role="status" className="text-[11px] text-tyba-amber">
            {t("prRateLimited", {
              remaining: pr.rate_limit.remaining,
              limit: pr.rate_limit.limit,
              time: rateResetTime(pr.rate_limit),
            })}
          </p>
        )}
        {/* A atualização falhou mas há dado na tela: avisa sem esconder o PR. */}
        {error && (
          <p role="status" className="text-[11px] text-tyba-amber" title={error}>
            {t("gitRefreshFailed")} ·{" "}
            <button type="button" onClick={() => load(true)} className="hover:underline">
              {t("gitRefreshRetry")}
            </button>
          </p>
        )}
      </div>

      <div role="tablist" className="flex shrink-0 gap-1 tyba-divide-b px-[22px] text-[12px]">
        {tabs.map((tab) => (
          <button
            key={tab.key}
            role="tab"
            aria-selected={section === tab.key}
            onClick={() => setSection(tab.key)}
            className={`-mb-px flex items-center gap-1.5 border-b-2 px-2.5 py-2 ${
              section === tab.key
                ? "border-tyba-blue text-tyba-text"
                : "border-transparent text-tyba-text-muted hover:text-tyba-text"
            }`}
          >
            {t(tab.label)}
            {tab.count !== undefined && (
              <span className={tab.tone ?? "text-tyba-text-faint"}>{tab.count}</span>
            )}
          </button>
        ))}
      </div>

      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-[3] flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto px-[22px] py-4">
          {section === "conversation" && (
            <div className="relative flex flex-col gap-3.5 pl-[30px]">
              <div className="absolute bottom-0 left-[10px] top-2 w-px bg-tyba-border" />
              {timeline.map(renderEvent)}
            </div>
          )}
          {section === "commits" && (
            <ul className="flex flex-col">
              {pr.commits.map((c) => (
                <li key={c.sha} className="flex items-center gap-3 tyba-divide-b py-2 text-[12px]">
                  <span className="shrink-0 font-mono text-tyba-blue">{c.sha.slice(0, 7)}</span>
                  <span className="min-w-0 flex-1 truncate">{c.subject}</span>
                  <span className="shrink-0 text-tyba-text-faint">
                    {c.author} · {relative(c.authored_at, now)}
                  </span>
                </li>
              ))}
            </ul>
          )}
          {section === "files" && (
            <ul className="flex flex-col">
              {pr.files.map((f) => (
                <li key={f.path} className="flex items-center gap-3 tyba-divide-b py-2 font-mono text-[12px]">
                  <span className="min-w-0 flex-1 truncate">{f.path}</span>
                  <span className="text-tyba-green">+{f.additions}</span>
                  <span className="text-tyba-red">−{f.deletions}</span>
                </li>
              ))}
            </ul>
          )}
          {section === "checks" && (
            <ul className="flex flex-col">
              {pr.checks.map((c, i) => (
                <li key={`${c.name}:${i}`} className="flex items-center gap-3 tyba-divide-b py-2 text-[12px]">
                  <CheckMark status={c.status} />
                  <span className="min-w-0 flex-1 truncate">
                    {c.name}
                    {c.workflow && <span className="text-tyba-text-faint"> · {c.workflow}</span>}
                  </span>
                  <span className="text-tyba-text-faint">{checkDuration(c) ?? ""}</span>
                  {c.url && (
                    <button
                      type="button"
                      onClick={() => void openExternalUrl(c.url as string)}
                      className="rounded-[5px] border border-tyba-border-strong px-2 py-0.5 text-[11px] text-tyba-text-muted hover:text-tyba-text"
                    >
                      {t("prViewLog")}
                    </button>
                  )}
                </li>
              ))}
              {pr.checks.length === 0 && <li className="py-3 text-[12px] text-tyba-text-faint">{t("prChecksNone")}</li>}
            </ul>
          )}
        </div>
        {pr.state !== "merged" && pr.state !== "closed" && (
          <form
            aria-label={t("prReviewForm")}
            className="flex shrink-0 flex-col gap-2 tyba-divide-t bg-tyba-raised px-[22px] pb-2.5 pt-3"
            onSubmit={(e) => {
              e.preventDefault();
              void submitReview();
            }}
          >
            <label htmlFor="pr-review-body" className="text-[11px] text-tyba-text-faint">
              {t("prReviewBodyLabel")}
            </label>
            <textarea
              id="pr-review-body"
              rows={2}
              spellCheck
              autoCorrect="on"
              autoCapitalize="sentences"
              value={reviewBody}
              onChange={(e) => setReviewBody(e.target.value)}
              placeholder={t("prReviewBodyPlaceholder")}
              className="resize-none rounded-[6px] border border-tyba-border-field bg-tyba-sunken px-2.5 py-2 text-[12px] text-tyba-text outline-none placeholder:text-tyba-text-faint focus:border-tyba-blue/50"
            />
            <div className="flex flex-wrap items-center gap-2 text-[12px]">
              <fieldset className="flex flex-wrap gap-1.5">
                <legend className="sr-only">{t("prReviewDecision")}</legend>
                {(["comment", "approve", "request_changes"] as const).map((ev) => {
                  const on = reviewEvent === ev;
                  const tone =
                    ev === "approve"
                      ? on
                        ? "border-tyba-green/50 bg-tyba-green/[.08] text-tyba-green"
                        : "border-tyba-border-strong text-tyba-green"
                      : ev === "request_changes"
                        ? on
                          ? "border-tyba-red/50 bg-tyba-red/[.08] text-tyba-red"
                          : "border-tyba-border-strong text-tyba-red"
                        : on
                          ? "border-tyba-text/30 bg-tyba-text/[.05] text-tyba-text"
                          : "border-tyba-border-strong text-tyba-text-muted";
                  return (
                    <label
                      key={ev}
                      className={`flex cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-[6px] border px-2.5 py-1 ${tone}`}
                    >
                      <input
                        type="radio"
                        name="pr-review-event"
                        checked={on}
                        onChange={() => setReviewEvent(ev)}
                        className="m-0"
                      />
                      {t(`prReviewEvent_${ev}`)}
                    </label>
                  );
                })}
              </fieldset>
              <span className="flex-1" />
              <button
                type="submit"
                disabled={
                  acting !== null || (reviewEvent !== "approve" && !reviewBody.trim())
                }
                className="shrink-0 whitespace-nowrap rounded-[6px] bg-tyba-text px-3.5 py-1.5 font-medium text-tyba-text-inverse disabled:opacity-40"
              >
                {acting === "review" ? t("prReviewSending") : t("prReviewSend")}
              </button>
            </div>
            {actionNote?.from === "review" && (
              <p role="status" className={`text-[11px] ${actionNote.error ? "text-tyba-red" : "text-tyba-green"}`}>
                {actionNote.text}
              </p>
            )}
            <p className="text-right text-[11px] text-tyba-text-faint">{t("prReviewNote")}</p>
          </form>
        )}
        </div>

        <aside className="flex min-w-0 flex-[2] flex-col tyba-divide-l bg-[var(--tyba-panel-bg)]">
          <section className="flex flex-col gap-2 tyba-divide-b px-[18px] py-3.5 text-[12px]">
            <div className="flex justify-between text-[11px] text-tyba-text-faint">
              <span className="tyba-label">{t("prReviewers")}</span>
              <span>{t("prReviewersApproved", { count: approvedCount, total: reviewers.length })}</span>
            </div>
            {reviewers.length === 0 && <span className="text-tyba-text-faint">{t("prNoReviewers")}</span>}
            {reviewers.map((r) => (
              <div key={r.author} className="flex flex-col gap-1">
                <div className="flex items-center gap-2">
                  <Avatar name={r.author} />
                  <span className="min-w-0 flex-1 truncate">{r.author}</span>
                  <span
                    className={
                      r.state === "approved"
                        ? "text-tyba-green"
                        : r.state === "changes_requested"
                          ? "text-tyba-red"
                          : "text-tyba-text-faint"
                    }
                  >
                    {t(`prReviewer_${r.state}`)}
                  </span>
                </div>
                {r.staleSince && (
                  <span className="flex items-center gap-1.5 pl-7 text-[11px] text-tyba-amber">
                    <Clock size={11} />
                    {t("prReviewStale", { sha: r.staleSince })}
                  </span>
                )}
              </div>
            ))}
          </section>
          <section className="flex flex-col gap-2 tyba-divide-b px-[18px] py-3.5 text-[12px]">
            <div className="flex justify-between text-[11px] text-tyba-text-faint">
              <span className="tyba-label">{t("prTabChecks")}</span>
              {checks.total > 0 && (
                <span className={TONE_TEXT[checks.tone]}>
                  {t("prChecksPassed", { count: checks.passed, total: checks.total })}
                </span>
              )}
            </div>
            {pr.checks.length === 0 && <span className="text-tyba-text-faint">{t("prChecksNone")}</span>}
            {pr.checks.map((c, i) => (
              <div key={`${c.name}:${i}`} className="flex items-center gap-2">
                <CheckMark status={c.status} />
                <span className="min-w-0 flex-1 truncate">{c.name}</span>
                <span className="text-[11px] text-tyba-text-faint">{checkDuration(c) ?? ""}</span>
              </div>
            ))}
          </section>
          {/* Depois do merge o formulário e os controles somem; o resultado fica. */}
          {pr.state === "merged" && (
            <section className="flex flex-col gap-2 px-[18px] py-3.5 text-[12px] text-tyba-text-muted">
              <span className="tyba-label text-[11px] text-tyba-text-faint">{t("prMerge")}</span>
              <span role="status" className="flex items-center gap-2 text-tyba-violet">
                <GitMerge size={12} />
                {t("prMerged")}
              </span>
            </section>
          )}
          {pr.state !== "merged" && pr.state !== "closed" && (
            <section className="flex flex-col gap-2 px-[18px] py-3.5 text-[12px] text-tyba-text-muted">
              <span className="tyba-label text-[11px] text-tyba-text-faint">{t("prMerge")}</span>
              <span className="flex items-center gap-2">
                {pr.merge_state === "behind" ? (
                  <span className="text-tyba-amber">●</span>
                ) : (
                  <span className="text-tyba-green">✓</span>
                )}
                {t(pr.merge_state === "behind" ? "prMergeBehind" : "prMergeUpToDate", { base: pr.base })}
              </span>
              <span className="flex items-center gap-2">
                {pr.mergeable === "conflicting" ? (
                  <span className="text-tyba-red">✗</span>
                ) : pr.mergeable === "unknown" ? (
                  <span className="text-tyba-text-faint">?</span>
                ) : (
                  <span className="text-tyba-green">✓</span>
                )}
                {t(
                  pr.mergeable === "conflicting"
                    ? "prMergeConflicts"
                    : pr.mergeable === "unknown"
                      ? "prMergeUnknown"
                      : "prMergeNoConflicts",
                )}
              </span>
              {reviewers
                .filter((r) => r.staleSince)
                .map((r) => (
                  <span key={r.author} className="flex items-center gap-2">
                    <span className="text-tyba-amber">●</span>
                    {t("prMergeAwaitingReview", { author: r.author })}
                  </span>
                ))}
              <label htmlFor="pr-merge-strategy" className="pt-1 text-[11px] text-tyba-text-faint">
                {t("prMergeStrategy")}
              </label>
              <select
                id="pr-merge-strategy"
                value={strategy}
                onChange={(e) => setStrategy(e.target.value as PrMergeStrategy)}
                className="rounded-[6px] border border-tyba-border-field bg-tyba-raised px-2.5 py-1.5 text-[12px] text-tyba-text outline-none"
              >
                <option value="squash">{t("prMergeSquash")}</option>
                <option value="merge">{t("prMergeCommit")}</option>
                <option value="rebase">{t("prMergeRebase")}</option>
              </select>
              <button
                type="button"
                disabled={!mergeReady || acting !== null}
                title={mergeReady ? undefined : t("prMergeBlocked")}
                onClick={() => void merge()}
                className="whitespace-nowrap rounded-[6px] border border-tyba-green/55 px-3 py-1.5 text-[12px] text-tyba-green hover:bg-tyba-green/[.08] disabled:border-tyba-border-strong disabled:text-tyba-text-faint disabled:hover:bg-transparent"
              >
                {acting === "merge" ? t("prMerging") : t("prMergeAction")}
              </button>
              {actionNote?.from === "merge" && (
                <p role="status" className={`text-[11px] ${actionNote.error ? "text-tyba-red" : "text-tyba-green"}`}>
                  {actionNote.text}
                </p>
              )}
            </section>
          )}
          <div className="flex-1" />
          <p className="tyba-divide-t px-[18px] py-3 text-[11px] text-tyba-text-faint">{t("prExternalNote")}</p>
        </aside>
      </div>
    </div>
  );
}
