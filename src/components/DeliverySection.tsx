import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowSquareOut,
  CheckCircle,
  CircleNotch,
  GitMerge,
  GitPullRequest,
  XCircle,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  forgePrForSession,
  forgeStatus as fetchForgeStatus,
  type ForgeStatus,
  type PullRequest,
  type Session,
} from "../lib/ipc";
import { overallChecksTone, primaryDeliveryAction } from "../lib/forge";
import { openExternalUrl } from "../lib/clipboard";
import { OpenPrDialog } from "./OpenPrDialog";
import { MergeLocalDialog } from "./MergeLocalDialog";
import { PrCommentsPanel } from "./PrCommentsPanel";
import { PostDeliveryDialog } from "./PostDeliveryDialog";

interface Props {
  session: Session;
  hasUncommittedWork: boolean;
  onSendToAgent: (prompt: string) => Promise<void>;
  onClose: () => void;
}

const CHECK_ICON = {
  success: <CheckCircle size={13} className="text-tyba-green" />,
  failure: <XCircle size={13} className="text-tyba-red" />,
  pending: <CircleNotch size={13} className="animate-spin text-tyba-yellow" />,
};

export function DeliverySection({
  session,
  hasUncommittedWork,
  onSendToAgent,
  onClose,
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

  return (
    <div className="flex flex-col border-t border-tyba-border">
      <div className="flex h-10 shrink-0 items-center gap-2 px-4">
        <span className="tyba-label">{t("deliveryTitle")}</span>
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
        {status && (
          <Button
            size="sm"
            className="h-6 gap-1.5 px-2.5 text-[11px]"
            onClick={() => setOpenPr(true)}
          >
            <GitPullRequest size={12} />
            {action === "open_mr" ? t("deliveryOpenMr") : t("deliveryOpenPr")}
          </Button>
        )}
        <Button
          variant="outline"
          size="sm"
          className="h-6 gap-1.5 px-2.5 text-[11px] text-tyba-red hover:bg-tyba-red/10 hover:text-tyba-red"
          onClick={() => setOpenMerge(true)}
        >
          <GitMerge size={12} />
          {t("deliveryMergeLocal")}
        </Button>
      </div>

      {pr && (
        <div className="flex h-9 shrink-0 items-center gap-2 border-t border-tyba-border px-4 text-[11px]">
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
