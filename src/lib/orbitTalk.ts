// Conversa e memória na tela (quadro MaestroConversa). Quem enfileira,
// entrega e grava é o core (`orbit_send`, `orbit_note_write`); aqui só se lê
// o retrato do run para desenhar a fila, a última mensagem e a nota viva.

import type { Label, OrbitEdge, Run } from "./orbit";

/**
 * Mensagens na fila entre os dois agentes da conversa, nos dois sentidos
 * (a etiqueta "1 mensagem na fila"): as de `from` esperando `to` e as de
 * volta.
 */
export function talkQueued(edge: Pick<OrbitEdge, "from" | "to">, run: Run | null): number {
  if (!run) return 0;
  const waiting = (to: string, from: string) => run.nodes[to]?.inboxFrom?.[from] ?? 0;
  return waiting(edge.to, edge.from) + waiting(edge.from, edge.to);
}

/** Quantos agentes leem e escrevem a nota (ligados a ela por memória). */
export const memoryAgents = (noteId: string, edges: readonly Pick<OrbitEdge, "from" | "to" | "kind">[]) =>
  new Set(edges.filter((e) => e.to === noteId && e.kind === "memory").map((e) => e.from)).size;

/**
 * A cápsula da nota viva: "ao vivo · 2 agentes escrevem" com o run rodando,
 * só "2 agentes escrevem" fora dele; com cadeado, eles só leem. Sem agente
 * ligado por memória, nada.
 */
export function noteMemoryLabel(agents: number, locked: boolean, live: boolean): Label | null {
  if (agents === 0) return null;
  const key = locked ? "orbitNoteAgentsRead" : "orbitNoteAgentsWrite";
  return { key: live ? `${key}Live` : key, params: { count: agents } };
}

/** "14:22": a hora local de um instante em ms (rodapé da nota, quem escreveu). */
export function clockTime(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/**
 * A escrita de agente na nota ainda não foi posta na tela: mais nova que a
 * abertura da tela (antes dela, o grafo lido já a traz) e que a última posta.
 */
export const freshWrite = (
  noteId: string,
  at: number,
  applied: ReadonlyMap<string, number>,
  since: number,
): boolean => at > Math.max(since, applied.get(noteId) ?? 0);
