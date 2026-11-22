import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Check, Clock, GithubLogo, WarningCircle, X, type Icon } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { SegmentedControl } from "@/components/ui/segmented";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Textarea } from "@/components/ui/textarea";
import {
  RISK_LABEL,
  approvalActions,
  attentionCount,
  dayBucket,
  decideApproval,
  filterEntries,
  historyEntries,
  isRequestKind,
  resetParts,
  unreadCount,
  type DayBucket,
  type NotificationFilter,
  entryDiffTarget,
} from "@/lib/notifications";
import { formatElapsed } from "@/lib/agentTree";
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
import { useNow } from "./ui/loading";

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

// Quadro Inbox (aprovado em 2026-11-15): linha solta com o ícone num
// quadradinho por tipo; o que precisa de você vira a placa branca com
// contorno amarelo. Botões pequenos do Muriki (primário, vidro, fantasma).
const BTN =
  "inline-flex h-[34px] shrink-0 items-center justify-center gap-2 rounded-[10px] px-3.5 text-[13px] font-semibold whitespace-nowrap outline-none transition-colors focus-visible:[box-shadow:var(--tyba-focus-ring)] disabled:pointer-events-none disabled:opacity-45";
const BTN_PRIMARY = `${BTN} bg-tyba-blue text-tyba-text-inverse hover:bg-tyba-blue/90`;
const BTN_GLASS = `${BTN} bg-[var(--tyba-glass-piece)] text-tyba-text shadow-[inset_0_0_0_1px_var(--tyba-line)] hover:bg-tyba-hover`;
const BTN_GHOST = `${BTN} text-tyba-text-muted hover:bg-tyba-hover hover:text-tyba-text`;
const ITEM = "relative mx-2 my-0.5 flex gap-2.5 rounded-[12px] px-3 py-2.5";
const ITEM_NEED =
  "tyba-pill-on shadow-[var(--tyba-pill-on),inset_0_0_0_1.5px_var(--tyba-amber-fill)]!";
const SEC =
  "flex items-center gap-1.5 px-4 pb-1.5 pt-3.5 text-[11px] font-semibold uppercase tracking-[0.04em] text-tyba-text-faint";
const TITLE = "text-[13px] leading-[18px]";
const DESC = "mt-0.5 text-[12px] leading-[17px] text-tyba-text-muted";
const META = "mt-1 flex min-w-0 items-center gap-2 font-mono text-[10.5px] text-tyba-text-faint";
const ACTS = "mt-2 flex flex-wrap items-center gap-1.5";

type GlyphTone = "amber" | "blue" | "red" | "neutral";
const GLYPH_TONE: Record<GlyphTone, string> = {
  amber: "bg-tyba-amber-tint text-tyba-amber",
  blue: "bg-tyba-blue-tint text-tyba-blue",
  red: "bg-tyba-red-tint text-tyba-red",
  neutral: "bg-tyba-neutral-tint text-tyba-text-muted",
};

/** O quadradinho do tipo (quadro Inbox, `.nx-gl`). */
function Glyph({ icon: Glyphicon, tone, label }: { icon: Icon; tone: GlyphTone; label: string }) {
  return (
    <span
      role="img"
      aria-label={label}
      className={`mt-px grid size-[26px] shrink-0 place-items-center rounded-[8px] ${GLYPH_TONE[tone]}`}
    >
      <Glyphicon size={15} weight="bold" />
    </span>
  );
}

const riskTone = (risk: RiskLevel | null): GlyphTone => (risk === "red" ? "red" : "amber");

function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/**
 * Inbox (quadro Inbox): coluna de 360px ao lado da lateral. Em cima, o que
 * pede você agora (pedidos do TYBA e o prompt nativo esperando no
 * terminal); embaixo, o histórico que o core guarda (terminou, falhou,
 * limite), por dia.
 */
