// Serviços e portas: leituras do que o core manda (lista, origem, recusa).
// Origem, recusa e o encerrar são decididos no core; aqui só se escolhe o
// texto e o tom de cada coisa.

import type { KillRefusal, PortEntry, ServiceOrigin, SessionId } from "./ipc";

/** Cadência da tela aberta (a sidebar usa o poll lento do core). */
export const SCREEN_POLL_MS = 3000;
/** A tela remota é rede: mais devagar. */
export const REMOTE_POLL_MS = 10000;

export function isMine(origin: ServiceOrigin): boolean {
  return origin.kind === "session";
}

/** "Só das minhas sessões": só o que descende do shell de uma sessão. */
export function filterPorts(ports: readonly PortEntry[], onlyMine: boolean): PortEntry[] {
  return onlyMine ? ports.filter((p) => isMine(p.origin)) : [...ports];
}

export function portUrl(port: number): string {
  return `http://localhost:${port}`;
}

/** Ponto da linha: verde = sessão do TYBA ou Docker, âmbar = fora do TYBA,
 *  apagado = sistema/outro usuário/TYBA. */
export type PortTone = "green" | "amber" | "faint";

export function portTone(origin: ServiceOrigin): PortTone {
  switch (origin.kind) {
    case "session":
    case "docker":
      return "green";
    case "outside":
      return "amber";
    default:
      return "faint";
  }
}

type Translate = (key: string, options?: Record<string, unknown>) => string;

/** Texto da coluna Origem. Sessão que a lista não tem mais vira "sessão". */
export function originText(
  origin: ServiceOrigin,
  t: Translate,
  titleOf: (id: SessionId) => string | null,
): string {
  switch (origin.kind) {
    case "session":
      return titleOf(origin.session_id) ?? t("servicesOriginSession");
    case "tyba":
      return "TYBA";
    case "docker":
      return "Docker";
    case "outside":
      return t("servicesOriginOutside");
    case "system":
      return t("servicesOriginSystem");
    case "other_user":
      return t("servicesOriginOtherUser");
  }
}

const REFUSAL_KEY: Record<KillRefusal, string> = {
  pid1: "servicesRefusePid1",
  tyba: "servicesRefuseTyba",
  system: "servicesRefuseSystem",
  other_user: "servicesRefuseOtherUser",
  session_shell: "servicesRefuseSessionShell",
  docker: "servicesRefuseDocker",
  unknown: "servicesRefuseUnknown",
};

export function refusalKey(refusal: KillRefusal): string {
  return REFUSAL_KEY[refusal];
}

/** Chip da sidebar: ":3000 · :5432", até `max` portas e "+N" do resto. */
export function chipText(ports: readonly number[], max = 2): string | null {
  if (ports.length === 0) return null;
  const shown = ports.slice(0, max).map((p) => `:${p}`);
  const rest = ports.length - shown.length;
  return rest > 0 ? `${shown.join(" · ")} +${rest}` : shown.join(" · ");
}

/** As portas das sessões de um workspace, juntas e ordenadas. */
export function workspacePorts(
  bySession: Readonly<Record<SessionId, readonly number[]>>,
  sessions: Iterable<SessionId>,
): number[] {
  const all = new Set<number>();
  for (const id of sessions) for (const p of bySession[id] ?? []) all.add(p);
  return [...all].sort((a, b) => a - b);
}

/** Segundos que faltam para o SIGKILL ser oferecido (0 = já pode). */
export function forceWaitSeconds(sentAt: number, forceAfterMs: number, now: number): number {
  return Math.max(0, Math.ceil((sentAt + forceAfterMs - now) / 1000));
}


/**
 * O container que publica esta porta no host, pelo texto de portas do
 * `docker ps` ("0.0.0.0:5432->5432/tcp, :::5432->5432/tcp"). `null` se nenhum.
 */
export function containerForPort<T extends { ports: string }>(
  containers: T[],
  port: number,
): T | null {
  const published = new RegExp(`:${port}->`);
  return containers.find((c) => published.test(c.ports)) ?? null;
}
