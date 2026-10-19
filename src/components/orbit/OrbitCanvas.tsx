import {
  Fragment,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import { useTranslation } from "react-i18next";
import { CaretDown, Check, Trash, Warning } from "@phosphor-icons/react";
import {
  Background,
  BackgroundVariant,
  BaseEdge,
  EdgeLabelRenderer,
  MiniMap,
  Panel,
  ReactFlow,
  ViewportPortal,
  applyEdgeChanges,
  applyNodeChanges,
  getBezierPath,
  useReactFlow,
  useStore,
  type FinalConnectionState,
  type Connection,
  type Edge,
  type EdgeChange,
  type EdgeProps,
  type NodeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/base.css";
import { toastError } from "@/lib/toast";
import { clipBounds, copyNodes, pasteClip, type Clip } from "@/lib/graphClipboard";
import { bundleEdges, unbundleEdgeChanges, type BundledEdgeData } from "@/lib/edgeBundle";
import { canvasKey, enterTerminal, keyOwnedElsewhere } from "@/lib/graphKeys";
import {
  freeSpot,
  minimapTone,
  nodeRect,
  outsideView,
  rightOf,
} from "@/lib/graphLayout";
import { paletteEntries, recentDirs, type PaletteEntry } from "@/lib/graphPalette";
import { listSnippets, type Snippet } from "@/lib/ipc";
import { ringState, visibleCardApprovalIds } from "@/lib/orbitCard";
import { SNAP_PX, deletePlan, snapToGuides, type Guide } from "@/lib/orbitHandling";
import { IS_MAC } from "@/lib/platform";
import { getTerm } from "@/lib/termRegistry";
import {
  SUBAGENT_HANDLE,
  SUB_H,
  SUB_COLS,
  SUB_GAP,
  SUB_ROUNDS_H,
  SUB_W,
  blockOrigins,
  blockSize,
  groupRounds,
  isSubagentNodeId,
  pillPosition,
  roundsAt,
  subagentKey,
  subagentNodeId,
  subagentRoundsNodeId,
  type BlockOffsets,
} from "@/lib/orbitSubagents";

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
  OUT_HANDLE,
  edgeDetail,
  edgeHasValue,
  edgeKindChoices,
  createdFlow,
  ephemeralCreated,
  lastDir,
  newNode,
  pickConnection,
  shownSession,
  nodeUnderPointer,
  produces,
  rememberDir,
  routeNodeChanges,
  runStopNode,
  type EdgeKind,
  type FlowEdgeData,
  type Integration,
  type OrbitEdge,
  type OrbitNode,
} from "@/lib/orbit";
import { talkQueued } from "@/lib/orbitTalk";
import { keepIfSame } from "@/lib/stableItems";
import { AgentNode } from "./nodes/AgentNode";
import { BrowserNode } from "./nodes/BrowserNode";
import { IntegrationNode } from "./nodes/IntegrationNode";
import {
  EDGE_COLOR,
  NodeDeleteContext,
  useOrbit,
  type NodeDelete,
  type OrbitFlowNode,
} from "./nodes/NodeFrame";
import { NodeFocus } from "./nodes/NodeFocus";
import { NodePalette, PALETTE_H, PALETTE_W } from "./NodePalette";
import { NoteNode } from "./nodes/NoteNode";
import { ProcessNode } from "./nodes/ProcessNode";
import { ServiceNode } from "./nodes/ServiceNode";
import { useTerminalPane } from "@/components/TerminalPaneHost";
import { TerminalNode } from "./nodes/TerminalNode";
import {
  SubagentNode,
  SubagentRoundsNode,
  type SubagentFlowNode,
  type SubagentRoundsFlowNode,
} from "./nodes/SubagentNode";

export type OrbitFlowEdge = Edge<FlowEdgeData>;

/** O que o canvas desenha: os nós do grafo e os filhos efêmeros dos subagentes. */
type CanvasNode = OrbitFlowNode | SubagentFlowNode | SubagentRoundsFlowNode;

const nodeTypes = {
  agent: AgentNode,
  process: ProcessNode,
  note: NoteNode,
  integration: IntegrationNode,
  browser: BrowserNode,
  service: ServiceNode,
  terminal: TerminalNode,
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
 * Pílula de subagente: arrasta (nos-8), com a posição na vista dela; não se
 * seleciona, liga nem apaga — é do claude, não do grafo.
 */
const SUBAGENT = { ...EPHEMERAL, draggable: true } as const;

/**
 * Nó criado pelo orquestrador: responde como qualquer nó (clicar, digitar no
 * terminal, arrastar, redimensionar, selecionar) — com o EPHEMERAL o xyflow
 * punha `pointer-events: none` nele e nada funcionava. Só não se liga: ele é
 * do run, não do grafo, até "Manter no grafo". Apagar (lixeira ou Delete)
 * descarta: some da vista do run (`onBeforeDelete`).
 */
const CREATED = { connectable: false } as const;

/** Onde o dono deixou cada criado no canvas (fora do grafo salvo). */
export type CreatedView = Record<
  string,
  {
    position: { x: number; y: number };
    width?: number;
    height?: number;
    selected?: boolean;
    /** No meio do gesto (a sombra do arrasto, a cápsula "L × A"). */
    dragging?: boolean;
    resizing?: boolean;
  }
>;

/** O que a etiqueta da ligação faz: trocar o tipo e remover (só ligação do grafo). */
interface EdgeActions {
  /** Tipos para os quais a ligação pode ir; vazio = não é do grafo (sem menu). */
  choices: (edgeId: string) => EdgeKind[];
  retype: (edgeId: string, kind: EdgeKind) => void;
  remove: (edgeId: string) => void;
}

const EdgeActionsContext = createContext<EdgeActions>({
  choices: () => [],
  retype: () => {},
  remove: () => {},
});

/**
 * Ligação com a etiqueta do tipo no meio (quadro OrbitCanvasLiso:
 * "contexto ▾"). A etiqueta é menu: troca o tipo entre os que o par aceita e
 * remove a ligação — o tipo calculado ao ligar nunca fica preso. Várias
 * ligações do mesmo par (grafo de antes da porta única) chegam juntas
 * (`bundleEdges`): uma curva só, "contexto + diff", cada tipo com o seu menu.
 */
function OrbitEdgeView({
  id,
  source,
  target,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
  selected,
}: EdgeProps<Edge<BundledEdgeData>>) {
  const { t } = useTranslation();
  const [path, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
  });
  const { run } = useOrbit();
  const members = data ? (data.bundle ?? [{ id, kind: data.kind }]) : [];
  // Juntas, a curva leva a cor neutra; cada tipo tem a sua na etiqueta.
  const kind = members.length === 1 ? members[0].kind : undefined;
  const sourceRun = run?.nodes[source];
  // Quadro: tracejado enquanto o valor não saiu nesta execução.
  const pending =
    sourceRun !== undefined &&
    members.some((m) => !(run && edgeHasValue({ from: source, kind: m.kind }, run)));
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
              : kind === "url" ||
                  kind === "plan" ||
                  kind === "control" ||
                  kind === "talk" ||
                  kind === "memory"
                ? EDGE_COLOR[kind]
                : "color-mix(in srgb, var(--tyba-text) 36%, transparent)",
          strokeWidth: selected ? 2.4 : 1.6,
          strokeDasharray: pending ? "3 6" : undefined,
        }}
      />
      {members.length > 0 && (
        <EdgeLabelRenderer>
          <div
            className="nodrag nopan pointer-events-auto absolute flex flex-col items-start gap-0.5 font-mono whitespace-nowrap"
            style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}
          >
            <span className="flex items-center gap-1">
              {members.map((m, i) => (
                <Fragment key={m.id}>
                  {i > 0 && <span className="text-[11px] text-tyba-text-faint">+</span>}
                  <EdgeLabel id={m.id} kind={m.kind} color={EDGE_COLOR[m.kind]} />
                </Fragment>
              ))}
            </span>
            {members.map((m) => {
              // Conversa: a fila do par (quadro: "1 mensagem na fila").
              const queued = m.kind === "talk" ? talkQueued({ from: source, to: target }, run) : 0;
              const detail =
                queued > 0
                  ? { key: "orbitTalkQueued", params: { count: queued } }
                  : edgeDetail(m.kind, sourceRun);
              return (
                detail && (
                  <span
                    key={m.id}
                    className="rounded-[4px] bg-tyba-bg/80 px-1 text-[10.5px] text-tyba-text-faint"
                  >
                    {t(detail.key, detail.params)}
                  </span>
                )
              );
            })}
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  );
}

