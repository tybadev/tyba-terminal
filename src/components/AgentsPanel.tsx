import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowsInSimple, ArrowsOutSimple, TreeStructure, X } from "@phosphor-icons/react";

import type { Session, SubagentRun, SubagentSnapshot } from "@/lib/ipc";
import { orchestratorVisual } from "@/lib/agentsPanel";
import { AgentIcon } from "./icons/AgentIcon";

interface Props {
  session: Session;
  snapshot: SubagentSnapshot | null;
  expanded: boolean;
  onToggleExpand: () => void;
  onClose: () => void;
  onSelect: (agentId: string) => void;
}

function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  if (minutes === 0) return `${seconds}s`;
  return `${minutes}m ${seconds.toString().padStart(2, "0")}s`;
}

function nodeClass(status: SubagentRun["status"], active: boolean): string {
  if (status === "running")
    return "bg-tyba-green [box-shadow:var(--tyba-glow-green)] animate-pulse";
  if (status === "starting") return "bg-tyba-amber animate-pulse";
  if (active) return "bg-tyba-text-muted";
  return "bg-tyba-bg ring-1 ring-inset ring-tyba-text-faint";
}

export function AgentsPanel({
  session,
  snapshot,
  expanded,
  onToggleExpand,
  onClose,
  onSelect,
}: Props) {
  const { t } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  const subagents = snapshot?.subagents ?? [];
  const focused = snapshot?.focused ?? null;
  const anyRunning = subagents.some(
    (s) => s.status === "running" || s.status === "starting",
  );

  useEffect(() => {
    if (!anyRunning) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [anyRunning]);

  const visual = orchestratorVisual(session.status, session.attention, subagents);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-tyba-bg">
      <header className="flex h-8 shrink-0 items-center gap-2 border-b border-tyba-border px-3">
        <TreeStructure size={13} className="shrink-0 text-tyba-text-faint" />
        <span className="min-w-0 truncate text-[12px] text-tyba-text">
          {t("agentsTitle")}
        </span>
        <div className="flex-1" />
        <button
          onClick={onToggleExpand}
          aria-label={t(expanded ? "agentsCollapse" : "agentsExpand")}
          className="text-tyba-text-faint hover:text-tyba-text"
        >
          {expanded ? <ArrowsInSimple size={13} /> : <ArrowsOutSimple size={13} />}
        </button>
        <button
          onClick={onClose}
          aria-label={t("agentsClose")}
          className="text-tyba-text-faint hover:text-tyba-text"
        >
          <X size={13} />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2.5">
        <div className="flex items-center gap-2.5">
          <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-tyba-amber/[.1] text-tyba-amber">
            <AgentIcon size={13} />
          </span>
          <span className="min-w-0 flex-1 truncate text-[12.5px] font-medium text-tyba-text">
            {session.title}
          </span>
          {visual && (
            <span className="flex shrink-0 items-center gap-1.5">
              <span className={`size-1.5 rounded-full ${visual.dotClass}`} />
              <span className={`text-[11px] ${visual.textClass}`}>
                {t(visual.labelKey)}
              </span>
            </span>
          )}
        </div>

        {subagents.length === 0 ? (
          <p className="mt-3 border-l border-tyba-border/60 pl-4 text-[12px] leading-relaxed text-tyba-text-faint">
            {t("agentsEmpty")}
          </p>
        ) : (
          <ul className="mt-1">
            {subagents.map((run, i) => {
              const clickable = run.agent_id != null;
              const active = run.agent_id != null && run.agent_id === focused;
              const isLast = i === subagents.length - 1;
              const duration = formatDuration(
                (run.ended_at_ms ?? now) - run.started_at_ms,
              );
              return (
                <li key={run.agent_id ?? `pending-${i}`} className="flex gap-2.5">
                  <div className="relative flex w-6 shrink-0 justify-center">
                    <span
                      className="absolute top-0 w-px bg-tyba-border"
                      style={{ height: isLast ? "18px" : "100%" }}
                    />
                    <span
                      className={`relative mt-[15px] size-2 shrink-0 rounded-full ${nodeClass(run.status, active)}`}
                    />
                  </div>
                  <button
                    type="button"
                    disabled={!clickable}
                    onClick={() => clickable && onSelect(run.agent_id as string)}
                    className={`group my-1 min-w-0 flex-1 rounded-[6px] px-2 py-1.5 text-left ${
                      active ? "bg-tyba-green/[.07]" : ""
                    } ${
                      clickable
                        ? "hover:bg-tyba-text/[.04]"
                        : "cursor-default opacity-70"
                    }`}
                  >
                    <div className="flex items-baseline gap-2">
                      <span
                        className={`shrink-0 truncate text-[12px] ${active ? "text-tyba-green" : "text-tyba-text"}`}
                      >
                        {run.agent_type || t("agentsUnknownType")}
                      </span>
                      <span className="min-w-0 flex-1 truncate text-[11px] text-tyba-text-faint">
                        {run.description}
                      </span>
                      <span className="shrink-0 font-mono text-[10px] tabular-nums text-tyba-text-faint">
                        {duration}
                      </span>
                    </div>
                    {run.status === "done" && run.summary && (
                      <p className="mt-1 line-clamp-2 border-l border-tyba-border pl-2 text-[11px] leading-snug text-tyba-text-muted">
                        {run.summary}
                      </p>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}
