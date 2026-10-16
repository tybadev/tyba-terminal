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
  type MaestroNode,
} from "@/lib/maestro";
import { ImportDialog } from "./ImportDialog";
import { IntegrationsPanel } from "./IntegrationsPanel";
import { MaestroCanvas, type MaestroFlowEdge } from "./MaestroCanvas";
import { RunTimeline } from "./RunTimeline";
import { MaestroContext, STATUS_TONE, type MaestroFlowNode } from "./nodes/NodeFrame";
import { nodeSession } from "./nodes/NodeTerminal";
import { SubagentMark, subagentTone, useSubagentTime } from "./nodes/SubagentNode";
import { useNodeSubagents } from "./useNodeSubagents";
import { SubagentTranscript } from "@/components/SubagentViewer";
import type { SubagentRun } from "@/lib/ipc";
import {
  subagentKey,
  subagentNodeId,
  subagentState,
} from "@/lib/maestroSubagents";

const SAVE_DEBOUNCE_MS = 500;

const tbtn =
  "inline-flex h-7 items-center gap-1.5 whitespace-nowrap rounded-[6px] border border-transparent px-2.5 font-mono text-[12px] text-tyba-text-muted hover:bg-tyba-raised hover:text-tyba-text disabled:cursor-not-allowed disabled:text-tyba-text-ghost disabled:hover:bg-transparent aria-pressed:border-tyba-border-strong aria-pressed:bg-tyba-raised aria-pressed:text-tyba-text";

const sep = <span aria-hidden className="mx-2 h-5 w-px bg-tyba-border-strong" />;

/**
 * O Maestro em tela cheia (quadro MaestroHarness): barra do grafo, canvas e,
 * quando aberto, o painel de variáveis. O grafo não é preso à pasta da
 * sessão: escolhe-se pelo nome na barra, e cada nó tem a própria pasta.
 */
export function MaestroScreen() {
  return (
    <ReactFlowProvider>
      <MaestroInner />
    </ReactFlowProvider>
  );
}

