import * as React from "react";
import { CaretDown, Check } from "@phosphor-icons/react";

import { cn } from "@/lib/utils";
import { FIELD_LABEL, FIELD_SURFACE } from "@/components/ui/input";
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
  label,
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
  /** Rótulo em cima do campo (Muriki `Select label`, `.mk-field`), como no
   *  `Input`. O `className` continua sendo do campo. */
  label?: React.ReactNode;
  "aria-label"?: string;
}) {
  const autoId = React.useId();
  const current = options.find((o) => o.value === value);
  if (label) {
    const fieldId = id ?? autoId;
    return (
      <div data-slot="field" className="flex min-w-0 flex-col gap-1.5">
        <label htmlFor={fieldId} className={FIELD_LABEL}>
          {label}
        </label>
        <Select
          value={value}
          options={options}
          onChange={onChange}
          className={className}
          icon={icon}
          disabled={disabled}
          id={fieldId}
          aria-label={ariaLabel}
        />
      </div>
    );
  }
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
            // Item do Menu do Muriki (14px), como o resto dos menus.
            className="justify-between gap-3"
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
