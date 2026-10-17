import {
  Fragment,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowClockwise,
  ArrowsOut,
  Broom,
  DotsThree,
  FolderSimple,
  Stop,
  Trash,
} from "@phosphor-icons/react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  Handle,
  NodeResizeControl,
  Position,
  ResizeControlVariant,
  useConnection,
  useStore,
  useUpdateNodeInternals,
  type Node,
  type ResizeDragEvent,
} from "@xyflow/react";

import {
  NEW_IN_HANDLE,
  OUT_HANDLE,
  accepts,
  formatDuration,
  nodeControls,
  nodeElapsed,
  nodeSession,
  nodeStatusLabel,
  pickConnection,
  produces,
  runCreated,
  shownStatus,
  nodeDir,
  rememberDir,
  shortDir,
  type EdgeKind,
  type FlowNodeData,
  type Graph,
  type Integration,
  type OrbitNode,
  type NodeRun,
  type NodeStatus,
  type Run,
} from "@/lib/orbit";
import type { ApprovalDecision, ApprovalRequest, SubagentRun } from "@/lib/ipc";
import {
  approvalLine,
  failureFooter,
  ringState,
  type AgentTelemetry,
  type RingState,
} from "@/lib/orbitCard";
import { decideApproval } from "@/lib/notifications";
import { sizeLabel } from "@/lib/orbitHandling";

export type OrbitFlowNode = Node<FlowNodeData>;

/**
 * O que os nós precisam da tela, em três partes (canvas-7): as ações e o que
 * quase nunca muda (estável), a execução à vista (reativa, estado que veio do
 * core, só desenhado aqui) e o relógio (só o badge de tempo lê). Arrastar um
 * nó não muda nenhuma delas, e o tique de 1 s só redesenha quem mostra tempo.
 */
export interface CanvasActions {
  integrations: ReadonlyMap<string, Integration>;
  updateNode: (id: string, update: (node: OrbitNode) => OrbitNode) => void;
  /** Pasta padrão do grafo: vale para nó salvo antes da pasta por nó. */
  graphDir: string;
  /** Id do grafo à vista (o navegador nativo é por grafo + nó). */
  graphId: string;
  runNode: (nodeId: string) => void;
  /** Parar só este nó, sem apagar (o core mata o process group dele). */
  stopNode: (nodeId: string) => void;
  /** ↻ Reiniciar o que roda ou rodar de novo o que terminou/falhou. */
  restartNode: (nodeId: string) => void;
  setOpenSubagent: (nodeId: string | null) => void;
  setFocusNode: (nodeId: string | null) => void;
  /** "Abrir no navegador" do processo: o navegador ligado nele, ou um novo. */
  openInBrowser: (fromId: string, url: string) => void;
  /**
   * "Manter": o criado `nodeId` vai para o grafo (rodapé do criado), ou, com
   * `null`, todos os que `orchestrator` criou ("Manter tudo" no ⋯ dele).
   */
  keepCreated: (nodeId: string | null, orchestrator?: string) => void;
  /** Permitir/Negar da cápsula âmbar: resolve o pedido no core. */
  answerApproval: (requestId: number, decision: ApprovalDecision) => void;
}

export interface CanvasState {
  graph: Pick<Graph, "nodes" | "edges">;
  run: Run | null;
  /** O run à vista é o mais novo: os terminais vivos são dele. */
  live: boolean;
  /** Subagentes do claude de cada nó com sessão viva (por id de nó). */
  subagents: ReadonlyMap<string, SubagentRun[]>;
  /** Id do nó-filho aberto no painel, ou `null`. */
  openSubagent: string | null;
  /** Nó aberto no modo foco (terminal em tamanho real por cima do canvas), ou `null`. */
  focusNode: string | null;
  /** Nós que um orquestrador criou no run à vista (efêmeros, fora do grafo salvo). */
  createdIds: ReadonlySet<string>;
  /** Quem o orquestrador criou no run à vista (efêmero ou mantido) → criador. */
  createdBy: ReadonlyMap<string, string>;
  /** Pedido de aprovação pendente de cada nó (o mais antigo da sessão dele). */
  approvals: ReadonlyMap<string, ApprovalRequest>;
  /** O que a sessão de cada nó de agente reportou (modelo, contexto, diff). */
  telemetry: ReadonlyMap<string, AgentTelemetry>;
  /** "#N" do run à vista (0 sem run). */
  runNumber: number;
  /** Nota de plano → "#N" da execução mais nova que escreveu nela. */
  planWriters: ReadonlyMap<string, number>;
}

export type OrbitCtx = CanvasActions & CanvasState;

export const CanvasActionsContext = createContext<CanvasActions>({
  integrations: new Map(),
  updateNode: () => {},
  graphDir: "",
  graphId: "",
  runNode: () => {},
  stopNode: () => {},
  restartNode: () => {},
  setOpenSubagent: () => {},
  setFocusNode: () => {},
  openInBrowser: () => {},
  keepCreated: () => {},
  answerApproval: () => {},
});

export const CanvasStateContext = createContext<CanvasState>({
  graph: { nodes: [], edges: [] },
  run: null,
  live: false,
  subagents: new Map(),
  openSubagent: null,
  focusNode: null,
  createdIds: new Set(),
  createdBy: new Map(),
  approvals: new Map(),
  telemetry: new Map(),
  runNumber: 0,
  planWriters: new Map(),
});

