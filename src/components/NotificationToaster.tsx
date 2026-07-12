import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRight, Warning } from "@phosphor-icons/react";

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
  agentReadyWarnings: Record<SessionId, boolean>;
  onDismissAgentReady: (sessionId: SessionId) => void;
  onGoToSession: (sessionId: SessionId) => void;
}

export function NotificationToaster({
  sessions,
  agentReadyWarnings,
  onDismissAgentReady,
  onGoToSession,
}: Props) {
  const { t } = useTranslation();
  const [toasts, setToasts] = useState<ApprovalToastItem[]>([]);
  const [confirmingId, setConfirmingId] = useState<number | null>(null);
  const timers = useRef(new Map<number, ReturnType<typeof setTimeout>>());
  const agentReadyTimers = useRef(new Map<SessionId, ReturnType<typeof setTimeout>>());

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
      <ToastViewport />
    </ToastProvider>
  );
}
