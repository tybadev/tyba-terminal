import {
  useCallback,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type CSSProperties,
  type DragEvent,
  type ReactElement,
} from "react";
import { Trans, useTranslation } from "react-i18next";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { homeDir } from "@tauri-apps/api/path";
import {
  ArrowClockwise,
  BracketsCurly,
  CaretDown,
  Check,
  ClockCounterClockwise,
  CornersOut,
  DotsThree,
  Eye,
  FileArrowDown,
  FileArrowUp,
  GearSix,
  MagnifyingGlassMinus,
  MagnifyingGlassPlus,
  MapTrifold,
  PencilSimple,
  Play,
  IdentificationBadge,
  Lock,
  LockOpen,
  PlugsConnected,
  Plus,
  Stop,
  Trash,
  Warning,
  X,
} from "@phosphor-icons/react";
import { ReactFlowProvider, useReactFlow, useViewport } from "@xyflow/react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Shortcut } from "@/components/ui/kbd";
import { cn } from "@/lib/utils";
import { PLATE, PLATE_ACRYLIC, PLATE_LINE } from "@/components/plate";
import {
  EMPTY_RUN_VIEW,
  INTEGRATION_DRAG_TYPE,
  agentSessions as agentSessionsOf,
  browserFed,
  findSessionNode,
  integrationsById,
  invalidEdges,
  isLiveRun,
  createdByOf,
  deleteGraph,
  ephemeralCreated,
  keepCreated,
  NEW_IN_HANDLE,
  OUT_HANDLE,
  isLocalUrl,
  retainedContainers,
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
  listRoles,
  newNode,
  onRun,
  pickGraph,
  rememberDir,
  rememberGraph,
  runGet,
  runNode,
  runNumber,
  runRestartNode,
  runReducer,
  runStart,
  runStop,
  runStopNode,
  runsList,
  saveGraph,
  selectedRun,
  stoppableRuns,
  terminalClose,
  terminalOpen,
  terminalRun,
  terminalTitle,
  toFlow,
  type Graph,
  type ImportPreview,
  type Integration,
  type Role,
  type OrbitNode,
  type Plan,
  runElapsed,
  runShownStatus,
  runStatusKey,
  type VarEnvProblem,
  varEnvProblems,
  revealSecret,
  secretsSupported,
  setSecret,
  staleTerminals,
  toggleDrawer,
  type OrbitDrawer,
} from "@/lib/orbit";
import { browserRetain } from "@/lib/orbitBrowser";
import { roomAgents } from "@/lib/orbitTalk";
import { ConfirmDelete } from "./ConfirmDelete";
import { ImportDialog } from "./ImportDialog";
import { RolesPanel } from "./RolesPanel";
import { IntegrationsPanel } from "./IntegrationsPanel";
import { OrbitCanvas, type CreatedView, type OrbitFlowEdge } from "./OrbitCanvas";
import { RunTimeline } from "./RunTimeline";
import { TerminalFolderPrompt } from "./TerminalFolderPrompt";
import { CanvasProvider, STATUS_TONE, type OrbitFlowNode } from "./nodes/NodeFrame";
import { DRAWER } from "./drawer";
import { useStripLevel } from "./useStripLevel";
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
  plansInHistory,
  record,
  redo as redoShot,
  restoreShot,
  undo as undoShot,
} from "@/lib/graphHistory";
import { freeSpot, nodeRect, rightOf } from "@/lib/graphLayout";
import { applyPlans, freshPlans, planWriters } from "@/lib/orbitPlan";
import { deletedNotice } from "@/lib/orbitHandling";
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

/** Botão da faixa só ícone: padding 7, como a engrenagem (quadro FaixaOrbit). */
const ico = (on: boolean, base = tbtn) => (on ? cn(base, "px-[7px]") : base);

/**
 * Botão da faixa que perde o rótulo num degrau (quadro FaixaOrbit F4): o
 * rótulo vira o tooltip do TYBA, embaixo do botão. Fora do modo ícone o
 * tooltip não aparece — o rótulo está à vista.
 */