/**
 * Apagar pela lixeira (quadro OrbitManuseio 4 e 5): o canvas decide se o nó
 * some na hora ou se a cápsula pergunta antes ("Parar e apagar X?"). Quem
 * está perguntando é estado de tela, efêmero.
 */
export interface NodeDelete {
  /** Nó cuja cápsula pergunta agora, ou `null`. */
  asking: string | null;
  request: (nodeId: string) => void;
  confirm: (nodeId: string) => void;
  cancel: () => void;
}

export const NodeDeleteContext = createContext<NodeDelete>({
  asking: null,
  request: () => {},
  confirm: () => {},
  cancel: () => {},
});

/** Relógio dos tempos (ms); só anda enquanto algo roda. */
export const ClockContext = createContext(0);

/** As três partes do contexto do canvas, de fora para dentro. */
export function CanvasProvider({
  actions,
  state,
  now,
  children,
}: {
  actions: CanvasActions;
  state: CanvasState;
  now: number;
  children: ReactNode;
}) {
  return (
    <CanvasActionsContext.Provider value={actions}>
      <CanvasStateContext.Provider value={state}>
        <ClockContext.Provider value={now}>{children}</ClockContext.Provider>
      </CanvasStateContext.Provider>
    </CanvasActionsContext.Provider>
  );
}

/** Só as ações: quem usa isto não redesenha quando a execução muda. */
export const useCanvasActions = () => useContext(CanvasActionsContext);
export const useClock = () => useContext(ClockContext);
export const useOrbit = (): OrbitCtx => ({
  ...useContext(CanvasActionsContext),
  ...useContext(CanvasStateContext),
});

/** Cor de cada tipo de ligação (quadro OrbitHarness). */
export const EDGE_COLOR: Record<EdgeKind, string> = {
  context: "var(--tyba-violet)",
  diff: "var(--tyba-green)",
  file: "var(--tyba-text-muted)",
  log: "var(--tyba-cyan)",
  json: "var(--tyba-blue)",
  url: "var(--tyba-cyan)",
  plan: "var(--tyba-violet)",
  control: "var(--tyba-amber)",
  // Quadro MaestroConversa: conversa âmbar, memória violeta.
  talk: "var(--tyba-amber)",
  memory: "var(--tyba-violet)",
};

/** Cor do texto e do ponto de cada estado (quadro: rodando verde, aguardando âmbar). */
export const STATUS_TONE: Record<NodeStatus, { text: string; dot: string }> = {
  waiting: { text: "text-tyba-amber", dot: "border-[1.5px] border-tyba-amber" },
  running: { text: "text-tyba-green", dot: "bg-tyba-green animate-pulse" },
  done: { text: "text-tyba-text-muted", dot: "bg-tyba-green" },
  failed: { text: "text-tyba-red", dot: "bg-tyba-red" },
  skipped: { text: "text-tyba-text-faint", dot: "border-[1.5px] border-tyba-text-faint" },
  stopped: { text: "text-tyba-text-faint", dot: "bg-tyba-text-faint" },
};

/** Estado e tempo do nó na execução à vista (cabeçalho do modo foco). */
export function NodeStatusBadge({ nodeId, nodeRun }: { nodeId: string; nodeRun: NodeRun }) {
  const { t } = useTranslation();
  const { graph, run } = useOrbit();
  const now = useClock();
  const label = run ? nodeStatusLabel(graph, run, nodeId) : null;
  const elapsed = nodeElapsed(nodeRun, now, run);
  const tone = STATUS_TONE[shownStatus(run, nodeRun)];
  return (
    <span
      className={`flex shrink-0 items-center gap-1.5 text-[11.5px] whitespace-nowrap ${tone.text}`}
      title={nodeRun.reason ?? undefined}
    >
      {label &&
        t(label.key, {
          ...label.params,
          ...(label.params?.kind ? { kind: t(`orbitEdge_${label.params.kind}`) } : {}),
        })}
      {elapsed !== null && <span className="text-tyba-text-faint">{formatDuration(elapsed)}</span>}
      <i className={`size-[7px] shrink-0 rounded-full ${tone.dot}`} />
    </span>
  );
}

/**
 * Cor da porta de saída: a do que o nó produz primeiro (a saída principal).
 * Centro na ponta direita da cápsula (inset de 14px, `right-3.5`), para a
 * ligação sair dela e não do vazio ao lado.
 */
const portStyle = (color: string) => ({
  width: 16,
  height: 16,
  right: 6,
  top: 12,
  borderRadius: "50%",
  border: "3px solid var(--orbit-canvas, var(--tyba-bg))",
  background: color,
  transform: "none",
  // Acima da cápsula (`z-[1]`), que agora fica embaixo da bolinha.
  zIndex: 2,
});

/**
 * A entrada não aparece: o nó inteiro é alvo, e ela só ancora a ligação —
 * na ponta esquerda da cápsula (`left-3.5`), onde a linha encosta.
 */
const inStyle = {
  width: 16,
  height: 16,
  left: 6,
  top: 12,
  opacity: 0,
  border: 0,
  transform: "none",
};

/** O anel do nó: o estado dele, ou o dono usando a página do navegador (quadro OrbitPlano). */
type FrameRing = RingState | "user";

