import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { translateError } from "../lib/errors";
import { ArrowSquareOut, GitPullRequest } from "@phosphor-icons/react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { openExternalUrl } from "../lib/clipboard";
import {
  forgeCreatePr,
  worktreePush,
  type ForgeStatus,
  type PullRequest,
  type Session,
} from "../lib/ipc";
import { forgeCliReady, type DeliveryAction } from "../lib/forge";

interface Props {
  session: Session;
  status: ForgeStatus;
  action: Extract<DeliveryAction, "open_pr" | "open_mr">;
  open: boolean;
  onClose: () => void;
  onCreated: (pr: PullRequest) => void;
}

export function OpenPrDialog({
  session,
  status,
  action,
  open,
  onClose,
  onCreated,
}: Props) {
  const { t } = useTranslation();
  const isMr = action === "open_mr";
  const [title, setTitle] = useState(session.title);
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pushing, setPushing] = useState(false);

  useEffect(() => {
    if (!open) return;
    setTitle(session.title);
    setBody("");
    setError(null);
    setBusy(false);
    setPushing(false);
  }, [open, session.title]);

  if (!open) return null;

  const ready = forgeCliReady(status);

  const submit = async () => {
    const trimmed = title.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    setError(null);
    try {
      const pr = await forgeCreatePr(session.id, trimmed, body);
      onCreated(pr);
    } catch (e) {
      setError(translateError(e, t));
    } finally {
      setBusy(false);
    }
  };

  const pushAndOpenWeb = async () => {
    if (pushing) return;
    setPushing(true);
    setError(null);
    try {
      await worktreePush(session.id);
      if (status.web_create_url) {
        await openExternalUrl(status.web_create_url);
      }
      onClose();
    } catch (e) {
      setError(translateError(e, t));
    } finally {
      setPushing(false);
    }
  };

  return (
    <Dialog open onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="max-w-[520px] border-tyba-border-strong bg-tyba-surface">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-[14px]">
            <GitPullRequest size={16} className="text-tyba-green" />
            {isMr ? t("prDialogTitleMr") : t("prDialogTitlePr")}
          </DialogTitle>
          <DialogDescription className="text-[12px] text-tyba-text-faint">
            {session.worktree?.branch}
          </DialogDescription>
        </DialogHeader>

        {!ready ? (
          <>
            <div className="rounded-[6px] border border-tyba-amber/30 bg-tyba-amber/[.06] p-3 text-[12px] text-tyba-text">
              <div className="font-medium text-tyba-amber">
                {status.installed
                  ? t("prDialogCliUnauthTitle", { cli: status.cli })
                  : t("prDialogCliMissingTitle", { cli: status.cli })}
              </div>
              <div className="pt-1 text-tyba-text-muted">
                {status.installed
                  ? t("prDialogCliUnauthBody", { cli: status.cli })
                  : t("prDialogCliMissingBody", { cli: status.cli })}
              </div>
            </div>
            {!status.web_create_url && (
              <div className="text-[12px] text-tyba-text-faint">
                {t("prDialogFallbackNoUrl")}
              </div>
            )}
            {error && (
              <div className="rounded-[4px] border border-tyba-red/40 bg-tyba-red/10 p-2 text-[12px] text-tyba-red">
                {error}
              </div>
            )}
            <div className="flex justify-end gap-2">
              <Button variant="ghost" size="sm" onClick={onClose}>
                {t("cancel")}
              </Button>
              <Button
                size="sm"
                className="gap-1.5"
                disabled={pushing || !status.web_create_url}
                onClick={() => void pushAndOpenWeb()}
              >
                <ArrowSquareOut size={13} />
                {pushing
                  ? t("prDialogFallbackPushing")
                  : t("prDialogFallbackAction")}
              </Button>
            </div>
          </>
        ) : (
          <>
            <div className="flex flex-col gap-1.5">
              <span className="tyba-label">{t("prDialogFieldTitle")}</span>
              <Input
                autoFocus
                value={title}
                onChange={(e) => setTitle(e.target.value)}
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <span className="tyba-label">{t("prDialogFieldBody")}</span>
              <textarea
                value={body}
                onChange={(e) => setBody(e.target.value)}
                rows={6}
                className="max-h-64 min-h-24 resize-y rounded-[4px] border border-tyba-border bg-transparent px-2 py-1.5 font-mono text-[12px] text-tyba-text outline-none placeholder:text-tyba-text-faint focus:border-tyba-green/50"
              />
            </div>

            {error && (
              <div className="rounded-[4px] border border-tyba-red/40 bg-tyba-red/10 p-2 text-[12px] text-tyba-red">
                {error}
              </div>
            )}

            <div className="flex justify-end gap-2">
              <Button variant="ghost" size="sm" onClick={onClose} disabled={busy}>
                {t("cancel")}
              </Button>
              <Button
                size="sm"
                disabled={!title.trim() || busy}
                onClick={() => void submit()}
              >
                {busy
                  ? t("prDialogCreating")
                  : isMr
                    ? t("prDialogSubmitMr")
                    : t("prDialogSubmitPr")}
              </Button>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
