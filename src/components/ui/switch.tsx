import { Switch as SwitchPrimitive } from "radix-ui";

import { cn } from "@/lib/utils";

// Switch do Muriki (`.mk-switch`): trilho 46×28, knob branco de 22px com a
// sombra do knob e a mola de 320ms; ligado pinta o trilho da cor primária.
// No clique o knob alarga para 28px, do lado onde ele está. Mola e knob que
// alarga são do claro do TYBA (`--tyba-mk-*`); nos outros temas o fallback
// mantém o knob fixo e a mola de 300ms.
function Switch({
  className,
  ...props
}: React.ComponentProps<typeof SwitchPrimitive.Root>) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      className={cn(
        "peer group inline-flex h-7 w-[46px] shrink-0 cursor-pointer items-center rounded-full transition-colors duration-200 outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-45 data-[state=checked]:bg-primary data-[state=unchecked]:bg-[var(--tyba-switch-track)] data-[state=unchecked]:[box-shadow:inset_0_1px_2px_color-mix(in_srgb,var(--tyba-text)_12%,transparent)]",
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        className={cn(
          "pointer-events-none block h-[22px] w-[22px] rounded-full bg-white [box-shadow:var(--tyba-knob-shadow)] [transition:translate_var(--tyba-mk-spring,300ms)_cubic-bezier(.3,1.4,.5,1),transform_var(--tyba-mk-spring,300ms)_cubic-bezier(.3,1.4,.5,1),width_200ms] group-active:w-[var(--tyba-mk-knob-press,22px)] data-[state=checked]:translate-x-[21px] group-active:data-[state=checked]:translate-x-[var(--tyba-mk-knob-press-x,21px)] data-[state=unchecked]:translate-x-[3px]",
        )}
      />
    </SwitchPrimitive.Root>
  );
}

export { Switch };
