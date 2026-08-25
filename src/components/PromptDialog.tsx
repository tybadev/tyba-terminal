import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  placeholder?: string;
  initial?: string;
  onSubmit: (value: string) => void;
}

export function PromptDialog({
  open,
  onOpenChange,
  title,
  placeholder,
  initial = "",
  onSubmit,
}: Props) {
  const { t } = useTranslation();
  const [value, setValue] = useState(initial);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setValue(initial);
      requestAnimationFrame(() => inputRef.current?.select());
    }
  }, [open, initial]);

  const submit = () => {
    onOpenChange(false);
    onSubmit(value.trim());
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        showCloseButton={false}
        className="top-28 max-w-[420px] translate-y-0 gap-0 rounded-[6px] border-tyba-border-strong bg-tyba-surface p-0 shadow-2xl"
      >
        <DialogHeader className="sr-only">
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{title}</DialogDescription>
        </DialogHeader>
        <div className="flex h-10 items-center gap-2.5 tyba-divide-b px-3">
          <span
            aria-hidden
            className="shrink-0 font-mono text-[13px] font-bold text-tyba-green"
          >
            ❯
          </span>
          <span className="shrink-0 text-[12px] text-tyba-text-faint">
            {title}
          </span>
          <input
            ref={inputRef}
            value={value}
            placeholder={placeholder}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submit();
            }}
            className="w-full bg-transparent font-mono text-[13px] text-tyba-text outline-none placeholder:text-tyba-text-faint"
          />
        </div>
        <div className="flex items-center gap-4 px-3 py-1.5 font-mono text-[10px] text-tyba-text-faint">
          <span>↵ {t("hintRun")}</span>
          <span className="ml-auto">esc {t("hintClose")}</span>
        </div>
      </DialogContent>
    </Dialog>
  );
}
