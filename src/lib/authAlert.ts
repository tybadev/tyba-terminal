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
 * Review round 1, Fix 2 (decisão do dono): uma sessão que SAIU por causa de
 * auth precisa mostrar a RAZÃO junto do convite de retomar -- "não morrer em
 * silêncio" é a manchete da Entrega C, e antes desta função o dono via só
 * "retomar conversa", sem saber POR QUE ela morreu. Chaves próprias (e não
 * reuso de `authAlertMessageKey`): a frase muda de registro -- não é mais
 * "o agente parou, aja agora", é "foi por isso que ele saiu, retome depois
 * de resolver".
 *
 * `switch` exaustivo, mesmo desenho de `authAlertMessageKey`.
 */
export function authAlertExitedMessageKey(kind: AuthAlertKind): string {
  switch (kind) {
    case "NotLoggedIn":
      return "authAlertExitedNotLoggedIn";
    case "TokenExpiredOrRevoked":
      return "authAlertExitedTokenExpiredOrRevoked";
    case "CreditBalanceLow":
      return "authAlertExitedCreditBalanceLow";
    case "InvalidApiKey":
      return "authAlertExitedInvalidApiKey";
  }
}

/** O que a faixa de uma sessão SAÍDA mostra -- review round 1, Fix 2.
 *
 * A razão do auth (quando existe) tem prioridade sobre o convite puro de
 * retomar: é a notícia mais importante, e as duas nunca cabem em duas
 * faixas ao mesmo tempo (`agentNotice`/`resumeNotice`/faixa de auth já são
 * mutuamente exclusivas por `exited`). `showResumeAction` separado de
 * `messageKey` porque a razão pode aparecer SEM convite de retomar (core
 * disse que não há conversa retomável, ou o dono já dispensou o convite) --
 * nesse caso a faixa vira só informativa, sem o botão de ação.
 */
export interface ExitedSessionNotice {
  tone: "cyan" | "red";
  messageKey: string;
  messageParams?: Record<string, string>;
  showResumeAction: boolean;
}

export function exitedSessionNotice(
  authAlertKind: AuthAlertKind | null,
  resumeNotice: { binary: string } | null,
): ExitedSessionNotice | null {
  if (authAlertKind) {
    return {
      tone: "red",
      messageKey: authAlertExitedMessageKey(authAlertKind),
      showResumeAction: resumeNotice !== null,
    };
  }
  if (resumeNotice) {
    return {
      tone: "cyan",
      messageKey: "agentResumeNotice",
      messageParams: { binary: resumeNotice.binary },
      showResumeAction: true,
    };
  }
  return null;
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
 * F3, metade "recovery" -- e, desde o review round 1 (Fix 1), a limpeza de
 * entrada ÓRFÃ. Duas razões distintas pra uma entrada sumir, e as duas
 * precisam de tratamento aqui:
 *
 * - **Recovery**: a sessão segue na lista e voltou a `running` -- o mesmo
 *   sinal de progresso que zera o dedupe no core (`agent/auth_watch.rs`).
 * - **Órfã** (achado do reviewer): o core pode emitir `{Runtime, kind}` pro
 *   settle que acorda com `sessions.get` já devolvendo `None` -- "ausente"
 *   é um dos braços de R6 (`absent_session_at_settle_time_emits_runtime_alert`
 *   no core), e é real: o dono fecha/descarta uma sessão travada ANTES dos
 *   2500ms do settle. O evento chega, o front grava a entrada, e sem esta
 *   limpeza ela nunca mais sai do `Map` -- nenhum "recovery" é possível pra
 *   uma sessão que não existe mais. Cresce sem teto numa sessão longa com
 *   vários agentes travados-e-fechados.
 *
 * A sessão `exited` mas AINDA presente na lista (`sessions`, que guarda as
 * mortas -- ver `SessionManager::restore`) NÃO é órfã: essa entrada
 * sobrevive de propósito, porque é ela que alimenta a razão do auth na
 * faixa de "saiu por quê" (`exitedSessionNotice`, Fix 2). Só quem SOME da
 * lista de verdade (sessão descartada) é limpo aqui.
 *
 * Devolve a MESMA referência quando nada muda, pra `setState` não disparar
 * um re-render à toa.
 */
export function withoutRecoveredAuthAlerts(
  prev: Map<SessionId, AuthAlertKind>,
  sessions: Pick<Session, "id" | "status">[],
): Map<SessionId, AuthAlertKind> {
  if (prev.size === 0) return prev;
  const byId = new Map(sessions.map((s) => [s.id, s]));
  let changed = false;
  const next = new Map(prev);
  for (const sessionId of prev.keys()) {
    const session = byId.get(sessionId);
    if (!session || session.status.state === "running") {
      next.delete(sessionId);
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
