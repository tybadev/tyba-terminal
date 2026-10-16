import { createContext, useContext, useEffect, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { FolderSimple, Trash } from "@phosphor-icons/react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  Handle,
  NodeResizer,
  Position,
  useNodeConnections,
  useReactFlow,
  useUpdateNodeInternals,
  type Node,
} from "@xyflow/react";

import {
  NEW_IN_HANDLE,
  accepts,
  inHandle,
  kindOfOutHandle,
  outHandle,
  formatDuration,
  nodeElapsed,
  nodeStatusLabel,
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
import type { SubagentRun } from "@/lib/ipc";

export type MaestroFlowNode = Node<FlowNodeData>;

/**
 * O que os nós precisam da tela: integrações salvas, como mudar o próprio nó
 * e a execução à vista (estado que veio do core, só desenhado aqui).
 */
export interface MaestroCtx {
  integrations: ReadonlyMap<string, Integration>;
  updateNode: (id: string, update: (node: MaestroNode) => MaestroNode) => void;
  graph: Pick<Graph, "nodes" | "edges">;
  /** Pasta padrão do grafo: vale para nó salvo antes da pasta por nó. */
  graphDir: string;
  run: Run | null;
  /** O run à vista é o mais novo: os terminais vivos são dele. */
  live: boolean;
  now: number;
  /** Zoom em 100%: terminal real no nó. Fora disso, prévia em texto. */
  actualSize: boolean;
  runNode: (nodeId: string) => void;
  /** Subagentes do claude de cada nó com sessão viva (por id de nó). */
  subagents: ReadonlyMap<string, SubagentRun[]>;
  /** Id do nó-filho aberto no painel, ou `null`. */
  openSubagent: string | null;
  setOpenSubagent: (nodeId: string | null) => void;
  /** Nó aberto no modo foco (terminal em tamanho real por cima do canvas), ou `null`. */
  focusNode: string | null;
  setFocusNode: (nodeId: string | null) => void;
  /** "Abrir no navegador" do processo: o navegador ligado nele, ou um novo. */
  openInBrowser: (fromId: string, url: string) => void;
  /** Nós que um orquestrador criou no run à vista (efêmeros, fora do grafo salvo). */
  createdIds: ReadonlySet<string>;
  /** Quem o orquestrador criou no run à vista (efêmero ou mantido) → criador. */
  createdBy: ReadonlyMap<string, string>;
}

export const MaestroContext = createContext<MaestroCtx>({
  integrations: new Map(),
  updateNode: () => {},
  graph: { nodes: [], edges: [] },
  graphDir: "",
  run: null,
  live: false,
  now: 0,
  actualSize: true,
  runNode: () => {},
  subagents: new Map(),
  openSubagent: null,
  setOpenSubagent: () => {},
  focusNode: null,
  setFocusNode: () => {},
  openInBrowser: () => {},
  createdIds: new Set(),
  createdBy: new Map(),
});

export const useMaestro = () => useContext(MaestroContext);

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

/** Estado e tempo do nó na execução à vista. */
export function NodeStatusBadge({ nodeId, nodeRun }: { nodeId: string; nodeRun: NodeRun }) {
  const { t } = useTranslation();
  const { graph, run, now } = useMaestro();
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

const handleStyle = (color: string, filled: boolean) => ({
  width: 14,
  height: 14,
  borderRadius: "50%",
  border: `2px solid ${color}`,
  background: filled ? color : "var(--tyba-bg)",
});

/**
 * Moldura comum dos nós: cabeçalho (marca, nome, papel), portas tipadas e o
 * corpo. Portas de saída: uma por tipo que o nó produz. Portas de entrada:
 * uma linha por ligação que chega, mais a entrada livre onde nasce a próxima.
 */
export function NodeFrame({
  id,
  node,
  selected,
  mark,
  markClass,
  label,
  extra,
  status,
  children,
}: {
  id: string;
  node: MaestroNode;
  selected: boolean;
  mark: ReactNode;
  markClass: string;
  label: string;
  /** Ação do nó no cabeçalho (o ▶ da integração manual). */
  extra?: ReactNode;
  /** Toma o lugar do estado do nó no cabeçalho (subagentes trabalhando). */
  status?: ReactNode;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const { integrations, updateNode, run, createdIds, createdBy } = useMaestro();
  const nodeRun = run?.nodes[id];
  // Criado pelo orquestrador no run à vista: tracejado violeta e "criado pelo
  // Orquestrador". Enquanto efêmero, também sem editar nem apagar; mantido no
  // grafo, edita como qualquer nó.
  const created = createdBy.has(id);
  const locked = createdIds.has(id);
  const orchestrates = !created && node.kind.type === "agent" && node.kind.orchestrate === true;
  const { deleteElements } = useReactFlow();
  const updateInternals = useUpdateNodeInternals();

  const outputs = produces(node.kind, integrations);
  const takesInput = accepts(node.kind, integrations).length > 0;
  const incoming = useNodeConnections({ handleType: "target" }).flatMap((c) => {
    const kind = kindOfOutHandle(c.sourceHandle);
    return kind && c.edgeId ? [{ edgeId: c.edgeId, kind }] : [];
  });
  const inputRows = incoming.length + (takesInput ? 1 : 0);
  const rows = Math.max(outputs.length, inputRows);

  // Porta nova (ligação criada, tipo de saída trocado) só existe para o
  // xyflow depois de medida de novo.
  // `orchestrates` entra porque ligar o toggle cria a porta do orquestrador sem
  // mudar o tamanho do nó: sem medir de novo, o xyflow não acha a porta e
  // descarta a ligação tracejada até os criados.
  const handleKey = `${incoming.map((i) => i.edgeId).join(",")}|${outputs.join(",")}|${takesInput}|${orchestrates}`;
  useEffect(() => {
    updateInternals(id);
  }, [id, handleKey, updateInternals]);

  return (
    <div
      className={`group flex h-full w-full flex-col rounded-[10px] border bg-tyba-surface font-mono text-tyba-text ${
        created
          ? "border-dashed border-tyba-violet/70 shadow-[var(--tyba-shadow-md)]"
          : selected
            ? "border-tyba-violet shadow-[0_0_0_1px_var(--tyba-violet),0_0_0_5px_color-mix(in_srgb,var(--tyba-violet)_18%,transparent)]"
            : orchestrates
              ? "border-tyba-violet shadow-[var(--tyba-shadow-md)]"
              : "border-tyba-border-strong shadow-[var(--tyba-shadow-md)]"
      }`}
    >
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
      <div className="flex h-[46px] shrink-0 items-center gap-2.5 border-b border-tyba-border px-3.5">
        <span
          className={`flex size-6 shrink-0 items-center justify-center rounded-[6px] ${markClass}`}
        >
          {mark}
        </span>
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
          className="nodrag min-w-0 flex-1 bg-transparent text-[14.5px] font-medium text-tyba-text outline-none"
        />
        <span className="min-w-0 shrink truncate text-[11.5px] text-tyba-text-faint">
          {label}
        </span>
        {status ?? (nodeRun && <NodeStatusBadge nodeId={id} nodeRun={nodeRun} />)}
        {extra}
        {!locked && (
          <button
            type="button"
            aria-label={t("maestroDeleteNode")}
            title={t("maestroDeleteNode")}
            onClick={() => void deleteElements({ nodes: [{ id }] })}
            className="nodrag shrink-0 rounded-[4px] p-1 text-tyba-text-faint opacity-0 transition-opacity hover:text-tyba-red focus-visible:opacity-100 group-hover:opacity-100"
          >
            <Trash size={13} />
          </button>
        )}
      </div>
      {rows > 0 && (
        <div className="shrink-0 border-b border-tyba-border py-1.5">
          {Array.from({ length: rows }, (_, i) => {
            const input = incoming[i];
            const isNewInput = !input && takesInput && i === incoming.length;
            const output = outputs[i];
            return (
              <div
                key={i}
                className="relative flex h-6 items-center justify-between gap-2 px-3.5 text-[11px] text-tyba-text-faint"
              >
                <span className="truncate">
                  {input && (
                    <>
                      <Handle
                        type="target"
                        id={inHandle(input.edgeId)}
                        position={Position.Left}
                        isConnectable={false}
                        style={handleStyle(EDGE_COLOR[input.kind], false)}
                      />
                      <b className="font-medium text-tyba-text-muted">{t("maestroInput")}:</b>{" "}
                      {t(`maestroEdge_${input.kind}`)}
                    </>
                  )}
                  {isNewInput && (
                    <>
                      <Handle
                        type="target"
                        id={NEW_IN_HANDLE}
                        position={Position.Left}
                        style={handleStyle("var(--tyba-border-strong)", false)}
                      />
                      <span className="text-tyba-text-ghost">{t("maestroInput")}</span>
                    </>
                  )}
                </span>
                {output && (
                  <span className="truncate">
                    <b className="font-medium text-tyba-text-muted">{t("maestroOutput")}:</b>{" "}
                    {t(`maestroEdge_${output}`)}
                    <Handle
                      type="source"
                      id={outHandle(output)}
                      position={Position.Right}
                      style={handleStyle(EDGE_COLOR[output], true)}
                    />
                  </span>
                )}
              </div>
            );
          })}
        </div>
      )}
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-b-[9px]">
        {children}
        {created && (
          <div className="shrink-0 border-t border-tyba-border px-3.5 py-1.5 text-[11.5px] text-tyba-violet">
            {t("maestroCreatedBy")}
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
  const { updateNode, graphDir } = useMaestro();
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
