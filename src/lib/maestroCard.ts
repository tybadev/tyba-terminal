// O que o cartão do nó mostra (quadro MaestroCardsD), sem DOM: o estado do
// anel da marca e o pedido que deixa a cápsula âmbar. Tudo vem do core (o
// estado do nó no run, os pedidos de aprovação); aqui só se escolhe o desenho.

import type { ApprovalRequest } from "./ipc";
import type { NodeRun } from "./maestro";

/**
 * Anel em volta do ícone:
 * - `working`: gira nas cores da marca (o nó trabalha);
 * - `live`: verde cheio, o nó está no ar (processo/serviço com URL);
 * - `waiting`: tracejado (na fila do run, ou esperando o ▶);
 * - `owner`: tracejado escuro sobre a cápsula âmbar (precisa do dono);
 * - `failed`: vermelho; `done`: verde cheio;
 * - `idle`: apagado (fora de execução, parado, pulado).
 */
export type RingState = "idle" | "working" | "live" | "waiting" | "owner" | "failed" | "done";

export function ringState(nodeRun: NodeRun | undefined, needsOwner: boolean): RingState {
  if (needsOwner) return "owner";
  switch (nodeRun?.status) {
    case "running":
      return nodeRun.url ? "live" : "working";
    case "waiting":
      return "waiting";
    case "failed":
      return "failed";
    case "done":
      return "done";
    default:
      return "idle";
  }
}

/**
 * O pedido que a cápsula do nó mostra: o mais antigo da sessão do nó (um por
 * vez; o próximo aparece quando este se resolve). Sem sessão, nenhum.
 */
export function nodeApproval(
  sessionId: string | null,
  approvals: readonly ApprovalRequest[],
): ApprovalRequest | null {
  if (!sessionId) return null;
  let oldest: ApprovalRequest | null = null;
  for (const a of approvals) {
    if (a.session_id !== sessionId) continue;
    if (
      !oldest ||
      a.requested_at_ms < oldest.requested_at_ms ||
      (a.requested_at_ms === oldest.requested_at_ms && a.id < oldest.id)
    ) {
      oldest = a;
    }
  }
  return oldest;
}

/** A cápsula do nó (quadro MaestroCardsD): 40 px no topo, 14 px de cada lado. */
export const CAPSULE = { inset: 14, height: 40 } as const;

/**
 * Abaixo deste zoom a cápsula tem menos de 20 px de altura e o texto dela,
 * menos de 7 px: está no canvas, mas ninguém lê o pedido nem acerta o
 * Permitir. Conta como fora da tela.
 */
export const MIN_CAPSULE_ZOOM = 0.5;

/**
 * Os pedidos que uma cápsula mostra DE FATO agora: saem do toast do canto
 * (acionável em um lugar só, `hiddenApprovalIds`). Antes saía todo pedido
 * com cápsula — também a do nó fora da tela ou com a tela de Serviços por
 * cima, e o pedido ficava sem lugar nenhum para responder. Vale só com o
 * canvas em primeiro plano e a cápsula inteira dentro da área visível
 * (`view` é o transform do canvas, `pane` a área em px); na dúvida o toast
 * fica — pedido em dois lugares é melhor que em nenhum.
 */
export function visibleCardApprovalIds(params: {
  cards: Iterable<{ id: string; rect: { x: number; y: number; w: number; h: number } }>;
  approvals: ReadonlyMap<string, ApprovalRequest>;
  view: { x: number; y: number; zoom: number };
  pane: { w: number; h: number };
  foreground: boolean;
}): Set<number> {
  const ids = new Set<number>();
  const { view, pane } = params;
  if (!params.foreground || view.zoom < MIN_CAPSULE_ZOOM || pane.w <= 0 || pane.h <= 0) {
    return ids;
  }
  for (const { id, rect } of params.cards) {
    const request = params.approvals.get(id);
    if (!request) continue;
    const left = (rect.x + CAPSULE.inset) * view.zoom + view.x;
    const right = (rect.x + rect.w - CAPSULE.inset) * view.zoom + view.x;
    const top = rect.y * view.zoom + view.y;
    const bottom = (rect.y + CAPSULE.height) * view.zoom + view.y;
    if (left >= 0 && top >= 0 && right <= pane.w && bottom <= pane.h) ids.add(request.id);
  }
  return ids;
}
