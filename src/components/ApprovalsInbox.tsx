// Inbox de aprovações no sino: pedidos pendentes das sessões de agente.
// Estado vive no core (princípio #1); a lista chega por prop do App,
// que mantém a assinatura única de list + eventos.
// Ação vermelha nunca é 1-clique: Aprovar vira Confirmar (anti-fadiga).

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Bell, Warning } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  resolveApproval,
  type ApprovalDecision,
  type ApprovalRequest,
  type Session,
  type SessionId,
} from "../lib/ipc";

export function ApprovalsInbox({
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

  const decide = (request: ApprovalRequest, decision: ApprovalDecision) => {
    if (
      decision === "approved" &&
      request.risk === "red" &&
      confirming !== request.id
    ) {
      setConfirming(request.id);
      return;
    }
    setConfirming(null);
    resolveApproval(request.id, decision).catch(() => {});
  };

  const sessionTitle = (id: SessionId) =>
    sessions.find((s) => s.id === id)?.title ?? id.slice(0, 8);

  const count = approvals.length;

  return (
    <DropdownMenu
      open={open}
      onOpenChange={(next) => {
        setConfirming(null);
        onOpenChange(next);
      }}
    >
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label={t("approvals")}
          className="relative size-6 rounded-[4px] text-tyba-text-muted hover:text-tyba-text"
        >
          <Bell size={16} />
          {count > 0 && (
            <span className="absolute right-0.5 top-0.5 size-1.5 rounded-full bg-tyba-violet [box-shadow:var(--tyba-glow-violet)]" />
          )}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="w-96 border-tyba-border-strong bg-tyba-overlay shadow-lg"
      >
        <DropdownMenuLabel className="tyba-label flex items-center justify-between">
          {t("approvals")}
          {count > 0 && (
            <span className="rounded-full bg-tyba-violet-tint px-2 py-0.5 text-[10px] font-medium normal-case tracking-normal text-tyba-violet">
              {t("pendingCount", { count })}
            </span>
          )}
        </DropdownMenuLabel>
        {count === 0 ? (
          <div className="px-2 py-5 text-center text-xs text-tyba-text-faint">
            {t("notificationsEmpty")}
          </div>
        ) : (
          <div className="flex max-h-80 flex-col gap-1 overflow-y-auto p-1">
            {approvals.map((request) => (
              <div
                key={request.id}
                className={`rounded-md border bg-tyba-raised p-2.5 ${
                  request.risk === "red"
                    ? "border-tyba-red/35"
                    : "border-tyba-border"
                }`}
              >
                <div className="flex items-center gap-2">
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
                <div className="mt-2 flex gap-2">
                  <Button
                    size="sm"
                    onClick={() => decide(request, "approved")}
                    className={`h-6 rounded-[4px] px-2.5 text-[11px] ${
                      request.risk === "red" && confirming === request.id
                        ? "bg-tyba-red text-white hover:bg-tyba-red/90"
                        : ""
                    }`}
                  >
                    {request.risk === "red" && confirming === request.id
                      ? t("confirmApprove")
                      : t("approve")}
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => decide(request, "denied")}
                    className="h-6 rounded-[4px] px-2.5 text-[11px] text-tyba-text-muted"
                  >
                    {t("deny")}
                  </Button>
                </div>
              </div>
            ))}
          </div>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
