import type { SessionKind } from "./ipc";

export const RICH_INPUT_PREF_KEY = "pref.richInput";

export interface RichInputPref {
  version: number;
  autoShow: boolean;
  autoOpenOnStart: boolean;
  autoDismiss: boolean;
  submitWithCtrlEnter: boolean;
  warnOnSensitivePrompt: boolean;
  showOnMatch: boolean;
  agentRegex: string;
}

export const DEFAULT_RICH_INPUT: RichInputPref = {
  version: 1,
  autoShow: true,
  autoOpenOnStart: false,
  autoDismiss: true,
  submitWithCtrlEnter: false,
  warnOnSensitivePrompt: true,
  showOnMatch: true,
  agentRegex: "",
};

const bool = (value: unknown, fallback: boolean): boolean =>
  typeof value === "boolean" ? value : fallback;

export function parseRichInputPref(raw: string | null): RichInputPref {
  if (!raw) return DEFAULT_RICH_INPUT;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return DEFAULT_RICH_INPUT;
  }
  if (typeof parsed !== "object" || parsed === null) return DEFAULT_RICH_INPUT;
  const pref = parsed as Record<string, unknown>;
  const version = typeof pref.version === "number" ? pref.version : 0;
  if (version > DEFAULT_RICH_INPUT.version) return DEFAULT_RICH_INPUT;
  return {
    version: DEFAULT_RICH_INPUT.version,
    autoShow: bool(pref.autoShow, DEFAULT_RICH_INPUT.autoShow),
    autoOpenOnStart: bool(
      pref.autoOpenOnStart,
      DEFAULT_RICH_INPUT.autoOpenOnStart,
    ),
    autoDismiss: bool(pref.autoDismiss, DEFAULT_RICH_INPUT.autoDismiss),
    submitWithCtrlEnter: bool(
      pref.submitWithCtrlEnter,
      DEFAULT_RICH_INPUT.submitWithCtrlEnter,
    ),
    warnOnSensitivePrompt: bool(
      pref.warnOnSensitivePrompt,
      DEFAULT_RICH_INPUT.warnOnSensitivePrompt,
    ),
    showOnMatch: bool(pref.showOnMatch, DEFAULT_RICH_INPUT.showOnMatch),
    agentRegex:
      typeof pref.agentRegex === "string"
        ? pref.agentRegex
        : DEFAULT_RICH_INPUT.agentRegex,
  };
}

export type EnterAction = "submit" | "newline" | "none";

export function enterAction(
  mods: { shift: boolean; ctrlOrMeta: boolean },
  submitWithCtrlEnter: boolean,
): EnterAction {
  if (mods.shift) return "newline";
  if (mods.ctrlOrMeta) return submitWithCtrlEnter ? "submit" : "none";
  return submitWithCtrlEnter ? "newline" : "submit";
}

export interface AtQuery {
  start: number;
  query: string;
}

export function atQuery(text: string, caret: number): AtQuery | null {
  const before = text.slice(0, caret);
  const at = before.lastIndexOf("@");
  if (at === -1) return null;
  if (at > 0 && !/\s/.test(before[at - 1])) return null;
  const query = before.slice(at + 1);
  if (/\s/.test(query)) return null;
  return { start: at, query };
}

export function insertToken(
  text: string,
  caret: number,
  at: AtQuery,
  relpath: string,
): { text: string; caret: number } {
  const token = `@${relpath} `;
  const next = text.slice(0, at.start) + token + text.slice(caret);
  return { text: next, caret: at.start + token.length };
}

export function toRelPath(path: string, cwd: string | null): string {
  if (!cwd) return path;
  const prefix = cwd.endsWith("/") ? cwd : `${cwd}/`;
  return path.startsWith(prefix) ? path.slice(prefix.length) : path;
}

export function shouldShowRichInput(
  kind: SessionKind,
  agentMatch: boolean,
  pref: RichInputPref,
): boolean {
  if (kind.type === "agent") return true;
  return agentMatch && pref.showOnMatch;
}
