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

/**
 * Handler amarrado a uma sessão, com identidade estável entre renders.
 *
 * O problema que ele resolve: a lista de painéis é um `map`, e amarrar a sessão
 * ali dentro (`onPick={(id, e) => pickBlock(s.id, id, e)}`) cria uma função
 * nova a cada render — mesmo que `pickBlock` seja um `useCallback` estável. A
 * prop com identidade nova desce até o cartão e derruba o `memo` dele, que é
 * justamente o que impede milhares de nós de DOM de serem recriados a cada
 * quadro enquanto um comando escreve.
 *
 * Guarda uma função por sessão e devolve sempre a MESMA. `run` precisa ter
 * identidade estável (um `useCallback` com dependências vazias): o cache
 * captura a primeira, e uma `run` que muda depois não seria vista.
 *
 * Não há remoção pelo mesmo motivo de `withEntry`: `SessionId` é UUID e não se
 * repete, então nenhuma sessão nova herda o handler de uma morta. O que sobra é
 * uma closure por sessão fechada na memória da janela, e varrer isso a cada
 * mudança de lista custaria mais do que deixar.
 */
export function handlerCache<A extends unknown[]>(
  run: (id: string, ...args: A) => void,
): (id: string) => (...args: A) => void {
  const cache = new Map<string, (...args: A) => void>();
  return (id) => {
    const hit = cache.get(id);
    if (hit) return hit;
    const bound = (...args: A) => run(id, ...args);
    cache.set(id, bound);
    return bound;
  };
}
