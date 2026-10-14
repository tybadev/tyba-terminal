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
      {/* Altura limitada: título e botões ficam sempre à vista e só o
          detalhe rola. Em confirmação destrutiva o foco começa em Cancelar,
          para um Enter distraído não confirmar. */}
      <DialogContent
        className="flex max-h-[80vh] max-w-[440px] flex-col border-tyba-border-strong bg-tyba-surface"
        showCloseButton={false}
      >
        <DialogHeader className="shrink-0">
          <DialogTitle>{request.title}</DialogTitle>
        </DialogHeader>
        {request.detail && (
          <DialogDescription asChild>
            <div className="min-h-0 overflow-y-auto whitespace-pre-wrap break-words rounded-md bg-tyba-sunken px-3 py-2">
              {request.detail}
            </div>
          </DialogDescription>
        )}
        <div className="flex shrink-0 justify-end gap-2">
          <Button
            autoFocus={request.destructive}
            variant="ghost"
            onClick={() => answerConfirm(request.id, false)}
          >
            {request.cancelLabel ?? t("cancel")}
          </Button>
          <Button
            autoFocus={!request.destructive}
            variant={request.destructive ? "destructive" : "default"}
            onClick={() => answerConfirm(request.id, true)}
          >
            {request.confirmLabel ?? t("confirm")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
