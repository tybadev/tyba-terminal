export type KeyAction =
  | "palette"
  | "panel"
  | "settings"
  | "newSession"
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
  | "paneDown";

export const KEY_ACTIONS: KeyAction[] = [
  "palette",
  "panel",
  "settings",
  "newSession",
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
];

export type Bindings = Record<KeyAction, string>;

export const DEFAULT_BINDINGS: Bindings = {
  palette: "meta+k",
  panel: "meta+b",
  settings: "meta+,",
  newSession: "meta+n",
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
};

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

const SYMBOLS: Record<string, string> = {
  meta: "⌘",
  ctrl: "⌃",
  alt: "⌥",
  shift: "⇧",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
  enter: "↵",
  escape: "⎋",
  " ": "␣",
};

export function formatCombo(combo: string): string {
  return comboKeys(combo).join("");
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
