import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { X } from "@phosphor-icons/react";

import { Shortcut } from "@/components/ui/kbd";
import {
  actionsByCategory,
  ACTION_LABEL_KEYS,
  KEY_CATEGORY_LABEL_KEYS,
  type Bindings,
} from "../lib/keys";

export function ShortcutsPanel({
  open,
  bindings,
  onClose,
}: {
  open: boolean;
  bindings: Bindings;
  onClose: () => void;
}) {
  const { t } = useTranslation();

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <aside className="flex h-full w-72 shrink-0 flex-col border-l border-tyba-border bg-tyba-surface">
      <header className="flex h-8 shrink-0 items-center justify-between border-b border-tyba-border px-3">
        <span className="text-[12px] font-medium text-tyba-text">
          {t("shortcuts")}
        </span>
        <button
          aria-label={t("close")}
          onClick={onClose}
          className="rounded-[4px] p-1 text-tyba-text-faint transition-colors hover:bg-tyba-text/[.05] hover:text-tyba-text"
        >
          <X size={14} />
        </button>
      </header>
      <div className="flex-1 overflow-y-auto px-3 py-3">
        {actionsByCategory().map(([category, actions]) =>
          actions.length === 0 ? null : (
            <section key={category} className="mb-5 last:mb-0">
              <span className="tyba-label">
                {t(KEY_CATEGORY_LABEL_KEYS[category])}
              </span>
              <div className="flex flex-col gap-px pt-2">
                {actions.map((action) => (
                  <div
                    key={action}
                    className="flex h-8 items-center justify-between gap-3 rounded-[4px] px-2 text-[13px] text-tyba-text-muted hover:bg-tyba-text/[.03]"
                  >
                    <span className="min-w-0 truncate">
                      {t(ACTION_LABEL_KEYS[action])}
                    </span>
                    <Shortcut combo={bindings[action]} />
                  </div>
                ))}
              </div>
            </section>
          ),
        )}
      </div>
    </aside>
  );
}
