import { useCallback, useMemo, useRef, type Dispatch, type SetStateAction } from "react";
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
  type Connection,
  type Edge,
  type EdgeChange,
  type EdgeProps,
  type NodeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/base.css";

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
  inHandle,
  kindOfOutHandle,
  newNode,
  type FlowEdgeData,
  type Integration,
  type NodeKind,
} from "@/lib/maestro";
import { AgentNode } from "./nodes/AgentNode";
import { IntegrationNode } from "./nodes/IntegrationNode";
import { EDGE_COLOR, useMaestro, type MaestroFlowNode } from "./nodes/NodeFrame";
import { NoteNode } from "./nodes/NoteNode";
import { ProcessNode } from "./nodes/ProcessNode";

export type MaestroFlowEdge = Edge<FlowEdgeData>;

const nodeTypes = {
  agent: AgentNode,
  process: ProcessNode,
  note: NoteNode,
  integration: IntegrationNode,
};

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
  const pending = sourceRun !== undefined && sourceRun.status !== "done";
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
}: {
  nodes: MaestroFlowNode[];
  edges: MaestroFlowEdge[];
  setNodes: Dispatch<SetStateAction<MaestroFlowNode[]>>;
  setEdges: Dispatch<SetStateAction<MaestroFlowEdge[]>>;
  integrations: readonly Integration[];
  showMinimap: boolean;
  error: string | null;
}) {
  const { t } = useTranslation();
  const { screenToFlowPosition } = useReactFlow();
  const wrapper = useRef<HTMLDivElement>(null);
  const byId = useMemo(() => new Map(integrations.map((i) => [i.id, i])), [integrations]);

  const onNodesChange = useCallback(
    (changes: NodeChange<MaestroFlowNode>[]) => setNodes((ns) => applyNodeChanges(changes, ns)),
    [setNodes],
  );
  const onEdgesChange = useCallback(
    (changes: EdgeChange<MaestroFlowEdge>[]) => setEdges((es) => applyEdgeChanges(changes, es)),
    [setEdges],
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
    (c: Connection | MaestroFlowEdge) => {
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
      <ReactFlow<MaestroFlowNode, MaestroFlowEdge>
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        isValidConnection={isValidConnection}
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
                  addNode("Claude", { type: "agent", runner: "claude", role: "", prompt: "" })
                }
              >
                Claude
              </DropdownMenuItem>
              <DropdownMenuItem
                className="text-xs"
                onSelect={() =>
                  addNode("Codex", { type: "agent", runner: "codex", role: "", prompt: "" })
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
                  onSelect={() => addNode(i.name, { type: "integration", integrationId: i.id })}
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
    </div>
  );
}
