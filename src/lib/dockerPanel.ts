import type { ContainerInfo } from "./ipc";

/**
 * Imagem curta para a linha: sem o digest (`@sha256:…`), que é o que
 * espremia o nome do container. `full` vai para o tooltip.
 */
export function shortImage(image: string): { short: string; full: string } {
  const full = image.trim();
  const noDigest = full.replace(/@sha256:[0-9a-f]+$/i, "");
  // Imagem referida só por id ("sha256:abc…"): 12 primeiros, como o docker.
  const short = /^sha256:[0-9a-f]+$/i.test(noDigest)
    ? noDigest.slice(0, "sha256:".length + 12)
    : noDigest;
  return { short, full };
}

/** Portas publicadas como lista (para chips): ["3000", "5432"]. */
export function portList(ports: string): string[] {
  const seen: string[] = [];
  for (const part of ports.split(",")) {
    const match = /:(\d+)->/.exec(part.trim());
    if (match && !seen.includes(match[1])) seen.push(match[1]);
  }
  return seen;
}

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const UNIT: Record<string, number> = {
  second: SECOND,
  minute: MINUTE,
  hour: HOUR,
  day: DAY,
  week: 7 * DAY,
  month: 30 * DAY,
  year: 365 * DAY,
};

/**
 * O trecho de tempo que o `docker ps` escreve ("26 hours", "About an hour",
 * "Less than a second", "a minute") em ms. Não reconhecido → null.
 */
export function parseDockerSpan(text: string): number | null {
  const t = text.trim().toLowerCase();
  if (/^less than a second$/.test(t)) return 0;
  const m = /^(?:about )?(an?|\d+) (second|minute|hour|day|week|month|year)s?$/.exec(t);
  if (!m) return null;
  const n = m[1] === "a" || m[1] === "an" ? 1 : Number(m[1]);
  return n * UNIT[m[2]];
}

/** "Up 26 hours (healthy)" → há quanto tempo está de pé, em ms. */
export function uptimeMs(status: string): number | null {
  const m = /^Up (.+?)(?: \(.*\))?$/.exec(status.trim());
  return m ? parseDockerSpan(m[1]) : null;
}

/** "Exited (0) 6 weeks ago" → há quanto tempo parou, em ms. */
export function exitedAgoMs(status: string): number | null {
  const m = /^Exited \(-?\d+\) (.+) ago$/.exec(status.trim());
  return m ? parseDockerSpan(m[1]) : null;
}

/**
 * Duração compacta, sem quebrar linha: "45 s", "3 min", "26 h", "6 d".
 * Horas até 48 h, como o próprio docker (que só passa a dias em "2 days").
 */
export function compactSpan(ms: number | null | undefined): string | null {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return null;
  if (ms < MINUTE) return `${Math.floor(ms / SECOND)} s`;
  if (ms < HOUR) return `${Math.floor(ms / MINUTE)} min`;
  if (ms < 2 * DAY) return `${Math.floor(ms / HOUR)} h`;
  return `${Math.floor(ms / DAY)} d`;
}

export type Health = "healthy" | "unhealthy" | "starting";

export function healthOf(status: string): Health | null {
  if (/\(healthy\)/.test(status)) return "healthy";
  if (/\(unhealthy\)/.test(status)) return "unhealthy";
  if (/\(health: starting\)/.test(status)) return "starting";
  return null;
}

/** Parado há mais que isso some atrás de "mostrar antigos". */
export const OLD_EXITED_MS = 7 * DAY;

export type ContainerGroup = {
  /** `null` = "Avulsos" (sem projeto compose). */
  project: string | null;
  inRepo: boolean;
  running: ContainerInfo[];
  /** Parados recentes, mais recentes primeiro. */
  stopped: ContainerInfo[];
  /** Parados há mais de `OLD_EXITED_MS`. */
  old: ContainerInfo[];
};

export const displayName = (c: ContainerInfo) => c.service ?? c.name;

export function matchesQuery(c: ContainerInfo, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return [c.name, c.service, c.image, c.compose_project]
    .filter((v): v is string => Boolean(v))
    .some((v) => v.toLowerCase().includes(q));
}

/**
 * Agrupa por projeto compose: o do repo primeiro, depois os outros em ordem
 * alfabética, "Avulsos" por último. Dentro: rodando (por nome), parados
 * recentes (mais novo primeiro) e antigos.
 */
