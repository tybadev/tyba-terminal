import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, CheckCircle, Warning, XCircle } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  Toast,
  ToastAction,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
} from "@/components/ui/toast";
import { RISK_DOT, RISK_LABEL, canAlwaysAllow } from "@/lib/notifications";
import {
  onApprovalRequested,
  onApprovalResolved,
  resolveApproval,
  sessionMarkSeen,
  type ApprovalDecision,
  type ApprovalRequest,
  type Session,
  type SessionId,
} from "@/lib/ipc";
import {
  addApprovalToast,
  removeApprovalToast,
  type ApprovalToastItem,
} from "@/lib/toastQueue";

const AUTO_DISMISS_MS = 8000;
const TURN_END_SETTLE_MS = 2500;

interface OutcomeToast {
  key: string;
  sessionId: SessionId;
  kind: "turn_ended" | "failed";
  reason: string | null;
  summary: string | null;
}

function SessionDeepLink({
  label,
  onClick,
}: {
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="group mt-0.5 flex min-w-0 items-center gap-1 truncate text-[11px] text-tyba-text-muted hover:text-tyba-text hover:underline"
    >
      <span className="min-w-0 truncate font-mono">{label}</span>
      <ArrowRight
        aria-hidden="true"
        size={12}
        weight="bold"
        className="shrink-0 opacity-60 transition-transform motion-safe:group-hover:translate-x-0.5"
      />
    </button>
  );
}

interface Props {
  sessions: Session[];
  activeSessionId: SessionId | null;
  agentReadyWarnings: Record<SessionId, boolean>;
  onDismissAgentReady: (sessionId: SessionId) => void;
  onGoToSession: (sessionId: SessionId) => void;
}

