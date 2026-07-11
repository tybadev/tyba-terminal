import { useState } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle } from "@phosphor-icons/react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { disposeSession, worktreeRemove, type Session } from "../lib/ipc";

interface Props {
  session: Session;
  kind: "pr" | "merge";
  open: boolean;
  onClose: () => void;
  onRemoved: () => void;
}

export function PostDeliveryDialog({
  session,
  kind,
  open,
  onClose,
  onRemoved,
}: Props) {
  const { t } = useTranslation();
  const [armed, setArmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!open) return null;

  const remove = async () => {
    const path = session.worktree?.path;
    if (!path || busy) return;
    if (!armed) {
      setArmed(true);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await disposeSession(session.id);
      await worktreeRemove(path, true, true);
      onRemoved();
    } catch (e) {
      setError(String(e));
      setBusy(false);
      setArmed(false);
    }
  };

  return (
    <Dialog
      open
      onOpenChange={(next) => {
        if (!next) {
          setArmed(false);
          onClose();
        }
      }}
    >
      <DialogContent className="max-w-[440px] border-tyba-border-strong bg-tyba-surface">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-[14px]">
            <CheckCircle size={16} className="text-tyba-green" />
            {t("postDeliveryTitle")}
          </DialogTitle>
          <DialogDescription className="text-[12px] text-tyba-text-faint">
            {kind === "pr"
              ? t("postDeliveryBodyPr")
              : t("postDeliveryBodyMerge")}
          </DialogDescription>
        </DialogHeader>

        {error && (
          <div className="rounded-[4px] border border-tyba-red/40 bg-tyba-red/10 p-2 text-[12px] text-tyba-red">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setArmed(false);
              onClose();
            }}
            disabled={busy}
          >
            {t("postDeliveryKeep")}
          </Button>
          <Button
            size="sm"
            disabled={busy}
            onClick={() => void remove()}
            className={
              armed
                ? "bg-tyba-red text-white hover:bg-tyba-red/90"
                : "border border-tyba-red/40 bg-transparent text-tyba-red hover:bg-tyba-red/10"
            }
          >
            {busy
              ? t("postDeliveryRemoving")
              : armed
                ? t("worktreeRemoveConfirm")
                : t("postDeliveryRemove")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
