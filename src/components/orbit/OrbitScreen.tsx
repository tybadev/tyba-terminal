import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type DragEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { homeDir } from "@tauri-apps/api/path";
import {
  BracketsCurly,
  CaretDown,
  Check,
  ClockCounterClockwise,
  CornersOut,
  FileArrowDown,
  FileArrowUp,
  GearSix,
  MagnifyingGlassMinus,
  MagnifyingGlassPlus,
  MapTrifold,
  PencilSimple,
  Play,
  PlugsConnected,
  Plus,
  Stop,
  Warning,
  X,
} from "@phosphor-icons/react";
import { ReactFlowProvider, useReactFlow, useViewport } from "@xyflow/react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  EMPTY_RUN_VIEW,
  INTEGRATION_DRAG_TYPE,
  agentSessions as agentSessionsOf,
  browserFed,
  findSessionNode,
  integrationsById,
  invalidEdges,
  createdByOf,
  ephemeralCreated,
  keepCreated,
  NEW_IN_HANDLE,
  OUT_HANDLE,
  isLocalUrl,
  serviceContainers,
  serviceLogsRetain,
  emptyGraph,
  exportFileName,
  exportGraph,
  formatDuration,
  fromFlow,
  importPreview,
  integrationTest,
  joinPath,
  lastDir,
  lastGraphId,
  listGraphs,
  listIntegrations,
  newNode,
  onRun,
  pickGraph,
  rememberGraph,
  runGet,
  runNode,
  runNumber,
  runReducer,
  runStart,
  runStop,
  runsList,
  saveGraph,
  selectedRun,
  stoppableRuns,
  toFlow,
  type Graph,
  type ImportPreview,
  type Integration,
  type OrbitNode,
} from "@/lib/orbit";
import { ImportDialog } from "./ImportDialog";
import { IntegrationsPanel } from "./IntegrationsPanel";
import { OrbitCanvas, type CreatedView, type OrbitFlowEdge } from "./OrbitCanvas";
import { RunTimeline } from "./RunTimeline";
import { CanvasProvider, STATUS_TONE, type OrbitFlowNode } from "./nodes/NodeFrame";
import { SubagentMark, subagentTone, useSubagentTime } from "./nodes/SubagentNode";
import { useNodeSubagents } from "./useNodeSubagents";
import { SubagentTranscript } from "@/components/SubagentViewer";
import {
  resolveApproval,
  type ApprovalDecision,
  type ApprovalRequest,
  type Session,
  type SubagentRun,
} from "@/lib/ipc";
import { nodeApproval, type AgentTelemetry } from "@/lib/orbitCard";
import { keepIfSame } from "@/lib/stableItems";
import {
  EMPTY_HISTORY,
  graphShot,
  keepInHistory,
  record,
  redo as redoShot,
  restoreShot,
  undo as undoShot,
} from "@/lib/graphHistory";
import { freeSpot, nodeRect, rightOf } from "@/lib/graphLayout";
import { applyPlans, freshPlans } from "@/lib/orbitPlan";
import { IS_MAC } from "@/lib/platform";
import {
  subagentKey,
  subagentNodeId,
  subagentState,
} from "@/lib/orbitSubagents";

const SAVE_DEBOUNCE_MS = 500;
/** Quanto tempo o canvas fica parado para a edição virar um passo do ⌘Z. */
const HISTORY_SETTLE_MS = 400;
/** Quanto o aviso "Nó apagado · Desfazer" fica à vista. */
const UNDO_NOTICE_MS = 8000;

const tbtn =
  "inline-flex h-7 items-center gap-1.5 whitespace-nowrap rounded-[6px] border border-transparent px-2.5 font-mono text-[12px] text-tyba-text-muted hover:bg-tyba-raised hover:text-tyba-text disabled:cursor-not-allowed disabled:text-tyba-text-ghost disabled:hover:bg-transparent aria-pressed:border-tyba-border-strong aria-pressed:bg-tyba-raised aria-pressed:text-tyba-text";

const sep = <span aria-hidden className="mx-2 h-5 w-px bg-tyba-border-strong" />;

/**
 * O Orbit em tela cheia (quadro OrbitHarness): barra do grafo, canvas e,
 * quando aberto, o painel de variáveis. O grafo não é preso à pasta da
 * sessão: escolhe-se pelo nome na barra, e cada nó tem a própria pasta.
 */
export function OrbitScreen({
  focusSession = null,
  approvals = [],
  onCardApprovals,
  covered = false,
  sessions = [],
}: {
  focusSession?: FocusSessionRequest | null;
  /** As sessões do core (as dos nós trazem modelo, contexto e diff do agente). */
  sessions?: readonly Session[];
  /** Outra tela do app por cima do Orbit (Serviços e portas). */
  covered?: boolean;
  /** Pedidos de aprovação pendentes (do core, via App). */
  approvals?: readonly ApprovalRequest[];
  /** Os ids que as cápsulas dos nós mostram agora: saem do toast. */
  onCardApprovals?: (ids: ReadonlySet<number>) => void;
}) {
  return (
    <ReactFlowProvider>
      <OrbitInner
        focusSession={focusSession}
        approvals={approvals}
        onCardApprovals={onCardApprovals}
        covered={covered}
        sessions={sessions}
      />
    </ReactFlowProvider>
  );
}

/**
 * Pedido de fora (Serviços e portas, no Workspace) para ir ao nó de uma
 * sessão: o `nonce` repete o pedido para a mesma sessão.
 */
export interface FocusSessionRequest {
  sessionId: string;
  nonce: number;
}

