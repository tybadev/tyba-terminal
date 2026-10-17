// O que o cartão do nó mostra (quadro OrbitCardsD), sem DOM: o estado do
// anel da marca e o pedido que deixa a cápsula âmbar. Tudo vem do core (o
// estado do nó no run, os pedidos de aprovação); aqui só se escolhe o desenho.

import type { ApprovalRequest } from "./ipc";
import { formatDuration, type NodeKind, type NodeRun, type Runner } from "./orbit";

/**
 * Anel em volta do ícone:
 * - `working`: gira nas cores da marca (o nó trabalha);
 * - `live`: verde cheio, o nó está no ar (processo/serviço com URL);
 * - `waiting`: tracejado (na fila do run, ou esperando o ▶);
 * - `owner`: tracejado escuro sobre a cápsula âmbar (precisa do dono);
 * - `failed`: vermelho; `done`: verde cheio (também o agente parado numa
 *   execução assentada, `settled`);
 * - `idle`: apagado (fora de execução, parado, pulado).
 */
export type RingState = "idle" | "working" | "live" | "waiting" | "owner" | "failed" | "done";

export function ringState(
  nodeRun: NodeRun | undefined,
  needsOwner: boolean,
  settled = false,
): RingState {
  if (needsOwner) return "owner";
  // Agente parado numa execução assentada: pronto, o anel não gira.
  if (settled && nodeRun?.status === "running" && nodeRun.orchestrating) return "done";
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

/** A cápsula do nó (quadro OrbitCardsD): 40 px no topo, 14 px de cada lado. */
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

/** O que a sessão do agente do nó reportou (statusline/hook do Claude, no core). */
export interface AgentTelemetry {
  /** Nome do modelo ("Opus 5.5"), ou o id quando não há nome. */
  model: string | null;
  contextPct: number | null;
  linesAdded: number | null;
  linesRemoved: number | null;
}

/** "Claude · Opus 5.5" (quadro OrbitCardsD: "Claude · Opus"); sem modelo, só o agente. */
export function agentName(runner: Runner, model: string | null | undefined): string {
  const base = runner === "claude" ? "Claude" : "Codex";
  const m = model?.replace(/^claude[\s-]+/i, "").trim();
  return m ? `${base} · ${m}` : base;
}

/** "+42 −7" (o que o agente mudou na sessão); vazio se o core não disse. */
export function diffSummary(t: Pick<AgentTelemetry, "linesAdded" | "linesRemoved"> | undefined): string {
  if (!t || (t.linesAdded == null && t.linesRemoved == null)) return "";
  return `+${t.linesAdded ?? 0} −${t.linesRemoved ?? 0}`;
}

/** Uma linha da grade do ⋯: a chave de i18n do rótulo e o valor (`null` = o tempo, que anda). */
export interface DetailRow {
  key: string;
  value: string | null;
}

/**
 * A grade do ⋯ do agente na ordem do quadro OrbitCardsD — em duas colunas,
 * agente | pasta / papel | diff / contexto | tempo — e, depois dela, o que o
 * quadro não tem mas o nó rodando esconde (a tarefa, cuja barra recolhe
 * enquanto roda).
 */
export function agentDetailRows(p: {
  runner: Runner;
  role: string;
  dir: string;
  task: string;
  telemetry: AgentTelemetry | undefined;
}): DetailRow[] {
  const pct = p.telemetry?.contextPct;
  return [
    { key: "orbitDetailAgent", value: agentName(p.runner, p.telemetry?.model) },
    { key: "orbitDetailDir", value: p.dir },
    { key: "orbitRole", value: p.role },
    { key: "orbitDetailDiff", value: diffSummary(p.telemetry) },
    { key: "orbitDetailContext", value: pct == null ? "" : `${Math.round(pct)}%` },
    { key: "orbitDetailTime", value: null },
    { key: "orbitDetailTask", value: p.task },
  ];
}

/**
 * A linha embaixo do comando no card que espera o dono (quadro: "Codex ·
 * revisor · pede há 40s"): quem pede e há quanto tempo.
 */
export function approvalLine(
  kind: NodeKind,
  requestedAtMs: number,
  now: number,
): { who: string; ago: string } {
  const who =
    kind.type === "agent"
      ? [kind.runner === "claude" ? "Claude" : "Codex", kind.role.trim()].filter(Boolean).join(" · ")
      : "";
  return { who, ago: formatDuration(Math.max(0, now - requestedAtMs)) };
}

/**
 * O rodapé do nó que falhou (quadro: "saiu com 1 · Ver log inteiro"): o
 * código de saída e o motivo do core (o que houver), e se há log para ver
 * (processo e integração gravam o `log.txt`). `null` = não falhou.
 */
export function failureFooter(
  kind: NodeKind,
  nodeRun: NodeRun | undefined,
): { exitCode: number | null; reason: string; log: boolean } | null {
  if (nodeRun?.status !== "failed") return null;
  return {
    exitCode: nodeRun.exitCode,
    reason: nodeRun.reason ?? "",
    log: kind.type === "process" || kind.type === "integration",
  };
}

/**
 * O card do processo que caiu mostra o fim do `log.txt` no lugar do
 * terminal: a sessão já morreu (o terminal some quando o nó sai de
 * `running`) e o erro de quem cai em 0s ficava invisível. Só quem rodou
 * (teve sessão) tem log.
 */
export function showsFailureLog(kind: NodeKind, nodeRun: NodeRun | undefined): boolean {
  return kind.type === "process" && nodeRun?.status === "failed" && !!nodeRun.sessionId;
}
