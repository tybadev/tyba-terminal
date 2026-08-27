/**
 * O estado de rolagem da faixa de abas, como número puro.
 *
 * Fica fora do componente porque é a parte que tem resposta certa e errada: o
 * filete na posição errada mente sobre onde você está, e isso não se vê num
 * screenshot — se vê contando pixel. O que sobra no `TabBar` é montar o JSX.
 */

/** O que a faixa precisa saber sobre si mesma para se desenhar. */
export interface Faixa {
  /** Quanto já foi rolado para a esquerda. */
  scrollLeft: number;
  /** A largura visível. */
  clientWidth: number;
  /** A largura total do conteúdo, visível ou não. */
  scrollWidth: number;
}

/**
 * Folga em pixels antes de considerar que há conteúdo escondido.
 *
 * Sem ela, arredondamento de layout (uma faixa de 800,4px com 800px de
 * conteúdo) acende o fade para meio pixel que ninguém vê — e o fade fica
 * piscando enquanto a janela é redimensionada.
 */
const FOLGA = 2;

/** Há aba escondida antes do começo da vista? */
export function temAntes(faixa: Faixa): boolean {
  return faixa.scrollLeft > FOLGA;
}

/** Há aba escondida depois do fim da vista? */
export function temDepois(faixa: Faixa): boolean {
  return faixa.scrollLeft + faixa.clientWidth < faixa.scrollWidth - FOLGA;
}

/** A geometria do filete, em porcentagem da largura da faixa. */
export interface Filete {
  /** Distância da borda esquerda. */
  esquerda: number;
  /** Comprimento do trecho aceso. */
  largura: number;
}

/**
 * Comprimento mínimo do filete, em porcentagem.
 *
 * Com muitas abas a proporção real fica minúscula — 13 abas dão ~8%, e numa
 * faixa estreita isso vira um ponto de 6px que não se lê como posição. O piso
 * custa precisão e compra legibilidade, que é a troca certa para um indicador
 * que existe por dois segundos.
 */
const MINIMO = 12;

/**
 * Onde o filete acende, dado o estado da faixa.
 *
 * A proporção é a mesma de uma barra de rolagem — visível sobre total —, mas o
 * deslocamento é calculado sobre o espaço QUE SOBRA depois do piso, não sobre a
 * largura toda. Sem isso, com o piso ativo o filete passaria da borda direita
 * no fim da rolagem: ele começaria em 92% e teria 12% de comprimento.
 */
export function filete(faixa: Faixa): Filete {
  const total = Math.max(faixa.scrollWidth, 1);
  const largura = Math.max(MINIMO, (faixa.clientWidth / total) * 100);
  const rolavel = Math.max(faixa.scrollWidth - faixa.clientWidth, 0);
  // Nada a rolar: o filete ocupa a faixa inteira em vez de encolher para o
  // piso e mentir que há mais conteúdo.
  if (rolavel === 0) return { esquerda: 0, largura: 100 };
  const progresso = Math.min(Math.max(faixa.scrollLeft / rolavel, 0), 1);
  return { esquerda: progresso * (100 - largura), largura };
}

/**
 * Quanto rolar para trazer um elemento inteiro para dentro da vista.
 *
 * Devolve `null` quando ele já está visível — e isso importa: rolar "para a
 * posição em que já se está" cancela o gesto de rolagem que o usuário estava
 * fazendo, e a faixa dá um tranco a cada troca de aba.
 *
 * A margem afasta o alvo da borda. Encostado, ele fica sob o fade e parece
 * cortado justamente quando acabou de ser escolhido.
 */
export function trazerParaVista(
  faixa: Faixa,
  alvo: { esquerda: number; largura: number },
  margem = 24,
): number | null {
  const inicioVisivel = faixa.scrollLeft;
  const fimVisivel = faixa.scrollLeft + faixa.clientWidth;
  if (alvo.esquerda < inicioVisivel + margem) {
    return Math.max(0, alvo.esquerda - margem);
  }
  if (alvo.esquerda + alvo.largura > fimVisivel - margem) {
    const destino = alvo.esquerda + alvo.largura + margem - faixa.clientWidth;
    return Math.min(destino, faixa.scrollWidth - faixa.clientWidth);
  }
  return null;
}

/**
 * Quanto a roda do mouse deve rolar a faixa na horizontal.
 *
 * Trackpad já entrega `deltaX` e o navegador rola sozinho — devolver `0` ali
 * evita rolar em dobro. Mouse comum só tem `deltaY`, e sem isto a roda não
 * move a faixa: o usuário gira e nada acontece, que é pior que não ter rolagem.
 */
export function rodaParaHorizontal(evento: {
  deltaX: number;
  deltaY: number;
}): number {
  if (Math.abs(evento.deltaX) > Math.abs(evento.deltaY)) return 0;
  return evento.deltaY;
}
