import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { Slot } from "radix-ui"

import { cn } from "@/lib/utils"

// Botão do Muriki (quadro de componentes): raio 10px, 600, aperta 2% no
// clique. default = primário líquido; outline/secondary = vidro (branco, fio
// fino, sombra do knob); ghost = transparente; destructive = vidro com o
// texto em vermelho. As cores vêm dos tokens `--tyba-control-*`/`--primary`.
// Tamanho padrão = `mk-btn` (40px, ícone 18px); sm = `mk-btn-sm` (34px).
const buttonVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-2 rounded-[10px] border border-transparent text-sm font-semibold whitespace-nowrap transition-[transform,background-color,box-shadow,color] duration-150 outline-none active:scale-[.98] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-45 aria-invalid:outline-2 aria-invalid:outline-destructive [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default:
          "bg-primary text-primary-foreground [box-shadow:var(--tyba-primary-shadow)] hover:bg-[color-mix(in_srgb,var(--primary)_88%,black)] [&_kbd]:border-transparent! [&_kbd]:bg-white/20! [&_kbd]:text-primary-foreground! [&_kbd]:shadow-none!",
        destructive:
          "bg-[var(--tyba-control-bg)] text-tyba-red [box-shadow:inset_0_0_0_1px_var(--tyba-control-line),var(--tyba-control-shadow)] hover:bg-tyba-red-tint focus-visible:outline-destructive",
        outline:
          "bg-[var(--tyba-control-bg)] text-tyba-text [box-shadow:inset_0_0_0_1px_var(--tyba-control-line),var(--tyba-control-shadow)] hover:bg-[color-mix(in_srgb,var(--tyba-text)_4%,var(--tyba-control-bg))]",
        secondary:
          "bg-[var(--tyba-control-bg)] text-tyba-text [box-shadow:inset_0_0_0_1px_var(--tyba-control-line),var(--tyba-control-shadow)] hover:bg-[color-mix(in_srgb,var(--tyba-text)_4%,var(--tyba-control-bg))]",
        ghost:
          "text-tyba-text-muted hover:bg-tyba-text/5 hover:text-tyba-text",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        default: "h-10 px-5 has-[>svg]:px-4 [&_svg:not([class*='size-'])]:size-[18px]",
        xs: "h-6 gap-1 rounded-[8px] px-2 text-xs has-[>svg]:px-1.5 [&_svg:not([class*='size-'])]:size-3",
        sm: "h-[34px] gap-1.5 px-3.5 text-[13px] has-[>svg]:px-3",
        lg: "h-10 px-5 has-[>svg]:px-4 [&_svg:not([class*='size-'])]:size-[18px]",
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
