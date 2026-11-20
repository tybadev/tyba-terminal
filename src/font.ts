import { getPref, onConfigPrefs, setPref } from "./lib/ipc";
import { getEffectiveBase, onEffectiveBaseChange } from "./theme";

export type UiFont = "mono" | "grotesk";

export const UI_FONTS: UiFont[] = ["mono", "grotesk"];

export const UI_FONT_LABELS: Record<UiFont, string> = {
  mono: "JetBrains Mono",
  grotesk: "Sans",
};

const STORAGE_KEY = "tyba.font";
const PREF_KEY = "pref.ui_font";

const fontListeners = new Set<(font: UiFont) => void>();

const stored = localStorage.getItem(STORAGE_KEY);
let font: UiFont = stored === "grotesk" ? "grotesk" : "mono";
// Sem escolha salva, a interface segue o tema: a sans do Muriki no claro (o
// quadro aprovado) e a mono no escuro de sempre. Escolher "mono" ou "sans"
// vale nos dois temas (dono, 2026-11-02).
let explicit = stored === "mono" || stored === "grotesk";
let dirty = false;

function apply() {
  const root = document.documentElement;
  if (!explicit) {
    root.setAttribute("data-font", "auto");
  } else if (font === "grotesk") {
    root.setAttribute("data-font", "grotesk");
  } else {
    root.removeAttribute("data-font");
  }
}

function notify() {
  const effective = getUiFont();
  for (const cb of fontListeners) cb(effective);
}

function commit(next: UiFont) {
  font = next;
  explicit = true;
  localStorage.setItem(STORAGE_KEY, next);
  apply();
  notify();
}

function reset() {
  font = "mono";
  explicit = false;
  localStorage.removeItem(STORAGE_KEY);
  apply();
  notify();
}

export function getUiFont(): UiFont {
  if (!explicit) return getEffectiveBase() === "light" ? "grotesk" : "mono";
  return font;
}

export function onUiFontChange(cb: (font: UiFont) => void): () => void {
  fontListeners.add(cb);
  return () => fontListeners.delete(cb);
}

export function setUiFont(next: UiFont) {
  if (explicit && next === font) return;
  dirty = true;
  commit(next);
  void setPref(PREF_KEY, next).catch(() => {});
}

async function init() {
  try {
    const remote = await getPref(PREF_KEY);
    if (dirty) return;
    if (remote === "mono" || remote === "grotesk") {
      if (!explicit || remote !== font) commit(remote);
    } else if (explicit) {
      // O config.toml é a fonte da verdade: sem valor lá, vale o padrão — o
      // cache local não reescreve o arquivo.
      reset();
    }
  } catch {
    apply();
  }
}

apply();
void init();
onEffectiveBaseChange(() => {
  if (!explicit) notify();
});
// Edição na mão do ~/.tyba/config.toml: o core manda as prefs já validadas.
void onConfigPrefs((prefs) => {
  const raw = prefs[PREF_KEY];
  if (raw === "mono" || raw === "grotesk") {
    if (!explicit || raw !== font) commit(raw);
  } else if (explicit) {
    reset();
  }
}).catch(() => {});
