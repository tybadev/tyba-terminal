/**
 * Estado medido por sessão.
 *
 * Painel é o dono da medida, não a janela. A altura de linha do terminal e a
 * altura do header do bloco em execução saem de um `ResizeObserver` sobre
 * elementos de LARGURAS diferentes quando a tela está dividida — guardá-las num
 * valor único faz o último painel a medir sobrescrever o outro, e a lista do
 * vizinho passa a encurtar pela altura errada.
 */

/**
 * O `Record` com o valor de uma sessão trocado ou criado.
 *
 * Devolve o **mesmo objeto** quando nada muda. Isso não é micro-otimização: a
 * medida chega de um `ResizeObserver`, e devolver um objeto novo a cada
 * relatório re-renderiza, o que remede, o que relata de novo. Comparar por
 * valor corta o ciclo no primeiro passo.
 *
 * Não há caminho de remoção porque não há o que ele consertaria: `SessionId` é
 * UUID e nunca se repete, então a medida de uma sessão fechada não é herdada
 * por ninguém. O que sobra é um número por sessão morta na memória da janela —
 * custo que não paga uma varredura de limpeza a cada mudança de lista.
 */
export function withEntry<T>(
  prev: Record<string, T>,
  id: string,
  value: T,
): Record<string, T> {
  if (prev[id] === value) return prev;
  return { ...prev, [id]: value };
}
