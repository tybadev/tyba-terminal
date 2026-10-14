import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Bell, CaretDown, CaretRight, Check, Diamond, Info, Warning, X, XCircle } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Textarea } from "@/components/ui/textarea";
import {
  RISK_LABEL,
  approvalActions,
  attentionCount,
  dayBucket,
  decideApproval,
  filterEntries,
  groupBySession,
  historyEntries,
  isDoneGroup,
  isRequestKind,
  resetParts,
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
  type RiskLevel,
  type Session,
  type SessionId,
} from "../lib/ipc";
import { PANEL_HEADER } from "./SidePanelBar";
import "./notifications.css";

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

// Mockup Notificacoes: ação é texto (`.tb`), a principal tem contorno âmbar
// (`.tbp`) e a secundária é link (`.lnk`). Nenhum cartão: a linha é grade
// ícone · conteúdo · ponto de não lida (`.row`).
const TEXT_BTN =
  "inline-flex shrink-0 items-center gap-[5px] whitespace-nowrap py-0.5 text-[11px] text-tyba-text-muted outline-none hover:text-tyba-text focus-visible:ring-2 focus-visible:ring-tyba-cyan rounded-[3px]";
const PRIMARY_BTN =
  "notif-primary shrink-0 whitespace-nowrap rounded-[4px] px-2 py-0.5 text-[11px] outline-none focus-visible:ring-2 focus-visible:ring-tyba-cyan";
const LINK_BTN =
  "whitespace-nowrap text-[11px] underline underline-offset-[3px] outline-none hover:text-tyba-text focus-visible:ring-2 focus-visible:ring-tyba-cyan rounded-[3px]";
const ROW = "grid grid-cols-[16px_minmax(0,1fr)_6px] gap-x-2.5 pb-3 pt-2";
const META = "mt-0.5 text-[11px] leading-normal text-tyba-text-faint";
const CMD = "notif-cmd mt-1.5 line-clamp-3 w-fit max-w-full rounded-[4px] px-1.5 py-0.5 font-mono text-[12px]";
const ACTS = "mt-2 flex flex-wrap items-center gap-3.5";
const SECTION = "flex justify-between pb-1 text-[11px] text-tyba-text-faint";
const GROUP_HEAD =
  "sticky top-0 z-[1] flex min-w-0 items-center gap-2 bg-tyba-surface py-1.5 text-[11px] text-tyba-text-faint";

