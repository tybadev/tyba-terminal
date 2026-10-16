// Maestro: espelho tipado do modelo e dos commands do core (`src-tauri/src/maestro`).
// Princípio #1: quem valida e grava é o core (`maestro_save_graph`). O que
// mora aqui é só tradução — modelo ↔ nós/ligações do canvas — e a regra de
// ligação válida que o front usa para OFERECER ligações, nunca para decidir.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

// ------------------------------------------------------------ execução

export type RunStatus = "running" | "done" | "failed" | "stopped";

export type NodeStatus = "waiting" | "running" | "done" | "failed" | "skipped" | "stopped";

/** Estado de um nó numa execução, como o core manda (tempos em ms). */
export interface NodeRun {
  status: NodeStatus;
  /** Sessão do terminal do nó. Só aparece no canvas, nunca na grade. */
  sessionId: string | null;
  startedAt: number | null;
  finishedAt: number | null;
  exitCode: number | null;
  /** Tipo da saída → caminho do arquivo com o valor. */
  outputs: Partial<Record<EdgeKind, string>>;
  reason: string | null;
}

export interface Run {
  id: string;
  graphId: string;
  startedAt: number;
  finishedAt: number | null;
  status: RunStatus;
  nodes: Record<string, NodeRun>;
}

/** ▶ Executar: o core recusa grafo não consentido ou inválido. */
export const runStart = (graphId: string) => invoke<string>("maestro_run_start", { graphId });

/** ■ Parar: o core mata o process group de cada nó rodando. */
export const runStop = (runId: string) => invoke<void>("maestro_run_stop", { runId });

export const runGet = (runId: string) => invoke<Run | null>("maestro_run_get", { runId });

/** As 20 últimas execuções do grafo, mais nova primeiro. */
export const runsList = (graphId: string) => invoke<Run[]>("maestro_runs_list", { graphId });

/** O ▶ num nó de integração manual: roda assim que as entradas chegarem. */
export const runNode = (runId: string, nodeId: string) =>
  invoke<void>("maestro_run_node", { runId, nodeId });

/** Cada mudança de estado do run chega com o run inteiro. */
export const onRun = (runId: string, handler: (run: Run) => void): Promise<UnlistenFn> =>
  listen<Run>(`maestro://run/${runId}`, (e) => handler(e.payload));

/** Execuções guardadas por grafo (o core poda o resto). */
export const KEEP_RUNS = 20;

/** O que a tela sabe das execuções: o histórico e a que está à vista. */
export interface RunView {
  /** Mais nova primeiro, no máximo {@link KEEP_RUNS}. */
  runs: Run[];
  /** Execução desenhada no canvas e na linha da execução. */
  selectedId: string | null;
}

export type RunAction =
  | { type: "history"; runs: Run[] }
  | { type: "update"; run: Run }
  | { type: "select"; runId: string }
  | { type: "reset" };

export const EMPTY_RUN_VIEW: RunView = { runs: [], selectedId: null };

const NODE_RANK: Record<NodeStatus, number> = {
  waiting: 0,
  running: 1,
  done: 2,
  failed: 2,
  skipped: 2,
  stopped: 2,
};

/**
 * Quanto o run já andou. Estado de nó só anda para a frente, então um retrato
 * com menos progresso que o guardado é velho (evento que chegou depois do
 * `runGet`, ou o contrário) e não pode desfazer o que já se viu.
 */
export function runProgress(run: Run): number {
  let total = run.status === "running" ? 0 : 1;
  for (const n of Object.values(run.nodes)) {
    total += NODE_RANK[n.status] * 2 + (n.sessionId ? 1 : 0);
  }
  return total;
}

const newestFirst = (runs: Run[]) =>
  [...runs].sort((a, b) => b.startedAt - a.startedAt).slice(0, KEEP_RUNS);

export function runReducer(view: RunView, action: RunAction): RunView {
  switch (action.type) {
    case "reset":
      return EMPTY_RUN_VIEW;
    case "history": {
      // O que já chegou por evento pode ser mais novo que a lista.
      const byId = new Map(view.runs.map((r) => [r.id, r]));
      const listed = new Set(action.runs.map((r) => r.id));
      const runs = newestFirst([
        ...action.runs.map((r) => {
          const known = byId.get(r.id);
          return known && runProgress(known) > runProgress(r) ? known : r;
        }),
        ...view.runs.filter((r) => !listed.has(r.id)),
      ]);
      const keep = runs.some((r) => r.id === view.selectedId);
      return { runs, selectedId: keep ? view.selectedId : (runs[0]?.id ?? null) };
    }
    case "update": {
      const { run } = action;
      const old = view.runs.find((r) => r.id === run.id);
      if (old && runProgress(old) > runProgress(run)) return view;
      const runs = newestFirst([run, ...view.runs.filter((r) => r.id !== run.id)]);
      // Run novo vai para a vista; atualização de outro run não tira o dono
      // de onde ele está olhando.
      const selectedId = old && view.selectedId ? view.selectedId : run.id;
      return { runs, selectedId };
    }
    case "select":
      return view.runs.some((r) => r.id === action.runId)
        ? { ...view, selectedId: action.runId }
        : view;
  }
}

