import type { ObservedAgent, Session, SessionStatus } from "./ipc";

export const isFinishedStatus = (status: SessionStatus): boolean =>
  status.state === "exited" || status.state === "failed";

export const sameSessionStatus = (
  a: SessionStatus,
  b: SessionStatus,
): boolean => {
  if (a.state !== b.state) return false;
  switch (a.state) {
    case "awaiting_input": {
      const other = b as typeof a;
      return a.hint === other.hint && a.reason === other.reason;
    }
    case "idle":
      return a.summary === (b as typeof a).summary;
    case "exited":
      return a.code === (b as typeof a).code;
    case "failed":
      return a.reason === (b as typeof a).reason;
    default:
      return true;
  }
};

export interface StatusVisual {
  dotClass: string;
  textClass: string;
  labelKey: string;
  rank: number;
}

export const statusVisual = (
  status: SessionStatus,
  attention: boolean,
): StatusVisual | null => {
  switch (status.state) {
    case "failed":
      return {
        dotClass: "bg-tyba-red",
        textClass: "text-tyba-red",
        labelKey: "sessionFailed",
        rank: 4,
      };
    case "awaiting_input":
      return {
        dotClass:
          "bg-tyba-amber [box-shadow:var(--tyba-glow-amber)] motion-safe:animate-pulse",
        textClass: "text-tyba-amber",
        labelKey:
          status.reason === "approval"
            ? "sessionBlocked"
            : "sessionAwaiting",
        rank: 3,
      };
    case "idle":
      if (!attention) return null;
      return {
        dotClass: "bg-tyba-green",
        textClass: "text-tyba-green",
        labelKey: "sessionFinished",
        rank: 2,
      };
    case "running":
      return {
        dotClass:
          "bg-tyba-blue [box-shadow:var(--tyba-glow-blue)] motion-safe:animate-pulse",
        textClass: "text-tyba-blue",
        labelKey: "sessionInProgress",
        rank: 1,
      };
    case "exited":
      return null;
  }
};

/**
 * O palpite de tela mudou?
 *
 * Precisa existir porque `observed` NÃO mexe no `status`: um shell com um
 * agente cru dentro fica `running` do primeiro ao último segundo, e a atenção
 * também não se mexe. Comparar só status e atenção é declarar "nada mudou"
 * justamente quando o agente apareceu.
 */
export const sameObserved = (
  a: ObservedAgent | null | undefined,
  b: ObservedAgent | null | undefined,
): boolean => {
  if (!a || !b) return !a && !b;
  return a.agent === b.agent && a.state === b.state;
};

/**
 * A sessão atualizada, ou `null` quando o evento não traz novidade.
 *
 * Mora aqui, e não dentro do listener no `App`, por dois motivos: é decisão,
 * não renderização (princípio 1 — o webview é burro), e porque dentro do
 * `setSessions` nenhum teste alcançava. Foi exatamente ali que o `observed` se
 * perdeu: o listener comparava status e atenção para decidir se havia
 * novidade, e depois copiava **só** esses dois campos. As duas metades
 * ignoravam `observed`, então um agente descoberto na tela nunca chegava à
 * lista de agentes — enquanto a faixa âmbar, que vem por outro canal, dizia
 * que ele estava lá.
 */
export const mergeSessionUpdate = (
  current: Session,
  incoming: Session,
): Session | null => {
  // Sessão que já terminou não ressuscita por evento atrasado.
  if (isFinishedStatus(current.status) && !isFinishedStatus(incoming.status)) {
    return null;
  }
  if (
    sameSessionStatus(current.status, incoming.status) &&
    current.attention === incoming.attention &&
    sameObserved(current.observed, incoming.observed)
  ) {
    return null;
  }
  return {
    ...current,
    status: incoming.status,
    attention: incoming.attention,
    observed: incoming.observed ?? null,
  };
};
