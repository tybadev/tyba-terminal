import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Tray, Warning } from "@phosphor-icons/react";

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
import {
  RISK_DOT,
  RISK_LABEL,
  approvalChoices,
  choiceForKey,
  shouldAutoClosePopover,
  toNotificationItems,
} from "@/lib/notifications";
import { Textarea } from "@/components/ui/textarea";
import { Kbd } from "@/components/ui/kbd";
import {
  resolveApproval,
  type ApprovalDecision,
  type ApprovalRequest,
  type Session,
  type SessionId,
} from "../lib/ipc";

export function NotificationsInbox({
  sessions,
  approvals,
  open,
  onOpenChange,
}: {
  sessions: Session[];
  approvals: ApprovalRequest[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState<number | null>(null);
  const [feedbackFor, setFeedbackFor] = useState<number | null>(null);
  const [feedback, setFeedback] = useState("");

  const decide = (
    request: ApprovalRequest,
    decision: ApprovalDecision,
    withFeedback?: string,
  ) => {
    if (
      decision === "approved" &&
      request.risk === "red" &&
      confirming !== request.id
    ) {
      setConfirming(request.id);
      return;
    }
    setConfirming(null);
    setFeedbackFor(null);
    setFeedback("");
    resolveApproval(request.id, decision, withFeedback).catch(() => {});
  };

  const pickByKey = (request: ApprovalRequest, key: string) => {
    const choice = choiceForKey(request.risk, key);
    if (!choice) return;
    if (choice.wantsFeedback) {
      setFeedbackFor(request.id);
      return;
    }
    decide(request, choice.decision);
  };

  const sessionTitle = (id: SessionId) =>
    sessions.find((s) => s.id === id)?.title ?? id.slice(0, 8);

  const count = approvals.length;
  const previousCountRef = useRef(count);

  useEffect(() => {
    if (
      shouldAutoClosePopover({
        open,
        previousCount: previousCountRef.current,
        nextCount: count,
      })
    ) {
      onOpenChange(false);
    }
    previousCountRef.current = count;
  }, [count, open, onOpenChange]);

  const items = toNotificationItems(approvals);
  const first = items[0]?.approval;

  useEffect(() => {
    if (!open || !first || feedbackFor !== null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const target = e.target as HTMLElement | null;
      if (target && ["INPUT", "TEXTAREA"].includes(target.tagName)) return;
      const choice = choiceForKey(first.risk, e.key);
      if (!choice) return;
      e.preventDefault();
      pickByKey(first, e.key);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setConfirming(null);
        onOpenChange(next);
      }}
    >
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label={t("notifications")}
          className="relative size-6 rounded-[4px] text-tyba-text-muted hover:text-tyba-text"
        >
          <Tray size={16} weight={count > 0 ? "fill" : "regular"} />
          {count > 0 && (
            <span className="absolute -right-0.5 -top-0.5 flex min-w-3.5 items-center justify-center rounded-full bg-tyba-violet px-1 font-mono text-[9px] font-semibold leading-none text-white [box-shadow:var(--tyba-glow-violet)]">
              {count}
            </span>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-96">
        <div className="tyba-label flex items-center justify-between px-2 py-1.5">
          {t("notifications")}
          {count > 0 && (
            <span className="rounded-full bg-tyba-violet-tint px-2 py-0.5 text-[10px] font-medium normal-case tracking-normal text-tyba-violet">
              {t("pendingCount", { count })}
            </span>
          )}
        </div>
        <div
          role="separator"
          aria-orientation="horizontal"
          className="-mx-1 my-1 h-px bg-tyba-border"
        />
        {count === 0 ? (
          <div className="px-2 py-5 text-center text-xs text-tyba-text-faint">
            {t("notificationsEmpty")}
          </div>
        ) : (
          <div className="flex max-h-80 flex-col gap-1 overflow-y-auto p-1">
            {items.map((item) => {
              switch (item.kind) {
                case "approval": {
                  const request = item.approval;
                  return (
                    <div
                      key={item.id}
                      className={`motion-safe:animate-tyba-pop-in rounded-md border bg-tyba-raised p-2.5 ${
                        request.risk === "red"
                          ? "border-tyba-red/35"
                          : "border-tyba-border"
                      }`}
                    >
                      <div className="flex items-center gap-2">
                        <span
                          role="img"
                          aria-label={t(RISK_LABEL[request.risk])}
                          className={`size-1.5 shrink-0 rounded-full ${RISK_DOT[request.risk]}`}
                        />
                        <code className="min-w-0 flex-1 truncate font-mono text-xs">
                          {request.command}
                        </code>
                        {request.risk === "red" && (
                          <span className="flex shrink-0 items-center gap-1 rounded-full bg-tyba-red-tint px-2 py-0.5 text-[10px] font-medium text-tyba-red">
                            <Warning size={10} weight="bold" />
                            {t("redAction")}
                          </span>
                        )}
                      </div>
                      <div className="mt-0.5 truncate font-mono text-[10px] text-tyba-text-faint">
                        {sessionTitle(request.session_id)}
                        {request.cwd ? ` · ${request.cwd}` : ""}
                      </div>
                      {request.context && (
                        <div className="mt-1 text-[11px] text-tyba-text-muted">
                          {request.context}
                        </div>
                      )}
                      <div className="mt-2 flex flex-wrap gap-2">
                        {approvalChoices(request.risk).map((choice) => {
                          const isRedConfirm =
                            choice.decision === "approved" &&
                            request.risk === "red" &&
                            confirming === request.id;
                          const button = (
                            <Button
                              key={choice.index}
                              size="sm"
                              variant={
                                choice.decision === "approved"
                                  ? "default"
                                  : "outline"
                              }
                              onClick={() =>
                                choice.wantsFeedback
                                  ? setFeedbackFor(request.id)
                                  : decide(request, choice.decision)
                              }
                              className={`h-6 gap-1.5 rounded-[4px] px-2.5 text-[11px] ${
                                choice.decision === "approved"
                                  ? ""
                                  : "text-tyba-text-muted"
                              } ${
                                isRedConfirm
                                  ? "bg-tyba-red text-white hover:bg-tyba-red/90"
                                  : ""
                              }`}
                            >
                              <Kbd>{choice.index}</Kbd>
                              {isRedConfirm
                                ? t("confirmApprove")
                                : t(choice.labelKey)}
                            </Button>
                          );
                          return choice.decision === "approved_always" ? (
                            <Tooltip key={choice.index}>
                              <TooltipTrigger asChild>{button}</TooltipTrigger>
                              <TooltipContent side="bottom">
                                {t("alwaysAllowHint")}
                              </TooltipContent>
                            </Tooltip>
                          ) : (
                            button
                          );
                        })}
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
                                setFeedbackFor(null);
                                setFeedback("");
                              }
                            }}
                            placeholder={t("denyFeedbackPlaceholder")}
                            className="min-h-0 resize-none text-[11px]"
                          />
                          <div className="flex gap-2">
                            <Button
                              size="sm"
                              onClick={() => decide(request, "denied", feedback)}
                              className="h-6 rounded-[4px] px-2.5 text-[11px]"
                            >
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
                  );
                }
                default:
                  return null;
              }
            })}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}
