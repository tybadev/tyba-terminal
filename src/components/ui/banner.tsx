import * as React from "react"
import { Info, Warning, WarningCircle } from "@phosphor-icons/react"

import { cn } from "@/lib/utils"

// Faixa de aviso do Muriki (`Banner`, `.mk-banner`, quadro Main): 12 16 de
// folga, raio 10, 14px, ícone de 18 no tom; o título em 600 e o texto
// seguido na mesma linha; a ação (um Button) fica à direita.
const TONE = {
  warning: {
    box: "bg-tyba-amber-tint shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--tyba-amber)_25%,transparent)]",
    icon: "text-tyba-amber",
    Icon: Warning,
  },
  error: {
    box: "bg-tyba-red-tint shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--tyba-red)_25%,transparent)]",
    icon: "text-tyba-red",
    Icon: WarningCircle,
  },
  info: {
    box: "bg-tyba-blue-tint",
    icon: "text-tyba-blue",
    Icon: Info,
  },
} as const

function Banner({
  tone = "info",
  title,
  action,
  close,
  className,
  children,
  ...props
}: React.ComponentProps<"div"> & {
  tone?: keyof typeof TONE
  title?: React.ReactNode
  action?: React.ReactNode
  /** O X de dispensar, depois da ação. */
  close?: React.ReactNode
}) {
  const { box, icon, Icon } = TONE[tone]
  return (
    <div
      role={tone === "info" ? "status" : "alert"}
      data-slot="banner"
      className={cn(
        "flex items-center gap-3 rounded-[10px] px-4 py-3 text-[14px] text-tyba-text",
        box,
        className,
      )}
      {...props}
    >
      <Icon aria-hidden size={18} className={cn("shrink-0", icon)} />
      <span className="min-w-0 flex-1">
        {title && <b className="font-semibold">{title}</b>} {children}
      </span>
      {action}
      {close}
    </div>
  )
}

export { Banner }
