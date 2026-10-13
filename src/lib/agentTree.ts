import type { ApprovalRequest, SessionId, SubagentRun } from "./ipc";

/**
 * Luz de status de um nó da árvore de agentes (A5.3).
 *
 * - `running`: rodando (verde, pulsa)
 * - `starting`: pedido de spawn, ainda sem id (âmbar, pulsa)
 * - `awaiting`: tem aprovação pendente (âmbar fixo)
 * - `done`: concluído (apagado)
 * - `failed`: interrompido sem resposta final (vermelho)
 */
export type AgentLight = "running" | "starting" | "awaiting" | "done" | "failed";

export interface AgentTreeRow {
  key: string;
  run: SubagentRun;
  light: AgentLight;
  /** Aprovações pendentes deste subagente. */
  pending: number;
  elapsedMs: number;
}

export interface AgentTree {
  rows: AgentTreeRow[];
  /** Aprovações do agente principal, e as de subagente que não está na lista. */
  mainPending: number;
  active: number;
}

const isActive = (run: SubagentRun) =>
  run.status === "running" || run.status === "starting";

const lightOf = (run: SubagentRun, pending: number): AgentLight => {
  if (pending > 0) return "awaiting";
  if (run.status === "running") return "running";
  if (run.status === "starting") return "starting";
  return run.interrupted ? "failed" : "done";
};

/**
 * Monta a árvore do painel: quem roda no topo (mais novo primeiro), quem
 * terminou embaixo (mais novo primeiro). O core já manda nessa ordem; ordenar
 * aqui de novo é barato e deixa a função correta sozinha.
 */
export function buildAgentTree(
  sessionId: SessionId,
  subagents: SubagentRun[],
  approvals: ApprovalRequest[],
  now: number,
): AgentTree {
  const mine = approvals.filter((a) => a.session_id === sessionId);
  const ids = new Set(
    subagents.map((s) => s.agent_id).filter((id): id is string => id != null),
  );
  const pendingBy = new Map<string, number>();
  let mainPending = 0;
  for (const a of mine) {
    if (a.agent_id && ids.has(a.agent_id)) {
      pendingBy.set(a.agent_id, (pendingBy.get(a.agent_id) ?? 0) + 1);
    } else {
      mainPending += 1;
    }
  }

  const ordered = subagents
    .map((run, index) => ({ run, index }))
    .sort((a, b) => {
      const activeDiff = Number(isActive(b.run)) - Number(isActive(a.run));
      if (activeDiff !== 0) return activeDiff;
      const startDiff = b.run.started_at_ms - a.run.started_at_ms;
      return startDiff !== 0 ? startDiff : a.index - b.index;
    });

  const rows = ordered.map(({ run, index }) => {
    const pending = run.agent_id ? (pendingBy.get(run.agent_id) ?? 0) : 0;
    return {
      key: run.agent_id ?? `pending-${index}`,
      run,
      light: lightOf(run, pending),
      pending,
      elapsedMs: Math.max(0, (run.ended_at_ms ?? now) - run.started_at_ms),
    };
  });

  return {
    rows,
    mainPending,
    active: subagents.filter(isActive).length,
  };
}

export interface AgentRound {
  round: number;
  /** Início da rodada: o nó mais antigo dela. */
  startedAtMs: number;
  /** A rodada mais nova da sessão. */
  latest: boolean;
  rows: AgentTreeRow[];
  counts: Record<AgentLight, number>;
}

const isLive = (row: AgentTreeRow) =>
  row.light === "running" ||
  row.light === "starting" ||
  row.light === "awaiting";

/**
 * Agrupa as linhas por rodada, a mais nova no topo. Com descrições iguais
 * entre rodadas ("Teste 1" duas vezes), a lista plana parecia duplicada.
 * Dentro da rodada a ordem de `buildAgentTree` se mantém.
 */
export function groupByRound(rows: AgentTreeRow[]): AgentRound[] {
  const byRound = new Map<number, AgentTreeRow[]>();
  for (const row of rows) {
    const list = byRound.get(row.run.round) ?? [];
    list.push(row);
    byRound.set(row.run.round, list);
  }
  const rounds = [...byRound.keys()].sort((a, b) => b - a);
  return rounds.map((round, i) => {
    const list = byRound.get(round)!;
    const counts: Record<AgentLight, number> = {
      running: 0,
      starting: 0,
      awaiting: 0,
      done: 0,
      failed: 0,
    };
    for (const row of list) counts[row.light] += 1;
    return {
      round,
      startedAtMs: Math.min(...list.map((r) => r.run.started_at_ms)),
      latest: i === 0,
      rows: list,
      counts,
    };
  });
}

/**
 * Linhas que aparecem com a rodada recolhida: só as vivas (rodando, iniciando,
 * esperando aprovação). Quem roda nunca some atrás de um cabeçalho fechado.
 */
export const collapsedRows = (round: AgentRound): AgentTreeRow[] =>
  round.rows.filter(isLive);

/** Idade da rodada em unidade de exibição. */
export function roundAge(
  startedAtMs: number,
  now: number,
): { unit: "now" | "min" | "h"; count: number } {
  const seconds = Math.max(0, Math.floor((now - startedAtMs) / 1000));
  if (seconds < 60) return { unit: "now", count: 0 };
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return { unit: "min", count: minutes };
  return { unit: "h", count: Math.floor(minutes / 60) };
}

export const LIGHT_CLASS: Record<AgentLight, string> = {
  running:
    "bg-tyba-green-fill [box-shadow:var(--tyba-glow-green)] motion-safe:animate-pulse",
  starting: "bg-tyba-amber-fill motion-safe:animate-pulse",
  awaiting: "bg-tyba-amber-fill",
  done: "bg-tyba-text-faint",
  failed: "bg-tyba-red-fill",
};

export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes === 0) return `${seconds}s`;
  return `${minutes}m ${seconds.toString().padStart(2, "0")}s`;
}
