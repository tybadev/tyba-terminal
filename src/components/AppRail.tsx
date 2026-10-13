import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  ClockCounterClockwise,
  Files,
  GitDiff,
  GitPullRequest,
  Lightning,
  type Icon,
} from "@phosphor-icons/react";

import "./gitScreen.css";
import { Shortcut } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { PanelIcon, type PanelTab } from "./SidePanelBar";
import type { GitSection } from "../lib/gitScreen";
import type { PanelIndicator, PanelKind } from "../lib/sidePanel";

const AREA_TEXT: Record<PanelKind, string> = {
  history: "text-tyba-green",
  docker: "text-tyba-blue",
  git: "text-tyba-blue",
  files: "text-tyba-cyan",
  tunnels: "text-tyba-cyan",
  agents: "text-tyba-amber",
  diff: "text-tyba-violet",
};
const AREA_BAR: Record<PanelKind, string> = {
  history: "bg-tyba-green",
  docker: "bg-tyba-blue",
  git: "bg-tyba-blue",
  files: "bg-tyba-cyan",
  tunnels: "bg-tyba-cyan",
  agents: "bg-tyba-amber",
  diff: "bg-tyba-violet",
};
const DOT: Record<NonNullable<PanelIndicator["dot"]>, string> = {
  green: "bg-tyba-green",
  amber: "bg-tyba-amber",
  red: "bg-tyba-red",
};

const SECTION: Record<GitSection, { icon: Icon; label: string }> = {
  changes: { icon: Files, label: "gitScreenChanges" },
  review: { icon: GitDiff, label: "gitScreenReview" },
  history: { icon: ClockCounterClockwise, label: "gitScreenHistory" },
  prs: { icon: GitPullRequest, label: "gitScreenPrs" },
  ci: { icon: Lightning, label: "gitScreenCi" },
};

export type RailGit = {
  sections: GitSection[];
  active: GitSection;
  changed: number;
  ciFailing: boolean;
  onPick: (section: GitSection) => void;
};

/**
 * Trilho único na borda direita da janela: as abas do painel lateral (antes
 * no header). Com a tela de Git aberta, as seções dela aparecem embaixo do
 * item Git — um trilho só. O marcador corre até o item ativo.
 */
export function AppRail({
  tabs,
  lit,
  git,
  onClick,
  onEscape,
  footer,
}: {
  tabs: PanelTab[];
  lit: PanelKind | null;
  git: RailGit | null;
  onClick: (kind: PanelKind) => void;
  onEscape: () => void;
  footer?: ReactNode;
}) {
  const { t } = useTranslation();
  const navRef = useRef<HTMLElement>(null);
  const [marker, setMarker] = useState<{ top: number; height: number; bar: string } | null>(null);

  const activeKey = git ? `git:${git.active}` : lit ? `panel:${lit}` : null;
  useLayoutEffect(() => {
    const nav = navRef.current;
    const el = activeKey ? nav?.querySelector<HTMLElement>(`[data-rail="${activeKey}"]`) : null;
    if (!nav || !el) {
      setMarker(null);
      return;
    }
    setMarker({
      top: el.offsetTop + 4,
      height: el.offsetHeight - 8,
      bar: git ? "bg-tyba-blue" : lit ? AREA_BAR[lit] : "bg-tyba-blue",
    });
  }, [activeKey, git, lit, tabs.length]);

  if (tabs.length === 0) return null;

  return (
    <nav
      ref={navRef}
      aria-label={t("sidePanelNav")}
      onKeyDown={(e) => {
        if (e.key === "Escape" && (lit || git)) {
          e.preventDefault();
          onEscape();
        }
      }}
      className="relative z-40 flex w-[52px] shrink-0 flex-col items-center gap-1.5 tyba-divide-l bg-[var(--tyba-panel-bg)] py-3"
    >
      {marker && (
        <span
          aria-hidden
          className={`tyba-git-rail-marker absolute right-0 w-0.5 rounded-l-[2px] ${marker.bar}`}
          style={{ top: marker.top, height: marker.height }}
        />
      )}
      {tabs.map((tab) => {
        const on = tab.kind === lit;
        return (
          <div key={tab.kind} className="flex flex-col items-center gap-1">
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  data-rail={`panel:${tab.kind}`}
                  aria-label={tab.label}
                  aria-pressed={on}
                  onClick={() => onClick(tab.kind)}
                  className={`relative flex size-9 items-center justify-center rounded-[8px] outline-none transition-colors focus-visible:ring-2 focus-visible:ring-tyba-cyan ${
                    on
                      ? `${AREA_TEXT[tab.kind]} bg-[var(--tyba-lit-bg)] shadow-[var(--tyba-lit-glow)]`
                      : "text-tyba-text-faint hover:text-tyba-text"
                  }`}
                >
                  <PanelIcon kind={tab.kind} size={17} />
                  {tab.indicator.dot && (
                    <span
                      aria-hidden
                      className={`absolute right-1 top-1 size-[5px] rounded-full ${DOT[tab.indicator.dot]}`}
                    />
                  )}
                  {tab.indicator.count !== null && (
                    <span
                      aria-hidden
                      className="absolute right-0.5 top-0.5 flex h-3 min-w-3 items-center justify-center rounded-[6px] bg-tyba-amber px-[3px] text-[9px] leading-none text-tyba-text-inverse"
                    >
                      {tab.indicator.count > 9 ? "9+" : tab.indicator.count}
                    </span>
                  )}
                </button>
              </TooltipTrigger>
              <TooltipContent side="left" className="flex items-center gap-2">
                {tab.tooltip}
                {tab.shortcut && <Shortcut combo={tab.shortcut} />}
              </TooltipContent>
            </Tooltip>

            {tab.kind === "git" && git && (
              <div className="flex flex-col items-center gap-0.5 rounded-[10px] bg-tyba-text/[.04] p-1">
                {git.sections.map((section) => {
                  const { icon: SectionIcon, label } = SECTION[section];
                  const sectionOn = section === git.active;
                  return (
                    <Tooltip key={section}>
                      <TooltipTrigger asChild>
                        <button
                          type="button"
                          data-rail={`git:${section}`}
                          aria-label={t(label)}
                          aria-pressed={sectionOn}
                          onClick={() => git.onPick(section)}
                          className={`relative flex size-8 items-center justify-center rounded-[7px] outline-none transition-colors focus-visible:ring-2 focus-visible:ring-tyba-cyan ${
                            sectionOn
                              ? "bg-tyba-blue/[.14] text-tyba-blue"
                              : "text-tyba-text-faint hover:text-tyba-text"
                          }`}
                        >
                          <SectionIcon size={15} />
                          {section === "changes" && git.changed > 0 && (
                            <span
                              aria-hidden
                              className="absolute -right-0.5 -top-0.5 flex h-3 min-w-3 items-center justify-center rounded-[6px] bg-tyba-amber px-[3px] text-[9px] leading-none text-tyba-text-inverse"
                            >
                              {git.changed > 99 ? "99+" : git.changed}
                            </span>
                          )}
                          {section === "ci" && git.ciFailing && (
                            <span aria-hidden className="absolute right-1 top-1 size-[5px] rounded-full bg-tyba-red" />
                          )}
                        </button>
                      </TooltipTrigger>
                      <TooltipContent side="left">{t(label)}</TooltipContent>
                    </Tooltip>
                  );
                })}
              </div>
            )}
          </div>
        );
      })}
      <span className="flex-1" />
      {footer}
    </nav>
  );
}
