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

/** "O Claude está pedindo algo · projeto · branch", no idioma da UI. */
export function askingText(
  ask: AgentAsk,
  t: (key: string, params?: Record<string, string>) => string,
): string {
  return [t("askingBody", { agent: ask.agent }), ask.project, ask.branch]
    .filter((part): part is string => Boolean(part))
    .join(" · ");
}
