import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ArrowClockwise, ArrowLeft, ArrowSquareOut, Browser } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import { openExternalUrl } from "@/lib/clipboard";
import { frameInUse } from "@/lib/graphKeys";
import { browserRefusal, isLocalUrl, openedChipLeft, type NodeKind } from "@/lib/orbit";
import {
  agentActiveLeft,
  agentStepKey,
  browserBack,
  browserNative,
  browserNavigate,
  browserReload,
  previewTime,
  type BrowserState,
} from "@/lib/orbitBrowser";
import { browserControllers } from "@/lib/orbitTalk";
import { NodeFrame, useOrbit, type OrbitFlowNode } from "./NodeFrame";
import { useBrowserPreview, useBrowserState, useNativeOcclusion } from "./useNativeOcclusion";

type BrowserKind = Extract<NodeKind, { type: "browser" }>;

/** Atributo `sandbox` do iframe: a página roda, mas não navega o app. */
const SANDBOX = "allow-scripts allow-same-origin allow-forms allow-popups";

const BTN =
  "nodrag flex shrink-0 items-center rounded-[4px] p-1 text-tyba-text-muted hover:bg-tyba-raised hover:text-tyba-text disabled:text-tyba-text-ghost disabled:hover:bg-transparent";

/**
 * A página pode ir para o iframe: endereço local (a regra do core) e nunca a
 * origem do próprio app — com `allow-same-origin`, o TYBA em dev servido em
 * localhost se enxergaria por dentro do frame.
 */
function frameable(url: string): boolean {
  if (!isLocalUrl(url)) return false;
  try {
    return new URL(url).origin !== window.location.origin;
  } catch {
    return false;
  }
}

/** O sistema tem a view nativa? `null` enquanto o core não respondeu. */
function useNative(): boolean | null {
  const [native, setNative] = useState<boolean | null>(null);
  useEffect(() => {
    let alive = true;
    void browserNative().then((n) => alive && setNative(n));
    return () => {
      alive = false;
    };
  }, []);
  return native;
}

/**
 * Nó de navegador (quadro OrbitNavegador): voltar, recarregar, endereço e
 * "Abrir fora". No macOS a página é uma view nativa do core por cima do card
 * (Fase 5: documento de topo, login fica salvo); nos outros sistemas, por
 * enquanto, um iframe com `sandbox`.
 */
export function BrowserNode(props: NodeProps<OrbitFlowNode>) {
  const native = useNative();
  if (native === null) return <IframeBrowserNode {...props} pending />;
  return native ? <NativeBrowserNode {...props} /> : <IframeBrowserNode {...props} />;
}

/** A barra do nó: ←, ⟳, endereço e "Abrir fora". */
function Toolbar({
  draft,
  setDraft,
  submit,
  canBack,
  canReload,
  onBack,
  onReload,
  outside,
  outsideHint,
}: {
  draft: string;
  setDraft: (v: string) => void;
  submit: () => void;
  canBack: boolean;
  canReload: boolean;
  onBack: () => void;
  onReload: () => void;
  outside: string | null;
  outsideHint: string;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex shrink-0 items-center gap-1.5 px-2.5 pb-1">
      <button
        type="button"
        aria-label={t("orbitBrowserBack")}
        title={t("orbitBrowserBack")}
        disabled={!canBack}
        onClick={onBack}
        className={BTN}
      >
        <ArrowLeft size={13} />
      </button>
      <button
        type="button"
        aria-label={t("orbitBrowserReload")}
        title={t("orbitBrowserReload")}
        disabled={!canReload}
        onClick={onReload}
        className={BTN}
      >
        <ArrowClockwise size={13} />
      </button>
      <input
        aria-label={t("orbitBrowserAddress")}
        placeholder="http://localhost:5173"
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit();
        }}
        onBlur={submit}
        className="nodrag h-7 min-w-0 flex-1 rounded-[6px] border border-tyba-border-strong bg-tyba-sunken px-2.5 font-mono text-[12px] text-tyba-text outline-none focus:border-tyba-cyan/60"
      />
      <button
        type="button"
        disabled={!outside}
        title={outsideHint}
        onClick={() => outside && void openExternalUrl(outside).catch(() => {})}
        className="nodrag inline-flex h-6 shrink-0 items-center gap-1 rounded-[4px] border border-tyba-border-strong px-2 text-[11px] text-tyba-text-muted hover:text-tyba-text disabled:text-tyba-text-ghost"
      >
        <ArrowSquareOut size={12} />
        {t("orbitBrowserOpenOutside")}
      </button>
    </div>
  );
}

