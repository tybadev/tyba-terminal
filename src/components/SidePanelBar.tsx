import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowsLeftRight,
  ClockCounterClockwise,
  GitDiff,
  TreeView,
  X,
} from "@phosphor-icons/react";

import { Shortcut } from "@/components/ui/kbd";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { AgentIcon } from "./icons/AgentIcon";
import { DockerIcon } from "./icons/DockerIcon";
import { GitIcon } from "./icons/GitIcon";
import type { PanelIndicator, PanelKind } from "../lib/sidePanel";

/**
 * Cor de cada área (classes literais: o Tailwind só gera o que lê no fonte).
 * A tinta, o brilho e o sublinhado do ícone aceso saem de `currentColor`
 * (`--tyba-lit-bg`/`--tyba-lit-glow`), então basta a cor de texto da área.
 */
const AREA: Record<PanelKind, { text: string; bar: string }> = {
  history: { text: "text-tyba-green", bar: "bg-tyba-green" },
  docker: { text: "text-tyba-blue", bar: "bg-tyba-blue" },
  git: { text: "text-tyba-blue", bar: "bg-tyba-blue" },
  files: { text: "text-tyba-cyan", bar: "bg-tyba-cyan" },
  tunnels: { text: "text-tyba-cyan", bar: "bg-tyba-cyan" },
  agents: { text: "text-tyba-amber", bar: "bg-tyba-amber" },
  diff: { text: "text-tyba-violet", bar: "bg-tyba-violet" },
};

/**
 * Marca de verdade onde a coisa tem marca (Docker, Git); Phosphor no que é
 * conceito (histórico, arquivos, diff, túneis). A marca guarda o desenho e
 * pinta com `currentColor`: neutra apagada, cor da área acesa.
 */
export function PanelIcon({
  kind,
  size = 16,
  className,
}: {
  kind: PanelKind;
  size?: number;
  className?: string;
}) {
  switch (kind) {
    case "history":
      return <ClockCounterClockwise size={size} className={className} />;
    case "docker":
      return <DockerIcon size={size} className={className} />;
    case "tunnels":
      return <ArrowsLeftRight size={size} className={className} />;
    case "files":
      return <TreeView size={size} className={className} />;
    case "git":
      return <GitIcon size={size - 1} className={className} />;
    case "diff":
      return <GitDiff size={size} className={className} />;
    case "agents":
      return <AgentIcon size={size} className={className} />;
  }
}

/** Texto da cor da área, para o ícone do cabeçalho do painel. */
export const areaText = (kind: PanelKind) => AREA[kind].text;

const DOT: Record<NonNullable<PanelIndicator["dot"]>, string> = {
  green: "bg-tyba-green",
  amber: "bg-tyba-amber",
  red: "bg-tyba-red",
};

export type PanelTab = {
  kind: PanelKind;
  label: string;
  tooltip: string;
  shortcut?: string;
  indicator: PanelIndicator;
};

/** Os ícones da direita do header: as abas do painel lateral único. */
export function SidePanelBar({
  tabs,
  lit,
  onClick,
  onEscape,
}: {
  tabs: PanelTab[];
  lit: PanelKind | null;
  onClick: (kind: PanelKind) => void;
  /** Esc com o foco num ícone fecha o painel aberto. */
  onEscape: () => void;
}) {
  const { t } = useTranslation();
  if (tabs.length === 0) return null;
  return (
    <nav
      aria-label={t("sidePanelNav")}
      className="flex items-center gap-0.5"
      onKeyDown={(e) => {
        if (e.key === "Escape" && lit) {
          e.preventDefault();
          onEscape();
        }
      }}
    >
      {tabs.map((tab) => {
        const on = tab.kind === lit;
        const area = AREA[tab.kind];
        return (
          <Tooltip key={tab.kind}>
            <TooltipTrigger asChild>
              <button
                type="button"
                aria-label={tab.label}
                aria-pressed={on}
                onClick={() => onClick(tab.kind)}
                className={`relative flex h-7 w-[30px] items-center justify-center rounded-[6px] outline-none focus-visible:ring-2 focus-visible:ring-tyba-cyan ${
                  on
                    ? `${area.text} bg-[var(--tyba-lit-bg)] shadow-[var(--tyba-lit-glow)]`
                    : "text-tyba-text-faint hover:text-tyba-text"
                }`}
              >
                <PanelIcon kind={tab.kind} />
                {tab.indicator.dot && (
                  <span
                    aria-hidden
                    className={`absolute right-[5px] top-1 size-[5px] rounded-full ${DOT[tab.indicator.dot]}`}
                  />
                )}
                {tab.indicator.count !== null && (
                  <span
                    aria-hidden
                    className="absolute right-[3px] top-[3px] flex h-3 min-w-3 items-center justify-center rounded-[6px] bg-tyba-amber px-[3px] text-[9px] leading-none text-tyba-text-inverse"
                  >
                    {tab.indicator.count > 9 ? "9+" : tab.indicator.count}
                  </span>
                )}
                {on && (
                  <span
                    aria-hidden
                    className={`absolute inset-x-[7px] -bottom-px h-0.5 rounded-[1px] ${area.bar}`}
                  />
                )}
              </button>
            </TooltipTrigger>
            <TooltipContent side="bottom" className="flex items-center gap-2">
              {tab.tooltip}
              {tab.shortcut && <Shortcut combo={tab.shortcut} />}
            </TooltipContent>
          </Tooltip>
        );
      })}
    </nav>
  );
}

/**
 * Cabeçalho comum dos painéis (mockup): ícone da área, título, extras e ✕.
 * `padding: 12px 16px`, linha fina embaixo.
 */
export function SidePanelHeader({
  kind,
  title,
  children,
  onClose,
}: {
  kind: PanelKind;
  title: string;
  children?: ReactNode;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  return (
    <header className="flex shrink-0 items-center gap-2.5 border-b border-tyba-border px-4 py-3">
      <PanelIcon kind={kind} size={16} className={`shrink-0 ${areaText(kind)}`} />
      <span className="min-w-0 flex-1 truncate text-[13px] text-tyba-text">
        {title}
      </span>
      {children}
      <button
        type="button"
        onClick={onClose}
        aria-label={t("sidePanelClose")}
        className="rounded-[4px] text-tyba-text-faint outline-none hover:text-tyba-text focus-visible:ring-2 focus-visible:ring-tyba-cyan"
      >
        <X size={13} />
      </button>
    </header>
  );
}

/** Seção do painel com a linha fina de baixo, mais quieta que a do cabeçalho. */
export const PANEL_SECTION = "shrink-0 border-b border-[var(--tyba-border-soft)]";

/** Linha elevada (selecionada/destacada): fundo raised + aresta de luz. */
export const RAISED_ROW = "bg-tyba-raised shadow-[var(--tyba-row-raised)]";
