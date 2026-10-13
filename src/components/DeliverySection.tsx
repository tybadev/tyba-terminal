import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowSquareOut,
  CheckCircle,
  CircleNotch,
  XCircle,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  forgePrForSession,
  forgeStatus as fetchForgeStatus,
  type ConflictState,
  type ForgeStatus,
  type PullRequest,
  type Session,
} from "../lib/ipc";
import {
  deliveryButtonKind,
  overallChecksTone,
  primaryDeliveryAction,
  type DeliveryButtonKind,
} from "../lib/forge";
import { openExternalUrl } from "../lib/clipboard";
import { OpenPrDialog } from "./OpenPrDialog";
import { MergeLocalDialog } from "./MergeLocalDialog";
import { PrCommentsPanel } from "./PrCommentsPanel";
import { PostDeliveryDialog } from "./PostDeliveryDialog";

interface Props {
  session: Session;
  hasUncommittedWork: boolean;
  aheadCount: number;
  isWorktree: boolean;
  onSendToAgent: (prompt: string) => Promise<void>;
  onResolveConflicts: (state: ConflictState) => Promise<void>;
  onClose: () => void;
  /** Esquerda da barra (mockup Diff: enviar comentários ao agente). */
  leading?: ReactNode;
  /** Antes de Merge local e PR (o push do Diff). */
  trailing?: ReactNode;
}

const CHECK_ICON = {
  success: <CheckCircle size={13} className="text-tyba-green" />,
  failure: <XCircle size={13} className="text-tyba-red" />,
  pending: <CircleNotch size={13} className="animate-spin text-tyba-amber" />,
};

export function DeliverySection({
  session,
  hasUncommittedWork,
  aheadCount,
  isWorktree,
  onSendToAgent,
  onResolveConflicts,
  onClose,
  leading,
  trailing,
}: Props) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<ForgeStatus | null | undefined>(
    undefined,
  );
  const [pr, setPr] = useState<PullRequest | null | undefined>(undefined);
  const [openPr, setOpenPr] = useState(false);
  const [openMerge, setOpenMerge] = useState(false);
  const [postDelivery, setPostDelivery] = useState<"pr" | "merge" | null>(
    null,
  );
  const [statusError, setStatusError] = useState(false);

  const refresh = useCallback(() => {
    setStatusError(false);
    void fetchForgeStatus(session.id)
      .then(setStatus)
      .catch(() => {
        setStatus(null);
        setStatusError(true);
      });
    void forgePrForSession(session.id)
      .then(setPr)
      .catch(() => setPr(null));
  }, [session.id]);

  useEffect(() => {
    setStatus(undefined);
    setPr(undefined);
    refresh();
  }, [refresh]);

  const action = primaryDeliveryAction(status ?? null);
  const checksTone = pr ? overallChecksTone(pr.checks) : null;
  const buttonKind = deliveryButtonKind(action, pr ?? null);
  const buttonLabelKey: Record<DeliveryButtonKind, string> = {
    view_pr: "deliveryViewPr",
    view_mr: "deliveryViewMr",
    open_pr: "deliveryOpenPr",
    open_mr: "deliveryOpenMr",
  };

  return (
    <div className="flex flex-col tyba-divide-t">
      {/* Mockup Diff: uma barra só — enviar à esquerda, entregar à direita. */}
      <div className="flex shrink-0 items-center gap-2.5 bg-[var(--tyba-panel-bg)] px-4 pb-2.5 pt-3">
        {leading ?? <span className="tyba-label">{t("deliveryTitle")}</span>}
        <div className="flex-1" />
        {status === null && !statusError && (
          <span className="text-[11px] text-tyba-text-faint">
            {t("deliveryHintNoForge")}
          </span>
        )}
        {status === null && statusError && (
          <button
            type="button"
            onClick={refresh}
            className="text-[11px] text-tyba-amber hover:underline"
          >
            {t("deliveryStatusError")}
          </button>
        )}
        {trailing}
        <Button
          variant="ghost"
          size="sm"
          className="h-auto shrink-0 rounded-[6px] border border-tyba-border-strong bg-transparent px-3 py-[5px] text-[12px] font-normal text-tyba-text-muted hover:bg-transparent hover:text-tyba-text"
          onClick={() => setOpenMerge(true)}
        >
          {t("deliveryMergeLocal")}
        </Button>
        {status &&
          (() => {
            const deliverable = !isWorktree || aheadCount > 0;
            const demoted = !pr && hasUncommittedWork;
            const blocked = !pr && !deliverable;
            return (
              <Button
                size="sm"
                variant={demoted || blocked ? "outline" : "default"}
                className={`h-auto shrink-0 rounded-[6px] px-3.5 py-1.5 text-[12px] font-normal ${
                  demoted || blocked
                    ? ""
                    : "bg-tyba-green text-tyba-text-inverse hover:bg-tyba-green/90"
                }`}
                disabled={blocked}
                title={blocked ? t("deliveryCommitFirst") : undefined}
                onClick={() =>
                  pr ? void openExternalUrl(pr.url) : setOpenPr(true)
                }
              >
                {t(buttonLabelKey[buttonKind])}
              </Button>
            );
          })()}
      </div>
      <p className="bg-[var(--tyba-panel-bg)] px-4 pb-2.5 text-right text-[11px] text-tyba-text-faint">
        {t(status ? "deliveryApprovalNote" : "deliveryApprovalNoteMergeOnly")}
      </p>

      {pr && (
        <div className="flex h-9 shrink-0 items-center gap-2 tyba-divide-t px-4 text-[11px]">
          <span className="rounded-full border border-tyba-border px-2 py-0.5 text-tyba-text-muted">
            {t(`prState_${pr.state}`, { defaultValue: pr.state })}
          </span>
          <span className="font-mono text-tyba-text-faint">
            #{pr.number}
          </span>
          {pr.checks.length === 0 ? (
            <span className="text-tyba-text-faint">{t("prChecksNone")}</span>
          ) : (
            <span className="flex items-center gap-1">
              {checksTone && CHECK_ICON[checksTone]}
            </span>
          )}
          <button
            onClick={() => void openExternalUrl(pr.url)}
            className="flex items-center gap-1 text-tyba-text-faint hover:text-tyba-text"
          >
            <ArrowSquareOut size={12} />
            {t("prOpenInForge")}
          </button>
        </div>
      )}

      {pr && (
        <PrCommentsPanel
          sessionId={session.id}
          pr={pr}
          onSendToAgent={onSendToAgent}
        />
      )}

      {status && (
        <OpenPrDialog
          session={session}
          status={status}
          action={action === "open_mr" ? "open_mr" : "open_pr"}
          open={openPr}
          onClose={() => setOpenPr(false)}
          onCreated={(created) => {
            setOpenPr(false);
            setPr(created);
            setPostDelivery("pr");
          }}
        />
      )}

      <MergeLocalDialog
        session={session}
        open={openMerge}
        onClose={() => setOpenMerge(false)}
        onMerged={() => {
          setOpenMerge(false);
          setPostDelivery("merge");
        }}
        onResolveConflicts={onResolveConflicts}
      />

      <PostDeliveryDialog
        session={session}
        kind={postDelivery ?? "pr"}
        hasUncommittedWork={hasUncommittedWork}
        open={postDelivery !== null}
        onClose={() => setPostDelivery(null)}
        onRemoved={() => {
          setPostDelivery(null);
          onClose();
        }}
      />
    </div>
  );
}
