import type { Session, SessionId } from "./ipc";

/** Um pedido de permissão que o core avisou (`agent://asking`). */
export interface Ask {
  body: string;
  /** Ordem de chegada: o card mostra o mais antigo. */
  at: number;
}

/** O pedido continua aberto enquanto a sessão está no prompt nativo. */
const stillAsking = (session: Session | undefined): boolean =>
  session?.status.state === "awaiting_input" &&
  session.status.reason === "native_prompt";

/** Um pendente por sessão: o pedido novo da mesma sessão só troca o texto,
 * e o de outra sessão não apaga este. */
export function recordAsk(
  asks: ReadonlyMap<SessionId, Ask>,
  sessionId: SessionId,
  body: string,
  now: number,
): Map<SessionId, Ask> {
  const next = new Map(asks);
  next.set(sessionId, { body, at: asks.get(sessionId)?.at ?? now });
  return next;
}

/** Tira o que não está mais pedindo (a sessão saiu do prompt nativo ou
 * sumiu). Devolve o mesmo mapa quando nada mudou. */
export function pruneAsks(
  asks: ReadonlyMap<SessionId, Ask>,
  sessions: readonly Session[],
): ReadonlyMap<SessionId, Ask> {
  let changed = false;
  const next = new Map(asks);
  for (const id of asks.keys()) {
    if (!stillAsking(sessions.find((s) => s.id === id))) {
      next.delete(id);
      changed = true;
    }
  }
  return changed ? next : asks;
}

/** O card de pedido: o mais antigo ainda aberto de uma sessão que não está
 * na tela. A sessão em foco já mostra o prompt; ela não ganha card. */
export function nextAsk(
  asks: ReadonlyMap<SessionId, Ask>,
  sessions: readonly Session[],
  activeSessionId: SessionId | null,
): { sessionId: SessionId; body: string } | null {
  let best: { sessionId: SessionId; body: string; at: number } | null = null;
  for (const [sessionId, ask] of asks) {
    if (sessionId === activeSessionId) continue;
    if (!stillAsking(sessions.find((s) => s.id === sessionId))) continue;
    if (!best || ask.at < best.at) best = { sessionId, ...ask };
  }
  return best ? { sessionId: best.sessionId, body: best.body } : null;
}
