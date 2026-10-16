import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import { useTranslation } from "react-i18next";
import { Plus, Warning } from "@phosphor-icons/react";
import {
  Background,
  BackgroundVariant,
  BaseEdge,
  EdgeLabelRenderer,
  MiniMap,
  Panel,
  ReactFlow,
  applyEdgeChanges,
  applyNodeChanges,
  getBezierPath,
  useReactFlow,
  type FinalConnectionState,
  type Connection,
  type Edge,
  type EdgeChange,
  type EdgeProps,
  type NodeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/base.css";
import { toastError } from "@/lib/toast";
import {
  SUBAGENT_HANDLE,
  SUB_H,
  SUB_COLS,
  SUB_GAP,
  SUB_ROUNDS_H,
  SUB_W,
  gridPosition,
  groupRounds,
  isSubagentNodeId,
  roundsPosition,
  subagentKey,
  subagentNodeId,
  subagentRoundsNodeId,
} from "@/lib/maestroSubagents";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  NEW_IN_HANDLE,
  connectionProblem,
  edgeDetail,
  edgeHasValue,
  createdFlow,
  ephemeralCreated,
  inHandle,
  kindOfOutHandle,
  newNode,
  nodeSession,
  nodeUnderPointer,
  routeNodeChanges,
  type FlowEdgeData,
  type Integration,
  type MaestroNode,
  type NodeKind,
} from "@/lib/maestro";
import { keepIfSame } from "@/lib/stableItems";
import { AgentNode } from "./nodes/AgentNode";
import { BrowserNode } from "./nodes/BrowserNode";
import { IntegrationNode } from "./nodes/IntegrationNode";
import { EDGE_COLOR, useMaestro, type MaestroFlowNode } from "./nodes/NodeFrame";
import { NodeFocus } from "./nodes/NodeFocus";
import { NoteNode } from "./nodes/NoteNode";
import { ProcessNode } from "./nodes/ProcessNode";
import { ServiceNode } from "./nodes/ServiceNode";
import {
  SubagentNode,
  SubagentRoundsNode,
  type SubagentFlowNode,
  type SubagentRoundsFlowNode,
} from "./nodes/SubagentNode";

export type MaestroFlowEdge = Edge<FlowEdgeData>;

/** O que o canvas desenha: os nós do grafo e os filhos efêmeros dos subagentes. */
type CanvasNode = MaestroFlowNode | SubagentFlowNode | SubagentRoundsFlowNode;

const nodeTypes = {
  agent: AgentNode,
  process: ProcessNode,
  note: NoteNode,
  integration: IntegrationNode,
  browser: BrowserNode,
  service: ServiceNode,
  subagent: SubagentNode,
  subagentRounds: SubagentRoundsNode,
};

const SUB_EDGE_STYLE = {
  stroke: "var(--tyba-violet)",
  strokeWidth: 1.4,
  strokeDasharray: "4 4",
};

const EPHEMERAL = {
  draggable: false,
  selectable: false,
  connectable: false,
  deletable: false,
  focusable: false,
} as const;

/**
 * Nó criado pelo orquestrador: responde como qualquer nó (clicar, digitar no
 * terminal, arrastar, redimensionar, selecionar) — com o EPHEMERAL o xyflow
 * punha `pointer-events: none` nele e nada funcionava. Só não se apaga nem
 * se liga: ele é do run, não do grafo, até "Manter no grafo".
 */
const CREATED = { connectable: false, deletable: false } as const;

/** Onde o dono deixou cada criado no canvas (fora do grafo salvo). */
type CreatedView = Record<
  string,
  { position: { x: number; y: number }; width?: number; height?: number; selected?: boolean }
>;

