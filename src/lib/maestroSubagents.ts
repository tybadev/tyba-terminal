import { freeSpot, type Rect } from "./graphLayout";
import type { SubagentRun } from "./ipc";

/**
 * Subagentes ao vivo no canvas do Maestro (quadro MaestroSubagentes): cada
 * subagente da rodada mais nova do claude de um nó vira um nó-filho efêmero,
 * em pílula (quadro MaestroCardsD), numa coluna embaixo dele. Nada daqui
 * entra no grafo salvo.
 */

export const SUB_W = 240;
export const SUB_H = 34;
export const SUB_GAP = 10;
export const SUB_COLS = 1;
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

export interface Box {
  position: { x: number; y: number };
  width: number;
  height: number;
}

/** Onde o bloco de pílulas começaria sem ninguém no caminho: logo embaixo do pai. */
const blockStart = (parent: Box) => ({
  x: parent.position.x,
  y: parent.position.y + parent.height + SUB_TOP,
});

/** Posição da i-ésima pílula a partir do canto do bloco. */
export function pillPosition(origin: { x: number; y: number }, i: number) {
  const col = i % SUB_COLS;
  const row = Math.floor(i / SUB_COLS);
  return {
    x: origin.x + col * (SUB_W + SUB_GAP),
    y: origin.y + row * (SUB_H + SUB_GAP),
  };
}

/** Linha "rodada anterior", logo abaixo da última linha do bloco. */
export function roundsAt(origin: { x: number; y: number }, count: number) {
  const rows = Math.max(1, Math.ceil(count / SUB_COLS));
  return { x: origin.x, y: origin.y + rows * (SUB_H + SUB_GAP) };
}

/** Tamanho do bloco: as pílulas e, se houver, a linha "rodada anterior". */
export function blockSize(count: number, withRounds: boolean): { w: number; h: number } {
  const rows = Math.max(1, Math.ceil(count / SUB_COLS));
  return {
    w: SUB_COLS * SUB_W + (SUB_COLS - 1) * SUB_GAP,
    h: rows * (SUB_H + SUB_GAP) + (withRounds ? SUB_ROUNDS_H : 0),
  };
}

/**
 * Onde o bloco de subagentes cai (nos-8): embaixo do pai, andando para a
 * direita de quem estiver no caminho (o Dev server logo abaixo do Claude),
 * para as pílulas nunca taparem nem roubarem o clique de outro nó.
 */
export function blockOrigin(
  parent: Box,
  size: { w: number; h: number },
  occupied: readonly Rect[],
): { x: number; y: number } {
  return freeSpot(blockStart(parent), size, occupied, "right", SUB_GAP * 2);
}

/** Um nó que pode ter bloco: `size` nulo sem subagentes (só ocupa espaço). */
export interface BlockParent {
  id: string;
  box: Box;
  size: { w: number; h: number } | null;
}

/** Deslocamento de cada bloco em relação ao pai, do último cálculo. */
export type BlockOffsets = ReadonlyMap<string, { dx: number; dy: number }>;

/**
 * Onde cai o bloco de cada pai com subagentes, e o deslocamento de cada um
 * para o próximo quadro. O desvio (`blockOrigin`) varre todos os nós para
 * cada pai — refeito a cada quadro de arrasto, O(n²). Durante o arrasto o
 * bloco já posto segue o pai pelo deslocamento de antes; no fim do arrasto
 * (`dragging` falso) recalcula o desvio de todos. Bloco novo no meio do
 * arrasto calcula na hora.
 */
export function blockOrigins(
  parents: readonly BlockParent[],
  prev: BlockOffsets | null,
  dragging: boolean,
): { origins: Map<string, { x: number; y: number }>; offsets: BlockOffsets } {
  const origins = new Map<string, { x: number; y: number }>();
  const offsets = new Map<string, { dx: number; dy: number }>();
  let occupied: Rect[] | null = null;
  for (const p of parents) {
    if (!p.size) continue;
    const kept = dragging ? prev?.get(p.id) : undefined;
    let origin: { x: number; y: number };
    if (kept) {
      origin = { x: p.box.position.x + kept.dx, y: p.box.position.y + kept.dy };
    } else {
      occupied ??= parents.map((q) => ({
        x: q.box.position.x,
        y: q.box.position.y,
        w: q.box.width,
        h: q.box.height,
      }));
      origin = blockOrigin(p.box, p.size, occupied);
      occupied.push({ ...origin, ...p.size });
    }
    origins.set(p.id, origin);
    offsets.set(p.id, { dx: origin.x - p.box.position.x, dy: origin.y - p.box.position.y });
  }
  return { origins, offsets };
}

/** Posição da i-ésima pílula sem ninguém no caminho. */
export const gridPosition = (parent: Box, i: number) => pillPosition(blockStart(parent), i);

/** Linha "rodada anterior" sem ninguém no caminho. */
export const roundsPosition = (parent: Box, count: number) => roundsAt(blockStart(parent), count);

/**
 * O que o cartão diz embaixo: a ação em curso, o resumo, ou que parou. Pronto
 * sem resumo (o core nem sempre recebe o texto final) cai na última ação que
 * se viu e, sem nenhuma, em "terminou" — nunca um cartão mudo.
 */
export type SubagentLine =
  | { kind: "activity"; text: string }
  | { kind: "summary"; text: string }
  | { kind: "interrupted" }
  | { kind: "finished" }
  | null;

const actionText = (tool: string, detail: string | null | undefined) => {
  const d = detail?.trim();
  return d ? `${tool} · ${d}` : tool;
};

export function subagentLine(run: SubagentRun): SubagentLine {
  const state = subagentState(run);
  if (state === "running") {
    if (!run.activity_tool) return null;
    return { kind: "activity", text: actionText(run.activity_tool, run.activity_detail) };
  }
  const summary = run.summary?.trim();
  if (summary) return { kind: "summary", text: summary };
  if (state === "stopped") return { kind: "interrupted" };
  if (run.activity_tool) {
    return { kind: "activity", text: actionText(run.activity_tool, run.activity_detail) };
  }
  const last = run.recent_activity?.[0];
  if (last) return { kind: "activity", text: actionText(last.tool, last.detail) };
  return { kind: "finished" };
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
