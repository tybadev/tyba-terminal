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