export function NotificationToaster({
  sessions,
  activeSessionId,
  agentReadyWarnings,
  onDismissAgentReady,
  onGoToSession,
}: Props) {
  const { t } = useTranslation();
  const [toasts, setToasts] = useState<ApprovalToastItem[]>([]);
  const [confirmingId, setConfirmingId] = useState<number | null>(null);
  const timers = useRef(new Map<number, ReturnType<typeof setTimeout>>());
  const agentReadyTimers = useRef(new Map<SessionId, ReturnType<typeof setTimeout>>());

  const [outcomes, setOutcomes] = useState<OutcomeToast[]>([]);
  const outcomeTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const settleTimers = useRef(new Map<SessionId, ReturnType<typeof setTimeout>>());
  const prevStates = useRef(
    new Map<SessionId, { state: Session["status"]["state"]; attention: boolean }>(),
  );
  const sessionsRef = useRef(sessions);
  sessionsRef.current = sessions;
  const activeSessionRef = useRef(activeSessionId);
  activeSessionRef.current = activeSessionId;

  const dismissOutcome = (key: string) => {
    const timer = outcomeTimers.current.get(key);
    if (timer) {
      clearTimeout(timer);
      outcomeTimers.current.delete(key);
    }
    setOutcomes((prev) => prev.filter((o) => o.key !== key));
  };

  const pushOutcome = (outcome: OutcomeToast) => {
    setOutcomes((prev) => [
      ...prev.filter((o) => o.sessionId !== outcome.sessionId),
      outcome,
    ]);
    outcomeTimers.current.set(
      outcome.key,
      setTimeout(() => dismissOutcome(outcome.key), AUTO_DISMISS_MS),
    );
  };

  useEffect(() => {
    for (const session of sessions) {
      if (session.kind.type !== "agent") continue;
      const status = session.status;
      const prev = prevStates.current.get(session.id);
      const next = status.state;
      if (prev?.state === next && prev.attention === session.attention) continue;
      prevStates.current.set(session.id, {
        state: next,
        attention: session.attention,
      });
      const settle = settleTimers.current.get(session.id);
      if (settle) {
        clearTimeout(settle);
        settleTimers.current.delete(session.id);
      }
      if (prev === undefined) continue;
      if (next === "failed" && prev.state !== "failed") {
        pushOutcome({
          key: `${session.id}:${Date.now()}`,
          sessionId: session.id,
          kind: "failed",
          reason: status.state === "failed" ? status.reason : null,
          summary: null,
        });
        continue;
      }
      const enteredIdle = next === "idle" && prev.state !== "idle";
      const attentionRaised =
        next === "idle" && prev.state === "idle" && !prev.attention && session.attention;
      if (!enteredIdle && !attentionRaised) continue;
      settleTimers.current.set(
        session.id,
        setTimeout(() => {
          settleTimers.current.delete(session.id);
          const latest = sessionsRef.current.find((s) => s.id === session.id);
          if (latest?.status.state !== "idle" || !latest.attention) return;
          if (!document.hasFocus() || activeSessionRef.current === session.id) {
            return;
          }
          pushOutcome({
            key: `${session.id}:${Date.now()}`,
            sessionId: session.id,
            kind: "turn_ended",
            reason: null,
            summary: latest.status.state === "idle" ? latest.status.summary : null,
          });
        }, TURN_END_SETTLE_MS),
      );
    }
    for (const id of [...prevStates.current.keys()]) {
      if (!sessions.some((s) => s.id === id)) {
        prevStates.current.delete(id);
        const settle = settleTimers.current.get(id);
        if (settle) {
          clearTimeout(settle);
          settleTimers.current.delete(id);
        }
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessions]);

  useEffect(() => {
    const pending = settleTimers.current;
    const shown = outcomeTimers.current;
    return () => {
      pending.forEach((timer) => clearTimeout(timer));
      pending.clear();
      shown.forEach((timer) => clearTimeout(timer));
      shown.clear();
    };
  }, []);

  const dismiss = (id: number) => {
    const timer = timers.current.get(id);
    if (timer) {
      clearTimeout(timer);
      timers.current.delete(id);
    }
    setToasts((prev) => removeApprovalToast(prev, id));
    setConfirmingId((prev) => (prev === id ? null : prev));
  };

  const dismissAgentReady = (sessionId: SessionId) => {
    const timer = agentReadyTimers.current.get(sessionId);
    if (timer) {
      clearTimeout(timer);
      agentReadyTimers.current.delete(sessionId);
    }
    onDismissAgentReady(sessionId);
  };

  const agentReadySessionIds = Object.keys(agentReadyWarnings).filter(
    (id) => agentReadyWarnings[id],
  );

  useEffect(() => {
    for (const id of agentReadySessionIds) {
      if (agentReadyTimers.current.has(id)) continue;
      agentReadyTimers.current.set(
        id,
        setTimeout(() => dismissAgentReady(id), AUTO_DISMISS_MS),
      );
    }
    for (const id of [...agentReadyTimers.current.keys()]) {
      if (!agentReadySessionIds.includes(id)) {
        clearTimeout(agentReadyTimers.current.get(id));
        agentReadyTimers.current.delete(id);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentReadySessionIds.join(",")]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const track = (p: Promise<() => void>) => {
      void p.then((un) => (disposed ? un() : unlisteners.push(un)));
    };
    track(
      onApprovalRequested((request) => {
        setToasts((prev) => addApprovalToast(prev, request));
        const timer = setTimeout(() => dismiss(request.id), AUTO_DISMISS_MS);
        timers.current.set(request.id, timer);
      }),
    );
    track(onApprovalResolved(({ id }) => dismiss(id)));
    return () => {
      disposed = true;
      unlisteners.forEach((un) => un());
      timers.current.forEach((timer) => clearTimeout(timer));
      timers.current.clear();
    };
  }, []);

  useEffect(() => {
    const readyTimers = agentReadyTimers.current;
    return () => {
      readyTimers.forEach((timer) => clearTimeout(timer));
      readyTimers.clear();
    };
  }, []);

  const sessionTitle = (id: SessionId) =>
    sessions.find((s) => s.id === id)?.title ?? id.slice(0, 8);

  const decide = (request: ApprovalRequest, decision: ApprovalDecision) => {
    if (
      decision === "approved" &&
      request.risk === "red" &&
      confirmingId !== request.id
    ) {
      setConfirmingId(request.id);
      return;
    }
    dismiss(request.id);
    resolveApproval(request.id, decision).catch(() => {});
  };

  return (
    <ToastProvider swipeDirection="right" duration={Infinity}>
      {toasts.map(({ id, approval }) => (
        <Toast
          key={id}
          onOpenChange={(nextOpen) => {
            if (!nextOpen) dismiss(id);
          }}
        >
          <div className="flex items-start gap-2">
            <span
              role="img"
              aria-label={t(RISK_LABEL[approval.risk])}
              className={`mt-1 size-1.5 shrink-0 rounded-full ${RISK_DOT[approval.risk]}`}
            />
            <div className="min-w-0 flex-1">
              <ToastTitle>{t("approvalRequested")}</ToastTitle>
              <ToastDescription>
                <code className="block truncate font-mono text-xs">
                  {approval.command}
                </code>
              </ToastDescription>
              <SessionDeepLink
                label={sessionTitle(approval.session_id)}
                onClick={() => onGoToSession(approval.session_id)}
              />
              {approval.risk === "red" && (
                <span className="mt-1 inline-flex items-center gap-1 rounded-full bg-tyba-red-tint px-2 py-0.5 text-[10px] font-medium text-tyba-red">
                  <Warning size={10} weight="bold" />
                  {t("redAction")}
                </span>
              )}
              <div className="mt-2 flex flex-wrap gap-2">
                <ToastAction altText={t("approve")} asChild>
                  <Button
                    size="sm"
                    onClick={() => decide(approval, "approved")}
                    className={`h-6 rounded-[4px] px-2.5 text-[11px] ${
                      approval.risk === "red" && confirmingId === id
                        ? "bg-tyba-red text-white hover:bg-tyba-red/90"
                        : ""
                    }`}
                  >
                    {approval.risk === "red" && confirmingId === id
                      ? t("confirmApprove")
                      : t("approve")}
                  </Button>
                </ToastAction>
                <ToastAction altText={t("deny")} asChild>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => decide(approval, "denied")}
                    className="h-6 rounded-[4px] px-2.5 text-[11px] text-tyba-text-muted"
                  >
                    {t("deny")}
                  </Button>
                </ToastAction>
                {canAlwaysAllow(approval.risk) && (
                  <ToastAction altText={t("alwaysAllow")} asChild>
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => decide(approval, "approved_always")}
                      className="h-6 rounded-[4px] px-2.5 text-[11px] text-tyba-text-muted"
                    >
                      {t("alwaysAllow")}
                    </Button>
                  </ToastAction>
                )}
              </div>
            </div>
          </div>
        </Toast>
      ))}
      {agentReadySessionIds.map((sessionId) => (
        <Toast
          key={`agent-ready:${sessionId}`}
          onOpenChange={(nextOpen) => {
            if (!nextOpen) dismissAgentReady(sessionId);
          }}
        >
          <div className="min-w-0 flex-1">
            <ToastTitle>{t("agentReadyTimeoutTitle")}</ToastTitle>
            <ToastDescription>{t("agentReadyTimeoutBody")}</ToastDescription>
            <SessionDeepLink
              label={sessionTitle(sessionId)}
              onClick={() => onGoToSession(sessionId)}
            />
            <div className="mt-2 flex flex-wrap gap-2">
              <ToastAction altText={t("dismiss")} asChild>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => dismissAgentReady(sessionId)}
                  className="h-6 rounded-[4px] px-2.5 text-[11px] text-tyba-text-muted"
                >
                  {t("dismiss")}
                </Button>
              </ToastAction>
            </div>
          </div>
        </Toast>
      ))}
      {outcomes.map((outcome) => {
        const session = sessions.find((s) => s.id === outcome.sessionId);
        const branch = session?.worktree?.branch ?? null;
        return (
          <Toast
            key={outcome.key}
            onOpenChange={(nextOpen) => {
              if (!nextOpen) dismissOutcome(outcome.key);
            }}
          >
            <div className="flex items-start gap-2">
              {outcome.kind === "failed" ? (
                <XCircle
                  aria-hidden="true"
                  size={16}
                  weight="fill"
                  className="mt-0.5 shrink-0 text-tyba-red"
                />
              ) : (
                <CheckCircle
                  aria-hidden="true"
                  size={16}
                  weight="fill"
                  className="mt-0.5 shrink-0 text-tyba-green"
                />
              )}
              <div className="min-w-0 flex-1">
                <ToastTitle>
                  {outcome.kind === "failed"
                    ? t("sessionFailedTitle")
                    : t("sessionTurnEndedTitle")}
                </ToastTitle>
                <ToastDescription>
                  {outcome.kind === "failed" && outcome.reason ? (
                    <code className="block truncate font-mono text-xs">
                      {outcome.reason}
                    </code>
                  ) : (
                    <span className="flex min-w-0 flex-col gap-0.5">
                      {outcome.summary && (
                        <span className="line-clamp-2 text-xs text-tyba-text-muted">
                          {outcome.summary}
                        </span>
                      )}
                      {branch && (
                        <span className="truncate font-mono text-[11px] text-tyba-text-faint">
                          {branch}
                        </span>
                      )}
                    </span>
                  )}
                </ToastDescription>
                <SessionDeepLink
                  label={sessionTitle(outcome.sessionId)}
                  onClick={() => {
                    dismissOutcome(outcome.key);
                    void sessionMarkSeen(outcome.sessionId).catch(() => {});
                    onGoToSession(outcome.sessionId);
                  }}
                />
              </div>
            </div>
          </Toast>
        );
      })}
      <ToastViewport />
    </ToastProvider>
  );
}
