import type { BootFailureKind } from "./ipc";

/**
 * Qual título o aviso de arranque leva, pela origem da falha.
 *
 * > [!warning] As duas origens pedem frases OPOSTAS, e por muito tempo levaram
 * > a mesma. Com a thread de boot morta o app está vazio, e o certo é dizer que
 * > sessões e layout podem estar faltando. Com o banco degradado o arranque
 * > terminou inteiro: essa frase promete ao usuário que ele perdeu o que está
 * > vendo na tela. O que pode faltar ali é o histórico que o banco não tinha
 * > para dar.
 *
 * A mensagem não serve para escolher: uma é prosa em pt-BR e a outra é string
 * crua de pânico, sem forma comum para casar.
 *
 * `switch` sem `default` de propósito. Origem nova no core faz o retorno virar
 * `string | undefined` e a atribuição quebrar na compilação — que é o único
 * jeito de a terceira origem não cair calada no ramo do pânico.
 */
export function bootFailureTitleKey(kind: BootFailureKind): string {
  switch (kind) {
    case "bootThreadDied":
      return "bootFailed";
    case "storeDegraded":
      return "bootStoreDegraded";
  }
}
