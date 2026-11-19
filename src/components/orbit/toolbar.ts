// Ferramentas do Orbit (quadro Orbit, 2026-11-15): a barra do Muriki no pé
// do canvas. Puro: a tecla entra, a ferramenta sai. O canvas só age quando o
// teclado não é de outro dono (`keyOwnedElsewhere`).

/** Modo do ponteiro: selecionar (o de sempre) ou mover o canvas. */
export type PointerTool = "cursor" | "hand";

/** O que cada botão de nó cria (a entrada da paleta com esse id). */
export type NodeTool = "agent" | "term" | "proc" | "note" | "web";

export type OrbitTool = PointerTool | NodeTool | "link";

export const NODE_TOOL_ENTRY: Record<NodeTool, string> = {
  agent: "type:claude",
  term: "type:terminal",
  proc: "type:process",
  note: "type:note",
  web: "type:browser",
};

const KEYS: Record<string, OrbitTool> = {
  v: "cursor",
  h: "hand",
  a: "agent",
  t: "term",
  p: "proc",
  n: "note",
  b: "web",
  l: "link",
};

type KeyLike = Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">;

/** A ferramenta da tecla sozinha (V, H, A, T, P, N, B, L), ou `null`. */
export function toolKey(e: KeyLike): OrbitTool | null {
  if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return null;
  return KEYS[e.key.toLowerCase()] ?? null;
}

/**
 * "Ligar" com dois nós selecionados: da esquerda para a direita, que é para
 * onde as ligações correm no canvas. Fora disso, nada a ligar.
 */
export function linkPair(
  selected: readonly { id: string; position: { x: number } }[],
): { from: string; to: string } | null {
  if (selected.length !== 2) return null;
  const [a, b] = selected[0].position.x <= selected[1].position.x ? selected : [selected[1], selected[0]];
  return { from: a.id, to: b.id };
}
