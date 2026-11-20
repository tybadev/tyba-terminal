import * as React from "react"

import { cn } from "@/lib/utils"

// Campo do Muriki: raio 10px, fundo de controle, o fio fino por fora e o
// anel da marca no foco. Também é a pele do Textarea e do gatilho do Select.
export const FIELD_SURFACE =
  "rounded-[10px] border border-transparent bg-[var(--tyba-control-bg)] text-sm font-medium text-tyba-text [box-shadow:0_0_0_1px_var(--tyba-control-line)] transition-[color,box-shadow] duration-150 outline-none selection:bg-primary selection:text-primary-foreground placeholder:text-tyba-text-faint disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-45 focus-visible:[box-shadow:var(--tyba-field-focus)] aria-invalid:[box-shadow:0_0_0_1.5px_var(--tyba-red)]"

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      autoCapitalize="off"
      autoCorrect="off"
      spellCheck={false}
      type={type}
      data-slot="input"
      className={cn(
        FIELD_SURFACE,
        "h-10 w-full min-w-0 py-1 pr-3 pl-4 file:inline-flex file:h-7 file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-foreground",
        className
      )}
      {...props}
    />
  )
}

export { Input }
