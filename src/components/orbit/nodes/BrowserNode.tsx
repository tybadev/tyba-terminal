import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowClockwise, ArrowLeft, ArrowSquareOut, Browser } from "@phosphor-icons/react";
import type { NodeProps } from "@xyflow/react";

import { openExternalUrl } from "@/lib/clipboard";
import { frameInUse } from "@/lib/graphKeys";
import { browserRefusal, isLocalUrl, openedChipLeft, type NodeKind } from "@/lib/orbit";
import { NodeFrame, useOrbit, type OrbitFlowNode } from "./NodeFrame";

type BrowserKind = Extract<NodeKind, { type: "browser" }>;

/** Atributo `sandbox` do iframe: a página roda, mas não navega o app. */
const SANDBOX = "allow-scripts allow-same-origin allow-forms allow-popups";

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

/**
 * Nó de navegador (quadro OrbitNavegador): voltar, recarregar, endereço e
 * "Abrir fora"; a página num iframe com `sandbox`. Só endereço local carrega —
 * o resto mostra o aviso e nada vai para o frame (a CSP `frame-src` também
 * recusa). Na execução vale a url do nó; sem ela, a que chega pela ligação.
 *
 * Clicar na página passa o teclado para ela ("você" na cápsula); clicar fora
 * devolve. O Esc dentro da página é dela e não chega ao app, por isso o
 * rodapé diz "clique fora". O login não fica salvo aqui dentro: para o
 * WKWebView a página é de terceiro (o app é `tauri://localhost`) e o cookie
 * dela é recusado — "Abrir fora" fica à mão, e a solução é a Fase 5.
 */
export function BrowserNode({ id, data, selected }: NodeProps<OrbitFlowNode>) {
  const { t } = useTranslation();
  const { updateNode, run } = useOrbit();
  const node = data.node;
  const kind = node.kind as BrowserKind;
  const runUrl = run?.nodes[id]?.url ?? null;
  const opened = run?.nodes[id]?.opened ?? null;

  // Endereços visitados por aqui (a navegação dentro do frame é de outra
  // origem e não se lê): o ← volta para o anterior.
  const [history, setHistory] = useState<string[]>(() => (kind.url ? [kind.url] : []));
  const [draft, setDraft] = useState(kind.url);
  const [reload, setReload] = useState(0);
  const current = history[history.length - 1] ?? "";

  // O dono usando a página: o foco saiu do app para o iframe deste nó. Estado
  // de tela efêmero (o controle de verdade, com o agente esperando, é da Fase 5).
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

  // Agente abriu: o core já gravou a url no nó; o canvas acompanha, para o
  // próximo salvar não a desfazer. E o chip "<agente> abriu" por uns segundos.
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

  const ok = current !== "" && frameable(current);
  const btn =
    "nodrag flex shrink-0 items-center rounded-[4px] p-1 text-tyba-text-muted hover:bg-tyba-raised hover:text-tyba-text disabled:text-tyba-text-ghost disabled:hover:bg-transparent";

  return (
    <NodeFrame
      id={id}
      node={node}
      selected={selected}
      mark={<Browser size={13} weight="bold" />}
      markTone="text-tyba-cyan"
      meta={inUse ? t("orbitBrowserInUse") : t("orbitBrowserLocalOnly")}
      status={
        inUse ? (
          <span className="shrink-0 rounded-full bg-tyba-cyan/[.18] px-[9px] py-[2px] text-[11.5px] text-tyba-text">
            {t("orbitBrowserYou")}
          </span>
        ) : openedBy ? (
          <span className="max-w-[160px] shrink-0 truncate rounded-full bg-tyba-cyan/[.18] px-[9px] py-[2px] text-[11.5px] text-tyba-text">
            {t("orbitBrowserOpenedBy", { name: openedBy })}
          </span>
        ) : undefined
      }
      inUse={inUse}
      details={[{ label: t("orbitDetailUrl"), value: current }]}
    >
      <div className="flex shrink-0 items-center gap-1.5 px-2.5 pb-1">
        <button
          type="button"
          aria-label={t("orbitBrowserBack")}
          title={t("orbitBrowserBack")}
          disabled={history.length < 2}
          onClick={back}
          className={btn}
        >
          <ArrowLeft size={13} />
        </button>
        <button
          type="button"
          aria-label={t("orbitBrowserReload")}
          title={t("orbitBrowserReload")}
          disabled={!ok}
          onClick={() => setReload((n) => n + 1)}
          className={btn}
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
          disabled={!ok}
          title={t("orbitBrowserOpenOutsideHint")}
          onClick={() => void openExternalUrl(current).catch(() => {})}
          className="nodrag inline-flex h-6 shrink-0 items-center gap-1 rounded-[4px] border border-tyba-border-strong px-2 text-[11px] text-tyba-text-muted hover:text-tyba-text disabled:text-tyba-text-ghost"
        >
          <ArrowSquareOut size={12} />
          {t("orbitBrowserOpenOutside")}
        </button>
      </div>
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
        ) : (
          <p className="absolute inset-0 flex items-center justify-center px-6 text-center font-mono text-[12px] text-tyba-text-faint">
            {current === "" ? t("orbitBrowserEmpty") : t(browserRefusal(current))}
          </p>
        )}
      </div>
    </NodeFrame>
  );
}
