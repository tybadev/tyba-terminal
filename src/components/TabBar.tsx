import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Plus,
  ShippingContainer,
  SlidersHorizontal,
  TerminalWindow,
  X,
} from "@phosphor-icons/react";

import { AgentIcon } from "./icons/AgentIcon";

import i18n from "../i18n";
import { formatCombo, tabDigitCombo } from "@/lib/keys";
import { ClaudeIcon } from "./icons/ClaudeIcon";
import { OpenAIIcon } from "./icons/OpenAIIcon";
import {
  leafSessions,
  type Session,
  type SessionCwd,
  type SessionId,
  type Tab,
  type TabId,
} from "../lib/ipc";
import { compactPath } from "../lib/workspaceCwd";
import {
  filete,
  rodaParaHorizontal,
  temAntes,
  temDepois,
  trazerParaVista,
} from "../lib/tabScroll";

interface Props {
  tabs: Tab[];
  activeTab: TabId | null;
  sessions: Session[];
  cwds: Record<string, SessionCwd>;
  onActivate: (id: TabId) => void;
  onClose: (id: TabId) => void;
  onNew: () => void;
}

function runnerLabel(kind: Session["kind"]): string | null {
  if (kind.type !== "agent") return null;
  if (kind.runner === "claude_code") return "claude";
  if (kind.runner === "codex") return "codex";
  return kind.runner.custom;
}

function agentGlyph(label: string): React.ReactNode {
  if (label === "claude") return <ClaudeIcon size={12} />;
  if (label === "codex") return <OpenAIIcon size={12} />;
  return <AgentIcon size={12} />;
}

function tabIcon(tab: Tab, sessions: Map<SessionId, Session>): React.ReactNode {
  if (tab.view === "containers") return <ShippingContainer size={12} />;
  if (tab.view === "settings") return <SlidersHorizontal size={12} />;
  if (tab.root) {
    for (const sid of leafSessions(tab.root)) {
      const session = sessions.get(sid);
      const label = session && runnerLabel(session.kind);
      if (label) return agentGlyph(label);
    }
  }
  return <TerminalWindow size={12} />;
}

function tabLabel(
  tab: Tab,
  sessions: Map<SessionId, Session>,
  cwds: Record<string, SessionCwd>,
): string {
  if (tab.view) return i18n.t(tab.view);
  if (tab.root) {
    for (const sid of leafSessions(tab.root)) {
      const cwd = cwds[sid]?.cwd;
      if (cwd) return compactPath(cwd);
    }
  }
  if (tab.title) return tab.title;
  if (!tab.root) return "shell";
  const bound = leafSessions(tab.root)
    .map((id) => sessions.get(id)?.title)
    .filter(Boolean);
  return bound[0] ?? "shell";
}

