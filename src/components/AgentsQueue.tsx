import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { GitDiff, ShieldSlash, X } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import {
  buildRows,
  boardOrder,
  oldestApprovalBySession,
  wantsAttention,
  type AgentRow,
  type SessionPlace,
} from "../lib/agentsBoard";
import {
  resolveApproval,
  type ApprovalDecision,
  type ApprovalRequest,
  type LayoutState,
  type Session,
  type SessionId,
} from "../lib/ipc";
import { approvalActions, decideApproval } from "../lib/notifications";
import { AgentIcon } from "./icons/AgentIcon";

interface Props {
  sessions: Session[];
  layout: LayoutState;
  approvals: ApprovalRequest[];
  onJump: (sessionId: SessionId, place: SessionPlace) => void;
  onReviewDiff: (sessionId: SessionId) => void;
  onClose: () => void;
}

/**
 * O que esta linha permite fazer — e o que ela **não** permite é informação.
 *
 * Agente observado só tem "ir para": não porque faltou botão, mas porque não há
 * gate ali, logo não há decisão sua a tomar. O status de segunda classe fica
 * visível pelo que a linha não consegue fazer, em vez de por um selo que o
 * usuário precisa aprender a ler.
 */
function Row({
  row,
  approval,
  onJump,
  onReviewDiff,
}: {
  row: AgentRow;
  approval: ApprovalRequest | undefined;
  onJump: Props["onJump"];
  onReviewDiff: Props["onReviewDiff"];
}) {
  const { t } = useTranslation();
  // Mesmo caminho de decisão do toast e do painel de notificações
  // (lib/notifications#decideApproval) — o gate de risco vermelho (arma
  // confirmação no 1º clique, resolve no 2º) e o resolveApproval de fato
  // chamado são idênticos nas três superfícies; só o layout aqui é o da
  // fila, inline por natureza. Ação vermelha nunca é automática (princípio
  // 4 do CLAUDE.md), e a lista não compra exceção.
  const [confirmingId, setConfirmingId] = useState<number | null>(null);
  const [feedbackOpen, setFeedbackOpen] = useState(false);
  const [feedback, setFeedback] = useState("");

  const decide = (decision: ApprovalDecision, withFeedback?: string) => {
    if (!approval) return;
    const effect = decideApproval({
      request: approval,
      decision,
      confirmingId,
      feedback: withFeedback,
    });
    if (effect.type === "armRedConfirm") {
      setConfirmingId(effect.requestId);
      return;
    }
    setConfirmingId(null);
    setFeedbackOpen(false);
    setFeedback("");
    resolveApproval(effect.requestId, effect.decision, effect.feedback).catch(
      () => {},
    );
  };

  const espera = wantsAttention(row);
  const concluiu =
    row.session.status.state === "idle" && row.session.attention === true;
  const resumo =
    row.session.status.state === "idle" ? row.session.status.summary : null;

  return (
    <li
      className={`relative tyba-divide-b px-3 py-2.5 [&:last-child]:shadow-none ${
        espera ? "bg-tyba-amber/[.04]" : ""
      }`}
    >
      {/* Barra de atenção: quem espera ganha a lateral âmbar, que é a cor que o
          sistema já reserva para agente de IA pedindo alguém. */}
      {espera && (
        <span
          className="absolute inset-y-0 left-0 w-0.5 bg-tyba-amber"
          aria-hidden
        />
      )}

      <div className="flex items-center gap-2">
        <span
          className={`size-2 shrink-0 rounded-full ${row.visual.dotClass}`}
          aria-hidden
        />
        <AgentIcon size={13} className="shrink-0 text-tyba-text-muted" />
        <span className="min-w-0 truncate text-[13px] text-tyba-text">
          {row.place.workspaceName}
        </span>
        <span className={`shrink-0 text-[11px] ${row.visual.textClass}`}>
          {t(row.visual.labelKey)}
        </span>
        {row.observed && (
          <ShieldSlash
            size={12}
            className="shrink-0 text-tyba-amber"
            aria-label={t("agentsBoardNoGate")}
          />
        )}
      </div>

      {/* O que ele quer rodar, quando quer. Mono e recuado: é texto de máquina,
          e o recuo é o que separa "o agente é este" de "o agente pede isto". */}
      {approval && (
        <p className="mt-1.5 ml-4 truncate rounded-[3px] border border-tyba-border bg-tyba-bg px-2 py-1 font-mono text-[11px] text-tyba-text-muted">
          {approval.command}
        </p>
      )}
      {!approval && resumo && (
        <p className="mt-1 ml-4 truncate text-[11px] text-tyba-text-faint">
          {resumo}
        </p>
      )}

      <div className="mt-1.5 ml-4 flex flex-wrap items-center gap-1">
        {approval &&
          approvalActions(approval.risk).map((action) => {
            const isConfirmApprove =
              action.id === "approve" &&
              approval.risk === "red" &&
              confirmingId === approval.id;
            const label = isConfirmApprove
              ? t("confirmApprove")
              : t(action.labelKey);
            const onClick = () => {
              if (action.id === "denyWithReason") {
                setFeedbackOpen(true);
                return;
              }
              const decision: ApprovalDecision =
                action.id === "alwaysAllow"
                  ? "approved_always"
                  : action.id === "deny"
                    ? "denied"
                    : "approved";
              decide(decision);
            };
            return (
              <Button
                key={action.id}
                size="sm"
                variant="ghost"
                onClick={onClick}
                className={`h-6 px-2 text-[11px] ${
                  action.id === "approve"
                    ? isConfirmApprove
                      ? "bg-tyba-red/15 text-tyba-red"
                      : "text-tyba-green hover:bg-tyba-green/10"
                    : action.id === "deny"
                      ? "text-tyba-red hover:bg-tyba-red/10"
                      : "text-tyba-text-faint hover:bg-tyba-text/[.05] hover:text-tyba-text"
                }`}
              >
                {label}
              </Button>
            );
          })}
        {concluiu && !row.observed && (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => onReviewDiff(row.session.id)}
            className="h-6 gap-1 px-2 text-[11px] text-tyba-blue hover:bg-tyba-blue/10"
          >
            <GitDiff size={11} />
            {t("diffReviewAction")}
          </Button>
        )}
        <Button
          size="sm"
          variant="ghost"
          onClick={() => onJump(row.session.id, row.place)}
          className="h-6 px-2 text-[11px] text-tyba-text-faint hover:text-tyba-text"
        >
          {t("agentsQueueGoTo")}
        </Button>
      </div>
      {approval && feedbackOpen && (
        <div className="motion-safe:animate-tyba-pop-in mt-1.5 ml-4 space-y-1.5">
          <Textarea
            autoFocus
            rows={2}
            value={feedback}
            onChange={(e) => setFeedback(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                decide("denied", feedback);
              }
              if (e.key === "Escape") {
                e.preventDefault();
                setFeedbackOpen(false);
                setFeedback("");
              }
            }}
            placeholder={t("denyFeedbackPlaceholder")}
            className="min-h-0 resize-none text-[11px]"
          />
          <div className="flex gap-2">
            <Button
              size="sm"
              onClick={() => decide("denied", feedback)}
              className="h-6 rounded-[4px] px-2.5 text-[11px]"
            >
              {t("denyAndTell")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                setFeedbackOpen(false);
                setFeedback("");
              }}
              className="h-6 rounded-[4px] px-2.5 text-[11px] text-tyba-text-muted"
            >
              {t("cancel")}
            </Button>
          </div>
        </div>
      )}
    </li>
  );
}

