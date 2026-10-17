import type { IBufferCell, IBufferLine, ILink, ILinkProvider, Terminal } from "@xterm/xterm";

import { isSafeExternalUrl } from "./clipboard";

const URL_PATTERN =
  "(https?):[/]{2}[^\\s\"'!*(){}|\\\\^<>`]*[^\\s\"':,.!?{}|\\\\^~\\[\\]`()<>]";

const MAX_LOGICAL_LENGTH = 4096;

export interface UrlMatch {
  url: string;
  start: number;
  end: number;
}

export interface WrappedRow {
  text: string;
  isWrapped: boolean;
}

export interface LinkModifierEvent {
  metaKey: boolean;
  ctrlKey: boolean;
}

export interface TerminalLinkCallbacks {
  activate: (event: MouseEvent, uri: string) => void;
  hover: (event: MouseEvent, uri: string) => void;
  leave: (event: MouseEvent) => void;
}

export function hasOpenModifier(
  event: LinkModifierEvent,
  isMac: boolean,
): boolean {
  return isMac ? event.metaKey : event.ctrlKey;
}

export function findUrlsInLine(line: string): UrlMatch[] {
  const regex = new RegExp(URL_PATTERN, "gi");
  const matches: UrlMatch[] = [];
  let match: RegExpExecArray | null;
  while ((match = regex.exec(line)) !== null) {
    const url = match[0];
    if (regex.lastIndex === match.index) regex.lastIndex++;
    if (!isSafeExternalUrl(url)) continue;
    matches.push({ url, start: match.index, end: match.index + url.length });
  }
  return matches;
}

/** Prefixo do "link" de caminho: não é URL, e por isso nunca vai ao navegador. */
export const PATH_URI_PREFIX = "tyba-path:";

export interface PathMatch {
  path: string;
  line?: number;
  column?: number;
  start: number;
  end: number;
}

// Um caminho de arquivo como ferramenta imprime: `src/app.tsx`, `./a/b.rs:12`,
// `/abs/x.py:3:7`. A extensão começa com letra — `1.2.3` é versão, não arquivo.
const PATH_PATTERN =
  /(?<![\w/.~-])((?:~|\.{1,2})?\/?(?:[\w@.-]+\/)*[\w@-][\w@.-]*\.[A-Za-z][A-Za-z0-9]{0,7})(?::(\d+)(?::(\d+))?)?(?![\w/])/g;

/**
 * Caminhos de arquivo numa linha. Para não acender toda palavra com ponto, só
 * conta quem tem uma barra no caminho ou um `:linha` depois — `e.g.` e
 * `package.json` soltos no meio de uma frase ficam de fora; `src/x.ts` e
 * `x.ts:12` entram. O que está dentro de uma URL não é caminho.
 */
export function findPathsInLine(line: string): PathMatch[] {
  const urls = findUrlsInLine(line);
  const inUrl = (start: number, end: number) => urls.some((u) => start < u.end && end > u.start);
  const found: PathMatch[] = [];
  for (const m of line.matchAll(PATH_PATTERN)) {
    const start = m.index ?? 0;
    const end = start + m[0].length;
    const path = m[1];
    if (!path.includes("/") && !m[2]) continue;
    if (inUrl(start, end)) continue;
    found.push({
      path,
      line: m[2] ? Number(m[2]) : undefined,
      column: m[3] ? Number(m[3]) : undefined,
      start,
      end,
    });
  }
  return found;
}

/** `tyba-path:src/a.ts:12` → `{ path, line }`. */
export function parsePathUri(uri: string): { path: string; line?: number } | null {
  if (!uri.startsWith(PATH_URI_PREFIX)) return null;
  const rest = uri.slice(PATH_URI_PREFIX.length);
  const m = /^(.*?)(?::(\d+))?$/.exec(rest);
  if (!m || !m[1]) return null;
  return { path: m[1], line: m[2] ? Number(m[2]) : undefined };
}

export function logicalLineWindow(
  rows: ReadonlyArray<{ isWrapped: boolean }>,
  index: number,
): { start: number; end: number } {
  let start = index;
  while (start > 0 && rows[start]?.isWrapped) start--;
  let end = index;
  while (end + 1 < rows.length && rows[end + 1]?.isWrapped) end++;
  return { start, end };
}

export function findUrlsAcrossRows(
  rows: ReadonlyArray<WrappedRow>,
  index: number,
): UrlMatch[] {
  if (index < 0 || index >= rows.length) return [];
  const { start, end } = logicalLineWindow(rows, index);
  let joined = "";
  for (let i = start; i <= end; i++) joined += rows[i].text;
  return findUrlsInLine(joined);
}

/**
 * Uma linha do buffer como o detector de links a enxerga: célula nunca escrita
 * vira espaço (o Ink do Claude pula trecho em branco movendo o cursor, e sem o
 * espaço duas palavras viravam uma só), e as células vazias do fim ficam de
 * fora.
 */
