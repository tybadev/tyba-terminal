import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { GitPullRequest, Plus, X } from "@phosphor-icons/react";

import { AgentIcon } from "./icons/AgentIcon";
import { DockerIcon } from "./icons/DockerIcon";
import { GitIcon } from "./icons/GitIcon";

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
import { tabAttention } from "../lib/sessionStatus";
import {
  baseName,
  containerName,
  isEditorDirty,
  langGlyph,
  parseContentView,
  useContentTabsVersion,
} from "../lib/contentTabs";
import { compactPath, dirLabel } from "../lib/workspaceCwd";
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

/** Sessão de container (shell `docker exec`) que a aba mostra, se for o caso. */
function containerSession(tab: Tab, sessions: Map<SessionId, Session>): Session | null {
  if (!tab.root) return null;
  for (const sid of leafSessions(tab.root)) {
    const session = sessions.get(sid);
    if (session?.kind.type === "container") return session;
  }
  return null;
}

/** Aba de container (logs ou shell): ícone e filete em azul docker. */
function isContainerTab(tab: Tab, sessions: Map<SessionId, Session>): boolean {
  return (
    parseContentView(tab.view)?.kind === "container-logs" ||
    containerSession(tab, sessions) !== null
  );
}

// Só os glifos que os mockups mostram: selo de linguagem no editor, docker
// azul no container, git no diff/histórico e o glifo do agente. Terminal e as
// demais abas são só texto (mockup Proposta) — `null` aqui.
function tabIcon(tab: Tab, sessions: Map<SessionId, Session>): React.ReactNode {
  const content = parseContentView(tab.view);
  if (content?.kind === "editor") {
    const glyph = langGlyph(content.rel);
    return (
      <span className={`font-mono text-[10px] font-medium ${glyph.className}`}>
        {glyph.text}
      </span>
    );
  }
  if (content?.kind === "container-logs" || containerSession(tab, sessions)) {
    return <DockerIcon size={13} className="text-tyba-blue" />;
  }
  if (content?.kind === "session-diff") return <GitIcon size={12} />;
  if (content?.kind === "git-log") return <GitIcon size={12} className="text-tyba-blue" />;
  if (content?.kind === "pr-list" || content?.kind === "pr") {
    return <GitPullRequest size={12} className="text-tyba-blue" />;
  }
  if (tab.root) {
    for (const sid of leafSessions(tab.root)) {
      const session = sessions.get(sid);
      const label = session && runnerLabel(session.kind);
      if (label) return agentGlyph(label);
    }
  }
  return null;
}

/** O cwd da primeira sessão da aba que já tem um — o rótulo usa a pasta, o
 *  tooltip o caminho inteiro. */
function tabCwd(tab: Tab, cwds: Record<string, SessionCwd>): string | null {
  if (!tab.root || tab.view) return null;
  for (const sid of leafSessions(tab.root)) {
    const cwd = cwds[sid]?.cwd;
    if (cwd) return cwd;
  }
  return null;
}

