import * as React from "react"

import { cn } from "@/lib/utils"
import { Kbd } from "@/components/ui/kbd"

// Campo do Muriki: raio 10px, fundo de controle, o fio fino por fora e o
// anel da marca no foco. Também é a pele do Textarea e do gatilho do Select.
export const FIELD_SURFACE =
  "rounded-[10px] border border-transparent bg-[var(--tyba-control-bg)] text-sm font-medium text-tyba-text [box-shadow:0_0_0_1px_var(--tyba-control-line)] transition-[color,box-shadow] duration-150 outline-none selection:bg-primary selection:text-primary-foreground placeholder:text-tyba-text-faint disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-45 focus-visible:[box-shadow:var(--tyba-field-focus)] aria-invalid:[box-shadow:0_0_0_1.5px_var(--tyba-red)]"

// Com `icon` e/ou `kbd` (Muriki `Input icon kbd`): o campo vira a caixa —
// ícone à esquerda, a tecla embutida à direita, o anel no foco de dentro.
// O `className` então dimensiona a caixa, não o `<input>`. `trail` fica
// dentro da caixa, à direita do texto (contagem, botão de limpar).
function Input({
  className,
  type,
  icon,
  kbd,
  trail,
  ...props
}: React.ComponentProps<"input"> & {
  icon?: React.ReactNode
  kbd?: string
  trail?: React.ReactNode
}) {
  if (icon || kbd || trail) {
    return (
      <label
        data-slot="input"
        className={cn(
          FIELD_SURFACE,
          "flex h-10 w-full min-w-0 items-center gap-2 pr-3 pl-4 text-tyba-text-faint focus-within:[box-shadow:var(--tyba-field-focus)] [&>svg]:shrink-0",
          className
        )}
      >
        {icon}
        <input
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          type={type}
          className="min-w-0 flex-1 bg-transparent text-tyba-text outline-none placeholder:text-tyba-text-faint"
          {...props}
        />
        {trail}
        {kbd && <Kbd>{kbd}</Kbd>}
      </label>
    )
  }
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
