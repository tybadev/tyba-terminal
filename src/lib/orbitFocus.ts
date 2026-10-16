/**
 * Modo foco do nó do Orbit: o terminal da sessão do nó em tamanho real, por
 * cima do canvas.
 *
 * Fechar NÃO é Esc: o claude usa Esc para interromper o turno, e o foco existe
 * justamente para o dono falar com ele. Fecha pelo botão ⤡ ou por ⌘. (Ctrl+.
 * fora do mac) — nenhum agente nem shell usa essa combinação (o xterm nem tem
 * byte para ela), e no mac é o "cancelar/sair" de sempre.
 */
export function isFocusCloseKey(
  e: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">,
  isMac: boolean,
): boolean {
  if (e.key !== "." || e.altKey || e.shiftKey) return false;
  return isMac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
}