/** Estado do anel → traço (quadro OrbitCardsD). */
const RING_STROKE: Record<FrameRing, { stroke: string; dash?: string }> = {
  idle: { stroke: "color-mix(in srgb, var(--tyba-text) 10%, transparent)" },
  working: { stroke: "color-mix(in srgb, var(--tyba-text) 8%, transparent)" },
  live: { stroke: "var(--tyba-green)" },
  done: { stroke: "var(--tyba-green)" },
  waiting: { stroke: "color-mix(in srgb, var(--tyba-text) 18%, transparent)", dash: "3 3" },
  owner: { stroke: "var(--tyba-text-inverse)", dash: "4 3" },
  failed: { stroke: "var(--tyba-red)" },
  user: { stroke: "var(--tyba-cyan)" },
};

/**
 * O anel da marca em volta do ícone do nó: gira nas cores da marca enquanto
 * trabalha, tracejado esperando, vermelho se falhou, verde cheio no ar.
 */
export function NodeRing({
  state,
  tone,
  children,
}: {
  state: FrameRing;
  /** Classe de cor do ícone (o `currentColor` do miolo). */
  tone: string;
  children: ReactNode;
}) {
  const ring = RING_STROKE[state];
  const owner = state === "owner";
  return (
    <span
      className={`relative flex size-[30px] shrink-0 items-center justify-center ${
        owner ? "text-tyba-amber" : tone
      }`}
    >
      <svg width="30" height="30" viewBox="0 0 30 30" aria-hidden className="absolute inset-0">
        <circle
          cx="15"
          cy="15"
          r="12"
          style={{
            fill: owner
              ? "var(--tyba-text-inverse)"
              : "color-mix(in srgb, currentColor 12%, var(--tyba-sunken))",
          }}
          stroke={ring.stroke}
          strokeWidth="2.5"
          strokeDasharray={ring.dash}
        />
      </svg>
      {state === "working" && (
        <svg
          width="30"
          height="30"
          viewBox="0 0 30 30"
          aria-hidden
          className="absolute inset-0 animate-[spin_1.6s_linear_infinite] motion-reduce:animate-none"
        >
          <path
            d="M15 3 A12 12 0 0 1 27 15"
            fill="none"
            stroke="var(--tyba-green)"
            strokeWidth="2.5"
            strokeLinecap="round"
          />
          <path
            d="M27 15 A12 12 0 0 1 19 26.3"
            fill="none"
            stroke="var(--tyba-cyan)"
            strokeWidth="2.5"
            strokeLinecap="round"
          />
        </svg>
      )}
      <span className="relative flex">{children}</span>
    </span>
  );
}

/** O tempo do nó (só ele lê o relógio). */
export function Elapsed({ nodeRun, className }: { nodeRun: NodeRun; className?: string }) {
  const now = useClock();
  const { run } = useOrbit();
  const elapsed = nodeElapsed(nodeRun, now, run);
  return elapsed === null ? null : <span className={className}>{formatDuration(elapsed)}</span>;
}

/**
 * O estado na cápsula: rodando, só o tempo (o anel já diz que trabalha);
 * nos outros estados, a palavra (aguardando contexto de X, falhou, pronto).
 */
function CapsuleStatus({ nodeId, nodeRun }: { nodeId: string; nodeRun: NodeRun }) {
  const { t } = useTranslation();
  const { graph, run } = useOrbit();
  if (nodeRun.status === "running" && !nodeRun.orchestrating) {
    // No ar, quem fala é a porta (a ação do nó) e o anel verde.
    if (nodeRun.url) return null;
    return <Elapsed nodeRun={nodeRun} className="shrink-0 text-[11.5px] text-tyba-green" />;
  }
  const label = run ? nodeStatusLabel(graph, run, nodeId) : null;
  if (!label) return null;
  return (
    <span
      className={`min-w-0 shrink truncate text-[11.5px] ${STATUS_TONE[shownStatus(run, nodeRun)].text}`}
      title={nodeRun.reason ?? undefined}
    >
      {t(label.key, {
        ...label.params,
        ...(label.params?.kind ? { kind: t(`orbitEdge_${label.params.kind}`) } : {}),
      })}
    </span>
  );
}

/** Uma linha da grade de detalhes do ⋯. */
export interface NodeDetail {
  label: string;
  value: ReactNode;
}

const capsuleBtn = "nodrag shrink-0 rounded-full px-3 py-[3px] text-[11.5px] leading-[18px]";

/** Botão da grade do ⋯ (quadro: pílula clara). */
const detailBtn =
  "inline-flex items-center gap-1.5 rounded-full bg-tyba-text/[.08] px-3 py-[3px] text-[11.5px]";

/**
 * Pedido do agente na cápsula âmbar: Permitir/Negar ali mesmo. Vermelho pede
 * dois cliques para permitir (a regra de `decideApproval`, a mesma do toast e
 * do painel da sessão); negar é sempre direto.
 */