/** "contexto ▾": o tipo da ligação e o menu para trocar ou remover. */
function EdgeLabel({ id, kind, color }: { id: string; kind: EdgeKind; color: string }) {
  const { t } = useTranslation();
  const { choices, retype, remove } = useContext(EdgeActionsContext);
  const options = choices(id);
  const pill = (
    <>
      <i className="size-1.5 rounded-full" style={{ background: color }} />
      {t(`orbitEdge_${kind}`)}
      {kind === "context" && (
        <span className="rounded-[4px] border border-tyba-cyan/35 px-1 text-[10px] leading-[14px] text-tyba-cyan">
          {t("orbitRedacted")}
        </span>
      )}
    </>
  );
  const pillClass =
    "flex items-center gap-1.5 rounded-[4px] border bg-tyba-raised px-1.5 text-[11px] leading-[18px] text-tyba-text shadow-[var(--tyba-shadow-md)]";
  const border = { borderColor: `color-mix(in srgb, ${color} 60%, transparent)` };
  if (options.length === 0) {
    return (
      <span className={pillClass} style={border}>
        {pill}
      </span>
    );
  }
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={t("orbitEdgeMenu", { kind: t(`orbitEdge_${kind}`) })}
          className={`${pillClass} hover:bg-tyba-overlay`}
          style={border}
        >
          {pill}
          <CaretDown size={9} weight="bold" className="text-tyba-text-faint" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="center" className="w-48">
        <DropdownMenuLabel className="text-[11px] font-normal text-tyba-text-faint">
          {t("orbitEdgeKind")}
        </DropdownMenuLabel>
        {options.map((k) => (
          <DropdownMenuItem
            key={k}
            className="text-xs"
            onSelect={() => k !== kind && retype(id, k)}
          >
            <i className="size-1.5 rounded-full" style={{ background: EDGE_COLOR[k] }} />
            <span className="flex-1">{t(`orbitEdge_${k}`)}</span>
            {k === kind && <Check size={12} />}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem className="text-xs text-tyba-red" onSelect={() => remove(id)}>
          <Trash size={12} />
          {t("orbitEdgeRemove")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

const edgeTypes = { orbit: OrbitEdgeView };

/** Onde a paleta abriu: o canto dela no canvas, o ponto no grafo e de onde veio. */
interface PaletteAt {
  /** Canto da paleta, em px dentro do canvas. */
  screen: { x: number; y: number };
  /** O ponto no grafo onde o nó nasce. */
  flow: { x: number; y: number };
  /** Ligação solta no vazio: o nó de origem (o novo nasce ligado nele). */
  from: string | null;
  /** ⌘K ou "/": o nó novo nasce perto do selecionado, se houver. */
  near: boolean;
}

/** O nó do canvas para um nó do modelo. */
const flowNodeOf = (node: OrbitNode, selected: boolean): OrbitFlowNode => ({
  id: node.id,
  type: node.kind.type,
  position: node.position,
  width: node.size.w,
  height: node.size.h,
  selected,
  data: { node },
});

const modKey = IS_MAC ? "⌘" : "Ctrl+";

/**
 * O canvas do grafo. Nós e ligações vêm da tela (estado controlado); aqui
 * mora o que é do xyflow: arrastar, redimensionar, ligar e apagar. O front só
 * OFERECE ligação válida — quem decide é o `orbit_save_graph` no core.
 * Nó novo nasce pela paleta (duplo clique, ⌘K, "/" ou ligação solta no
 * vazio), sempre num espaço livre.
 */
export function OrbitCanvas({
  nodes,
  edges,
  setNodes,
  setEdges,
  integrations,
  minimap,
  onOverflow,
  createdView,
  setCreatedView,
  onUndo,
  onRedo,
  onRemoved,
  error,
  newDir,
  foreground,
  onCardApprovals,
  onAddTerminal,
}: {
  nodes: OrbitFlowNode[];
  edges: OrbitFlowEdge[];
  setNodes: Dispatch<SetStateAction<OrbitFlowNode[]>>;
  setEdges: Dispatch<SetStateAction<OrbitFlowEdge[]>>;
  integrations: readonly Integration[];
  /** Minimapa: ligado/desligado pelo dono, ou `null` = sozinho (liga quando o grafo passa da tela). */
  minimap: boolean | null;
  /** Algum nó saiu da área visível (o minimapa automático segue isto). */
  onOverflow: (overflow: boolean) => void;
  /** Onde o dono deixou cada criado (vale enquanto o run está à vista). */
  createdView: CreatedView;
  setCreatedView: Dispatch<SetStateAction<CreatedView>>;
  onUndo: () => void;
  onRedo: () => void;
  /** Nós do grafo apagados agora (o aviso "Nota “X” apagada · Desfazer"). */
  onRemoved: (removed: readonly OrbitNode[]) => void;
  error: string | null;
  /** Pasta onde o nó novo nasce (a última usada, ou a home). */
  newDir: string;
  /** Nada da tela cobre o canvas (Serviços, Integrações, importar). */
  foreground: boolean;
  /** Os pedidos que uma cápsula mostra na tela agora: saem do toast. */
  onCardApprovals: (ids: ReadonlySet<number>) => void;
  /** Terminal da paleta: a tela pergunta a pasta e cria o nó ali, já aberto. */
  onAddTerminal: (position: { x: number; y: number }) => void;
}) {
  const paneHost = useTerminalPane();
  const { t } = useTranslation();
  const { screenToFlowPosition, zoomTo, fitView, setCenter } = useReactFlow();
  const wrapper = useRef<HTMLDivElement>(null);
  const byId = useMemo(() => new Map(integrations.map((i) => [i.id, i])), [integrations]);
  const { subagents, focusNode, setFocusNode, run, runs, live, approvals, createdIds } = useOrbit();

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
    const session = shownSession(flow.data.node.kind, run, runs, flow.id, live);
    return session ? { node: flow.data.node, session } : null;
  }, [focusNode, nodes, run, runs, live]);
  useEffect(() => {
    if (focusNode && !focused) setFocusNode(null);
  }, [focusNode, focused, setFocusNode]);

  // Nós que o orquestrador criou no run rodando: fora do state (nunca
  // salvos), com o terminal/página de verdade, a ligação tracejada do criador
  // e as ligações url deles. Os mantidos já estão em `nodes`; daqui só sai a
  // tracejada até eles. Onde o dono os arrastou/redimensionou fica em
  // `createdView` (da tela, que leva a posição ao "Manter no grafo").
  // Os modelos dos nós ficam os mesmos durante o arrasto (canvas-7): os
  // criados não são refeitos a cada quadro.
  const modelsRef = useRef<readonly OrbitNode[] | null>(null);
  const models = keepIfSame(
    modelsRef.current,
    nodes.map((n) => n.data.node),
  );
  modelsRef.current = models;
  // Criados que o dono descartou (lixeira): somem da vista deste run.
  const [discarded, setDiscarded] = useState<ReadonlySet<string>>(new Set());
  const runId = run?.id ?? null;
  useEffect(() => setDiscarded(new Set()), [runId]);
  const created = useMemo(() => {
    const flow = createdFlow(run, models);
    const shown = (id: string) => !discarded.has(id);
    const ns: OrbitFlowNode[] = flow.nodes.filter((n) => shown(n.id)).map((n) => {
      const view = createdView[n.id];
      const width = view?.width ?? n.width;
      const height = view?.height ?? n.height;
      return {
        ...n,
        position: view?.position ?? n.position,
        width,
        height,
        selected: view?.selected ?? false,
        dragging: view?.dragging,
        resizing: view?.resizing,
        // Com `measured`, o xyflow acha as portas já medidas quando o nó é
        // recriado a cada render.
        measured: { width, height },
        ...CREATED,
      };
    });
    const es: Edge[] = [
      ...flow.byEdges.map((e) => ({ ...e, style: SUB_EDGE_STYLE, ...EPHEMERAL })),
      ...flow.edges.map((e) => ({ ...e, ...EPHEMERAL })),
    ].filter((e) => shown(e.source) && shown(e.target));
    return {
      nodes: ns,
      edges: es,
      nodeIds: new Set(ns.map((n) => n.id)),
      edgeIds: new Set(es.map((e) => e.id)),
    };
  }, [run, models, createdView, discarded]);

  // Filhos efêmeros: montados a cada render a partir do nó do agente (do
  // grafo ou criado pelo orquestrador), fora do state `nodes`/`edges` — por
  // isso nunca chegam ao modelo salvo. O bloco de pílulas cai embaixo do pai
  // num espaço livre (nos-8); a pílula que o dono arrastou fica onde ele a pôs.
  const [subView, setSubView] = useState<Record<string, { x: number; y: number }>>({});
  // Deslocamento de cada bloco ao pai: durante o arrasto o bloco segue o pai
  // sem refazer o desvio de todos a cada quadro (ver `blockOrigins`).
  const blockOffsets = useRef<BlockOffsets | null>(null);
  const ephemeral = useMemo(() => {
    const ns: (SubagentFlowNode | SubagentRoundsFlowNode)[] = [];
    const es: Edge[] = [];
    const parents = [...nodes, ...created.nodes].map((parent) => {
      const rounds = groupRounds(subagents.get(parent.id) ?? []);
      return {
        id: parent.id,
        rounds,
        box: {
          position: parent.position,
          width: parent.width ?? parent.data.node.size.w,
          height: parent.height ?? parent.data.node.size.h,
        },
        size: rounds ? blockSize(rounds.current.length, rounds.previous > 0) : null,
      };
    });
    const dragging =
      nodes.some((n) => n.dragging) || created.nodes.some((n) => n.dragging);
    const { origins, offsets } = blockOrigins(parents, blockOffsets.current, dragging);
    blockOffsets.current = offsets;
    for (const parent of parents) {
      const rounds = parent.rounds;
      const origin = origins.get(parent.id);
      if (!rounds || !origin) continue;
      rounds.current.forEach((run, i) => {
        const id = subagentNodeId(parent.id, subagentKey(run));
        ns.push({
          id,
          type: "subagent",
          position: subView[id] ?? pillPosition(origin, i),
          width: SUB_W,
          height: SUB_H,
          // Com `measured`, o xyflow guarda as portas já medidas quando o nó
          // é recriado; sem ele, a aresta sumiria (as mudanças de medida dos
          // efêmeros não voltam para o state).
          measured: { width: SUB_W, height: SUB_H },
          data: { run },
          ...SUBAGENT,
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
          position: roundsAt(origin, rounds.current.length),
          width: SUB_COLS * SUB_W + (SUB_COLS - 1) * SUB_GAP,
          height: SUB_ROUNDS_H,
          measured: { width: SUB_COLS * SUB_W + (SUB_COLS - 1) * SUB_GAP, height: SUB_ROUNDS_H },
          data: { count: rounds.previous },
          ...EPHEMERAL,
        });
      }
    }
    return { nodes: ns, edges: es };
  }, [nodes, created.nodes, subagents, subView]);

  const shownNodes = useMemo<CanvasNode[]>(
    () =>
      ephemeral.nodes.length || created.nodes.length
        ? [...nodes, ...created.nodes, ...ephemeral.nodes]
        : nodes,
    [nodes, ephemeral.nodes, created.nodes],
  );
  // A mesma ligação do par desenhada uma vez (`bundleEdges`); o state segue com todas.
  const bundled = useMemo(() => bundleEdges(edges), [edges]);
  const shownEdges = useMemo<Edge[]>(
    () =>
      ephemeral.edges.length || created.edges.length
        ? [...bundled.edges, ...created.edges, ...ephemeral.edges]
        : (bundled.edges as Edge[]),
    [bundled.edges, ephemeral.edges, created.edges],
  );

  // O arrasto lê os nós por ref: o `onNodesChange` não muda a cada quadro.
  const shownRef = useRef(shownNodes);
  shownRef.current = shownNodes;
  const nodesRef = useRef(nodes);
  nodesRef.current = nodes;

  // Guias violeta do arrasto (quadro OrbitManuseio 2): estado de tela.
  const [guides, setGuides] = useState<Guide[]>([]);
  const zoom = useStore((s) => s.transform[2]);
  /**
   * Um card arrastado sozinho gruda de leve (6px na tela) na borda ou no
   * centro dos outros. O xyflow refaz a posição a partir do ponteiro a cada
   * passo, então ajustar a mudança basta — inclusive a do soltar.
   */
  const snapDrag = useCallback(
    (changes: NodeChange<CanvasNode>[]): NodeChange<CanvasNode>[] => {
      const drags = changes.filter(
        (c) => c.type === "position" && typeof c.dragging === "boolean" && c.position,
      );
      const one = drags.length === 1 ? drags[0] : null;
      if (!one || one.type !== "position" || !one.position || isSubagentNodeId(one.id)) {
        if (drags.length) setGuides((g) => (g.length ? [] : g));
        return changes;
      }
      const all = shownRef.current;
      const me = all.find((n) => n.id === one.id);
      if (!me) return changes;
      const others = all
        .filter((n) => n.id !== one.id && n.type !== "subagent" && n.type !== "subagentRounds")
        .map(nodeRect);
      const snap = snapToGuides({ ...nodeRect(me), ...one.position }, others, SNAP_PX / zoom);
      const next = one.dragging ? snap.guides : [];
      setGuides((g) => (JSON.stringify(g) === JSON.stringify(next) ? g : next));
      return changes.map((c) => (c === one ? { ...one, position: { x: snap.x, y: snap.y } } : c));
    },
    [zoom],
  );

  const onNodesChange = useCallback(
    (raw: NodeChange<CanvasNode>[]) => {
      const changes = snapDrag(raw);
      const routed = routeNodeChanges(changes, created.nodeIds, isSubagentNodeId);
      const graph = routed.graph as NodeChange<OrbitFlowNode>[];
      if (graph.length) setNodes((ns) => applyNodeChanges(graph, ns));
      const removedIds = new Set(graph.flatMap((c) => (c.type === "remove" ? [c.id] : [])));
      if (removedIds.size > 0) {
        onRemoved(
          nodesRef.current.filter((n) => removedIds.has(n.id)).map((n) => n.data.node),
        );
      }
      if (routed.created.length) {
        const moved = applyNodeChanges(
          routed.created as NodeChange<OrbitFlowNode>[],
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
              dragging: n.dragging,
              resizing: n.resizing,
            };
          }
          return next;
        });
      }
      if (routed.ephemeral.length) {
        setSubView((view) => {
          const next = { ...view };
          for (const c of routed.ephemeral) {
            if (c.type === "position" && c.position) next[c.id] = c.position;
          }
          return next;
        });
      }
    },
    [snapDrag, setNodes, created.nodeIds, created.nodes, setCreatedView, onRemoved],
  );
  const onEdgesChange = useCallback(
    (changes: EdgeChange<Edge>[]) => {
      const mine = unbundleEdgeChanges(
        changes.filter(
          (c) => !("id" in c) || (!isSubagentNodeId(c.id) && !created.edgeIds.has(c.id)),
        ),
        bundled.members,
      ) as EdgeChange<OrbitFlowEdge>[];
      if (mine.length) setEdges((es) => applyEdgeChanges(mine, es));
    },
    [setEdges, created.edgeIds, bundled.members],
  );

  // ---------------------------------------------------------- apagar

  // Quadro OrbitManuseio 4 e 5: nó parado some na hora (com as ligações e o
  // aviso de desfazer); rodando, a cápsula pergunta e o core para o nó antes
  // de ele sair. Criado pelo orquestrador: some só da vista do run (o core
  // não tem descartar; o que não é mantido já fica fora do grafo).
  const [asking, setAsking] = useState<string | null>(null);
  const removeNow = useCallback(
    (ids: ReadonlySet<string>) => {
      const fromCreated = [...ids].filter((id) => created.nodeIds.has(id));
      if (fromCreated.length) setDiscarded((d) => new Set([...d, ...fromCreated]));
      const removed = nodesRef.current.filter((n) => ids.has(n.id)).map((n) => n.data.node);
      if (removed.length === 0) return;
      const gone = new Set(removed.map((n) => n.id));
      setNodes((ns) => ns.filter((n) => !gone.has(n.id)));
      setEdges((es) => es.filter((e) => !gone.has(e.source) && !gone.has(e.target)));
      onRemoved(removed);
    },
    [created.nodeIds, setNodes, setEdges, onRemoved],
  );
  const planOf = useCallback(
    (id: string) => {
      const node = (shownRef.current.find((n) => n.id === id)?.data as { node?: OrbitNode })
        ?.node;
      return node ? deletePlan(node.kind, run, id, live) : null;
    },
    [run, live],
  );
  const stopNode = useCallback(
    (id: string) => (run ? runStopNode(run.id, id) : Promise.resolve()),
    [run],
  );
  const nodeDelete = useMemo<NodeDelete>(
    () => ({
      asking,
      request: (id) => {
        const plan = planOf(id);
        if (!plan) return;
        if (plan.ask) return setAsking(id);
        if (plan.stop) void stopNode(id).catch(() => {});
        removeNow(new Set([id]));
      },
      confirm: (id) => {
        setAsking(null);
        stopNode(id)
          .then(() => removeNow(new Set([id])))
          .catch((e) => toastError(String(e)));
      },
      cancel: () => setAsking(null),
    }),
    [asking, planOf, stopNode, removeNow],
  );
  // O nó que perguntava saiu da vista (desfazer, outro run): sem pergunta.
  useEffect(() => {
    if (asking && !shownNodes.some((n) => n.id === asking)) setAsking(null);
  }, [asking, shownNodes]);
  // Enter confirma, Esc cancela — antes do terminal e dos atalhos do canvas.
  const nodeDeleteRef = useRef(nodeDelete);
  nodeDeleteRef.current = nodeDelete;
  useEffect(() => {
    if (!asking) return;
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Enter" && e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Enter") nodeDeleteRef.current.confirm(asking);
      else nodeDeleteRef.current.cancel();
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, [asking]);
  // Delete/Backspace seguem o mesmo caminho: quem roda pergunta (o primeiro
  // deles) e fica; criado sai da vista; o resto o xyflow apaga.
  const onBeforeDelete = useCallback(
    async ({ nodes: ns, edges: es }: { nodes: CanvasNode[]; edges: Edge[] }) => {
      const held = new Set<string>();
      for (const n of ns) {
        const plan = planOf(n.id);
        if (!plan) continue;
        if (plan.ask) {
          held.add(n.id);
          setAsking((a) => a ?? n.id);
        } else if (created.nodeIds.has(n.id)) {
          held.add(n.id);
          if (plan.stop) void stopNode(n.id).catch(() => {});
          setDiscarded((d) => new Set([...d, n.id]));
        } else if (plan.stop) {
          void stopNode(n.id).catch(() => {});
        }
      }
      return {
        nodes: ns.filter((n) => !held.has(n.id)),
        edges: es.filter((e) => !held.has(e.source) && !held.has(e.target)),
      };
    },
    [planOf, stopNode, created.nodeIds],
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

  // Uma porta de saída por nó e o nó inteiro como alvo (canvas-3): o tipo da
  // ligação sai do par (`pickConnection`), não de qual bolinha se agarrou.
  const isValidConnection = useCallback(
    (c: Connection | Edge) => {
      if (c.sourceHandle !== OUT_HANDLE || c.targetHandle !== NEW_IN_HANDLE) return false;
      return "kind" in pickConnection(model.nodes, model.edges, byId, c.source, c.target);
    },
    [model, byId],
  );

  const link = useCallback(
    (source: string, target: string, kind: EdgeKind) =>
      setEdges((es) => [
        ...es,
        {
          id: crypto.randomUUID(),
          type: "orbit",
          source,
          target,
          sourceHandle: OUT_HANDLE,
          targetHandle: NEW_IN_HANDLE,
          data: { kind },
        },
      ]),
    [setEdges],
  );

  const onConnect = useCallback(
    (c: Connection) => {
      const pick = pickConnection(model.nodes, model.edges, byId, c.source, c.target);
      if ("kind" in pick) link(c.source, c.target, pick.kind);
    },
    [model, byId, link],
  );

  // ---------------------------------------------------------- paleta

  const [palette, setPalette] = useState<PaletteAt | null>(null);
  const [snippets, setSnippets] = useState<Snippet[]>([]);
  // Onde o ponteiro está no canvas (⌘K e "/" abrem a paleta ali).
  const pointer = useRef<{ x: number; y: number } | null>(null);

  const openPalette = useCallback(
    (client: { x: number; y: number } | null, from: string | null, near: boolean) => {
      const rect = wrapper.current?.getBoundingClientRect();
      if (!rect) return;
      const at = client ?? { x: rect.left + rect.width / 2, y: rect.top + rect.height / 3 };
      const clamp = (v: number, max: number) => Math.max(8, Math.min(v, max - 8));
      setPalette({
        screen: {
          x: clamp(at.x - rect.left, rect.width - PALETTE_W),
          y: clamp(at.y - rect.top, rect.height - PALETTE_H),
        },
        flow: screenToFlowPosition(at),
        from,
        near,
      });
      void listSnippets(null)
        .then(setSnippets)
        .catch(() => {});
    },
    [screenToFlowPosition],
  );

  const entries = useMemo(
    () =>
      paletteEntries({
        labels: {
          claude: t("graphAgentClaude"),
          codex: t("graphAgentCodex"),
          process: t("orbitKindProcess"),
          terminal: t("orbitKindTerminal"),
          service: t("orbitKindService"),
          browser: t("orbitKindBrowser"),
          note: t("orbitKindNote"),
        },
        dir: newDir,
        integrations,
        snippets,
        dirs: recentDirs(
          lastDir(),
          models.map((m) => m.kind),
        ),
      }),
    [t, newDir, integrations, snippets, models],
  );

  const paletteSource = palette?.from ? model.nodes.find((n) => n.id === palette.from) : undefined;
  const paletteLink = useMemo(
    () => (paletteSource ? { source: paletteSource.kind, integrations: byId } : null),
    [paletteSource, byId],
  );

  /** Todo retângulo à vista: o nó novo não cai em cima de nenhum. */
  const occupied = useCallback(() => shownNodes.map(nodeRect), [shownNodes]);

  const pick = (entry: PaletteEntry) => {
    if (!palette) return;
    const node = newNode(crypto.randomUUID(), entry.title, structuredClone(entry.kind), {
      x: 0,
      y: 0,
    });
    const selected = palette.near ? nodes.find((n) => n.selected) : undefined;
    const want = selected
      ? rightOf(nodeRect(selected), 80)
      : palette.from
        ? { x: palette.flow.x, y: palette.flow.y - 40 }
        : { x: palette.flow.x - node.size.w / 2, y: palette.flow.y - 20 };
    node.position = freeSpot(want, node.size, occupied());
    if (node.kind.type === "terminal") {
      setPalette(null);
      onAddTerminal(node.position);
      return;
    }
    if (entry.group === "folder" && "dir" in entry.kind) rememberDir(entry.kind.dir);
    setNodes((ns) => [...ns.map((n) => ({ ...n, selected: false })), flowNodeOf(node, true)]);
    if (palette.from) {
      const made = pickConnection([...model.nodes, node], model.edges, byId, palette.from, node.id);
      if ("kind" in made) link(palette.from, node.id, made.kind);
    }
    setPalette(null);
  };

  // Soltar a ligação em qualquer ponto do nó liga nele: o xyflow só informa
  // o nó de destino com o ponteiro a até `connectionRadius` da entrada; no
  // corpo do nó o `toNode` vem nulo, e o nó sai do que está sob o ponteiro.
  // Ligação recusada diz por quê, em vez de sumir calada. Soltar no vazio
  // abre a paleta ali, só com quem aceita a origem (canvas-8).
  const onConnectEnd = useCallback(
    (event: MouseEvent | TouchEvent, state: FinalConnectionState) => {
      if (state.isValid || !state.fromNode || state.fromHandle?.id !== OUT_HANDLE) return;
      const point = "changedTouches" in event ? event.changedTouches[0] : event;
      const to =
        state.toNode?.id ??
        (point ? nodeUnderPointer(document.elementFromPoint(point.clientX, point.clientY)) : null);
      const from = state.fromNode.id;
      if (!to) {
        if (point && model.nodes.some((n) => n.id === from)) {
          openPalette({ x: point.clientX, y: point.clientY }, from, false);
        }
        return;
      }
      // Nó que não é do grafo (criado pelo orquestrador, subagente): não liga.
      if (!model.nodes.some((n) => n.id === to)) return;
      const pick = pickConnection(model.nodes, model.edges, byId, from, to);
      if ("problem" in pick) {
        if (pick.problem !== "self") toastError(t(`orbitConnection_${pick.problem}`));
        return;
      }
      link(from, to, pick.kind);
    },
    [model, byId, link, t, openPalette],
  );

  const edgeActions = useMemo<EdgeActions>(
    () => ({
      choices: (edgeId) => edgeKindChoices(model.nodes, model.edges, byId, edgeId),
      retype: (edgeId, kind) =>
        setEdges((es) => es.map((e) => (e.id === edgeId ? { ...e, data: { kind } } : e))),
      remove: (edgeId) => setEdges((es) => es.filter((e) => e.id !== edgeId)),
    }),
    [model, byId, setEdges],
  );

  // ---------------------------------------------------------- atalhos

  // ⌘C guarda aqui (só no canvas: não é texto para a área de transferência).
  const clipboard = useRef<Clip | null>(null);

  /** A cópia ao lado do original, num espaço livre, já selecionada. */
  const paste = (clip: Clip) => {
    const box = clipBounds(clip);
    if (!box) return;
    const at = freeSpot(rightOf(box, 40), { w: box.w, h: box.h }, occupied());
    const pasted = pasteClip(clip, at, () => crypto.randomUUID());
    setNodes((ns) => [
      ...ns.map((n) => ({ ...n, selected: false })),
      ...pasted.nodes.map((n) => flowNodeOf(n, true)),
    ]);
    setEdges((es) => [...es, ...pasted.edges.map(flowEdgeOf)]);
  };

  const selection = () => {
    const ids = new Set(nodes.filter((n) => n.selected).map((n) => n.id));
    return ids.size > 0 ? copyNodes(model.nodes, model.edges, ids) : null;
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.defaultPrevented || palette || focusNode || asking) return;
    const action = canvasKey(e, IS_MAC);
    if (!action) return;
    if (keyOwnedElsewhere(document.activeElement) || keyOwnedElsewhere(e.target as Element)) {
      return;
    }
    // Foco fora da tela do grafo (outra tela aberta por cima): não é do canvas.
    const active = document.activeElement;
    const screen = wrapper.current?.closest("[data-graph-screen]");
    if (active && active !== document.body && !screen?.contains(active)) return;
    // Texto destacado na página (uma nota, um painel): ⌘C é dele.
    const text = window.getSelection();
    if (action === "copy" && text && !text.isCollapsed) return;
    // Enter num botão ou link é dele; sem um nó com terminal, também não é do canvas.
    let enterSession: string | null = null;
    if (action === "focusTerminal") {
      if (active?.closest('button, a, [role="button"]')) return;
      enterSession = enterTerminal(
        shownNodes.filter((n) => n.selected),
        (id) => {
          const node = (shownNodes.find((n) => n.id === id)?.data as { node?: OrbitNode })
            ?.node;
          return node ? shownSession(node.kind, run, runs, id, live) : null;
        },
      );
      if (!enterSession) return;
    }
    e.preventDefault();
    e.stopPropagation();
    if (e.repeat && action !== "undo" && action !== "redo") return;
    switch (action) {
      case "undo":
        return onUndo();
      case "redo":
        return onRedo();
      case "duplicate": {
        const clip = selection();
        if (clip) paste(clip);
        return;
      }
      case "copy":
        clipboard.current = selection() ?? clipboard.current;
        return;
      case "paste":
        if (clipboard.current) paste(clipboard.current);
        return;
      case "selectAll":
        setNodes((ns) => ns.map((n) => ({ ...n, selected: true })));
        return;
      case "zoomActual":
        void zoomTo(1, { duration: 150 });
        return;
      case "fit":
        void fitView({ padding: 0.15, maxZoom: 1, duration: 200 });
        return;
      case "palette":
        openPalette(pointer.current, null, true);
        return;
      case "focusTerminal":
        // A linha do TYBA quando ela é dona do teclado (nó Terminal), senão o xterm.
        if (enterSession) {
          if (paneHost) paneHost.focusInput(enterSession);
          else getTerm(enterSession)?.term.focus();
        }
        return;
    }
  };
  const onKeyRef = useRef(onKey);
  onKeyRef.current = onKey;
  useEffect(() => {
    const handler = (e: KeyboardEvent) => onKeyRef.current(e);
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  // ---------------------------------------------------------- minimapa

  // Liga sozinho quando algum nó sai da área visível (canvas-10).
  const transform = useStore((s) => s.transform);
  const paneW = useStore((s) => s.width);
  const paneH = useStore((s) => s.height);
  const overflow = useMemo(
    () =>
      outsideView(
        shownNodes.map(nodeRect),
        { x: transform[0], y: transform[1], zoom: transform[2] },
        { w: paneW, h: paneH },
      ),
    [shownNodes, transform, paneW, paneH],
  );
  useEffect(() => onOverflow(overflow), [overflow, onOverflow]);
  // O toast do pedido só sai quando a cápsula do nó está à vista e o canvas
  // em primeiro plano (o modo foco cobre o canvas inteiro).
  const cardIdsKey = useMemo(
    () =>
      [
        ...visibleCardApprovalIds({
          cards: shownNodes.map((n) => ({ id: n.id, rect: nodeRect(n) })),
          approvals,
          view: { x: transform[0], y: transform[1], zoom: transform[2] },
          pane: { w: paneW, h: paneH },
          foreground: foreground && !focusNode,
        }),
      ]
        .sort((a, b) => a - b)
        .join(","),
    [shownNodes, approvals, transform, paneW, paneH, foreground, focusNode],
  );
  useEffect(() => {
    onCardApprovals(new Set(cardIdsKey ? cardIdsKey.split(",").map(Number) : []));
  }, [cardIdsKey, onCardApprovals]);
  useEffect(() => () => onCardApprovals(new Set()), [onCardApprovals]);
  const showMinimap = minimap ?? overflow;
  const nodeColor = useCallback(
    (n: CanvasNode) =>
      isSubagentNodeId(n.id)
        ? "color-mix(in srgb, var(--tyba-violet) 50%, transparent)"
        : minimapTone(
            ringState(run?.nodes[n.id], approvals.has(n.id), run?.settled),
            createdIds.has(n.id),
          ),
    [run, approvals, createdIds],
  );

  return (
    <div
      ref={wrapper}
      // Fundo do canvas num preto mais claro que o do app, para o card (preto
      // puro) se destacar; segue o tema por mistura com o texto.
      className="relative min-h-0 flex-1 bg-[var(--orbit-canvas)] [--orbit-canvas:color-mix(in_srgb,var(--tyba-text)_5%,var(--tyba-bg))]"
      onMouseMove={(e) => {
        pointer.current = { x: e.clientX, y: e.clientY };
      }}
      onMouseLeave={() => {
        pointer.current = null;
      }}
      onDoubleClick={(e) => {
        // Só no vazio do canvas: duplo clique no nó, na ligação ou num painel é deles.
        if (!(e.target as Element).classList?.contains("react-flow__pane")) return;
        openPalette({ x: e.clientX, y: e.clientY }, null, false);
      }}
    >
      <NodeDeleteContext.Provider value={nodeDelete}>
        <EdgeActionsContext.Provider value={edgeActions}>
          <ReactFlow<CanvasNode, Edge>
            nodes={shownNodes}
            edges={shownEdges}
            nodeTypes={nodeTypes}
            edgeTypes={edgeTypes}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            onConnectEnd={onConnectEnd}
            onBeforeDelete={onBeforeDelete}
            isValidConnection={isValidConnection}
            connectionRadius={36}
            deleteKeyCode={["Backspace", "Delete"]}
            // Dois dedos movem o canvas; ⌘ + roda (ou pinça) dá zoom (canvas-9).
            panOnScroll
            zoomOnPinch
            zoomActivationKeyCode={IS_MAC ? "Meta" : "Control"}
            zoomOnDoubleClick={false}
            minZoom={0.2}
            maxZoom={2}
            fitView
            fitViewOptions={{ padding: 0.15, maxZoom: 1 }}
            proOptions={{ hideAttribution: true }}
            connectionLineStyle={{ stroke: "var(--tyba-violet)", strokeWidth: 1.6 }}
            aria-label={t("orbitCanvas")}
          >
            {guides.length > 0 && (
              <ViewportPortal>
                {guides.map((g) => (
                  <div
                    key={`${g.axis}:${g.at}:${g.from}`}
                    aria-hidden
                    className="pointer-events-none absolute bg-tyba-violet/70"
                    style={{
                      zIndex: 1000,
                      ...(g.axis === "x"
                        ? { left: g.at, top: g.from, width: 1 / zoom, height: g.to - g.from }
                        : { top: g.at, left: g.from, height: 1 / zoom, width: g.to - g.from }),
                    }}
                  />
                ))}
              </ViewportPortal>
            )}
            <Background
              variant={BackgroundVariant.Dots}
              gap={24}
              size={1.3}
              color="var(--tyba-border-strong)"
            />
            {showMinimap && (
              <MiniMap<CanvasNode>
                pannable
                zoomable
                bgColor="var(--tyba-surface)"
                maskColor="color-mix(in srgb, var(--tyba-bg) 70%, transparent)"
                nodeColor={nodeColor}
                onNodeClick={(_, n) => {
                  const r = nodeRect(n);
                  void setCenter(r.x + r.w / 2, r.y + r.h / 2, { duration: 200, zoom: 1 });
                }}
              />
            )}
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
                <p className="font-mono text-[12.5px] text-tyba-text-faint">
                  {t("orbitEmpty", { mod: modKey })}
                </p>
              </Panel>
            )}
            <Panel position="bottom-left">
              <p className="font-mono text-[11.5px] text-tyba-text-faint">
                {t("graphHint", { mod: modKey })}
              </p>
            </Panel>
          </ReactFlow>
        </EdgeActionsContext.Provider>
      </NodeDeleteContext.Provider>
      {palette && (
        <NodePalette
          at={palette.screen}
          entries={entries}
          link={paletteLink}
          linkLine={
            paletteSource
              ? t("graphPaletteAccepts", {
                  kinds: produces(paletteSource.kind, byId)
                    .map((k) => t(`orbitEdge_${k}`))
                    .join(", "),
                  node: paletteSource.title,
                })
              : null
          }
          onPick={pick}
          onClose={() => setPalette(null)}
        />
      )}
      {focused && (
        <NodeDeleteContext.Provider value={nodeDelete}>
          <NodeFocus node={focused.node} sessionId={focused.session} />
        </NodeDeleteContext.Provider>
      )}
    </div>
  );
}

/** A ligação do canvas para uma ligação do modelo. */
const flowEdgeOf = (e: OrbitEdge): OrbitFlowEdge => ({
  id: e.id,
  type: "orbit",
  source: e.from,
  target: e.to,
  sourceHandle: OUT_HANDLE,
  targetHandle: NEW_IN_HANDLE,
  data: { kind: e.kind },
});
