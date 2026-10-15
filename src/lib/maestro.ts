// Maestro: espelho tipado do modelo e dos commands do core (`src-tauri/src/maestro`).
// Princípio #1: quem valida e grava é o core (`maestro_save_graph`). O que
// mora aqui é só tradução — modelo ↔ nós/ligações do canvas — e a regra de
// ligação válida que o front usa para OFERECER ligações, nunca para decidir.

import { invoke } from "@tauri-apps/api/core";

export type Runner = "claude" | "codex";

/** Tipo do valor que uma ligação leva. */
export type EdgeKind = "context" | "diff" | "file" | "log" | "json";

/** Saída declarada de processo ou integração. `context` só o agente produz. */
export type OutputKind = "log" | "json" | "file" | "diff";

export const EDGE_KINDS: readonly EdgeKind[] = [
  "context",
  "diff",
  "file",
  "log",
  "json",
];

export const OUTPUT_KINDS: readonly OutputKind[] = ["log", "json", "file", "diff"];

export type NodeKind =
  | { type: "agent"; runner: Runner; role: string; prompt: string }
  | { type: "process"; command: string; output: OutputKind; file: string | null }
  | { type: "note"; markdown: string }
  | { type: "integration"; integrationId: string };

export interface MaestroNode {
  id: string;
  title: string;
  kind: NodeKind;
  position: { x: number; y: number };
  size: { w: number; h: number };
}

export interface MaestroEdge {
  id: string;
  from: string;
  to: string;
  kind: EdgeKind;
}

export interface Graph {
  id: string;
  name: string;
  /** Pasta do workspace. Todo nó roda nela. */
  dir: string;
  nodes: MaestroNode[];
  edges: MaestroEdge[];
  variables: Record<string, string>;
}

export type Trigger = { type: "onInput"; kind: EdgeKind } | { type: "manual" };

export interface Integration {
  id: string;
  name: string;
  script: string;
  trigger: Trigger;
  output: OutputKind;
  file: string | null;
}

// ---------------------------------------------------------------- IPC

export const listGraphs = () => invoke<Graph[]>("maestro_list_graphs");

export const getGraph = (id: string) =>
  invoke<Graph | null>("maestro_get_graph", { id });

/** Valida no core antes de gravar; o erro de validação volta como string. */
export const saveGraph = (graph: Graph) =>
  invoke<void>("maestro_save_graph", { graph });

export const deleteGraph = (id: string) =>
  invoke<void>("maestro_delete_graph", { id });

export const listIntegrations = () =>
  invoke<Integration[]>("maestro_list_integrations");

export const saveIntegration = (integration: Integration) =>
  invoke<void>("maestro_save_integration", { integration });

export const deleteIntegration = (id: string) =>
  invoke<void>("maestro_delete_integration", { id });

// ---------------------------------------------------- regra de ligação

type IntegrationsById = ReadonlyMap<string, Integration>;

export const integrationsById = (list: readonly Integration[]): IntegrationsById =>
  new Map(list.map((i) => [i.id, i]));

/** O que o nó produz (tabela de rules; espelha `produces` do core). */
export function produces(kind: NodeKind, integrations: IntegrationsById): EdgeKind[] {
  switch (kind.type) {
    case "agent":
      return ["context", "diff"];
    case "process":
      return [kind.output];
    case "note":
      return ["file"];
    case "integration": {
      const i = integrations.get(kind.integrationId);
      return i ? [i.output] : [];
    }
  }
}

/** O que o nó aceita (tabela de rules; espelha `accepts` do core). */
export function accepts(kind: NodeKind, integrations: IntegrationsById): EdgeKind[] {
  switch (kind.type) {
    case "agent":
    case "process":
      return [...EDGE_KINDS];
    case "note":
      return [];
    case "integration": {
      const trigger = integrations.get(kind.integrationId)?.trigger;
      if (!trigger) return [];
      return trigger.type === "manual" ? [...EDGE_KINDS] : [trigger.kind];
    }
  }
}

/** `to` alcança `from` pelas ligações existentes? Então `from → to` fecha ciclo. */
function reaches(edges: readonly MaestroEdge[], start: string, goal: string): boolean {
  const seen = new Set<string>();
  const stack = [start];
  while (stack.length > 0) {
    const id = stack.pop()!;
    if (id === goal) return true;
    if (seen.has(id)) continue;
    seen.add(id);
    for (const e of edges) if (e.from === id) stack.push(e.to);
  }
  return false;
}

export type ConnectionProblem =
  | "unknownNode"
  | "self"
  | "notProduced"
  | "notAccepted"
  | "duplicate"
  | "cycle";

/**
 * Por que a ligação `from → to` do tipo `kind` não vale, ou `null` se vale.
 * Mesmas regras do `validate_graph` do core; a ligação repetida (mesma
 * origem, destino e tipo) o core aceitaria, mas não acrescenta nada.
 */