/**
 * O chip da cápsula: "você", "<agente> usando" (o agente agindo na página,
 * quadro MaestroCardsD), "<agente> abriu" ou "controlado por X".
 */
function statusChip(
  t: (k: string, o?: Record<string, string>) => string,
  inUse: boolean,
  openedBy: string | null,
  controllers: string[],
  agentUsing: string | null = null,
): ReactNode {
  if (inUse)
    return (
      <span className="shrink-0 rounded-full bg-tyba-cyan/[.18] px-[9px] py-[2px] text-[11.5px] text-tyba-text">
        {t("orbitBrowserYou")}
      </span>
    );
  if (agentUsing)
    return (
      <span className="max-w-[200px] shrink-0 truncate text-[11.5px] text-tyba-cyan">
        {t("orbitBrowserAgentUsing", { name: agentUsing })}
      </span>
    );
  if (openedBy)
    return (
      <span className="max-w-[160px] shrink-0 truncate rounded-full bg-tyba-cyan/[.18] px-[9px] py-[2px] text-[11.5px] text-tyba-text">
        {t("orbitBrowserOpenedBy", { name: openedBy })}
      </span>
    );
  if (controllers.length > 0)
    return (
      <span className="max-w-[200px] shrink-0 truncate text-[11.5px] text-tyba-cyan">
        {t("orbitBrowserControlledBy", { name: controllers.join(", ") })}
      </span>
    );
  return undefined;
}

/**
 * Agente abriu (`orbit_browser_open`): o chip "<agente> abriu" por uns
 * segundos, e a url local vai para o nó, para o próximo salvar não a desfazer.
 */
function useOpenedBy(id: string, kind: BrowserKind): string | null {
  const { updateNode, run } = useOrbit();
  const runUrl = run?.nodes[id]?.url ?? null;
  const opened = run?.nodes[id]?.opened ?? null;
  const [openedBy, setOpenedBy] = useState<string | null>(null);
  useEffect(() => {
    if (!opened) return;
    if (runUrl && isLocalUrl(runUrl) && kind.url !== runUrl) {
      updateNode(id, (n) => ({ ...n, kind: { type: "browser", url: runUrl } }));
    }
    const left = openedChipLeft(opened, Date.now());
    if (left <= 0) return;
    setOpenedBy(opened.by);
    const timer = window.setTimeout(() => setOpenedBy(null), left);
    return () => window.clearTimeout(timer);
    // Só quando um agente abre de novo.
  }, [opened?.at]);
  return openedBy;
}

/**
 * O agente agindo na página: o nome dele enquanto o estado ativo vale
 * ({@link agentActiveLeft}); depois volta a "controlado por".
 */
function useAgentUsing(state: BrowserState | null): string | null {
  const [, tick] = useState(0);
  const left = agentActiveLeft(state, Date.now());
  useEffect(() => {
    if (left <= 0) return;
    const timer = window.setTimeout(() => tick((n) => n + 1), Math.min(left, 1000));
    return () => window.clearTimeout(timer);
  });
  return left > 0 ? (state?.agent?.by ?? null) : null;
}

/** O rodapé com o último passo do agente: "clicou em “Semana” · 0 erros no console". */
function agentStepLine(
  t: (k: string, o?: Record<string, unknown>) => string,
  state: BrowserState | null,
): string | null {
  const step = state?.agent;
  if (!step) return null;
  const done = t(agentStepKey(step), { name: step.target ?? "" });
  if (state.consoleErrors == null) return done;
  return `${done} · ${t("orbitBrowserConsoleErrors", { count: state.consoleErrors })}`;
}

/**
 * macOS: a página numa view nativa do core, presa à janela por cima do card.
 * O card desenha só o marcador (onde a view fica) e, quando ela some — pan,
 * zoom fora da faixa, menu por cima —, o último print. Clicar na página passa
 * o teclado para ela ("você"); o core devolve.
 */
