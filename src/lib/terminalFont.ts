// Fonte do terminal (quadro SettingsSpecimen: "Fonte — A do terminal"). Vale
// para o xterm e para os blocos, que precisam da mesma métrica: muda a
// `--tyba-font-mono` do documento e anuncia para os terminais abertos.
// A preferência mora no ~/.tyba/config.toml (`[font] terminal`).

import { getPref, onConfigPrefs, setPref } from "./ipc";

export type TerminalFont = "jetbrains" | "system";

export const TERMINAL_FONTS: TerminalFont[] = ["jetbrains", "system"];

const PREF_KEY = "pref.terminal_font";
export const TERMINAL_FONT_EVENT = "tyba:terminal-font";

/** A pilha de cada escolha. A Nerd Font logo depois da primeira (ver styles.css). */
const FAMILY: Record<TerminalFont, string> = {
  jetbrains: '"JetBrains Mono", "Symbols Nerd Font Mono", ui-monospace, SFMono-Regular, Menlo, monospace',
  system: 'ui-monospace, SFMono-Regular, "SF Mono", "Symbols Nerd Font Mono", Menlo, Consolas, monospace',
};

export const parseTerminalFont = (raw: unknown): TerminalFont => (raw === "system" ? "system" : "jetbrains");

export const terminalFontFamily = (font: TerminalFont): string => FAMILY[font];

let font: TerminalFont = "jetbrains";

export const getTerminalFont = (): TerminalFont => font;

function commit(next: TerminalFont) {
  if (next === font) return;
  font = next;
  if (typeof document !== "undefined") {
    document.documentElement.style.setProperty("--tyba-font-mono", FAMILY[next]);
    window.dispatchEvent(new CustomEvent(TERMINAL_FONT_EVENT, { detail: FAMILY[next] }));
  }
}

export function setTerminalFont(next: TerminalFont) {
  commit(next);
  void setPref(PREF_KEY, next).catch(() => {});
}

/** Lê a preferência do core e acompanha a edição à mão do config.toml. */
export function initTerminalFont() {
  void getPref(PREF_KEY)
    .then((raw) => commit(parseTerminalFont(raw)))
    .catch(() => {});
  void onConfigPrefs((prefs) => commit(parseTerminalFont(prefs[PREF_KEY])));
}