function MaestroInner() {
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
  const [nodes, setNodes] = useState<MaestroFlowNode[]>([]);
  const [edges, setEdges] = useState<MaestroFlowEdge[]>([]);
  const [integrations, setIntegrations] = useState<Integration[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [showVars, setShowVars] = useState(false);
  const [showMinimap, setShowMinimap] = useState(false);
  const lastSaved = useRef<string | null>(null);
  const [runView, dispatchRun] = useReducer(runReducer, EMPTY_RUN_VIEW);
  const [now, setNow] = useState(() => Date.now());
  const [showInts, setShowInts] = useState(false);
  const [notice, setNotice] = useState<{ text: string; warn: boolean } | null>(null);
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
          found ?? emptyGraph(crypto.randomUUID(), t("maestroNewGraphName"), lastDir() ?? home);
        rememberGraph(graph.id);
        const flow = toFlow(graph);
        // Grafo novo e vazio conta como "salvo": só vai para o banco na
        // primeira edição, para não deixar grafo vazio de quem só espiou.
        lastSaved.current = JSON.stringify(fromFlow(graph, flow.nodes, flow.edges));
        setIntegrations(ints);
        setNodes(flow.nodes as MaestroFlowNode[]);
        setEdges(flow.edges);
        setMeta({ id: graph.id, name: graph.name, dir: graph.dir, variables: graph.variables });
        setError(null);
        if (found) {
          void runsList(graph.id)
            .then((runs) => alive && dispatchRun({ type: "history", runs }))
            .catch(() => {});
        }
      })
      .catch((e) => alive && setError(t("maestroLoadError", { error: String(e) })));
    return () => {
      alive = false;
    };
  }, [home, want, t, reload]);

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
        .catch((e) => setError(t("maestroSaveError", { error: String(e) })));
    }, SAVE_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [snapshot, t]);

  // Escuta cada run rodando; depois de assinar, relê o run para não perder o
  // que mudou antes (o core já emite na largada). O reducer descarta retrato
  // velho, então a ordem entre evento e leitura não importa.
  const runningIds = runView.runs
    .filter((r) => r.status === "running")
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
  const latest = runView.runs[0] ?? null;
  const running = runView.runs.find((r) => r.status === "running") ?? null;
  const live = run !== null && run.id === latest?.id;

  // Subagentes de cada nó de agente com terminal vivo no run à vista.
  const agentSessions = useMemo(() => {
    const out = new Map<string, string>();
    if (!run) return out;
    for (const n of nodes) {
      const kind = n.data.node.kind;
      if (kind.type !== "agent") continue;
      const session = nodeSession(kind, run.nodes[n.id], live);
      if (session) out.set(n.id, session);
    }
    return out;
  }, [nodes, run, live]);
  const subagents = useNodeSubagents(agentSessions);
  const [openSub, setOpenSub] = useState<string | null>(null);
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
  const ticking = runningId !== null || subagentsActive;
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
      setError(t("maestroRunError", { error: String(e) }));
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
        .catch((e) => setError(t("maestroRunError", { error: String(e) })));
    }
  };

  const releaseNode = useCallback(
    (nodeId: string) => {
      if (!run) return;
      runNode(run.id, nodeId).catch((e) => setError(t("maestroRunError", { error: String(e) })));
    },
    [run, t],
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
      const folder = await openFileDialog({ directory: true, title: t("maestroExportPick") });
      if (typeof folder !== "string") return;
      // Grafo novo e vazio ainda não foi para o banco: o export lê de lá.
      await flush(true);
      const path = joinPath(folder, exportFileName(meta.name));
      const { cleared, secretFields } = await exportGraph(meta.id, path);
      const warnings = [
        cleared.length > 0 && t("maestroExportedCleared", { names: cleared.join(", ") }),
        secretFields.length > 0 &&
          t("maestroExportedSecrets", {
            fields: secretFields.map((f) => `${f.name} (${f.field})`).join(", "),
          }),
      ].filter(Boolean);
      setNotice({ text: [t("maestroExported", { path }), ...warnings].join(" "), warn: warnings.length > 0 });
    } catch (e) {
      setNotice({ text: t("maestroExportError", { error: String(e) }), warn: true });
    }
  };

  const importFile = async () => {
    try {
      const path = await openFileDialog({
        multiple: false,
        directory: false,
        title: t("maestroImportPick"),
        filters: [{ name: "TYBA Maestro", extensions: ["toml"] }],
      });
      if (typeof path !== "string") return;
      setImporting({ path, preview: await importPreview(path) });
    } catch (e) {
      setNotice({ text: t("maestroImportError", { error: String(e) }), warn: true });
    }
  };

  // Depois do Confirmar, o grafo importado é o que fica aberto.
  const imported = (graphId: string) => {
    setImporting(null);
    setWant({ id: graphId, fresh: false });
    setReload((n) => n + 1);
    setNotice({ text: t("maestroImported"), warn: false });
  };

  const dropIntegration = (e: DragEvent) => {
    const integrationId = e.dataTransfer.getData(INTEGRATION_DRAG_TYPE);
    const integration = integrations.find((i) => i.id === integrationId);
    // Soltar em cima da própria gaveta não cria nó.
    if (!integration || (e.target as Element).closest("[data-maestro-sheet]")) return;
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
    (id: string, update: (node: MaestroNode) => MaestroNode) =>
      setNodes((ns) =>
        ns.map((n) => (n.id === id ? { ...n, data: { node: update(n.data.node) } } : n)),
      ),
    [],
  );
  const { zoomIn, zoomOut, zoomTo, fitView, screenToFlowPosition } = useReactFlow();
  const { zoom } = useViewport();
  const actualSize = Math.round(zoom * 100) === 100;

  const graph = useMemo(
    () => ({
      nodes: nodes.map((n) => n.data.node),
      edges: edges.flatMap((e) =>
        e.data ? [{ id: e.id, from: e.source, to: e.target, kind: e.data.kind }] : [],
      ),
    }),
    [nodes, edges],
  );
  const graphDir = meta?.dir ?? "";
  // Nó novo nasce na última pasta usada; sem ela, na home.
  const newDir = lastDir() ?? (home || graphDir);
  const ctx = useMemo(
    () => ({
      integrations: new Map(integrations.map((i) => [i.id, i])),
      updateNode,
      graph,
      graphDir,
      run,
      live,
      now,
      actualSize,
      runNode: releaseNode,
      subagents,
      openSubagent: opened ? openSub : null,
      setOpenSubagent: setOpenSub,
    }),
    [
      integrations,
      updateNode,
      graph,
      graphDir,
      run,
      live,
      now,
      actualSize,
      releaseNode,
      subagents,
      opened,
      openSub,
    ],
  );

  /** Troca de grafo: o que está no debounce vai antes para o banco. */
  const switchTo = (next: { id: string | null; fresh: boolean }) => {
    void flush()
      .catch(() => {})
      .finally(() => setWant(next));
  };

  return (
    <MaestroContext.Provider value={ctx}>
      <div className="flex min-w-0 flex-1 flex-col bg-tyba-bg font-mono">
        <div
          role="toolbar"
          aria-label={meta?.name || t("maestroGraphUntitled")}
          className="flex h-12 shrink-0 items-center gap-1.5 border-b border-tyba-border bg-tyba-surface pr-3.5 pl-[18px]"
        >
          <input
            ref={nameInput}
            aria-label={t("maestroGraphName")}
            placeholder={t("maestroGraphUntitled")}
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
                aria-label={t("maestroGraphs")}
                title={t("maestroGraphs")}
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
                  <span className="truncate">{g.name || t("maestroGraphUntitled")}</span>
                </DropdownMenuItem>
              ))}
              {graphs.length > 0 && <DropdownMenuSeparator />}
              <DropdownMenuItem
                className="gap-2 text-xs"
                onSelect={() => switchTo({ id: null, fresh: true })}
              >
                <Plus size={12} />
                {t("maestroNewGraph")}
              </DropdownMenuItem>
              <DropdownMenuItem
                className="gap-2 text-xs"
                disabled={!meta}
                onSelect={() => {
                  renaming.current = true;
                }}
              >
                <PencilSimple size={12} />
                {t("maestroRenameGraph")}
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
            {t("maestroRun")}
          </button>
          <button type="button" disabled={stoppable.length === 0} onClick={stop} className={tbtn}>
            <Stop size={13} weight="fill" />
            {t("maestroStop")}
          </button>
          {sep}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button type="button" disabled={runView.runs.length === 0} className={tbtn}>
                <ClockCounterClockwise size={13} />
                {t("maestroRuns")}
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
                    {t(`maestroRunStatus_${r.status}`)} ·{" "}
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
            {t("maestroVariables")}
          </button>
          <button
            type="button"
            disabled={!meta}
            aria-pressed={showInts}
            aria-expanded={showInts}
            aria-controls="maestro-integrations"
            onClick={() => setShowInts((v) => !v)}
            className={tbtn}
          >
            <PlugsConnected size={13} />
            {t("maestroIntegrations")}
          </button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                disabled={!meta}
                aria-label={t("maestroGraphSettings")}
                title={t("maestroGraphSettings")}
                className={tbtn}
              >
                <GearSix size={13} />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-56">
              <DropdownMenuItem className="gap-2 text-xs" onSelect={() => void exportFile()}>
                <FileArrowDown size={13} />
                {t("maestroExport")}
              </DropdownMenuItem>
              <DropdownMenuItem className="gap-2 text-xs" onSelect={() => void importFile()}>
                <FileArrowUp size={13} />
                {t("maestroImport")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <span className="flex-1" />
          <div
            role="group"
            aria-label={t("maestroZoom")}
            className="inline-flex items-center overflow-hidden rounded-[6px] border border-tyba-border-strong"
          >
            <button
              type="button"
              aria-label={t("maestroZoomOut")}
              onClick={() => void zoomOut({ duration: 150 })}
              className="flex px-[7px] py-[5px] text-tyba-text-muted hover:text-tyba-text"
            >
              <MagnifyingGlassMinus size={13} />
            </button>
            <button
              type="button"
              title={t("maestroZoomActual")}
              onClick={() => void zoomTo(1, { duration: 150 })}
              className="min-w-[34px] px-1.5 text-center text-[11.5px] text-tyba-text"
            >
              {Math.round(zoom * 100)}%
            </button>
            <button
              type="button"
              aria-label={t("maestroZoomIn")}
              onClick={() => void zoomIn({ duration: 150 })}
              className="flex px-[7px] py-[5px] text-tyba-text-muted hover:text-tyba-text"
            >
              <MagnifyingGlassPlus size={13} />
            </button>
          </div>
          <button
            type="button"
            onClick={() => void fitView({ padding: 0.15, maxZoom: 1, duration: 200 })}
            className={tbtn}
          >
            <CornersOut size={13} />
            {t("maestroFit")}
          </button>
          <button
            type="button"
            aria-label={t("maestroMinimap")}
            title={t("maestroMinimap")}
            aria-pressed={showMinimap}
            onClick={() => setShowMinimap((v) => !v)}
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
            <MaestroCanvas
              key={meta.id}
              nodes={nodes}
              edges={edges}
              setNodes={setNodes}
              setEdges={setEdges}
              integrations={integrations}
              showMinimap={showMinimap}
              error={error}
              newDir={newDir}
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
            graph={graph}
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
    </MaestroContext.Provider>
  );
}

