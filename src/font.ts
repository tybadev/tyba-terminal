import { getPref, setPref } from "./lib/ipc";

export type UiFont = "mono" | "grotesk";

export const UI_FONTS: UiFont[] = ["mono", "grotesk"];

const STORAGE_KEY = "tyba.font";
const PREF_KEY = "pref.ui_font";

const fontListeners = new Set<(font: UiFont) => void>();

let font: UiFont = localStorage.getItem(STORAGE_KEY) === "grotesk" ? "grotesk" : "mono";

function apply() {
  const root = document.documentElement;
  if (font === "grotesk") {
    root.setAttribute("data-font", "grotesk");
  } else {
    root.removeAttribute("data-font");
  }
}

export function getUiFont(): UiFont {
  return font;
}

export function onUiFontChange(cb: (font: UiFont) => void): () => void {
  fontListeners.add(cb);
  return () => fontListeners.delete(cb);
}

export function setUiFont(next: UiFont) {
  if (next === font) return;
  font = next;
  localStorage.setItem(STORAGE_KEY, next);
  apply();
  for (const cb of fontListeners) cb(next);
  void setPref(PREF_KEY, next).catch(() => {});
}

async function init() {
  try {
    const remote = await getPref(PREF_KEY);
    if (remote === "mono" || remote === "grotesk") {
      if (remote !== font) {
        font = remote;
        localStorage.setItem(STORAGE_KEY, remote);
        apply();
        for (const cb of fontListeners) cb(remote);
      }
    } else if (font !== "mono") {
      void setPref(PREF_KEY, font).catch(() => {});
    }
  } catch {
    apply();
  }
}

apply();
void init();