function OrbitInner({
  focusSession,
  approvals,
  onCardApprovals,
  covered,
  sessions,
}: {
  focusSession: FocusSessionRequest | null;
  approvals: readonly ApprovalRequest[];
  onCardApprovals?: (ids: ReadonlySet<number>) => void;
  covered: boolean;
  sessions: readonly Session[];
}) {
  const { t, i18n } = useTranslation();
  // Home: onde nasce o nó quando ainda não há pasta usada. `null` = lendo.
  const [home, setHome] = useState<string | null>(null);
  useEffect(() => {
    homeDir()
      .then(setHome)
      .catch(() => setHome(""));
  }, []);
  // Qual grafo abrir: o último usado (ou o mais recente), ou um novo.
  const [want, setWant] = useState<{ id: string | null; fresh: boolean }>(() => ({
    id: lastGraphId(),
    fresh: false,
  }));
  const [graphs, setGraphs] = useState<Graph[]>([]);
  const nameInput = useRef<HTMLInputElement>(null);
  const renaming = useRef(false);

  const [meta, setMeta] = useState<Pick<Graph, "id" | "name" | "dir" | "variables"> | null>(null);
  const [nodes, setNodes] = useState<OrbitFlowNode[]>([]);
  // O modelo de cada nó, sem a posição do xyflow: arrastar troca o objeto do
  // nó a cada quadro, mas não o modelo, e esta lista fica a mesma (canvas-7).
  const nodeModelsRef = useRef<readonly OrbitNode[] | null>(null);
  const nodeModels = keepIfSame(
    nodeModelsRef.current,
    nodes.map((n) => n.data.node),
  );
  nodeModelsRef.current = nodeModels;
  const nodesRef = useRef(nodes);
  nodesRef.current = nodes;
  const [edges, setEdges] = useState<OrbitFlowEdge[]>([]);
  const edgesRef = useRef(edges);
  edgesRef.current = edges;
  const [integrations, setIntegrations] = useState<Integration[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [showVars, setShowVars] = useState(false);
  // Minimapa: `null` = sozinho (liga quando o grafo passa da tela, canvas-10).
  const [minimapPref, setMinimapPref] = useState<boolean | null>(null);
  const [overflow, setOverflow] = useState(false);
  const showMinimap = minimapPref ?? overflow;
  const lastSaved = useRef<string | null>(null);
  // Quando a tela leu o grafo do core, e o plano de cada nota já posto nela
  // (ver `freshPlans`).
  const loadedAt = useRef(0);
  const appliedPlans = useRef(new Map<string, number>());
  const [runView, dispatchRun] = useReducer(runReducer, EMPTY_RUN_VIEW);
  const [now, setNow] = useState(() => Date.now());
  const [showInts, setShowInts] = useState(false);
  const [notice, setNotice] = useState<{ text: string; warn: boolean; undo?: boolean } | null>(
    null,
  );
  const [importing, setImporting] = useState<{
    path: string;
    preview: ImportPreview;
  } | null>(null);
  const [reload, setReload] = useState(0);

  // Carrega o grafo escolhido (ou começa um vazio, gravado na primeira edição).
  useEffect(() => {
    if (home === null) return;
    let alive = true;
    setMeta(null);
    dispatchRun({ type: "reset" });
    void Promise.all([listGraphs(), listIntegrations()])
      .then(([list, ints]) => {
        if (!alive) return;
        setGraphs(list);
        const found = want.fresh ? null : pickGraph(list, want.id);
        const graph =
          found ?? emptyGraph(crypto.randomUUID(), t("orbitNewGraphName"), lastDir() ?? home);
        rememberGraph(graph.id);
        const flow = toFlow(graph);
        // Grafo novo e vazio conta como "salvo": só vai para o banco na
        // primeira edição, para não deixar grafo vazio de quem só espiou.
        lastSaved.current = JSON.stringify(fromFlow(graph, flow.nodes, flow.edges));
        loadedAt.current = Date.now();
        appliedPlans.current = new Map();
        setIntegrations(ints);
        setNodes(flow.nodes as OrbitFlowNode[]);
        setEdges(flow.edges);
        // Outro grafo: o ⌘Z começa do zero (o retrato dele entra no próximo assentar).
        historyRef.current = EMPTY_HISTORY;
        settledRef.current = null;
        setMeta({ id: graph.id, name: graph.name, dir: graph.dir, variables: graph.variables });
        setError(null);
        if (found) {
          void runsList(graph.id)
            .then((runs) => alive && dispatchRun({ type: "history", runs }))
            .catch(() => {});
        }
      })
      .catch((e) => alive && setError(t("orbitLoadError", { error: String(e) })));
    return () => {
      alive = false;
    };
  }, [home, want, t, reload]);

  // Desfazer/refazer (canvas-5): o canvas parado por um instante vira um
  // passo (arrasto e redimensionamento contam no fim; apagar, ligar e
  // adicionar na hora). O retrato é o grafo inteiro; voltar troca o canvas
  // por ele e o salvar com debounce leva ao core como qualquer edição.
  const historyRef = useRef(EMPTY_HISTORY);
  const settledRef = useRef<string | null>(null);
  const settle = useCallback(() => {
    const ns = nodesRef.current;
    if (ns.some((n) => n.dragging || n.resizing)) return;
    const now = graphShot(ns, edgesRef.current);
    const prev = settledRef.current;
    if (prev !== null && prev !== now) historyRef.current = record(historyRef.current, prev);
    settledRef.current = now;
  }, []);
  useEffect(() => {
    if (!meta) return;
    const timer = setTimeout(settle, HISTORY_SETTLE_MS);
    return () => clearTimeout(timer);
  }, [nodes, edges, meta, settle]);
  const applyShot = (shot: string) => {
    const flow = restoreShot(shot);
    settledRef.current = shot;
    setNodes(flow.nodes as OrbitFlowNode[]);
    setEdges(flow.edges);
  };
  const undo = () => {
    settle();
    const back = settledRef.current && undoShot(historyRef.current, settledRef.current);
    if (!back) return;
    historyRef.current = back.history;
    applyShot(back.shot);
    setNotice((n) => (n?.undo ? null : n));
  };
  const redo = () => {
    settle();
    const again = settledRef.current && redoShot(historyRef.current, settledRef.current);
    if (!again) return;
    historyRef.current = again.history;
    applyShot(again.shot);
  };
  const onRemoved = useCallback(
    (count: number) =>
      setNotice({
        text: count === 1 ? t("graphNodeDeleted") : t("graphNodesDeleted", { count }),
        warn: false,
        undo: true,
      }),
    [t],
  );
  const undoNotice = notice?.undo ? notice : null;
  useEffect(() => {
    if (!undoNotice) return;
    const timer = setTimeout(() => setNotice((n) => (n === undoNotice ? null : n)), UNDO_NOTICE_MS);
    return () => clearTimeout(timer);
  }, [undoNotice]);

  const snapshot = useMemo(
    () => (meta ? JSON.stringify(fromFlow(meta, nodes, edges)) : null),
    [meta, nodes, edges],
  );

  // Grava com debounce; o core valida e o erro aparece no canvas.
  useEffect(() => {
    if (!snapshot || snapshot === lastSaved.current) return;
    const timer = setTimeout(() => {
      saveGraph(JSON.parse(snapshot) as Graph)
        .then(() => {
          lastSaved.current = snapshot;
          setError(null);
        })
        .catch((e) => setError(t("orbitSaveError", { error: String(e) })));
    }, SAVE_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [snapshot, t]);

  // Escuta cada run rodando ou com agente vivo (o run fecha antes do agente,
  // e a saída dele chega como retrato novo com `liveAgents` sem a sessão);
  // depois de assinar, relê o run para não perder o que mudou antes (o core
  // já emite na largada). O reducer descarta retrato velho, então a ordem
  // entre evento e leitura não importa.
  const runningIds = stoppableRuns(runView)
    .map((r) => r.id)
    .join(",");
  useEffect(() => {
    if (!runningIds) return;
    let alive = true;
    const unlisten: (() => void)[] = [];
    for (const id of runningIds.split(",")) {
      void onRun(id, (run) => alive && dispatchRun({ type: "update", run }))
        .then((un) => {
          if (!alive) return un();
          unlisten.push(un);
          return runGet(id).then((run) => {
            if (alive && run) dispatchRun({ type: "update", run });
          });
        })
        .catch(() => {});
    }
    return () => {
      alive = false;
      unlisten.forEach((un) => un());
    };
  }, [runningIds]);

  const run = selectedRun(runView);

  // A nota de plano que o Orquestrador escreve (`orbit_plan`) vai para o
  // grafo salvo pelo core; a tela põe a nota e a ligação na cópia dela, senão
  // o salvar seguinte apagaria a nota ou voltaria o texto velho.
  const plans = useMemo(
    () =>
      meta
        ? freshPlans(
            runView.runs.flatMap((r) => (r.graphId === meta.id ? (r.plans ?? []) : [])),
            appliedPlans.current,
            loadedAt.current,
          )
        : [],
    [runView.runs, meta],
  );
  useEffect(() => {
    if (plans.length === 0) return;
    for (const p of plans) appliedPlans.current.set(p.note.id, p.updatedAt);
    const next = applyPlans(nodesRef.current, edgesRef.current, plans);
    setNodes(next.nodes);
    setEdges(next.edges);
  }, [plans]);
  // Onde o dono deixou cada nó criado pelo orquestrador (vale enquanto esse
  // run está à vista, e o "Manter no grafo" leva a posição junto).
  const [createdView, setCreatedView] = useState<CreatedView>({});
  const viewRunId = run?.id ?? null;
  useEffect(() => setCreatedView({}), [viewRunId]);
  const latest = runView.runs[0] ?? null;
  const running = runView.runs.find((r) => r.status === "running") ?? null;
  const live = run !== null && run.id === latest?.id;

  // Subagentes de cada nó de agente com terminal vivo no run à vista, os do
  // grafo e os que o orquestrador criou.
  const agentSessions = useMemo(
    () => agentSessionsOf(nodeModels, run, live),
    [nodeModels, run, live],
  );
  const subagents = useNodeSubagents(agentSessions);
  // Pedido de aprovação de cada nó de agente com sessão viva: a cápsula fica
  // âmbar com Permitir/Negar. Os que ela mostra saem do toast do canto.
  const nodeApprovals = useMemo(() => {
    const out = new Map<string, ApprovalRequest>();
    for (const [nodeId, sessionId] of agentSessions) {
      const request = nodeApproval(sessionId, approvals);
      if (request) out.set(nodeId, request);
    }
    return out;
  }, [agentSessions, approvals]);
  // Modelo, contexto e diff que a sessão de cada agente reportou (a grade do
  // ⋯). A chave segura o mapa: a lista de sessões muda a toda hora.
  const telemetryKey = JSON.stringify(
    [...agentSessions].map(([nodeId, sessionId]) => {
      const s = sessions.find((x) => x.id === sessionId);
      const u = s?.agent_usage;
      return [
        nodeId,
        u?.model ?? s?.agent_model ?? null,
        u?.context_pct ?? null,
        u?.lines_added ?? null,
        u?.lines_removed ?? null,
      ];
    }),
  );
  const telemetry = useMemo(
    () =>
      new Map<string, AgentTelemetry>(
        (JSON.parse(telemetryKey) as [string, string | null, number | null, number | null, number | null][]).map(
          ([nodeId, model, contextPct, linesAdded, linesRemoved]) => [
            nodeId,
            { model, contextPct, linesAdded, linesRemoved },
          ],
        ),
      ),
    [telemetryKey],
  );
  const cardApprovals = useCallback(
    (ids: ReadonlySet<number>) => onCardApprovals?.(ids),
    [onCardApprovals],
  );
  const [openSub, setOpenSub] = useState<string | null>(null);
  const [focusNode, setFocusNode] = useState<string | null>(null);
  const opened = useMemo(() => {
    if (!openSub) return null;
    for (const [nodeId, runs] of subagents) {
      const found = runs.find((r) => subagentNodeId(nodeId, subagentKey(r)) === openSub);
      if (found) return { run: found, sessionId: agentSessions.get(nodeId)! };
    }
    return null;
  }, [openSub, subagents, agentSessions]);
  const subagentsActive = [...subagents.values()].some((runs) =>
    runs.some((r) => subagentState(r) === "running"),
  );

  // Relógio dos tempos de nó, de execução e de subagente, só enquanto algo roda.
  const runningId = running?.id ?? null;
  // O "pede há 40s" da cápsula âmbar também anda (o run pode já ter fechado
  // com o agente vivo pedindo).
  const ticking = runningId !== null || subagentsActive || nodeApprovals.size > 0;
  useEffect(() => {
    if (!ticking) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [ticking, runningId]);

  /** O core lê o grafo do banco: o que ainda está no debounce vai antes. */
  const flush = async (always = false) => {
    if (!snapshot) return;
    if (always || snapshot !== lastSaved.current) {
      await saveGraph(JSON.parse(snapshot) as Graph);
      lastSaved.current = snapshot;
    }
  };

  const start = async () => {
    if (!snapshot || !meta) return;
    try {
      await flush();
      const id = await runStart(meta.id);
      const started = await runGet(id);
      if (started) dispatchRun({ type: "update", run: started });
      setError(null);
    } catch (e) {
      setError(t("orbitRunError", { error: String(e) }));
    }
  };

  // Parar alcança também o run já fechado com agente vivo; depois relê o run
  // para a tela saber que os agentes morreram.
  const stoppable = stoppableRuns(runView);
  const stop = () => {
    for (const r of stoppable) {
      runStop(r.id)
        .then(() => runGet(r.id))
        .then((run) => {
          if (run) dispatchRun({ type: "update", run });
        })
        .catch((e) => setError(t("orbitRunError", { error: String(e) })));
    }
  };

  // Lido por ref: a ação fica a mesma enquanto a execução muda.
  const runIdRef = useRef<string | null>(null);
  runIdRef.current = run?.id ?? null;
  const releaseNode = useCallback(
    (nodeId: string) => {
      const runId = runIdRef.current;
      if (!runId) return;
      runNode(runId, nodeId).catch((e) => setError(t("orbitRunError", { error: String(e) })));
    },
    [t],
  );

  const testIntegration = async (integrationId: string, nodeId: string) => {
    if (!meta) return;
    await flush();
    dispatchRun({ type: "update", run: await integrationTest(integrationId, meta.id, nodeId) });
  };

  const integrationSaved = (saved: Integration) =>
    setIntegrations((list) =>
      list.some((i) => i.id === saved.id)
        ? list.map((i) => (i.id === saved.id ? saved : i))
        : [...list, saved],
    );

  const exportFile = async () => {
    if (!meta) return;
    try {
      const folder = await openFileDialog({ directory: true, title: t("orbitExportPick") });
      if (typeof folder !== "string") return;
      // Grafo novo e vazio ainda não foi para o banco: o export lê de lá.
      await flush(true);
      const path = joinPath(folder, exportFileName(meta.name));
      const { cleared, secretFields } = await exportGraph(meta.id, path);
      const warnings = [
        cleared.length > 0 && t("orbitExportedCleared", { names: cleared.join(", ") }),
        secretFields.length > 0 &&
          t("orbitExportedSecrets", {
            fields: secretFields.map((f) => `${f.name} (${f.field})`).join(", "),
          }),
      ].filter(Boolean);
      setNotice({ text: [t("orbitExported", { path }), ...warnings].join(" "), warn: warnings.length > 0 });
    } catch (e) {
      setNotice({ text: t("orbitExportError", { error: String(e) }), warn: true });
    }
  };

  const importFile = async () => {
    try {
      const path = await openFileDialog({
        multiple: false,
        directory: false,
        title: t("orbitImportPick"),
        filters: [{ name: "TYBA Orbit", extensions: ["toml"] }],
      });
      if (typeof path !== "string") return;
      setImporting({ path, preview: await importPreview(path) });
    } catch (e) {
      setNotice({ text: t("orbitImportError", { error: String(e) }), warn: true });
    }
  };

  // Depois do Confirmar, o grafo importado é o que fica aberto.
  const imported = (graphId: string) => {
    setImporting(null);
    setWant({ id: graphId, fresh: false });
    setReload((n) => n + 1);
    setNotice({ text: t("orbitImported"), warn: false });
  };

  const dropIntegration = (e: DragEvent) => {
    const integrationId = e.dataTransfer.getData(INTEGRATION_DRAG_TYPE);
    const integration = integrations.find((i) => i.id === integrationId);
    // Soltar em cima da própria gaveta não cria nó.
    if (!integration || (e.target as Element).closest("[data-orbit-sheet]")) return;
    e.preventDefault();
    const at = screenToFlowPosition({ x: e.clientX, y: e.clientY });
    const node = newNode(
      crypto.randomUUID(),
      integration.name,
      { type: "integration", integrationId, dir: newDir },
      { x: 0, y: 0 },
    );
    node.position = { x: Math.round(at.x - node.size.w / 2), y: Math.round(at.y - 20) };
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

  const updateNode = useCallback(
    (id: string, update: (node: OrbitNode) => OrbitNode) =>
      setNodes((ns) =>
        ns.map((n) => (n.id === id ? { ...n, data: { node: update(n.data.node) } } : n)),
      ),
    [],
  );
  const { zoomIn, zoomOut, zoomTo, fitView, screenToFlowPosition } = useReactFlow();
  const { zoom } = useViewport();
  const mod = IS_MAC ? "⌘" : "Ctrl+";

  const graph = useMemo(
    () => ({
      nodes: nodeModels as OrbitNode[],
      edges: edges.flatMap((e) =>
        e.data ? [{ id: e.id, from: e.source, to: e.target, kind: e.data.kind }] : [],
      ),
    }),
    [nodeModels, edges],
  );
  // O que o canvas mostra: o grafo e, no run rodando, os nós que o
  // orquestrador criou (efêmeros) com as ligações url deles.
  const createdNow = useMemo(() => ephemeralCreated(run), [run]);
  const createdIds = useMemo(() => new Set(createdNow.map((c) => c.node.id)), [createdNow]);
  const createdBy = useMemo(() => createdByOf(run), [run]);
  const viewGraph = useMemo(
    () =>
      createdNow.length === 0
        ? graph
        : {
            nodes: [...graph.nodes, ...createdNow.map((c) => c.node)],
            edges: [...graph.edges, ...createdNow.flatMap((c) => c.edges)],
          },
    [graph, createdNow],
  );

  // Ligação que deixou de valer (saída do processo trocada, integração salva
  // com outro output ou gatilho) sai na hora, com aviso: a porta dela some,
  // o xyflow não a desenha, e o core recusaria todo salvar seguinte por ela.
  // Roda antes do debounce do salvar, então o grafo inválido nunca vai.
  useEffect(() => {
    const bad = invalidEdges(graph.nodes, graph.edges, integrationsById(integrations));
    if (bad.length === 0) return;
    const ids = new Set(bad.map((e) => e.id));
    setEdges((es) => es.filter((e) => !ids.has(e.id)));
    const title = (id: string) => graph.nodes.find((n) => n.id === id)?.title || id;
    setNotice({
      text: t("orbitEdgesRemoved", {
        edges: bad
          .map((e) => `${title(e.from)} → ${title(e.to)} (${t(`orbitEdge_${e.kind}`)})`)
          .join(", "),
      }),
      warn: true,
    });
  }, [graph, integrations, t]);

  // Log ao vivo dos serviços: só o seguidor de contêiner que um nó mostra fica.
  const containers = serviceContainers(graph.nodes).join(",");
  useEffect(() => {
    void serviceLogsRetain(containers ? containers.split(",") : []).catch(() => {});
  }, [containers]);
  useEffect(() => () => void serviceLogsRetain([]).catch(() => {}), []);

  // "Abrir no navegador" do processo: o navegador que já recebe a url dele, ou
  // um novo ao lado, ligado pela url e já com o endereço.
  const viewGraphRef = useRef(viewGraph);
  viewGraphRef.current = viewGraph;
  const openInBrowser = useCallback(
    (fromId: string, url: string) => {
      const fed = browserFed(viewGraphRef.current, fromId);
      if (fed) {
        setNodes((ns) => ns.map((n) => ({ ...n, selected: n.id === fed.id })));
        void fitView({ nodes: [{ id: fed.id }], padding: 0.2, maxZoom: 1, duration: 200 });
        return;
      }
      const from = nodesRef.current.find((n) => n.id === fromId);
      if (!from) return;
      const node = newNode(
        crypto.randomUUID(),
        t("orbitKindBrowser"),
        { type: "browser", url: isLocalUrl(url) ? url : "" },
        { x: 0, y: 0 },
      );
      // Ao lado do processo, num espaço livre (canvas-6).
      node.position = freeSpot(rightOf(nodeRect(from)), node.size, nodesRef.current.map(nodeRect));
      const edgeId = crypto.randomUUID();
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
      setEdges((es) => [
        ...es,
        {
          id: edgeId,
          type: "orbit",
          source: fromId,
          target: node.id,
          sourceHandle: OUT_HANDLE,
          targetHandle: NEW_IN_HANDLE,
          data: { kind: "url" },
        },
      ]);
    },
    [t, fitView],
  );

  const graphDir = meta?.dir ?? "";
  // Nó novo nasce na última pasta usada; sem ela, na home.
  const newDir = lastDir() ?? (home || graphDir);
  // Estável (ações) × reativo (execução à vista) × relógio: arrastar não
  // muda nenhum dos três, e o tique de 1 s só chega a quem mostra tempo.
  const actions = useMemo(
    () => ({
      integrations: new Map(integrations.map((i) => [i.id, i])),
      updateNode,
      graphDir,
      runNode: releaseNode,
      setOpenSubagent: setOpenSub,
      setFocusNode,
      openInBrowser,
      keepCreated: (nodeId: string | null) => void keepRef.current(nodeId),
      answerApproval: (id: number, decision: ApprovalDecision) =>
        void resolveApproval(id, decision).catch((e) =>
          setError(t("orbitApprovalError", { error: String(e) })),
        ),
    }),
    [integrations, updateNode, graphDir, releaseNode, openInBrowser, t],
  );
  const ctx = useMemo(
    () => ({
      graph: viewGraph,
      run,
      live,
      subagents,
      openSubagent: opened ? openSub : null,
      focusNode,
      createdIds,
      createdBy,
      approvals: nodeApprovals,
      telemetry,
    }),
    [
      viewGraph,
      run,
      live,
      subagents,
      opened,
      openSub,
      focusNode,
      createdIds,
      createdBy,
      nodeApprovals,
      telemetry,
    ],
  );

  // "Manter" (o criado `nodeId`, ou todos no "Manter tudo"): o core grava no
  // grafo e devolve o grafo inteiro, que passa a ser o da tela (o que estava
  // no debounce vai antes).
  const keep = async (nodeId: string | null) => {
    if (!run) return;
    try {
      await flush();
      const kept = await keepCreated(run.id, nodeId);
      const flow = toFlow(kept);
      lastSaved.current = JSON.stringify(fromFlow(kept, flow.nodes, flow.edges));
      // Onde o dono arrastou/redimensionou o criado vale no grafo: a posição
      // da vista entra no nó mantido, e o salvar leva ao core.
      const view = createdViewRef.current;
      const keptNodes = flow.nodes.map((n) => {
        const v = view[n.id];
        return v
          ? {
              ...n,
              position: v.position,
              width: v.width ?? n.width,
              height: v.height ?? n.height,
            }
          : n;
      }) as OrbitFlowNode[];
      // O Manter não é passo do ⌘Z (ver `keepInHistory`): o que estava
      // pendente assenta antes, e os mantidos entram em todo retrato da pilha.
      settle();
      const before = new Set(nodesRef.current.map((n) => n.id));
      const shot = graphShot(keptNodes, flow.edges);
      historyRef.current = keepInHistory(
        historyRef.current,
        shot,
        new Set(keptNodes.filter((n) => !before.has(n.id)).map((n) => n.id)),
      );
      settledRef.current = shot;
      setNodes(keptNodes);
      setEdges(flow.edges);
      const fresh = await runGet(run.id);
      if (fresh) dispatchRun({ type: "update", run: fresh });
    } catch (e) {
      setError(t("orbitKeepError", { error: String(e) }));
    }
  };
  const createdViewRef = useRef(createdView);
  createdViewRef.current = createdView;
  // O rodapé do criado e o ⋯ do orquestrador chamam o "Manter" pela ação
  // estável do contexto.
  const keepRef = useRef(keep);
  keepRef.current = keep;

  /** Troca de grafo: o que está no debounce vai antes para o banco. */
  const switchTo = (next: { id: string | null; fresh: boolean }) => {
    void flush()
      .catch(() => {})
      .finally(() => setWant(next));
  };

  // "Ir para o terminal" de uma sessão de nó (Serviços e portas, no
  // Workspace): a sessão de nó não está em layout nenhum, então o caminho é
  // o nó. Acha em que grafo/execução ela está, abre esse grafo, seleciona a
  // execução, centraliza o nó e abre o foco dele.
  const [goTo, setGoTo] = useState<{ graphId: string; runId: string; nodeId: string } | null>(
    null,
  );
  useEffect(() => {
    if (!focusSession) return;
    let alive = true;
    void (async () => {
      for (const g of await listGraphs()) {
        const hit = findSessionNode(await runsList(g.id), focusSession.sessionId);
        if (hit) {
          if (alive) setGoTo({ graphId: g.id, ...hit });
          return;
        }
      }
      if (alive) setNotice({ text: t("orbitSessionNotFound"), warn: true });
    })().catch(() => {});
    return () => {
      alive = false;
    };
  }, [focusSession, t]);
  useEffect(() => {
    if (!goTo || !meta) return;
    if (meta.id !== goTo.graphId) {
      if (want.id !== goTo.graphId) switchTo({ id: goTo.graphId, fresh: false });
      return;
    }
    // O histórico do grafo recém-aberto ainda está chegando.
    if (!runView.runs.some((r) => r.id === goTo.runId)) return;
    setGoTo(null);
    dispatchRun({ type: "select", runId: goTo.runId });
    setFocusNode(goTo.nodeId);
    setNodes((ns) => ns.map((n) => ({ ...n, selected: n.id === goTo.nodeId })));
    // Um quadro depois: o canvas recém-montado mede os nós antes de enquadrar.
    requestAnimationFrame(() => {
      void fitView({ nodes: [{ id: goTo.nodeId }], padding: 0.2, maxZoom: 1, duration: 200 });
    });
    // `switchTo` é refeito a cada render e fica fora; `want` diz o que já foi pedido.
  }, [goTo, meta, runView.runs, want.id, fitView]);

  return (
    <CanvasProvider actions={actions} state={ctx} now={now}>
      <div data-graph-screen className="flex min-w-0 flex-1 flex-col bg-tyba-bg font-mono">
        <div
          role="toolbar"
          aria-label={meta?.name || t("orbitGraphUntitled")}
          className="flex h-12 shrink-0 items-center gap-1.5 border-b border-tyba-border bg-tyba-surface pr-3.5 pl-[18px]"
        >
          <input
            ref={nameInput}
            aria-label={t("orbitGraphName")}
            placeholder={t("orbitGraphUntitled")}
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            value={meta?.name ?? ""}
            disabled={!meta}
            onChange={(e) => {
              const name = e.target.value;
              setMeta((m) => (m ? { ...m, name } : m));
            }}
            style={{ width: `${Math.max(12, (meta?.name.length ?? 12) + 1)}ch` }}
            className="bg-transparent text-[14px] font-medium text-tyba-text outline-none placeholder:text-tyba-text-ghost"
          />
          <DropdownMenu
            onOpenChange={(open) => {
              if (open) void listGraphs().then(setGraphs).catch(() => {});
            }}
          >
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                aria-label={t("orbitGraphs")}
                title={t("orbitGraphs")}
                className={`${tbtn} px-1.5`}
              >
                <CaretDown size={12} />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              align="start"
              className="w-64"
              onCloseAutoFocus={(e) => {
                if (!renaming.current) return;
                renaming.current = false;
                e.preventDefault();
                nameInput.current?.select();
              }}
            >
              {graphs.map((g) => (
                <DropdownMenuItem
                  key={g.id}
                  className="gap-2 text-xs"
                  onSelect={() => g.id !== meta?.id && switchTo({ id: g.id, fresh: false })}
                >
                  <Check
                    size={12}
                    className={g.id === meta?.id ? "shrink-0" : "invisible shrink-0"}
                  />
                  <span className="truncate">{g.name || t("orbitGraphUntitled")}</span>
                </DropdownMenuItem>
              ))}
              {graphs.length > 0 && <DropdownMenuSeparator />}
              <DropdownMenuItem
                className="gap-2 text-xs"
                onSelect={() => switchTo({ id: null, fresh: true })}
              >
                <Plus size={12} />
                {t("orbitNewGraph")}
              </DropdownMenuItem>
              <DropdownMenuItem
                className="gap-2 text-xs"
                disabled={!meta}
                onSelect={() => {
                  renaming.current = true;
                }}
              >
                <PencilSimple size={12} />
                {t("orbitRenameGraph")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          {sep}
          <button
            type="button"
            disabled={!meta || nodes.length === 0 || running !== null}
            onClick={() => void start()}
            className={`${tbtn} enabled:border-tyba-green/45 enabled:bg-tyba-green-tint enabled:font-medium enabled:text-tyba-green enabled:hover:border-tyba-green enabled:hover:bg-tyba-green-tint`}
          >
            <Play size={13} weight="fill" />
            {t("orbitRun")}
          </button>
          <button type="button" disabled={stoppable.length === 0} onClick={stop} className={tbtn}>
            <Stop size={13} weight="fill" />
            {t("orbitStop")}
          </button>
          {sep}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button type="button" disabled={runView.runs.length === 0} className={tbtn}>
                <ClockCounterClockwise size={13} />
                {t("orbitRuns")}
                {latest && (
                  <span className="rounded-[4px] bg-tyba-raised px-[5px] text-[10.5px] leading-4 text-tyba-text-muted">
                    #{runNumber(runView, latest.id)}
                  </span>
                )}
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-72">
              {runView.runs.map((r) => (
                <DropdownMenuItem
                  key={r.id}
                  className="gap-2 font-mono text-xs"
                  onSelect={() => dispatchRun({ type: "select", runId: r.id })}
                >
                  <i
                    className={`size-[7px] shrink-0 rounded-full ${
                      STATUS_TONE[r.status].dot
                    }`}
                  />
                  <span className={r.id === run?.id ? "text-tyba-text" : "text-tyba-text-muted"}>
                    #{runNumber(runView, r.id)}
                  </span>
                  <span className="text-tyba-text-faint">
                    {new Date(r.startedAt).toLocaleString(i18n.language, {
                      day: "2-digit",
                      month: "2-digit",
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </span>
                  <span className="ml-auto text-tyba-text-faint">
                    {t(`orbitRunStatus_${r.status}`)} ·{" "}
                    {formatDuration((r.finishedAt ?? now) - r.startedAt)}
                  </span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
          <button
            type="button"
            aria-pressed={showVars}
            onClick={() => setShowVars((v) => !v)}
            className={tbtn}
          >
            <BracketsCurly size={13} />
            {t("orbitVariables")}
          </button>
          <button
            type="button"
            disabled={!meta}
            aria-pressed={showInts}
            aria-expanded={showInts}
            aria-controls="orbit-integrations"
            onClick={() => setShowInts((v) => !v)}
            className={tbtn}
          >
            <PlugsConnected size={13} />
            {t("orbitIntegrations")}
          </button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                disabled={!meta}
                aria-label={t("orbitGraphSettings")}
                title={t("orbitGraphSettings")}
                className={tbtn}
              >
                <GearSix size={13} />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-56">
              <DropdownMenuItem className="gap-2 text-xs" onSelect={() => void exportFile()}>
                <FileArrowDown size={13} />
                {t("orbitExport")}
              </DropdownMenuItem>
              <DropdownMenuItem className="gap-2 text-xs" onSelect={() => void importFile()}>
                <FileArrowUp size={13} />
                {t("orbitImport")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <span className="flex-1" />
          <div
            role="group"
            aria-label={t("orbitZoom")}
            className="inline-flex items-center overflow-hidden rounded-[6px] border border-tyba-border-strong"
          >
            <button
              type="button"
              aria-label={t("orbitZoomOut")}
              onClick={() => void zoomOut({ duration: 150 })}
              className="flex px-[7px] py-[5px] text-tyba-text-muted hover:text-tyba-text"
            >
              <MagnifyingGlassMinus size={13} />
            </button>
            <button
              type="button"
              title={`${t("orbitZoomActual")} · ${mod}0`}
              onClick={() => void zoomTo(1, { duration: 150 })}
              className="min-w-[34px] px-1.5 text-center text-[11.5px] text-tyba-text"
            >
              {Math.round(zoom * 100)}%
            </button>
            <button
              type="button"
              aria-label={t("orbitZoomIn")}
              onClick={() => void zoomIn({ duration: 150 })}
              className="flex px-[7px] py-[5px] text-tyba-text-muted hover:text-tyba-text"
            >
              <MagnifyingGlassPlus size={13} />
            </button>
          </div>
          <button
            type="button"
            title={`${t("orbitFit")} · ${mod}1`}
            onClick={() => void fitView({ padding: 0.15, maxZoom: 1, duration: 200 })}
            className={tbtn}
          >
            <CornersOut size={13} />
            {t("orbitFit")}
          </button>
          <button
            type="button"
            aria-label={t("orbitMinimap")}
            title={t("orbitMinimap")}
            aria-pressed={showMinimap}
            onClick={() => setMinimapPref(!showMinimap)}
            className={tbtn}
          >
            <MapTrifold size={13} />
          </button>
        </div>
        <div
          className="relative flex min-h-0 flex-1"
          onDragOver={(e) => {
            if (!e.dataTransfer.types.includes(INTEGRATION_DRAG_TYPE)) return;
            e.preventDefault();
            e.dataTransfer.dropEffect = "copy";
          }}
          onDrop={dropIntegration}
        >
          {/* Só monta com o grafo carregado: o `fitView` inicial enquadra os nós dele. */}
          {meta ? (
            <OrbitCanvas
              key={meta.id}
              nodes={nodes}
              edges={edges}
              setNodes={setNodes}
              setEdges={setEdges}
              integrations={integrations}
              minimap={minimapPref}
              onOverflow={setOverflow}
              createdView={createdView}
              setCreatedView={setCreatedView}
              onUndo={undo}
              onRedo={redo}
              onRemoved={onRemoved}
              error={error}
              newDir={newDir}
              foreground={!covered && !showInts && importing === null}
              onCardApprovals={cardApprovals}
            />
          ) : (
            <div className="flex flex-1 items-center justify-center text-[12.5px] text-tyba-text-faint">
              {error}
            </div>
          )}
          {opened && (
            <SubagentPanel
              sessionId={opened.sessionId}
              run={opened.run}
              onClose={() => setOpenSub(null)}
            />
          )}
          {showVars && meta && (
            <VariablesPanel
              variables={meta.variables}
              onChange={(variables) => setMeta((m) => (m ? { ...m, variables } : m))}
              onClose={() => setShowVars(false)}
            />
          )}
          {showInts && meta && (
            <>
              <div
                aria-hidden
                onMouseDown={() => setShowInts(false)}
                className="absolute inset-0 z-10 bg-tyba-bg/55"
              />
              <IntegrationsPanel
                integrations={integrations}
                nodes={graph.nodes}
                onSaved={integrationSaved}
                onTest={testIntegration}
                onClose={() => setShowInts(false)}
              />
            </>
          )}
          {notice && (
            <div
              role="status"
              className={`absolute bottom-3 left-1/2 z-20 flex max-w-[640px] -translate-x-1/2 items-start gap-2 rounded-[6px] border px-3 py-2 text-[12px] shadow-[var(--tyba-shadow-md)] ${
                notice.warn
                  ? "border-tyba-amber/50 bg-tyba-amber-tint text-tyba-amber"
                  : "border-tyba-border-strong bg-tyba-surface text-tyba-text-muted"
              }`}
            >
              {notice.warn && <Warning size={14} className="mt-px shrink-0" />}
              <span className="break-all">{notice.text}</span>
              {notice.undo && (
                <button
                  type="button"
                  onClick={undo}
                  className="shrink-0 font-medium text-tyba-text hover:underline"
                >
                  {t("graphUndo")}
                </button>
              )}
              <button
                type="button"
                aria-label={t("close")}
                onClick={() => setNotice(null)}
                className="shrink-0 rounded-[4px] p-0.5 opacity-70 hover:opacity-100"
              >
                <X size={12} />
              </button>
            </div>
          )}
        </div>
        {meta && (
          <RunTimeline
            graph={viewGraph}
            run={run}
            number={run ? runNumber(runView, run.id) : 0}
            now={now}
          />
        )}
      </div>
      {importing && (
        <ImportDialog
          path={importing.path}
          defaultDir={newDir}
          preview={importing.preview}
          onCancel={() => setImporting(null)}
          onImported={imported}
        />
      )}
    </CanvasProvider>
  );
}

/**
 * O que um subagente fez (quadro OrbitSubagentes): estado, pedido, tempo e
 * o transcript, que segue ao vivo enquanto ele roda.
 */
function SubagentPanel({
  sessionId,
  run,
  onClose,
}: {
  sessionId: string;
  run: SubagentRun;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const state = subagentState(run);
  const time = useSubagentTime(run);
  return (
    <aside
      aria-label={run.description || t("orbitSubNoDescription")}
      className="flex w-[400px] shrink-0 flex-col border-l border-tyba-border bg-tyba-surface"
    >
      <div className="flex shrink-0 items-center gap-2.5 border-b border-tyba-border px-4 py-3">
        <span className={subagentTone(state)} title={t(`orbitSub_${state}`)}>
          <SubagentMark state={state} size={13} />
        </span>
        <span className="min-w-0 flex-1 truncate text-[13px] text-tyba-text">
          {run.description || t("orbitSubNoDescription")}
        </span>
        <span className="shrink-0 text-[12px] text-tyba-text-faint">{time}</span>
        <button
          type="button"
          aria-label={t("close")}
          onClick={onClose}
          className="shrink-0 rounded-[4px] p-1 text-tyba-text-faint hover:text-tyba-text"
        >
          <X size={13} />
        </button>
      </div>
      <SubagentTranscript
        sessionId={sessionId}
        run={run}
        scrollClassName="min-h-0 flex-1 overflow-y-auto px-4 py-3.5"
      />
    </aside>
  );
}

/**
 * Variáveis do grafo: pares nome/valor. A lista local guarda linhas ainda sem
 * nome; o grafo só recebe as que têm nome.
 */
function VariablesPanel({
  variables,
  onChange,
  onClose,
}: {
  variables: Record<string, string>;
  onChange: (variables: Record<string, string>) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [rows, setRows] = useState<[string, string][]>(() => Object.entries(variables));
  const commit = (next: [string, string][]) => {
    setRows(next);
    onChange(Object.fromEntries(next.filter(([name]) => name.trim() !== "")));
  };
  const field =
    "h-7 min-w-0 flex-1 rounded-[4px] border border-tyba-border bg-tyba-text/[.02] px-2 text-[12px] text-tyba-text outline-none focus:border-tyba-border-strong";

  return (
    <aside
      aria-label={t("orbitVariables")}
      className="flex w-[320px] shrink-0 flex-col gap-2 border-l border-tyba-border bg-tyba-surface p-3"
    >
      <div className="flex items-center justify-between">
        <span className="text-[13px] font-medium text-tyba-text">{t("orbitVariables")}</span>
        <button
          type="button"
          aria-label={t("close")}
          onClick={onClose}
          className="rounded-[4px] p-1 text-tyba-text-faint hover:text-tyba-text"
        >
          <X size={13} />
        </button>
      </div>
      <p className="text-[11.5px] text-tyba-text-faint">
        {t("orbitVarsHint", { example: "{{nome}}" })}
      </p>
      <div className="flex min-h-0 flex-col gap-1.5 overflow-y-auto">
        {rows.map(([name, value], i) => (
          <div key={i} className="flex items-center gap-1.5">
            <input
              aria-label={t("orbitVarName")}
              placeholder={t("orbitVarName")}
              spellCheck={false}
              autoCorrect="off"
              autoCapitalize="off"
              value={name}
              onChange={(e) =>
                commit(rows.map((r, j) => (j === i ? [e.target.value, r[1]] : r)))
              }
              className={field}
            />
            <input
              aria-label={t("orbitVarValue")}
              placeholder={t("orbitVarValue")}
              spellCheck={false}
              autoCorrect="off"
              autoCapitalize="off"
              value={value}
              onChange={(e) =>
                commit(rows.map((r, j) => (j === i ? [r[0], e.target.value] : r)))
              }
              className={field}
            />
            <button
              type="button"
              aria-label={t("orbitVarRemove")}
              onClick={() => commit(rows.filter((_, j) => j !== i))}
              className="rounded-[4px] p-1 text-tyba-text-faint hover:text-tyba-red"
            >
              <X size={12} />
            </button>
          </div>
        ))}
      </div>
      <button
        type="button"
        onClick={() => setRows([...rows, ["", ""]])}
        className="inline-flex h-7 items-center gap-1.5 self-start rounded-[6px] px-2 text-[12px] text-tyba-text-muted hover:bg-tyba-raised hover:text-tyba-text"
      >
        <Plus size={12} />
        {t("orbitVarAdd")}
      </button>
    </aside>
  );
}