function NativeBrowserNode({ id, data, selected }: NodeProps<OrbitFlowNode>) {
  const { t } = useTranslation();
  const { updateNode, run, graph, graphId } = useOrbit();
  const node = data.node;
  const kind = node.kind as BrowserKind;
  const runUrl = run?.nodes[id]?.url ?? null;
  const opened = run?.nodes[id]?.opened ?? null;
  const controllers = browserControllers(id, graph.nodes, graph.edges);
  const openedBy = useOpenedBy(id, kind);

  const { state, ready } = useBrowserState(graphId, id);
  const owner = state?.focus === "owner";
  const current = state?.url ?? "";
  const marker = useRef<HTMLDivElement>(null);
  useNativeOcclusion(graphId, id, marker, graphId !== "", owner);
  const preview = useBrowserPreview(graphId, id, state?.previewAt);

  const go = (url: string) => {
    if (graphId) void browserNavigate(graphId, id, url).catch(() => {});
  };

  // A barra segue a página (link clicado, voltar), menos enquanto o dono
  // digita nela.
  const [draft, setDraft] = useState(kind.url);
  const address = useRef<string>("");
  useEffect(() => {
    if (current && document.activeElement?.getAttribute("aria-label") !== t("orbitBrowserAddress"))
      setDraft(current);
    address.current = current;
  }, [current]);

  // Primeira vez: a url da execução ou a do nó, se o core ainda não tem
  // página. Depois, cada abertura de agente (`opened` muda mesmo com a mesma
  // url) navega de novo.
  const lastOpen = useRef<string | null>(null);
  useEffect(() => {
    if (!ready) return;
    const key = runUrl ? `${runUrl}#${opened?.at ?? 0}` : "";
    if (lastOpen.current === null) {
      lastOpen.current = key;
      const start = runUrl ?? kind.url;
      if (!state?.url && start) go(start);
      return;
    }
    if (runUrl && key !== lastOpen.current) {
      lastOpen.current = key;
      go(runUrl);
    }
  }, [ready, runUrl, opened?.at]);

  const submit = () => {
    const url = draft.trim();
    if (!url || url === address.current) return;
    go(url);
    // Só endereço local vira a url do nó: o core recusaria salvar o resto.
    if (isLocalUrl(url)) updateNode(id, (n) => ({ ...n, kind: { type: "browser", url } }));
  };

  const agentUsing = useAgentUsing(state);
  const step = agentStepLine(t, state);
  const meta = state?.refused
    ? t("orbitBrowserNavRefused", { why: t(`orbitBrowserRefused_${state.refused}`) })
    : owner
      ? t("orbitBrowserInUseNative")
      : step
        ? step
        : state?.ephemeral
          ? t("orbitBrowserEphemeral")
          : t("orbitBrowserLocalOnly");

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<Browser size={13} weight="bold" />}
      markTone="text-tyba-cyan"
      meta={meta}
      status={statusChip(t, owner, openedBy, controllers, agentUsing)}
      inUse={owner}
      details={[
        { label: t("orbitDetailUrl"), value: current },
        ...(state?.title ? [{ label: t("orbitDetailTitle"), value: state.title }] : []),
      ]}
    >
      <Toolbar
        draft={draft}
        setDraft={setDraft}
        submit={submit}
        canBack={current !== ""}
        canReload={current !== ""}
        onBack={() => graphId && void browserBack(graphId, id).catch(() => {})}
        onReload={() => graphId && void browserReload(graphId, id).catch(() => {})}
        outside={isLocalUrl(current) ? current : null}
        outsideHint={t("orbitBrowserOpenOutsideNativeHint")}
      />
      {/* O marcador: a view nativa fica exatamente aqui, por cima. Quando ela
          some, aparece o que está dentro (o último print ou o aviso). */}
      <div
        ref={marker}
        className="nodrag relative mx-1.5 mb-0.5 min-h-0 flex-1 overflow-hidden rounded-[10px] border border-tyba-border bg-tyba-sunken"
      >
        {!state?.live && current !== "" && preview && (
          // A prévia no tamanho da view (o print é da página inteira à
          // vista, sem o zoom do canvas).
          <img
            src={preview}
            alt=""
            draggable={false}
            className="absolute inset-0 h-full w-full object-cover object-left-top"
          />
        )}
        {(!state?.live || current === "") && !(preview && current !== "") && (
          <p className="absolute inset-0 flex items-center justify-center px-6 text-center font-mono text-[12px] text-tyba-text-faint">
            {current === ""
              ? state?.refused
                ? t(`orbitBrowserRefused_${state.refused}`)
                : t("orbitBrowserEmpty")
              : state?.loading
                ? t("orbitBrowserLoading")
                : state?.title || current}
          </p>
        )}
        {!state?.live && state?.previewAt != null && (
          <span className="absolute bottom-1.5 right-1.5 rounded-full bg-tyba-surface/90 px-2 py-[1px] text-[10.5px] text-tyba-text-muted">
            {t("orbitBrowserPreview", { time: previewTime(state.previewAt) })}
          </span>
        )}
      </div>
    </NodeFrame>
  );
}

