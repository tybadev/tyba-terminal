import type {
  ApprovalDecision,
  ApprovalRequest,
  NotificationEntry,
  RiskLevel,
  SessionId,
} from "./ipc";

export type NotificationKind = "approval";

export interface ApprovalNotification {
  kind: "approval";
  id: number;
  approval: ApprovalRequest;
}

export type NotificationItem = ApprovalNotification;

export const RISK_DOT: Record<RiskLevel, string> = {
  green: "bg-tyba-green-fill",
  yellow: "bg-tyba-amber-fill",
  red: "bg-tyba-red-fill",
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

// A aprovação é acionável em UM lugar só, sem exceção — não só toast vs.
// painel de notificações, mas qualquer superfície que já a mostra ao vivo
// (a fila de agentes inclusa). Cada superfície aberta contribui com os ids
// que ELA de fato renderiza, nunca com "todos os pendentes" por padrão: o
// painel é 1:1 (mostra todo approval), mas a fila colapsa pra um por sessão
// — esconder todos os pendentes enquanto só a fila está aberta deixaria um
// pedido sem NENHUM ponto de ação visível (agentQueueVisibleApprovalIds em
// lib/agentsBoard é quem sabe quais são esses ids). O painel vermelho
// dentro da sessão também contribui — só com os pedidos que um painel NA
// TELA mostra (panelVisibleApprovalIds em lib/approvalPanel).
export function hiddenApprovalIds(params: {
  approvals: ApprovalRequest[];
  notificationsOpen: boolean;
  agentQueueOpen: boolean;
  agentQueueVisibleIds: ReadonlySet<number>;
  sessionPanelIds?: ReadonlySet<number>;
  /** Os que o painel de agentes NA TELA deixa aprovar na linha do subagente
   * (`agentsPanelActionableIds`). */
  agentsPanelIds?: ReadonlySet<number>;
}): Set<number> {
  const ids = new Set<number>();
  if (params.notificationsOpen) {
    for (const approval of params.approvals) ids.add(approval.id);
  }
  if (params.agentQueueOpen) {
    for (const id of params.agentQueueVisibleIds) ids.add(id);
  }
  for (const id of params.sessionPanelIds ?? []) ids.add(id);
  for (const id of params.agentsPanelIds ?? []) ids.add(id);
  return ids;
}

// --- histórico (painel Notificações, mockup Notificacoes) -------------------

export type NotificationFilter = "all" | "requests" | "errors" | "done";

/** Pedido (do TYBA ou do prompt nativo do Claude). */
export function isRequestKind(e: NotificationEntry): boolean {
  return e.kind === "approval" || e.kind === "native_approval";
}

/** Pedido pendente aparece ao vivo em "Precisa de você" (o do TYBA vem de
 *  `approvals`; o do prompt nativo, de `nativePending`), não no histórico —
 *  senão aparece duas vezes. */
export function historyEntries(entries: NotificationEntry[]): NotificationEntry[] {
  return entries.filter((e) => !(isRequestKind(e) && e.pending));
}

/** Amarelo do Claude esperando no prompt nativo. Só se decide no terminal:
 *  no painel ele só tem "Abrir na sessão". */
export function nativePending(entries: NotificationEntry[]): NotificationEntry[] {
  return entries.filter((e) => e.kind === "native_approval" && e.pending);
}

/** Grupo atenuado (mockup Notificacoes, `.done`): tudo nele já concluiu. */
export function isDoneGroup(items: NotificationEntry[]): boolean {
  return items.length > 0 && items.every((e) => e.category === "done");
}

export function filterEntries(
  entries: NotificationEntry[],
  filter: NotificationFilter,
): NotificationEntry[] {
  if (filter === "all") return entries;
  if (filter === "errors") return entries.filter((e) => e.category === "error");
  if (filter === "done") return entries.filter((e) => e.category === "done");
  return entries.filter(isRequestKind);
}

export type DayBucket = "today" | "yesterday" | "earlier";

/** Por data local, não por 24h fixas: o dia da troca de horário tem 23h ou 25h. */
export function dayBucket(atMs: number, now: number): DayBucket {
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  if (atMs >= start.getTime()) return "today";
  start.setDate(start.getDate() - 1);
  if (atMs >= start.getTime()) return "yesterday";
  return "earlier";
}

/** Quando o limite volta, no fuso local: só a hora se for nas próximas 24h,
 *  data + hora se depois — como `format_reset` no core (agent/usage.rs).
 *  `resetsAt` em segundos. */
export function resetParts(resetsAt: number, nowMs: number): { time: string; date: string | null } {
  const at = new Date(resetsAt * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  const time = `${pad(at.getHours())}:${pad(at.getMinutes())}`;
  if (resetsAt * 1000 - nowMs < 24 * 60 * 60 * 1000) return { time, date: null };
  return { time, date: `${pad(at.getDate())}/${pad(at.getMonth() + 1)}` };
}

export interface SessionGroup<T> {
  session: SessionId;
  title: string;
  agent: string | null;
  items: T[];
}

/** Agrupa por sessão mantendo a ordem de quem tem o item mais recente. */
export function groupBySession<T extends { session_id: SessionId }>(
  items: T[],
  meta: (item: T) => { title: string; agent: string | null },
): SessionGroup<T>[] {
  const groups = new Map<SessionId, SessionGroup<T>>();
  for (const item of items) {
    let g = groups.get(item.session_id);
    if (!g) {
      g = { session: item.session_id, ...meta(item), items: [] };
      groups.set(item.session_id, g);
    }
    g.items.push(item);
  }
  return [...groups.values()];
}

/** Badge do ícone: pedidos pendentes (do TYBA e do prompt nativo) + erros
 *  não lidos ("pedem você"). */
export function attentionCount(pendingApprovals: number, entries: NotificationEntry[]): number {
  return (
    pendingApprovals +
    nativePending(entries).length +
    entries.filter((e) => !e.read && e.category === "error").length
  );
}

export function unreadCount(entries: NotificationEntry[]): number {
  return entries.filter((e) => !e.read).length;
}
