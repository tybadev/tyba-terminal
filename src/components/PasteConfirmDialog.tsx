import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { hasUnsafeControlChars } from "@/lib/clipboard";

const CONTROL_CHAR = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g;

interface Props {
  text: string | null;
  onCancel: () => void;
  onConfirm: (mode: "raw" | "single") => void;
}

export function PasteConfirmDialog({ text, onCancel, onConfirm }: Props) {
  const { t } = useTranslation();
  if (text === null) return null;

  const lines = text.split(/\r\n|\n|\r/);
  const visible = text.replace(CONTROL_CHAR, "␣");
  const hasControls = hasUnsafeControlChars(text);

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="max-w-[520px] gap-0 rounded-[6px] border-tyba-border-strong bg-tyba-surface p-0 shadow-2xl">
        <DialogHeader className="border-b border-tyba-border px-4 py-3">
          <DialogTitle className="text-[13px] text-tyba-text">
            {t("pasteMultilineTitle")}
          </DialogTitle>
          <DialogDescription className="text-[12px] text-tyba-text-faint">
            {t("pasteMultilineBody", { count: lines.length })}
          </DialogDescription>
        </DialogHeader>
        {hasControls && (
          <p className="border-b border-tyba-border bg-tyba-amber-tint px-4 py-2 text-[11px] text-tyba-amber">
            {t("pasteControlChars")}
          </p>
        )}
        <pre className="max-h-52 overflow-auto whitespace-pre-wrap break-all border-b border-tyba-border bg-tyba-sunken px-4 py-3 font-mono text-[12px] text-tyba-text">
          {visible}
        </pre>
        <div className="flex items-center justify-end gap-2 px-4 py-3">
          <Button variant="ghost" size="sm" onClick={onCancel}>
            {t("cancel")}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => onConfirm("single")}>
            {t("pasteAsSingleLine")}
          </Button>
          <Button size="sm" onClick={() => onConfirm("raw")}>
            {t("pasteConfirm")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
