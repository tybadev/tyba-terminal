import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

const PREVIEW_LINES = 8;

interface Props {
  text: string | null;
  onCancel: () => void;
  onConfirm: (mode: "raw" | "single") => void;
}

export function PasteConfirmDialog({ text, onCancel, onConfirm }: Props) {
  const { t } = useTranslation();
  if (text === null) return null;

  const lines = text.split(/\r\n|\n|\r/);
  const preview = lines.slice(0, PREVIEW_LINES);
  const hidden = lines.length - preview.length;

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
        <pre className="max-h-52 overflow-auto whitespace-pre-wrap break-all border-b border-tyba-border bg-tyba-sunken px-4 py-3 font-mono text-[12px] text-tyba-text">
          {preview.join("\n")}
          {hidden > 0 ? `\n… +${hidden}` : ""}
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
