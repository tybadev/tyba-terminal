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

      const matches = findUrlsInLine(text);
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
