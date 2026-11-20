import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { Slot } from "radix-ui"

import { cn } from "@/lib/utils"

// Botão do Muriki (quadro de componentes, `.mk-btn`): raio 10px, 600, aperta
// 2% no clique. default = primário líquido (`brand-glass` com desfoque e fio
// branco); outline/secondary = vidro (`glass-button` .46, aresta branca,
// `shadow-button-glass`); ghost = transparente; destructive = vidro com o
// texto em vermelho. No claro do TYBA tudo vem dos `--tyba-mk-*`; nos outros
// temas o fallback de cada um é o valor de antes (`--tyba-control-*`,
// `--primary`), então o escuro não muda.
// Tamanho padrão = `mk-btn` (40px, 0 20px, mínimo 88, ícone 18px); sm =
// `mk-btn-sm` (34px, 0 16px, mínimo 72, gap 8).
const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-2 rounded-[10px] border border-transparent text-sm font-semibold whitespace-nowrap transition-[transform,background-color,box-shadow,color] duration-150 outline-none active:scale-[.98] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-45 aria-invalid:outline-2 aria-invalid:outline-destructive [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default:
          "bg-[var(--tyba-mk-brand-glass,var(--primary))] text-primary-foreground border-[var(--tyba-mk-brand-edge,transparent)] tyba-mk-filter-sm-sat [box-shadow:var(--tyba-mk-shadow-button,var(--tyba-primary-shadow))] hover:bg-[var(--tyba-mk-brand-glass-hover,color-mix(in_srgb,var(--primary)_88%,black))] [&_kbd]:border-transparent! [&_kbd]:bg-white/20! [&_kbd]:text-primary-foreground! [&_kbd]:shadow-none!",
        destructive:
          "bg-[var(--tyba-mk-glass-button,var(--tyba-control-bg))] border-[var(--tyba-mk-glass-edge,transparent)] tyba-mk-filter-md [box-shadow:var(--tyba-mk-shadow-button-glass,inset_0_0_0_1px_var(--tyba-control-line),var(--tyba-control-shadow))] text-tyba-red hover:bg-tyba-red-tint focus-visible:outline-destructive",
        outline:
          "bg-[var(--tyba-mk-glass-button,var(--tyba-control-bg))] border-[var(--tyba-mk-glass-edge,transparent)] tyba-mk-filter-md [box-shadow:var(--tyba-mk-shadow-button-glass,inset_0_0_0_1px_var(--tyba-control-line),var(--tyba-control-shadow))] text-tyba-text hover:bg-[var(--tyba-mk-glass-hover,color-mix(in_srgb,var(--tyba-text)_4%,var(--tyba-control-bg)))]",
        secondary:
          "bg-[var(--tyba-mk-glass-button,var(--tyba-control-bg))] border-[var(--tyba-mk-glass-edge,transparent)] tyba-mk-filter-md [box-shadow:var(--tyba-mk-shadow-button-glass,inset_0_0_0_1px_var(--tyba-control-line),var(--tyba-control-shadow))] text-tyba-text hover:bg-[var(--tyba-mk-glass-hover,color-mix(in_srgb,var(--tyba-text)_4%,var(--tyba-control-bg)))]",
        ghost:
          "text-tyba-text-muted hover:bg-tyba-text/5 hover:text-tyba-text",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        default: "h-10 min-w-[var(--tyba-mk-btn-min,auto)] px-[var(--tyba-mk-btn-px,20px)] has-[>svg]:px-[var(--tyba-mk-btn-px,16px)] [&_svg:not([class*='size-'])]:size-[18px]",
        xs: "h-6 gap-1 rounded-[8px] px-2 text-xs has-[>svg]:px-1.5 [&_svg:not([class*='size-'])]:size-3",
        sm: "h-[34px] min-w-[var(--tyba-mk-btn-min-sm,auto)] gap-[var(--tyba-mk-btn-gap-sm,6px)] px-[var(--tyba-mk-btn-px-sm,14px)] text-[13px] has-[>svg]:px-[var(--tyba-mk-btn-px-sm,12px)]",
        lg: "h-10 min-w-[var(--tyba-mk-btn-min,auto)] px-[var(--tyba-mk-btn-px,20px)] has-[>svg]:px-[var(--tyba-mk-btn-px,16px)] [&_svg:not([class*='size-'])]:size-[18px]",
        icon: "size-10 [&_svg:not([class*='size-'])]:size-[18px]",
        "icon-xs": "size-6 rounded-[8px] [&_svg:not([class*='size-'])]:size-3",
        "icon-sm": "size-[34px]",
        "icon-lg": "size-10 [&_svg:not([class*='size-'])]:size-[18px]",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

const Button = React.forwardRef<
  HTMLButtonElement,
  React.ComponentProps<"button"> &
    VariantProps<typeof buttonVariants> & {
      asChild?: boolean
    }
>(function Button(
  { className, variant = "default", size = "default", asChild = false, ...props },
  ref,
) {
  const Comp = asChild ? Slot.Root : "button"

  return (
    <Comp
      ref={ref}
      data-slot="button"
      data-variant={variant}
      data-size={size}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  )
})

export { Button, buttonVariants }
