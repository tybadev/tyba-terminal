import {
  Fragment,
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { ArrowsOut, DotsThree, FolderSimple, Trash } from "@phosphor-icons/react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  Handle,
  NodeResizer,
  Position,
  useConnection,
  useReactFlow,
  useUpdateNodeInternals,
  type Node,
} from "@xyflow/react";

import {
  NEW_IN_HANDLE,
  OUT_HANDLE,
  accepts,
  formatDuration,
  nodeElapsed,
  nodeSession,
  nodeStatusLabel,
  pickConnection,
  produces,
  nodeDir,
  rememberDir,
  shortDir,
  type EdgeKind,
  type FlowNodeData,
  type Graph,
  type Integration,
  type MaestroNode,
  type NodeRun,
  type NodeStatus,
  type Run,
} from "@/lib/maestro";
import type { ApprovalDecision, ApprovalRequest, SubagentRun } from "@/lib/ipc";
import { ringState, type RingState } from "@/lib/maestroCard";
import { decideApproval } from "@/lib/notifications";

export type MaestroFlowNode = Node<FlowNodeData>;

/**
 * O que os nós precisam da tela, em três partes (canvas-7): as ações e o que
 * quase nunca muda (estável), a execução à vista (reativa, estado que veio do
 * core, só desenhado aqui) e o relógio (só o badge de tempo lê). Arrastar um
 * nó não muda nenhuma delas, e o tique de 1 s só redesenha quem mostra tempo.
 */
export interface CanvasActions {
  integrations: ReadonlyMap<string, Integration>;
  updateNode: (id: string, update: (node: MaestroNode) => MaestroNode) => void;
  /** Pasta padrão do grafo: vale para nó salvo antes da pasta por nó. */
  graphDir: string;
  runNode: (nodeId: string) => void;
  setOpenSubagent: (nodeId: string | null) => void;
  setFocusNode: (nodeId: string | null) => void;
  /** "Abrir no navegador" do processo: o navegador ligado nele, ou um novo. */
  openInBrowser: (fromId: string, url: string) => void;
  /** "Manter" na cápsula do criado: os criados do run vão para o grafo. */
  keepCreated: () => void;
  /** Permitir/Negar da cápsula âmbar: resolve o pedido no core. */
  answerApproval: (requestId: number, decision: ApprovalDecision) => void;
}

export interface CanvasState {
  graph: Pick<Graph, "nodes" | "edges">;
  run: Run | null;
  /** O run à vista é o mais novo: os terminais vivos são dele. */
  live: boolean;
  /** Zoom em 100%: o terminal do nó usa WebGL. Fora disso, renderer DOM. */
  actualSize: boolean;
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
}

export type MaestroCtx = CanvasActions & CanvasState;

