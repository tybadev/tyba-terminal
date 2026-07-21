import { IS_MAC } from "./platform";

export type KeyAction =
  | "paletteActions"
  | "paletteSessions"
  | "panel"
  | "files"
  | "settings"
  | "newSession"
  | "newWorktreeSession"
  | "newTab"
  | "newWindow"
  | "closePane"
  | "openFolder"
  | "prevSession"
  | "nextSession"
  | "prevTab"
  | "nextTab"
  | "splitRight"
  | "splitDown"
  | "nextPane"
  | "paneLeft"
  | "paneRight"
  | "paneUp"
  | "paneDown"
  | "copy"
  | "paste"
  | "search"
  | "selectAll"
  | "richInput";

export const KEY_ACTIONS: KeyAction[] = [
  "paletteActions",
  "paletteSessions",
  "panel",
  "files",
  "settings",
  "newSession",
  "newWorktreeSession",
  "newTab",
  "newWindow",
  "closePane",
  "openFolder",
  "prevSession",
  "nextSession",
  "prevTab",
  "nextTab",
  "splitRight",
  "splitDown",
  "nextPane",
  "paneLeft",
  "paneRight",
  "paneUp",
  "paneDown",
  "copy",
  "paste",
  "search",
  "selectAll",
  "richInput",
];

export type KeyCategory =
  | "general"
  | "sessions"
  | "tabs"
  | "panes"
  | "navigation"
  | "terminal";

export const KEY_CATEGORY_ORDER: KeyCategory[] = [
  "general",
  "sessions",
  "tabs",
  "panes",
  "navigation",
  "terminal",
];

export const KEY_ACTION_CATEGORY: Record<KeyAction, KeyCategory> = {
  paletteActions: "general",
  paletteSessions: "general",
  panel: "general",
  files: "general",
  settings: "general",
  newSession: "sessions",
  newWorktreeSession: "sessions",
  newWindow: "sessions",
  openFolder: "sessions",
  newTab: "tabs",
  closePane: "tabs",
  splitRight: "panes",
  splitDown: "panes",
  prevSession: "navigation",
  nextSession: "navigation",
  prevTab: "navigation",
  nextTab: "navigation",
  nextPane: "navigation",
  paneLeft: "navigation",
  paneRight: "navigation",
  paneUp: "navigation",
  paneDown: "navigation",
  copy: "terminal",
  paste: "terminal",
  search: "terminal",
  selectAll: "terminal",
  richInput: "general",
};

export const ACTION_LABEL_KEYS: Record<KeyAction, string> = {
  paletteActions: "paletteActions",
  paletteSessions: "paletteSessions",
  panel: "togglePanel",
  files: "filesPanel",
  settings: "settings",
  newSession: "newSession",
  newWorktreeSession: "worktreeNewSession",
  newTab: "newTab",
  newWindow: "newWindow",
  closePane: "closePane",
  openFolder: "openProjectFolder",
  prevSession: "prevSessionNav",
  nextSession: "nextSessionNav",
  prevTab: "prevTabNav",
  nextTab: "nextTabNav",
  splitRight: "splitRight",
  splitDown: "splitDown",
  nextPane: "nextPane",
  paneLeft: "paneLeft",
  paneRight: "paneRight",
  paneUp: "paneUp",
  paneDown: "paneDown",
  copy: "copySelection",
  paste: "pasteClipboard",
  search: "searchTerminal",
  selectAll: "selectAllTerminal",
  richInput: "richInputToggle",
};

export const KEY_CATEGORY_LABEL_KEYS: Record<KeyCategory, string> = {
  general: "catGeneral",
  sessions: "catSessions",
  tabs: "catTabs",
  panes: "catPanes",
  navigation: "catNavigation",
  terminal: "catTerminal",
};

export function isTerminalAction(action: KeyAction): boolean {
  return KEY_ACTION_CATEGORY[action] === "terminal";
}

export function actionsByCategory(): [KeyCategory, KeyAction[]][] {
  return KEY_CATEGORY_ORDER.map((cat) => [
    cat,
    KEY_ACTIONS.filter((a) => KEY_ACTION_CATEGORY[a] === cat),
  ]);
}

export type Bindings = Record<KeyAction, string>;

export const MAC_BINDINGS: Bindings = {
  paletteActions: "meta+p",
  paletteSessions: "meta+shift+p",
  panel: "meta+b",
  files: "meta+shift+e",
  settings: "meta+,",
  newSession: "meta+n",
  newWorktreeSession: "meta+shift+t",
  newTab: "meta+t",
  newWindow: "meta+shift+n",
  closePane: "meta+w",
  openFolder: "meta+o",
  prevSession: "meta+shift+arrowup",
  nextSession: "meta+shift+arrowdown",
  prevTab: "meta+shift+arrowleft",
  nextTab: "meta+shift+arrowright",
  splitRight: "meta+d",
  splitDown: "meta+shift+d",
  nextPane: "meta+]",
  paneLeft: "meta+alt+arrowleft",
  paneRight: "meta+alt+arrowright",
  paneUp: "meta+alt+arrowup",
  paneDown: "meta+alt+arrowdown",
  copy: "meta+c",
  paste: "meta+v",
  search: "meta+f",
  selectAll: "meta+a",
  richInput: "meta+shift+g",
};

