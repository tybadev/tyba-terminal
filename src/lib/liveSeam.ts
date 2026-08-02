/**
 * A emenda entre a saída ao vivo e o cartão do bloco.
 *
 * Enquanto o comando roda a saída vive no xterm; ao terminar ela vira cartão
 * DOM. Se as duas ocupam alturas diferentes, o conteúdo antigo salta no
 * instante da troca — e antes disso sobra um vazio embaixo da saída, porque a
 * faixa tem altura fixa e a saída não.
 *
 * A conta aqui recorta a faixa na altura da saída real. O xterm continua com o
 * MESMO tamanho: quem se move é a imagem dele, por `transform` e `clip-path`,
 * que não entram no layout e portanto não acordam o `ResizeObserver` que
 * redimensionaria o PTY.
 */

export interface SeamRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Fração do painel que o terminal ocupa em modo prompt.
 *
 * É CONSTANTE: o xterm é dimensionado por ela do início ao fim da sessão. O que
 * varia é quanto dele fica à mostra, e isso é recorte, não tamanho.
 */
export const LIVE_FRACTION = 0.5;

/**
 * Quanto um comando precisa durar para a faixa ao vivo abrir.
 *
 * `clear`, `cd`, `echo`: abrir e fechar a faixa em 30ms é um pisca que chama
 * mais atenção do que a coisa que ele ia mostrar.
 */
export const LIVE_DELAY_MS = 120;

/**
 * Piso da faixa revelada, em fração da altura dela.
 *
 * Comando que ainda não escreveu nada tem `usedRows = 0`, e faixa de altura
 * zero faz o cursor piscando sumir — o comando parece travado no escuro. Uma
 * réstia basta para o cursor aparecer.
 */
const MIN_USED = 0.08;

/**
 * Quanto da faixa a saída de fato ocupa, entre {@link MIN_USED} e 1.
 *
 * `scrolled` é o sinal de que a saída passou da altura da tela: aí ela encheu a
 * faixa inteira e não há o que recortar. Sem esse ramo, uma saída longa
 * reportaria o cursor parado na última linha da viewport e a conta daria certo
 * por acidente — até o dia em que o cursor não estivesse lá.
 */
export function usedFraction(
  usedRows: number,
  totalRows: number,
  scrolled = false,
): number {
  if (scrolled) return 1;
  if (!Number.isFinite(usedRows) || !Number.isFinite(totalRows)) return 1;
  if (totalRows <= 0) return 1;
  const raw = usedRows / totalRows;
  if (!Number.isFinite(raw)) return 1;
  return Math.min(1, Math.max(MIN_USED, raw));
}

/**
 * O quanto a imagem do terminal desce, em fração da altura DELE.
 *
 * Serve aos dois lados da mesma conta: `translateY` desloca a faixa para que seu
 * topo encoste no fim da lista, e `clip-path` corta embaixo a parte que sobrou.
 * Por serem a mesma fração, a parte visível cai exatamente onde a lista termina.
 */
export function hiddenFraction(used: number): number {
  return Math.min(1, Math.max(0, 1 - used));
}

/** Altura da faixa revelada, em % do painel — sem contar o padding do terminal. */
export function liveHeight(pane: SeamRect, used: number): number {
  return pane.height * LIVE_FRACTION * used;
}

/**
 * O pedaço da faixa que a conta em % não enxerga, em px.
 *
 * As linhas não ocupam a caixa inteira do terminal: sobra o padding vertical
 * dele. Recortar por fração da altura TOTAL corta alguns pixels acima do fim do
 * texto — a última linha sai partida ao meio.
 *
 * Some quando a faixa está cheia (`used = 1`) e é máximo quando ela é uma
 * réstia, que é justamente onde o erro relativo dói mais.
 *
 * Só aparecia com prompt que pergunta sem quebrar linha (`Ok to proceed? (y)`).
 * Com `seq` ou `for` o cursor para numa linha vazia depois da saída, e essa
 * folga de uma linha escondia o déficit.
 */
export function padSlackPx(padY: number, used: number): number {
  if (!Number.isFinite(padY) || padY <= 0) return 0;
  return padY * hiddenFraction(used);
}

/**
 * A caixa do terminal — sempre a mesma, viva ou não.
 *
 * Não recebe `used` de propósito: é o tamanho que o `fit` mede e que vira
 * `resizeSession`. Fazê-lo depender do comando é o bug que reabria o `vim` com
 * outra altura.
 */
export function termRect(pane: SeamRect): SeamRect {
  const height = pane.height * LIVE_FRACTION;
  return {
    left: pane.left,
    top: pane.top + pane.height - height,
    width: pane.width,
    height,
  };
}

/** Onde a saída aparece depois do recorte: colada no fim do painel. */
export function liveRect(pane: SeamRect, used: number): SeamRect {
  const height = liveHeight(pane, used);
  return {
    left: pane.left,
    top: pane.top + pane.height - height,
    width: pane.width,
    height,
  };
}

/**
 * O que sobra para a lista de blocos.
 *
 * Ociosa, a lista cobre o painel inteiro — é ela que esconde o terminal. Com
 * comando rodando, ela cede só a altura que a saída realmente usa, e não mais
 * metade do painel para uma faixa que estaria 35% vazia.
 */
export function blocksRect(pane: SeamRect, live: boolean, used = 1): SeamRect {
  if (!live) return pane;
  return {
    left: pane.left,
    top: pane.top,
    width: pane.width,
    height: pane.height - liveHeight(pane, used),
  };
}
