import type { ITheme } from "@xterm/xterm";

import {
  getThemeState,
  onThemeChanged,
  setThemeModeCmd,
  setThemeSlot,
  type TerminalPalette,
  type Theme,
  type ThemeMode,
  type ThemeState,
} from "./lib/ipc";

export type { Theme, ThemeMode, ThemeState };

const STORAGE_KEY = "tyba.theme";
const MIGRATED_KEY = "tyba.theme.migrated";

export const THEMES: ThemeMode[] = ["dark", "light", "system"];

const media = window.matchMedia("(prefers-color-scheme: light)");

const FALLBACK_DARK: TerminalPalette = {
  background: "#070709",
  foreground: "#f2f2f5",
  cursor: "#7cc544",
  cursorAccent: "#070709",
  selectionBackground: "#7cc5444d",
  ansi: [
    "#1a1a20", "#f0503c", "#7cc544", "#f5a93b",
    "#4c7df0", "#ec4899", "#2dd4bf", "#f2f2f5",
    "#6a6a76", "#f4715f", "#a8e05f", "#f5c93b",
    "#7da3f5", "#a78bfa", "#5eead4", "#ffffff",
  ],
};

const FALLBACK_LIGHT: TerminalPalette = {
  background: "#fbfbfd",
  foreground: "#17171c",
  cursor: "#4e9622",
  cursorAccent: "#fbfbfd",
  selectionBackground: "#4e96224d",
  ansi: [
    "#2a2a31", "#d6402e", "#4e9622", "#c47c14",
    "#3b66d6", "#d33d84", "#0f9d8c", "#e9e9ee",
    "#6d6d78", "#e05a48", "#6fae2e", "#d69a2b",
    "#5d84e0", "#7a4ce8", "#16b3a0", "#ffffff",
  ],
};

let mode: ThemeMode = (() => {
  const saved = localStorage.getItem(STORAGE_KEY);
  return saved === "light" || saved === "system" ? saved : "dark";
})();

let state: ThemeState | null = null;

const terminalListeners = new Set<(theme: ITheme) => void>();
const modeListeners = new Set<(mode: ThemeMode) => void>();
let appliedVars: string[] = [];

function effectiveBase(): "dark" | "light" {
  return mode === "light" || (mode === "system" && media.matches)
    ? "light"
    : "dark";
}

function activeTheme(): Theme | null {
  if (!state) return null;
  return effectiveBase() === "light" ? state.light : state.dark;
}

function hexToRgba(hex: string, alpha: number): string | null {
  const raw = hex.replace("#", "");
  const size = raw.length === 3 ? 1 : 2;
  if (raw.length !== 3 && raw.length !== 6 && raw.length !== 8) return null;
  const channel = (i: number) => {
    const part = raw.slice(i * size, (i + 1) * size);
    return parseInt(size === 1 ? part + part : part, 16);
  };
  const [r, g, b] = [channel(0), channel(1), channel(2)];
  if ([r, g, b].some(Number.isNaN)) return null;
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

const kebab = (key: string) => key.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);

const GLOW_KEYS = ["green", "amber", "violet", "cyan", "red"];
const TINT_KEYS = ["green", "amber", "magenta", "violet", "blue", "cyan", "red"];

function computeVars(theme: Theme): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const [key, color] of Object.entries(theme.ui)) {
    vars[`--tyba-${kebab(key)}`] = color;
  }
  const dark = theme.base === "dark";
  for (const key of TINT_KEYS) {
    const color = theme.ui[key];
    const tint = color && hexToRgba(color, dark ? 0.14 : 0.12);
    if (tint) vars[`--tyba-${key}-tint`] = tint;
  }
  for (const key of GLOW_KEYS) {
    const color = theme.ui[key];
    const glow = color && hexToRgba(color, dark ? 0.45 : 0.28);
    if (glow) vars[`--tyba-glow-${key}`] = dark ? `0 0 12px ${glow}` : `0 0 10px ${glow}`;
  }
  const faint = theme.ui.textFaint;
  const neutral = faint && hexToRgba(faint, 0.14);
  if (neutral) vars["--tyba-neutral-tint"] = neutral;
  return vars;
}

function toXtermTheme(palette: TerminalPalette): ITheme {
  const [
    black, red, green, yellow, blue, magenta, cyan, white,
    brightBlack, brightRed, brightGreen, brightYellow,
    brightBlue, brightMagenta, brightCyan, brightWhite,
  ] = palette.ansi;
  return {
    background: palette.background,
    foreground: palette.foreground,
    cursor: palette.cursor,
    cursorAccent: palette.cursorAccent,
    selectionBackground: palette.selectionBackground,
    black, red, green, yellow, blue, magenta, cyan, white,
    brightBlack, brightRed, brightGreen, brightYellow,
    brightBlue, brightMagenta, brightCyan, brightWhite,
  };
}

function terminalPalette(): TerminalPalette {
  const theme = activeTheme();
  if (theme) return theme.terminal;
  return effectiveBase() === "light" ? FALLBACK_LIGHT : FALLBACK_DARK;
}

export function getTerminalTheme(): ITheme {
  return toXtermTheme(terminalPalette());
}

export function onTerminalThemeChange(cb: (theme: ITheme) => void): () => void {
  terminalListeners.add(cb);
  return () => terminalListeners.delete(cb);
}

export function onThemeModeChange(cb: (mode: ThemeMode) => void): () => void {
  modeListeners.add(cb);
  return () => modeListeners.delete(cb);
}

function apply() {
  const root = document.documentElement;
  if (effectiveBase() === "light") {
    root.setAttribute("data-theme", "light");
  } else {
    root.removeAttribute("data-theme");
  }

  for (const name of appliedVars) root.style.removeProperty(name);
  appliedVars = [];

  const theme = activeTheme();
  if (theme && !theme.builtin) {
    const vars = computeVars(theme);
    for (const [name, value] of Object.entries(vars)) {
      root.style.setProperty(name, value);
    }
    appliedVars = Object.keys(vars);
  }

  const xtermTheme = getTerminalTheme();
  for (const cb of terminalListeners) cb(xtermTheme);
}

media.addEventListener("change", () => {
  if (mode === "system") apply();
});

export function getThemeMode(): ThemeMode {
  return mode;
}

export function setThemeMode(next: ThemeMode) {
  mode = next;
  localStorage.setItem(STORAGE_KEY, next);
  apply();
  for (const cb of modeListeners) cb(next);
  void setThemeModeCmd(next).catch(() => {});
}

export async function applyTheme(theme: Theme): Promise<void> {
  await setThemeSlot(theme.base, theme.id);
  if (effectiveBase() !== theme.base) {
    setThemeMode(theme.base);
  }
}

function absorb(next: ThemeState) {
  state = next;
  if (next.mode !== mode) {
    mode = next.mode;
    localStorage.setItem(STORAGE_KEY, next.mode);
    for (const cb of modeListeners) cb(next.mode);
  }
  apply();
}

async function init() {
  try {
    await onThemeChanged(absorb);
    const remote = await getThemeState();
    const saved = localStorage.getItem(STORAGE_KEY) as ThemeMode | null;
    if (
      !localStorage.getItem(MIGRATED_KEY) &&
      saved &&
      THEMES.includes(saved) &&
      saved !== remote.mode
    ) {
      await setThemeModeCmd(saved);
      localStorage.setItem(MIGRATED_KEY, "1");
      absorb({ ...remote, mode: saved });
      return;
    }
    localStorage.setItem(MIGRATED_KEY, "1");
    absorb(remote);
  } catch {
    apply();
  }
}

apply();
void init();