function ApprovalButtons({ request }: { request: ApprovalRequest }) {
  const { t } = useTranslation();
  const { answerApproval } = useCanvasActions();
  const [armed, setArmed] = useState<number | null>(null);
  const decide = (decision: "approved" | "denied") => {
    const effect = decideApproval({ request, decision, confirmingId: armed });
    if (effect.type === "armRedConfirm") setArmed(request.id);
    else if (effect.type === "resolve") answerApproval(effect.requestId, effect.decision);
  };
  return (
    <>
      <button
        type="button"
        onClick={() => decide("approved")}
        className={`${capsuleBtn} bg-tyba-text-inverse text-tyba-amber`}
      >
        {armed === request.id ? t("orbitApproveConfirm") : t("orbitApprove")}
      </button>
      <button
        type="button"
        onClick={() => decide("denied")}
        className={`${capsuleBtn} bg-[color-mix(in_srgb,var(--tyba-text-inverse)_18%,transparent)] text-tyba-text-inverse`}
      >
        {t("orbitDeny")}
      </button>
    </>
  );
}

/**
 * O pedido embaixo da janela (quadro OrbitCardsD): o comando e "Codex ·
 * revisor · pede há 40s". Só esta linha lê o relógio.
 */
function ApprovalFooter({ node, request }: { node: OrbitNode; request: ApprovalRequest }) {
  const { t } = useTranslation();
  const now = useClock();
  const line = approvalLine(node.kind, request.requested_at_ms, now);
  return (
    <div className="shrink-0 px-3.5 pt-1 pb-2 text-[11.5px]">
      <div className="truncate text-tyba-text">{request.command}</div>
      <div className="truncate text-[11px] text-tyba-text-faint">
        {[line.who, t("orbitAsksAgo", { ago: line.ago })].filter(Boolean).join(" · ")}
      </div>
    </div>
  );
}

/** Botão de algo que o core ainda não faz: aparece no lugar do quadro, sem ação, com a dica. */
export const soonClass = "cursor-not-allowed text-tyba-text-ghost";

/** Os risquinhos do canto de baixo à direita (quadro OrbitManuseio 1 e 3). */
function Grip({ active }: { active: boolean }) {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden className="pointer-events-none">
      <path
        d="M13 5 L5 13 M13 9 L9 13"
        stroke={active ? "var(--tyba-violet)" : "var(--tyba-text-muted)"}
        strokeWidth={active ? 1.6 : 1.4}
        strokeLinecap="round"
      />
    </svg>
  );
}

const LINE_POSITIONS = ["top", "right", "bottom", "left"] as const;
const CORNER_POSITIONS = ["top-left", "top-right", "bottom-left", "bottom-right"] as const;

/**
 * As quatro bordas e os cantos redimensionam sempre, sem precisar selecionar
 * (alvo invisível de 8px nas bordas e 14px nos cantos); os risquinhos no
 * canto de baixo à direita aparecem no hover e no selecionado. Shift mantém a
 * proporção do começo do gesto. Mínimo 260 × 150.
 */
function FrameResize({ showGrip, resizing }: { showGrip: boolean; resizing: boolean }) {
  const [shift, setShift] = useState(false);
  const onResizeStart = useCallback((e: ResizeDragEvent) => {
    const source = e.sourceEvent as MouseEvent | undefined;
    setShift(source?.shiftKey === true);
  }, []);
  // Shift no meio do gesto também vale, enquanto ele dura.
  useEffect(() => {
    if (!resizing) return;
    const key = (e: KeyboardEvent) => e.key === "Shift" && setShift(e.type === "keydown");
    window.addEventListener("keydown", key);
    window.addEventListener("keyup", key);
    return () => {
      window.removeEventListener("keydown", key);
      window.removeEventListener("keyup", key);
    };
  }, [resizing]);
  const common = {
    minWidth: 260,
    minHeight: 150,
    keepAspectRatio: shift,
    onResizeStart,
  };
  return (
    <>
      {LINE_POSITIONS.map((p) => (
        <NodeResizeControl
          key={p}
          {...common}
          position={p}
          variant={ResizeControlVariant.Line}
          // A de cima fica sob a cápsula (que arrasta); as outras, sobre a janela.
          className={p === "top" ? "z-0 !h-2" : p === "bottom" ? "z-[2] !h-2" : "z-[2] !w-2"}
          style={{ borderColor: "transparent" }}
        />
      ))}
      {CORNER_POSITIONS.map((p) => (
        <NodeResizeControl
          key={p}
          {...common}
          position={p}
          className="z-[2]"
          style={{ width: 14, height: 14, border: "none", background: "transparent" }}
        />
      ))}
      <NodeResizeControl
        {...common}
        position="bottom-right"
        autoScale={false}
        className={`z-[2] ${showGrip ? "opacity-100" : "opacity-0 group-hover:opacity-100"}`}
        style={{
          left: "auto",
          top: "auto",
          right: 6,
          bottom: 6,
          translate: "none",
          width: 14,
          height: 14,
          border: "none",
          borderRadius: 0,
          background: "transparent",
        }}
      >
        <Grip active={resizing} />
      </NodeResizeControl>
    </>
  );
}

/**
 * O nome na cápsula é texto (arrastar por ele move o card); duplo clique
 * abre o campo (quadro OrbitManuseio 6): Enter e sair do campo salvam, Esc
 * volta o nome antigo.
 */
