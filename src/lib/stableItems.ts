/**
 * Devolve `prev` quando `next` tem os mesmos itens (mesma referência, mesma
 * ordem); senão, `next`. Serve para derivar do estado do canvas algo que só
 * muda quando o conteúdo muda: arrastar um nó troca o objeto do nó do xyflow
 * a cada quadro, mas não o modelo dele — e quem lê só o modelo não redesenha.
 */
export function keepIfSame<T>(prev: readonly T[] | null, next: readonly T[]): readonly T[] {
  if (prev === null || prev.length !== next.length) return next;
  for (let i = 0; i < next.length; i++) if (prev[i] !== next[i]) return next;
  return prev;
}
