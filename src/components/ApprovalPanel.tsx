import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { WarningOctagon } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import {
  INITIAL_PANEL_UI,
  panelStep,
  redReasonText,
  type PanelEvent,
} from "@/lib/approvalPanel";
import { resolveApproval, type ApprovalRequest } from "@/lib/ipc";
import { approvalActions } from "@/lib/notifications";

interface Props {
  request: ApprovalRequest;
  rect: { left: number; top: number; width: number; height: number };
  /** O painel da sessão tem o foco — só aí o painel puxa o teclado. */
  focused: boolean;
}

/**
 * Painel vermelho dentro da sessão (R4, CA-07..10). Sobreposto ao painel,
 * ancorado embaixo — não muda o `rect` nem as linhas do PTY. Não tem
 * "fechar": some quando o App tira o pedido da lista, o que só acontece em
 * `approvals://resolved`. Quem monta usa `key={request.id}`, então a
 * confirmação armada de um pedido nunca vaza para o seguinte.
 */
export function ApprovalPanel({ request, rect, focused }: Props) {
  const { t } = useTranslation();
  const [ui, setUi] = useState(INITIAL_PANEL_UI);
  const [sending, setSending] = useState(false);
  const denyRef = useRef<HTMLButtonElement>(null);

  // Foco inicial no Negar, nunca no Aprovar: Enter acidental recusa. O rAF
  // deixa o efeito do TerminalView (que foca o xterm) rodar antes.
  useEffect(() => {
    if (!focused) return;
    const frame = requestAnimationFrame(() => denyRef.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, [focused, request.id]);

  const dispatch = (event: PanelEvent) => {
    const step = panelStep(request, ui, event);
    setUi(step.ui);
    if (!step.effect) return;
    const { requestId, decision, feedback } = step.effect;
    setSending(true);
    // O painel NÃO some aqui: continua até o core confirmar com `resolved`.
    // Se a chamada falhar, os botões voltam e o pedido segue decidível.
    resolveApproval(requestId, decision, feedback).catch(() =>
      setSending(false),
    );
  };

  const reason = redReasonText(request.risk_reason);

  return (
    <div
      className="pointer-events-none z-20 flex items-end justify-center p-3"
      style={{
        position: "absolute",
        left: `${rect.left}%`,
        top: `${rect.top}%`,
        width: `${rect.width}%`,
        height: `${rect.height}%`,
      }}
    >
      <div
        role="alertdialog"
        aria-modal="false"
        aria-labelledby={`approval-panel-title-${request.id}`}
        onKeyDown={(e) => {
          if (e.key !== "Escape") return;
          // Esc não fecha nem decide (CA-09) — e não vaza para o terminal.
          e.preventDefault();
          e.stopPropagation();
          dispatch({ type: "escape" });
        }}
        className="pointer-events-auto flex max-h-full w-full max-w-2xl flex-col gap-2 rounded-[6px] border border-tyba-red/60 bg-tyba-sunken p-3 shadow-2xl"
      >
        <div className="flex items-start gap-2">
          <WarningOctagon
            size={16}
            weight="fill"
            className="mt-0.5 shrink-0 text-tyba-red"
          />
          <div className="min-w-0 flex-1">
            <p
              id={`approval-panel-title-${request.id}`}
              className="text-[13px] font-semibold text-tyba-red"
            >
              {t("approvalPanelTitle")}
            </p>
            <p className="mt-0.5 break-words text-[12px] text-tyba-text-muted">
              {t(reason.key, reason.params)}
            </p>
          </div>
        </div>

        <pre
          tabIndex={0}
          className="max-h-48 min-h-0 overflow-y-auto whitespace-pre-wrap break-words rounded-[3px] border border-tyba-red/30 bg-tyba-bg px-2 py-1.5 font-mono text-[11px] leading-relaxed text-tyba-text select-text"
        >
          {request.command}
        </pre>

        {request.cwd && (
          <p className="break-all font-mono text-[10px] text-tyba-text-faint">
            <span className="text-tyba-text-muted">
              {t("approvalPanelCwd")}:
            </span>{" "}
            {request.cwd}
          </p>
        )}
        {request.context && (
          <p className="line-clamp-3 break-words text-[11px] text-tyba-text-muted">
            {request.context}
          </p>
        )}

        {ui.confirming && (
          <p className="text-[11px] text-tyba-red">
            {t("approvalPanelConfirmHint")}
          </p>
        )}

        <div className="flex flex-wrap items-center justify-end gap-2">
          {approvalActions(request.risk).map((action) => {
            const isApprove = action.id === "approve";
            const label =
              isApprove && ui.confirming
                ? t("confirmApprove")
                : t(action.labelKey);
            return (
              <Button
                key={action.id}
                ref={action.id === "deny" ? denyRef : undefined}
                size="sm"
                variant={isApprove && ui.confirming ? "default" : "outline"}
                disabled={sending}
                onClick={() => dispatch({ type: "action", action: action.id })}
                className={`h-6 rounded-[4px] px-2.5 text-[11px] ${
                  isApprove
                    ? ui.confirming
                      ? "bg-tyba-red text-white hover:bg-tyba-red/90"
                      : "text-tyba-text"
                    : "text-tyba-text-muted"
                }`}
              >
                {label}
              </Button>
            );
          })}
        </div>

        {ui.feedbackOpen && (
          <div className="motion-safe:animate-tyba-pop-in space-y-1.5">
            <Textarea
              autoFocus
              rows={2}
              value={ui.feedback}
              disabled={sending}
              onChange={(e) =>
                dispatch({ type: "typeFeedback", text: e.target.value })
              }
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  dispatch({ type: "sendFeedback" });
                }
              }}
              placeholder={t("denyFeedbackPlaceholder")}
              className="min-h-0 resize-none font-mono text-[11px] md:text-[11px]"
            />
            <div className="flex justify-end gap-2">
              <Button
                size="sm"
                variant="ghost"
                disabled={sending}
                onClick={() => dispatch({ type: "cancelFeedback" })}
                className="h-6 rounded-[4px] px-2.5 text-[11px] text-tyba-text-muted"
              >
                {t("cancel")}
              </Button>
              <Button
                size="sm"
                disabled={sending}
                onClick={() => dispatch({ type: "sendFeedback" })}
                className="h-6 rounded-[4px] px-2.5 text-[11px]"
              >
                {t("denyAndTell")}
              </Button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
