// Conversa e memória na tela (quadro MaestroConversa). Quem enfileira,
// entrega e grava é o core (`orbit_send`, `orbit_note_write`); aqui só se lê
// o retrato do run para desenhar a fila, a última mensagem e a nota viva.

import type { Label, NodeRun, OrbitEdge, OrbitNode, Run } from "./orbit";

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

/**
 * Escrita de agente que chegou com o dono digitando na nota: fica guardada e
 * entra quando ele sai do campo, se ele não mudou nada desde que entrou. Se
 * mudou, o texto dele vence (o salvar dele vai ao core, e o agente lê o
 * salvo). Devolve o texto a pôr na nota, ou `null` para deixar como está.
 */
export function writeOnBlur(
  atFocus: string,
  owner: string,
  pending: string | null,
): string | null {
  if (pending === null || owner !== atFocus || pending === owner) return null;
  return pending;
}

type Titled = Pick<OrbitNode, "id" | "title">;
type Linked = Pick<OrbitEdge, "from" | "to" | "kind">;

const titleOf = (nodes: readonly Titled[], id: string) =>
  nodes.find((n) => n.id === id)?.title || id;

/**
 * Agentes ligados a este navegador por `control`, pelo título: a cápsula
 * diz "controlado por X" enquanto a ligação existir.
 */
export function browserControllers(
  browserId: string,
  nodes: readonly Titled[],
  edges: readonly Linked[],
): string[] {
  const ids = new Set(edges.filter((e) => e.to === browserId && e.kind === "control").map((e) => e.from));
  return [...ids].map((id) => titleOf(nodes, id));
}

/**
 * A nota mais recente que este agente escreveu no run ("✎ escreveu em X"):
 * nota ligada a ele por memória cuja última escrita é dele.
 */
export function lastNoteWritten(
  agentId: string,
  nodes: readonly Titled[],
  edges: readonly Linked[],
  run: Pick<Run, "nodes"> | null,
): string | null {
  if (!run) return null;
  const me = titleOf(nodes, agentId);
  let best: { id: string; at: number } | null = null;
  for (const e of edges) {
    if (e.from !== agentId || e.kind !== "memory") continue;
    const w = run.nodes[e.to]?.written;
    if (w && w.by === me && (!best || w.at > best.at)) best = { id: e.to, at: w.at };
  }
  return best && titleOf(nodes, best.id);
}

/** Quem tem mensagem na fila deste agente, pelo título (o rodapé do destino). */
export function inboxSenders(
  nodeRun: Pick<NodeRun, "inboxFrom"> | null | undefined,
  nodes: readonly Titled[],
): string[] {
  return Object.entries(nodeRun?.inboxFrom ?? {})
    .filter(([, n]) => n > 0)
    .map(([id]) => titleOf(nodes, id));
}

/** O motivo curto do "não entregue" do core: sem o prefixo, uma linha, até 60. */
export function undeliveredReason(undelivered: string): string {
  const line = undelivered.replace(/^não entregue:\s*/, "").split("\n")[0].trim();
  return line.length > 60 ? `${line.slice(0, 59)}…` : line;
}

/** Agentes na sala: os que têm alguma ligação de conversa. */
export const roomAgents = (edges: readonly Linked[]): number =>
  new Set(edges.filter((e) => e.kind === "talk").flatMap((e) => [e.from, e.to])).size;

/**
 * Uma linha da nota viva com a autoria que o core põe no `append`
 * (` (dev, 14:22)`): o texto e o sufixo separados; sem o padrão, `null`.
 */
export function lineAuthor(line: string): { text: string; author: string } | null {
  const m = /^(.*\S) (\([^()\n]+, \d{2}:\d{2}\))$/.exec(line);
  return m ? { text: m[1], author: m[2] } : null;
}
