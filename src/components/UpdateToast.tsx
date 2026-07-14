import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ArrowSquareOut, Sparkle } from "@phosphor-icons/react";

import {
  Toast,
  ToastAction,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
} from "@/components/ui/toast";
import { changelogUrl } from "@/lib/changelog";
import type { UpdateStatus } from "@/lib/ipc";

const NEVER_AUTO_DISMISS_MS = 24 * 60 * 60 * 1000;

interface Props {
  status: UpdateStatus | null;
  onDismiss: () => void;
}

export function UpdateToast({ status, onDismiss }: Props) {
  const { t, i18n } = useTranslation();
  const open = status !== null && !status.dismissed;
  const href = changelogUrl(i18n.language);

  return (
    <ToastProvider swipeDirection="left">
      <Toast
        open={open}
        duration={NEVER_AUTO_DISMISS_MS}
        onOpenChange={(o) => {
          if (!o) onDismiss();
        }}
      >
        <div className="flex items-start gap-2.5">
          <Sparkle
            size={15}
            weight="fill"
            className="mt-0.5 shrink-0 text-tyba-violet"
          />
          <div className="min-w-0 flex-1">
            <ToastTitle className="text-[12px]">
              {t("updateAvailable", { version: status?.info.version ?? "" })}
            </ToastTitle>
            <ToastDescription className="text-[11px] text-tyba-text-faint">
              {t("updateHint")}
            </ToastDescription>
            <ToastAction
              altText={t("updateOpenChangelog")}
              className="mt-2 flex h-6 items-center gap-1.5 rounded-[4px] border border-tyba-border px-2 text-[11px] text-tyba-text-muted transition-colors hover:bg-tyba-text/[.06] hover:text-tyba-text"
              onClick={() => {
                void openUrl(href).catch(() => {});
                onDismiss();
              }}
            >
              <ArrowSquareOut size={12} />
              {t("updateOpenChangelog")}
            </ToastAction>
          </div>
        </div>
      </Toast>
      <ToastViewport className="bottom-4 left-4 right-auto top-auto" />
    </ToastProvider>
  );
}
