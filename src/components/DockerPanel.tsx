import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowClockwise,
  CaretDown,
  CaretRight,
  FolderSimple,
  Heartbeat,
  MagnifyingGlass,
  Play,
  Scroll,
  Stop,
  TerminalWindow,
  Trash,
  WarningCircle,
} from "@phosphor-icons/react";

import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { PANEL_SECTION, RAISED_ROW, SidePanelHeader } from "./SidePanelBar";
import { requestConfirm } from "../lib/confirm";
import { openContainerTab } from "../lib/contentTabs";
import {
  compactSpan,
  containerCounts,
  displayName,
  exitCode,
  exitReason,
  exitedAgoMs,
  formatBytes,
  formatCpu,
  groupContainers,
  healthOf,
  portList,
  shortImage,
  uptimeMs,
  type ContainerGroup,
} from "../lib/dockerPanel";
import { translateError } from "../lib/errors";
import {
  dockerAvailable,
  dockerContainerOp,
  dockerListContainers,
  dockerOpenDesktop,
  dockerPanelUsage,
  dockerRemoveContainer,
  type ContainerInfo,
  type ContainerOp,
  type PanelUsage,
} from "../lib/ipc";

/** Mesma cadência do painel completo: remoto é rede, local é barato. */
const LOCAL_REFRESH_MS = 3000;
const REMOTE_REFRESH_MS = 15000;
/** CPU/RAM: `docker stats` amostra ~2s; o core ainda segura em ≥5s. */
const USAGE_REFRESH_MS = 5000;
/** Acima disso aparece a busca. */
const FILTER_THRESHOLD = 8;
const LOOSE_KEY = "\u0000avulsos";

const groupKey = (g: ContainerGroup) => g.project ?? LOOSE_KEY;

const TEXT_BUTTON =
  "rounded-[5px] border border-tyba-border-strong px-[9px] py-[3px] text-[11px] text-tyba-text-muted outline-none hover:text-tyba-text focus-visible:ring-2 focus-visible:ring-tyba-cyan";