export function NotificationsInbox({
  sessions,
  approvals,
  open,
  onOpenChange,
  onGoToSession,
  onBadgeChange,
}: {
  sessions: Session[];
  approvals: ApprovalRequest[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onGoToSession: (sessionId: SessionId) => void;
  /** Contagem do item Inbox da lateral (quadro PropostaClaro): a coluna não
   *  tem botão próprio, quem abre é a lateral. */
  onBadgeChange: (count: number) => void;
}) {
  const { t } = useTranslation();
  const [feedbackFor, setFeedbackFor] = useState<number | null>(null);
  const [feedback, setFeedback] = useState("");
  const [entries, setEntries] = useState<NotificationEntry[]>([]);
  const [filter, setFilter] = useState<NotificationFilter>("all");
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

  const history = useMemo(() => historyEntries(entries), [entries]);
  // O prompt nativo esperando no terminal é "precisa de você" (quadro Inbox):
  // sobe para o topo, fora dos dias e de qualquer filtro.
  const nativeWaiting = useMemo(
    () => history.filter((e) => e.kind === "native_approval" && e.pending),
    [history],
  );
  const past = useMemo(
    () => history.filter((e) => !(e.kind === "native_approval" && e.pending)),
    [history],
  );
  const shown = useMemo(() => filterEntries(past, filter), [past, filter]);
  const badge = attentionCount(approvals.length, entries);
  useEffect(() => onBadgeChange(badge), [badge, onBadgeChange]);
  // Só o histórico: os pedidos pendentes do TYBA têm a seção "Precisa de você".
  const unread = unreadCount(history);
  const liveCount = approvals.length + nativeWaiting.length;
  const now = useNow(open && liveCount > 0);

  const goToSession = (sessionId: SessionId) => {
    onGoToSession(sessionId);
    onOpenChange(false);
  };

  const markRead = (e: NotificationEntry) => {
    if (!e.read) void notificationsMarkRead([e.id]).catch(() => {});
  };

  // "Abrir na sessão": a única ação do vermelho (R4) e do amarelo que
  // espera no prompt nativo do Claude (decide-se no terminal, não aqui).
  const openInSessionBtn = (sessionId: SessionId, onOpen?: () => void) => (
    <button
      type="button"
      className={BTN_PRIMARY}
      onClick={(ev) => {
        ev.stopPropagation();
        onOpen?.();
        goToSession(sessionId);
      }}
    >
      <ArrowRight size={16} weight="bold" />
      {t("notifOpenInSession")}
    </button>
  );

  const ago = (ms: number) => t("agentsAgo", { time: formatElapsed(Math.max(0, now - ms)) });

  const approvalCard = (request: ApprovalRequest) => {
    // Pedido do TYBA: aprovar (primário), negar, "não, e dizer…", abrir a
    // sessão e por último o "sempre permitir". O conjunto vem de approvalActions.
    const actions = approvalActions(request.risk);
    const ordered = [...actions.filter((a) => a.id !== "alwaysAllow"), ...actions.filter((a) => a.id === "alwaysAllow")];
    const red = request.risk === "red";
    return (
      <div key={request.id} className={`motion-safe:animate-tyba-pop-in ${ITEM} ${ITEM_NEED}`}>
        <Glyph icon={WarningCircle} tone={riskTone(request.risk)} label={t(RISK_LABEL[request.risk])} />
        <div className="min-w-0 flex-1">
          <div className={`${TITLE} font-semibold text-tyba-text`}>
            {red ? t("notifRedTitle") : t("notifAskTitle")}
          </div>
          <div className={`${DESC} line-clamp-3 break-all`} title={request.command}>
            {request.command} · {t("notifInSession", { session: sessionTitle(request.session_id) })}
          </div>
          {request.context && <div className={DESC}>{request.context}</div>}
          <div className={META}>
            {ago(request.requested_at_ms)} · {red ? t("redAction") : t("notifAsks")}
          </div>
          <div className={ACTS}>
            {/* R4: vermelho não se aprova de fora da sessão — só abrir nela. */}
            {red ? (
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
                    action.id === "approve" ? BTN_PRIMARY : action.id === "deny" ? BTN_GLASS : BTN_GHOST;
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
                <button type="button" className={BTN_GHOST} onClick={() => onGoToSession(request.session_id)}>
                  {t("notifOpenSession")}
                </button>
              </>
            )}
          </div>
          {feedbackFor === request.id && !red && (
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
                className="min-h-0 resize-none text-[12px]"
              />
              <div className="flex gap-2">
                <Button size="sm" onClick={() => decide(request, "denied", feedback)} className="h-7 rounded-[8px] px-3 text-[12px]">
                  {t("denyAndTell")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    setFeedbackFor(null);
                    setFeedback("");
                  }}
                  className="h-7 rounded-[8px] px-3 text-[12px] text-tyba-text-muted"
                >
                  {t("cancel")}
                </Button>
              </div>
            </div>
          )}
        </div>
      </div>
    );
  };

  function entryTitle(e: NotificationEntry): string {
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
  }

  /** O prompt nativo esperando no terminal: decide-se lá, nunca daqui. */
  const nativeCard = (e: NotificationEntry) => (
    <div key={e.id} className={`motion-safe:animate-tyba-pop-in ${ITEM} ${ITEM_NEED}`}>
      <Glyph icon={WarningCircle} tone={riskTone(e.risk)} label={t(RISK_LABEL[e.risk ?? "yellow"])} />
      <div className="min-w-0 flex-1 pr-3">
        <div className={`${TITLE} font-semibold text-tyba-text`}>{entryTitle(e)}</div>
        <div className={`${DESC} truncate`} title={e.command ?? undefined}>
          {[e.command, e.session_title && t("notifInSession", { session: e.session_title })]
            .filter(Boolean)
            .join(" · ")}
        </div>
        <div className={META}>
          {ago(e.at_ms)} · {t("notifAnswerInPrompt")}
        </div>
        <div className={ACTS}>{openInSessionBtn(e.session_id, () => markRead(e))}</div>
      </div>
      {!e.read && <UnreadDot label={t("notifUnread")} />}
    </div>
  );

  /** O quadradinho por tipo: pedido, falha, limite, turno terminado. Lida,
   *  a linha de turno fica neutra (quadro: "Ontem"). */
  const entryGlyph = (e: NotificationEntry) => {
    if (isRequestKind(e)) return <Glyph icon={WarningCircle} tone="neutral" label={t("notifGlyphRequest")} />;
    if (e.kind === "usage_limit")
      return <Glyph icon={Clock} tone={e.category === "error" ? "red" : "amber"} label={t("notifGlyphWarning")} />;
    if (e.category === "error") return <Glyph icon={X} tone="red" label={t("notifGlyphError")} />;
    if (e.category === "info") return <Glyph icon={WarningCircle} tone="amber" label={t("notifGlyphWarning")} />;
    return <Glyph icon={Check} tone={e.read ? "neutral" : "blue"} label={t("notifGlyphDone")} />;
  };

  const entryRow = (e: NotificationEntry) => {
    const reset = e.usage?.resets_at ? resetParts(e.usage.resets_at, Date.now()) : null;
    const resets = reset && (reset.date ? t("notifResetsOn", reset) : t("notifResetsAt", reset));
    // O diff do turno enquanto ele for o último; turno antigo fica só com os números.
    const diffTarget = entryDiffTarget(
      e,
      sessions.find((s) => s.id === e.session_id),
    );
    const limit = e.kind === "usage_limit";
    // Quadro: o aviso de limite diz quem e quando volta; o resto, o resumo.
    const desc = limit
      ? [e.agent ?? e.session_title, resets].filter(Boolean).join(" · ")
      : (e.detail ?? e.command);
    return (
      <div
        key={e.id}
        role="button"
        tabIndex={0}
        onClick={() => markRead(e)}
        onKeyDown={(ev) => {
          if (ev.key === "Enter") markRead(e);
        }}
        className={`${ITEM} cursor-default outline-none hover:bg-[color-mix(in_srgb,var(--tyba-paper-solid)_70%,transparent)] focus-visible:[box-shadow:var(--tyba-focus-ring)]`}
      >
        {entryGlyph(e)}
        <div className="min-w-0 flex-1 pr-3">
          <div className={`${TITLE} ${e.read ? "font-medium text-tyba-text-muted" : "font-semibold text-tyba-text"}`}>
            {entryTitle(e)}
          </div>
          {desc && (
            <div className={`${DESC} line-clamp-2 break-words`} title={desc}>
              {desc}
            </div>
          )}
          <div className={`${META} whitespace-nowrap tabular-nums`}>
            {!limit && <span className="min-w-0 truncate">{e.session_title}</span>}
            {!limit && <span>·</span>}
            {e.turn && e.turn.files !== null && (
              <>
                <span className="shrink-0">
                  {e.turn.files > 0 ? t("diffFilesSummary", { count: e.turn.files }) : t("pillTurnNoChanges")}
                </span>
                {e.turn.files > 0 && (
                  <>
                    <span className="shrink-0 font-semibold text-tyba-green">+{e.turn.insertions}</span>
                    <span className="-ml-1 shrink-0 font-semibold text-tyba-red">−{e.turn.deletions}</span>
                  </>
                )}
                <span>·</span>
              </>
            )}
            <span className="shrink-0">{clock(e.at_ms)}</span>
          </div>
          {/* Quadro: falha e limite são só aviso, sem botões. */}
          {e.kind === "agent_finished" && (
            <div className={ACTS}>
              {diffTarget && (
                <button
                  type="button"
                  className={BTN_GLASS}
                  onClick={(ev) => {
                    ev.stopPropagation();
                    markRead(e);
                    void openGitScreen(e.session_id, diffTarget).catch(() => {});
                    onOpenChange(false);
                  }}
                >
                  <GithubLogo size={16} />
                  {t("notifViewDiff")}
                </button>
              )}
              <button
                type="button"
                className={BTN_GHOST}
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
        {!e.read && <UnreadDot label={t("notifUnread")} />}
      </div>
    );
  };

  return (
    <>
      {open && (
        <aside
          ref={panelRef}
          aria-label={t("notifications")}
          // Coluna encostada na borda direita, empurrando o terminal (quadro
          // Inbox): a mesma seção de vidro da lateral, 360px. Entra deslizando
          // da direita, sem animação para quem pede menos movimento.
          data-inbox-section
          className="tyba-section tyba-mk-column flex w-[360px] shrink-0 flex-col overflow-hidden motion-safe:animate-tyba-panel-in"
        >
          <div className="flex shrink-0 items-center gap-2 px-4 pb-2.5 pt-3.5">
            <span className="text-[15px] font-semibold text-tyba-text">{t("inbox")}</span>
            {unread > 0 && (
              <span className="text-[12px] text-tyba-text-muted">{t("notifUnreadCount", { count: unread })}</span>
            )}
            <button
              type="button"
              disabled={unread === 0}
              onClick={() => void notificationsMarkRead(null).catch(() => {})}
              className={`${BTN_GHOST} ml-auto`}
            >
              <Check size={16} />
              {t("notifMarkAllRead")}
            </button>
          </div>

          <div className="shrink-0 px-4 pb-1">
            <SegmentedControl
              aria-label={t("notifFilter")}
              value={filter}
              onChange={setFilter}
              options={FILTERS.map((f) => ({ value: f.key, label: t(f.label) }))}
            />
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto pb-3">
            {/* "Precisa de você" aparece em qualquer filtro: com o painel aberto
                o toast esconde os pedidos, então nenhum pode sumir daqui. */}
            {liveCount > 0 && (
              <section aria-label={t("notifNeedsYou")}>
                <div className={SEC}>{t("notifNeedsYou")}</div>
                {approvals.map(approvalCard)}
                {nativeWaiting.map(nativeCard)}
              </section>
            )}

            {BUCKETS.map((bucket) => {
              const inBucket = shown.filter((e) => dayBucket(e.at_ms, Date.now()) === bucket);
              if (inBucket.length === 0) return null;
              return (
                <section key={bucket} aria-label={t(BUCKET_LABEL[bucket])}>
                  <div className={SEC}>{t(BUCKET_LABEL[bucket])}</div>
                  {inBucket.map(entryRow)}
                </section>
              );
            })}

            {liveCount === 0 && shown.length === 0 && (
              <p className="px-6 py-8 text-center text-[12px] text-tyba-text-faint">{t("notificationsEmpty")}</p>
            )}
          </div>
        </aside>
      )}
    </>
  );
}

/** Não lida (quadro Inbox, `.nx-un`): o ponto azul no canto. */
function UnreadDot({ label }: { label: string }) {
  return (
    <span role="img" aria-label={label} className="absolute right-3 top-3.5 size-[7px] rounded-full bg-tyba-blue" />
  );
}
