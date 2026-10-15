import type { IBufferCell, ILink, ILinkProvider, Terminal } from "@xterm/xterm";

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

interface CellRef {
  y: number;
  startX: number;
  endX: number;
}

export function createTerminalLinkProvider(
  term: Terminal,
  callbacks: TerminalLinkCallbacks,
  options?: { files?: () => boolean },
): ILinkProvider {
  return {
    provideLinks(y, callback) {
      const buffer = term.buffer.active;
      const anchorIndex = y - 1;
      if (!buffer.getLine(anchorIndex)) {
        callback(undefined);
        return;
      }

      let startIndex = anchorIndex;
      let climbed = 0;
      while (
        startIndex > 0 &&
        buffer.getLine(startIndex)?.isWrapped &&
        climbed < MAX_LOGICAL_LENGTH
      ) {
        startIndex--;
        climbed += term.cols;
      }
      let endIndex = anchorIndex;
      let accumulated = 0;
      while (accumulated < MAX_LOGICAL_LENGTH) {
        const next = buffer.getLine(endIndex + 1);
        if (!next?.isWrapped) break;
        endIndex++;
        accumulated += term.cols;
      }

      let text = "";
      const cellRefs: CellRef[] = [];
      const cell: IBufferCell = buffer.getNullCell();
      for (let ly = startIndex; ly <= endIndex; ly++) {
        const line = buffer.getLine(ly);
        if (!line) continue;
        for (let x = 0; x < line.length; x++) {
          line.getCell(x, cell);
          const width = cell.getWidth();
          if (width === 0) continue;
          const chars = cell.getChars();
          if (chars === "") continue;
          for (let k = 0; k < chars.length; k++) {
            cellRefs.push({ y: ly, startX: x, endX: x + width });
          }
          text += chars;
        }
      }

      const matches = findUrlsInLine(text).map((m) => ({ start: m.start, end: m.end, url: m.url }));
      if (options?.files?.()) {
        for (const p of findPathsInLine(text)) {
          matches.push({
            start: p.start,
            end: p.end,
            url: `${PATH_URI_PREFIX}${p.path}${p.line ? `:${p.line}` : ""}`,
          });
        }
      }
      if (matches.length === 0) {
        callback(undefined);
        return;
      }

      const links: ILink[] = [];
      for (const found of matches) {
        const first = cellRefs[found.start];
        const last = cellRefs[found.end - 1];
        if (!first || !last) continue;
        const uri = found.url;
        links.push({
          text: uri,
          range: {
            start: { x: first.startX + 1, y: first.y + 1 },
            end: { x: last.endX, y: last.y + 1 },
          },
          activate: (event) => callbacks.activate(event, uri),
          hover: (event) => callbacks.hover(event, uri),
          leave: (event) => callbacks.leave(event),
        });
      }
      callback(links.length > 0 ? links : undefined);
    },
  };
}
