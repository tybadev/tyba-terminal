// Aba "git · histórico": só apresentação. O log, o cursor e (fatia 2) as
// lanes do grafo vêm prontos do core; aqui é tempo relativo, chip de ref e a
// decisão de pedir a próxima página.

import type { GraphEdge, GraphRow, LogPage, LogRow, RefLabel } from "./ipc";

export const HISTORY_PAGE = 200;
/** Faltando estas linhas para o fim da lista, pede a próxima página. */
export const LOAD_AHEAD = 40;

export type Relative =
  | { unit: "now" }
  | { unit: "min" | "hour" | "days"; count: number }
  | { unit: "yesterday" }
  | { unit: "date"; date: Date };

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

/** "há 4 min", "há 1 h", "ontem", "há 2 dias" — e data acima de uma semana. */
export function relativeTime(iso: string, now: number): Relative {
  const date = new Date(iso);
  const t = date.getTime();
  if (Number.isNaN(t)) return { unit: "now" };
  const diff = Math.max(0, now - t);
  if (diff < MIN) return { unit: "now" };
  if (diff < HOUR) return { unit: "min", count: Math.floor(diff / MIN) };
  if (diff < DAY) return { unit: "hour", count: Math.floor(diff / HOUR) };
  const days = Math.floor(diff / DAY);
  if (days === 1) return { unit: "yesterday" };
  if (days < 7) return { unit: "days", count: days };
  return { unit: "date", date };
}

export type ChipTone = "head" | "local" | "remote" | "tag" | "agent";

/** Cor do chip: HEAD/branch atual azul cheio, local violeta, `origin/*`
 *  azul vazado, tag neutra vazada, branch de agente (`tyba/*`) âmbar. */
export function chipTone(ref: RefLabel): ChipTone {
  if (ref.kind === "agent") return "agent";
  if (ref.kind === "head" || ref.is_head) return "head";
  if (ref.kind === "remote") return "remote";
  if (ref.kind === "tag") return "tag";
  return "local";
}

export function chipLabel(ref: RefLabel): string {
  return ref.is_head && ref.kind !== "head" ? `HEAD → ${ref.name}` : ref.name;
}

export function shortSha(sha: string): string {
  return sha.slice(0, 7);
}

/** "Guilherme Teodoro" → "GT"; nome de uma palavra só → duas letras. */
export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return (words[0][0] + words[words.length - 1][0]).toUpperCase();
}

/** Uma linha da lista: o commit e (sem filtro) a linha de grafo dele. */
export interface HistoryEntry {
  row: LogRow;
  graph: GraphRow | null;
}

export function entriesOf(page: LogPage): HistoryEntry[] {
  return page.rows.map((row, i) => ({ row, graph: page.graph[i] ?? null }));
}

/** Junta a página nova à lista. O core já não repete commit entre páginas;
 *  o filtro é cinto e suspensório para a chave do virtualizador. */
export function appendPage(
  entries: readonly HistoryEntry[],
  page: readonly HistoryEntry[],
): HistoryEntry[] {
  if (entries.length === 0) return [...page];
  const seen = new Set(entries.map((e) => e.row.sha));
  return [...entries, ...page.filter((e) => !seen.has(e.row.sha))];
}

// --- grafo ------------------------------------------------------------------

/** Geometria do mockup: colunas a cada 16px, a primeira em x=14. */
export const LANE_W = 16;
const LANE_X0 = 14;
/** Colunas desenhadas; além disso o SVG corta (repo com centenas de pontas). */
export const MAX_DRAWN_LANES = 16;

/** Cor 0 é a do agente (âmbar); as outras giram na paleta. */
export const GRAPH_COLORS = [
  "var(--tyba-amber)",
  "var(--tyba-blue)",
  "var(--tyba-violet)",
  "var(--tyba-cyan)",
  "var(--tyba-green)",
  "var(--tyba-magenta)",
  "var(--tyba-lime)",
  "var(--tyba-red)",
];

export function graphColor(index: number): string {
  if (index === 0) return GRAPH_COLORS[0];
  return GRAPH_COLORS[1 + ((index - 1) % (GRAPH_COLORS.length - 1))];
}

export function laneX(lane: number): number {
  return LANE_X0 + lane * LANE_W;
}

/** Largura da coluna do grafo para as linhas carregadas. */
export function graphColumnWidth(entries: readonly HistoryEntry[]): number {
  let lanes = 1;
  for (const e of entries) if (e.graph && e.graph.width > lanes) lanes = e.graph.width;
  return laneX(Math.min(lanes, MAX_DRAWN_LANES) - 1) + LANE_X0;
}

/** Caminho SVG de uma aresta numa linha de altura `h` (nó no meio). */
export function edgePath(edge: GraphEdge, nodeLane: number, h: number): string {
  const mid = h / 2;
  const node = laneX(nodeLane);
  switch (edge.kind) {
    case "pass": {
      const x = laneX(edge.lane);
      return `M${x} 0V${h}`;
    }
    case "in": {
      const x = laneX(edge.from);
      return x === node
        ? `M${x} 0V${mid}`
        : `M${x} 0C${x} ${mid * 0.55} ${node} ${mid * 0.45} ${node} ${mid}`;
    }
    case "out": {
      const x = laneX(edge.to);
      return x === node
        ? `M${node} ${mid}V${h}`
        : `M${node} ${mid}C${node} ${mid + mid * 0.55} ${x} ${mid + mid * 0.45} ${x} ${h}`;
    }
  }
}

export function shouldLoadMore(opts: {
  lastVisible: number;
  count: number;
  hasMore: boolean;
  loading: boolean;
}): boolean {
  return opts.hasMore && !opts.loading && opts.lastVisible >= opts.count - LOAD_AHEAD;
}

/** Segundos desde a última leitura, para o "atualizado há Ns". */
export function secondsSince(at: number | null, now: number): number | null {
  return at === null ? null : Math.max(0, Math.floor((now - at) / 1000));
}

/** Busca vazia ou só espaço é "sem filtro" — não dispara recarga. */
export function cleanFilterText(text: string): string | null {
  const trimmed = text.trim();
  return trimmed ? trimmed : null;
}

export type HistoryKeyAction =
  | { kind: "move"; index: number }
  | { kind: "open" }
  | { kind: "copy" };

/** Teclado da lista: ↑/↓ andam (presos nas pontas), Enter abre o detalhe,
 *  `c` copia o hash. Com modificador é atalho de outro lugar, não daqui. */
export function historyKey(
  e: { key: string; metaKey?: boolean; ctrlKey?: boolean; altKey?: boolean },
  index: number,
  count: number,
): HistoryKeyAction | null {
  if (count === 0 || e.metaKey || e.ctrlKey || e.altKey) return null;
  switch (e.key) {
    case "ArrowDown":
      return { kind: "move", index: index < 0 ? 0 : Math.min(index + 1, count - 1) };
    case "ArrowUp":
      return { kind: "move", index: index < 0 ? 0 : Math.max(index - 1, 0) };
    case "Enter":
      return index >= 0 ? { kind: "open" } : null;
    case "c":
      return index >= 0 ? { kind: "copy" } : null;
    default:
      return null;
  }
}
