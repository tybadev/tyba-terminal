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

/**
 * Os pedidos que as cápsulas na tela mostram: saem do toast do canto
 * (acionável em um lugar só, `hiddenApprovalIds`).
 */
export function nodeApprovalIds(
  sessionIds: Iterable<string>,
  approvals: readonly ApprovalRequest[],
): Set<number> {
  const ids = new Set<number>();
  for (const s of sessionIds) {
    const a = nodeApproval(s, approvals);
    if (a) ids.add(a.id);
  }
  return ids;
}
