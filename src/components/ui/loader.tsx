import * as React from "react"

import { cn } from "@/lib/utils"

// Carregando do Muriki (`Loader`, `.mk-loader`, quadro Main): trilho de vidro
// 240×16 em pílula, a gota azul de 56 com o ponto âmbar deslizando e o
// rótulo 13/600 apagado embaixo. Parado em `prefers-reduced-motion`.
function Loader({
  label,
  className,
  ...props
}: React.ComponentProps<"div"> & { label?: React.ReactNode }) {
  return (
    <div
      role="status"
      data-slot="loader"
      className={cn("inline-flex flex-col items-center gap-4", className)}
      {...props}
    >
      <span
        aria-hidden
        className="tyba-mk-glass relative h-4 w-60 overflow-hidden rounded-full bg-[var(--tyba-mk-glass-strong,var(--tyba-raised))] shadow-[inset_0_0_0_1px_var(--tyba-line)]"
      >
        <span className="tyba-loader-drop absolute top-0.5 bottom-0.5 w-14 rounded-full bg-tyba-blue shadow-[0_1px_4px_color-mix(in_srgb,var(--tyba-blue)_25%,transparent)] after:absolute after:top-0.5 after:right-1.5 after:size-1.5 after:rounded-full after:bg-tyba-amber-fill after:content-['']" />
      </span>
      {label && <span className="text-[13px] font-semibold text-tyba-text-muted">{label}</span>}
    </div>
  )
}

export { Loader }
