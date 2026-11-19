import { useEffect, useState } from "react";
import { CheckCircle, Info, Warning, WarningOctagon } from "@phosphor-icons/react";

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
                className={`shrink-0 self-start ${TONE_CLASS[toast.tone]}`}
              />
              <div className="min-w-0 flex-1">
                <ToastTitle>{toast.title}</ToastTitle>
                {toast.detail && (
                  <ToastDescription>
                    <span className="block break-words font-mono text-xs">
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
