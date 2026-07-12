import { CaretUpDown, Check } from "@phosphor-icons/react";

import { cn } from "@/lib/utils";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

export interface SelectOption {
  value: string;
  label: string;
}

export function Select({
  value,
  options,
  onChange,
  className,
}: {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  className?: string;
}) {
  const current = options.find((o) => o.value === value);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          className={cn(
            "flex h-8 items-center justify-between gap-2 rounded-[4px] border border-tyba-border bg-tyba-text/[.02] px-2.5 text-[13px] text-tyba-text transition-colors outline-none hover:border-tyba-border-strong focus-visible:[box-shadow:var(--tyba-focus-ring)]",
            className,
          )}
        >
          <span className="truncate">{current?.label ?? value}</span>
          <CaretUpDown size={13} className="shrink-0 text-tyba-text-faint" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className="max-h-64 min-w-[var(--radix-dropdown-menu-trigger-width)]"
      >
        {options.map((o) => (
          <DropdownMenuItem
            key={o.value}
            onSelect={() => onChange(o.value)}
            className="justify-between gap-3 text-xs"
          >
            {o.label}
            {o.value === value && (
              <Check size={12} weight="bold" className="text-tyba-green" />
            )}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
