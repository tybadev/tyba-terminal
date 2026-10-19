import type { AgentAsk, SessionId } from "./ipc";

/** O card de pedido: o mais antigo da lista do core (que já vem em ordem de
 * chegada) de uma sessão que existe e não está na tela. A sessão em foco já
 * mostra o prompt; ela não ganha card. */
export function visibleAsk(
  asks: readonly AgentAsk[],
  activeSessionId: SessionId | null,
  knownSessions: ReadonlySet<SessionId>,
): AgentAsk | null {
  return (
    asks.find(
      (ask) =>
        ask.session_id !== activeSessionId && knownSessions.has(ask.session_id),
    ) ?? null
  );
}