function CapsuleTitle({
  id,
  title,
  locked,
  editing,
  setEditing,
}: {
  id: string;
  title: string;
  locked: boolean;
  editing: boolean;
  setEditing: (on: boolean) => void;
}) {
  const { t } = useTranslation();
  const { updateNode } = useCanvasActions();
  const [draft, setDraft] = useState(title);
  // Esc fecha o campo sem salvar: o blur que vem depois não salva.
  const closed = useRef(false);
  const save = () => {
    if (closed.current) return;
    closed.current = true;
    setEditing(false);
    if (draft !== title) updateNode(id, (n) => ({ ...n, title: draft }));
  };
  if (!editing) {
    return (
      <span
        onDoubleClick={() => {
          if (locked) return;
          closed.current = false;
          setDraft(title);
          setEditing(true);
        }}
        className="min-w-0 flex-1 truncate font-sans text-[14px] font-semibold text-tyba-text"
      >
        {title}
      </span>
    );
  }
  return (
    <input
      aria-label={t("orbitNodeTitle")}
      autoFocus
      spellCheck={false}
      autoCorrect="off"
      autoCapitalize="off"
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onFocus={(e) => e.target.select()}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          e.stopPropagation();
          save();
        } else if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          closed.current = true;
          setEditing(false);
        }
      }}
      onBlur={save}
      className="nodrag min-w-0 flex-1 rounded-[6px] bg-tyba-sunken px-2 py-[3px] font-sans text-[14px] font-semibold text-tyba-text caret-tyba-violet outline outline-1 outline-tyba-violet/60"
    />
  );
}

/**
 * Moldura comum dos nós (quadro OrbitCardsD, "ilha"): a cápsula do
 * cabeçalho flutua sobre a janela sem moldura. Na cápsula, o anel da marca
 * com o estado, o nome, o estado curto, a ação do nó e o ⋯ (a grade de
 * detalhes). Cápsula inteira âmbar quando o agente pede permissão, com
 * Permitir/Negar nela. Uma porta de saída na borda direita, na cor do que o
 * nó produz; o nó inteiro é alvo — ao arrastar uma ligação, o nó que aceita
 * acende e diz o tipo que ela vai ter.
 */