/**
 * A fila de agentes: não é um diretório, é o que espera por você.
 *
 * Nasceu como página e estava errado — página contradiz a promessa da feature.
 * Como painel lateral, o terminal continua visível enquanto você decide, que é
 * o que separa "resolver" de "ser teletransportado".
 *
 * O diferencial sobre o herdr não é a aparência: é o **gate**. Ele só consegue
 * dizer "esse travou, vá lá"; aqui a linha sabe no que travou e resolve.
 */
export function AgentsQueue({
  sessions,
  layout,
  approvals,
  onJump,
  onReviewDiff,
  onClose,
}: Props) {
  const { t } = useTranslation();
  const rows = useMemo(
    () => boardOrder(buildRows(sessions, layout)),
    [sessions, layout],
  );
  const esperando = useMemo(() => rows.filter(wantsAttention).length, [rows]);
  // Mesma função que o toaster usa (via agentQueueVisibleApprovalIds em
  // App.tsx) para saber quais ids a fila torna acionáveis — uma cópia
  // divergente aqui era exatamente o defeito que deixava um segundo pedido
  // pendente da mesma sessão sem toast E sem linha na fila.
  const porSessao = useMemo(
    () => oldestApprovalBySession(approvals),
    [approvals],
  );

  return (
    <div className="flex h-full w-full flex-col overflow-hidden">
      <header className="flex shrink-0 items-center gap-2 tyba-divide-b px-3 py-2">
        <h2 className="text-[11px] uppercase tracking-[0.12em] text-tyba-text-muted">
          {t("agentsQueue")}
        </h2>
        {esperando > 0 && (
          <span className="rounded-full bg-tyba-amber/15 px-1.5 text-[10px] text-tyba-amber">
            {t("agentsQueueWaiting", { count: esperando })}
          </span>
        )}
        <span className="flex-1" />
        <button
          type="button"
          onClick={onClose}
          aria-label={t("close")}
          className="text-tyba-text-faint transition-colors hover:text-tyba-text"
        >
          <X size={13} />
        </button>
      </header>

      {rows.length === 0 ? (
        /* O vazio é a recompensa, não um placeholder: a promessa da feature é
           que você NÃO precisa olhar. Sem ilustração, sem card — só a frase e
           silêncio. */
        <p className="px-3 py-4 text-[12px] text-tyba-text-faint">
          {t("agentsQueueEmpty")}
        </p>
      ) : (
        <ul className="min-h-0 flex-1 overflow-y-auto">
          {rows.map((row) => (
            <Row
              key={row.session.id}
              row={row}
              approval={porSessao.get(row.session.id)}
              onJump={onJump}
              onReviewDiff={onReviewDiff}
            />
          ))}
        </ul>
      )}
    </div>
  );
}
