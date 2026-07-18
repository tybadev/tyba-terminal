import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import {
  answerConfirm,
  subscribeConfirm,
  type ConfirmRequest,
} from "@/lib/confirm";

export function ConfirmHost() {
  const { t } = useTranslation();
  const [request, setRequest] = useState<ConfirmRequest | null>(null);

  useEffect(() => subscribeConfirm(setRequest), []);

  if (!request) return null;

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) answerConfirm(request.id, false);
      }}
    >
      <DialogContent
        className="max-w-[440px] border-tyba-border-strong bg-tyba-surface"
        showCloseButton={false}
      >
        <DialogHeader>
          <DialogTitle>{request.title}</DialogTitle>
          {request.detail && (
            <DialogDescription>{request.detail}</DialogDescription>
          )}
        </DialogHeader>
        <div className="flex justify-end gap-2">
          <Button
            variant="ghost"
            onClick={() => answerConfirm(request.id, false)}
          >
            {t("cancel")}
          </Button>
          <Button
            autoFocus
            className={request.destructive ? "bg-tyba-red text-white" : ""}
            onClick={() => answerConfirm(request.id, true)}
          >
            {request.confirmLabel ?? t("confirm")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
