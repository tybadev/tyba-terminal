import { useEffect, useState } from "react";
import { Bell, CheckCircle, Info, Warning, WarningCircle, WarningOctagon } from "@phosphor-icons/react";

import { buttonVariants } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import {
  Toast,
  ToastAction,
  ToastClose,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
} from "@/components/ui/toast";
import {
  dismissToast,
  subscribeToasts,
  toastDuration,
  type ToastMessage,
  type ToastTone,
} from "@/lib/toast";

const TONE_ICON: Record<ToastTone, typeof Info> = {
  info: Info,
  success: CheckCircle,
  warning: Warning,
  error: WarningOctagon,
};

/** O ícone do toast do Muriki (`check-circle`, `warning-circle`, `bell`),
 *  em traço regular e centrado: só no claro do TYBA. */
const MK_TONE_ICON: Record<ToastTone, typeof Info> = {
  info: Bell,
  success: CheckCircle,
  warning: Warning,
  error: WarningCircle,
};

const TONE_CLASS: Record<ToastTone, string> = {
  info: "text-tyba-blue",
  success: "text-tyba-green",
  warning: "text-tyba-amber",
  error: "text-tyba-red",
};

export function ToastHost() {
  const [toasts, setToasts] = useState<ToastMessage[]>([]);

  useEffect(() => subscribeToasts(setToasts), []);

  return (
    <ToastProvider swipeDirection="right" duration={Infinity}>
      {toasts.map((toast) => {
        const Icon = TONE_ICON[toast.tone];
        const MkIcon = MK_TONE_ICON[toast.tone];
        return (
          <Toast
            key={toast.id}
            duration={toastDuration(toast)}
            onOpenChange={(open) => {
              if (!open) dismissToast(toast.id);
            }}
          >
            {/* Linha do Muriki: ícone, texto, ação e fechar lado a lado. */}
            <div className="flex items-center gap-3">
              <Icon
                aria-hidden="true"
                size={20}
                weight="fill"
                className={`tyba-mk-hide shrink-0 self-start ${TONE_CLASS[toast.tone]}`}
              />
              <MkIcon
                aria-hidden="true"
                size={20}
                className={`tyba-mk-only shrink-0 ${TONE_CLASS[toast.tone]}`}
              />
              <div className="min-w-0 flex-1">
                <ToastTitle>{toast.title}</ToastTitle>
                {toast.detail && (
                  <ToastDescription>
                    {/* No claro, a descrição sans 13 do Muriki; fora dele, mono 12. */}
                    <span className="block break-words [font-family:var(--tyba-mk-toast-font,var(--font-mono))] text-[length:var(--tyba-mk-toast-desc,0.75rem)] leading-[var(--tyba-mk-toast-lh,1rem)]">
                      {toast.detail}
                    </span>
                  </ToastDescription>
                )}
              </div>
              {toast.action && (
                <ToastAction
                  altText={toast.action.label}
                  onClick={toast.action.run}
                  className={cn(
                    buttonVariants({ variant: "ghost", size: "sm" }),
                    "shrink-0 text-tyba-blue hover:text-tyba-blue",
                  )}
                >
                  {toast.action.label}
                </ToastAction>
              )}
              <ToastClose className="static shrink-0 rounded-[8px] p-1.5" />
            </div>
          </Toast>
        );
      })}
      <ToastViewport className="bottom-4 top-auto" />
    </ToastProvider>
  );
}
