import { useInsertionEffect, useMemo, useRef } from "react";

/**
 * Identidade estável para prop de componente `memo`.
 *
 * `memo` compara props com `Object.is` campo a campo: uma única prop com
 * identidade nova derruba a comparação inteira e o componente re-renderiza como
 * se não fosse `memo` — custo puro, sem nada na tela que denuncie.
 *
 * Este arquivo é a terceira peça contra a MESMA classe de defeito, e ela já
 * apareceu três vezes no `App`:
 *
 * - objeto montado no JSX (`rect`, `opened`) — resolvido por `sessionBlocksData`,
 *   que recorta o estado em primitivos;
 * - arrow amarrando a sessão dentro de um `map` (`(id, e) => pick(s.id, id, e)`)
 *   — resolvido por `handlerCache`, que guarda uma função por sessão;
 * - `useCallback` cuja lista de dependências muda — o que sobra, e o que este
 *   arquivo cobre. Um handler que lê estado que muda a toda hora (qual painel
 *   está ativo, se a linha é do TYBA) ganha identidade nova em cada uma dessas
 *   mudanças, e leva junto todo `useCallback` que o tenha na própria lista.
 */

/**
 * Encaminha a chamada para a versão mais recente guardada no ref.
 *
 * Separada do hook porque é ela que carrega o risco da troca, e risco quer
 * teste: identidade fixa vale zero se a função chamada for a de um render
 * antigo — em vez de custar quadro, o defeito passaria a agir sobre a sessão
 * errada. Ver `stableProps.test.ts`.
 */
export function callLatest<A extends unknown[], R>(ref: {
  current: (...args: A) => R;
}): (...args: A) => R {
  return (...args: A) => ref.current(...args);
}

/**
 * Handler com identidade FIXA para sempre, que chama a closure do último
 * render.
 *
 * Substitui `useCallback` quando a função é prop de um componente `memo`: não
 * há lista de dependências para acertar, então não há como esquecer uma nem
 * como incluir uma que muda demais. Em troca, a função só serve para ser
 * CHAMADA — nunca como dependência de efeito, porque ela nunca sinaliza que o
 * que ela lê mudou.
 *
 * > [!warning] O ref é escrito em `useInsertionEffect`, não no corpo do render:
 * > um render que o React descarta (Suspense, transição interrompida) deixaria
 * > guardada uma closure que nunca chegou à tela. Insertion effect roda antes
 * > de todo layout effect, então nenhum handler dispara com a versão anterior.
 */
export function useStableCallback<A extends unknown[], R>(
  fn: (...args: A) => R,
): (...args: A) => R {
  const ref = useRef(fn);
  useInsertionEffect(() => {
    ref.current = fn;
  });
  return useMemo(() => callLatest(ref), []);
}
