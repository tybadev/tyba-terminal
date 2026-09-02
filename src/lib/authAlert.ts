import type { AuthAlertKind, AuthPhase, Session, SessionId } from "./ipc";
import type { ToastInput } from "./toast";

/**
 * Entrega C -- mapeia `(kind, phase)` (o core manda só isso -- ver `ipc.ts`)
 * pra chave de i18n. Mesmo desenho de `sandboxWarningTitleKey`: a chave mora
 * aqui, o texto pt-BR/en mora em `i18n/index.ts`.
 *
 * `switch` EXAUSTIVO sobre `kind`, sem `default` (F4 do contrato de
 * cobertura): um `AuthAlertKind` novo sem `case` aqui quebra o typecheck
 * ("not all code paths return a value"), em vez de cair silenciosamente
 * num texto genérico que não diz o que fazer.
 *
 * `NotLoggedIn` é o único kind com DUAS mensagens -- a ação muda conforme a
 * fase: no preflight a tela de login ainda nem apareceu (o toast de
 * `agent://open-url` da Entrega B vem depois); no runtime o turno já
 * abortou, e `/login` é a única saída imediata.
 */
export function authAlertMessageKey(
  kind: AuthAlertKind,
  phase: AuthPhase,
): string {
  switch (kind) {
    case "NotLoggedIn":
      return phase === "preflight"
        ? "authAlertNotLoggedInPreflight"
        : "authAlertNotLoggedInRuntime";
    case "TokenExpiredOrRevoked":
      return "authAlertTokenExpiredOrRevoked";
    case "CreditBalanceLow":
      return "authAlertCreditBalanceLow";
    case "InvalidApiKey":
      return "authAlertInvalidApiKey";
  }
}

/**
 * F1 do contrato: o preflight vira toast sticky (tone warning), SEM ação --
 * a URL de login vem depois via `agent://open-url` da Entrega B, e este
 * toast não é ela. Sem `sticky`, o auto-dismiss de ~9s (`toastDuration`)
 * apagaria o aviso antes do dono ligar os pontos, o mesmo raciocínio de
 * segurança que `sandboxWarningToastInput` já aplica.
 */
export function authAlertToastInput(message: string): ToastInput {
  return {
    tone: "warning",
    title: message,
    sticky: true,
  };
}

/**
 * F2: o handler de `agent://auth-alert` (fase runtime) guarda a faixa por
 * `session_id` -- extraído do `useEffect` de `App.tsx` pra ficar testável
 * sem React nem IPC real. Uma sessão que já tinha uma faixa e recebe um
 * kind NOVO troca de kind (a última notícia é a que fica na tela).
 */
export function withRuntimeAuthAlert(
  prev: Map<SessionId, AuthAlertKind>,
  sessionId: SessionId,
  kind: AuthAlertKind,
): Map<SessionId, AuthAlertKind> {
  return new Map(prev).set(sessionId, kind);
}

/**
 * F3, metade "recovery": remove a faixa de toda sessão que a lista mais
 * recente reporta `running` -- o mesmo sinal de progresso que zera o
 * dedupe no core (`agent/auth_watch.rs`). Devolve a MESMA referência
 * quando nada muda, pra `setState` não disparar um re-render à toa.
 */
export function withoutRecoveredAuthAlerts(
  prev: Map<SessionId, AuthAlertKind>,
  sessions: Pick<Session, "id" | "status">[],
): Map<SessionId, AuthAlertKind> {
  if (prev.size === 0) return prev;
  let changed = false;
  const next = new Map(prev);
  for (const s of sessions) {
    if (next.has(s.id) && s.status.state === "running") {
      next.delete(s.id);
      changed = true;
    }
  }
  return changed ? next : prev;
}

/** F3, metade "dismiss": remove só a sessão pedida. */
export function withoutAuthAlert(
  prev: Map<SessionId, AuthAlertKind>,
  sessionId: SessionId,
): Map<SessionId, AuthAlertKind> {
  if (!prev.has(sessionId)) return prev;
  const next = new Map(prev);
  next.delete(sessionId);
  return next;
}