/**
 * Iframe (Windows/Linux até a Fase 5 chegar neles): só endereço local
 * carrega — o resto mostra o aviso e nada vai para o frame (a CSP
 * `frame-src` também recusa). Na execução vale a url do nó; sem ela, a que
 * chega pela ligação.
 *
 * Clicar na página passa o teclado para ela ("você" na cápsula); clicar fora
 * devolve. O Esc dentro da página é dela e não chega ao app, por isso o
 * rodapé diz "clique fora". O login não fica salvo aqui dentro: a página é de
 * terceiro para o webview e o cookie dela é recusado — "Abrir fora" fica à
 * mão.
 */
function IframeBrowserNode({
  id,
  data,
  selected,
  pending = false,
}: NodeProps<OrbitFlowNode> & { pending?: boolean }) {
  const { t } = useTranslation();
  const { updateNode, run, graph } = useOrbit();
  const node = data.node;
  const kind = node.kind as BrowserKind;
  const runUrl = run?.nodes[id]?.url ?? null;
  const opened = run?.nodes[id]?.opened ?? null;
  // Agente ligado por `control` (quadro MaestroConversa): "controlado por X"
  // enquanto a ligação existir.
  const controllers = browserControllers(id, graph.nodes, graph.edges);
  const openedBy = useOpenedBy(id, kind);

  // Endereços visitados por aqui (a navegação dentro do frame é de outra
  // origem e não se lê): o ← volta para o anterior.
  const [history, setHistory] = useState<string[]>(() => (kind.url ? [kind.url] : []));
  const [draft, setDraft] = useState(kind.url);
  const [reload, setReload] = useState(0);
  const current = history[history.length - 1] ?? "";

  // O dono usando a página: o foco saiu do app para o iframe deste nó.
  const frame = useRef<HTMLIFrameElement>(null);
  const [inUse, setInUse] = useState(false);
  useEffect(() => {
    // No `blur` o elemento ativo ainda pode não ser o frame: confere depois.
    const onBlur = () =>
      window.setTimeout(() => setInUse(frameInUse(document.activeElement, frame.current)), 0);
    const onFocus = () => setInUse(false);
    window.addEventListener("blur", onBlur);
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("focus", onFocus);
    };
  }, []);
  // Página nova (navegar, recarregar): o frame é outro.
  useEffect(() => setInUse(false), [current, reload]);

  const go = (url: string) => {
    setDraft(url);
    setHistory((h) => (h[h.length - 1] === url ? h : [...h, url]));
  };

  // A url da execução chega pela ligação, ou um agente abriu uma página
  // (`opened` muda mesmo que a url seja a mesma): vai para o frame.
  useEffect(() => {
    if (runUrl) go(runUrl);
  }, [runUrl, opened?.at]);

  const submit = () => {
    const url = draft.trim();
    if (!url) return;
    go(url);
    // Só endereço local vira a url do nó: o core recusaria salvar o resto.
    if (isLocalUrl(url)) updateNode(id, (n) => ({ ...n, kind: { type: "browser", url } }));
  };

  const back = () => {
    if (history.length < 2) return;
    const prev = history.slice(0, -1);
    setHistory(prev);
    setDraft(prev[prev.length - 1]);
  };

  const ok = !pending && current !== "" && frameable(current);

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<Browser size={13} weight="bold" />}
      markTone="text-tyba-cyan"
      meta={inUse ? t("orbitBrowserInUse") : t("orbitBrowserLocalOnly")}
      status={statusChip(t, inUse, openedBy, controllers)}
      inUse={inUse}
      details={[{ label: t("orbitDetailUrl"), value: current }]}
    >
      <Toolbar
        draft={draft}
        setDraft={setDraft}
        submit={submit}
        canBack={history.length >= 2}
        canReload={ok}
        onBack={back}
        onReload={() => setReload((n) => n + 1)}
        outside={ok ? current : null}
        outsideHint={t("orbitBrowserOpenOutsideHint")}
      />
      <div className="nodrag nowheel nopan relative mx-1.5 mb-0.5 min-h-0 flex-1 overflow-hidden rounded-[10px] border border-tyba-border bg-tyba-sunken">
        {ok ? (
          <iframe
            key={`${current}#${reload}`}
            ref={frame}
            title={node.title}
            src={current}
            sandbox={SANDBOX}
            referrerPolicy="no-referrer"
            className="absolute inset-0 h-full w-full border-0 bg-white"
          />
        ) : pending ? null : (
          <p className="absolute inset-0 flex items-center justify-center px-6 text-center font-mono text-[12px] text-tyba-text-faint">
            {current === "" ? t("orbitBrowserEmpty") : t(browserRefusal(current))}
          </p>
        )}
      </div>
    </NodeFrame>
  );
}