export const PC_BINDINGS: Bindings = {
  paletteActions: "ctrl+shift+p",
  paletteSessions: "ctrl+alt+shift+p",
  panel: "ctrl+shift+b",
  files: "ctrl+shift+e",
  settings: "ctrl+,",
  newSession: "ctrl+shift+n",
  newWorktreeSession: "ctrl+alt+shift+t",
  newTab: "ctrl+shift+t",
  newWindow: "ctrl+alt+shift+n",
  closePane: "ctrl+shift+w",
  openFolder: "ctrl+shift+o",
  prevSession: "ctrl+shift+arrowup",
  nextSession: "ctrl+shift+arrowdown",
  prevTab: "ctrl+pageup",
  nextTab: "ctrl+pagedown",
  splitRight: "ctrl+alt+d",
  splitDown: "ctrl+shift+d",
  nextPane: "ctrl+tab",
  paneLeft: "ctrl+alt+arrowleft",
  paneRight: "ctrl+alt+arrowright",
  paneUp: "ctrl+alt+arrowup",
  paneDown: "ctrl+alt+arrowdown",
  copy: "ctrl+shift+c",
  paste: "ctrl+shift+v",
  search: "ctrl+shift+f",
  selectAll: "ctrl+shift+a",
  richInput: "ctrl+shift+g",
};

export const DEFAULT_BINDINGS: Bindings = IS_MAC ? MAC_BINDINGS : PC_BINDINGS;

export const TAB_DIGIT_MODIFIER = IS_MAC ? "meta" : "alt";

export function isTabDigitChord(e: KeyboardEvent): boolean {
  const held = IS_MAC ? e.metaKey && !e.altKey : e.altKey && !e.metaKey;
  return held && !e.ctrlKey && !e.shiftKey;
}

export function isPaneResizeChord(e: KeyboardEvent): boolean {
  return IS_MAC
    ? e.metaKey && e.ctrlKey && !e.shiftKey && !e.altKey
    : e.ctrlKey && e.altKey && e.shiftKey && !e.metaKey;
}

export const PANE_RESIZE_COMBO_PREFIX = IS_MAC ? "meta+ctrl" : "ctrl+alt+shift";

export const OPEN_LATEST_SESSION_COMBO = IS_MAC
  ? "meta+shift+o"
  : "ctrl+alt+shift+o";

export const SEND_PROMPT_COMBO = IS_MAC ? "meta+enter" : "ctrl+enter";

export function tabDigitCombo(index: number): string {
  return `${TAB_DIGIT_MODIFIER}+${index}`;
}

export const BINDINGS_PREF_KEY = "pref.keybindings";

export const captureState = { active: false };

export function comboOf(e: KeyboardEvent): string | null {
  const key = e.key.toLowerCase();
  if (["meta", "control", "alt", "shift"].includes(key)) return null;
  const parts: string[] = [];
  if (e.metaKey) parts.push("meta");
  if (e.ctrlKey) parts.push("ctrl");
  if (e.altKey) parts.push("alt");
  if (e.shiftKey) parts.push("shift");
  if (parts.length === 0) return null;
  parts.push(key);
  return parts.join("+");
}

const SHARED_SYMBOLS: Record<string, string> = {
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
  enter: "↵",
  escape: "⎋",
  " ": "␣",
};

const MAC_SYMBOLS: Record<string, string> = {
  ...SHARED_SYMBOLS,
  meta: "⌘",
  ctrl: "⌃",
  alt: "⌥",
  shift: "⇧",
  tab: "⇥",
  pageup: "⇞",
  pagedown: "⇟",
};

const PC_SYMBOLS: Record<string, string> = {
  ...SHARED_SYMBOLS,
  meta: "Super",
  ctrl: "Ctrl",
  alt: "Alt",
  shift: "Shift",
  tab: "Tab",
  pageup: "PgUp",
  pagedown: "PgDn",
};

const SYMBOLS = IS_MAC ? MAC_SYMBOLS : PC_SYMBOLS;

const COMBO_SEPARATOR = IS_MAC ? "" : "+";

export function formatCombo(combo: string): string {
  return comboKeys(combo).join(COMBO_SEPARATOR);
}

export function comboKeys(combo: string): string[] {
  return combo
    .split("+")
    .map((part) => SYMBOLS[part] ?? part.toUpperCase());
}

export function parseBindings(raw: string | null): Bindings {
  if (!raw) return { ...DEFAULT_BINDINGS };
  try {
    const parsed = JSON.parse(raw) as Partial<Bindings>;
    const merged = { ...DEFAULT_BINDINGS };
    for (const action of KEY_ACTIONS) {
      const combo = parsed[action];
      if (typeof combo === "string" && combo.includes("+")) {
        merged[action] = combo;
      }
    }
    return merged;
  } catch {
    return { ...DEFAULT_BINDINGS };
  }
}
