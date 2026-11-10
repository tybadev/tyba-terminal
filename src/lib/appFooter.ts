/**
 * Rodapé do app (quadro GavetaPaneEstreito, escada do Rodapé).
 */

/**
 * Escada do rodapé estreito: o degrau em que cada peça sai. Mede e tira um
 * degrau por vez até caber; o estado da pílula nunca sai.
 *
 *  1 rótulo "atalhos" (fica o ícone de teclado)
 *  2 rótulo "snippets" (fica a tecla)
 *  3 rótulo "histórico"
 *  4 rótulo "comandos"
 *  5 rótulo "Rich Input" (ficam o ícone e a tecla)
 *  6 pílula compacta
 *  7 hora
 *  8 nome da sessão (fica o ícone, nome no tooltip)
 */
export const FOOTER_STEP = {
  shortcutsLabel: 1,
  snippetsLabel: 2,
  historyLabel: 3,
  commandsLabel: 4,
  richInputLabel: 5,
  pillCompact: 6,
  clock: 7,
  cwdLabel: 8,
} as const;
export const FOOTER_TOP_STEP = 8;

/** A dica do rodapé (pela `key` que o App dá) e o degrau em que o rótulo dela sai. */
export const HINT_LABEL_STEP: Record<string, number> = {
  shortcuts: FOOTER_STEP.shortcutsLabel,
  snippets: FOOTER_STEP.snippetsLabel,
  history: FOOTER_STEP.historyLabel,
  actions: FOOTER_STEP.commandsLabel,
};

/** A peça que sai no degrau `at` ainda aparece no degrau `step`? */
export function footerKeeps(step: number, at: number): boolean {
  return step < at;
}