export const CanvasActionsContext = createContext<CanvasActions>({
  integrations: new Map(),
  updateNode: () => {},
  graphDir: "",
  runNode: () => {},
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
  actualSize: true,
  subagents: new Map(),
  openSubagent: null,
  focusNode: null,
  createdIds: new Set(),
  createdBy: new Map(),
  approvals: new Map(),
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
export const useMaestro = (): MaestroCtx => ({
  ...useContext(CanvasActionsContext),
  ...useContext(CanvasStateContext),
});

/** Cor de cada tipo de ligação (quadro MaestroHarness). */
export const EDGE_COLOR: Record<EdgeKind, string> = {
  context: "var(--tyba-violet)",
  diff: "var(--tyba-green)",
  file: "var(--tyba-text-muted)",
  log: "var(--tyba-cyan)",
  json: "var(--tyba-blue)",
  url: "var(--tyba-cyan)",
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
  const { graph, run } = useMaestro();
  const now = useClock();
  const label = run ? nodeStatusLabel(graph, run, nodeId) : null;
  const elapsed = nodeElapsed(nodeRun, now);
  const tone = STATUS_TONE[nodeRun.status];
  return (
    <span
      className={`flex shrink-0 items-center gap-1.5 text-[11.5px] whitespace-nowrap ${tone.text}`}
      title={nodeRun.reason ?? undefined}
    >
      {label &&
        t(label.key, {
          ...label.params,
          ...(label.params?.kind ? { kind: t(`maestroEdge_${label.params.kind}`) } : {}),
        })}
      {elapsed !== null && <span className="text-tyba-text-faint">{formatDuration(elapsed)}</span>}
      <i className={`size-[7px] shrink-0 rounded-full ${tone.dot}`} />
    </span>
  );
}

/** Cor da porta de saída: a do que o nó produz primeiro (a saída principal). */
const portStyle = (color: string) => ({
  width: 16,
  height: 16,
  right: -8,
  top: 12,
  borderRadius: "50%",
  border: "3px solid var(--tyba-bg)",
  background: color,
  transform: "none",
});

/** A entrada não aparece: o nó inteiro é alvo, e ela só ancora a ligação. */
const inStyle = {
  width: 16,
  height: 16,
  left: -8,
  top: 12,
  opacity: 0,
  border: 0,
  transform: "none",
};

/** Estado do anel → traço (quadro MaestroCardsD). */
const RING_STROKE: Record<RingState, { stroke: string; dash?: string }> = {
  idle: { stroke: "color-mix(in srgb, var(--tyba-text) 10%, transparent)" },
  working: { stroke: "color-mix(in srgb, var(--tyba-text) 8%, transparent)" },
  live: { stroke: "var(--tyba-green)" },
  done: { stroke: "var(--tyba-green)" },
  waiting: { stroke: "color-mix(in srgb, var(--tyba-text) 18%, transparent)", dash: "3 3" },
  owner: { stroke: "var(--tyba-text-inverse)", dash: "4 3" },
  failed: { stroke: "var(--tyba-red)" },
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
  state: RingState;
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
function Elapsed({ nodeRun, className }: { nodeRun: NodeRun; className?: string }) {
  const now = useClock();
  const elapsed = nodeElapsed(nodeRun, now);
  return elapsed === null ? null : <span className={className}>{formatDuration(elapsed)}</span>;
}

/**
 * O estado na cápsula: rodando, só o tempo (o anel já diz que trabalha);
 * nos outros estados, a palavra (aguardando contexto de X, falhou, pronto).
 */
function CapsuleStatus({ nodeId, nodeRun }: { nodeId: string; nodeRun: NodeRun }) {
  const { t } = useTranslation();
  const { graph, run } = useMaestro();
  if (nodeRun.status === "running" && !nodeRun.orchestrating) {
    // No ar, quem fala é a porta (a ação do nó) e o anel verde.
    if (nodeRun.url) return null;
    return <Elapsed nodeRun={nodeRun} className="shrink-0 text-[11.5px] text-tyba-green" />;
  }
  const label = run ? nodeStatusLabel(graph, run, nodeId) : null;
  if (!label) return null;
  return (
    <span
      className={`min-w-0 shrink truncate text-[11.5px] ${STATUS_TONE[nodeRun.status].text}`}
      title={nodeRun.reason ?? undefined}
    >
      {t(label.key, {
        ...label.params,
        ...(label.params?.kind ? { kind: t(`maestroEdge_${label.params.kind}`) } : {}),
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
        {armed === request.id ? t("maestroApproveConfirm") : t("maestroApprove")}
      </button>
      <button
        type="button"
        onClick={() => decide("denied")}
        className={`${capsuleBtn} bg-[color-mix(in_srgb,var(--tyba-text-inverse)_18%,transparent)] text-tyba-text-inverse`}
      >
        {t("maestroDeny")}
      </button>
    </>
  );
}

/**
 * Moldura comum dos nós (quadro MaestroCardsD, "ilha"): a cápsula do
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
  paper = false,
  children,
}: {
  id: string;
  node: MaestroNode;
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
  /** Nota: tom de papel. */
  paper?: boolean;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const {
    integrations,
    updateNode,
    setFocusNode,
    keepCreated,
    graph,
    run,
    live,
    createdIds,
    createdBy,
    approvals,
  } = useMaestro();
  const nodeRun = run?.nodes[id];
  // Criado por outro agente no run à vista: contorno tracejado violeta e
  // "criado pelo X". Enquanto efêmero, sem editar nem apagar, e "Manter".
  const creator = createdBy.get(id);
  const created = creator !== undefined;
  const locked = createdIds.has(id);
  const orchestrates = !created && node.kind.type === "agent" && node.kind.orchestrate === true;
  const session = nodeSession(node.kind, run, id, live);
  const request = approvals.get(id) ?? null;
  const ring = ringState(nodeRun, request !== null);
  const failed = ring === "failed";
  const [open, setOpen] = useState(false);
  const { deleteElements } = useReactFlow();
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

  // Porta nova (o nó passou a produzir/aceitar, o toggle do orquestrador)
  // só existe para o xyflow depois de medida de novo.
  const handleKey = `${outputs[0] ?? ""}|${takesInput}|${orchestrates}`;
  useEffect(() => {
    updateInternals(id);
  }, [id, handleKey, updateInternals]);

  const creatorTitle = creator ? graph.nodes.find((n) => n.id === creator)?.title : undefined;
  const failure =
    nodeRun?.status === "failed"
      ? [
          nodeRun.exitCode !== null ? t("maestroExitCode", { code: nodeRun.exitCode }) : "",
          nodeRun.reason ?? "",
        ]
          .filter(Boolean)
          .join(" · ")
      : "";

  const capsule = request
    ? "border-tyba-amber bg-tyba-amber text-tyba-text-inverse"
    : `${
        created
          ? "border-dashed border-tyba-violet/80"
          : failed
            ? "border-tyba-red/60"
            : "border-[color-mix(in_srgb,var(--tyba-text)_10%,transparent)]"
      } ${
        paper
          ? "bg-[color-mix(in_srgb,var(--tyba-violet)_7%,var(--tyba-surface))]"
          : "bg-[color-mix(in_srgb,var(--tyba-text)_5%,var(--tyba-surface))]"
      }`;
  const win = dropKind
    ? "border-tyba-violet shadow-[0_0_0_4px_color-mix(in_srgb,var(--tyba-violet)_22%,transparent)]"
    : selected
      ? "border-tyba-violet shadow-[0_0_0_1px_var(--tyba-violet)]"
      : request
        ? "border-tyba-amber/40"
        : created
          ? "border-dashed border-tyba-violet/50"
          : failed
            ? "border-tyba-red/35"
            : "border-tyba-border";

  return (
    <div className="group relative flex h-full w-full flex-col pt-5 font-mono text-tyba-text">
      <NodeResizer
        isVisible={selected}
        minWidth={260}
        minHeight={150}
        lineStyle={{ borderColor: "transparent" }}
        handleStyle={{
          width: 8,
          height: 8,
          borderRadius: 2,
          border: "1px solid var(--tyba-violet)",
          background: "var(--tyba-bg)",
        }}
      />
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
          title={t("maestroOutPort", {
            kinds: outputs.map((k) => t(`maestroEdge_${k}`)).join(", "),
          })}
          style={portStyle(EDGE_COLOR[outputs[0]])}
        />
      )}
      <div
        className={`absolute top-0 right-3.5 left-3.5 z-[1] flex h-10 items-center gap-2.5 rounded-full border pr-2.5 pl-1.5 shadow-[var(--tyba-shadow-md)] ${capsule}`}
        title={request ? request.command : undefined}
      >
        <NodeRing state={ring} tone={markTone}>
          {mark}
        </NodeRing>
        {request ? (
          <span className="min-w-0 flex-1 truncate font-sans text-[14px] font-semibold">
            {t("maestroNeedsYou", { title: node.title })}
          </span>
        ) : (
          <input
            aria-label={t("maestroNodeTitle")}
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            value={node.title}
            readOnly={locked}
            onChange={(e) => {
              const title = e.target.value;
              updateNode(id, (n) => ({ ...n, title }));
            }}
            className="nodrag min-w-0 flex-1 truncate bg-transparent font-sans text-[14px] font-semibold text-tyba-text outline-none"
          />
        )}
        {request ? (
          <ApprovalButtons request={request} />
        ) : (
          <>
            {status ?? (nodeRun && <CapsuleStatus nodeId={id} nodeRun={nodeRun} />)}
            {extra}
            {locked && (
              <button
                type="button"
                title={t("maestroKeepHint")}
                onClick={keepCreated}
                className="nodrag shrink-0 text-[11.5px] text-tyba-violet hover:underline"
              >
                {t("maestroKeep")}
              </button>
            )}
            <button
              type="button"
              aria-label={t("maestroNodeDetails")}
              aria-pressed={open}
              title={t("maestroNodeDetails")}
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
      </div>
      <div
        className={`relative flex min-h-0 flex-1 flex-col overflow-hidden rounded-[16px] border pt-6 ${
          paper
            ? "bg-[color-mix(in_srgb,var(--tyba-violet)_4%,var(--tyba-surface))]"
            : "bg-tyba-sunken"
        } ${win}`}
      >
        {request && (
          <div className="shrink-0 truncate px-3.5 pb-1.5 text-[11.5px] text-tyba-text">
            {request.command}
          </div>
        )}
        {children}
        {(failure || created || meta) && (
          <div
            className={`shrink-0 truncate px-3.5 pt-1 pb-2 text-[11px] ${
              failure ? "text-tyba-red" : created ? "text-tyba-violet" : "text-tyba-text-faint"
            }`}
          >
            {failure ||
              (created
                ? t("maestroCreatedByNode", { node: creatorTitle ?? t("maestroAnotherAgent") })
                : meta)}
          </div>
        )}
        {open && (
          <div className="nodrag nowheel absolute inset-0 overflow-y-auto bg-tyba-sunken px-4 pt-7 pb-3.5 text-[11px]">
            <div className="grid grid-cols-[auto_1fr_auto_1fr] gap-x-3 gap-y-1.5">
              {[
                ...details,
                ...(nodeRun
                  ? [
                      {
                        label: t("maestroDetailState"),
                        value: t(`maestroStatus_${nodeRun.status}`),
                      },
                      ...(nodeRun.startedAt !== null
                        ? [{ label: t("maestroDetailTime"), value: <Elapsed nodeRun={nodeRun} /> }]
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
            <div className="flex gap-2 pt-3">
              {session && (
                <button
                  type="button"
                  onClick={() => setFocusNode(id)}
                  className="inline-flex items-center gap-1.5 rounded-full bg-tyba-text/[.08] px-3 py-[3px] text-[11.5px] text-tyba-text hover:bg-tyba-text/[.12]"
                >
                  <ArrowsOut size={12} />
                  {t("maestroOpen")}
                </button>
              )}
              {!locked && (
                <button
                  type="button"
                  onClick={() => void deleteElements({ nodes: [{ id }] })}
                  className="inline-flex items-center gap-1.5 rounded-full bg-tyba-text/[.08] px-3 py-[3px] text-[11.5px] text-tyba-text hover:text-tyba-red"
                >
                  <Trash size={12} />
                  {t("maestroDeleteNode")}
                </button>
              )}
            </div>
          </div>
        )}
        {dropKind && (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center bg-[color-mix(in_srgb,var(--tyba-violet)_10%,transparent)]">
            <span className="rounded-full border border-tyba-violet/60 bg-tyba-surface px-3 py-1 text-[12px] text-tyba-text shadow-[var(--tyba-shadow-md)]">
              {t("maestroDropHere", { kind: t(`maestroEdge_${dropKind}`) })}
            </span>
          </div>
        )}
      </div>
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
export function NodeDir({ id, kind }: { id: string; kind: MaestroNode["kind"] }) {
  const { t } = useTranslation();
  const { updateNode, graphDir } = useCanvasActions();
  const dir = nodeDir(kind, graphDir);
  if (dir === null) return null;
  const pick = async () => {
    const chosen = await openDialog({
      directory: true,
      multiple: false,
      defaultPath: dir || undefined,
      title: t("maestroNodeDirPick"),
    }).catch(() => null);
    if (typeof chosen !== "string") return;
    rememberDir(chosen);
    updateNode(id, (n) => ("dir" in n.kind ? { ...n, kind: { ...n.kind, dir: chosen } } : n));
  };
  return (
    <button
      type="button"
      onClick={() => void pick()}
      title={dir ? t("maestroNodeDirTitle", { dir }) : t("maestroNodeDirPick")}
      aria-label={t("maestroNodeDirPick")}
      className={`nodrag inline-flex max-w-[45%] shrink-0 items-center gap-1 rounded-[4px] px-1 text-[11px] leading-[20px] hover:bg-tyba-raised hover:text-tyba-text ${
        dir ? "text-tyba-text-faint" : "text-tyba-amber"
      }`}
    >
      <FolderSimple size={12} className="shrink-0" />
      <span className="truncate">{dir ? shortDir(dir) : t("maestroNodeDirNone")}</span>
    </button>
  );
}
