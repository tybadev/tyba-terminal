/**
 * "Abrir caminhos do terminal no TYBA": ⌘-clique num `src/x.ts:12` abre o
 * arquivo na tela de Arquivos. Ligado por padrão; a escolha mora no core
 * (`pref.links.files`) e fica num espelho aqui porque quem pergunta é o
 * provedor de links do xterm, a cada linha que o mouse cruza.
 */
import { getPref, setPref } from "./ipc";

export const FILE_LINKS_PREF_KEY = "pref.links.files";

let enabled = true;
void getPref(FILE_LINKS_PREF_KEY)
  .then((v) => {
    enabled = v !== "off";
  })
  .catch(() => {});

export const fileLinksEnabled = () => enabled;

export function setFileLinksEnabled(next: boolean) {
  enabled = next;
  void setPref(FILE_LINKS_PREF_KEY, next ? "on" : "off").catch(() => {});
}