/** Ícone do pedido (losango) na cor do risco. */
function RiskGlyph({ risk, label }: { risk: RiskLevel | null; label: string }) {
  const color = risk === "red" ? "text-tyba-red-fill" : risk === "green" ? "text-tyba-green-fill" : "text-tyba-amber-fill";
  return <Diamond role="img" aria-label={label} size={14} weight="bold" className={`mt-0.5 ${color}`} />;
}

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
  const [feedbackFor, setFeedbackFor] = useState<number | null>(null);
  const [feedback, setFeedback] = useState("");
  const [entries, setEntries] = useState<NotificationEntry[]>([]);
  const [filter, setFilter] = useState<NotificationFilter>("all");
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const panelRef = useRef<HTMLElement>(null);

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
      if (e.key !== "Escape" || e.defaultPrevented) return;
      // Mesmo critério do DiffView: Esc só fecha com o foco no painel (ou em
      // lugar nenhum) — um Esc digitado no terminal não derruba o painel.
      const active = document.activeElement;
      if (active && active !== document.body && !panelRef.current?.contains(active)) return;
      onOpenChange(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onOpenChange]);

  // Mesmo caminho de decisão do toast (lib/notifications#decideApproval).
  // Vermelho não se aprova daqui (R4): o card só oferece "Abrir na sessão",
  // e se um approve vermelho chegasse, o gate pede confirmação e paramos.
  const decide = (request: ApprovalRequest, decision: ApprovalDecision, withFeedback?: string) => {
    const effect = decideApproval({ request, decision, confirmingId: null, feedback: withFeedback });
    if (effect.type === "armRedConfirm") return;
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
  // Só o histórico: os pedidos pendentes têm a contagem própria em "Precisa de você".
  const unread = unreadCount(history);
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

  const agentLabel = (agent: string | null) => agent && <span className="shrink-0">{agent}</span>;

  const groupHeader = (key: string, title: string, agent: string | null, count: number) => {
    const isOpen = !collapsed.has(key);
    return (
      <div className={GROUP_HEAD}>
        <button
          type="button"
          aria-expanded={isOpen}
          onClick={() => toggleGroup(key)}
          className="flex min-w-0 flex-1 items-center gap-1.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-tyba-cyan rounded-[3px]"
        >
          {isOpen ? <CaretDown size={10} weight="bold" /> : <CaretRight size={10} weight="bold" />}
          <span className="truncate text-tyba-text-muted">{title}</span>
          {agentLabel(agent)}
        </button>
        <span>{count}</span>
      </div>
    );
  };

  // Grupo de pedidos não recolhe: nenhum pedido pode sumir de todas as telas
  // (com o painel aberto, o toast os esconde — hiddenApprovalIds).
  const requestGroupHeader = (title: string, agent: string | null, count: number) => (
    <div className={GROUP_HEAD}>
      <span className="flex min-w-0 flex-1 items-center gap-1.5">
        <span className="truncate text-tyba-text-muted">{title}</span>
        {agentLabel(agent)}
      </span>
      <span>{count}</span>
    </div>
  );

  const goToSession = (sessionId: SessionId) => {
    onGoToSession(sessionId);
    onOpenChange(false);
  };

  const markRead = (e: NotificationEntry) => {
    if (!e.read) void notificationsMarkRead([e.id]).catch(() => {});
  };

  // "Abrir na sessão →": a única ação do vermelho (R4) e do amarelo que
  // espera no prompt nativo do Claude (decide-se no terminal, não aqui).
  const openInSessionBtn = (sessionId: SessionId) => (
    <button type="button" className={`${TEXT_BTN} !text-tyba-text`} onClick={() => goToSession(sessionId)}>
      {t("notifOpenInSession")}
      <ArrowRight size={11} weight="bold" />
    </button>
  );

  const approvalCard = (request: ApprovalRequest) => {
    // Mockup: aprovar (contorno âmbar), negar, "não, e dizer…" e por último
    // o "sempre permitir" como link. O conjunto continua vindo de approvalActions.
    const actions = approvalActions(request.risk);
    const ordered = [...actions.filter((a) => a.id !== "alwaysAllow"), ...actions.filter((a) => a.id === "alwaysAllow")];
    return (
      <div key={request.id} className={`motion-safe:animate-tyba-pop-in ${ROW}`}>
        <RiskGlyph risk={request.risk} label={t(RISK_LABEL[request.risk])} />
        <div className="min-w-0">
          <div className="text-[13px] font-bold leading-[1.45] text-tyba-text">
            {request.risk === "red" ? t("notifRedTitle") : t("notifAskTitle")}
          </div>
          <div className={META}>
            <span className="text-tyba-text-muted">{request.risk === "red" ? t("redAction") : t("notifAsks")}</span>
            {" · "}
            {sessionTitle(request.session_id)} · {clock(request.requested_at_ms)}
          </div>
          <code className={CMD} title={request.command}>
            {request.command}
          </code>
          {request.context && <div className="mt-1 text-[11px] text-tyba-text-muted">{request.context}</div>}
          <div className={ACTS}>
            {/* R4: vermelho não se aprova de fora da sessão — só abrir nela. */}
            {request.risk === "red" ? (
              openInSessionBtn(request.session_id)
            ) : (
              <>
                {ordered.map((action) => {
                  const onClick = () => {
                    if (action.id === "denyWithReason") {
                      setFeedbackFor(request.id);
                      return;
                    }
                    const decision: ApprovalDecision =
                      action.id === "alwaysAllow" ? "approved_always" : action.id === "deny" ? "denied" : "approved";
                    decide(request, decision);
                  };
                  const className =
                    action.id === "approve"
                      ? PRIMARY_BTN
                      : action.id === "alwaysAllow"
                        ? `${LINK_BTN} text-tyba-text-faint`
                        : TEXT_BTN;
                  const button = (
                    <button key={action.id} type="button" onClick={onClick} className={className}>
                      {t(action.labelKey)}
                    </button>
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
                <button type="button" className={TEXT_BTN} onClick={() => onGoToSession(request.session_id)}>
                  {t("notifOpenSession")}
                </button>
              </>
            )}
          </div>
          {feedbackFor === request.id && request.risk !== "red" && (
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
        <span />
      </div>
    );
  };

  const entryTitle = (e: NotificationEntry): string => {
    const who = e.agent ?? e.session_title;
    switch (e.kind) {
      case "approval":
        return e.decision === "denied"
          ? t("notifApprovalDenied", { who })
          : e.decision
            ? t("notifApprovalApproved", { who })
            : t("notifApprovalExpired", { who });
      case "native_approval":
        return e.decision === "denied"
          ? t("notifNativeDenied", { who })
          : e.decision
            ? t("notifNativeApproved", { who })
            : e.pending
              ? t("notifNativeTitle", { who })
              : t("notifNativeExpired", { who });
      case "agent_finished":
        return t("notifFinished", { who, session: e.session_title });
      case "agent_failed":
        return t("notifFailed", { who });
      case "usage_limit":
        return e.usage && e.usage.pct >= 100
          ? t("notifLimitHit", { who, window: t(`notifWindow_${e.usage.window}`) })
          : t("notifLimitWarn", { pct: Math.round(e.usage?.pct ?? 0), window: t(`notifWindow_${e.usage?.window ?? "five_hour"}`) });
    }
  };

  /** Ícone por item (mockup): losango = pedido, ⊗ = erro, △ = aviso, ✓ = concluído. */
  const entryGlyph = (e: NotificationEntry) => {
    if (isRequestKind(e) && e.pending) return <RiskGlyph risk={e.risk} label={t(RISK_LABEL[e.risk ?? "yellow"])} />;
    if (isRequestKind(e)) return <Diamond role="img" aria-label={t("notifGlyphRequest")} size={14} weight="bold" className="mt-0.5 text-tyba-text-faint" />;
    if (e.category === "error") return <XCircle role="img" aria-label={t("notifGlyphError")} size={14} weight="bold" className="mt-0.5 text-tyba-red-fill" />;
    if (e.category === "info") return <Warning role="img" aria-label={t("notifGlyphWarning")} size={14} weight="bold" className="mt-0.5 text-tyba-amber-fill" />;
    return <Check role="img" aria-label={t("notifGlyphDone")} size={14} weight="bold" className="mt-0.5 text-tyba-text-faint" />;
  };

  const entryRow = (e: NotificationEntry, doneGroup: boolean) => {
    const reset = e.usage?.resets_at ? resetParts(e.usage.resets_at, Date.now()) : null;
    const resets = reset && (reset.date ? t("notifResetsOn", reset) : t("notifResetsAt", reset));
    return (
      <div
        key={e.id}
        role="button"
        tabIndex={0}
        onClick={() => markRead(e)}
        onKeyDown={(ev) => {
          if (ev.key === "Enter") markRead(e);
        }}
        className={`${ROW} cursor-default rounded-[4px] outline-none focus-visible:ring-2 focus-visible:ring-tyba-cyan`}
      >
        {entryGlyph(e)}
        <div className="min-w-0">
          <div
            className={`text-[13px] leading-[1.45] ${e.read ? "" : "font-bold"} ${
              doneGroup ? "text-tyba-text-muted" : "text-tyba-text"
            }`}
          >
            {entryTitle(e)}
          </div>
          <div className={`${META} truncate`}>
            {e.session_title} · {clock(e.at_ms)}
            {resets && ` · ${resets}`}
            {e.detail && ` · ${e.detail}`}
          </div>
          {e.command && (
            <code className={CMD} title={e.command}>
              {e.command}
            </code>
          )}
          {/* Amarelo esperando no prompt nativo: decide-se no terminal, nunca daqui. */}
          {e.kind === "native_approval" && e.pending && <div className={ACTS}>{openInSessionBtn(e.session_id)}</div>}
          {(e.kind === "agent_finished" || e.kind === "agent_failed" || e.kind === "usage_limit") && (
            <div className={ACTS}>
              {e.kind === "agent_finished" && (
                <button
                  type="button"
                  className={TEXT_BTN}
                  onClick={(ev) => {
                    ev.stopPropagation();
                    markRead(e);
                    void openGitScreen(e.session_id, "review").catch(() => {});
                    onOpenChange(false);
                  }}
                >
                  {t("notifViewDiff")}
                </button>
              )}
              <button
                type="button"
                className={TEXT_BTN}
                onClick={(ev) => {
                  ev.stopPropagation();
                  markRead(e);
                  goToSession(e.session_id);
                }}
              >
                {t("notifOpenSession")}
              </button>
            </div>
          )}
        </div>
        {e.read ? (
          <span />
        ) : (
          <span role="img" aria-label={t("notifUnread")} className="mt-[7px] size-1.5 rounded-full bg-tyba-blue" />
        )}
      </div>
    );
  };

  const requestGroups = groupBySession(approvals, (a) => ({
    title: sessionTitle(a.session_id),
    agent: sessionAgent(a.session_id),
  }));
  const liveCount = approvals.length;
  const firstBucket = BUCKETS.find((b) => shown.some((e) => dayBucket(e.at_ms, now) === b));

  return (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            aria-label={badge > 0 ? t("notifAria", { count: badge }) : t("notifications")}
            aria-pressed={open}
            onClick={() => onOpenChange(!open)}
            className={`relative h-7 w-[30px] rounded-[6px] ${
              open ? "bg-[var(--tyba-lit-bg)] text-tyba-blue shadow-[var(--tyba-lit-glow)]" : "text-tyba-text-faint hover:text-tyba-text"
            }`}
          >
            <Bell size={16} weight={badge > 0 ? "fill" : "regular"} />
            {badge > 0 && (
              <span className="absolute right-[3px] top-[3px] flex h-3 min-w-3 items-center justify-center rounded-[6px] bg-tyba-red-fill px-[3px] text-[9px] leading-none text-tyba-text-inverse">
                {badge > 9 ? "9+" : badge}
              </span>
            )}
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">{t("notifications")}</TooltipContent>
      </Tooltip>

      {open && (
        <aside
          ref={panelRef}
          aria-label={t("notifications")}
          className="motion-safe:animate-tyba-panel-in fixed bottom-[26px] right-0 top-[38px] z-50 flex w-[380px] flex-col tyba-divide-l bg-[var(--tyba-panel-bg)] tyba-glass shadow-[-24px_0_48px_rgba(0,0,0,0.35)]"
        >
          <div className={PANEL_HEADER}>
            <Bell size={16} className="shrink-0 text-tyba-blue" />
            <span className="min-w-0 flex-1 truncate text-[13px] text-tyba-text">{t("notifications")}</span>
            {unread > 0 && <span className="text-[11px] text-tyba-text-faint">{t("notifUnreadCount", { count: unread })}</span>}
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  aria-label={t("notifHowItWorks")}
                  className="rounded-[4px] text-tyba-text-faint outline-none hover:text-tyba-text focus-visible:ring-2 focus-visible:ring-tyba-cyan"
                >
                  <Info size={14} />
                </button>
              </TooltipTrigger>
              <TooltipContent side="bottom" className="max-w-[280px] text-[11px] leading-normal">
                {t("notifHowItWorksBody")}
              </TooltipContent>
            </Tooltip>
            <button
              type="button"
              aria-label={t("sidePanelClose")}
              onClick={() => onOpenChange(false)}
              className="rounded-[4px] text-tyba-text-faint outline-none hover:text-tyba-text focus-visible:ring-2 focus-visible:ring-tyba-cyan"
            >
              <X size={13} />
            </button>
          </div>

          <div className="notif-filters flex shrink-0 flex-col gap-2 px-4 py-2.5">
            <div role="tablist" aria-label={t("notifFilter")} className="flex gap-1.5">
              {FILTERS.map((f) => (
                <button
                  key={f.key}
                  type="button"
                  role="tab"
                  aria-selected={filter === f.key}
                  onClick={() => setFilter(f.key)}
                  className="notif-chip flex items-center gap-1 whitespace-nowrap rounded-full px-2.5 py-[3px] text-[11px] outline-none focus-visible:ring-2 focus-visible:ring-tyba-cyan"
                >
                  {t(f.label)}
                  {counts[f.key] > 0 && (
                    <span className={f.key === "errors" ? "text-tyba-red" : "text-tyba-amber"}>{counts[f.key]}</span>
                  )}
                </button>
              ))}
            </div>
            <div className="flex justify-end">
              <button
                type="button"
                disabled={unread === 0}
                onClick={() => void notificationsMarkRead(null).catch(() => {})}
                className={`${LINK_BTN} text-tyba-text-muted disabled:opacity-40 disabled:no-underline`}
              >
                {t("notifMarkAllRead")}
              </button>
            </div>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-3">
            {/* "Precisa de você" aparece em qualquer filtro: com o painel aberto
                o toast esconde os pedidos, então nenhum pode sumir daqui. */}
            {liveCount > 0 && (
              <>
                <div className={`${SECTION} pt-3.5`}>
                  <span>{t("notifNeedsYou")}</span>
                  <span>{liveCount}</span>
                </div>
                {requestGroups.map((g) => (
                  <section key={`req:${g.session}`} aria-label={g.title}>
                    {requestGroupHeader(g.title, g.agent, g.items.length)}
                    {g.items.map(approvalCard)}
                  </section>
                ))}
              </>
            )}

            {BUCKETS.map((bucket) => {
              const inBucket = shown.filter((e) => dayBucket(e.at_ms, now) === bucket);
              if (inBucket.length === 0) return null;
              const groups = groupBySession(inBucket, (e) => ({ title: e.session_title, agent: e.agent }));
              const first = liveCount === 0 && bucket === firstBucket;
              return (
                <div key={bucket}>
                  <div className={`${SECTION} ${first ? "pt-3.5" : "pt-[18px]"}`}>
                    <span>{t(BUCKET_LABEL[bucket])}</span>
                    <span>{inBucket.length}</span>
                  </div>
                  {groups.map((g) => {
                    const key = `${bucket}:${g.session}`;
                    const done = isDoneGroup(g.items);
                    return (
                      <section key={key} aria-label={g.title}>
                        {groupHeader(key, g.title, g.agent, g.items.length)}
                        {!collapsed.has(key) && g.items.map((e) => entryRow(e, done))}
                      </section>
                    );
                  })}
                </div>
              );
            })}

            {liveCount === 0 && shown.length === 0 && (
              <p className="px-2 py-8 text-center text-[12px] text-tyba-text-faint">{t("notificationsEmpty")}</p>
            )}
          </div>
        </aside>
      )}
    </>
  );
}
