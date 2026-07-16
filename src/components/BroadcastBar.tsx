import { useTranslation } from "react-i18next";
import { Broadcast, Check, WarningCircle } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

export interface BroadcastTarget {
  sessionId: string;
  alias: string;
  color: string | null;
}

interface BroadcastBarProps {
  targets: BroadcastTarget[];
  enabled: boolean;
  selected: string[];
  onToggle: (on: boolean) => void;
  onSelectedChange: (ids: string[]) => void;
}

export function BroadcastBar({
  targets,
  enabled,
  selected,
  onToggle,
  onSelectedChange,
}: BroadcastBarProps) {
  const { t } = useTranslation();
  const selectedSet = new Set(selected);
  const count = selected.length;

  const toggleTarget = (sessionId: string) => {
    if (selectedSet.has(sessionId)) {
      onSelectedChange(selected.filter((id) => id !== sessionId));
    } else {
      onSelectedChange([...selected, sessionId]);
    }
  };

  const selectAll = () => onSelectedChange(targets.map((target) => target.sessionId));
  const selectNone = () => onSelectedChange([]);

  if (!enabled) {
    return (
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-tyba-border bg-tyba-surface px-3">
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              aria-pressed={false}
              aria-label={t("broadcastEnable")}
              onClick={() => onToggle(true)}
              className="flex items-center gap-1.5 rounded-[4px] px-2 py-1 text-[11px] text-tyba-text-faint transition-colors hover:bg-tyba-text/[.05] hover:text-tyba-text-muted"
            >
              <Broadcast size={13} />
              {t("broadcastToggle")}
            </button>
          </TooltipTrigger>
          <TooltipContent side="bottom">{t("broadcastEnableHint")}</TooltipContent>
        </Tooltip>
      </div>
    );
  }

  return (
    <div className="flex min-h-9 shrink-0 flex-wrap items-center gap-2 border-b border-tyba-amber/50 bg-tyba-amber-tint px-3 py-1.5">
      <button
        type="button"
        aria-pressed={true}
        aria-label={t("broadcastDisable")}
        onClick={() => onToggle(false)}
        className="flex shrink-0 items-center gap-1.5 rounded-[4px] border border-tyba-amber/60 bg-tyba-amber/15 px-2 py-1 text-[11px] font-semibold text-tyba-amber transition-colors hover:bg-tyba-amber/25"
      >
        <Broadcast size={14} weight="fill" />
        {t("broadcastActive", { count })}
      </button>

      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5">
        {targets.map((target) => {
          const checked = selectedSet.has(target.sessionId);
          return (
            <button
              key={target.sessionId}
              type="button"
              role="checkbox"
              aria-checked={checked}
              aria-label={target.alias}
              onClick={() => toggleTarget(target.sessionId)}
              className={cn(
                "flex shrink-0 items-center gap-1.5 rounded-[4px] border px-2 py-1 text-[11px] transition-colors",
                checked
                  ? "border-tyba-amber/60 bg-tyba-surface text-tyba-text"
                  : "border-tyba-border-strong bg-transparent text-tyba-text-faint opacity-60 hover:opacity-100",
              )}
            >
              <span
                className="size-1.5 shrink-0 rounded-full"
                style={{
                  background: target.color
                    ? `var(--tyba-${target.color})`
                    : "var(--tyba-text-faint)",
                }}
              />
              <span className="max-w-[120px] truncate">{target.alias}</span>
              {checked && <Check size={11} weight="bold" className="shrink-0" />}
            </button>
          );
        })}
      </div>

      <div className="flex shrink-0 items-center gap-1.5">
        <button
          type="button"
          onClick={selectAll}
          className="rounded-[4px] px-1.5 py-1 text-[11px] text-tyba-text-faint transition-colors hover:text-tyba-text"
        >
          {t("broadcastSelectAll")}
        </button>
        <span className="text-tyba-text-faint">·</span>
        <button
          type="button"
          onClick={selectNone}
          className="rounded-[4px] px-1.5 py-1 text-[11px] text-tyba-text-faint transition-colors hover:text-tyba-text"
        >
          {t("broadcastSelectNone")}
        </button>
      </div>
    </div>
  );
}

interface BroadcastConfirmDialogProps {
  open: boolean;
  command: string;
  targets: number;
  onConfirm: () => void;
  onCancel: () => void;
}

export function BroadcastConfirmDialog({
  open,
  command,
  targets,
  onConfirm,
  onCancel,
}: BroadcastConfirmDialogProps) {
  const { t } = useTranslation();

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onCancel()}>
      <DialogContent className="max-w-[480px] gap-0 rounded-[6px] border-tyba-red/50 bg-tyba-surface p-0 shadow-2xl">
        <DialogHeader className="gap-1.5 border-b border-tyba-border px-4 py-3">
          <DialogTitle className="flex items-center gap-2 text-[14px] text-tyba-text">
            <WarningCircle size={18} weight="fill" className="shrink-0 text-tyba-red" />
            {t("broadcastConfirmTitle")}
          </DialogTitle>
          <DialogDescription className="text-[12px] text-tyba-text-faint">
            {t("broadcastConfirmBody", { count: targets })}
          </DialogDescription>
        </DialogHeader>

        <div className="flex items-center gap-2 border-b border-tyba-border bg-tyba-red-tint px-4 py-2.5">
          <span className="rounded-[4px] border border-tyba-red/50 bg-tyba-red/15 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-[0.08em] text-tyba-red">
            {t("riskRed")}
          </span>
          <span className="text-[13px] font-semibold text-tyba-red">
            {t("broadcastConfirmCount", { count: targets })}
          </span>
        </div>

        <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-all border-b border-tyba-border bg-tyba-sunken px-4 py-3 font-mono text-[12px] text-tyba-text">
          {command}
        </pre>

        <div className="flex items-center justify-end gap-2 px-4 py-3">
          <Button variant="ghost" size="sm" autoFocus onClick={onCancel}>
            {t("cancel")}
          </Button>
          <Button variant="destructive" size="sm" onClick={onConfirm}>
            {t("broadcastConfirmAction")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
