import type { Session, SessionId } from "./ipc";

/** Sessões que valem uma pergunta ao core sobre retomar a conversa.
 *
 * Filtro barato do lado de cá para não disparar um IPC por pane: só sessão de
 * agente, já morta, e com id de conversa capturado. O veredito final continua
 * sendo do core (`can_resume_agent_session`) — é ele que sabe se o binário
 * ainda está no PATH e se a pasta sobreviveu. */
export const resumeCandidates = (sessions: Session[]): SessionId[] =>
  sessions.filter(isResumeCandidate).map((s) => s.id);

export const isResumeCandidate = (session: Session): boolean =>
  session.kind.type === "agent" &&
  (session.status.state === "exited" || session.status.state === "failed") &&
  Boolean(session.agent_conversation_id);

/** Se o convite aparece neste pane.
 *
 * `resumable` é a resposta do core; `undefined` é "ainda não perguntei" e
 * também não mostra nada — o convite só nasce depois do sim. Dispensado pelo
 * usuário some até o app reabrir. */
export const showAgentResumeInvite = (
  session: Session,
  resumable: boolean | undefined,
  dismissed: boolean,
): boolean => isResumeCandidate(session) && resumable === true && !dismissed;
