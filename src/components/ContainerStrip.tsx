import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { DotsThree } from "@phosphor-icons/react";

import {
  dockerContainerAction,
  dockerContainerDetail,
  dockerContainerStats,
  type ContainerDetail,
  type ContainerStats,
} from "@/lib/ipc";
import {
  containerName,
  openContainerTab,
  rememberContainerName,
  useContentTabsVersion,
} from "@/lib/contentTabs";
import {
  compactCpu,
  compactMem,
  formatRunningFor,
  sinceLabel,
} from "@/lib/containerLogs";
import { requestConfirm } from "@/lib/confirm";
import { toastError } from "@/lib/toast";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

interface Props {
  containerId: string;
  /** Qual aba está aberta: o botão oferece a outra. */
  current: "logs" | "shell";
  host?: string | null;
  /** Depois de reiniciar: a aba de logs religa o seguidor. */
  onRestarted?: () => void;
}

const DETAIL_EVERY_MS = 10_000;
const STATS_EVERY_MS = 5_000;
/** Abaixo desta largura, as ações secundárias vão para o menu "⋯": botão
 *  cortado no meio é pior que um clique a mais. Parar fica sempre à vista. */
const COMPACT_BELOW_PX = 760;

/**
 * Faixa de contexto sob a barra de abas de uma aba de container: quem é, em
 * que estado, portas e consumo — e as ações (a outra aba, arquivos,
 * reiniciar, parar).
 */
