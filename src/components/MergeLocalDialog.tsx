import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { GitMerge, WarningCircle } from "@phosphor-icons/react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import {
  worktreeMergeIntoBase,
  worktreeMergePreview,
  type MergePreview,
  type MergeStrategy,
  type Session,
} from "../lib/ipc";
import { mergeGate } from "../lib/forge";

interface Props {
  session: Session;
  open: boolean;
  onClose: () => void;
  onMerged: () => void;
}

export function MergeLocalDialog({ session, open, onClose, onMerged }: Props) {
  const { t } = useTranslation();
  const [preview, setPreview] = useState<MergePreview | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [strategy, setStrategy] = useState<MergeStrategy>("squash");
  const [armed, setArmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setPreview(null);
    setLoadError(null);
    setArmed(false);
    setBusy(false);
    setError(null);
    void worktreeMergePreview(session.id)
      .then(setPreview)
      .catch((e) => setLoadError(String(e)));
  }, [open, session.id]);

  if (!open) return null;

  const gate = preview ? mergeGate(preview) : { enabled: false, reason: null };

  const blockedMessage = () => {
    if (!preview || gate.enabled) return null;
    if (gate.reason === "base_dirty") return t("mergeDialogBlockedDirty");
    if (gate.reason === "conflicts") {
      return t("mergeDialogBlockedConflicts", {
        files: preview.conflicts.join(", "),
      });
    }
    return t("mergeDialogBlockedNothing");
  };

  const confirm = async () => {
    if (!armed) {
      setArmed(true);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await worktreeMergeIntoBase(session.id, strategy);
      onMerged();
    } catch (e) {
      setError(String(e));
      setBusy(false);
      setArmed(false);
    }
  };

  return (
    <Dialog open onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="max-w-[480px] border-tyba-border-strong bg-tyba-surface">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-[14px]">
            <GitMerge size={16} className="text-tyba-red" />
            {t("mergeDialogTitle")}
            <span className="rounded-full border border-tyba-red/40 px-1.5 py-0.5 text-[10px] font-normal text-tyba-red">
              {t("redAction")}
            </span>
          </DialogTitle>
          {preview && (
            <DialogDescription className="font-mono text-[12px] text-tyba-text-faint">
              {t("mergeDialogSummary", {
                source: preview.source_branch,
                base: preview.base_branch,
              })}
            </DialogDescription>
          )}
        </DialogHeader>

        {loadError && (
          <div className="rounded-[4px] border border-tyba-red/40 bg-tyba-red/10 p-2 text-[12px] text-tyba-red">
            {loadError}
          </div>
        )}

        {!preview && !loadError && (
          <div className="p-2 text-[12px] text-tyba-text-faint">
            {t("mergeDialogLoading")}
          </div>
        )}

        {preview && (
          <>
            <div className="flex items-center gap-3 rounded-[6px] border border-tyba-border p-3 font-mono text-[12px] text-tyba-text">
              <span>{t("mergeDialogCommits", { count: preview.commits })}</span>
              <span className="text-tyba-text-faint">·</span>
              <span>
                {t("mergeDialogFiles", { count: preview.files_changed })}
              </span>
            </div>

            <div className="flex flex-col gap-1.5">
              <span className="tyba-label">{t("mergeDialogStrategy")}</span>
              <div className="flex gap-2">
                <button
                  type="button"
                  aria-pressed={strategy === "squash"}
                  onClick={() => setStrategy("squash")}
                  className={`flex-1 rounded-[6px] border p-2 text-left text-[12px] transition-colors ${
                    strategy === "squash"
                      ? "border-tyba-green/50 bg-tyba-green/10 text-tyba-text"
                      : "border-tyba-border text-tyba-text-muted hover:bg-white/[.03]"
                  }`}
                >
                  <div className="font-medium">{t("mergeDialogSquash")}</div>
                  <div className="text-[11px] text-tyba-text-faint">
                    {t("mergeDialogSquashHint")}
                  </div>
                </button>
                <button
                  type="button"
                  aria-pressed={strategy === "merge_commit"}
                  onClick={() => setStrategy("merge_commit")}
                  className={`flex-1 rounded-[6px] border p-2 text-left text-[12px] transition-colors ${
                    strategy === "merge_commit"
                      ? "border-tyba-green/50 bg-tyba-green/10 text-tyba-text"
                      : "border-tyba-border text-tyba-text-muted hover:bg-white/[.03]"
                  }`}
                >
                  <div className="font-medium">
                    {t("mergeDialogMergeCommit")}
                  </div>
                  <div className="text-[11px] text-tyba-text-faint">
                    {t("mergeDialogMergeCommitHint")}
                  </div>
                </button>
              </div>
            </div>

            {!gate.enabled && (
              <div className="flex items-start gap-2 rounded-[6px] border border-tyba-yellow/30 bg-tyba-yellow/[.06] p-2.5 text-[12px] text-tyba-yellow">
                <WarningCircle size={14} className="mt-0.5 shrink-0" />
                <span>{blockedMessage()}</span>
              </div>
            )}
          </>
        )}

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
            disabled={!preview || !gate.enabled || busy}
            onClick={() => void confirm()}
            className={
              armed ? "bg-tyba-red text-white hover:bg-tyba-red/90" : undefined
            }
          >
            {busy
              ? t("mergeDialogMerging")
              : armed
                ? t("mergeDialogConfirmArm")
                : t("mergeDialogConfirm")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
