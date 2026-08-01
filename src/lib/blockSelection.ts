import type { Block } from "./ipc";

/**
 * Quais blocos estão marcados para copiar de uma vez.
 *
 * Marcar bloco é diferente de selecionar texto: a lista é virtualizada, e o
 * bloco fora da viewport está desmontado. Uma seleção de texto arrastada por
 * cinquenta blocos copiaria só os que estavam na tela — menos do que foi
 * destacado, sem avisar.
 */
export interface BlockSelection {
  /** Ids marcados. Sem ordem garantida; quem ordena é a lista. */
  ids: number[];
  /** De onde o shift-clique estende. */
  anchor: number;
}

export type SelectMode = "replace" | "range" | "toggle";

/**
 * O teclado é de um campo de texto agora?
 *
 * `Esc` só sai da seleção de blocos quando não é: com o foco na caixa de
 * comando, `Esc` é dela — fecha a sugestão. O `textarea` do xterm entra aqui
 * pelo mesmo motivo (`Esc` é uma tecla que o programa lá dentro espera).
 */
export function inTextField(active: Element | null): boolean {
  if (!active) return false;
  const tag = active.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    (active as HTMLElement).isContentEditable
  );
}

export function modeFor(event: {
  shiftKey: boolean;
  metaKey: boolean;
  ctrlKey: boolean;
}): SelectMode {
  if (event.shiftKey) return "range";
  if (event.metaKey || event.ctrlKey) return "toggle";
  return "replace";
}

function rangeBetween(order: number[], a: number, b: number): number[] {
  const from = order.indexOf(a);
  const to = order.indexOf(b);
  // Bloco que saiu da lista (podado pela retenção enquanto a seleção existia):
  // o intervalo perdeu uma ponta, e marcar só o clicado é o menos surpreendente.
  if (from < 0 || to < 0) return [b];
  const [start, end] = from <= to ? [from, to] : [to, from];
  return order.slice(start, end + 1);
}

/**
 * `order` são os ids na ordem em que a lista os desenha — é o que dá sentido a
 * "daqui até ali".
 *
 * Devolve `null` quando não sobra nada marcado.
 */
export function selectBlock(
  current: BlockSelection | null,
  order: number[],
  id: number,
  mode: SelectMode,
): BlockSelection | null {
  if (mode === "range" && current) {
    return { ids: rangeBetween(order, current.anchor, id), anchor: current.anchor };
  }
  if (mode === "toggle" && current) {
    const ids = current.ids.includes(id)
      ? current.ids.filter((other) => other !== id)
      : [...current.ids, id];
    return ids.length === 0 ? null : { ids, anchor: id };
  }
  // Clicar de novo no único marcado desmarca — é como se sai da seleção sem
  // procurar onde clicar fora.
  if (current && current.ids.length === 1 && current.ids[0] === id) return null;
  return { ids: [id], anchor: id };
}

/**
 * Os blocos marcados, na ordem da lista — nunca na ordem em que foram clicados.
 *
 * Filtra a lista em vez de mapear os ids: id que já não está na lista (podado
 * pela retenção) some sozinho, em vez de virar um furo no meio da cópia.
 */
export function pickedBlocks(
  selection: BlockSelection | null,
  blocks: Block[],
): Block[] {
  if (!selection) return [];
  const marked = new Set(selection.ids);
  return blocks.filter((block) => marked.has(block.id));
}
