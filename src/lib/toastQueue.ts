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

// A aprovação é acionável em exatamente UM lugar por vez. Nem sempre esse
// lugar é "todos os pendentes escondidos": o painel de notificações é 1:1
// (some tudo), mas a fila de agentes colapsa pra um pedido por sessão — só
// os ids que ELA de fato renderiza podem sumir do toast, senão um segundo
// pedido pendente da mesma sessão fica sem NENHUM ponto de ação visível
// (ver hiddenApprovalIds em lib/notifications e agentQueueVisibleApprovalIds
// em lib/agentsBoard, que decidem o conjunto). Os toasts pendentes continuam
// guardados em estado (não se perdem), só somem da tela enquanto o próprio
// id estiver no conjunto escondido; se a superfície fechar antes de a
// aprovação ser resolvida, o toast volta a aparecer.
export function visibleApprovalToasts(
  toasts: ApprovalToastItem[],
  hiddenApprovalIds: ReadonlySet<number>,
): ApprovalToastItem[] {
  if (hiddenApprovalIds.size === 0) return toasts;
  return toasts.filter((toast) => !hiddenApprovalIds.has(toast.id));
}