/** Ligação com a etiqueta do tipo no meio (quadro: a "pílula"). */
function MaestroEdgeView({
  id,
  source,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
  selected,
}: EdgeProps<MaestroFlowEdge>) {
  const { t } = useTranslation();
  const [path, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
  });
  const { run } = useMaestro();
  const kind = data?.kind;
  const color = kind ? EDGE_COLOR[kind] : "var(--tyba-text-faint)";
  const sourceRun = run?.nodes[source];
  const detail = kind ? edgeDetail(kind, sourceRun) : null;
  // Quadro: tracejado enquanto o valor não saiu nesta execução.
  const pending =
    sourceRun !== undefined && !(run && kind && edgeHasValue({ from: source, kind }, run));
  return (
    <>
      <BaseEdge
        id={id}
        path={path}
        style={{
          stroke: selected
            ? "var(--tyba-violet)"
            : kind === "context"
              ? "color-mix(in srgb, var(--tyba-violet) 70%, transparent)"
              : kind === "url"
                ? "var(--tyba-cyan)"
                : "color-mix(in srgb, var(--tyba-text) 36%, transparent)",
          strokeWidth: selected ? 2.4 : 1.6,
          strokeDasharray: pending ? "3 6" : undefined,
        }}
      />
      {kind && (
        <EdgeLabelRenderer>
          <div
            className={`nodrag nopan pointer-events-auto absolute flex flex-col items-start gap-0.5 rounded-[7px] border bg-tyba-surface px-2 py-1 font-mono whitespace-nowrap shadow-[var(--tyba-shadow-md)] ${
              sourceRun?.status === "running" ? "border-tyba-cyan/60" : "border-tyba-border-strong"
            }`}
            style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}
          >
            <span className="flex items-center gap-1.5 text-[11px] font-medium text-tyba-text">
              <i className="size-1.5 rounded-[2px]" style={{ background: color }} />
              {t(`maestroEdge_${kind}`)}
              {kind === "context" && (
                <span className="rounded-[4px] border border-tyba-cyan/35 px-1 text-[10px] leading-[15px] font-normal text-tyba-cyan">
                  {t("maestroRedacted")}
                </span>
              )}
            </span>
            {detail && (
              <span className="text-[10.5px] text-tyba-text-faint">
                {t(detail.key, detail.params)}
              </span>
            )}
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  );
}

const edgeTypes = { maestro: MaestroEdgeView };

/**
 * O canvas do grafo. Nós e ligações vêm da tela (estado controlado); aqui
 * mora o que é do xyflow: arrastar, redimensionar, ligar e apagar. O front só
 * OFERECE ligação válida — quem decide é o `maestro_save_graph` no core.
 */
