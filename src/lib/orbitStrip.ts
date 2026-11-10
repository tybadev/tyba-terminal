/**
 * Degraus da faixa do Orbit (quadro FaixaOrbit): os rótulos saem antes de
 * as placas se sobreporem, sem trocar botão de lugar.
 *
 *  0 inteira
 *  1 Execuções, Variáveis, Integrações, Papéis e Ajustar só ícone
 *  2 Nó terminal só + e o nome do grafo trunca
 *  3 os quatro painéis entram no menu ⋯
 *  4 ❯ Terminal, Executar e Parar só ícone (abaixo disso a faixa quebra)
 */
export const STRIP_TOP_LEVEL = 4;
/** Vão mínimo entre a placa da esquerda e a da direita. */
export const STRIP_GAP = 12;
/** Folga para voltar um degrau, para não piscar na borda. */
export const STRIP_HYSTERESIS = 24;

/**
 * Escolhe o menor degrau em que as duas placas e o vão cabem.
 *
 * `needs[l]` é a largura que o degrau `l` pede (placas + vão), ou `undefined`
 * se ainda não foi medido — aí devolve esse degrau para medi-lo antes de
 * decidir. Voltar para um degrau menor que o `current` exige a folga.
 * `top` é o último degrau (o rodapé do app tem 8).
 */
export function pickStripLevel(
  needs: readonly (number | undefined)[],
  available: number,
  current: number,
  top: number = STRIP_TOP_LEVEL,
): number {
  for (let l = 0; l < top; l++) {
    const need = needs[l];
    if (need === undefined) return l;
    const slack = l < current ? STRIP_HYSTERESIS : 0;
    if (need + slack <= available) return l;
  }
  return top;
}
