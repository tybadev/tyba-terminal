import { getCurrentWebview } from "@tauri-apps/api/webview";

import { getPref, onConfigPrefs, setPref } from "./lib/ipc";

/**
 * Escala da interface (Configurações › Aparência): o zoom do webview, que
 * aumenta tudo junto — cromo, telas e o terminal. Mora no `config.toml`
 * (`[appearance] ui_scale`), como as outras preferências de aparência.
 */
export type UiScale = 100 | 110 | 125;

export const UI_SCALES: UiScale[] = [100, 110, 125];
export const DEFAULT_UI_SCALE: UiScale = 100;

const STORAGE_KEY = "tyba.uiScale";
const PREF_KEY = "pref.ui_scale";

export function parseUiScale(value: string | null | undefined): UiScale {
  const n = Number(value);
  return UI_SCALES.find((s) => s === n) ?? DEFAULT_UI_SCALE;
}

const listeners = new Set<(scale: UiScale) => void>();
let scale: UiScale = DEFAULT_UI_SCALE;
let dirty = false;

/** O fator de agora (1, 1,1 ou 1,25): quem manda coordenada do DOM para uma
 *  view nativa multiplica por ele. */
export function uiScaleFactor(): number {
  return scale / 100;
}

export function getUiScale(): UiScale {
  return scale;
}

export function onUiScaleChange(cb: (scale: UiScale) => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

function commit(next: UiScale) {
  scale = next;
  try {
    localStorage.setItem(STORAGE_KEY, String(next));
  } catch {
    // Sem storage o valor ainda vale nesta janela.
  }
  void getCurrentWebview()
    .setZoom(next / 100)
    .catch(() => {});
  for (const cb of listeners) cb(next);
}

export function setUiScale(next: UiScale) {
  if (next === scale) return;
  dirty = true;
  commit(next);
  void setPref(PREF_KEY, String(next)).catch(() => {});
}

/** Chamado uma vez no arranque de cada janela (`main.tsx`). */
export function initUiScale() {
  let cached: string | null = null;
  try {
    cached = localStorage.getItem(STORAGE_KEY);
  } catch {
    cached = null;
  }
  // O cache evita o salto do 100% para a escala salva no primeiro quadro.
  const first = parseUiScale(cached);
  if (first !== DEFAULT_UI_SCALE) commit(first);
  void getPref(PREF_KEY)
    .then((remote) => {
      if (dirty) return;
      // O config.toml é a fonte da verdade: sem valor lá, vale o padrão.
      const next = parseUiScale(remote);
      if (next !== scale) commit(next);
    })
    .catch(() => {});
  // Edição na mão do ~/.tyba/config.toml: o core manda as prefs validadas.
  void onConfigPrefs((prefs) => {
    const next = parseUiScale(prefs[PREF_KEY]);
    if (next !== scale) commit(next);
  }).catch(() => {});
}
