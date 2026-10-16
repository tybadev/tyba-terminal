import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  BracketsCurly,
  ClockCounterClockwise,
  CornersOut,
  FolderSimple,
  GitBranch,
  MagnifyingGlassMinus,
  MagnifyingGlassPlus,
  MapTrifold,
  Play,
  PlugsConnected,
  Plus,
  Stop,
  X,
} from "@phosphor-icons/react";
import { ReactFlowProvider, useReactFlow, useViewport } from "@xyflow/react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  EMPTY_RUN_VIEW,
  emptyGraph,
  formatDuration,
  fromFlow,
  listGraphs,
  listIntegrations,
  onRun,
  runGet,
  runNode,
  runNumber,
  runReducer,
  runStart,
  runStop,
  runsList,
  saveGraph,
  selectedRun,
  toFlow,
  type Graph,
  type Integration,
  type MaestroNode,
} from "@/lib/maestro";
import { MaestroCanvas, type MaestroFlowEdge } from "./MaestroCanvas";
import { RunTimeline } from "./RunTimeline";
import { MaestroContext, STATUS_TONE, type MaestroFlowNode } from "./nodes/NodeFrame";

const SAVE_DEBOUNCE_MS = 500;

const baseName = (dir: string) => dir.split(/[\\/]/).filter(Boolean).pop() ?? dir;

const tbtn =
  "inline-flex h-7 items-center gap-1.5 whitespace-nowrap rounded-[6px] border border-transparent px-2.5 font-mono text-[12px] text-tyba-text-muted hover:bg-tyba-raised hover:text-tyba-text disabled:cursor-not-allowed disabled:text-tyba-text-ghost disabled:hover:bg-transparent aria-pressed:border-tyba-border-strong aria-pressed:bg-tyba-raised aria-pressed:text-tyba-text";

const sep = <span aria-hidden className="mx-2 h-5 w-px bg-tyba-border-strong" />;

/**
 * O Maestro em tela cheia (quadro MaestroHarness): barra do grafo, canvas e,
 * quando aberto, o painel de variáveis. Um grafo por pasta de workspace; com
 * mais de uma pasta aberta, o chip da pasta vira seletor.
 */
export function MaestroScreen(props: {
  dirs: string[];
  activeDir: string | null;
  branch: string | null;
}) {
  return (
    <ReactFlowProvider>
      <MaestroInner {...props} />
    </ReactFlowProvider>
  );
}

