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

export function shouldAutoClosePopover(params: {
  open: boolean;
  previousCount: number;
  nextCount: number;
}): boolean {
  return params.open && params.previousCount > 0 && params.nextCount === 0;
}
