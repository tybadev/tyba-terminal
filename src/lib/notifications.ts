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

export interface ApprovalChoice {
  index: number;
  decision: ApprovalDecision;
  labelKey: string;
  wantsFeedback: boolean;
}

export function approvalChoices(risk: RiskLevel): ApprovalChoice[] {
  const choices: ApprovalChoice[] = [
    {
      index: 1,
      decision: "approved",
      labelKey: "approvalChoiceYes",
      wantsFeedback: false,
    },
  ];
  if (canAlwaysAllow(risk)) {
    choices.push({
      index: choices.length + 1,
      decision: "approved_always",
      labelKey: "approvalChoiceAlways",
      wantsFeedback: false,
    });
  }
  choices.push({
    index: choices.length + 1,
    decision: "denied",
    labelKey: "approvalChoiceNo",
    wantsFeedback: true,
  });
  return choices;
}

export function choiceForKey(
  risk: RiskLevel,
  key: string,
): ApprovalChoice | null {
  const index = Number.parseInt(key, 10);
  if (Number.isNaN(index)) return null;
  return approvalChoices(risk).find((c) => c.index === index) ?? null;
}

export function shouldAutoClosePopover(params: {
  open: boolean;
  previousCount: number;
  nextCount: number;
}): boolean {
  return params.open && params.previousCount > 0 && params.nextCount === 0;
}