export function groupContainers(
  list: readonly ContainerInfo[],
  query = "",
): ContainerGroup[] {
  const groups = new Map<string, ContainerGroup>();
  const LOOSE = "\u0000avulsos";
  for (const c of list) {
    if (!matchesQuery(c, query)) continue;
    const key = c.compose_project ?? LOOSE;
    let g = groups.get(key);
    if (!g) {
      g = { project: c.compose_project, inRepo: false, running: [], stopped: [], old: [] };
      groups.set(key, g);
    }
    if (c.in_repo) g.inRepo = true;
    if (c.state === "running") g.running.push(c);
    else {
      const ago = exitedAgoMs(c.status);
      (ago !== null && ago > OLD_EXITED_MS ? g.old : g.stopped).push(c);
    }
  }
  const byName = (a: ContainerInfo, b: ContainerInfo) =>
    displayName(a).localeCompare(displayName(b));
  const byRecent = (a: ContainerInfo, b: ContainerInfo) =>
    (exitedAgoMs(a.status) ?? Infinity) - (exitedAgoMs(b.status) ?? Infinity) ||
    byName(a, b);
  for (const g of groups.values()) {
    g.running.sort(byName);
    g.stopped.sort(byRecent);
    g.old.sort(byRecent);
  }
  return [...groups.values()].sort((a, b) => {
    if (a.inRepo !== b.inRepo) return a.inRepo ? -1 : 1;
    if ((a.project === null) !== (b.project === null)) return a.project === null ? 1 : -1;
    return (a.project ?? "").localeCompare(b.project ?? "");
  });
}

export function containerCounts(list: readonly ContainerInfo[]): {
  running: number;
  stopped: number;
} {
  const running = list.filter((c) => c.state === "running").length;
  return { running, stopped: list.length - running };
}

/** `docker ps` diz "Exited (137) 2 minutes ago": o código é o que importa. */
export function exitCode(status: string): number | null {
  const match = /^Exited \((-?\d+)\)/.exec(status.trim());
  return match ? Number(match[1]) : null;
}

/**
 * Por que um container parado saiu. O `docker inspect` (core) diz se foi o
 * OOM killer; sem ele, vale o código do `docker ps`. Saída 0 é "limpa".
 */
export type ExitWhy =
  | { kind: "oom" }
  | { kind: "code"; code: number }
  | { kind: "clean" };

export function exitReason(
  inspected: { oom_killed: boolean; exit_code: number | null } | undefined,
  psCode: number | null,
): ExitWhy | null {
  if (inspected?.oom_killed) return { kind: "oom" };
  const code = inspected?.exit_code ?? psCode;
  if (code == null) return null;
  return code === 0 ? { kind: "clean" } : { kind: "code", code };
}

/** "4,1%" / "24%": inteiro a partir de 10%, uma casa abaixo. */
export function formatCpu(percent: number | null | undefined, locale?: string): string | null {
  if (percent == null || !Number.isFinite(percent) || percent < 0) return null;
  const digits = percent >= 10 ? 0 : 1;
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: digits }).format(percent)}%`;
}

/** Bytes → "182 MB" / "1,5 GB" (base 1024, rótulo curto como no mockup). */
export function formatBytes(bytes: number | null | undefined, locale?: string): string | null {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) return null;
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = value >= 10 || unit === 0 ? 0 : 1;
  return `${new Intl.NumberFormat(locale, { maximumFractionDigits: digits }).format(value)} ${units[unit]}`;
}

/**
 * A leitura da lista: o `ps -a` direto, e o `version` só quando ele falha.
 *
 * Sobre ssh cada `docker` paga a ida inteira (canal, shell e CLI do servidor),
 * e o `version` antes de todo `ps` dobrava a espera da lista remota. O `ps`
 * que responde já prova que o docker responde; o `version` fica para separar
 * "sem docker" (lista vazia, tela de indisponível) de erro (sobe a quem chamou).
 */
export async function loadContainers<T>(
  list: () => Promise<T[]>,
  available: () => Promise<boolean>,
  onStep: (cmd: "ps -a" | "version") => void,
): Promise<{ available: boolean; list: T[] }> {
  onStep("ps -a");
  try {
    return { available: true, list: await list() };
  } catch (e) {
    onStep("version");
    if (!(await available().catch(() => false))) return { available: false, list: [] };
    throw e;
  }
}