export function TabBar({
  tabs,
  activeTab,
  sessions,
  cwds,
  onActivate,
  onClose,
  onNew,
}: Props) {
  const { t } = useTranslation();
  const byId = new Map(sessions.map((s) => [s.id, s]));

  const faixaRef = useRef<HTMLDivElement>(null);
  const ativaRef = useRef<HTMLButtonElement>(null);
  const apagarRef = useRef<number | undefined>(undefined);
  const [borda, setBorda] = useState({ antes: false, depois: false });
  const [marca, setMarca] = useState({ esquerda: 0, largura: 100 });
  const [rolando, setRolando] = useState(false);

  const medir = useCallback(() => {
    const el = faixaRef.current;
    if (!el) return;
    setBorda({ antes: temAntes(el), depois: temDepois(el) });
    setMarca(filete(el));
  }, []);

  // O filete acende ao rolar e apaga sozinho. O mesmo filamento significa
  // "vivo" no topo da aba; deixá-lo aceso embaixo o tempo todo daria duas
  // leituras para a mesma luz.
  const aoRolar = useCallback(() => {
    medir();
    setRolando(true);
    window.clearTimeout(apagarRef.current);
    apagarRef.current = window.setTimeout(() => setRolando(false), 700);
  }, [medir]);

  // A faixa muda de tamanho sem ninguém rolar: abrir a sidebar, dividir o
  // painel, redimensionar a janela. Sem observar isso, o fade fica aceso
  // apontando para abas que passaram a caber.
  useEffect(() => {
    const el = faixaRef.current;
    if (!el) return;
    medir();
    const observador = new ResizeObserver(medir);
    observador.observe(el);
    return () => observador.disconnect();
  }, [medir, tabs.length]);

  useEffect(() => () => window.clearTimeout(apagarRef.current), []);

  // Trocar de aba por atalho (⌘1…⌘9) podia ativar uma aba fora da vista: o
  // conteúdo mudava e a faixa não dava sinal de onde ele foi parar.
  useEffect(() => {
    const el = faixaRef.current;
    const alvo = ativaRef.current;
    if (!el || !alvo) return;
    const destino = trazerParaVista(el, {
      esquerda: alvo.offsetLeft,
      largura: alvo.offsetWidth,
    });
    if (destino === null) return;
    el.scrollTo({ left: destino, behavior: "smooth" });
  }, [activeTab, tabs.length]);

  return (
    <div className="tyba-divide-b relative bg-tyba-surface">
      <div
        ref={faixaRef}
        onScroll={aoRolar}
        onWheel={(e) => {
          const passo = rodaParaHorizontal(e.nativeEvent);
          if (passo === 0 || !faixaRef.current) return;
          faixaRef.current.scrollLeft += passo;
        }}
        className={`tyba-sem-barra flex h-8 items-stretch gap-px overflow-x-auto px-1 ${
          borda.antes ? "tyba-fade-x--ini" : ""
        } ${borda.depois ? "tyba-fade-x--fim" : ""}`}
      >
      {tabs.map((tab, i) => {
        const isActive = tab.id === activeTab;
        return (
          <button
            key={tab.id}
            ref={isActive ? ativaRef : undefined}
            onClick={() => onActivate(tab.id)}
            title={formatCombo(tabDigitCombo(i + 1))}
            className={`group relative flex max-w-44 min-w-24 shrink-0 items-center gap-1.5 rounded-t-[4px] px-2.5 text-[12px] transition-colors ${
              isActive
                // `sunken`, a cor do CONTEÚDO, e não `bg`. Com `bg` havia três
                // superfícies empilhadas — a faixa em `surface`, a aba em `bg`,
                // o painel em `sunken` — e a aba ficava boiando numa cor que
                // não era de ninguém. No `tyba-dark` isso não aparecia porque
                // `bg` e `sunken` distam 5/255.
                // E é o que faz a aba ABRIR na linha: como ela ocupa a altura
                // toda da faixa (`items-stretch`), o fundo dela cobre a divisa
                // `inset` do container justamente sob a aba ativa. A linha
                // deixa de passar por baixo dela.
                ? "bg-tyba-sunken text-tyba-text"
                : "text-tyba-text-faint hover:bg-tyba-text/[.03] hover:text-tyba-text-muted"
            }`}
          >
            {isActive && (
              <span
                className="absolute inset-x-1 top-0 h-0.5 rounded-full"
                style={{ background: "var(--tyba-gradient-soft)" }}
              />
            )}
            <span
              className={`shrink-0 ${
                isActive ? "text-tyba-text-muted" : "text-tyba-text-faint"
              }`}
            >
              {tabIcon(tab, byId)}
            </span>
            <span className="min-w-0 flex-1 truncate text-left font-mono text-[11px]">
              {tabLabel(tab, byId, cwds)}
            </span>
            <span
              role="button"
              aria-label={t("closeTab")}
              onClick={(e) => {
                e.stopPropagation();
                onClose(tab.id);
              }}
              className="rounded-[3px] text-tyba-text-faint opacity-0 transition-opacity hover:text-tyba-text group-hover:opacity-100"
            >
              <X size={11} weight="bold" />
            </span>
          </button>
        );
      })}
      <button
        onClick={onNew}
        aria-label={t("newTab")}
        
        className="flex w-8 shrink-0 items-center justify-center rounded-[4px] text-tyba-text-faint transition-colors hover:bg-tyba-text/[.03] hover:text-tyba-text"
      >
        <Plus size={13} weight="bold" />
      </button>
      </div>
      {/* Fora da faixa que rola: dentro dele o filete rolaria junto e a
          posição deixaria de significar posição. E fora da máscara, senão o
          fade das bordas o apagaria justamente nas pontas, que é onde ele
          precisa ser lido. */}
      {(borda.antes || borda.depois) && (
        <div
          aria-hidden
          className={`tyba-filete ${rolando ? "tyba-filete--vivo" : ""}`}
          style={{ left: `${marca.esquerda}%`, width: `${marca.largura}%` }}
        />
      )}
    </div>
  );
}
