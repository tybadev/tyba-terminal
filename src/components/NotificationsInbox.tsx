import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { CaretDown, CaretRight, Tray, Warning, X } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Textarea } from "@/components/ui/textarea";
import {
  RISK_DOT,
  RISK_LABEL,
  approvalActions,
  attentionCount,
  dayBucket,
  decideApproval,
  filterEntries,
  groupBySession,
  historyEntries,
  unreadCount,
  type DayBucket,
  type NotificationFilter,
} from "@/lib/notifications";
import { openGitScreen } from "@/lib/gitScreen";
import {
  notificationsList,
  notificationsMarkRead,
  onNotificationsChanged,
  resolveApproval,
  type ApprovalDecision,
  type ApprovalRequest,
  type NotificationEntry,
  type Session,
  type SessionId,
} from "../lib/ipc";

const FILTERS: { key: NotificationFilter; label: string }[] = [
  { key: "all", label: "notifFilterAll" },
  { key: "requests", label: "notifFilterRequests" },
  { key: "errors", label: "notifFilterErrors" },
  { key: "done", label: "notifFilterDone" },
];

const BUCKETS: DayBucket[] = ["today", "yesterday", "earlier"];
const BUCKET_LABEL: Record<DayBucket, string> = {
  today: "notifToday",
  yesterday: "notifYesterday",
  earlier: "notifEarlier",
};

const SMALL_BTN =
  "h-6 shrink-0 rounded-[5px] border border-tyba-border-strong px-2 text-[11px] text-tyba-text-muted hover:text-tyba-text";

function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/**
 * Notificações (mockup Notificacoes): painel de 380px à direita. Em cima, o
 * que pede você agora (pedidos ao vivo, acionáveis); embaixo, o histórico
 * que o core guarda (terminou, falhou, limite), por dia e por sessão.
 */
