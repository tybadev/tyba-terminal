import type { ApprovalRequest, DrawerTab, SessionId, SubagentRun } from "./ipc";
import type { PauseReason } from "./promptQueue";

/**
 * A gaveta Fila/Agentes do pane (quadros 4A′–4E de GavetaFilaAgentes). Abrir,
 * recolher e a aba escolhida pelo dono vêm do core (`DrawerState`); aqui só se
 * deriva o que a pílula e as abas mostram.
 */

export type RoundProgress = {
  /** Concluídos de verdade: interrompido e parado não contam. */
  done: number;
  total: number;
  /** A rodada não assentou (mesma regra do core: algum terminou e nenhum roda). */
  running: boolean;
  interrupted: number;
};

/** Concluídos / total da rodada atual (a mais nova). `null` sem subagentes. */
export function roundProgress(subagents: readonly SubagentRun[]): RoundProgress | null {
  if (subagents.length === 0) return null;
  const round = Math.max(...subagents.map((s) => s.round));
  let done = 0;
  let total = 0;
  let interrupted = 0;
  let anyDone = false;
  let anyRunning = false;
  for (const s of subagents) {
    if (s.round !== round) continue;
    total += 1;
    if (s.status === "running") anyRunning = true;
    if (s.status !== "done") continue;
    anyDone = true;
    if (s.interrupted) interrupted += 1;
    else if (!s.blocked) done += 1;
  }
  return { done, total, running: !(anyDone && !anyRunning), interrupted };
}

/** Pedidos pendentes dos subagentes desta sessão — vermelho incluso: ele
 *  também pede o dono, só não se decide na gaveta. */
export function pendingSubagentApprovals(
  sessionId: SessionId,
  subagents: readonly SubagentRun[],
  approvals: readonly ApprovalRequest[],
): number {
  const ids = new Set(subagents.map((s) => s.agent_id).filter(Boolean));
  return approvals.filter(
    (a) => a.session_id === sessionId && a.agent_id != null && ids.has(a.agent_id),
  ).length;
}

export type PillState =
  | { kind: "run" }
  | { kind: "ok" }
  | { kind: "attn"; count: number }
  | { kind: "fail"; count: number };

export type PillAgents = { done: number; total: number; state: PillState | null };

/**
 * A parte "Agentes x/y + estado" da pílula; `null` quando a sessão não tem
 * subagentes (a pílula é só "Fila · N"). Atenção vence tudo; o ✓ e o vermelho
 * do fim da rodada ficam até o dono abrir a gaveta.
 */
export function drawerPill(
  subagents: readonly SubagentRun[],
  pending: number,
  outcomeSeen: boolean,
): PillAgents | null {
  const progress = roundProgress(subagents);
  if (!progress) return null;
  const { done, total } = progress;
  if (pending > 0) return { done, total, state: { kind: "attn", count: pending } };
  if (progress.running) return { done, total, state: { kind: "run" } };
  if (outcomeSeen) return { done, total, state: null };
  if (progress.interrupted > 0) {
    return { done, total, state: { kind: "fail", count: progress.interrupted } };
  }
  return { done, total, state: { kind: "ok" } };
}

/**
 * Qual aba a gaveta mostra (quadro 4C): a do dono, se ele escolheu (o core a
 * zera quando a gaveta recolhe); senão a que pede você (aprovação pendente →
 * Agentes), Agentes enquanto a rodada roda e Fila fora dela. Sem subagentes
 * não há aba: é a fila.
 */
export function drawerTab(input: {
  owner: DrawerTab | null;
  hasSubagents: boolean;
  pending: number;
  running: boolean;
}): DrawerTab {
  if (!input.hasSubagents) return "queue";
  if (input.owner) return input.owner;
  return input.pending > 0 || input.running ? "agents" : "queue";
}

/** A fila pausada por aprovação de subagente diz "ver em Agentes". */
export function queueGoesToAgents(paused: PauseReason | null, pending: number): boolean {
  return paused === "approval" && pending > 0;
}
