import type { ApprovalRequest } from "./ipc";

export interface ApprovalToastItem {
  id: number;
  approval: ApprovalRequest;
}

export function addApprovalToast(
  toasts: ApprovalToastItem[],
  approval: ApprovalRequest,
): ApprovalToastItem[] {
  if (toasts.some((toast) => toast.id === approval.id)) return toasts;
  return [...toasts, { id: approval.id, approval }];
}

export function removeApprovalToast(
  toasts: ApprovalToastItem[],
  id: number,
): ApprovalToastItem[] {
  if (!toasts.some((toast) => toast.id === id)) return toasts;
  return toasts.filter((toast) => toast.id !== id);
}

// A aprovação é acionável em exatamente UM lugar por vez: com o painel de
// notificações fechado, é o toast; aberto, é o painel — nunca os dois. Os
// toasts pendentes continuam guardados em estado (não se perdem), só somem
// da tela enquanto o painel estiver aberto; se ele fechar sem a aprovação
// ser resolvida, o toast volta a aparecer.
export function visibleApprovalToasts(
  toasts: ApprovalToastItem[],
  notificationsPanelOpen: boolean,
): ApprovalToastItem[] {
  return notificationsPanelOpen ? [] : toasts;
}
