import type { SubagentRun } from "./ipc";

/**
 * Subagentes ao vivo no canvas do Maestro (quadro MaestroSubagentes): cada
 * subagente da rodada mais nova do claude de um nó vira um nó-filho efêmero,
 * em grade embaixo dele. Nada daqui entra no grafo salvo.
 */

export const SUB_W = 240;
export const SUB_H = 150;
export const SUB_GAP = 30;
export const SUB_COLS = 3;
/** Distância entre o fundo do nó do agente e a primeira linha da grade. */
export const SUB_TOP = 80;
export const SUB_ROUNDS_H = 24;

/** Porta invisível embaixo do nó do agente, de onde saem as arestas. */
export const SUBAGENT_HANDLE = "subagents";

/** Prefixo dos ids efêmeros: o canvas usa para deixá-los fora do modelo. */
const SUB_PREFIX = "sub:";

export const isSubagentNodeId = (id: string) => id.startsWith(SUB_PREFIX);

/** Chave estável do subagente: o id, ou (ainda sem id) quando e o que pediu. */
export const subagentKey = (run: SubagentRun) =>
  run.agent_id ?? `start-${run.started_at_ms}-${run.description}`;

export const subagentNodeId = (parentId: string, key: string) =>
  `${SUB_PREFIX}${parentId}:${key}`;

export const subagentRoundsNodeId = (parentId: string) => `${SUB_PREFIX}${parentId}:rounds`;

export type SubagentState = "running" | "done" | "stopped";

/** Estado do cartão: rodando (inclui iniciando), pronto, ou parou. */
export function subagentState(run: SubagentRun): SubagentState {
  if (run.status !== "done") return "running";
  return run.interrupted || run.blocked ? "stopped" : "done";
}

export interface SubagentRounds {
  round: number;
  /** Subagentes da rodada mais nova, na ordem em que nasceram. */
  current: SubagentRun[];
  /** Quantos subagentes ficaram nas rodadas anteriores. */
  previous: number;
}

/** Separa a rodada mais nova das anteriores. `null` sem subagente. */
export function groupRounds(runs: readonly SubagentRun[]): SubagentRounds | null {
  if (runs.length === 0) return null;
  const round = Math.max(...runs.map((r) => r.round));
  const current = runs
    .filter((r) => r.round === round)
    .sort((a, b) => a.started_at_ms - b.started_at_ms);
  return { round, current, previous: runs.length - current.length };
}

export interface RoundTally {
  running: number;
  done: number;
  stopped: number;
}

export function roundTally(runs: readonly SubagentRun[]): RoundTally {
  const tally: RoundTally = { running: 0, done: 0, stopped: 0 };
  for (const r of runs) tally[subagentState(r)] += 1;
  return tally;
}

interface Box {
  position: { x: number; y: number };
  width: number;
  height: number;
}

/** Posição do i-ésimo cartão: 3 colunas, alinhadas à esquerda do agente. */
export function gridPosition(parent: Box, i: number): { x: number; y: number } {
  const col = i % SUB_COLS;
  const row = Math.floor(i / SUB_COLS);
  return {
    x: parent.position.x + col * (SUB_W + SUB_GAP),
    y: parent.position.y + parent.height + SUB_TOP + row * (SUB_H + SUB_GAP),
  };
}

/** Linha "rodada anterior", logo abaixo da última linha da grade. */
export function roundsPosition(parent: Box, count: number): { x: number; y: number } {
  const rows = Math.max(1, Math.ceil(count / SUB_COLS));
  return {
    x: parent.position.x,
    y: parent.position.y + parent.height + SUB_TOP + rows * (SUB_H + SUB_GAP),
  };
}

/** O que o cartão diz embaixo: a ação em curso, o resumo, ou que parou. */
export type SubagentLine =
  | { kind: "activity"; text: string }
  | { kind: "summary"; text: string }
  | { kind: "interrupted" }
  | null;

export function subagentLine(run: SubagentRun): SubagentLine {
  const state = subagentState(run);
  if (state === "running") {
    if (!run.activity_tool) return null;
    const detail = run.activity_detail?.trim();
    return { kind: "activity", text: detail ? `${run.activity_tool} · ${detail}` : run.activity_tool };
  }
  const summary = run.summary?.trim();
  if (summary) return { kind: "summary", text: summary };
  return state === "stopped" ? { kind: "interrupted" } : null;
}

/** Tempo do subagente: até agora se roda, até o fim se acabou. */
export const subagentElapsed = (run: SubagentRun, now: number) =>
  Math.max(0, (run.ended_at_ms ?? now) - run.started_at_ms);

/**
 * "12s" enquanto roda; "10,4s" (uma casa, no idioma) quando acabou em menos de
 * um minuto; "3m10s" acima disso.
 */
export function formatSubagentTime(ms: number, finished: boolean, lang: string): string {
  const s = Math.max(0, ms) / 1000;
  if (s >= 60) {
    const m = Math.floor(s / 60);
    return `${m}m${String(Math.floor(s % 60)).padStart(2, "0")}s`;
  }
  if (!finished) return `${Math.floor(s)}s`;
  const n = new Intl.NumberFormat(lang, { maximumFractionDigits: 1, minimumFractionDigits: 1 });
  return `${n.format(Math.floor(s * 10) / 10)}s`;
}