function tabLabel(
  tab: Tab,
  sessions: Map<SessionId, Session>,
  cwds: Record<string, SessionCwd>,
): string {
  const content = parseContentView(tab.view);
  if (content?.kind === "editor") return baseName(content.rel);
  if (content?.kind === "container-logs") {
    return i18n.t("tabContainerLogs", { name: containerName(content.containerId) });
  }
  if (content?.kind === "git-log") return i18n.t("tabGitHistory");
  if (content?.kind === "pr-list") return i18n.t("tabPrList");
  if (content?.kind === "pr") return i18n.t("tabPr", { number: content.number });
  if (content?.kind === "session-diff") {
    const cwd = cwds[content.session]?.cwd;
    return cwd ? `${i18n.t("tabSessionDiff")} · ${compactPath(cwd)}` : i18n.t("tabSessionDiff");
  }
  if (tab.view) return i18n.t(tab.view);
  const container = containerSession(tab, sessions);
  if (container?.kind.type === "container") {
    const id = container.kind.container_id;
    const known = containerName(id);
    const name = known !== id.slice(0, 12) ? known : container.title.replace(/^sh: /, "");
    return i18n.t("tabContainerShell", { name });
  }
  const cwd = tabCwd(tab, cwds);
  if (cwd) return dirLabel(cwd);
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
  // Rótulo (nome do container) e ponto de não salvo vêm do estado efêmero.
  useContentTabsVersion();

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
    // Sem fundo: a faixa é transparente e a aurora do canvas passa por ela
    // (mockup Proposta/PropostaClaro). Só a aba ativa pinta, com `sunken`.
    <div className="tyba-divide-b relative">
      <div
        ref={faixaRef}
        onScroll={aoRolar}
        onWheel={(e) => {
          const passo = rodaParaHorizontal(e.nativeEvent);
          if (passo === 0 || !faixaRef.current) return;
          faixaRef.current.scrollLeft += passo;
        }}
        className={`tyba-sem-barra flex h-[34px] items-stretch gap-px overflow-x-auto pr-1 ${
          borda.antes ? "tyba-fade-x--ini" : ""
        } ${borda.depois ? "tyba-fade-x--fim" : ""}`}
      >
      {tabs.map((tab, i) => {
        const isActive = tab.id === activeTab;
        // Selo de "parou por sua causa" — decidido em `tabAttention` (testado);
        // aqui só se pinta, no mesmo desenho do ponto da barra lateral.
        const attention = tab.root
          ? tabAttention(leafSessions(tab.root).map((sid) => byId.get(sid)))
          : null;
        const icon = tabIcon(tab, byId);
        return (
          <button
            key={tab.id}
            ref={isActive ? ativaRef : undefined}
            onClick={() => onActivate(tab.id)}
            title={(() => {
              const combo = formatCombo(tabDigitCombo(i + 1));
              const cwd = tabCwd(tab, cwds);
              return cwd ? `${compactPath(cwd)} · ${combo}` : combo;
            })()}
            className={`group relative flex max-w-44 min-w-24 shrink-0 items-center gap-1.5 px-3.5 text-[13px] transition-colors ${
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
            {isActive &&
              (isContainerTab(tab, byId) ? (
                // Aba de container: o filete é azul docker — "isto roda lá".
                <span
                  aria-hidden
                  className="absolute inset-x-0 top-0 h-[2px] rounded-[2px] bg-tyba-blue"
                />
              ) : (
                // A linha viva estática: "esta é a selecionada". A mesma do
                // painel ativo, para os dois lerem como uma coisa só.
                // O filamento cheio, de ponta a ponta da aba (mockup): o
                // `flow-line` some nas pontas e é o fio de quem executa.
                <span
                  aria-hidden
                  className="absolute inset-x-0 top-0 h-[2px]"
                  style={{ background: "var(--tyba-filament)" }}
                />
              ))}
            {icon ? (
              <span
                className={`relative shrink-0 ${
                  isActive ? "text-tyba-text-muted" : "text-tyba-text-faint"
                }`}
              >
                {icon}
                {attention && (
                  <span
                    aria-hidden
                    title={t(attention.labelKey)}
                    className={`absolute -right-0.5 -top-0.5 size-1.5 rounded-full ${attention.dotClass}`}
                  />
                )}
              </span>
            ) : (
              // Aba só texto: o selo de atenção, sem ícone onde pousar, vira
              // um ponto antes do nome.
              attention && (
                <span
                  aria-hidden
                  title={t(attention.labelKey)}
                  className={`size-1.5 shrink-0 rounded-full ${attention.dotClass}`}
                />
              )
            )}
            {attention && (
              <span className="sr-only">{t(attention.labelKey)}</span>
            )}
            <span className="min-w-0 flex-1 truncate text-left font-mono text-[13px]">
              {tabLabel(tab, byId, cwds)}
            </span>
            {isEditorDirty(tab.view) && (
              <span
                role="img"
                aria-label={t("filesDirty")}
                title={t("filesDirty")}
                className="size-[7px] shrink-0 rounded-full bg-tyba-text group-hover:hidden"
              />
            )}
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
