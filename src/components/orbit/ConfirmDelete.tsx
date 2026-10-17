import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

/**
 * Pergunta antes de apagar grafo ou integração. Quem recusa é o core
 * (execução aberta, integração em uso): o motivo aparece aqui e o diálogo fica.
 */
export function ConfirmDelete({
  title,
  body,
  onConfirm,
  onClose,
}: {
  title: string;
  body: string;
  onConfirm: () => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const confirm = () => {
    setBusy(true);
    setError(null);
    onConfirm()
      .then(onClose)
      .catch((e) => setError(String(e)))
      .finally(() => setBusy(false));
  };
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-[440px] gap-0 rounded-[6px] border-tyba-border-strong bg-tyba-surface p-0 font-mono shadow-2xl">
        <DialogHeader className="tyba-divide-b px-4 py-3">
          <DialogTitle className="text-[13px] text-tyba-text">{title}</DialogTitle>
          <DialogDescription className="text-[12px] text-tyba-text-faint">{body}</DialogDescription>
        </DialogHeader>
        {error && (
          <p role="alert" className="tyba-divide-b bg-tyba-red-tint px-4 py-2 text-[11.5px] text-tyba-red">
            {error}
          </p>
        )}
        <div className="flex items-center justify-end gap-2 px-4 py-3">
          <Button variant="ghost" size="sm" onClick={onClose}>
            {t("orbitDeleteCancel")}
          </Button>
          <Button variant="destructive" size="sm" disabled={busy} onClick={confirm}>
            {t("orbitDeleteConfirm")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