function MaestroInner({
  dirs,
  activeDir,
  branch,
}: {
  dirs: string[];
  activeDir: string | null;
  branch: string | null;
}) {
  const { t, i18n } = useTranslation();
  const [dir, setDir] = useState<string | null>(activeDir ?? dirs[0] ?? null);
  useEffect(() => {
    if (dir === null && (activeDir ?? dirs[0])) setDir(activeDir ?? dirs[0]);
  }, [dir, activeDir, dirs]);

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

  // Carrega o grafo da pasta (ou começa um vazio, gravado na primeira edição).
  useEffect(() => {
    if (!dir) return;
    let alive = true;
    setMeta(null);
    dispatchRun({ type: "reset" });
    void Promise.all([listGraphs(), listIntegrations()])
      .then(([graphs, ints]) => {
        if (!alive) return;
        const found = graphs.find((g) => g.dir === dir);
        const graph = found ?? emptyGraph(crypto.randomUUID(), dir);
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
  }, [dir, t]);

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

  // Relógio dos tempos de nó e de execução, só enquanto algo roda.
  const runningId = running?.id ?? null;
  useEffect(() => {
    if (!runningId) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [runningId]);

  const start = async () => {
    if (!snapshot || !meta) return;
    try {
      // O core roda o grafo do banco: o que ainda está no debounce vai antes.
      if (snapshot !== lastSaved.current) {
        await saveGraph(JSON.parse(snapshot) as Graph);
        lastSaved.current = snapshot;
      }
      const id = await runStart(meta.id);
      const started = await runGet(id);
      if (started) dispatchRun({ type: "update", run: started });
      setError(null);
    } catch (e) {
      setError(t("maestroRunError", { error: String(e) }));
    }
  };

  const stop = () => {
    if (!running) return;
    runStop(running.id).catch((e) => setError(t("maestroRunError", { error: String(e) })));
  };

  const releaseNode = useCallback(
    (nodeId: string) => {
      if (!run) return;
      runNode(run.id, nodeId).catch((e) => setError(t("maestroRunError", { error: String(e) })));
    },
    [run, t],
  );

  const updateNode = useCallback(
    (id: string, update: (node: MaestroNode) => MaestroNode) =>
      setNodes((ns) =>
        ns.map((n) => (n.id === id ? { ...n, data: { node: update(n.data.node) } } : n)),
      ),
    [],
  );
  const { zoomIn, zoomOut, zoomTo, fitView } = useReactFlow();
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
  const live = run !== null && run.id === latest?.id;
  const ctx = useMemo(
    () => ({
      integrations: new Map(integrations.map((i) => [i.id, i])),
      updateNode,
      graph,
      run,
      live,
      now,
      actualSize,
      runNode: releaseNode,
    }),
    [integrations, updateNode, graph, run, live, now, actualSize, releaseNode],
  );

  if (!dir) {
    return (
      <div className="flex flex-1 items-center justify-center bg-tyba-bg font-mono text-[13px] text-tyba-text-faint">
        {t("maestroNoDir")}
      </div>
    );
  }

  const dirChip = (
    <span className="inline-flex items-center gap-1.5 rounded-[5px] border border-tyba-border bg-tyba-raised px-2 py-[3px] text-[11px] text-tyba-text-muted">
      <FolderSimple size={12} />
      {baseName(dir)}
      {branch && dir === activeDir && (
        <>
          <span className="text-tyba-text-faint">·</span>
          <GitBranch size={12} className="text-tyba-blue" />
          <span className="text-tyba-blue">{branch}</span>
        </>
      )}
    </span>
  );

  return (
    <MaestroContext.Provider value={ctx}>
      <div className="flex min-w-0 flex-1 flex-col bg-tyba-bg font-mono">
        <div
          role="toolbar"
          aria-label={meta?.name ?? baseName(dir)}
          className="flex h-12 shrink-0 items-center gap-1.5 border-b border-tyba-border bg-tyba-surface pr-3.5 pl-[18px]"
        >
          <input
            aria-label={t("maestroGraphName")}
            spellCheck={false}
            autoCorrect="off"
            autoCapitalize="off"
            value={meta?.name ?? ""}
            disabled={!meta}
            onChange={(e) => {
              const name = e.target.value;
              setMeta((m) => (m ? { ...m, name } : m));
            }}
            style={{ width: `${Math.max(6, (meta?.name.length ?? 6) + 1)}ch` }}
            className="mr-1 bg-transparent text-[14px] font-medium text-tyba-text outline-none"
          />
          {dirs.length > 1 ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button type="button" aria-label={t("maestroDir")} title={dir}>
                  {dirChip}
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start">
                {dirs.map((d) => (
                  <DropdownMenuItem key={d} className="text-xs" onSelect={() => setDir(d)}>
                    <span className="truncate" title={d}>
                      {d}
                    </span>
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          ) : (
            <span title={dir}>{dirChip}</span>
          )}
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
          <button type="button" disabled={!running} onClick={stop} className={tbtn}>
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
          <button type="button" disabled title={t("comingSoon")} className={tbtn}>
            <PlugsConnected size={13} />
            {t("maestroIntegrations")}
          </button>
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
        <div className="flex min-h-0 flex-1">
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
            />
          ) : (
            <div className="flex flex-1 items-center justify-center text-[12.5px] text-tyba-text-faint">
              {error}
            </div>
          )}
          {showVars && meta && (
            <VariablesPanel
              variables={meta.variables}
              onChange={(variables) => setMeta((m) => (m ? { ...m, variables } : m))}
              onClose={() => setShowVars(false)}
            />
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
    </MaestroContext.Provider>
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
