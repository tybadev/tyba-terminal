import type {
  AwaitingReason,
  CanoFailure,
  ObservedAgent,
  Session,
  SessionStatus,
} from "./ipc";
import { sameAgentUsage } from "./agentUsage";

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

/**
 * Guarda de exaustividade: se o core ganhar um `AwaitingReason` novo, o `tsc`
 * falha aqui em vez de o motivo cair calado no rótulo de outro.
 */
const unhandledReason = (reason: never): never => {
  throw new Error(`AwaitingReason sem tratamento: ${String(reason)}`);
};

/**
 * O rótulo de cada motivo de espera. `native_prompt` tem o seu: o Claude está
 * parado no prompt nativo pedindo permissão — não é uma pergunta aberta
 * (`reply`), e tratá-lo como tal apagaria o "precisa de você" (R5).
 */
export const awaitingLabelKey = (reason: AwaitingReason): string => {
  switch (reason) {
    case "approval":
      return "sessionBlocked";
    case "native_prompt":
      return "sessionNeedsPermission";
    case "reply":
      return "sessionAwaiting";
    default:
      return unhandledReason(reason);
  }
};

/**
 * O agente está parado esperando uma **decisão** (aprovar ou negar), não uma
 * resposta em texto? Vermelho do TYBA e prompt nativo do Claude têm o mesmo
 * peso (R5): os dois bloqueiam o agente até alguém decidir.
 */
export const awaitsDecision = (reason: AwaitingReason): boolean => {
  switch (reason) {
    case "approval":
    case "native_prompt":
      return true;
    case "reply":
      return false;
    default:
      return unhandledReason(reason);
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
        dotClass: "bg-tyba-red-fill",
        textClass: "text-tyba-red",
        labelKey: "sessionFailed",
        rank: 4,
      };
    case "awaiting_input":
      return {
        dotClass:
          "bg-tyba-amber-fill [box-shadow:var(--tyba-glow-amber)] motion-safe:animate-pulse",
        textClass: "text-tyba-amber",
        labelKey: awaitingLabelKey(status.reason),
        rank: 3,
      };
    case "idle":
      if (!attention) return null;
      return {
        dotClass: "bg-tyba-green-fill",
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
 * O selo de atenção de uma aba: a sessão da aba que está esperando alguém
 * (`awaiting_input`), com a espera por decisão (vermelho, prompt nativo) antes
 * da espera por resposta. `null` quando ninguém ali espera.
 *
 * Só `awaiting_input` acende o selo — trabalhando, concluído e falhou já têm
 * sinal próprio na barra lateral; na aba o selo diz uma coisa só: "parou por
 * sua causa".
 */
export const tabAttention = (
  sessions: ReadonlyArray<Session | undefined>,
): StatusVisual | null => {
  let pick: (SessionStatus & { state: "awaiting_input" }) | null = null;
  for (const session of sessions) {
    const status = session?.status;
    if (status?.state !== "awaiting_input") continue;
    if (!pick || (awaitsDecision(status.reason) && !awaitsDecision(pick.reason))) {
      pick = status;
    }
  }
  return pick ? statusVisual(pick, false) : null;
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

export const sameCanoFailure = (
  a: CanoFailure | null | undefined,
  b: CanoFailure | null | undefined,
): boolean => {
  if (!a || !b) return !a && !b;
  return a.reason === b.reason && a.detail === b.detail;
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
    sameObserved(current.observed, incoming.observed) &&
    current.connection === incoming.connection &&
    sameCanoFailure(current.connection_failure, incoming.connection_failure) &&
    sameAgentUsage(current.agent_usage, incoming.agent_usage)
  ) {
    return null;
  }
  return {
    ...current,
    status: incoming.status,
    attention: incoming.attention,
    observed: incoming.observed ?? null,
    agent_usage: incoming.agent_usage ?? null,
    connection: incoming.connection,
    connection_failure: incoming.connection_failure ?? null,
  };
};

/**
 * Hosts com alguma SSH Session ainda em `connecting`. O card mostra
 * "Conectando…" a partir daqui — estado do core, não do clique.
 */
export const connectingHostIds = (sessions: Session[]): Set<string> => {
  const ids = new Set<string>();
  for (const s of sessions) {
    if (s.kind.type === "ssh" && s.connection === "connecting") {
      ids.add(s.kind.host_id);
    }
  }
  return ids;
};
