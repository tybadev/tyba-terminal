import type * as React from "react";
import { CaretDown, Check } from "@phosphor-icons/react";

import { cn } from "@/lib/utils";
import { FIELD_SURFACE } from "@/components/ui/input";
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
  icon,
  disabled,
  id,
  "aria-label": ariaLabel,
}: {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  className?: string;
  /** Ícone à esquerda do valor (Muriki `Select icon`). */
  icon?: React.ReactNode;
  disabled?: boolean;
  id?: string;
  "aria-label"?: string;
}) {
  const current = options.find((o) => o.value === value);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          id={id}
          disabled={disabled}
          aria-label={ariaLabel}
          className={cn(
            FIELD_SURFACE,
            "flex h-10 cursor-pointer items-center justify-between gap-2 pr-3 pl-4 data-[state=open]:[box-shadow:var(--tyba-field-focus)]",
            className,
          )}
        >
          {icon && <span className="flex shrink-0 text-tyba-text-faint">{icon}</span>}
          <span className="min-w-0 flex-1 truncate text-left">{current?.label ?? value}</span>
          <CaretDown size={14} className="shrink-0 text-tyba-text-faint" />
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
              <Check size={12} weight="bold" className="text-primary" />
            )}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
