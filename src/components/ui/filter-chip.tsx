import * as React from "react"
import { X } from "@phosphor-icons/react"
import { useTranslation } from "react-i18next"

import { cn } from "@/lib/utils"

// Chip de filtro do Muriki (`FilterChip`, `.mk-fchip`): pílula de 30px com
// 0 12px, 13px/500 e ícone de 14px. Apagado: vidro .46 com a aresta branca e
// a sombra do botão de vidro, desfoque de 12px (no claro do TYBA; nos outros
// temas o fallback é o fundo de controle com o fio fino de antes). Aceso: o
// azul suave da marca com o fio da marca a 25%. Também é gatilho de menu
// (`asChild` do Radix), por isso repassa ref e props.
//
// Com `label` (Muriki `FilterChip icon label value active`): ícone, rótulo,
// o valor em negrito e, aceso, o X de 12px. Sem `label`, só os filhos.
const FilterChip = React.forwardRef<
  HTMLButtonElement,
  React.ComponentProps<"button"> & {
    active?: boolean
    icon?: React.ReactNode
    label?: React.ReactNode
    value?: React.ReactNode
  }
>(function FilterChip(
  { className, active = false, type = "button", icon, label, value, children, ...props },
  ref,
) {
  const { t } = useTranslation()
  return (
    <button
      ref={ref}
      type={type}
      data-slot="filter-chip"
      data-active={active || undefined}
      className={cn(
        "inline-flex h-[30px] shrink-0 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-full px-3 text-[13px] font-medium outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-45 [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-3.5 [&_b]:font-semibold",
        active
          ? "bg-[var(--tyba-brand-soft,color-mix(in_oklab,var(--tyba-blue)_10%,transparent))] text-tyba-blue [box-shadow:inset_0_0_0_1px_color-mix(in_srgb,var(--tyba-blue)_25%,transparent)] [&_b]:text-tyba-blue"
          : "tyba-mk-filter-sm bg-[var(--tyba-mk-glass-button,var(--tyba-control-bg))] text-tyba-text-muted [box-shadow:var(--tyba-mk-shadow-chip,inset_0_0_0_1px_var(--tyba-control-line),var(--tyba-control-shadow))] hover:bg-[var(--tyba-mk-glass-hover,var(--tyba-control-bg))] hover:text-tyba-text [&_b]:text-tyba-text",
        className
      )}
      {...props}
    >
      {label === undefined ? (
        children
      ) : (
        <>
          {icon}
          <span>{label}</span>
          {value !== undefined && <b>{value}</b>}
          {active && (
            <span
              aria-label={t("filterRemove")}
              className="-mr-1 grid place-items-center opacity-70"
            >
              <X size={12} />
            </span>
          )}
        </>
      )}
    </button>
  )
})

export { FilterChip }