export interface LinkRow {
  text: string;
  isWrapped: boolean;
  /** Célula logo depois do último caractere não-branco. Sem ele, o comprimento do texto aparado. */
  endCol?: number;
}

/** Posição de um caractere: `row` indexa o array de linhas, `index` o texto dela. */
export interface RowPos {
  row: number;
  index: number;
}

/** `end` é o último caractere do link (inclusive). */
export interface RowLink {
  uri: string;
  start: RowPos;
  end: RowPos;
}

/**
 * Quanto antes da borda direita uma linha pode acabar e ainda contar como
 * "cheia". O TUI (Claude Code, Ink) quebra a URL longa ele mesmo, com `\n` e
 * recuo — não é o wrap do xterm, então `isWrapped` vem falso e a URL chegava
 * cortada no navegador. A linha cortada encosta na largura de wrap do TUI, que
 * fica a poucas colunas da borda (prefixo `⏺ ` e margem).
 */
const HARD_WRAP_MARGIN = 6;
const URL_TAIL = /https?:\/\/\S*$/i;
const URL_CONTINUATION = /^[^\s"'<>`{}|\\^]+$/;

/**
 * A linha `next` continua a URL que termina `segment`, quebrada pelo próprio
 * programa? Devolve quantos caracteres de recuo pular em `next`, ou `null`.
 *
 * Só quando a linha anterior chega perto da borda direita E termina dentro de
 * uma URL, e a seguinte começa com um pedaço sem espaço que não é URL nova.
 * Prosa que por acaso termina numa URL longe da borda não junta.
 */
function hardWrapLead(
  segment: string,
  prevEnd: number,
  next: LinkRow,
  cols: number,
): number | null {
  if (prevEnd < cols - HARD_WRAP_MARGIN) return null;
  if (!URL_TAIL.test(segment.trimEnd())) return null;
  const rest = next.text.trimStart();
  const token = /^\S+/.exec(rest)?.[0];
  if (!token || /^https?:\/\//i.test(token)) return null;
  if (!URL_CONTINUATION.test(token)) return null;
  return next.text.length - rest.length;
}

/**
 * Links (URL e, com `files`, caminho) do trecho que contém a linha `target`.
 *
 * O trecho junta a linha às vizinhas quebradas pelo xterm (`isWrapped`) e às
 * quebradas pelo programa no meio de uma URL (ver `hardWrapLead`), para o
 * link valer inteiro de qualquer linha em que o mouse estiver.
 */
export function findLinksInRows(
  rows: ReadonlyArray<LinkRow>,
  target: number,
  cols: number,
  options?: { files?: boolean },
): RowLink[] {
  if (target < 0 || target >= rows.length) return [];
  const endOf = (r: LinkRow) => r.endCol ?? r.text.trimEnd().length;

  let text = "";
  let map: RowPos[] = [];
  let first = 0;
  let found: { text: string; map: RowPos[] } | null = null;
  const append = (row: number, from: number) => {
    const t = rows[row].text;
    for (let i = from; i < t.length; i++) map.push({ row, index: i });
    text += t.slice(from);
  };
  for (let i = 0; i <= rows.length; i++) {
    let lead: number | null = null;
    if (i < rows.length && i > first) {
      lead = rows[i].isWrapped ? 0 : hardWrapLead(text, endOf(rows[i - 1]), rows[i], cols);
    }
    if (lead !== null) {
      append(i, lead);
      continue;
    }
    if (i > target) {
      found = { text, map };
      break;
    }
    if (i === rows.length) break;
    text = "";
    map = [];
    first = i;
    append(i, 0);
  }
  if (!found) return [];

  const spans: Array<{ start: number; end: number; uri: string }> = findUrlsInLine(found.text).map(
    (m) => ({ start: m.start, end: m.end, uri: m.url }),
  );
  if (options?.files) {
    for (const p of findPathsInLine(found.text)) {
      spans.push({
        start: p.start,
        end: p.end,
        uri: `${PATH_URI_PREFIX}${p.path}${p.line ? `:${p.line}` : ""}`,
      });
    }
  }
  const links: RowLink[] = [];
  for (const s of spans) {
    const start = found.map[s.start];
    const end = found.map[s.end - 1];
    if (start && end) links.push({ uri: s.uri, start, end });
  }
  return links;
}

/** Intervalo de células de um link, 1-based e inclusivo, como o xterm usa. */
export interface CellRange {
  start: { x: number; y: number };
  end: { x: number; y: number };
}

export function rangeContains(range: CellRange, x: number, y: number): boolean {
  if (y < range.start.y || y > range.end.y) return false;
  if (y === range.start.y && x < range.start.x) return false;
  if (y === range.end.y && x > range.end.x) return false;
  return true;
}

/** Célula (0-based, relativa à viewport) sob um ponto da tela, ou `null` fora dela. */
export function cellAtPoint(
  point: { clientX: number; clientY: number },
  box: { left: number; top: number; width: number; height: number },
  cols: number,
  rows: number,
): { col: number; row: number } | null {
  if (cols <= 0 || rows <= 0 || box.width <= 0 || box.height <= 0) return null;
  const dx = point.clientX - box.left;
  const dy = point.clientY - box.top;
  if (dx < 0 || dy < 0 || dx >= box.width || dy >= box.height) return null;
  return {
    col: Math.floor((dx / box.width) * cols),
    row: Math.floor((dy / box.height) * rows),
  };
}

/**
 * O clique abre o link em vez de ir para o terminal?
 *
 * Vale com ou sem mouse tracking. Com o tracking ligado (o Claude Code em tela
 * cheia liga 1000/1002/1003/1006), o xterm manda o clique ao programa, que
 * redesenha a linha (a seleção dele) entre o mousedown e o mouseup; o
 * Linkifier do xterm troca o objeto do link nesse redesenho e o `activate`
 * nunca dispara. Por isso quem abre é o TYBA, no mousedown, antes do xterm.
 */
export function opensLinkOnMouseDown(
  event: LinkModifierEvent & { button: number },
  isMac: boolean,
  hasLink: boolean,
): boolean {
  return hasLink && event.button === 0 && hasOpenModifier(event, isMac);
}

/** Vizinhas lidas acima e abaixo da linha pedida, para juntar URL quebrada. */
const LINK_WINDOW_ROWS = 8;

interface ReadRow {
  row: LinkRow;
  startX: number[];
  endX: number[];
}

function readRow(line: IBufferLine, cell: IBufferCell): ReadRow {
  let text = "";
  const startX: number[] = [];
  const endX: number[] = [];
  let written = 0;
  let endCol = 0;
  for (let x = 0; x < line.length; x++) {
    line.getCell(x, cell);
    const width = cell.getWidth();
    if (width === 0) continue;
    const raw = cell.getChars();
    const chars = raw === "" ? " " : raw;
    for (let k = 0; k < chars.length; k++) {
      startX.push(x);
      endX.push(x + width);
    }
    text += chars;
    if (raw !== "") written = text.length;
    if (raw.trim() !== "") endCol = x + width;
  }
  return {
    row: { text: text.slice(0, written), isWrapped: line.isWrapped, endCol },
    startX: startX.slice(0, written),
    endX: endX.slice(0, written),
  };
}

/** Links da linha `y` do buffer (1-based, como o `provideLinks` do xterm). */
export function linksAtBufferLine(
  term: Terminal,
  y: number,
  files: boolean,
): Array<{ uri: string; range: CellRange }> {
  const buffer = term.buffer.active;
  const anchor = y - 1;
  if (!buffer.getLine(anchor)) return [];
  let from = Math.max(0, anchor - LINK_WINDOW_ROWS);
  let climbed = 0;
  while (from > 0 && buffer.getLine(from)?.isWrapped && climbed < MAX_LOGICAL_LENGTH) {
    from--;
    climbed += term.cols;
  }
  let to = anchor + LINK_WINDOW_ROWS;
  let accumulated = 0;
  while (buffer.getLine(to + 1)?.isWrapped && accumulated < MAX_LOGICAL_LENGTH) {
    to++;
    accumulated += term.cols;
  }
  const cell: IBufferCell = buffer.getNullCell();
  const read: ReadRow[] = [];
  for (let ly = from; ly <= to; ly++) {
    const line = buffer.getLine(ly);
    if (!line) break;
    read.push(readRow(line, cell));
  }
  return findLinksInRows(
    read.map((r) => r.row),
    anchor - from,
    term.cols,
    { files },
  ).map((link) => ({
    uri: link.uri,
    range: {
      start: { x: read[link.start.row].startX[link.start.index] + 1, y: from + link.start.row + 1 },
      end: { x: read[link.end.row].endX[link.end.index], y: from + link.end.row + 1 },
    },
  }));
}

export function createTerminalLinkProvider(
  term: Terminal,
  callbacks: TerminalLinkCallbacks,
  options?: { files?: () => boolean },
): ILinkProvider {
  return {
    provideLinks(y, callback) {
      const links: ILink[] = linksAtBufferLine(term, y, options?.files?.() ?? false).map(
        ({ uri, range }) => ({
          text: uri,
          range,
          activate: (event) => callbacks.activate(event, uri),
          hover: (event) => callbacks.hover(event, uri),
          leave: (event) => callbacks.leave(event),
        }),
      );
      callback(links.length > 0 ? links : undefined);
    },
  };
}
