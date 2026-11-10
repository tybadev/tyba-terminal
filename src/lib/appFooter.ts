/**
 * Rodapé do app e a gaveta Fila/Agentes que sobe dele (quadro
 * GavetaPaneEstreito: Pílulas, R1–R5, escada do Rodapé).
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

/** Gaveta: mínimo, máximo, folgas (quadro R3/R4). */
export const DRAWER_MIN = 360;
export const DRAWER_MAX = 430;
/** Borda direita da gaveta para dentro da borda direita do pane ativo. */
export const DRAWER_INSET = 8;
/** Distância mínima da borda esquerda da janela. */
export const DRAWER_EDGE = 8;
/** Base da gaveta acima do topo do rodapé. */
export const DRAWER_LIFT = 6;

export type DrawerBox = {
  /** Em px, da borda esquerda da janela. */
  left: number;
  width: number;
  /** Em px, da borda de baixo da janela. */
  bottom: number;
  /** Abaixo de 430 as abas viram ícone + contagem. */
  compact: boolean;
};

/**
 * Onde a gaveta aberta fica: popover que sobe do rodapé, borda direita 8 px
 * dentro do pane ativo (com o detalhe aberto, o pane termina antes dele, e a
 * gaveta não o cobre). Largura do pane menos as folgas, entre 360 e 430;
 * abaixo do pane ela cresce para a esquerda por cima da lateral, e se nem
 * assim couber encosta a 8 px da borda esquerda da janela.
 */
export function drawerBox(input: {
  paneLeft: number;
  paneRight: number;
  footerTop: number;
  viewportHeight: number;
}): DrawerBox {
  const paneWidth = input.paneRight - input.paneLeft;
  const width = Math.min(DRAWER_MAX, Math.max(DRAWER_MIN, paneWidth - 2 * DRAWER_INSET));
  const right = input.paneRight - DRAWER_INSET;
  const left = Math.max(DRAWER_EDGE, right - width);
  return {
    left,
    width,
    bottom: input.viewportHeight - input.footerTop + DRAWER_LIFT,
    compact: width < DRAWER_MAX,
  };
}