export function NotificationsInbox({
  sessions,
  approvals,
  open,
  onOpenChange,
  onGoToSession,
}: {
  sessions: Session[];
  approvals: ApprovalRequest[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onGoToSession: (sessionId: SessionId) => void;
}) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState<number | null>(null);
  const [feedbackFor, setFeedbackFor] = useState<number | null>(null);
  const [feedback, setFeedback] = useState("");
  const [entries, setEntries] = useState<NotificationEntry[]>([]);
  const [filter, setFilter] = useState<NotificationFilter>("all");
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());

  const load = useCallback(() => {
    notificationsList()
      .then(setEntries)
      .catch(() => {});
  }, []);

  useEffect(() => {
    load();
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void onNotificationsChanged(load).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [load]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onOpenChange(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onOpenChange]);

  // Mesmo caminho de decisão do toast (lib/notifications#decideApproval):
  // o gate do vermelho e o resolveApproval chamado são os mesmos.
  const decide = (request: ApprovalRequest, decision: ApprovalDecision, withFeedback?: string) => {
    const effect = decideApproval({ request, decision, confirmingId: confirming, feedback: withFeedback });
    if (effect.type === "armRedConfirm") {
      setConfirming(effect.requestId);
      return;
    }
    setConfirming(null);
    setFeedbackFor(null);
    setFeedback("");
    resolveApproval(effect.requestId, effect.decision, effect.feedback).catch(() => {});
  };

  const sessionTitle = (id: SessionId) => sessions.find((s) => s.id === id)?.title ?? id.slice(0, 8);
  const sessionAgent = (id: SessionId): string | null => {
    const kind = sessions.find((s) => s.id === id)?.kind;
    if (kind?.type !== "agent") return null;
    if (kind.runner === "claude_code") return "claude";
    if (kind.runner === "codex") return "codex";
    return kind.runner.custom;
  };

  const history = useMemo(() => historyEntries(entries), [entries]);
  const shown = useMemo(() => filterEntries(history, filter), [history, filter]);
  const badge = attentionCount(approvals.length, entries);
  const unread = unreadCount(history) + approvals.length;
  const showRequests = filter === "all" || filter === "requests";
  const now = Date.now();

  const counts: Record<NotificationFilter, number> = {
    all: 0,
    requests: approvals.length,
    errors: history.filter((e) => e.category === "error" && !e.read).length,
    done: 0,
  };

  const toggleGroup = (key: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  const groupHeader = (key: string, title: string, agent: string | null, count: number) => {
    const isOpen = !collapsed.has(key);
    return (
      <div className="flex items-center justify-between px-1 pb-1 pt-2">
        <button
          type="button"
          aria-expanded={isOpen}
          onClick={() => toggleGroup(key)}
          className="flex min-w-0 items-center gap-1.5 text-[12px] text-tyba-text hover:text-tyba-text"
        >
          {isOpen ? <CaretDown size={10} /> : <CaretRight size={10} />}
          <span className="truncate">{title}</span>
          {agent && (
            <span className="shrink-0 rounded-[4px] bg-tyba-amber/[.12] px-1.5 text-[10px] text-tyba-amber">
              {agent}
            </span>
          )}
        </button>
        <span className="text-[11px] text-tyba-text-faint">{count}</span>
      </div>
    );
  };

  const approvalCard = (request: ApprovalRequest) => (
    <div key={request.id} className="motion-safe:animate-tyba-pop-in flex gap-2 rounded-[8px] bg-tyba-raised p-2.5 shadow-[var(--tyba-raised-ring)]">
      <div className="min-w-0 flex-1">
        <div className="text-[12.5px] text-tyba-text">
          {request.risk === "red" ? t("notifRedTitle") : t("notifAskTitle")}
        </div>
        <div className="mt-0.5 flex items-center gap-1.5 text-[11px] text-tyba-text-faint">
          {request.risk === "red" ? (
            <span className="flex items-center gap-1 text-tyba-red">
              <Warning size={10} weight="bold" />
              {t("redAction")}
            </span>
          ) : (
            <span className="flex items-center gap-1">
              <span
                role="img"
                aria-label={t(RISK_LABEL[request.risk])}
                className={`size-1.5 rounded-full ${RISK_DOT[request.risk]}`}
              />
              {t("notifAsks")}
            </span>
          )}
          · {sessionTitle(request.session_id)} · {clock(request.requested_at_ms)}
        </div>
        <code className="mt-1.5 block truncate rounded-[5px] bg-tyba-sunken px-2 py-1 font-mono text-[11.5px] text-tyba-text" title={request.command}>
          {request.command}
        </code>
        {request.context && <div className="mt-1 text-[11px] text-tyba-text-muted">{request.context}</div>}
        <div className="mt-2 flex flex-wrap gap-1.5">
          {approvalActions(request.risk).map((action) => {
            const isConfirmApprove = action.id === "approve" && request.risk === "red" && confirming === request.id;
            const label = isConfirmApprove ? t("confirmApprove") : t(action.labelKey);
            const onClick = () => {
              if (action.id === "denyWithReason") {
                setFeedbackFor(request.id);
                return;
              }
              const decision: ApprovalDecision =
                action.id === "alwaysAllow" ? "approved_always" : action.id === "deny" ? "denied" : "approved";
              decide(request, decision);
            };
            const button = (
              <Button
                key={action.id}
                size="sm"
                variant={action.id === "approve" ? "default" : "outline"}
                onClick={onClick}
                className={`h-6 gap-1.5 rounded-[5px] px-2.5 text-[11px] ${action.id === "approve" ? "" : "text-tyba-text-muted"} ${
                  isConfirmApprove ? "bg-tyba-red text-white hover:bg-tyba-red/90" : ""
                }`}
              >
                {label}
              </Button>
            );
            return action.id === "alwaysAllow" ? (
              <Tooltip key={action.id}>
                <TooltipTrigger asChild>{button}</TooltipTrigger>
                <TooltipContent side="bottom">{t("alwaysAllowHint")}</TooltipContent>
              </Tooltip>
            ) : (
              button
            );
          })}
          <button type="button" className={SMALL_BTN} onClick={() => onGoToSession(request.session_id)}>
            {t("notifOpenSession")}
          </button>
        </div>
        {feedbackFor === request.id && (
          <div className="motion-safe:animate-tyba-pop-in mt-2 space-y-1.5">
            <Textarea
              autoFocus
              rows={2}
              value={feedback}
              onChange={(e) => setFeedback(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  decide(request, "denied", feedback);
                }
                if (e.key === "Escape") {
                  e.preventDefault();
                  e.stopPropagation();
                  setFeedbackFor(null);
                  setFeedback("");
                }
              }}
              placeholder={t("denyFeedbackPlaceholder")}
              className="min-h-0 resize-none text-[11px]"
            />
            <div className="flex gap-2">
              <Button size="sm" onClick={() => decide(request, "denied", feedback)} className="h-6 rounded-[4px] px-2.5 text-[11px]">
                {t("denyAndTell")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  setFeedbackFor(null);
                  setFeedback("");
                }}
                className="h-6 rounded-[4px] px-2.5 text-[11px] text-tyba-text-muted"
              >
                {t("cancel")}
              </Button>
            </div>
          </div>
        )}
      </div>
      <span role="img" aria-label={t("notifUnread")} className="mt-1.5 size-1.5 shrink-0 rounded-full bg-tyba-blue" />
    </div>
  );

  const entryTitle = (e: NotificationEntry): string => {
    const who = e.agent ?? e.session_title;
    switch (e.kind) {
      case "approval":
        return e.decision === "denied"
          ? t("notifApprovalDenied", { who })
          : e.decision
            ? t("notifApprovalApproved", { who })
            : t("notifApprovalExpired", { who });
      case "agent_finished":
        return t("notifFinished", { who, session: e.session_title });
      case "agent_failed":
        return t("notifFailed", { who });
      case "usage_limit":
        return e.usage && e.usage.pct >= 100
          ? t("notifLimitHit", { who, window: t(`notifWindow_${e.usage.window}`) })
          : t("notifLimitWarn", { pct: e.usage?.pct ?? 0, window: t(`notifWindow_${e.usage?.window ?? "five_hour"}`) });
    }
  };

  const entryRow = (e: NotificationEntry) => {
    const resets = e.usage?.resets_at ? clock(e.usage.resets_at * 1000) : null;
    return (
      <div
        key={e.id}
        role="button"
        tabIndex={0}
        onClick={() => !e.read && void notificationsMarkRead([e.id]).catch(() => {})}
        onKeyDown={(ev) => {
          if (ev.key === "Enter" && !e.read) void notificationsMarkRead([e.id]).catch(() => {});
        }}
        className="flex cursor-default gap-2 rounded-[8px] px-2.5 py-2 outline-none hover:bg-tyba-raised/60 focus-visible:bg-tyba-raised/60"
      >
        <div className="min-w-0 flex-1">
          <div className={`text-[12.5px] ${e.category === "error" ? "text-tyba-red" : "text-tyba-text"}`}>{entryTitle(e)}</div>
          <div className="mt-0.5 truncate text-[11px] text-tyba-text-faint">
            {e.session_title} · {clock(e.at_ms)}
            {resets && ` · ${t("notifResetsAt", { time: resets })}`}
            {e.detail && ` · ${e.detail}`}
          </div>
          {e.command && (
            <code className="mt-1 block truncate font-mono text-[11px] text-tyba-text-muted" title={e.command}>
              {e.command}
            </code>
          )}
          {(e.kind === "agent_finished" || e.kind === "agent_failed" || e.kind === "usage_limit") && (
            <div className="mt-1.5 flex gap-1.5">
              {e.kind === "agent_finished" && (
                <button
                  type="button"
                  className={SMALL_BTN}
                  onClick={(ev) => {
                    ev.stopPropagation();
                    void openGitScreen(e.session_id, "review").catch(() => {});
                    onOpenChange(false);
                  }}
                >
                  {t("notifViewDiff")}
                </button>
              )}
              <button
                type="button"
                className={SMALL_BTN}
                onClick={(ev) => {
                  ev.stopPropagation();
                  onGoToSession(e.session_id);
                  onOpenChange(false);
                }}
              >
                {t("notifOpenSession")}
              </button>
            </div>
          )}
        </div>
        {!e.read && (
          <span role="img" aria-label={t("notifUnread")} className="mt-1.5 size-1.5 shrink-0 rounded-full bg-tyba-blue" />
        )}
      </div>
    );
  };

  const requestGroups = groupBySession(approvals, (a) => ({
    title: sessionTitle(a.session_id),
    agent: sessionAgent(a.session_id),
  }));

  return (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            aria-label={badge > 0 ? t("notifAria", { count: badge }) : t("notifications")}
            aria-pressed={open}
            onClick={() => {
              setConfirming(null);
              onOpenChange(!open);
            }}
            className={`relative h-7 w-[30px] rounded-[6px] ${
              open ? "bg-[var(--tyba-lit-bg)] text-tyba-blue shadow-[var(--tyba-lit-glow)]" : "text-tyba-text-faint hover:text-tyba-text"
            }`}
          >
            <Tray size={16} weight={badge > 0 ? "fill" : "regular"} />
            {badge > 0 && (
              <span className="absolute right-[3px] top-[3px] flex h-3 min-w-3 items-center justify-center rounded-[6px] bg-tyba-red-fill px-[3px] text-[9px] leading-none text-white">
                {badge > 9 ? "9+" : badge}
              </span>
            )}
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">{t("notifications")}</TooltipContent>
      </Tooltip>

      {open && (
        <aside
          aria-label={t("notifications")}
          className="motion-safe:animate-tyba-panel-in fixed bottom-[26px] right-0 top-[38px] z-50 flex w-[380px] flex-col tyba-divide-l bg-[var(--tyba-panel-bg)] tyba-glass shadow-[-24px_0_48px_rgba(0,0,0,0.35)]"
        >
          <div className="flex shrink-0 items-center gap-2.5 tyba-divide-b px-4 py-3">
            <span className="text-[13px] text-tyba-text">{t("notifications")}</span>
            {unread > 0 && <span className="text-[11px] text-tyba-text-faint">{t("notifUnreadCount", { count: unread })}</span>}
            <span className="flex-1" />
            <button
              type="button"
              aria-label={t("sidePanelClose")}
              onClick={() => onOpenChange(false)}
              className="rounded-[4px] text-tyba-text-faint hover:text-tyba-text"
            >
              <X size={13} />
            </button>
          </div>

          <div className="flex shrink-0 items-center gap-2 tyba-divide-b px-3 py-2">
            <div role="tablist" aria-label={t("notifFilter")} className="flex gap-0.5 rounded-[7px] bg-tyba-sunken p-0.5 shadow-[inset_0_0_0_1px_var(--tyba-border)]">
              {FILTERS.map((f) => (
                <button
                  key={f.key}
                  type="button"
                  role="tab"
                  aria-selected={filter === f.key}
                  onClick={() => setFilter(f.key)}
                  className={`flex items-center gap-1 whitespace-nowrap rounded-[5px] px-2 py-0.5 text-[11.5px] ${
                    filter === f.key ? "bg-tyba-raised text-tyba-text shadow-[var(--tyba-raised-ring)]" : "text-tyba-text-muted hover:text-tyba-text"
                  }`}
                >
                  {t(f.label)}
                  {counts[f.key] > 0 && (
                    <span className={f.key === "errors" ? "text-tyba-red" : "text-tyba-amber"}>{counts[f.key]}</span>
                  )}
                </button>
              ))}
            </div>
            <span className="flex-1" />
            <button
              type="button"
              disabled={unreadCount(history) === 0}
              onClick={() => void notificationsMarkRead(null).catch(() => {})}
              className="whitespace-nowrap text-[11px] text-tyba-text-muted hover:text-tyba-text disabled:opacity-40"
            >
              {t("notifMarkAllRead")}
            </button>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-4">
            {showRequests && approvals.length > 0 && (
              <>
                <div className="flex items-center justify-between px-1 pb-1 pt-3 text-[11px] text-tyba-amber">
                  <span className="tyba-label !text-tyba-amber">{t("notifNeedsYou")}</span>
                  <span>{approvals.length}</span>
                </div>
                {requestGroups.map((g) => (
                  <section key={`req:${g.session}`} aria-label={g.title} className="flex flex-col gap-1.5">
                    {groupHeader(`req:${g.session}`, g.title, g.agent, g.items.length)}
                    {!collapsed.has(`req:${g.session}`) && g.items.map(approvalCard)}
                  </section>
                ))}
              </>
            )}

            {BUCKETS.map((bucket) => {
              const inBucket = shown.filter((e) => dayBucket(e.at_ms, now) === bucket);
              if (inBucket.length === 0) return null;
              const groups = groupBySession(inBucket, (e) => ({ title: e.session_title, agent: e.agent }));
              return (
                <div key={bucket}>
                  <div className="flex items-center justify-between px-1 pb-1 pt-4 text-[11px] text-tyba-text-faint">
                    <span className="tyba-label">{t(BUCKET_LABEL[bucket])}</span>
                    <span>{inBucket.length}</span>
                  </div>
                  {groups.map((g) => {
                    const key = `${bucket}:${g.session}`;
                    return (
                      <section key={key} aria-label={g.title}>
                        {groupHeader(key, g.title, g.agent, g.items.length)}
                        {!collapsed.has(key) && g.items.map(entryRow)}
                      </section>
                    );
                  })}
                </div>
              );
            })}

            {approvals.length === 0 && shown.length === 0 && (
              <p className="px-2 py-8 text-center text-[12px] text-tyba-text-faint">{t("notificationsEmpty")}</p>
            )}
          </div>
        </aside>
      )}
    </>
  );
}