export function ContainerStrip({ containerId, current, host, onRestarted }: Props) {
  const { t } = useTranslation();
  const [detail, setDetail] = useState<ContainerDetail | null>(null);
  const [stats, setStats] = useState<ContainerStats | null>(null);
  const [busy, setBusy] = useState<"restart" | "stop" | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [compact, setCompact] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) =>
      setCompact(entry.contentRect.width < COMPACT_BELOW_PX),
    );
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const loadDetail = useCallback(() => {
    void dockerContainerDetail(containerId, host)
      .then((d) => {
        setDetail(d);
        rememberContainerName(containerId, d.name);
      })
      .catch(() => {});
  }, [containerId, host]);

  useEffect(() => {
    loadDetail();
    const timer = window.setInterval(() => {
      loadDetail();
      setNow(Date.now());
    }, DETAIL_EVERY_MS);
    return () => window.clearInterval(timer);
  }, [loadDetail]);

  const running = detail?.state === "running";
  useEffect(() => {
    if (!running) {
      setStats(null);
      return;
    }
    let alive = true;
    const load = () =>
      void dockerContainerStats(containerId, host)
        .then((s) => {
          if (alive) setStats(s);
        })
        .catch(() => {});
    load();
    const timer = window.setInterval(load, STATS_EVERY_MS);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [running, containerId, host]);

  const act = useCallback(
    async (action: "restart" | "stop") => {
      if (action === "stop") {
        const ok = await requestConfirm({
          title: t("containerStopTitle", { name: detail?.name ?? containerId.slice(0, 12) }),
          detail: t("containerStopDetail"),
          confirmLabel: t("containerStop"),
          destructive: true,
        });
        if (!ok) return;
      }
      setBusy(action);
      try {
        await dockerContainerAction(containerId, action, host);
        if (action === "restart") onRestarted?.();
      } catch (e) {
        toastError(t(action === "restart" ? "containerRestartError" : "containerStopError"), e);
      } finally {
        setBusy(null);
        loadDetail();
      }
    },
    [containerId, host, detail?.name, loadDetail, onRestarted, t],
  );

  const minutes = sinceLabel(detail?.started_at ?? null, now);
  const stateLabel = !detail
    ? null
    : running
      ? t("containerRunningFor", { time: formatRunningFor(minutes ?? 0) })
      : t("containerState", { state: detail.state });
  const sep = <span className="shrink-0 text-tyba-text-ghost">·</span>;
  const chip =
    "shrink-0 whitespace-nowrap rounded-[5px] border border-tyba-border-field bg-transparent px-[9px] py-[3px] text-[11px] text-tyba-text-muted hover:bg-tyba-text/[.06] hover:text-tyba-text disabled:opacity-50";

  const secondary: { key: string; label: string; run: () => void; disabled?: boolean }[] = [
    {
      key: "switch",
      label: current === "logs" ? t("stripShell") : t("stripLogs"),
      run: () =>
        void openContainerTab(current === "logs" ? "shell" : "logs", containerId, {
          host,
        }).catch((e) => toastError(t("containerOpenError"), e)),
    },
    ...(host
      ? []
      : [
          {
            key: "files",
            label: t("containerFiles"),
            run: () =>
              void openContainerTab("files", containerId).catch((e) =>
                toastError(t("containerOpenError"), e),
              ),
          },
        ]),
    {
      key: "restart",
      label: busy === "restart" ? t("containerRestarting") : t("containerRestart"),
      run: () => void act("restart"),
      disabled: busy !== null,
    },
  ];

  return (
    <div
      ref={rootRef}
      className="flex h-9 shrink-0 items-center gap-2 tyba-divide-b bg-[var(--tyba-panel-bg)] px-[18px] text-[12px] text-tyba-text-muted"
    >
      <div className="flex min-w-0 flex-1 items-center gap-2 overflow-hidden whitespace-nowrap">
        <span className="min-w-0 max-w-[40%] shrink-0 truncate text-tyba-text">
          {detail?.name ?? containerId.slice(0, 12)}
        </span>
        {detail && (
          <>
            {sep}
            <span className="min-w-0 truncate" title={detail.image}>
              {detail.image}
            </span>
            {sep}
            <span className={`shrink-0 ${running ? "text-tyba-green" : "text-tyba-red"}`}>
              ● {stateLabel}
            </span>
            {detail.ports.length > 0 && (
              <>
                {sep}
                <span className="min-w-0 truncate" title={detail.ports.join(", ")}>
                  {detail.ports.join(", ")}
                </span>
              </>
            )}
            {stats && (
              <>
                {sep}
                <span className="shrink-0" title={`CPU ${stats.cpu} · RAM ${stats.mem}`}>
                  CPU {compactCpu(stats.cpu)} · RAM {compactMem(stats.mem)}
                </span>
              </>
            )}
          </>
        )}
      </div>
      {compact ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              aria-label={t("stripMore")}
              title={t("stripMore")}
              className={`${chip} flex items-center px-[6px]`}
            >
              <DotsThree size={14} weight="bold" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {secondary.map((a) => (
              <DropdownMenuItem
                key={a.key}
                disabled={a.disabled}
                onSelect={a.run}
                className="text-[12px]"
              >
                {a.label}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : (
        secondary.map((a) => (
          <button
            key={a.key}
            type="button"
            className={chip}
            disabled={a.disabled}
            onClick={a.run}
          >
            {a.label}
          </button>
        ))
      )}
      <button
        type="button"
        className={`${chip} border-tyba-red/35 text-tyba-red hover:bg-tyba-red/10 hover:text-tyba-red`}
        disabled={busy !== null || !running}
        onClick={() => void act("stop")}
      >
        {t("containerStop")}
      </button>
    </div>
  );
}

/**
 * Faixa azul no topo do shell de container: diz onde os comandos rodam antes
 * que alguém rode `rm` achando que está na própria máquina.
 */
export function ContainerShellBanner({ containerId }: { containerId: string }) {
  const { t } = useTranslation();
  useContentTabsVersion();
  const name = containerName(containerId);
  return (
    <div className="flex shrink-0 items-center justify-between gap-3 border-b border-tyba-blue/20 bg-tyba-blue/[.07] px-[18px] py-2 text-[11px] text-tyba-blue">
      <span className="min-w-0 truncate">{t("containerShellBanner", { name })}</span>
      <span className="shrink-0 font-mono text-tyba-text-faint">docker exec -it {name}</span>
    </div>
  );
}
