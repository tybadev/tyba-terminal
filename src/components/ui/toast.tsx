import * as React from "react";
import { Toast as ToastPrimitive } from "radix-ui";
import { X } from "@phosphor-icons/react";

import { cn } from "@/lib/utils";

function ToastProvider({
  swipeDirection = "right",
  ...props
}: React.ComponentProps<typeof ToastPrimitive.Provider>) {
  return (
    <ToastPrimitive.Provider swipeDirection={swipeDirection} {...props} />
  );
}

function ToastViewport({
  className,
  ...props
}: React.ComponentProps<typeof ToastPrimitive.Viewport>) {
  return (
    <ToastPrimitive.Viewport
      data-slot="toast-viewport"
      className={cn(
        "fixed right-4 top-12 z-[100] flex w-full max-w-sm flex-col gap-2 outline-none",
        className,
      )}
      {...props}
    />
  );
}

function Toast({
  className,
  ...props
}: React.ComponentProps<typeof ToastPrimitive.Root>) {
  return (
    <ToastPrimitive.Root
      data-slot="toast"
      className={cn(
        "pointer-events-auto relative w-full rounded-lg border border-tyba-border bg-tyba-overlay/95 p-3 text-tyba-text backdrop-blur-xl [box-shadow:var(--tyba-edge),var(--tyba-shadow-lg)]",
        "motion-safe:animate-tyba-pop-in",
        "data-[swipe=move]:translate-x-[var(--radix-toast-swipe-move-x)] data-[swipe=cancel]:translate-x-0 data-[swipe=end]:translate-x-[var(--radix-toast-swipe-end-x)] data-[swipe=move]:transition-none",
        className,
      )}
      {...props}
    />
  );
}

function ToastTitle({
  className,
  ...props
}: React.ComponentProps<typeof ToastPrimitive.Title>) {
  return (
    <ToastPrimitive.Title
      data-slot="toast-title"
      className={cn("tyba-label text-tyba-text-faint", className)}
      {...props}
    />
  );
}

function ToastDescription({
  className,
  ...props
}: React.ComponentProps<typeof ToastPrimitive.Description>) {
  return (
    <ToastPrimitive.Description
      data-slot="toast-description"
      className={cn("mt-1 text-tyba-text", className)}
      {...props}
    />
  );
}

function ToastAction({
  className,
  ...props
}: React.ComponentProps<typeof ToastPrimitive.Action>) {
  return (
    <ToastPrimitive.Action
      data-slot="toast-action"
      className={className}
      {...props}
    />
  );
}

/** Item 0 do contrato ("polir o alarme de deriva"): todo toast precisa de um
 * affordance ÓBVIO de fechar, não só swipe. `Toast.Close` do Radix já fecha
 * o toast (dispara `onOpenChange(false)`) sem precisar de handler próprio. */
function ToastClose({
  className,
  ...props
}: React.ComponentProps<typeof ToastPrimitive.Close>) {
  return (
    <ToastPrimitive.Close
      data-slot="toast-close"
      aria-label="Fechar"
      className={cn(
        "absolute right-1.5 top-1.5 rounded-[3px] p-0.5 text-tyba-text-faint transition-colors hover:bg-tyba-text/[.08] hover:text-tyba-text",
        className,
      )}
      {...props}
    >
      <X size={12} weight="bold" />
    </ToastPrimitive.Close>
  );
}

export {
  Toast,
  ToastAction,
  ToastClose,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
};
