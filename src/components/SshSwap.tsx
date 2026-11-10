import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { ArrowsLeftRight, X } from "@phosphor-icons/react";

import type { ReplacedShell, SessionId } from "../lib/ipc";
import { TOAST_AUTO_DISMISS_MS } from "../lib/toast";

/**
 * A marca da troca no topo do scrollback do pane integrado (quadros 1C/1D de
 * SshTrocaNoPane): "⇄ substituiu o shell local · ssh encerrado · HH:MM".
 */
export function SshSwapMark({ mark }: { mark: ReplacedShell }) {
  const { t } = useTranslation();
  const time = new Date(mark.at_ms).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
  return (
    <div className="mx-[18px] mb-1.5 mt-3 flex items-center gap-2 text-[11px] text-tyba-text-faint">
      <span className="h-px flex-1 bg-tyba-border-soft" />
      <ArrowsLeftRight size={12} className="shrink-0" />
      <span className="shrink-0">
        {t(mark.auto ? "sshSwapMarkAuto" : "sshSwapMarkManual", { time })}
      </span>
      <span className="h-px flex-1 bg-tyba-border-soft" />
    </div>
  );
}

/**
 * O toast discreto no pé do pane quando a troca foi automática ("Sempre abrir
 * integrado", quadro 1D). Some sozinho; sem "Desfazer" — o shell antigo já foi
 * encerrado.
 */
export function SshSwapToast({
  sessionId,
  onAdjust,
  onClose,
}: {
  sessionId: SessionId;
  /** Abre Configurações › SSH. */
  onAdjust: () => void;
  /** Estável (`useCallback`): o relógio do "some sozinho" depende dele. */
  onClose: (sessionId: SessionId) => void;
}) {
  const { t } = useTranslation();
  useEffect(() => {
    const timer = window.setTimeout(() => onClose(sessionId), TOAST_AUTO_DISMISS_MS);
    return () => window.clearTimeout(timer);
  }, [sessionId, onClose]);
  return (
    <div
      role="status"
      className="absolute bottom-[18px] left-1/2 z-20 flex -translate-x-1/2 items-center gap-2.5 whitespace-nowrap rounded-[9px] py-2 pl-3 pr-2 text-[12px] text-tyba-text backdrop-blur-xl backdrop-saturate-150"
      style={{ background: "var(--tyba-glass-float)", boxShadow: "var(--tyba-shadow-md)" }}
    >
      <span className="flex size-[22px] items-center justify-center rounded-[6px] bg-tyba-blue-tint text-tyba-blue">
        <ArrowsLeftRight size={13} />
      </span>
      <span>{t("sshSwapToast")}</span>
      <button
        onClick={() => {
          onAdjust();
          onClose(sessionId);
        }}
        className="inline-flex h-6 items-center rounded-[6px] px-[9px] text-[11.5px] text-tyba-text-muted hover:text-tyba-text"
      >
        {t("sshSwapToastAdjust")}
      </button>
      <button
        onClick={() => onClose(sessionId)}
        aria-label={t("close")}
        className="flex size-6 items-center justify-center rounded-[6px] text-tyba-text-faint hover:text-tyba-text"
      >
        <X size={12} />
      </button>
    </div>
  );
}