function IconButton({
  label,
  onClick,
  disabled,
  danger,
  children,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
  children: ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={label}
          disabled={disabled}
          onClick={(e) => {
            e.stopPropagation();
            onClick();
          }}
          className={`flex size-6 items-center justify-center rounded-[5px] outline-none focus-visible:ring-2 focus-visible:ring-tyba-cyan disabled:opacity-40 ${
            danger
              ? "text-tyba-text-muted hover:bg-tyba-red/[.12] hover:text-tyba-red"
              : "text-tyba-text-muted hover:bg-tyba-text/[.06] hover:text-tyba-text"
          }`}
        >
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}

/**
 * Painel Docker: os containers do alvo da sessão — a máquina local, ou o
 * servidor numa sessão SSH (`host` = alias). Feito para escala: agrupado por
 * projeto compose (o do repo aberto primeiro), rodando antes, parados
 * recolhidos, antigos escondidos, uma linha por container e ações só na
 * linha selecionada. O painel completo segue no workspace Docker.
 */
export function DockerPanel({
  host,
  repoRoot,
  onOpenDashboard,
  onClose,
}: {
  host: string | null;
  repoRoot: string | null;
  onOpenDashboard: () => void;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [available, setAvailable] = useState<boolean | null>(null);
  const [list, setList] = useState<ContainerInfo[] | null>(null);
  const [usage, setUsage] = useState<PanelUsage | null>(null);
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // Só o que o usuário mexeu; o resto segue o padrão (grupo do repo aberto,
  // parados e antigos recolhidos).
  const [openGroups, setOpenGroups] = useState<Record<string, boolean>>({});
  const [openStopped, setOpenStopped] = useState<Record<string, boolean>>({});
  const [openOld, setOpenOld] = useState<Record<string, boolean>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const ok = await dockerAvailable(host);
      setAvailable(ok);
      if (!ok) {
        setList([]);
        return;
      }
      // Tudo do alvo; `repoRoot` só marca (no core) qual grupo é o do repo.
      setList(await dockerListContainers(host ? null : repoRoot, true, host));
    } catch (e) {
      setError(translateError(e, t));
    }
  }, [host, repoRoot, t]);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(
      () => void refresh(),
      host ? REMOTE_REFRESH_MS : LOCAL_REFRESH_MS,
    );
    return () => window.clearInterval(timer);
  }, [refresh, host]);

  // CPU/RAM só existem enquanto este painel está montado (= aberto).
  const exitedKey = (list ?? [])
    .filter((c) => c.state !== "running")
    .map((c) => c.id)
    .join(",");
  const hasList = list !== null && list.length > 0;
  useEffect(() => {
    if (!hasList) return;
    let alive = true;
    const exited = exitedKey ? exitedKey.split(",") : [];
    const load = () =>
      void dockerPanelUsage(exited, host)
        .then((u) => {
          if (alive) setUsage(u);
        })
        .catch(() => {});
    load();
    const timer = window.setInterval(load, host ? REMOTE_REFRESH_MS : USAGE_REFRESH_MS);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [hasList, exitedKey, host]);

  const run = async (id: string, action: () => Promise<void>) => {
    setBusy(id);
    setError(null);
    try {
      await action();
      await refresh();
    } catch (e) {
      setError(translateError(e, t));
    } finally {
      setBusy(null);
    }
  };

  const op = (c: ContainerInfo, kind: ContainerOp) =>
    void run(c.id, () => dockerContainerOp(c.id, kind, host));

  const remove = async (c: ContainerInfo) => {
    const ok = await requestConfirm({
      title: t("dockerPanelRemoveTitle", { name: displayName(c) }),
      detail: t("dockerPanelRemoveDetail"),
      confirmLabel: t("dockerPanelRemove"),
      destructive: true,
    });
    if (ok) void run(c.id, () => dockerRemoveContainer(c.id, host));
  };

  const searching = query.trim().length > 0;
  const groups = useMemo(() => groupContainers(list ?? [], query), [list, query]);
  const counts = containerCounts(list ?? []);
  const defaultOpenKey = groups[0] ? groupKey(groups[0]) : null;
  const isGroupOpen = (g: ContainerGroup) =>
    searching || (openGroups[groupKey(g)] ?? groupKey(g) === defaultOpenKey);

  const row = (c: ContainerInfo) => {
    const running = c.state === "running";
    const isSelected = c.id === selectedId;
    const image = shortImage(c.image);
    const ports = portList(c.ports);
    const health = running ? healthOf(c.status) : null;
    const stat = running ? usage?.usage.find((u) => u.id === c.id) : undefined;
    const cpu = formatCpu(stat?.cpu_percent, i18n.language);
    const mem = formatBytes(stat?.mem_bytes, i18n.language);
    const up = running ? compactSpan(uptimeMs(c.status)) : null;
    const reason = running
      ? null
      : exitReason(usage?.exits.find((e) => e.id === c.id), exitCode(c.status));
    const failed = reason !== null && reason.kind !== "clean";
    const ago = running ? null : compactSpan(exitedAgoMs(c.status));
    const isBusy = busy === c.id;

    return (
      <li
        key={c.id}
        className={`group relative -mx-2 flex h-8 items-center gap-2 rounded-[6px] px-2 ${
          isSelected ? RAISED_ROW : "hover:bg-tyba-text/[.03]"
        }`}
      >
        <button
          type="button"
          onClick={() => setSelectedId(isSelected ? null : c.id)}
          aria-pressed={isSelected}
          title={`${c.name}\n${image.full}\n${c.status}`}
          className="flex min-w-0 flex-1 items-center gap-2 self-stretch rounded-[5px] text-left outline-none focus-visible:ring-2 focus-visible:ring-tyba-cyan"
        >
          <span
            aria-hidden
            className={`size-[7px] shrink-0 rounded-full ${
              // Parado é o ponto fantasma (mockup): quem diz que falhou é o
              // "saiu com N" em vermelho à direita.
              running ? "bg-tyba-green-fill" : "bg-tyba-text-ghost"
            }`}
          />
          {/* O nome ganha sempre: só trunca depois que a imagem já sumiu. */}
          <span
            className={`max-w-[62%] shrink-0 truncate text-[13px] ${
              running ? "text-tyba-text" : "text-tyba-text-muted"
            }`}
          >
            {displayName(c)}
          </span>
          <span className="min-w-0 flex-1 truncate text-[11px] text-tyba-text-faint">
            {image.short}
          </span>
          {ports.slice(0, 2).map((p) => (
            <span
              key={p}
              // Mockup: a porta é texto apagado (`node:22 · :3000`), sem chip.
              className="shrink-0 text-[11px] tabular-nums text-tyba-text-faint"
            >
              :{p}
            </span>
          ))}
          {ports.length > 2 && (
            <span className="shrink-0 text-[11px] text-tyba-text-faint">
              +{ports.length - 2}
            </span>
          )}
        </button>

        {/* Estado à direita; some quando as ações aparecem (selecionada,
            hover ou foco dentro da linha). */}
        <span
          className={`shrink-0 items-center gap-2 text-[11px] tabular-nums text-tyba-text-faint ${
            isSelected ? "hidden" : "flex group-hover:hidden group-focus-within:hidden"
          }`}
        >
          {health && (
            <Tooltip>
              <TooltipTrigger asChild>
                <span
                  role="img"
                  aria-label={t(
                    health === "healthy"
                      ? "dockerPanelHealthy"
                      : health === "unhealthy"
                        ? "dockerPanelUnhealthy"
                        : "dockerPanelHealthStarting",
                  )}
                  className={
                    health === "healthy"
                      ? "text-tyba-green"
                      : health === "unhealthy"
                        ? "text-tyba-red"
                        : "text-tyba-amber"
                  }
                >
                  {health === "unhealthy" ? (
                    <WarningCircle size={12} weight="fill" />
                  ) : (
                    <Heartbeat size={12} />
                  )}
                </span>
              </TooltipTrigger>
              <TooltipContent side="bottom">
                {t(
                  health === "healthy"
                    ? "dockerPanelHealthy"
                    : health === "unhealthy"
                      ? "dockerPanelUnhealthy"
                      : "dockerPanelHealthStarting",
                )}
              </TooltipContent>
            </Tooltip>
          )}
          {running ? (
            <>
              {cpu && <span className="w-[34px] text-right">{cpu}</span>}
              {mem && <span className="w-[52px] text-right">{mem}</span>}
              {up && (
                <span className="w-[34px] text-right" title={c.status}>
                  {up}
                </span>
              )}
            </>
          ) : failed ? (
            <span className="text-tyba-red" title={c.status}>
              {reason.kind === "oom"
                ? t("dockerPanelOom")
                : t("dockerPanelExited", { code: reason.code })}
            </span>
          ) : (
            ago && <span title={c.status}>{t("dockerPanelStoppedAgo", { span: ago })}</span>
          )}
        </span>

        <span
          className={`shrink-0 items-center gap-0.5 ${
            isSelected ? "flex" : "hidden group-hover:flex group-focus-within:flex"
          }`}
        >
          {running ? (
            <>
              <IconButton
                label={t("dockerPanelLogs")}
                onClick={() =>
                  void run(c.id, () =>
                    openContainerTab("logs", c.id, { host, name: c.name }),
                  )
                }
              >
                <Scroll size={14} />
              </IconButton>
              <IconButton
                label={t("dockerPanelShell")}
                onClick={() =>
                  void run(c.id, () =>
                    openContainerTab("shell", c.id, { host, name: c.name }),
                  )
                }
              >
                <TerminalWindow size={14} />
              </IconButton>
              {/* Arquivos do container (só leitura) ainda só no docker local. */}
              {!host && (
                <IconButton
                  label={t("containerFiles")}
                  onClick={() =>
                    void run(c.id, () =>
                      openContainerTab("files", c.id, { name: c.name }),
                    )
                  }
                >
                  <FolderSimple size={14} />
                </IconButton>
              )}
              <IconButton
                label={t("dockerPanelRestart")}
                disabled={isBusy}
                onClick={() => op(c, "restart")}
              >
                <ArrowClockwise size={14} />
              </IconButton>
              <IconButton
                label={t("dockerPanelStop")}
                disabled={isBusy}
                onClick={() => op(c, "stop")}
              >
                <Stop size={14} />
              </IconButton>
            </>
          ) : (
            <>
              <IconButton
                label={t("dockerPanelStart")}
                disabled={isBusy}
                onClick={() => op(c, "start")}
              >
                <Play size={14} />
              </IconButton>
              <IconButton
                label={t("dockerPanelLogs")}
                onClick={() =>
                  void run(c.id, () =>
                    openContainerTab("logs", c.id, { host, name: c.name }),
                  )
                }
              >
                <Scroll size={14} />
              </IconButton>
              <IconButton
                label={t("dockerPanelRemove")}
                danger
                disabled={isBusy}
                onClick={() => void remove(c)}
              >
                <Trash size={14} />
              </IconButton>
            </>
          )}
        </span>
      </li>
    );
  };

  const toggler = (
    open: boolean,
    onToggle: () => void,
    label: string,
  ) => (
    <li>
      <button
        type="button"
        aria-expanded={open}
        onClick={onToggle}
        className="-mx-2 flex h-7 w-[calc(100%+16px)] items-center gap-1.5 rounded-[5px] px-2 text-[11px] text-tyba-text-faint outline-none hover:text-tyba-text focus-visible:ring-2 focus-visible:ring-tyba-cyan"
      >
        {open ? <CaretDown size={10} weight="bold" /> : <CaretRight size={10} weight="bold" />}
        {label}
      </button>
    </li>
  );

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <SidePanelHeader kind="docker" title={t("sidePanelDocker")} onClose={onClose}>
        <span className="shrink-0 text-[11px] text-tyba-text-faint">
          {host ?? t("dockerPanelLocal")}
        </span>
      </SidePanelHeader>

      {available === false ? (
        <div className="flex flex-col items-start gap-2 px-4 py-3 text-[12px] text-tyba-text-muted">
          {t("dockerUnavailable")}
          {!host && (
            <button
              type="button"
              className={TEXT_BUTTON}
              onClick={() => void dockerOpenDesktop().catch(() => {})}
            >
              {t("dockerPanelOpenDesktop")}
            </button>
          )}
        </div>
      ) : (
        <>
          <div
            className={`${PANEL_SECTION} flex flex-col gap-2.5 px-4 py-3 text-[12px] text-tyba-text-muted`}
          >
            <div className="flex items-center justify-between gap-3">
              <span>{t("containers")}</span>
              <span className="shrink-0">
                <span className="text-tyba-green">
                  {t("dockerPanelRunning", { count: counts.running })}
                </span>
                {counts.stopped > 0 &&
                  ` · ${t("dockerPanelStopped", { count: counts.stopped })}`}
              </span>
            </div>
            {(list?.length ?? 0) > FILTER_THRESHOLD && (
              <label className="flex items-center gap-2 rounded-[6px] border border-tyba-border-field bg-tyba-sunken px-2.5 py-1.5 text-tyba-text-faint">
                <MagnifyingGlass size={13} className="shrink-0" />
                <input
                  type="text"
                  spellCheck={false}
                  autoCapitalize="off"
                  autoCorrect="off"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape" && query) {
                      e.preventDefault();
                      setQuery("");
                    }
                  }}
                  placeholder={t("dockerPanelFilter")}
                  aria-label={t("dockerPanelFilter")}
                  className="min-w-0 flex-1 bg-transparent text-[12px] text-tyba-text outline-none placeholder:text-tyba-text-faint"
                />
              </label>
            )}
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto px-4 py-2.5">
            {list?.length === 0 && (
              <p className="py-2 text-[12px] text-tyba-text-faint">
                {t("dockerPanelEmptyAll")}
              </p>
            )}
            {searching && groups.length === 0 && (
              <p className="py-2 text-[12px] text-tyba-text-faint">
                {t("dockerPanelNoMatch")}
              </p>
            )}
            {groups.map((g) => {
              const key = groupKey(g);
              const open = isGroupOpen(g);
              const stoppedCount = g.stopped.length + g.old.length;
              const stoppedOpen = searching || (openStopped[key] ?? false);
              const oldOpen = searching || (openOld[key] ?? false);
              return (
                <section key={key} className="pb-1">
                  <button
                    type="button"
                    aria-expanded={open}
                    onClick={() => setOpenGroups((s) => ({ ...s, [key]: !open }))}
                    className="-mx-2 flex h-8 w-[calc(100%+16px)] items-center gap-1.5 rounded-[6px] px-2 text-left outline-none hover:bg-tyba-text/[.03] focus-visible:ring-2 focus-visible:ring-tyba-cyan"
                  >
                    {open ? (
                      <CaretDown size={10} weight="bold" className="shrink-0 text-tyba-text-faint" />
                    ) : (
                      <CaretRight size={10} weight="bold" className="shrink-0 text-tyba-text-faint" />
                    )}
                    <span className="min-w-0 flex-1 truncate text-[12px] text-tyba-text">
                      {g.project ?? t("dockerPanelLoose")}
                      {g.inRepo && (
                        <span className="ml-2 text-[11px] text-tyba-blue">
                          {t("dockerPanelThisRepo")}
                        </span>
                      )}
                    </span>
                    <span className="shrink-0 text-[11px] text-tyba-text-faint">
                      {g.running.length > 0 && (
                        <span className="text-tyba-green">
                          {t("dockerPanelRunning", { count: g.running.length })}
                        </span>
                      )}
                      {g.running.length > 0 && stoppedCount > 0 && " · "}
                      {stoppedCount > 0 &&
                        t("dockerPanelStopped", { count: stoppedCount })}
                    </span>
                  </button>
                  {open && (
                    <ul className="pl-3">
                      {g.running.map(row)}
                      {stoppedCount > 0 &&
                        !searching &&
                        toggler(
                          stoppedOpen,
                          () => setOpenStopped((s) => ({ ...s, [key]: !stoppedOpen })),
                          t("dockerPanelStoppedToggle", { count: stoppedCount }),
                        )}
                      {stoppedOpen && g.stopped.map(row)}
                      {stoppedOpen &&
                        g.old.length > 0 &&
                        !searching &&
                        toggler(
                          oldOpen,
                          () => setOpenOld((s) => ({ ...s, [key]: !oldOpen })),
                          oldOpen
                            ? t("dockerPanelHideOld")
                            : t("dockerPanelShowOld", { count: g.old.length }),
                        )}
                      {stoppedOpen && oldOpen && g.old.map(row)}
                    </ul>
                  )}
                </section>
              );
            })}
          </div>

          <div className="flex shrink-0 items-center gap-2 border-t border-[var(--tyba-border-soft)] px-4 py-2.5 text-[11px] text-tyba-text-faint">
            {error ? (
              <p role="alert" className="min-w-0 flex-1 truncate text-tyba-red" title={error}>
                {error}
              </p>
            ) : (
              <p className="min-w-0 flex-1 truncate" title={t("dockerPanelNote")}>
                {host ? t("dockerPanelNoteRemote", { host }) : t("dockerPanelNoteLocal")}
              </p>
            )}
            <button type="button" className={TEXT_BUTTON} onClick={onOpenDashboard}>
              {t("dockerPanelOpenDashboard")}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
