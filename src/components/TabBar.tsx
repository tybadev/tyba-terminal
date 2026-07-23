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

  return (
    <div className="tyba-divide-b flex h-8 shrink-0 items-stretch gap-px overflow-x-auto bg-tyba-surface px-1">
      {tabs.map((tab, i) => {
        const isActive = tab.id === activeTab;
        return (
          <button
            key={tab.id}
            onClick={() => onActivate(tab.id)}
            title={formatCombo(tabDigitCombo(i + 1))}
            className={`group relative flex max-w-44 min-w-24 shrink-0 items-center gap-1.5 rounded-t-[4px] px-2.5 text-[12px] transition-colors ${
              isActive
                ? "bg-tyba-bg text-tyba-text"
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
  );
}