export function MaestroCanvas({
  nodes,
  edges,
  setNodes,
  setEdges,
  integrations,
  showMinimap,
  error,
  newDir,
}: {
  nodes: MaestroFlowNode[];
  edges: MaestroFlowEdge[];
  setNodes: Dispatch<SetStateAction<MaestroFlowNode[]>>;
  setEdges: Dispatch<SetStateAction<MaestroFlowEdge[]>>;
  integrations: readonly Integration[];
  showMinimap: boolean;
  error: string | null;
  /** Pasta onde o nó novo nasce (a última usada, ou a home). */
  newDir: string;
}) {
  const { t } = useTranslation();
  const { screenToFlowPosition } = useReactFlow();
  const wrapper = useRef<HTMLDivElement>(null);
  const byId = useMemo(() => new Map(integrations.map((i) => [i.id, i])), [integrations]);
  const { subagents, focusNode, setFocusNode, run, live } = useMaestro();

  // Nó no modo foco: só enquanto tem sessão viva. Sem ela (o processo acabou,
  // o nó sumiu, trocou o grafo) o foco fecha, para não reabrir sozinho depois.
  const focused = useMemo(() => {
    const flow = focusNode
      ? (nodes.find((n) => n.id === focusNode) ??
        ephemeralCreated(run)
          .map((c) => ({ id: c.node.id, data: { node: c.node } }))
          .find((n) => n.id === focusNode))
      : undefined;
    if (!flow) return null;
    const session = nodeSession(flow.data.node.kind, run, flow.id, live);
    return session ? { node: flow.data.node, session } : null;
  }, [focusNode, nodes, run, live]);
  useEffect(() => {
    if (focusNode && !focused) setFocusNode(null);
  }, [focusNode, focused, setFocusNode]);

  // Nós que o orquestrador criou no run rodando: fora do state (nunca
  // salvos), com o terminal/página de verdade, a ligação tracejada do criador
  // e as ligações url deles. Os mantidos já estão em `nodes`; daqui só sai a
  // tracejada até eles. Onde o dono os arrastou/redimensionou fica em
  // `createdView`, que vale enquanto esse run está à vista.
  const [createdView, setCreatedView] = useState<CreatedView>({});
  const runId = run?.id ?? null;
  useEffect(() => setCreatedView({}), [runId]);
  // Os modelos dos nós ficam os mesmos durante o arrasto (canvas-7): os
  // criados não são refeitos a cada quadro.
  const modelsRef = useRef<readonly MaestroNode[] | null>(null);
  const models = keepIfSame(
    modelsRef.current,
    nodes.map((n) => n.data.node),
  );
  modelsRef.current = models;
  const created = useMemo(() => {
    const flow = createdFlow(run, models);
    const ns: MaestroFlowNode[] = flow.nodes.map((n) => {
      const view = createdView[n.id];
      const width = view?.width ?? n.width;
      const height = view?.height ?? n.height;
      return {
        ...n,
        position: view?.position ?? n.position,
        width,
        height,
        selected: view?.selected ?? false,
        // Com `measured`, o xyflow acha as portas já medidas quando o nó é
        // recriado a cada render.
        measured: { width, height },
        ...CREATED,
      };
    });
    const es: Edge[] = [
      ...flow.byEdges.map((e) => ({ ...e, style: SUB_EDGE_STYLE, ...EPHEMERAL })),
      ...flow.edges.map((e) => ({ ...e, ...EPHEMERAL })),
    ];
    return {
      nodes: ns,
      edges: es,
      nodeIds: new Set(ns.map((n) => n.id)),
      edgeIds: new Set(es.map((e) => e.id)),
    };
  }, [run, models, createdView]);

  // Filhos efêmeros: montados a cada render a partir do nó do agente (do
  // grafo ou criado pelo orquestrador), fora do state `nodes`/`edges` — por
  // isso nunca chegam ao modelo salvo.
  const ephemeral = useMemo(() => {
    const ns: (SubagentFlowNode | SubagentRoundsFlowNode)[] = [];
    const es: Edge[] = [];
    for (const parent of [...nodes, ...created.nodes]) {
      const rounds = groupRounds(subagents.get(parent.id) ?? []);
      if (!rounds) continue;
      const box = {
        position: parent.position,
        width: parent.width ?? parent.data.node.size.w,
        height: parent.height ?? parent.data.node.size.h,
      };
      rounds.current.forEach((run, i) => {
        const id = subagentNodeId(parent.id, subagentKey(run));
        ns.push({
          id,
          type: "subagent",
          position: gridPosition(box, i),
          width: SUB_W,
          height: SUB_H,
          // Com `measured`, o xyflow guarda as portas já medidas quando o nó
          // é recriado; sem ele, a aresta sumiria (as mudanças de medida dos
          // efêmeros não voltam para o state).
          measured: { width: SUB_W, height: SUB_H },
          data: { run },
          ...EPHEMERAL,
        });
        es.push({
          id: `${id}:edge`,
          source: parent.id,
          sourceHandle: SUBAGENT_HANDLE,
          target: id,
          style: SUB_EDGE_STYLE,
          ...EPHEMERAL,
        });
      });
      if (rounds.previous > 0) {
        ns.push({
          id: subagentRoundsNodeId(parent.id),
          type: "subagentRounds",
          position: roundsPosition(box, rounds.current.length),
          width: SUB_COLS * SUB_W + (SUB_COLS - 1) * SUB_GAP,
          height: SUB_ROUNDS_H,
          measured: { width: SUB_COLS * SUB_W + (SUB_COLS - 1) * SUB_GAP, height: SUB_ROUNDS_H },
          data: { count: rounds.previous },
          ...EPHEMERAL,
        });
      }
    }
    return { nodes: ns, edges: es };
  }, [nodes, created.nodes, subagents]);

  const shownNodes = useMemo<CanvasNode[]>(
    () =>
      ephemeral.nodes.length || created.nodes.length
        ? [...nodes, ...created.nodes, ...ephemeral.nodes]
        : nodes,
    [nodes, ephemeral.nodes, created.nodes],
  );
  const shownEdges = useMemo<Edge[]>(
    () =>
      ephemeral.edges.length || created.edges.length
        ? [...edges, ...created.edges, ...ephemeral.edges]
        : edges,
    [edges, ephemeral.edges, created.edges],
  );

  const onNodesChange = useCallback(
    (changes: NodeChange<CanvasNode>[]) => {
      const routed = routeNodeChanges(changes, created.nodeIds, isSubagentNodeId);
      const graph = routed.graph as NodeChange<MaestroFlowNode>[];
      if (graph.length) setNodes((ns) => applyNodeChanges(graph, ns));
      if (routed.created.length) {
        const moved = applyNodeChanges(
          routed.created as NodeChange<MaestroFlowNode>[],
          created.nodes,
        );
        setCreatedView((view) => {
          const next = { ...view };
          for (const n of moved) {
            next[n.id] = {
              position: n.position,
              width: n.width,
              height: n.height,
              selected: n.selected,
            };
          }
          return next;
        });
      }
    },
    [setNodes, created.nodeIds, created.nodes],
  );
  const onEdgesChange = useCallback(
    (changes: EdgeChange<Edge>[]) => {
      const mine = changes.filter(
        (c) => !("id" in c) || (!isSubagentNodeId(c.id) && !created.edgeIds.has(c.id)),
      ) as EdgeChange<MaestroFlowEdge>[];
      if (mine.length) setEdges((es) => applyEdgeChanges(mine, es));
    },
    [setEdges, created.edgeIds],
  );

  const model = useMemo(
    () => ({
      nodes: nodes.map((n) => n.data.node),
      edges: edges.flatMap((e) =>
        e.data ? [{ id: e.id, from: e.source, to: e.target, kind: e.data.kind }] : [],
      ),
    }),
    [nodes, edges],
  );

  const isValidConnection = useCallback(
    (c: Connection | Edge) => {
      const kind = kindOfOutHandle(c.sourceHandle);
      if (!kind || c.targetHandle !== NEW_IN_HANDLE) return false;
      return connectionProblem(model.nodes, model.edges, byId, c.source, c.target, kind) === null;
    },
    [model, byId],
  );

  const onConnect = useCallback(
    (c: Connection) => {
      const kind = kindOfOutHandle(c.sourceHandle);
      if (!kind || !c.sourceHandle) return;
      const id = crypto.randomUUID();
      setEdges((es) => [
        ...es,
        {
          id,
          type: "maestro",
          source: c.source,
          target: c.target,
          sourceHandle: c.sourceHandle,
          targetHandle: inHandle(id),
          data: { kind },
        },
      ]);
    },
    [setEdges],
  );

  // Soltar a ligação em qualquer ponto do nó liga na entrada dele: acertar a
  // bolinha de 14px era o único jeito. O xyflow só informa o nó de destino
  // com o ponteiro a até `connectionRadius` de uma porta; no corpo do nó o
  // `toNode` vem nulo, e o nó sai do que está sob o ponteiro. Ligação
  // recusada diz por quê, em vez de sumir calada.
  const onConnectEnd = useCallback(
    (event: MouseEvent | TouchEvent, state: FinalConnectionState) => {
      if (state.isValid || !state.fromNode || !state.fromHandle) return;
      if (state.fromHandle.type !== "source") return;
      const kind = kindOfOutHandle(state.fromHandle.id);
      if (!kind) return;
      const point = "changedTouches" in event ? event.changedTouches[0] : event;
      const to =
        state.toNode?.id ??
        (point ? nodeUnderPointer(document.elementFromPoint(point.clientX, point.clientY)) : null);
      // Canvas vazio, ou nó que não é do grafo (criado pelo orquestrador,
      // subagente): não liga.
      if (!to || !model.nodes.some((n) => n.id === to)) return;
      const problem = connectionProblem(
        model.nodes,
        model.edges,
        byId,
        state.fromNode.id,
        to,
        kind,
      );
      if (problem) {
        if (problem !== "self") toastError(t(`maestroConnection_${problem}`));
        return;
      }
      onConnect({
        source: state.fromNode.id,
        target: to,
        sourceHandle: state.fromHandle.id ?? null,
        targetHandle: NEW_IN_HANDLE,
      });
    },
    [model, byId, onConnect, t],
  );

  const addNode = (title: string, kind: NodeKind) => {
    const rect = wrapper.current?.getBoundingClientRect();
    const center = rect
      ? screenToFlowPosition({ x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 })
      : { x: 0, y: 0 };
    // Cada nó novo desce um pouco, para não cair exatamente em cima do anterior.
    const nudge = (nodes.length % 6) * 24;
    const node = newNode(crypto.randomUUID(), title, kind, { x: 0, y: 0 });
    node.position = {
      x: Math.round(center.x - node.size.w / 2 + nudge),
      y: Math.round(center.y - node.size.h / 2 + nudge),
    };
    setNodes((ns) => [
      ...ns.map((n) => ({ ...n, selected: false })),
      {
        id: node.id,
        type: node.kind.type,
        position: node.position,
        width: node.size.w,
        height: node.size.h,
        selected: true,
        data: { node },
      },
    ]);
  };

  return (
    <div ref={wrapper} className="relative min-h-0 flex-1 bg-tyba-bg">
      <ReactFlow<CanvasNode, Edge>
        nodes={shownNodes}
        edges={shownEdges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        onConnectEnd={onConnectEnd}
        isValidConnection={isValidConnection}
        connectionRadius={36}
        deleteKeyCode={["Backspace", "Delete"]}
        minZoom={0.2}
        maxZoom={2}
        fitView
        fitViewOptions={{ padding: 0.15, maxZoom: 1 }}
        proOptions={{ hideAttribution: true }}
        connectionLineStyle={{ stroke: "var(--tyba-violet)", strokeWidth: 1.6 }}
        aria-label={t("maestroCanvas")}
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={24}
          size={1.3}
          color="var(--tyba-border-strong)"
        />
        {showMinimap && (
          <MiniMap
            pannable
            zoomable
            bgColor="var(--tyba-surface)"
            maskColor="color-mix(in srgb, var(--tyba-bg) 70%, transparent)"
            nodeColor="var(--tyba-border-strong)"
          />
        )}
        <Panel position="top-left">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                aria-label={t("maestroAddNode")}
                className="flex h-7 items-center gap-1.5 rounded-[6px] border border-tyba-border-strong bg-tyba-surface px-2.5 font-mono text-[12px] text-tyba-text-muted shadow-[var(--tyba-shadow-md)] hover:text-tyba-text"
              >
                <Plus size={13} weight="bold" />
                {t("maestroNode")}
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-56">
              <DropdownMenuItem
                className="text-xs"
                onSelect={() =>
                  addNode("Claude", {
                    type: "agent",
                    runner: "claude",
                    role: "",
                    prompt: "",
                    dir: newDir,
                    orchestrate: false,
                  })
                }
              >
                Claude
              </DropdownMenuItem>
              <DropdownMenuItem
                className="text-xs"
                onSelect={() =>
                  addNode("Codex", {
                    type: "agent",
                    runner: "codex",
                    role: "",
                    prompt: "",
                    dir: newDir,
                    orchestrate: false,
                  })
                }
              >
                Codex
              </DropdownMenuItem>
              <DropdownMenuItem
                className="text-xs"
                onSelect={() =>
                  addNode(t("maestroKindProcess"), {
                    type: "process",
                    command: "",
                    output: "log",
                    file: null,
                    dir: newDir,
                  })
                }
              >
                {t("maestroKindProcess")}
              </DropdownMenuItem>
              <DropdownMenuItem
                className="text-xs"
                onSelect={() => addNode(t("maestroKindNote"), { type: "note", markdown: "" })}
              >
                {t("maestroKindNote")}
              </DropdownMenuItem>
              <DropdownMenuItem
                className="text-xs"
                onSelect={() => addNode(t("maestroKindBrowser"), { type: "browser", url: "" })}
              >
                {t("maestroKindBrowser")}
              </DropdownMenuItem>
              <DropdownMenuItem
                className="text-xs"
                onSelect={() =>
                  addNode(t("maestroKindService"), {
                    type: "service",
                    source: { type: "container", containerId: "", name: "" },
                  })
                }
              >
                {t("maestroKindService")}
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuLabel className="text-[11px] font-normal text-tyba-text-faint">
                {t("maestroIntegrations")}
              </DropdownMenuLabel>
              {integrations.length === 0 && (
                <DropdownMenuItem disabled className="text-xs">
                  {t("maestroNoIntegrations")}
                </DropdownMenuItem>
              )}
              {integrations.map((i) => (
                <DropdownMenuItem
                  key={i.id}
                  className="text-xs"
                  onSelect={() =>
                    addNode(i.name, { type: "integration", integrationId: i.id, dir: newDir })
                  }
                >
                  {i.name}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </Panel>
        {error && (
          <Panel position="top-center">
            <div
              role="alert"
              className="flex max-w-[560px] items-start gap-2 rounded-[6px] border border-tyba-red/50 bg-tyba-red-tint px-3 py-2 font-mono text-[12px] text-tyba-red"
            >
              <Warning size={14} className="mt-px shrink-0" />
              <span>{error}</span>
            </div>
          </Panel>
        )}
        {nodes.length === 0 && (
          <Panel position="top-center" className="!top-1/3">
            <p className="font-mono text-[12.5px] text-tyba-text-faint">{t("maestroEmpty")}</p>
          </Panel>
        )}
      </ReactFlow>
      {focused && <NodeFocus node={focused.node} sessionId={focused.session} />}
    </div>
  );
}
