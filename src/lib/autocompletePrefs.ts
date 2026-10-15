/**
 * As duas escolhas do grupo Autocompletar: a lista abrir sozinha depois do
 * espaço num comando conhecido, e o "42×" de quantas vezes você usou. Ligadas
 * por padrão; moram no core e ficam num espelho aqui porque quem pergunta é a
 * linha de comando, a cada tecla.
 */
import { getPref, setPref } from "./ipc";

const AUTO_OPEN_KEY = "pref.autocomplete.auto_open";
const SHOW_USES_KEY = "pref.autocomplete.show_uses";

const state = { autoOpen: true, showUses: true };
void getPref(AUTO_OPEN_KEY)
  .then((v) => {
    state.autoOpen = v !== "off";
  })
  .catch(() => {});
void getPref(SHOW_USES_KEY)
  .then((v) => {
    state.showUses = v !== "off";
  })
  .catch(() => {});

export const autocompleteAutoOpen = () => state.autoOpen;
export const autocompleteShowUses = () => state.showUses;

export function setAutocompleteAutoOpen(next: boolean) {
  state.autoOpen = next;
  void setPref(AUTO_OPEN_KEY, next ? "on" : "off").catch(() => {});
}

export function setAutocompleteShowUses(next: boolean) {
  state.showUses = next;
  void setPref(SHOW_USES_KEY, next ? "on" : "off").catch(() => {});
}