export function NodeFrame({
  id,
  node,
  selected,
  mark,
  markTone,
  extra,
  status,
  meta,
  details = [],
  stateRows = true,
  paper = false,
  inUse = false,
  clearData,
  children,
}: {
  id: string;
  node: OrbitNode;
  selected: boolean;
  mark: ReactNode;
  /** Cor do ícone (classe `text-*`). */
  markTone: string;
  /** Ação do nó na cápsula (▶ Rodar, a porta no ar). */
  extra?: ReactNode;
  /** Toma o lugar do estado do nó na cápsula (subagentes trabalhando). */
  status?: ReactNode;
  /** Rodapé discreto: pasta, porta, comando. */
  meta?: ReactNode;
  /** Grade do ⋯, além do estado e do tempo. */
  details?: NodeDetail[];
  /** A grade ganha o estado e o tempo no fim; o agente põe o tempo na ordem do quadro. */
  stateRows?: boolean;
  /** Nota: tom de papel. */
  paper?: boolean;
  /** O dono está usando a página do navegador: cápsula e janela em ciano. */
  inUse?: boolean;
  /** Navegador nativo: "Apagar dados do navegador" no ⋯, confirmado na cápsula. */
  clearData?: () => void;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const {
    integrations,
    setFocusNode,
    keepCreated,
    stopNode,
    restartNode,
    graph,
    run,
    live,
    createdIds,
    createdBy,
    approvals,
  } = useOrbit();
  const nodeRun = run?.nodes[id];
  // Criado por outro agente no run à vista: contorno tracejado violeta e
  // "criado pelo X". Enquanto efêmero, sem editar nem ligar, e "Manter";
  // a lixeira descarta (some da vista do run).
  const creator = createdBy.get(id);
  const created = creator !== undefined;
  const locked = createdIds.has(id);
  const orchestrates = !created && node.kind.type === "agent";
  const session = nodeSession(node.kind, run, id, live);
  const controls = nodeControls(node.kind, run, id, live);
  const again = controls.restart || controls.rerun;
  const request = approvals.get(id) ?? null;
  const ring = ringState(nodeRun, request !== null, run?.settled);
  const failed = ring === "failed";
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const del = useContext(NodeDeleteContext);
  const deleting = del.asking === id;
  // Confirmar "Apagar dados do navegador": o mesmo jeito do apagar nó.
  const [clearing, setClearing] = useState(false);
  const asking = deleting || clearing;
  // Arrastando e redimensionando (quadro OrbitManuseio 2 e 3): só mudam no
  // começo e no fim do gesto; o tamanho da cápsula "L × A", a cada passo.
  const dragging = useStore((s) => s.nodeLookup.get(id)?.dragging === true);
  const resizeSize = useStore((s) => {
    const n = s.nodeLookup.get(id);
    return n?.resizing ? sizeLabel(n.measured.width ?? 0, n.measured.height ?? 0) : null;
  });
  const resizing = resizeSize !== null;
  const updateInternals = useUpdateNodeInternals();

  const outputs = produces(node.kind, integrations);
  const takesInput = !locked && accepts(node.kind, integrations).length > 0;

  // Arrastando uma ligação: este nó aceita? Então acende com o tipo que ela
  // vai ter (o mesmo cálculo do soltar).
  const from = useConnection((c) => (c.inProgress ? c.fromNode.id : null));
  const dropKind = useMemo(() => {
    if (!from || from === id || !takesInput || createdIds.has(from)) return null;
    const pick = pickConnection(graph.nodes, graph.edges, integrations, from, id);
    return "kind" in pick ? pick.kind : null;
  }, [from, id, takesInput, createdIds, graph, integrations]);

  // Porta nova (o nó passou a produzir/aceitar, o agente deixou de ser criado)
  // só existe para o xyflow depois de medida de novo.
  const handleKey = `${outputs[0] ?? ""}|${takesInput}|${orchestrates}`;
  useEffect(() => {
    updateInternals(id);
  }, [id, handleKey, updateInternals]);

  const creatorTitle = creator ? graph.nodes.find((n) => n.id === creator)?.title : undefined;
  const fail = failureFooter(node.kind, nodeRun);
  const failure = fail
    ? [
        fail.exitCode !== null ? t("orbitExitCode", { code: fail.exitCode }) : "",
        fail.reason,
      ]
        .filter(Boolean)
        .join(" · ")
    : "";
  const agent = node.kind.type === "agent";
  // "Manter tudo" no ⋯ de quem criou: vale também depois do fim do run.
  const keepAll = runCreated(run).some((c) => !c.kept && c.createdBy === id);

  const capsule = request
    ? "border-tyba-amber bg-tyba-amber text-tyba-text-inverse"
    : `${
        asking
          ? "border-tyba-red/70"
          : dragging || editing
            ? "border-tyba-violet/60"
            : created
              ? "border-dashed border-tyba-violet/80"
              : failed
                ? "border-tyba-red/60"
                : inUse
                  ? "border-tyba-cyan/60"
                  : "border-[color-mix(in_srgb,var(--tyba-text)_10%,transparent)] group-hover:border-[color-mix(in_srgb,var(--tyba-text)_18%,transparent)]"
      } ${
        paper
          ? "bg-[color-mix(in_srgb,var(--tyba-violet)_7%,var(--tyba-surface))]"
          : "bg-[color-mix(in_srgb,var(--tyba-text)_5%,var(--tyba-surface))]"
      }`;
  const win = dropKind
    ? "border-tyba-violet shadow-[0_0_0_4px_color-mix(in_srgb,var(--tyba-violet)_22%,transparent)]"
    : asking
      ? "border-tyba-red/35"
      : dragging
        ? "border-tyba-border shadow-[0_30px_70px_rgba(0,0,0,0.75)]"
        : selected || resizing
          ? "border-tyba-violet shadow-[0_0_0_1px_var(--tyba-violet)]"
          : request
            ? "border-tyba-amber/40"
            : created
              ? "border-dashed border-tyba-violet/50"
              : failed
                ? "border-tyba-red/35"
                : inUse
                  ? "border-tyba-cyan/50"
                  : "border-tyba-border group-hover:border-tyba-text/[.16]";

  return (
    <div
      className={`group relative flex h-full w-full flex-col pt-5 font-mono text-tyba-text ${
        dragging ? "rotate-[-0.6deg]" : ""
      }`}
    >
      <FrameResize showGrip={selected || resizing} resizing={resizing} />
      {/* A entrada existe sempre: as ligações dos criados (url, a tracejada
          do orquestrador) chegam nela mesmo onde o dono não liga nada. */}
      <Handle
        type="target"
        id={NEW_IN_HANDLE}
        position={Position.Left}
        isConnectable={takesInput}
        isConnectableStart={false}
        style={inStyle}
      />
      {outputs.length > 0 && (
        <Handle
          type="source"
          id={OUT_HANDLE}
          position={Position.Right}
          isConnectable={!locked}
          title={t("orbitOutPort", {
            kinds: outputs.map((k) => t(`orbitEdge_${k}`)).join(", "),
          })}
          style={portStyle(EDGE_COLOR[outputs[0]])}
        />
      )}
      {/* A cápsula inteira arrasta (menos os botões); o corpo é para usar. */}
      <div
        className={`absolute top-0 right-3.5 left-3.5 z-[1] flex h-10 items-center gap-2.5 rounded-full border pr-2.5 pl-1.5 ${
          dragging
            ? "cursor-grabbing shadow-[0_22px_50px_rgba(0,0,0,0.7)]"
            : "cursor-grab shadow-[var(--tyba-shadow-md)]"
        } ${capsule}`}
        title={request ? request.command : undefined}
      >
        <NodeRing
          state={asking ? "failed" : inUse && !request ? "user" : ring}
          tone={asking ? "text-tyba-red" : markTone}
        >
          {mark}
        </NodeRing>
        {request ? (
          <span className="min-w-0 flex-1 truncate font-sans text-[14px] font-semibold">
            {t("orbitNeedsYou", { title: node.title })}
          </span>
        ) : asking ? (
          <span className="min-w-0 flex-1 truncate font-sans text-[14px] font-semibold">
            {deleting
              ? t("orbitStopAndDelete", { title: node.title })
              : t("orbitBrowserClearDataAsk", { title: node.title })}
          </span>
        ) : (
          <CapsuleTitle
            id={id}
            title={node.title}
            locked={locked}
            editing={editing}
            setEditing={setEditing}
          />
        )}
        {request ? (
          <ApprovalButtons request={request} />
        ) : asking ? (
          <>
            <button
              type="button"
              onClick={() => {
                if (deleting) {
                  del.confirm(id);
                } else {
                  setClearing(false);
                  clearData?.();
                }
              }}
              className={`${capsuleBtn} bg-tyba-red text-tyba-text-inverse`}
            >
              {t("orbitDeleteConfirm")}
            </button>
            <button
              type="button"
              onClick={deleting ? del.cancel : () => setClearing(false)}
              className={`${capsuleBtn} bg-tyba-text/[.08] text-tyba-text`}
            >
              {t("orbitDeleteCancel")}
            </button>
          </>
        ) : (
          <>
            {status ?? (nodeRun && <CapsuleStatus nodeId={id} nodeRun={nodeRun} />)}
            {nodeRun?.stale && again && (
              <button
                type="button"
                title={t("orbitStaleHint")}
                onClick={() => restartNode(id)}
                className="nodrag shrink-0 truncate text-[11.5px] text-tyba-amber hover:underline"
              >
                {t("orbitStale")}
              </button>
            )}
            {extra}
            {!dragging && (
              <>
                <button
                  type="button"
                  aria-label={t("orbitTrash")}
                  title={t("orbitTrash")}
                  onClick={() => del.request(id)}
                  className="nodrag flex size-[26px] shrink-0 items-center justify-center rounded-full bg-tyba-text/[.06] text-tyba-text-muted opacity-0 group-hover:opacity-100 hover:text-tyba-text focus-visible:opacity-100"
                >
                  <Trash size={14} />
                </button>
                <button
                  type="button"
                  aria-label={t("orbitNodeDetails")}
                  aria-pressed={open}
                  title={t("orbitNodeDetails")}
                  onClick={() => setOpen((v) => !v)}
                  className={`nodrag flex shrink-0 items-center rounded-full px-1.5 py-0.5 ${
                    open
                      ? "bg-tyba-text/[.08] text-tyba-text"
                      : "text-tyba-text-faint hover:text-tyba-text"
                  }`}
                >
                  <DotsThree size={16} weight="bold" />
                </button>
              </>
            )}
          </>
        )}
      </div>
      <div
        className={`nodrag relative flex min-h-0 flex-1 flex-col overflow-hidden rounded-[16px] border pt-6 ${
          paper
            ? "bg-[color-mix(in_srgb,var(--tyba-violet)_4%,var(--tyba-surface))]"
            : "bg-tyba-sunken"
        } ${win}`}
      >
        {children}
        {asking ? (
          <div className="shrink-0 truncate px-3.5 pt-1 pb-2 text-[11px] text-tyba-red">
            {deleting ? t("orbitStopsBeforeLeaving") : t("orbitBrowserClearDataHint")}
          </div>
        ) : request ? (
          <ApprovalFooter node={node} request={request} />
        ) : (
          (fail || created || meta) && (
            <div
              className={`shrink-0 truncate px-3.5 pt-1 pb-2 text-[11px] ${
                fail ? "text-tyba-red" : created ? "text-tyba-violet" : "text-tyba-text-faint"
              }`}
            >
              {fail ? (
                <>
                  {failure}
                  {fail.log && (
                    <>
                      {failure && " · "}
                      <span
                        role="button"
                        aria-disabled="true"
                        title={t("orbitFullLogSoon")}
                        className="nodrag cursor-not-allowed text-tyba-text-muted"
                      >
                        {t("orbitFullLog")}
                      </span>
                    </>
                  )}
                </>
              ) : created ? (
                <>
                  {t("orbitCreatedByNode", { node: creatorTitle ?? t("orbitAnotherAgent") })}
                  {locked && (
                    <>
                      {" · "}
                      <button
                        type="button"
                        title={t("orbitKeepHint")}
                        onClick={() => keepCreated(id)}
                        className="nodrag text-tyba-text hover:underline"
                      >
                        {t("orbitKeep")}
                      </button>
                    </>
                  )}
                </>
              ) : (
                meta
              )}
            </div>
          )
        )}
        {open && (
          <div className="nodrag nowheel absolute inset-0 overflow-y-auto bg-tyba-sunken px-4 pt-7 pb-3.5 text-[11px]">
            <div className="grid grid-cols-[auto_1fr_auto_1fr] gap-x-3 gap-y-1.5">
              {[
                ...details,
                ...(nodeRun && stateRows
                  ? [
                      {
                        label: t("orbitDetailState"),
                        value: t(`orbitStatus_${nodeRun.status}`),
                      },
                      ...(nodeRun.startedAt !== null
                        ? [{ label: t("orbitDetailTime"), value: <Elapsed nodeRun={nodeRun} /> }]
                        : []),
                    ]
                  : []),
              ].map((d) => (
                <Fragment key={d.label}>
                  <span className="text-tyba-text-faint">{d.label}</span>
                  <span className="min-w-0 truncate text-tyba-text">{d.value || "—"}</span>
                </Fragment>
              ))}
            </div>
            <div className="flex flex-wrap gap-2 pt-3">
              {/* Quadro: ⤢ Abrir · ↻ Reiniciar · Trocar papel. O que o core
                  ainda não faz fica no lugar, sem ação, com a dica. */}
              {(session || agent) && (
                <button
                  type="button"
                  aria-disabled={session ? undefined : true}
                  title={session ? undefined : t("orbitOpenNoSession")}
                  onClick={() => session && setFocusNode(id)}
                  className={`${detailBtn} ${session ? "text-tyba-text hover:bg-tyba-text/[.12]" : soonClass}`}
                >
                  <ArrowsOut size={12} />
                  {t("orbitOpen")}
                </button>
              )}
              {(agent || again) && (
                <button
                  type="button"
                  aria-disabled={again ? undefined : true}
                  title={again ? undefined : t("orbitRestartNeedsRun")}
                  onClick={() => {
                    if (!again) return;
                    setOpen(false);
                    restartNode(id);
                  }}
                  className={`${detailBtn} ${again ? "text-tyba-text hover:bg-tyba-text/[.12]" : soonClass}`}
                >
                  <ArrowClockwise size={12} />
                  {t(controls.rerun ? "orbitRerun" : "orbitRestart")}
                </button>
              )}
              {controls.stop && (
                <button
                  type="button"
                  onClick={() => {
                    setOpen(false);
                    stopNode(id);
                  }}
                  className={`${detailBtn} text-tyba-text hover:bg-tyba-text/[.12]`}
                >
                  <Stop size={12} />
                  {t("orbitStopNode")}
                </button>
              )}
              {agent && (
                <button
                  type="button"
                  aria-disabled
                  title={t("orbitSwapRoleSoon")}
                  className={`${detailBtn} ${soonClass}`}
                >
                  {t("orbitSwapRole")}
                </button>
              )}
              {keepAll && (
                <button
                  type="button"
                  title={t("orbitKeepAllHint")}
                  onClick={() => keepCreated(null, id)}
                  className={`${detailBtn} text-tyba-text hover:bg-tyba-text/[.12]`}
                >
                  {t("orbitKeepAll")}
                </button>
              )}
              {clearData && (
                <button
                  type="button"
                  onClick={() => {
                    setOpen(false);
                    setClearing(true);
                  }}
                  className={`${detailBtn} text-tyba-text hover:text-tyba-red`}
                >
                  <Broom size={12} />
                  {t("orbitBrowserClearData")}
                </button>
              )}
              <button
                type="button"
                onClick={() => {
                  setOpen(false);
                  setClearing(false);
                  del.request(id);
                }}
                className={`${detailBtn} text-tyba-text hover:text-tyba-red`}
              >
                <Trash size={12} />
                {t("orbitDeleteNode")}
              </button>
            </div>
          </div>
        )}
        {dropKind && (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center bg-[color-mix(in_srgb,var(--tyba-violet)_10%,transparent)]">
            <span className="rounded-full border border-tyba-violet/60 bg-tyba-surface px-3 py-1 text-[12px] text-tyba-text shadow-[var(--tyba-shadow-md)]">
              {t("orbitDropHere", { kind: t(`orbitEdge_${dropKind}`) })}
            </span>
          </div>
        )}
      </div>
      {resizeSize && (
        <span className="pointer-events-none absolute right-[26px] -bottom-[26px] z-[2] rounded-full border border-tyba-violet/50 bg-tyba-surface px-[9px] py-0.5 text-[11px] whitespace-nowrap text-tyba-violet">
          {resizeSize}
        </span>
      )}
    </div>
  );
}

/** Área de texto no estilo do terminal do nó (o terminal real é o `NodeTerminal`). */
export const termClass =
  "nodrag nowheel min-h-0 w-full flex-1 resize-none bg-tyba-sunken px-3.5 py-2.5 font-mono text-[11.5px] leading-[17px] text-tyba-text-muted outline-none placeholder:text-tyba-text-ghost";

export const fieldClass =
  "nodrag h-6 min-w-0 rounded-[4px] border border-tyba-border bg-tyba-text/[.02] px-1.5 font-mono text-[11px] text-tyba-text outline-none focus:border-tyba-border-strong";

/**
 * A pasta onde o nó roda, encurtada. Clicar abre o seletor de pasta; a
 * escolhida vira a pasta do nó e a última usada (onde o próximo nó nasce).
 */
export function NodeDir({ id, kind }: { id: string; kind: OrbitNode["kind"] }) {
  const { t } = useTranslation();
  const { updateNode, graphDir } = useCanvasActions();
  const dir = nodeDir(kind, graphDir);
  if (dir === null) return null;
  const pick = async () => {
    const chosen = await openDialog({
      directory: true,
      multiple: false,
      defaultPath: dir || undefined,
      title: t("orbitNodeDirPick"),
    }).catch(() => null);
    if (typeof chosen !== "string") return;
    rememberDir(chosen);
    updateNode(id, (n) => ("dir" in n.kind ? { ...n, kind: { ...n.kind, dir: chosen } } : n));
  };
  return (
    <button
      type="button"
      onClick={() => void pick()}
      title={dir ? t("orbitNodeDirTitle", { dir }) : t("orbitNodeDirPick")}
      aria-label={t("orbitNodeDirPick")}
      className={`nodrag inline-flex max-w-[45%] shrink-0 items-center gap-1 rounded-[4px] px-1 text-[11px] leading-[20px] hover:bg-tyba-raised hover:text-tyba-text ${
        dir ? "text-tyba-text-faint" : "text-tyba-amber"
      }`}
    >
      <FolderSimple size={12} className="shrink-0" />
      <span className="truncate">{dir ? shortDir(dir) : t("orbitNodeDirNone")}</span>
    </button>
  );
}
