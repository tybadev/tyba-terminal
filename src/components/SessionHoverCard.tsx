import { useTranslation } from "react-i18next";
import { Folder, GitBranch, SquaresFour } from "@phosphor-icons/react";

import { HoverCardContent } from "@/components/ui/hover-card";

export interface SessionHoverCardProps {
  name: string;
  path?: string | null;
  branch?: string | null;
  changed?: number;
  runner?: string | null;
  runningCommand?: string | null;
  tabs: number;
  group?: string | null;
  color?: string | null;
  side: "right" | "bottom";
}

export function SessionHoverCard({
  name,
  path,
  branch,
  changed,
  runner,
  runningCommand,
  tabs,
  group,
  color,
  side,
}: SessionHoverCardProps) {
  const { t } = useTranslation();
  const running = Boolean(runningCommand);

  return (
    <HoverCardContent side={side} className="p-0">
      <div className="flex items-center gap-2 border-b border-tyba-border px-3 py-2">
        <span
          className={
            running
              ? "size-1.5 shrink-0 rounded-full bg-tyba-green [box-shadow:var(--tyba-glow-green)] motion-safe:animate-pulse"
              : "size-1.5 shrink-0 rounded-full bg-tyba-text-faint"
          }
        />
        <span className="text-[11px] uppercase tracking-[var(--tyba-tracking-wide)] text-tyba-text-muted">
          {running ? t("sessionRunning") : t("sessionIdle")}
        </span>
        {runner && (
          <span className="ml-auto rounded-[3px] bg-tyba-violet-tint px-1.5 py-0.5 font-mono text-[10px] leading-none text-tyba-violet">
            {runner}
          </span>
        )}
      </div>

      <div className="space-y-1.5 px-3 py-2.5">
        <div className="flex items-center gap-2">
          {color && (
            <span
              className="size-2 shrink-0 rounded-full"
              style={{ background: `var(--tyba-${color})` }}
            />
          )}
          <span className="min-w-0 flex-1 truncate text-[13px] font-medium leading-none text-tyba-text">
            {name}
          </span>
          {group && (
            <span className="shrink-0 truncate rounded-[3px] bg-tyba-neutral-tint px-1.5 py-0.5 text-[10px] leading-none text-tyba-text-muted">
              {group}
            </span>
          )}
        </div>

        {runningCommand && (
          <p className="truncate font-mono text-[11px] leading-relaxed text-tyba-text-muted">
            {runningCommand}
          </p>
        )}

        {path && (
          <p className="flex items-center gap-1.5 text-tyba-text-faint">
            <Folder size={12} className="shrink-0" />
            <span className="min-w-0 truncate font-mono text-[11px] leading-none">
              {path}
            </span>
          </p>
        )}
      </div>

      {(branch || changed || tabs > 0) && (
        <div className="flex items-center gap-3 border-t border-tyba-border px-3 py-1.5 text-[10px] text-tyba-text-faint">
          {branch && (
            <span className="flex min-w-0 items-center gap-1">
              <GitBranch size={11} className="shrink-0" />
              <span className="truncate font-mono">{branch}</span>
            </span>
          )}
          {changed ? (
            <span className="shrink-0 font-mono text-tyba-amber">
              ±{changed}
            </span>
          ) : null}
          <span className="ml-auto flex shrink-0 items-center gap-1">
            <SquaresFour size={11} />
            {t("tabsCount", { count: tabs })}
          </span>
        </div>
      )}
    </HoverCardContent>
  );
}
