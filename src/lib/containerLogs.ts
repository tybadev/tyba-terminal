// Recorte de tela dos logs de container. O parse (hora, nível, frame) já veio
// do core; aqui só se decide o que a lista mostra e o que o filtro acende.

import type { LogLine } from "./ipc";

export type LevelChip = "all" | "errors" | "warnings";
export type TimeWindow = "all" | "5m" | "15m" | "1h";

export const TIME_WINDOWS: TimeWindow[] = ["all", "5m", "15m", "1h"];

const WINDOW_MS: Record<TimeWindow, number | null> = {
  all: null,
  "5m": 5 * 60_000,
  "15m": 15 * 60_000,
  "1h": 60 * 60_000,
};

/** Teto de linhas na tela; o core guarda o mesmo tanto. */
export const LOG_LINES_CAP = 5000;

export function appendCapped(lines: LogLine[], more: LogLine[]): LogLine[] {
  if (more.length === 0) return lines;
  const next = lines.concat(more);
  return next.length > LOG_LINES_CAP ? next.slice(next.length - LOG_LINES_CAP) : next;
}

/** Linhas visíveis pelo chip de nível e pela janela de tempo. Continuação
 *  (stack trace) segue o nível herdado, então "Erros" mostra o trace junto. */
export function visibleLogs(
  lines: LogLine[],
  chip: LevelChip,
  window: TimeWindow,
  now: number,
): LogLine[] {
  const span = WINDOW_MS[window];
  const since = span === null ? null : now - span;
  let lastTs: number | null = null;
  return lines.filter((line) => {
    if (line.ts_ms !== null) lastTs = line.ts_ms;
    if (since !== null && (lastTs === null || lastTs < since)) return false;
    if (chip === "errors") return line.level === "error";
    if (chip === "warnings") return line.level === "warn";
    return true;
  });
}

/** O filtro acende, não esconde: a linha de contexto em volta do erro
 *  importa. Devolve quantas casam, para o "3 de 214". */
export function matchesQuery(line: LogLine, query: string): boolean {
  const q = query.trim().toLowerCase();
  return q.length > 0 && line.text.toLowerCase().includes(q);
}

export function countMatches(lines: LogLine[], query: string): number {
  if (!query.trim()) return 0;
  return lines.reduce((n, line) => n + (matchesQuery(line, query) ? 1 : 0), 0);
}

/** Trechos de `text` com as ocorrências de `query` marcadas. */
export function highlightParts(
  text: string,
  query: string,
): { text: string; hit: boolean }[] {
  const q = query.trim().toLowerCase();
  if (!q) return [{ text, hit: false }];
  const lower = text.toLowerCase();
  const out: { text: string; hit: boolean }[] = [];
  let from = 0;
  for (;;) {
    const at = lower.indexOf(q, from);
    if (at < 0) break;
    if (at > from) out.push({ text: text.slice(from, at), hit: false });
    out.push({ text: text.slice(at, at + q.length), hit: true });
    from = at + q.length;
  }
  if (from < text.length) out.push({ text: text.slice(from), hit: false });
  return out;
}

/** Erros e avisos que ABREM um evento (sem contar as linhas do trace). */
export function levelTotals(lines: LogLine[]): { errors: number; warnings: number } {
  let errors = 0;
  let warnings = 0;
  for (const line of lines) {
    if (line.continuation) continue;
    if (line.level === "error") errors += 1;
    else if (line.level === "warn") warnings += 1;
  }
  return { errors, warnings };
}

/** `15:24:02`, no fuso de quem olha. */
export function clockOf(tsMs: number | null): string {
  if (tsMs === null) return "";
  const d = new Date(tsMs);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** Linha como vai para Copiar/Baixar (o core mascara os segredos depois). */
export function exportLine(line: LogLine, withTime: boolean): string {
  const time = withTime && line.ts ? `${line.ts} ` : "";
  return `${time}${line.text}`;
}

/** "há 3 min" a partir do `StartedAt`. */
export function sinceLabel(startedAt: string | null, now: number): number | null {
  if (!startedAt) return null;
  const t = Date.parse(startedAt);
  if (Number.isNaN(t)) return null;
  return Math.max(0, Math.floor((now - t) / 60_000));
}

/** "25 min", "2 h 5 min", "3 d 4 h" — quanto tempo o container está de pé.
 *  Minuto é a menor unidade; abaixo dela, "<1 min". */
export function formatRunningFor(minutes: number): string {
  if (minutes < 1) return "<1 min";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    const m = minutes % 60;
    return m > 0 ? `${hours} h ${m} min` : `${hours} h`;
  }
  const days = Math.floor(hours / 24);
  const h = hours % 24;
  return h > 0 ? `${days} d ${h} h` : `${days} d`;
}

/** `4.12%` do docker → `4%`. O que não for número passa como veio. */
export function compactCpu(cpu: string): string {
  const n = Number.parseFloat(cpu);
  return Number.isFinite(n) ? `${Math.round(n)}%` : cpu;
}

const MEM_UNIT: Record<string, string> = {
  b: "B",
  kib: "KB",
  kb: "KB",
  mib: "MB",
  mb: "MB",
  gib: "GB",
  gb: "GB",
  tib: "TB",
  tb: "TB",
};

/** `358.2MiB` do docker → `358 MB`; `1.52GiB` → `1.5 GB`. */
export function compactMem(mem: string): string {
  const m = /^\s*([\d.]+)\s*([a-z]+)\s*$/i.exec(mem);
  const unit = m ? MEM_UNIT[m[2].toLowerCase()] : undefined;
  if (!m || !unit) return mem;
  const n = Number.parseFloat(m[1]);
  if (!Number.isFinite(n)) return mem;
  const shown =
    n < 10 && unit !== "B" ? n.toFixed(1).replace(/\.0$/, "") : String(Math.round(n));
  return `${shown} ${unit}`;
}

// Prefixos de hora que a própria aplicação escreve na linha. Com um deles, a
// coluna de hora do TYBA (o `--timestamps` do docker) seria uma segunda hora
// repetida: a lista usa a da linha.
const ISO_PREFIX =
  /^\[?\d{4}[-/]\d{2}[-/]\d{2}[T ](\d{2}):(\d{2}):(\d{2})(?:[.,]\d+)?(?: ?(?:Z|[+-]\d{2}(?::?\d{2})?|UTC|GMT|[A-Z]{2,4}T))?\]?(?=\s|$)/;
const SYSLOG_PREFIX = /^[A-Z][a-z]{2} {1,2}\d{1,2} (\d{2}):(\d{2}):(\d{2})(?=\s|$)/;
const CLOCK_PREFIX = /^\[(\d{2}):(\d{2}):(\d{2})(?:[.,]\d+)?\](?=\s|$)/;

/** Hora que a linha já traz no começo (ISO, syslog, postgres, `[HH:MM:SS]`):
 *  devolve `HH:MM:SS` como escrita na linha e o resto da mensagem. */
export function ownTimestamp(text: string): { clock: string; rest: string } | null {
  for (const re of [ISO_PREFIX, SYSLOG_PREFIX, CLOCK_PREFIX]) {
    const m = re.exec(text);
    if (!m) continue;
    const rest = text.slice(m[0].length).replace(/^\s*(?:[-|]\s+)?/, "");
    return { clock: `${m[1]}:${m[2]}:${m[3]}`, rest };
  }
  return null;
}