export const selectedRun = (view: RunView): Run | null =>
  view.runs.find((r) => r.id === view.selectedId) ?? null;

/** Número da execução na lista guardada: a mais antiga é #1. */
export const runNumber = (view: RunView, runId: string): number => {
  const i = view.runs.findIndex((r) => r.id === runId);
  return i < 0 ? 0 : view.runs.length - i;
};

/** "3m10s", "42s", "1h02m". */
export function formatDuration(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h${String(m % 60).padStart(2, "0")}m`;
}

/** Tempo do nó: até agora se roda, até o fim se acabou, nada se não começou. */
export function nodeElapsed(n: NodeRun | undefined, now: number): number | null {
  if (!n?.startedAt) return null;
  return (n.finishedAt ?? (n.status === "running" ? now : n.startedAt)) - n.startedAt;
}

/** Chave de i18n e parâmetros: o texto sai da tela, a regra fica testável. */
export interface Label {
  key: string;
  params?: Record<string, string | number>;
}

const baseOf = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

/**
 * Segunda linha da etiqueta da ligação: em que pé está o valor que ela leva,
 * a partir do estado do nó de origem na execução à vista.
 */
export function edgeDetail(kind: EdgeKind, source: NodeRun | undefined): Label | null {
  if (!source) return null;
  switch (source.status) {
    case "waiting":
      return { key: "maestroEdgeWaiting" };
    case "running":
      return { key: "maestroEdgeLive" };
    case "failed":
      return { key: "maestroEdgeSourceFailed" };
    case "skipped":
      return { key: "maestroEdgeSkipped" };
    case "stopped":
      return { key: "maestroEdgeStopped" };
    case "done": {
      const path = source.outputs[kind];
      if (!path) return { key: "maestroEdgeEmpty" };
      return kind === "file"
        ? { key: "maestroEdgeReadyFile", params: { name: baseOf(path) } }
        : { key: "maestroEdgeReady" };
    }
  }
}

/** O que a linha da execução e o nó dizem do estado. */
export function nodeStatusLabel(
  graph: Pick<Graph, "nodes" | "edges">,
  run: Run,
  nodeId: string,
): Label | null {
  const n = run.nodes[nodeId];
  if (!n) return null;
  if (n.status !== "waiting") return { key: `maestroStatus_${n.status}` };
  if (run.status !== "running") return { key: "maestroStatus_waiting" };
  const pending = graph.edges.find(
    (e) => e.to === nodeId && run.nodes[e.from]?.status !== "done",
  );
  if (pending) {
    const from = graph.nodes.find((x) => x.id === pending.from);
    return {
      key: "maestroWaitingFor",
      params: { kind: pending.kind, node: from?.title ?? pending.from },
    };
  }
  return { key: "maestroStatus_waiting" };
}

/**
 * Passos da linha da execução: nós agrupados pela profundidade no grafo (o
 * caminho mais longo desde um nó sem entrada). O mesmo grupo pode rodar junto.
 */
export function timelineSteps(graph: Pick<Graph, "nodes" | "edges">): string[][] {
  const depth = new Map<string, number>();
  const visit = (id: string, guard: Set<string>): number => {
    const known = depth.get(id);
    if (known !== undefined) return known;
    if (guard.has(id)) return 0;
    guard.add(id);
    const d = Math.max(
      0,
      ...graph.edges.filter((e) => e.to === id).map((e) => visit(e.from, guard) + 1),
    );
    guard.delete(id);
    depth.set(id, d);
    return d;
  };
  const groups: string[][] = [];
  for (const n of graph.nodes) {
    // Nota é pronta na hora: não é passo da execução.
    if (n.kind.type === "note") continue;
    const d = visit(n.id, new Set());
    (groups[d] ??= []).push(n.id);
  }
  return groups.filter((g) => g && g.length > 0);
}

/**
 * Dois agentes que podem rodar ao mesmo tempo (nenhum depende do outro). Na
 * mesma pasta, um vê no diff o que o outro mudou — a tela avisa.
 */
export function concurrentAgents(
  graph: Pick<Graph, "nodes" | "edges">,
): [MaestroNode, MaestroNode] | null {
  const agents = graph.nodes.filter((n) => n.kind.type === "agent");
  for (let i = 0; i < agents.length; i++) {
    for (let j = i + 1; j < agents.length; j++) {
      const [a, b] = [agents[i], agents[j]];
      if (!reaches(graph.edges, a.id, b.id) && !reaches(graph.edges, b.id, a.id)) {
        return [a, b];
      }
    }
  }
  return null;
}
