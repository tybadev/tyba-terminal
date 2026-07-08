import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowSquareOut,
  FileText,
  TerminalWindow,
  Trash,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { DockerIcon } from "./icons/DockerIcon";
import {
  dockerAvailable,
  dockerListContainers,
  dockerOpenDesktop,
  dockerOpenLogs,
  dockerOpenShell,
  dockerRemoveContainer,
  type ContainerInfo,
  type WorkspaceId,
} from "../lib/ipc";

const REFRESH_MS = 3000;
const HEALTHCHECK_MS = 30_000;
const IS_MAC = navigator.platform.toUpperCase().includes("MAC");

interface Props {
  repoRoot: string | null;
  workspaceId: WorkspaceId | null;
  projectName: string | null;
}

function RowAction({
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

export function ContainersPopover({ repoRoot, workspaceId, projectName }: Props) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [available, setAvailable] = useState(true);
  const [containers, setContainers] = useState<ContainerInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(false);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [showStopped, setShowStopped] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const check = () =>
      void dockerAvailable()
        .then((ok) => {
          if (!cancelled) setAvailable(ok);
        })
        .catch(() => {
          if (!cancelled) setAvailable(false);
        });
    check();
    const timer = window.setInterval(check, HEALTHCHECK_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const effectiveAll = showAll || !repoRoot;

  const load = useCallback(async () => {
    try {
      const list = await dockerListContainers(
        effectiveAll ? null : repoRoot,
        effectiveAll,
      );
      setContainers(list);
      setError(null);
      setAvailable(true);
    } catch (e) {
      setError(String(e));
      setAvailable(false);
    }
  }, [effectiveAll, repoRoot]);

  useEffect(() => {
    if (!open) return;
    void load();
    const timer = window.setInterval(() => void load(), REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [open, load]);

  const openTab = useCallback(
    (kind: "logs" | "shell", id: string) => {
      const call = kind === "logs" ? dockerOpenLogs : dockerOpenShell;
      void call(id, workspaceId).catch((e) => setError(String(e)));
      setOpen(false);
    },
    [workspaceId],
  );

  const remove = useCallback(
    (id: string) => {
      if (confirming !== id) {
        setConfirming(id);
        return;
      }
      setConfirming(null);
      setRemoving(id);
      void dockerRemoveContainer(id)
        .catch((e) => setError(String(e)))
        .finally(() => {
          setRemoving(null);
          void load();
        });
    },
    [confirming, load],
  );

  const running = containers?.filter((c) => c.state === "running") ?? [];
  const stopped = containers?.filter((c) => c.state !== "running") ?? [];

  const renderContainer = (c: ContainerInfo) => {
    const isRunning = c.state === "running";
    return (
      <div
        key={c.id}
        className={`group rounded-md px-2 py-1.5 transition-colors hover:bg-white/[.03] ${
          removing === c.id ? "opacity-40" : ""
        }`}
      >
        <div className="flex items-center gap-2">
          <span
            className={`size-1.5 shrink-0 rounded-full ${
              isRunning
                ? "bg-tyba-green [box-shadow:0_0_5px_rgba(124,197,68,.5)]"
                : "bg-tyba-text-faint"
            }`}
          />
          <span className="min-w-0 flex-1 truncate text-[12px] text-tyba-text">
            {c.name}
          </span>
          <span className="max-w-28 truncate font-mono text-[10px] text-tyba-text-faint">
            {c.image}
          </span>
        </div>
        <div className="flex items-center gap-2 pl-3.5">
          <span className="min-w-0 flex-1 truncate font-mono text-[10px] leading-5 text-tyba-text-faint">
            {c.status}
            {c.ports ? ` · ${c.ports}` : ""}
          </span>
          <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
            <RowAction
              label={t("containerLogs")}
              onClick={() => openTab("logs", c.id)}
            >
              <FileText size={13} />
            </RowAction>
            <RowAction
              label={t("containerShell")}
              onClick={() => openTab("shell", c.id)}
            >
              <TerminalWindow size={13} />
            </RowAction>
            {confirming === c.id ? (
              <button
                onClick={() => remove(c.id)}
                className="rounded-[3px] bg-tyba-red px-1.5 py-0.5 text-[10px] font-medium text-white hover:bg-tyba-red/90"
              >
                {t("containerRemoveConfirm")}
              </button>
            ) : (
              <RowAction
                label={t("containerRemove")}
                destructive
                onClick={() => remove(c.id)}
              >
                <Trash size={13} />
              </RowAction>
            )}
          </span>
        </div>
      </div>
    );
  };

  return (
    <DropdownMenu
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        setConfirming(null);
        setShowStopped(false);
        if (next) setError(null);
      }}
    >
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          aria-label={t("containers")}
          title={available ? undefined : t("dockerUnavailable")}
          className={`relative size-6 rounded-[4px] ${
            available
              ? "text-tyba-text-muted hover:text-tyba-text"
              : "text-tyba-text-faint"
          }`}
        >
          <DockerIcon size={16} />
          {!available ? (
            <span className="absolute right-0.5 top-0.5 size-1.5 rounded-full bg-tyba-red [box-shadow:0_0_6px_rgba(239,68,68,.6)]" />
          ) : (
            running.length > 0 && (
              <span className="absolute right-0.5 top-0.5 size-1.5 rounded-full bg-tyba-green [box-shadow:0_0_6px_rgba(124,197,68,.55)]" />
            )
          )}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="w-80 border-tyba-border-strong bg-tyba-overlay shadow-lg"
      >
        <DropdownMenuLabel className="tyba-label flex items-center gap-1.5">
          {t("containers")}
          {!effectiveAll && projectName && (
            <span className="min-w-0 truncate normal-case tracking-normal text-tyba-text-faint">
              · {projectName}
            </span>
          )}
        </DropdownMenuLabel>

        {error !== null ? (
          <div className="flex flex-col items-center gap-3 px-3 py-5 text-center">
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
                onClick={() => void dockerOpenDesktop().catch(() => {})}
                className="h-6 rounded-[4px] px-2.5 text-[11px] text-tyba-text-muted"
              >
                <ArrowSquareOut size={12} />
                {t("openDockerDesktop")}
              </Button>
            )}
          </div>
        ) : containers === null ? (
          <div className="flex flex-col gap-1 p-1">
            {[0, 1, 2].map((i) => (
              <div
                key={i}
                className="h-11 animate-pulse rounded-md bg-white/[.03]"
              />
            ))}
          </div>
        ) : containers.length === 0 ? (
          <div className="flex flex-col items-center gap-2 px-3 py-5 text-center">
            <p className="text-xs text-tyba-text-faint">
              {effectiveAll ? t("containersEmptyAll") : t("containersEmpty")}
            </p>
            {!effectiveAll && (
              <button
                onClick={() => setShowAll(true)}
                className="text-[11px] text-tyba-text-muted underline-offset-2 hover:text-tyba-text hover:underline"
              >
                {t("showAllContainers")}
              </button>
            )}
          </div>
        ) : (
          <div className="flex max-h-80 flex-col gap-px overflow-y-auto p-1">
            {running.map(renderContainer)}
            {running.length === 0 && (
              <p className="px-2 py-2 text-center text-xs text-tyba-text-faint">
                {t("containersNoneRunning")}
              </p>
            )}
            {stopped.length > 0 && (
              <button
                onClick={() => setShowStopped((v) => !v)}
                className="flex h-7 items-center justify-center gap-1 rounded-md text-[11px] text-tyba-text-faint transition-colors hover:bg-white/[.03] hover:text-tyba-text-muted"
              >
                {showStopped
                  ? t("containersHideStopped")
                  : t("containersShowStopped", { count: stopped.length })}
              </button>
            )}
            {showStopped && stopped.map(renderContainer)}
          </div>
        )}

        {error === null && repoRoot && (
          <div className="flex items-center justify-between border-t border-tyba-border px-2 py-1.5">
            <label className="flex cursor-pointer items-center gap-1.5 text-[11px] text-tyba-text-muted">
              <input
                type="checkbox"
                checked={showAll}
                onChange={(e) => setShowAll(e.target.checked)}
                className="size-3 accent-[var(--tyba-green)]"
              />
              {t("showAllContainers")}
            </label>
            {IS_MAC && (
              <button
                onClick={() => void dockerOpenDesktop().catch(() => {})}
                className="flex items-center gap-1 text-[11px] text-tyba-text-faint transition-colors hover:text-tyba-text"
              >
                {t("openDockerDesktop")}
                <ArrowSquareOut size={11} />
              </button>
            )}
          </div>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
