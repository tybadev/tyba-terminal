import type { ApprovalDecision, ApprovalRequest, RiskLevel } from "./ipc";

export type NotificationKind = "approval";

export interface ApprovalNotification {
  kind: "approval";
  id: number;
  approval: ApprovalRequest;
}

export type NotificationItem = ApprovalNotification;

export const RISK_DOT: Record<RiskLevel, string> = {
  green: "bg-tyba-green",
  yellow: "bg-tyba-amber",
  red: "bg-tyba-red",
};

export const RISK_LABEL: Record<RiskLevel, string> = {
  green: "riskGreen",
  yellow: "riskYellow",
  red: "riskRed",
};

export function toNotificationItems(
  approvals: ApprovalRequest[],
): NotificationItem[] {
  return approvals.map((approval) => ({
    kind: "approval",
    id: approval.id,
    approval,
  }));
}

export function availableApprovalActions(
  risk: RiskLevel,
): ApprovalDecision[] {
  return risk === "red"
    ? ["approved", "denied"]
    : ["approved", "denied", "approved_always"];
}

export function canAlwaysAllow(risk: RiskLevel): boolean {
  return availableApprovalActions(risk).includes("approved_always");
}

// O conjunto de ações é a fonte única para as duas superfícies (toast e
// painel de notificações) — cada uma renderiza a partir daqui, nunca lista
// os próprios rótulos, senão a divergência que motivou o unificar volta.
export type ApprovalActionId =
  | "approve"
  | "alwaysAllow"
  | "deny"
  | "denyWithReason";

export interface ApprovalAction {
  id: ApprovalActionId;
  labelKey: string;
}

export function approvalActions(risk: RiskLevel): ApprovalAction[] {
  const actions: ApprovalAction[] = [{ id: "approve", labelKey: "approve" }];
  if (canAlwaysAllow(risk)) {
    actions.push({ id: "alwaysAllow", labelKey: "alwaysAllow" });
  }
  actions.push({ id: "deny", labelKey: "deny" });
  actions.push({ id: "denyWithReason", labelKey: "approvalChoiceNo" });
  return actions;
}

// A decisão em si — risco vermelho, gate de confirmação, chamada a
// resolveApproval — é o gate de segurança e não muda com o unificar. As
// duas superfícies chamam esta mesma função pura para decidir o que fazer;
// só o efeito retornado é que cada uma executa (armar confirmação local ou
// resolver de fato), nunca a regra em si.
export type ApprovalEffect =
  | { type: "armRedConfirm"; requestId: number }
  | {
      type: "resolve";
      requestId: number;
      decision: ApprovalDecision;
      feedback?: string;
    };

export function decideApproval(params: {
  request: ApprovalRequest;
  decision: ApprovalDecision;
  confirmingId: number | null;
  feedback?: string;
}): ApprovalEffect {
  const { request, decision, confirmingId, feedback } = params;
  if (
    decision === "approved" &&
    request.risk === "red" &&
    confirmingId !== request.id
  ) {
    return { type: "armRedConfirm", requestId: request.id };
  }
  return feedback === undefined
    ? { type: "resolve", requestId: request.id, decision }
    : { type: "resolve", requestId: request.id, decision, feedback };
}

export function shouldAutoClosePopover(params: {
  open: boolean;
  previousCount: number;
  nextCount: number;
}): boolean {
  return params.open && params.previousCount > 0 && params.nextCount === 0;
}
