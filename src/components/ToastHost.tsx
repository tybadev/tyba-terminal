import { useEffect, useState } from "react";
import { Info, Warning, WarningOctagon } from "@phosphor-icons/react";

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
  warning: Warning,
  error: WarningOctagon,
};

const TONE_CLASS: Record<ToastTone, string> = {
  info: "text-tyba-blue",
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
            <ToastClose />
            <div className="flex items-start gap-2 pr-4">
              <Icon
                aria-hidden="true"
                size={16}
                weight="fill"
                className={`mt-0.5 shrink-0 ${TONE_CLASS[toast.tone]}`}
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
                {toast.action && (
                  <ToastAction
                    altText={toast.action.label}
                    onClick={toast.action.run}
                    className="tyba-label mt-2 inline-flex w-fit items-center rounded-md border border-tyba-border px-2 py-1 text-tyba-blue hover:bg-tyba-blue/10"
                  >
                    {toast.action.label}
                  </ToastAction>
                )}
              </div>
            </div>
          </Toast>
        );
      })}
      <ToastViewport className="bottom-4 top-auto" />
    </ToastProvider>
  );
}
