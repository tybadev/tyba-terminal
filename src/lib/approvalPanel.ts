import type {
  ApprovalRequest,
  RedReasonKey,
  RiskReason,
  SessionId,
} from "./ipc";
import {
  decideApproval,
  type ApprovalActionId,
  type ApprovalEffect,
} from "./notifications";

// Painel vermelho dentro da sessão (R4 do fluxo híbrido): vermelho se decide
// vendo o texto inteiro, no painel da sessão que pediu — não no toast do
// canto, que não tem espaço e some. Tudo que decide o QUE o painel mostra e
// O QUE um clique faz mora aqui, puro; o componente só desenha.

/**
 * O pedido que o painel de cada sessão mostra: o vermelho mais antigo dela.
 * Um por sessão — dois cartões empilhados cobririam o terminal inteiro, e o
 * segundo pedido aparece sozinho quando o primeiro é resolvido.
 *
 * Amarelo e verde nunca entram: amarelo do Codex continua no toast, e o do
 * Claude é do prompt nativo.
 */
export function panelRequestsBySession(
  approvals: readonly ApprovalRequest[],
): Map<SessionId, ApprovalRequest> {
  const bySession = new Map<SessionId, ApprovalRequest>();
  for (const request of approvals) {
    if (request.risk !== "red") continue;
    const current = bySession.get(request.session_id);
    if (!current || isOlder(request, current)) {
      bySession.set(request.session_id, request);
    }
  }
  return bySession;
}

function isOlder(a: ApprovalRequest, b: ApprovalRequest): boolean {
  if (a.requested_at_ms !== b.requested_at_ms) {
    return a.requested_at_ms < b.requested_at_ms;
  }
  // Mesmo milissegundo: o id do core é monotônico, então desempata pela
  // ordem de chegada.
  return a.id < b.id;
}

/**
 * Os ids que os painéis DE FATO mostram agora — só das sessões com painel na
 * tela. Entram em `hiddenApprovalIds` ("acionável em um lugar só"): o toast
 * do canto some para eles. Pedido de sessão fora da tela não entra, senão
 * ficaria sem ponto nenhum que leve até ele.
 */
export function panelVisibleApprovalIds(
  approvals: readonly ApprovalRequest[],
  visibleSessionIds: ReadonlySet<SessionId>,
): Set<number> {
  const ids = new Set<number>();
  for (const [sessionId, request] of panelRequestsBySession(approvals)) {
    if (visibleSessionIds.has(sessionId)) ids.add(request.id);
  }
  return ids;
}

/**
 * O único jeito de um pedido sair da lista (e o painel sumir):
 * `approvals://resolved`. Nenhuma tecla — Esc inclusive — passa por aqui.
 */
export function withoutResolved(
  approvals: readonly ApprovalRequest[],
  resolvedId: number,
): ApprovalRequest[] {
  return approvals.filter((request) => request.id !== resolvedId);
}

/** Toast de vermelho não decide (CA-10): aponta para a sessão. */
export function toastIsPointerOnly(request: ApprovalRequest): boolean {
  return request.risk === "red";
}

const RED_REASON_KEYS: Record<RedReasonKey, string> = {
  sudo: "redReasonSudo",
  network: "redReasonNetwork",
  pipe_to_shell: "redReasonPipeToShell",
  recursive_force_remove: "redReasonRecursiveForceRemove",
  permissions: "redReasonPermissions",
  git_push: "redReasonGitPush",
  pr_create: "redReasonPrCreate",
  forge_write: "redReasonForgeWrite",
  outside_worktree: "redReasonOutsideWorktree",
  web_access: "redReasonWebAccess",
  claude_config_write: "redReasonClaudeConfigWrite",
};

/**
 * Chave i18n + parâmetros do motivo. Chave desconhecida (core mais novo que
 * a interface) cai num texto genérico em vez de mostrar a chave crua.
 */
export function redReasonText(reason: RiskReason | null): {
  key: string;
  params: Record<string, string>;
} {
  if (!reason) return { key: "redReasonUnknown", params: {} };
  const key = RED_REASON_KEYS[reason.key] ?? "redReasonUnknown";
  const arg = reason.arg ?? "";
  return { key, params: { path: arg, action: arg } };
}

// ---- interação -----------------------------------------------------------

export interface PanelUi {
  /** Primeiro clique em Aprovar já dado; o próximo resolve. */
  confirming: boolean;
  /** "Negar e dizer o que fazer" aberto. */
  feedbackOpen: boolean;
  feedback: string;
}

export const INITIAL_PANEL_UI: PanelUi = {
  confirming: false,
  feedbackOpen: false,
  feedback: "",
};

export type PanelEvent =
  | { type: "action"; action: ApprovalActionId }
  | { type: "typeFeedback"; text: string }
  | { type: "sendFeedback" }
  | { type: "cancelFeedback" }
  | { type: "escape" };

export interface PanelStep {
  ui: PanelUi;
  /** O que chamar no core; `null` = só muda a interface. */
  effect: Extract<ApprovalEffect, { type: "resolve" }> | null;
}

/**
 * Um passo do painel. A regra do vermelho (dois cliques para aprovar) é a de
 * `decideApproval`, a mesma do toast e da Inbox — aqui só se traduz o efeito
 * em estado do painel.
 */
export function panelStep(
  request: ApprovalRequest,
  ui: PanelUi,
  event: PanelEvent,
): PanelStep {
  switch (event.type) {
    case "escape":
      // Esc não fecha nem decide (CA-09). O painel só sai com `resolved`.
      return { ui, effect: null };
    case "typeFeedback":
      return { ui: { ...ui, feedback: event.text }, effect: null };
    case "cancelFeedback":
      return { ui: { ...ui, feedbackOpen: false, feedback: "" }, effect: null };
    case "sendFeedback":
      return resolveVia(request, ui, "denied", ui.feedback);
    case "action":
      switch (event.action) {
        case "denyWithReason":
          return {
            ui: { ...ui, confirming: false, feedbackOpen: true },
            effect: null,
          };
        case "deny":
          return resolveVia(request, ui, "denied");
        case "approve":
          return resolveVia(request, ui, "approved");
        case "alwaysAllow":
          // Vermelho nunca oferece (approvalActions('red')); se chegar,
          // não vira nada.
          return { ui, effect: null };
      }
  }
}

function resolveVia(
  request: ApprovalRequest,
  ui: PanelUi,
  decision: "approved" | "denied",
  feedback?: string,
): PanelStep {
  const effect = decideApproval({
    request,
    decision,
    confirmingId: ui.confirming ? request.id : null,
    feedback,
  });
  if (effect.type === "armRedConfirm") {
    return { ui: { ...ui, confirming: true }, effect: null };
  }
  return { ui, effect };
}
