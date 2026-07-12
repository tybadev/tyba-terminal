import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowSquareOut,
  ArrowsClockwise,
  CaretDown,
  CaretRight,
  FileCode,
  FileText,
  FolderOpen,
  Play,
  ShippingContainer,
  Stop,
  TerminalWindow,
  Trash,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { IS_MAC } from "@/lib/platform";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  dockerComposeOp,
  dockerListContainers,
  dockerOpenComposeFile,
  dockerOpenDesktop,
  dockerOpenLogs,
  dockerOpenProject,
  dockerOpenShell,
  dockerRemoveContainer,
  type ComposeOp,
  type ContainerInfo,
} from "../lib/ipc";

const REFRESH_MS = 3000;
const LOOSE_KEY = "__loose__";

interface Props {
  onRunningChange?: (running: boolean) => void;
  onAvailableChange?: (available: boolean) => void;
}

interface ProjectGroup {
  key: string;
  project: string | null;
  workingDir: string | null;
  hasComposeFile: boolean;
  items: ContainerInfo[];
}

function PanelAction({
  label,
  onClick,
  destructive,
  children,
}: {
  label: string;
  onClick: () => void;
  destructive?: boolean;
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label={label}
          onClick={onClick}
          className={`rounded-[3px] p-1 transition-colors ${
            destructive
              ? "text-tyba-text-faint hover:text-tyba-red"
              : "text-tyba-text-faint hover:text-tyba-text"
          }`}
        >
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}

export function ContainersView({
  onRunningChange,
  onAvailableChange,
}: Props) {
  const { t } = useTranslation();
  const [containers, setContainers] = useState<ContainerInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [collapsedOverrides, setCollapsedOverrides] = useState<
    Record<string, boolean>
  >({});
  const [stoppedShown, setStoppedShown] = useState<Record<string, boolean>>({});
  const [confirmingRm, setConfirmingRm] = useState<string | null>(null);
  const [confirmingDown, setConfirmingDown] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const list = await dockerListContainers(null, true);
      setContainers(list);
      setError(null);
      onAvailableChange?.(true);
      onRunningChange?.(list.some((c) => c.state === "running"));
    } catch (e) {
      setError(String(e));
      onAvailableChange?.(false);
    }
  }, [onAvailableChange, onRunningChange]);

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [load]);

  const groups = useMemo<ProjectGroup[]>(() => {
    if (!containers) return [];
    const byKey = new Map<string, ContainerInfo[]>();
    for (const c of containers) {
      const key = c.compose_project ?? LOOSE_KEY;
      byKey.set(key, [...(byKey.get(key) ?? []), c]);
    }
    const result: ProjectGroup[] = [...byKey.entries()].map(([key, items]) => {
      const workingDir =
        items.find((c) => c.compose_working_dir)?.compose_working_dir ?? null;
      return {
        key,
        project: key === LOOSE_KEY ? null : key,
        workingDir,
        hasComposeFile: items.some((c) => c.config_files),
        items,
      };
    });
    result.sort((a, b) => {
      if ((a.project === null) !== (b.project === null)) {
        return a.project === null ? 1 : -1;
      }
      return (a.project ?? "").localeCompare(b.project ?? "");
    });
    return result;
  }, [containers]);

  const openTab = useCallback((kind: "logs" | "shell", id: string) => {
    const call = kind === "logs" ? dockerOpenLogs : dockerOpenShell;
    void call(id).catch((e) => setError(String(e)));
  }, []);

  const removeContainer = useCallback(
    (id: string) => {
      if (confirmingRm !== id) {
        setConfirmingRm(id);
        setConfirmingDown(null);
        return;
      }
      setConfirmingRm(null);
      setRemoving(id);
      void dockerRemoveContainer(id)
        .catch((e) => setError(String(e)))
        .finally(() => {
          setRemoving(null);
          void load();
        });
    },
    [confirmingRm, load],
  );

  const composeOp = useCallback(
    (project: string, op: ComposeOp) => {
      if (op === "down" && confirmingDown !== project) {
        setConfirmingDown(project);
        setConfirmingRm(null);
        return;
      }
      setConfirmingDown(null);
      void dockerComposeOp(project, op).catch((e) => setError(String(e)));
    },
    [confirmingDown],
  );

  const renderContainer = (c: ContainerInfo) => {
    const isRunning = c.state === "running";
    return (
      <div
        key={c.id}
        className={`group rounded-md px-2 py-1.5 transition-colors hover:bg-tyba-text/[.03] ${
          removing === c.id ? "opacity-40" : ""
        }`}
      >
        <div className="flex items-center gap-2">
          <span
            className={`size-1.5 shrink-0 rounded-full ${
              isRunning
                ? "bg-tyba-green [box-shadow:var(--tyba-glow-green)]"
                : "bg-tyba-text-faint"
            }`}
          />
          <ShippingContainer
            size={14}
            className={`shrink-0 ${
              isRunning ? "text-tyba-text-muted" : "text-tyba-text-faint"
            }`}
          />
          <span className="min-w-0 flex-1 truncate text-[12px] text-tyba-text">
            {c.service ?? c.name}
          </span>
          <span className="max-w-24 truncate font-mono text-[10px] text-tyba-text-faint">
            {c.ports.split(",")[0]?.trim().replace("0.0.0.0:", ":") || c.image}
          </span>
        </div>
        <div className="flex items-center gap-2 pl-9">
          <span className="min-w-0 flex-1 truncate font-mono text-[10px] leading-5 text-tyba-text-faint">
            {c.status}
          </span>
          <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
            <PanelAction
              label={t("containerLogs")}
              onClick={() => openTab("logs", c.id)}
            >
              <FileText size={13} />
            </PanelAction>
            <PanelAction
              label={t("containerShell")}
              onClick={() => openTab("shell", c.id)}
            >
              <TerminalWindow size={13} />
            </PanelAction>
            {confirmingRm === c.id ? (
              <button
                onClick={() => removeContainer(c.id)}
                className="rounded-[3px] bg-tyba-red px-1.5 py-0.5 text-[10px] font-medium text-white hover:bg-tyba-red/90"
              >
                {t("containerRemoveConfirm")}
              </button>
            ) : (
              <PanelAction
                label={t("containerRemove")}
                destructive
                onClick={() => removeContainer(c.id)}
              >
                <Trash size={13} />
              </PanelAction>
            )}
          </span>
        </div>
      </div>
    );
  };

  const renderGroup = (group: ProjectGroup) => {
    const collapsed = collapsedOverrides[group.key] ?? false;
    const running = group.items.filter((c) => c.state === "running");
    const stopped = group.items.filter((c) => c.state !== "running");
    const showStopped = stoppedShown[group.key] ?? false;
    return (
      <div key={group.key} className="flex flex-col gap-px">
        <button
          onClick={() =>
            setCollapsedOverrides((prev) => ({
              ...prev,
              [group.key]: !collapsed,
            }))
          }
          className="flex h-7 items-center gap-1.5 rounded-[4px] px-1.5 text-left transition-colors hover:bg-tyba-text/[.03]"
        >
          {collapsed ? (
            <CaretRight size={10} className="shrink-0 text-tyba-text-faint" />
          ) : (
            <CaretDown size={10} className="shrink-0 text-tyba-text-faint" />
          )}
          <span className="min-w-0 flex-1 truncate text-[11px] font-medium uppercase tracking-[0.1em] text-tyba-text-faint">
            {group.project ?? t("containersLoose")}
          </span>
          {running.length > 0 && (
            <span className="flex items-center gap-1 font-mono text-[10px] text-tyba-green">
              <span className="size-1 rounded-full bg-tyba-green" />
              {running.length}
            </span>
          )}
          {running.length === 0 && (
            <span className="font-mono text-[10px] text-tyba-text-faint">
              {group.items.length}
            </span>
          )}
        </button>
        {!collapsed && (
          <>
            {group.project && group.workingDir && (
              <div className="flex items-center gap-0.5 px-1.5 pb-0.5">
                <PanelAction
                  label={t("composeUp")}
                  onClick={() => composeOp(group.project as string, "up")}
                >
                  <Play size={12} />
                </PanelAction>
                <PanelAction
                  label={t("composeRestart")}
                  onClick={() => composeOp(group.project as string, "restart")}
                >
                  <ArrowsClockwise size={12} />
                </PanelAction>
                {confirmingDown === group.project ? (
                  <button
                    onClick={() => composeOp(group.project as string, "down")}
                    className="rounded-[3px] bg-tyba-amber px-1.5 py-0.5 text-[10px] font-medium text-tyba-text-inverse hover:bg-tyba-amber/90"
                  >
                    {t("composeDownConfirm")}
                  </button>
                ) : (
                  <PanelAction
                    label={t("composeDown")}
                    destructive
                    onClick={() => composeOp(group.project as string, "down")}
                  >
                    <Stop size={12} />
                  </PanelAction>
                )}
                <span className="flex-1" />
                <PanelAction
                  label={t("openProject")}
                  onClick={() =>
                    void dockerOpenProject(group.project as string).catch((e) =>
                      setError(String(e)),
                    )
                  }
                >
                  <FolderOpen size={12} />
                </PanelAction>
                {group.hasComposeFile && (
                  <PanelAction
                    label={t("viewCompose")}
                    onClick={() =>
                      void dockerOpenComposeFile(group.project as string).catch(
                        (e) => setError(String(e)),
                      )
                    }
                  >
                    <FileCode size={12} />
                  </PanelAction>
                )}
              </div>
            )}
            {running.map(renderContainer)}
            {stopped.length > 0 && (
              <button
                onClick={() =>
                  setStoppedShown((prev) => ({
                    ...prev,
                    [group.key]: !showStopped,
                  }))
                }
                className="flex h-6 items-center justify-center gap-1 rounded-md text-[10px] text-tyba-text-faint transition-colors hover:bg-tyba-text/[.03] hover:text-tyba-text-muted"
              >
                {showStopped
                  ? t("containersHideStopped")
                  : t("containersShowStopped", { count: stopped.length })}
              </button>
            )}
            {showStopped && stopped.map(renderContainer)}
          </>
        )}
      </div>
    );
  };

  return (
    <div className="flex min-h-0 flex-1 justify-center overflow-y-auto">
      <div className="flex w-full max-w-xl flex-col px-6 pt-5 pb-8">
        <div className="flex items-center justify-between pb-3">
          <span className="tyba-label">{t("containers")}</span>
          {IS_MAC && error === null && (
            <button
              onClick={() =>
                void dockerOpenDesktop().catch((e) => setError(String(e)))
              }
              className="flex items-center gap-1 text-[11px] text-tyba-text-faint transition-colors hover:text-tyba-text"
            >
              {t("openDockerDesktop")}
              <ArrowSquareOut size={11} />
            </button>
          )}
        </div>

      {error !== null ? (
        <div className="flex flex-col items-center gap-3 px-3 py-6 text-center">
          <p className="text-xs text-tyba-text-faint">
            {t("containersDaemonOff")}
          </p>
          <p className="max-w-full truncate font-mono text-[10px] text-tyba-text-faint">
            {error}
          </p>
          {IS_MAC && (
            <Button
              size="sm"
              variant="outline"
              onClick={() =>
                void dockerOpenDesktop().catch((e) => setError(String(e)))
              }
              className="h-6 rounded-[4px] px-2.5 text-[11px] text-tyba-text-muted"
            >
              <ArrowSquareOut size={12} />
              {t("openDockerDesktop")}
            </Button>
          )}
        </div>
      ) : containers === null ? (
        <div className="flex flex-col gap-1 p-2">
          {[0, 1, 2].map((i) => (
            <div
              key={i}
              className="h-9 animate-pulse rounded-md bg-tyba-text/[.03]"
            />
          ))}
        </div>
      ) : groups.length === 0 ? (
        <p className="px-3 py-6 text-center text-xs text-tyba-text-faint">
          {t("containersEmptyAll")}
        </p>
      ) : (
        <div className="flex flex-col gap-1.5">
          {groups.map(renderGroup)}
        </div>
      )}
      </div>
    </div>
  );
}