/**
 * O que um subagente fez (quadro MaestroSubagentes): estado, pedido, tempo e
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
      aria-label={run.description || t("maestroSubNoDescription")}
      className="flex w-[400px] shrink-0 flex-col border-l border-tyba-border bg-tyba-surface"
    >
      <div className="flex shrink-0 items-center gap-2.5 border-b border-tyba-border px-4 py-3">
        <span className={subagentTone(state)} title={t(`maestroSub_${state}`)}>
          <SubagentMark state={state} size={13} />
        </span>
        <span className="min-w-0 flex-1 truncate text-[13px] text-tyba-text">
          {run.description || t("maestroSubNoDescription")}
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
      aria-label={t("maestroVariables")}
      className="flex w-[320px] shrink-0 flex-col gap-2 border-l border-tyba-border bg-tyba-surface p-3"
    >
      <div className="flex items-center justify-between">
        <span className="text-[13px] font-medium text-tyba-text">{t("maestroVariables")}</span>
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
        {t("maestroVarsHint", { example: "{{nome}}" })}
      </p>
      <div className="flex min-h-0 flex-col gap-1.5 overflow-y-auto">
        {rows.map(([name, value], i) => (
          <div key={i} className="flex items-center gap-1.5">
            <input
              aria-label={t("maestroVarName")}
              placeholder={t("maestroVarName")}
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
              aria-label={t("maestroVarValue")}
              placeholder={t("maestroVarValue")}
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
              aria-label={t("maestroVarRemove")}
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
        {t("maestroVarAdd")}
      </button>
    </aside>
  );
}
