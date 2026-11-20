import * as React from "react";
import { ContextMenu as ContextMenuPrimitive } from "radix-ui";
import { CaretRight } from "@phosphor-icons/react";

import { cn } from "@/lib/utils";

// Mesmo item do menu do Muriki que o `dropdown-menu`: 14px em linha de 34px,
// aceso no cartão branco com a sombra do knob (tokens de `.tyba-pill-on`).
const MENU_ITEM =
  "relative flex min-h-[34px] cursor-default select-none items-center gap-3 rounded-[8px] px-3 text-[14px] leading-5 text-tyba-text outline-none focus:bg-[var(--tyba-glass-active)] focus:[background-image:var(--tyba-pill-sheen)] focus:[box-shadow:var(--tyba-pill-on)]";

function ContextMenu({
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Root>) {
  return <ContextMenuPrimitive.Root data-slot="context-menu" {...props} />;
}

function ContextMenuTrigger({
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Trigger>) {
  return (
    <ContextMenuPrimitive.Trigger data-slot="context-menu-trigger" {...props} />
  );
}

function ContextMenuContent({
  className,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Content>) {
  return (
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.Content
        data-slot="context-menu-content"
        className={cn(
          "z-50 min-w-40 overflow-hidden rounded-[14px] border border-tyba-border bg-tyba-overlay/[.97] p-1.5 backdrop-blur-xl [box-shadow:var(--tyba-edge),var(--tyba-shadow-lg)]",
          className,
        )}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  );
}

function ContextMenuItem({
  className,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Item>) {
  return (
    <ContextMenuPrimitive.Item
      data-slot="context-menu-item"
      className={cn(
        MENU_ITEM,
        "data-[disabled]:pointer-events-none data-[disabled]:opacity-40",
        className,
      )}
      {...props}
    />
  );
}

function ContextMenuSeparator({
  className,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Separator>) {
  return (
    <ContextMenuPrimitive.Separator
      data-slot="context-menu-separator"
      className={cn("mx-2 my-1.5 h-px bg-[var(--tyba-line)]", className)}
      {...props}
    />
  );
}

/** Coluna de atalho à direita do item (quadro: mono 11px, apagado). */
function ContextMenuShortcut({
  className,
  ...props
}: React.ComponentProps<"span">) {
  return (
    <span
      data-slot="context-menu-shortcut"
      className={cn(
        "ml-auto pl-4 font-mono text-[11px] text-tyba-text-faint",
        className,
      )}
      {...props}
    />
  );
}

function ContextMenuSub({
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.Sub>) {
  return <ContextMenuPrimitive.Sub data-slot="context-menu-sub" {...props} />;
}

function ContextMenuSubTrigger({
  className,
  children,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.SubTrigger>) {
  return (
    <ContextMenuPrimitive.SubTrigger
      data-slot="context-menu-sub-trigger"
      className={cn(
        MENU_ITEM,
        "data-[state=open]:bg-[var(--tyba-glass-active)] data-[state=open]:[box-shadow:var(--tyba-pill-on)]",
        className,
      )}
      {...props}
    >
      {children}
      <CaretRight size={11} className="ml-auto shrink-0" />
    </ContextMenuPrimitive.SubTrigger>
  );
}

function ContextMenuSubContent({
  className,
  ...props
}: React.ComponentProps<typeof ContextMenuPrimitive.SubContent>) {
  return (
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.SubContent
        data-slot="context-menu-sub-content"
        className={cn(
          "z-50 min-w-40 overflow-hidden rounded-[14px] border border-tyba-border bg-tyba-overlay/[.97] p-1.5 backdrop-blur-xl [box-shadow:var(--tyba-edge),var(--tyba-shadow-lg)]",
          className,
        )}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  );
}

export {
  ContextMenu,
  ContextMenuTrigger,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubTrigger,
  ContextMenuSubContent,
};
