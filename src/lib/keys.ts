export type KeyAction =
  | "palette"
  | "panel"
  | "newTab"
  | "closePane"
  | "openFolder";

export const KEY_ACTIONS: KeyAction[] = [
  "palette",
  "panel",
  "newTab",
  "closePane",
  "openFolder",
];

export type Bindings = Record<KeyAction, string>;

export const DEFAULT_BINDINGS: Bindings = {
  palette: "meta+k",
  panel: "meta+b",
  newTab: "meta+t",
  closePane: "meta+w",
  openFolder: "meta+o",
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
};

export function formatCombo(combo: string): string {
  return combo
    .split("+")
    .map((part) => SYMBOLS[part] ?? part.toUpperCase())
    .join("");
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
