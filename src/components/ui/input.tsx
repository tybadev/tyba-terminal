import * as React from "react"

import { cn } from "@/lib/utils"
import { Kbd } from "@/components/ui/kbd"

// Campo do Muriki (`.mk-input`): raio 10px, vidro forte (.8) com a aresta
// branca e desfoque de 12px no claro do TYBA, o fio fino por fora e o anel da
// marca com o halo no foco. Nos outros temas o fallback é o fundo de controle
// de antes. Também é a pele do Textarea e do gatilho do Select.
export const FIELD_SURFACE =
  "tyba-mk-filter-sm rounded-[10px] border border-[var(--tyba-mk-glass-edge,transparent)] bg-[var(--tyba-mk-glass-strong,var(--tyba-control-bg))] text-sm font-medium text-tyba-text [box-shadow:0_0_0_1px_var(--tyba-control-line)] transition-[color,box-shadow] duration-150 outline-none selection:bg-primary selection:text-primary-foreground placeholder:text-tyba-text-faint disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-45 focus-visible:[box-shadow:var(--tyba-field-focus)] aria-invalid:[box-shadow:0_0_0_1.5px_var(--tyba-red)]"

/** Rótulo do campo do Muriki (`.mk-label`): 13px/600 na cor do texto. */
export const FIELD_LABEL =
  "pl-0.5 text-[13px] leading-[18px] font-semibold text-tyba-text"

type InputProps = React.ComponentProps<"input"> & {
  icon?: React.ReactNode
  kbd?: string
  trail?: React.ReactNode
  /** Rótulo em cima do campo (Muriki `Input label`, `.mk-field`). */
  label?: React.ReactNode
}

// Com `label` (Muriki `Input label`): o rótulo em cima e o campo embaixo,
// 6px entre os dois. O `className` continua sendo do campo.
function Input({ label, id, ...props }: InputProps) {
  const autoId = React.useId()
  if (!label) return <InputField id={id} {...props} />
  const fieldId = id ?? autoId
  return (
    <div data-slot="field" className="flex min-w-0 flex-col gap-1.5">
      <label htmlFor={fieldId} className={FIELD_LABEL}>
        {label}
      </label>
      <InputField id={fieldId} {...props} />
    </div>
  )
}

// Com `icon` e/ou `kbd` (Muriki `Input icon kbd`): o campo vira a caixa —
// ícone à esquerda, a tecla embutida à direita, o anel no foco de dentro.
// O `className` então dimensiona a caixa, não o `<input>`. `trail` fica
// dentro da caixa, à direita do texto (contagem, botão de limpar).
function InputField({
  className,
  type,
  icon,
  kbd,
  trail,
  ...props
}: Omit<InputProps, "label">) {
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
