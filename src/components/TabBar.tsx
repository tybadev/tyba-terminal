import { useTranslation } from "react-i18next";
import {
  Plus,
  ShippingContainer,
  SlidersHorizontal,
  TerminalWindow,
  X,
} from "@phosphor-icons/react";

import i18n from "../i18n";
import {
  leafSessions,
  type Session,
  type SessionId,
  type Tab,
  type TabId,
} from "../lib/ipc";

interface Props {
  tabs: Tab[];
  activeTab: TabId | null;
  sessions: Session[];
  onActivate: (id: TabId) => void;
  onClose: (id: TabId) => void;
  onNew: () => void;
}

const VIEW_LABEL_KEYS: Record<string, string> = {
  containers: "containers",
  settings: "settings",
};

function tabIcon(tab: Tab): React.ReactNode {
  if (tab.view === "containers") return <ShippingContainer size={12} />;
  if (tab.view === "settings") return <SlidersHorizontal size={12} />;
  return <TerminalWindow size={12} />;
}

function tabLabel(tab: Tab, sessions: Map<SessionId, Session>): string {
  if (tab.title) return tab.title;
  if (tab.view) return i18n.t(VIEW_LABEL_KEYS[tab.view] ?? tab.view);
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
  onActivate,
  onClose,
  onNew,
}: Props) {
  const { t } = useTranslation();
  const byId = new Map(sessions.map((s) => [s.id, s]));

  return (
    <div className="flex h-8 shrink-0 items-stretch gap-px overflow-x-auto border-b border-tyba-border bg-tyba-surface px-1">
      {tabs.map((tab, i) => {
        const isActive = tab.id === activeTab;
        return (
          <button
            key={tab.id}
            onClick={() => onActivate(tab.id)}
            title={`⌘${i + 1}`}
            className={`group relative flex max-w-44 min-w-24 shrink-0 items-center gap-1.5 rounded-t-[4px] px-2.5 text-[12px] transition-colors ${
              isActive
                ? "bg-tyba-bg text-tyba-text"
                : "text-tyba-text-faint hover:bg-white/[.03] hover:text-tyba-text-muted"
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
              {tabIcon(tab)}
            </span>
            <span className="min-w-0 flex-1 truncate text-left">
              {tabLabel(tab, byId)}
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
        
        className="flex w-8 shrink-0 items-center justify-center rounded-[4px] text-tyba-text-faint transition-colors hover:bg-white/[.03] hover:text-tyba-text"
      >
        <Plus size={13} weight="bold" />
      </button>
    </div>
  );
}
