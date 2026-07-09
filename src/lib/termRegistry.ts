import type { Terminal } from "@xterm/xterm";
import type { SearchAddon } from "@xterm/addon-search";

import type { SessionId } from "./ipc";

export interface TermEntry {
  term: Terminal;
  search: SearchAddon;
}

const registry = new Map<SessionId, TermEntry>();

export function registerTerm(id: SessionId, entry: TermEntry): void {
  registry.set(id, entry);
}

export function unregisterTerm(id: SessionId): void {
  registry.delete(id);
}

export function getTerm(id: SessionId | null): TermEntry | undefined {
  if (id === null) return undefined;
  return registry.get(id);
}

export const TERMINAL_PASTE_EVENT = "tyba:terminal-paste";

export interface TerminalPasteDetail {
  sessionId: SessionId;
  text: string;
}

export function requestTerminalPaste(detail: TerminalPasteDetail): void {
  window.dispatchEvent(
    new CustomEvent<TerminalPasteDetail>(TERMINAL_PASTE_EVENT, { detail }),
  );
}