export function connectionProblem(
  nodes: readonly MaestroNode[],
  edges: readonly MaestroEdge[],
  integrations: IntegrationsById,
  from: string,
  to: string,
  kind: EdgeKind,
): ConnectionProblem | null {
  const source = nodes.find((n) => n.id === from);
  const target = nodes.find((n) => n.id === to);
  if (!source || !target) return "unknownNode";
  if (from === to) return "self";
  if (!produces(source.kind, integrations).includes(kind)) return "notProduced";
  if (!accepts(target.kind, integrations).includes(kind)) return "notAccepted";
  if (edges.some((e) => e.from === from && e.to === to && e.kind === kind)) {
    return "duplicate";
  }
  if (reaches(edges, to, from)) return "cycle";
  return null;
}

// ------------------------------------------- modelo ↔ canvas (xyflow)

/** Porta de saída: uma por tipo que o nó produz. */
export const outHandle = (kind: EdgeKind) => `out:${kind}`;

/** Porta de entrada livre, onde nasce uma ligação nova. */
export const NEW_IN_HANDLE = "in";

/** Porta de entrada de uma ligação que já existe: uma linha por entrada. */
export const inHandle = (edgeId: string) => `in:${edgeId}`;

export function kindOfOutHandle(handle: string | null | undefined): EdgeKind | null {
  if (!handle?.startsWith("out:")) return null;
  const kind = handle.slice(4) as EdgeKind;
  return EDGE_KINDS.includes(kind) ? kind : null;
}

/** O que o canvas guarda em cada nó: o nó do modelo, inteiro. */
export type FlowNodeData = { node: MaestroNode };
export type FlowEdgeData = { kind: EdgeKind };

/** Recorte do `Node` do xyflow que a conversão usa (mantém o teste sem DOM). */
export interface FlowNode {
  id: string;
  type: NodeKind["type"];
  position: { x: number; y: number };
  width?: number;
  height?: number;
  data: FlowNodeData;
}

export interface FlowEdge {
  id: string;
  type: "maestro";
  source: string;
  target: string;
  sourceHandle: string;
  targetHandle: string;
  data: FlowEdgeData;
}

export function toFlow(graph: Graph): { nodes: FlowNode[]; edges: FlowEdge[] } {
  return {
    nodes: graph.nodes.map((node) => ({
      id: node.id,
      type: node.kind.type,
      position: { ...node.position },
      width: node.size.w,
      height: node.size.h,
      data: { node },
    })),
    edges: graph.edges.map((edge) => ({
      id: edge.id,
      type: "maestro",
      source: edge.from,
      target: edge.to,
      sourceHandle: outHandle(edge.kind),
      targetHandle: inHandle(edge.id),
      data: { kind: edge.kind },
    })),
  };
}

/** Recorte da ligação do xyflow que volta para o modelo. */
export interface FlowEdgeLike {
  id: string;
  source: string;
  target: string;
  sourceHandle?: string | null;
  data?: FlowEdgeData;
}

/**
 * O grafo que vai para o core: metadados de `base`, nós e ligações do canvas.
 * Posição e tamanho saem do canvas (é onde o dono arrasta e redimensiona); o
 * resto do nó sai do `data`.
 */
export function fromFlow(
  base: Pick<Graph, "id" | "name" | "dir" | "variables">,
  nodes: readonly Pick<FlowNode, "id" | "position" | "width" | "height" | "data">[],
  edges: readonly FlowEdgeLike[],
): Graph {
  return {
    id: base.id,
    name: base.name,
    dir: base.dir,
    variables: { ...base.variables },
    nodes: nodes.map((n) => ({
      ...n.data.node,
      id: n.id,
      position: { x: n.position.x, y: n.position.y },
      size: {
        w: n.width ?? n.data.node.size.w,
        h: n.height ?? n.data.node.size.h,
      },
    })),
    edges: edges.flatMap((e) => {
      const kind = e.data?.kind ?? kindOfOutHandle(e.sourceHandle);
      return kind ? [{ id: e.id, from: e.source, to: e.target, kind }] : [];
    }),
  };
}

/** Tamanho inicial de cada tipo de nó (quadro MaestroHarness: nós grandes). */
export const DEFAULT_SIZE: Record<NodeKind["type"], { w: number; h: number }> = {
  agent: { w: 360, h: 300 },
  process: { w: 360, h: 250 },
  note: { w: 360, h: 170 },
  integration: { w: 360, h: 200 },
};

export function newNode(
  id: string,
  title: string,
  kind: NodeKind,
  position: { x: number; y: number },
): MaestroNode {
  return { id, title, kind, position, size: { ...DEFAULT_SIZE[kind.type] } };
}

/** Grafo novo de uma pasta: vazio, com o nome da pasta. */
export function emptyGraph(id: string, dir: string): Graph {
  const name = dir.split(/[\\/]/).filter(Boolean).pop() ?? dir;
  return { id, name, dir, nodes: [], edges: [], variables: {} };
}
