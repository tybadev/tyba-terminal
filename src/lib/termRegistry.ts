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

export interface TerminalPasteDetail {
  sessionId: SessionId;
  text: string;
}

export function isTermFocused(entry: TermEntry | undefined): boolean {
  if (!entry?.term.textarea) return false;
  return document.activeElement === entry.term.textarea;
}

const NATIVE_PASTE_SUPPRESS_MS = 200;
let suppressNativePasteUntil = 0;

export function suppressNativePaste(): void {
  suppressNativePasteUntil = performance.now() + NATIVE_PASTE_SUPPRESS_MS;
}

export function nativePasteSuppressed(): boolean {
  return performance.now() < suppressNativePasteUntil;
}
