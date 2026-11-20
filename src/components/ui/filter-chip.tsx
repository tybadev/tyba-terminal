import * as React from "react"

import { cn } from "@/lib/utils"

// Chip de filtro do Muriki (`FilterChip`, `.mk-fchip`): pílula de 30px com
// 0 12px, 13px/500 e ícone de 14px. Apagado: vidro de controle com o fio fino.
// Aceso: o tom suave da marca com o fio da marca a 25%. Também é gatilho de
// menu (`asChild` do Radix), por isso repassa ref e props.
const FilterChip = React.forwardRef<
  HTMLButtonElement,
  React.ComponentProps<"button"> & { active?: boolean }
>(function FilterChip({ className, active = false, type = "button", ...props }, ref) {
  return (
    <button
      ref={ref}
      type={type}
      data-slot="filter-chip"
      data-active={active || undefined}
      className={cn(
        "inline-flex h-[30px] shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-full px-3 text-[13px] font-medium outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-45 [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-3.5",
        active
          ? "bg-tyba-blue/10 text-tyba-blue [box-shadow:inset_0_0_0_1px_color-mix(in_srgb,var(--tyba-blue)_25%,transparent)]"
          : "bg-[var(--tyba-control-bg)] text-tyba-text-muted [box-shadow:inset_0_0_0_1px_var(--tyba-control-line),var(--tyba-control-shadow)] hover:text-tyba-text",
        className
      )}
      {...props}
    />
  )
})

export { FilterChip }
