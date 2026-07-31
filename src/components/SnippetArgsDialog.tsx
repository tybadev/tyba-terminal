import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { Snippet, SnippetPlaceholder } from "@/lib/ipc";

interface Props {
  snippet: Snippet;
  placeholders: SnippetPlaceholder[];
  onCancel: () => void;
  onConfirm: (values: [string, string][]) => void;
}

export function SnippetArgsDialog({
  snippet,
  placeholders,
  onCancel,
  onConfirm,
}: Props) {
  const { t } = useTranslation();
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(
      placeholders.map((p) => [p.name, p.default ?? ""] as const),
    ),
  );

  useEffect(() => {
    setValues(
      Object.fromEntries(
        placeholders.map((p) => [p.name, p.default ?? ""] as const),
      ),
    );
  }, [placeholders]);

  const confirm = () =>
    onConfirm(placeholders.map((p) => [p.name, values[p.name] ?? ""]));

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="max-w-[480px] gap-0 rounded-[6px] border-tyba-border-strong bg-tyba-surface p-0 shadow-2xl">
        <DialogHeader className="border-b border-tyba-border px-4 py-3">
          <DialogTitle className="text-[13px] text-tyba-text">
            {snippet.name}
          </DialogTitle>
          <DialogDescription className="font-mono text-[11px] text-tyba-text-faint">
            {snippet.command}
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-2 px-4 py-3">
          {placeholders.map((placeholder, index) => (
            <label key={placeholder.name} className="flex flex-col gap-1">
              <span className="text-[11px] text-tyba-text-muted">
                {placeholder.name}
              </span>
              <input
                autoFocus={index === 0}
                value={values[placeholder.name] ?? ""}
                onChange={(e) =>
                  setValues((prev) => ({
                    ...prev,
                    [placeholder.name]: e.target.value,
                  }))
                }
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    confirm();
                  }
                }}
                className="rounded-[4px] border border-tyba-border bg-transparent px-2 py-1 font-mono text-[12px] text-tyba-text outline-none focus:border-tyba-green/50"
              />
            </label>
          ))}
        </div>
        <p className="border-t border-tyba-border px-4 py-2 text-[11px] text-tyba-text-faint">
          {t("snippetPreview")}
        </p>
        <div className="flex items-center justify-end gap-2 px-4 py-3">
          <Button variant="ghost" size="sm" onClick={onCancel}>
            {t("cancel")}
          </Button>
          <Button size="sm" onClick={confirm}>
            {t("snippetInsert")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