function StripTip({
  label,
  combo,
  on,
  children,
}: {
  label: string;
  combo?: string;
  on: boolean;
  children: ReactElement;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      {on && (
        <TooltipContent side="bottom" sideOffset={8} className="flex items-center gap-2">
          {label}
          {combo && <Shortcut combo={combo} />}
        </TooltipContent>
      )}
    </Tooltip>
  );
}

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
  onBackToTerminal,
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
  /** "❯ Terminal", o primeiro item da paleta: volta para o terminal. */
  onBackToTerminal?: () => void;
}) {
  return (
    <ReactFlowProvider>
      <OrbitInner
        focusSession={focusSession}
        approvals={approvals}
        onCardApprovals={onCardApprovals}
        covered={covered}
        sessions={sessions}
        onBackToTerminal={onBackToTerminal}
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
  onBackToTerminal,
}: {
  focusSession: FocusSessionRequest | null;
  approvals: readonly ApprovalRequest[];
  onCardApprovals?: (ids: ReadonlySet<number>) => void;
  covered: boolean;
  sessions: readonly Session[];
  onBackToTerminal?: () => void;
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
  // Quadro FaixaOrbit: degrau da faixa pela largura da linha dela.
  const stripRow = useRef<HTMLDivElement>(null);
  const stripLeft = useRef<HTMLDivElement>(null);
  const stripRight = useRef<HTMLDivElement>(null);
  const { level: strip, height: stripHeight } = useStripLevel(stripRow, stripLeft, stripRight);
  // O nome do grafo trunca do degrau 2 em diante, menos com o foco nele.
  const [nameFocus, setNameFocus] = useState(false);
  const renaming = useRef(false);

  const [meta, setMeta] = useState<Pick<
    Graph,
    "id" | "name" | "dir" | "variables" | "secrets"
  > | null>(null);
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
  const [roles, setRoles] = useState<Role[]>([]);
  const [error, setError] = useState<string | null>(null);
  // O erro que o salvar automático pôs (e só ele o apaga ao salvar de novo).
  const saveError = useRef<string | null>(null);
  // A gaveta aberta (Variáveis, Integrações ou Papéis): uma por vez.
  const [drawer, setDrawer] = useState<OrbitDrawer | null>(null);
  const showVars = drawer?.kind === "vars";
  const showInts = drawer?.kind === "ints";
  const showRoles = drawer?.kind === "roles" ? drawer.start : null;
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
  const [notice, setNotice] = useState<{ text: string; warn: boolean; undo?: boolean } | null>(
    null,
  );
  const [importing, setImporting] = useState<{
    path: string;
    preview: ImportPreview;
  } | null>(null);
  const [reload, setReload] = useState(0);
  // Apagar o grafo: o diálogo aberto, e o id apagado (o salvar no debounce
  // não o grava de volta).
  const [askDelete, setAskDelete] = useState(false);
  const deletedId = useRef<string | null>(null);

  // Carrega o grafo escolhido (ou começa um vazio, gravado na primeira edição).
  useEffect(() => {
    if (home === null) return;
    let alive = true;
    setMeta(null);
    dispatchRun({ type: "reset" });
    void Promise.all([listGraphs(), listIntegrations(), listRoles()])
      .then(([list, ints, roleList]) => {
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
        setRoles(roleList);
        setNodes(flow.nodes as OrbitFlowNode[]);
        setEdges(flow.edges);
        // Outro grafo: o ⌘Z começa do zero (o retrato dele entra no próximo assentar).
        historyRef.current = EMPTY_HISTORY;
        settledRef.current = null;
        setMeta({
          id: graph.id,
          name: graph.name,
          dir: graph.dir,
          variables: graph.variables,
          secrets: graph.secrets ?? [],
        });
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
    (removed: readonly Pick<OrbitNode, "title" | "kind">[]) => {
      const notice = deletedNotice(removed);
      setNotice({ text: t(notice.key, notice.params), warn: false, undo: true });
    },
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
      const graph = JSON.parse(snapshot) as Graph;
      if (graph.id === deletedId.current) return;
      saveGraph(graph)
        .then(() => {
          lastSaved.current = snapshot;
          // Só apaga o erro que ele mesmo pôs: a recusa de outra ação (o core
          // recusou o Terminal) fica à vista até o dono agir.
          setError((cur) => (cur === saveError.current ? null : cur));
          saveError.current = null;
        })
        .catch((e) => {
          saveError.current = t("orbitSaveError", { error: String(e) });
          setError(saveError.current);
        });
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
  // Os runs do grafo na tela: o Terminal aberto vive no dele, à vista ou não.
  const graphRuns = useMemo(
    () => runView.runs.filter((r) => r.graphId === meta?.id),
    [runView.runs, meta?.id],
  );
  const runsRef = useRef(graphRuns);
  runsRef.current = graphRuns;
  // Quadro GavetasOrbit O2: Terminais abertos com variável mudada depois.
  const stale = useMemo(() => staleTerminals(graphRuns), [graphRuns]);
  // Esc fecha a gaveta também com o foco fora dela (no botão da faixa, no
  // canvas). Dentro dela o onKeyDown da própria fecha; num terminal, campo,
  // menu ou diálogo o Esc é de lá.
  const drawerOpen = drawer !== null;
  useEffect(() => {
    if (!drawerOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      const el = e.target instanceof Element ? e.target : null;
      if (
        el?.closest(
          "[data-orbit-sheet], input, textarea, select, [contenteditable=true], .xterm, [role=dialog], [role=menu]",
        )
      )
        return;
      setDrawer(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [drawerOpen]);

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
  // O plano vem do core e não é passo do ⌘Z (ver `plansInHistory`): o que
  // estava pendente assenta antes, e a nota entra em todo retrato da pilha.
  const putPlans = (list: Plan[]) => {
    if (list.length === 0) return;
    settle();
    const next = applyPlans(nodesRef.current, edgesRef.current, list);
    const shot = graphShot(next.nodes, next.edges);
    historyRef.current = plansInHistory(
      historyRef.current,
      shot,
      new Set(list.map((p) => p.note.id)),
    );
    settledRef.current = shot;
    setNodes(next.nodes);
    setEdges(next.edges);
  };
  const putPlansRef = useRef(putPlans);
  putPlansRef.current = putPlans;
  useEffect(() => {
    if (plans.length === 0) return;
    for (const p of plans) appliedPlans.current.set(p.note.id, p.updatedAt);
    putPlansRef.current(plans);
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

  // O botão trava no clique, antes de o core responder: sem segundo ▶.
  const [starting, setStarting] = useState(false);
  const start = async () => {
    if (!snapshot || !meta || starting) return;
    setStarting(true);
    try {
      await flush();
      const id = await runStart(meta.id);
      const started = await runGet(id);
      if (started) dispatchRun({ type: "update", run: started });
      setError(null);
    } catch (e) {
      setError(t("orbitRunError", { error: String(e) }));
    } finally {
      setStarting(false);
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

  // + Terminal (barra e paleta): o seletor de pasta pergunta onde; ↵ cria o
  // nó, grava o grafo (o core lê do banco) e pede ao core para abrir. Recusou
  // (outro Terminal aberto, pasta que sumiu): o nó sai e o porquê aparece.
  const [askTerminal, setAskTerminal] = useState(false);
  const terminalAt = useRef<{ x: number; y: number } | null>(null);
  const addTerminal = (at: { x: number; y: number } | null) => {
    terminalAt.current = at;
    setAskTerminal(true);
  };
  const createTerminal = async (dir: string) => {
    setAskTerminal(false);
    if (!meta) return;
    const node = newNode(crypto.randomUUID(), terminalTitle(dir), { type: "terminal", dir }, { x: 0, y: 0 });
    const mid = screenToFlowPosition({ x: window.innerWidth / 2, y: window.innerHeight / 2 });
    const want = terminalAt.current ?? { x: mid.x - node.size.w / 2, y: mid.y - node.size.h / 2 };
    node.position = freeSpot(want, node.size, nodesRef.current.map(nodeRect));
    if (dir) rememberDir(dir);
    const next: OrbitFlowNode[] = [
      ...nodesRef.current.map((n) => ({ ...n, selected: false })),
      {
        id: node.id,
        type: node.kind.type,
        position: node.position,
        width: node.size.w,
        height: node.size.h,
        selected: true,
        data: { node },
      },
    ];
    setNodes(next);
    try {
      const graph = fromFlow(meta, next, edgesRef.current);
      await saveGraph(graph);
      lastSaved.current = JSON.stringify(graph);
      dispatchRun({ type: "update", run: await terminalOpen(meta.id, node.id) });
      setError(null);
    } catch (e) {
      // O nó sai antes de o porquê aparecer, e o grafo sem ele já vai gravado
      // aqui: o salvar automático não tem o que gravar depois.
      const rest = nodesRef.current.filter((n) => n.id !== node.id);
      setNodes(rest);
      setError(t("orbitTerminalError", { error: String(e) }));
      const graph = fromFlow(meta, rest, edgesRef.current);
      await saveGraph(graph)
        .then(() => {
          lastSaved.current = JSON.stringify(graph);
        })
        .catch(() => {});
    }
  };

  // Abrir do Terminal salvo (fechado) e ⋯ → Fechar: o core decide e recusa;
  // o retrato novo chega pelo evento do run (e é relido aqui).
  const graphIdRef = useRef<string | null>(null);
  graphIdRef.current = meta?.id ?? null;
  const flushRef = useRef(flush);
  flushRef.current = flush;
  const openTerminal = useCallback(
    async (nodeId: string) => {
      const gid = graphIdRef.current;
      if (!gid) return;
      try {
        await flushRef.current();
        dispatchRun({ type: "update", run: await terminalOpen(gid, nodeId) });
        setError(null);
      } catch (e) {
        setError(t("orbitTerminalError", { error: String(e) }));
      }
    },
    [t],
  );
  const closeTerminal = useCallback(
    (nodeId: string) => {
      const gid = graphIdRef.current;
      if (!gid) return;
      // O run que segura o Terminal, não o à vista (pode ser o de outro teste).
      const runId = terminalRun(runsRef.current, nodeId)?.id ?? null;
      terminalClose(gid, nodeId)
        .then(() => (runId ? runGet(runId) : null))
        .then((run) => {
          if (run) dispatchRun({ type: "update", run });
        })
        .catch((e) => setError(t("orbitTerminalError", { error: String(e) })));
    },
    [t],
  );

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

  // Parar / ↻ de um nó só: o core decide (e recusa, com o porquê, o que não
  // pode); o retrato novo chega pelo evento do run.
  const stopOneNode = useCallback(
    (nodeId: string) => {
      const runId = runIdRef.current;
      if (!runId) return;
      runStopNode(runId, nodeId).catch((e) =>
        setError(t("orbitNodeStopError", { error: String(e) })),
      );
    },
    [t],
  );
  const restartOneNode = useCallback(
    (nodeId: string) => {
      const runId = runIdRef.current;
      if (!runId) return;
      runRestartNode(runId, nodeId).catch((e) =>
        setError(t("orbitRestartError", { error: String(e) })),
      );
    },
    [t],
  );

  const testIntegration = async (integrationId: string, nodeId: string) => {
    if (!meta) return;
    await flush();
    dispatchRun({ type: "update", run: await integrationTest(integrationId, meta.id, nodeId) });
  };

  /** Apaga o grafo aberto (o core recusa com execução aberta) e abre outro. */
  const removeGraph = async () => {
    if (!meta) return;
    deletedId.current = meta.id;
    try {
      await deleteGraph(meta.id);
    } catch (e) {
      deletedId.current = null;
      throw e;
    }
    setWant({ id: null, fresh: false });
  };

  const integrationSaved = (saved: Integration) =>
    setIntegrations((list) =>
      list.some((i) => i.id === saved.id)
        ? list.map((i) => (i.id === saved.id ? saved : i))
        : [...list, saved],
    );

  const roleSaved = (saved: Role) =>
    setRoles((list) =>
      (list.some((r) => r.name === saved.name)
        ? list.map((r) => (r.name === saved.name ? saved : r))
        : [...list, saved]
      ).sort((a, b) => a.name.localeCompare(b.name)),
    );

  // Renomear na biblioteca leva os nós junto: o core regravou os grafos
  // salvos; o canvas aberto acompanha aqui, senão o salvar automático dele
  // traria o nome antigo de volta.
  const roleRenamed = (from: string, to: string) => {
    setRoles((list) => list.filter((r) => r.name !== from));
    for (const n of nodesRef.current) {
      const kind = n.data.node.kind;
      if (kind.type === "agent" && kind.role === from) {
        updateNode(n.id, (node) => ({ ...node, kind: { ...kind, role: to } }));
      }
    }
    void listGraphs()
      .then(setGraphs)
      .catch(() => {});
  };

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
  // Os criados pelo agente entram pelo id vivo que o card achou.
  const [liveContainers, setLiveContainers] = useState<ReadonlyMap<string, string>>(new Map());
  const reportContainer = useCallback((nodeId: string, containerId: string | null) => {
    setLiveContainers((m) => {
      if ((m.get(nodeId) ?? null) === containerId) return m;
      const next = new Map(m);
      if (containerId) next.set(nodeId, containerId);
      else next.delete(nodeId);
      return next;
    });
  }, []);
  const containers = retainedContainers(viewGraph.nodes, createdIds, liveContainers).join(",");
  useEffect(() => {
    void serviceLogsRetain(containers ? containers.split(",") : []).catch(() => {});
  }, [containers]);
  useEffect(() => () => void serviceLogsRetain([]).catch(() => {}), []);

  // Navegador nativo (macOS): só vivem as views dos navegadores do grafo à
  // vista; trocar de grafo ou fechar o Orbit destrói as outras.
  const graphId = meta?.id ?? "";
  const browserIds = viewGraph.nodes
    .filter((n) => n.kind.type === "browser")
    .map((n) => n.id)
    .join(",");
  useEffect(() => {
    if (!graphId) return;
    void browserRetain(graphId, browserIds ? browserIds.split(",") : []).catch(() => {});
  }, [graphId, browserIds]);
  useEffect(() => () => void browserRetain("", []).catch(() => {}), []);

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
      graphId,
      runNode: releaseNode,
      stopNode: stopOneNode,
      restartNode: restartOneNode,
      setOpenSubagent: setOpenSub,
      setFocusNode,
      openInBrowser,
      keepCreated: (nodeId: string | null, orchestrator?: string) =>
        void keepRef.current(nodeId, orchestrator ?? null),
      answerApproval: (id: number, decision: ApprovalDecision) =>
        void resolveApproval(id, decision).catch((e) =>
          setError(t("orbitApprovalError", { error: String(e) })),
        ),
      openTerminal,
      closeTerminal,
      reportContainer,
      roles,
      openRoles: (opts?: { newName?: string; assignTo?: string }) =>
        setDrawer({ kind: "roles", start: opts ?? {} }),
    }),
    [
      integrations,
      roles,
      updateNode,
      graphDir,
      graphId,
      releaseNode,
      stopOneNode,
      restartOneNode,
      openInBrowser,
      openTerminal,
      closeTerminal,
      reportContainer,
      t,
    ],
  );
  const room = roomAgents(viewGraph.edges);
  const shownNumber = run ? runNumber(runView, run.id) : 0;
  const writers = useMemo(
    () =>
      planWriters(
        runView.runs.filter((r) => r.graphId === meta?.id),
        (id) => runNumber(runView, id),
      ),
    [runView, meta?.id],
  );
  const ctx = useMemo(
    () => ({
      graph: viewGraph,
      run,
      runs: graphRuns,
      live,
      subagents,
      openSubagent: opened ? openSub : null,
      focusNode,
      createdIds,
      createdBy,
      approvals: nodeApprovals,
      telemetry,
      runNumber: shownNumber,
      planWriters: writers,
    }),
    [
      viewGraph,
      run,
      graphRuns,
      live,
      subagents,
      opened,
      openSub,
      focusNode,
      createdIds,
      createdBy,
      nodeApprovals,
      telemetry,
      shownNumber,
      writers,
    ],
  );

  // "Manter" (o criado `nodeId`, ou todos os de `orchestrator` no "Manter
  // tudo"): o core grava no grafo e devolve o grafo inteiro, que passa a ser
  // o da tela (o que estava no debounce vai antes).
  const keep = async (nodeId: string | null, orchestrator: string | null) => {
    if (!run) return;
    try {
      await flush();
      const kept = await keepCreated(run.id, nodeId, orchestrator);
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

  // As execuções, no menu do botão e no submenu do ⋯ (degrau 3).
  const runItems = runView.runs.map((r) => (
    <DropdownMenuItem
      key={r.id}
      className="gap-2 font-mono text-xs"
      onSelect={() => dispatchRun({ type: "select", runId: r.id })}
    >
      <i className={`size-[7px] shrink-0 rounded-full ${STATUS_TONE[runShownStatus(r)].dot}`} />
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
        {t(runStatusKey(r))} · {formatDuration(runElapsed(r, now))}
      </span>
    </DropdownMenuItem>
  ));
  const runBadge = latest && (
    <span className="rounded-[4px] bg-tyba-raised px-[5px] text-[10.5px] leading-4 text-tyba-text-muted">
      #{runNumber(runView, latest.id)}
    </span>
  );

  return (
    <CanvasProvider actions={actions} state={ctx} now={now}>
      <div data-graph-screen className="flex min-w-0 flex-1 flex-col bg-tyba-bg font-mono">
        <div
          className="relative flex min-h-0 flex-1"
          onDragOver={(e) => {
            if (!e.dataTransfer.types.includes(INTEGRATION_DRAG_TYPE)) return;
            e.preventDefault();
            e.dataTransfer.dropEffect = "copy";
          }}
          onDrop={dropIntegration}
        >
          <div
            className="relative flex min-w-0 flex-1"
            // Gavetas 8 abaixo da faixa, que cresce quando a placa da direita desce.
            style={{ "--orbit-strip-bottom": `${12 + stripHeight + 8}px` } as CSSProperties}
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
                foreground={!covered && !showInts && importing === null && !askTerminal}
                onCardApprovals={cardApprovals}
                onAddTerminal={addTerminal}
              />
            ) : (
              <div className="flex flex-1 items-center justify-center text-[12.5px] text-tyba-text-faint">
                {error}
              </div>
            )}
            {/* A paleta flutua em vidro por cima do canvas (visual-vidro, 15ª
                rodada): o canvas passa por baixo e recebe pan/zoom em toda a
                área; só as placas capturam o clique. */}
            {/* Quadro FaixaOrbit: os rótulos saem em degraus antes de as placas
                se sobreporem; abaixo do último, a placa da direita desce. */}
            <div
              ref={stripRow}
              role="toolbar"
              aria-label={meta?.name || t("orbitGraphUntitled")}
              className="pointer-events-none absolute top-3 right-3 left-3 z-[5] flex flex-wrap items-start justify-between gap-x-3 gap-y-2"
            >
              <div
                ref={stripLeft}
                className={`pointer-events-auto flex h-11 items-center gap-1.5 px-2.5 ${PLATE} ${PLATE_LINE} ${PLATE_ACRYLIC}`}
              >
                {onBackToTerminal && (
                  <>
                    <StripTip label={t("terminalView")} on={strip >= 4}>
                      <button
                        type="button"
                        aria-label={strip >= 4 ? t("terminalView") : undefined}
                        onClick={onBackToTerminal}
                        className={ico(strip >= 4)}
                      >
                        <span aria-hidden className="text-tyba-green">
                          ❯
                        </span>
                        {strip < 4 && t("terminalView")}
                      </button>
                    </StripTip>
                    {sep}
                  </>
                )}
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
                  onFocus={() => setNameFocus(true)}
                  onBlur={() => setNameFocus(false)}
                  style={{
                    width: `${Math.min(
                      Math.max(12, (meta?.name.length ?? 12) + 1),
                      strip >= 2 && !nameFocus ? 15 : Infinity,
                    )}ch`,
                  }}
                  className="bg-transparent text-[14px] font-medium text-ellipsis text-tyba-text outline-none placeholder:text-tyba-text-ghost"
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
                    <DropdownMenuItem
                      className="gap-2 text-xs text-tyba-red focus:text-tyba-red"
                      disabled={!meta || running !== null}
                      title={running !== null ? t("orbitDeleteGraphRunning") : undefined}
                      onSelect={() => setAskDelete(true)}
                    >
                      <Trash size={12} />
                      {t("orbitDeleteGraph")}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
                {/* Quadro MaestroConversa: run vivo com agentes em conversa. */}
                {running !== null && room >= 2 && (
                  <span className="shrink-0 px-1 text-[12px] whitespace-nowrap text-tyba-green">
                    {t("orbitRoomActive", { count: room })}
                  </span>
                )}
                {sep}
                <StripTip label={t("orbitAddTerminal")} on={strip >= 2}>
                  <button
                    type="button"
                    aria-label={strip >= 2 ? t("orbitAddTerminal") : undefined}
                    disabled={!meta}
                    onClick={() => addTerminal(null)}
                    className={ico(
                      strip >= 2,
                      `${tbtn} rounded-full enabled:border-tyba-green/30 enabled:bg-tyba-green-tint enabled:text-tyba-green enabled:hover:border-tyba-green/60`,
                    )}
                  >
                    <Plus size={13} weight="bold" />
                    {strip < 2 && t("orbitAddTerminal")}
                  </button>
                </StripTip>
                {/* Com Terminal aberto o ▶ fica clicável: o core recusa e diz por quê. */}
                <StripTip label={t("orbitRun")} on={strip >= 4}>
                  <button
                    type="button"
                    aria-label={strip >= 4 ? t("orbitRun") : undefined}
                    disabled={
                      !meta || nodes.length === 0 || (running !== null && !isLiveRun(running)) || starting
                    }
                    onClick={() => void start()}
                    className={ico(
                      strip >= 4,
                      `${tbtn} enabled:border-tyba-green/45 enabled:bg-tyba-green-tint enabled:font-medium enabled:text-tyba-green enabled:hover:border-tyba-green enabled:hover:bg-tyba-green-tint`,
                    )}
                  >
                    <Play size={13} weight="fill" />
                    {strip < 4 && t("orbitRun")}
                  </button>
                </StripTip>
                <StripTip label={t("orbitStop")} on={strip >= 4}>
                  <button
                    type="button"
                    aria-label={strip >= 4 ? t("orbitStop") : undefined}
                    disabled={stoppable.length === 0}
                    onClick={stop}
                    className={ico(strip >= 4)}
                  >
                    <Stop size={13} weight="fill" />
                    {strip < 4 && t("orbitStop")}
                  </button>
                </StripTip>
                {sep}
                {strip >= 3 ? (
                  // Degrau 3: os quatro painéis no menu ⋯, aceso com a gaveta aberta.
                  <DropdownMenu>
                    <StripTip label={t("orbitPanels")} on>
                      <DropdownMenuTrigger asChild>
                        <button
                          type="button"
                          aria-label={t("orbitPanels")}
                          className={cn(
                            ico(true),
                            "data-[state=open]:border-tyba-border-strong data-[state=open]:bg-tyba-raised data-[state=open]:text-tyba-text",
                            drawerOpen && "border-tyba-border-strong bg-tyba-raised text-tyba-text",
                          )}
                        >
                          <DotsThree size={13} weight="bold" />
                        </button>
                      </DropdownMenuTrigger>
                    </StripTip>
                    <DropdownMenuContent align="start" className="w-56">
                      <DropdownMenuSub>
                        <DropdownMenuSubTrigger
                          disabled={runView.runs.length === 0}
                          className="gap-2 text-xs [&>svg:last-child]:ml-0"
                        >
                          <ClockCounterClockwise size={13} />
                          {t("orbitRuns")}
                          <span className="ml-auto">{runBadge}</span>
                        </DropdownMenuSubTrigger>
                        <DropdownMenuSubContent className="w-72">{runItems}</DropdownMenuSubContent>
                      </DropdownMenuSub>
                      <DropdownMenuItem
                        className="gap-2 text-xs"
                        onSelect={() => setDrawer((d) => toggleDrawer(d, "vars"))}
                      >
                        <BracketsCurly size={13} />
                        {t("orbitVariables")}
                        {showVars && <Check size={13} className="ml-auto" />}
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        className="gap-2 text-xs"
                        disabled={!meta}
                        onSelect={() => setDrawer((d) => toggleDrawer(d, "ints"))}
                      >
                        <PlugsConnected size={13} />
                        {t("orbitIntegrations")}
                        {showInts && <Check size={13} className="ml-auto" />}
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        className="gap-2 text-xs"
                        disabled={!meta}
                        onSelect={() => setDrawer((d) => toggleDrawer(d, "roles"))}
                      >
                        <IdentificationBadge size={13} />
                        {t("orbitRoles")}
                        {!!showRoles && <Check size={13} className="ml-auto" />}
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                ) : (
                  <>
                    <DropdownMenu>
                      <StripTip label={t("orbitRuns")} on={strip >= 1}>
                        <DropdownMenuTrigger asChild>
                          <button
                            type="button"
                            aria-label={strip >= 1 ? t("orbitRuns") : undefined}
                            disabled={runView.runs.length === 0}
                            className={ico(strip >= 1)}
                          >
                            <ClockCounterClockwise size={13} />
                            {strip < 1 && t("orbitRuns")}
                            {runBadge}
                          </button>
                        </DropdownMenuTrigger>
                      </StripTip>
                      <DropdownMenuContent align="start" className="w-72">
                        {runItems}
                      </DropdownMenuContent>
                    </DropdownMenu>
                    <StripTip label={t("orbitVariables")} on={strip >= 1}>
                      <button
                        type="button"
                        aria-label={strip >= 1 ? t("orbitVariables") : undefined}
                        aria-pressed={showVars}
                        onClick={() => setDrawer((d) => toggleDrawer(d, "vars"))}
                        className={ico(strip >= 1)}
                      >
                        <BracketsCurly size={13} />
                        {strip < 1 && t("orbitVariables")}
                      </button>
                    </StripTip>
                    <StripTip label={t("orbitIntegrations")} on={strip >= 1}>
                      <button
                        type="button"
                        aria-label={strip >= 1 ? t("orbitIntegrations") : undefined}
                        disabled={!meta}
                        aria-pressed={showInts}
                        aria-expanded={showInts}
                        aria-controls="orbit-integrations"
                        onClick={() => setDrawer((d) => toggleDrawer(d, "ints"))}
                        className={ico(strip >= 1)}
                      >
                        <PlugsConnected size={13} />
                        {strip < 1 && t("orbitIntegrations")}
                      </button>
                    </StripTip>
                    <StripTip label={t("orbitRoles")} on={strip >= 1}>
                      <button
                        type="button"
                        aria-label={strip >= 1 ? t("orbitRoles") : undefined}
                        disabled={!meta}
                        aria-pressed={!!showRoles}
                        aria-expanded={!!showRoles}
                        aria-controls="orbit-roles"
                        onClick={() => setDrawer((d) => toggleDrawer(d, "roles"))}
                        className={ico(strip >= 1)}
                      >
                        <IdentificationBadge size={13} />
                        {strip < 1 && t("orbitRoles")}
                      </button>
                    </StripTip>
                  </>
                )}
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
              </div>
              <div
                ref={stripRight}
                className={`pointer-events-auto ml-auto flex h-11 shrink-0 items-center gap-1.5 px-2.5 ${PLATE} ${PLATE_LINE} ${PLATE_ACRYLIC}`}
              >
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
                <StripTip
                  label={t("orbitFit")}
                  combo={`${IS_MAC ? "meta" : "ctrl"}+1`}
                  on={strip >= 1}
                >
                  <button
                    type="button"
                    aria-label={strip >= 1 ? t("orbitFit") : undefined}
                    title={strip >= 1 ? undefined : `${t("orbitFit")} · ${mod}1`}
                    onClick={() => void fitView({ padding: 0.15, maxZoom: 1, duration: 200 })}
                    className={ico(strip >= 1)}
                  >
                    <CornersOut size={13} />
                    {strip < 1 && t("orbitFit")}
                  </button>
                </StripTip>
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
            </div>
            {showVars && meta && (
              <VariablesPanel
                graphId={meta.id}
                variables={meta.variables}
                secrets={meta.secrets ?? []}
                stale={stale}
                nodeTitle={(id) => graph.nodes.find((n) => n.id === id)?.title ?? id}
                onChange={(variables, secrets) =>
                  setMeta((m) => (m ? { ...m, variables, secrets } : m))
                }
                onClose={() => setDrawer(null)}
              />
            )}
            {showInts && meta && (
              <IntegrationsPanel
                integrations={integrations}
                nodes={graph.nodes}
                onSaved={integrationSaved}
                onDeleted={(gone) => setIntegrations((list) => list.filter((i) => i.id !== gone))}
                onTest={testIntegration}
                onClose={() => setDrawer(null)}
              />
            )}
            {showRoles && meta && (
              <RolesPanel
                roles={roles}
                graphs={graphs}
                currentGraphId={meta.id}
                currentNodes={graph.nodes}
                start={showRoles}
                onSaved={(role) => {
                  roleSaved(role);
                  // "Novo papel…" do card: o papel salvo vai para aquele nó.
                  const assignTo = showRoles.assignTo;
                  if (assignTo) {
                    updateNode(assignTo, (node) =>
                      node.kind.type === "agent"
                        ? { ...node, kind: { ...node.kind, role: role.name } }
                        : node,
                    );
                    setDrawer({ kind: "roles", start: {} });
                  }
                }}
                onRenamed={roleRenamed}
                onDeleted={(gone) => setRoles((list) => list.filter((r) => r.name !== gone))}
                onClose={() => setDrawer(null)}
              />
            )}
          </div>
          {opened && (
            <SubagentPanel
              sessionId={opened.sessionId}
              run={opened.run}
              onClose={() => setOpenSub(null)}
            />
          )}
          {askDelete && meta && (
            <ConfirmDelete
              title={t("orbitDeleteGraphTitle", { name: meta.name || t("orbitGraphUntitled") })}
              body={t("orbitDeleteGraphBody")}
              onConfirm={removeGraph}
              onClose={() => setAskDelete(false)}
            />
          )}
          {notice?.undo ? (
            // Quadro OrbitManuseio 4: a pílula embaixo, com o desfazer do ⌘Z.
            <div
              role="status"
              className="absolute bottom-3 left-1/2 z-20 flex max-w-[640px] -translate-x-1/2 items-center gap-3 rounded-full border border-tyba-text/[.12] bg-tyba-surface py-1.5 pr-2 pl-4 text-[12px] whitespace-nowrap text-tyba-text shadow-[var(--tyba-shadow-md)]"
            >
              <span className="truncate">{notice.text}</span>
              <button
                type="button"
                onClick={undo}
                className="shrink-0 rounded-full bg-tyba-text/[.08] px-3 py-[5px] text-[11.5px] text-tyba-text hover:bg-tyba-text/[.12]"
              >
                {t("orbitUndoKey", { mod })}
              </button>
            </div>
          ) : notice && (
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
            onOpen={(nodeId) => {
              // "Abrir" na fila de atenção: o nó selecionado, enquadrado e
              // aberto (o mesmo do ir à sessão), para responder no prompt.
              setFocusNode(nodeId);
              setNodes((ns) => ns.map((n) => ({ ...n, selected: n.id === nodeId })));
              void fitView({ nodes: [{ id: nodeId }], padding: 0.2, maxZoom: 1, duration: 200 });
            }}
          />
        )}
      </div>
      <TerminalFolderPrompt
        open={askTerminal}
        home={home ?? ""}
        onChoose={(dir) => void createTerminal(dir)}
        onClose={() => setAskTerminal(false)}
      />
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
 *
 * Secreta (cadeado): o valor vai ao Keychain pelo core e a linha fica com o
 * valor vazio; o campo mostra a máscara e o olho revela só enquanto
 * pressionado, sem guardar o valor aqui.
 */
type VarRow = { name: string; value: string; secret: boolean };

function VariablesPanel({
  graphId,
  variables,
  secrets,
  stale,
  nodeTitle,
  onChange,
  onClose,
}: {
  graphId: string;
  variables: Record<string, string>;
  secrets: string[];
  /** Terminais ao vivo com variável mudada depois que abriram (do core). */
  stale: { nodes: string[]; vars: string[] };
  nodeTitle: (id: string) => string;
  onChange: (variables: Record<string, string>, secrets: string[]) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [rows, setRows] = useState<VarRow[]>(() =>
    Object.entries(variables).map(([name, value]) => ({
      name,
      value,
      secret: secrets.includes(name),
    })),
  );
  // Aviso de nome que não vira `$NOME`: a regra é do core.
  const namesKey = JSON.stringify(rows.map((r) => r.name));
  const [envProblems, setEnvProblems] = useState<VarEnvProblem[]>([]);
  useEffect(() => {
    let live = true;
    varEnvProblems(JSON.parse(namesKey) as string[])
      .then((p) => live && setEnvProblems(p))
      .catch(() => live && setEnvProblems([]));
    return () => {
      live = false;
    };
  }, [namesKey]);
  const [canLock, setCanLock] = useState(false);
  useEffect(() => {
    secretsSupported()
      .then(setCanLock)
      .catch(() => setCanLock(false));
  }, []);
  const [lockError, setLockError] = useState<string | null>(null);
  const commit = (next: VarRow[]) => {
    setRows(next);
    const named = next.filter((r) => r.name.trim() !== "");
    onChange(
      Object.fromEntries(named.map((r) => [r.name, r.secret ? "" : r.value])),
      named.filter((r) => r.secret).map((r) => r.name),
    );
  };
  const update = (i: number, patch: Partial<VarRow>) =>
    commit(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  // Trancar leva o valor digitado ao Keychain; destrancar traz o valor de volta
  // como texto comum (o core apaga o do Keychain ao salvar). Se o Keychain
  // não entrega (acesso negado), a variável continua trancada: nunca perde o
  // valor em silêncio.
  const toggleSecret = (i: number) => {
    const row = rows[i];
    if (row.secret) {
      setLockError(null);
      revealSecret(graphId, row.name)
        .then((value) => update(i, { secret: false, value: value ?? "" }))
        .catch((e) => setLockError(String(e)));
      return;
    }
    if (!canLock || row.name.trim() === "") return;
    setLockError(null);
    const stored = row.value === "" ? Promise.resolve() : setSecret(graphId, row.name, row.value);
    stored
      .then(() => update(i, { secret: true, value: "" }))
      .catch((e) => setLockError(String(e)));
  };
  const field =
    "h-7 min-w-0 rounded-[4px] border border-tyba-border bg-tyba-text/[.02] px-2 text-[12px] text-tyba-text outline-none focus:border-tyba-border-strong";
  const vbtn = "rounded-[4px] p-1 text-tyba-text-faint hover:text-tyba-text";

  return (
    <aside
      data-orbit-sheet
      role="dialog"
      aria-label={t("orbitVariables")}
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
      }}
      className={`${DRAWER} max-h-[calc(100%-var(--orbit-strip-bottom,64px)-12px)] w-[400px] gap-2 p-3`}
    >
      <div className="flex items-center justify-between">
        <span className="text-[13px] font-medium text-tyba-text">{t("orbitVariables")}</span>
        <button type="button" aria-label={t("close")} onClick={onClose} className={vbtn}>
          <X size={13} />
        </button>
      </div>
      <p className="text-[11.5px] leading-[17px] text-tyba-text-faint">
        <Trans
          i18nKey="orbitVarsHint"
          values={{ example: "{{nome}}", env: "$NOME" }}
          components={{ m: <span className="font-mono text-tyba-text-muted" /> }}
        />
      </p>
      {stale.nodes.length > 0 && (
        // O2: peça neutra com contorno inteiro e o ↻ âmbar; sem botão de
        // reabrir — parar o terminal ao vivo é do dono (■ no nó).
        <div
          role="status"
          className="grid shrink-0 grid-cols-[auto_1fr] gap-x-2.5 gap-y-1.5 rounded-[8px] border border-tyba-border-field bg-tyba-text/[.02] px-[11px] py-2.5 text-[11.5px] leading-[17px] text-tyba-text-muted"
        >
          <span className="row-span-2 flex size-[22px] items-center justify-center rounded-[6px] bg-tyba-amber-tint text-tyba-amber">
            <ArrowClockwise size={13} />
          </span>
          <span>{t("orbitVarsReapply")}</span>
          <span className="flex flex-wrap gap-1.5">
            {stale.nodes.map((id) => (
              <span
                key={id}
                className="inline-flex h-[22px] items-center gap-1.5 rounded-[5px] px-2 text-[11px] whitespace-nowrap text-tyba-text-muted shadow-[inset_0_0_0_1px_var(--tyba-border-field)]"
              >
                <i className="size-[7px] shrink-0 rounded-full bg-tyba-green-fill shadow-[0_0_8px_color-mix(in_srgb,var(--tyba-green-fill)_50%,transparent)]" />
                {nodeTitle(id)}
              </span>
            ))}
          </span>
        </div>
      )}
      <div className="flex gap-1.5 pr-[54px] text-[10.5px] tracking-[.08em] text-tyba-text-faint">
        <span className="min-w-0 flex-1">{t("orbitVarNameHead")}</span>
        <span className="min-w-0 flex-[1.25]">{t("orbitVarValueHead")}</span>
      </div>
      <div className="flex min-h-0 flex-col gap-1.5 overflow-y-auto">
        {rows.map((row, i) => (
          <div key={i} className="flex flex-col gap-1">
            <div className="flex items-center gap-1.5">
              <span
                className={`${field} flex flex-1 items-center gap-1 font-mono focus-within:border-tyba-border-strong`}
              >
                <input
                  aria-label={t("orbitVarName")}
                  placeholder={t("orbitVarName")}
                  spellCheck={false}
                  autoCorrect="off"
                  autoCapitalize="off"
                  value={row.name}
                  readOnly={row.secret}
                  title={row.secret ? t("orbitVarSecretRename") : undefined}
                  onChange={(e) => update(i, { name: e.target.value })}
                  className="h-full min-w-0 flex-1 bg-transparent outline-none"
                />
                {row.name.trim() !== "" && stale.vars.includes(row.name) && (
                  <span title={t("orbitVarChanged")} className="shrink-0 text-[12px] text-tyba-amber">
                    ↻
                  </span>
                )}
              </span>
              {row.secret ? (
                <SecretValue graphId={graphId} name={row.name} field={field} />
              ) : (
                <input
                  aria-label={t("orbitVarValue")}
                  placeholder={t("orbitVarValue")}
                  spellCheck={false}
                  autoCorrect="off"
                  autoCapitalize="off"
                  value={row.value}
                  onChange={(e) => update(i, { value: e.target.value })}
                  className={`${field} flex-[1.25] font-mono`}
                />
              )}
              <button
                type="button"
                aria-pressed={row.secret}
                aria-label={row.secret ? t("orbitVarSecret") : t("orbitVarMakeSecret")}
                title={
                  row.secret
                    ? t("orbitVarSecret")
                    : canLock
                      ? t("orbitVarMakeSecret")
                      : t("orbitVarSecretUnsupported")
                }
                disabled={!row.secret && (!canLock || row.name.trim() === "")}
                onClick={() => toggleSecret(i)}
                className={`${vbtn} disabled:cursor-not-allowed disabled:opacity-50 ${row.secret ? "text-tyba-amber hover:text-tyba-amber" : ""}`}
              >
                {row.secret ? <Lock size={14} /> : <LockOpen size={14} />}
              </button>
              <button
                type="button"
                aria-label={t("orbitVarRemove")}
                onClick={() => commit(rows.filter((_, j) => j !== i))}
                className="rounded-[4px] p-1 text-tyba-text-faint hover:text-tyba-red"
              >
                <X size={12} />
              </button>
            </div>
            {row.secret && (
              <p className="flex items-center gap-1.5 text-[11px] leading-[15px] text-tyba-text-faint">
                <Lock size={11} className="shrink-0 text-tyba-amber" />
                {t("orbitVarSecretNote")}
              </p>
            )}
            {row.name.trim() !== "" && envProblems[i] && (
              <p className="text-[11px] leading-[15px] text-tyba-amber">
                {t(
                  envProblems[i] === "invalid" ? "orbitVarEnvInvalid" : "orbitVarEnvReserved",
                  { env: `$${row.name}` },
                )}
              </p>
            )}
          </div>
        ))}
      </div>
      {lockError && <p className="text-[11px] leading-[15px] text-tyba-red">{lockError}</p>}
      <button
        type="button"
        onClick={() => setRows([...rows, { name: "", value: "", secret: false }])}
        className="inline-flex h-7 items-center gap-1.5 self-start rounded-[6px] px-2 text-[12px] text-tyba-text-muted hover:bg-tyba-raised hover:text-tyba-text"
      >
        <Plus size={12} />
        {t("orbitVarAdd")}
      </button>
    </aside>
  );
}

/**
 * Valor de uma secreta: a máscara, o olho que revela só enquanto pressionado
 * (o valor vem do core a cada vez e some ao soltar) e, ao clicar na máscara,
 * um campo de senha vazio para trocar o valor.
 */
function SecretValue({ graphId, name, field }: { graphId: string; name: string; field: string }) {
  const { t } = useTranslation();
  const [shown, setShown] = useState<string | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const holding = useRef(false);
  const press = () => {
    holding.current = true;
    revealSecret(graphId, name)
      .then((v) => holding.current && setShown(v ?? ""))
      .catch(() => setShown(null));
  };
  const release = () => {
    holding.current = false;
    setShown(null);
  };
  const save = () => {
    const value = draft;
    setDraft(null);
    if (value) void setSecret(graphId, name, value).catch(() => {});
  };
  if (draft !== null) {
    return (
      <input
        type="password"
        autoFocus
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        aria-label={t("orbitVarValue")}
        placeholder={t("orbitVarSecretNew")}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={save}
        onKeyDown={(e) => {
          if (e.key === "Enter") save();
          if (e.key === "Escape") {
            // Só desiste da troca: a gaveta fica.
            e.stopPropagation();
            setDraft(null);
          }
        }}
        className={`${field} flex-[1.25] font-mono`}
      />
    );
  }
  return (
    <span className={`${field} flex flex-[1.25] items-center justify-between gap-1 font-mono`}>
      <button
        type="button"
        aria-label={t("orbitVarSecretEdit")}
        title={t("orbitVarSecretEdit")}
        onClick={() => setDraft("")}
        className="min-w-0 flex-1 overflow-hidden text-left whitespace-nowrap"
      >
        {shown ?? <span className="tracking-[.12em]">••••••••••••</span>}
      </button>
      <button
        type="button"
        aria-label={t("orbitVarSecretReveal")}
        title={t("orbitVarSecretReveal")}
        onPointerDown={press}
        onPointerUp={release}
        onPointerLeave={release}
        onPointerCancel={release}
        onBlur={release}
        className="flex shrink-0 text-tyba-text-faint hover:text-tyba-text"
      >
        <Eye size={14} />
      </button>
    </span>
  );
}
