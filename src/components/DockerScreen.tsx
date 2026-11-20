import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowClockwise,
  Cube,
  HardDrives,
  Image as ImageIcon,
  MagnifyingGlass,
  Play,
  Stack,
  Stop,
  Trash,
  X,
  type Icon,
} from "@phosphor-icons/react";

import "./gitScreen.css";
import { Button } from "@/components/ui/button";
import { SegmentedControl } from "@/components/ui/segmented";
import { Input } from "@/components/ui/input";
import { ContainerFilesPanel } from "./ContainerFilesPanel";
import { ContainerLogsView } from "./ContainerLogsView";
import { ContainerShellBanner, ContainerStrip } from "./ContainerStrip";
import { DockerComposeSection } from "./DockerComposeSection";
import { DockerLoading } from "./DockerLoading";
import { PanelIcon } from "./SidePanelBar";
import { TerminalView } from "./TerminalView";
import { requestConfirm } from "../lib/confirm";
import { rememberContainerName } from "../lib/contentTabs";
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
  loadContainers,
  portList,
  shortImage,
  uptimeMs,
} from "../lib/dockerPanel";
import {
  composeProjects,
  mergeComposeOutput,
  mergeComposeSnapshot,
  visibleProjects,
} from "../lib/dockerCompose";
import { translateError } from "../lib/errors";
import {
  dockerAvailable,
  dockerComposeRuns,
  onComposeOutput,
  dockerContainerDetail,
  dockerContainerOp,
  dockerListContainers,
  dockerListImages,
  dockerListVolumes,
  dockerOpenDesktop,
  dockerPanelUsage,
  dockerRemoveContainer,
  dockerScreenPane,
  dockerScreenHideStopped,
  dockerScreenSection,
  dockerScreenSelect,
  dockerScreenShell,
  type ComposeRun,
  type ContainerDetail,
  type ContainerInfo,
  type ContainerPane,
  type DockerScreenState,
  type DockerSection,
  type ImageInfo,
  type PanelUsage,
  type Session,
  type SessionId,
  type VolumeInfo,
  type WorkspaceId,
} from "../lib/ipc";
import { CONTAINER_PANES, DOCKER_SECTIONS, escClosesScreen } from "../lib/screens";

/** Mesma cadência do painel antigo: remoto é rede, local é barato. */
const LOCAL_REFRESH_MS = 3000;
const REMOTE_REFRESH_MS = 15000;
/** CPU/RAM: `docker stats` amostra ~2s; o core ainda segura em ≥5s. */
const USAGE_REFRESH_MS = 5000;

const RAIL: Record<DockerSection, { icon: Icon; label: string }> = {
  containers: { icon: Cube, label: "dockerScreenContainers" },
  compose: { icon: Stack, label: "dockerScreenComposeTitle" },
  images: { icon: ImageIcon, label: "dockerScreenImages" },
  volumes: { icon: HardDrives, label: "dockerScreenVolumes" },
};

const PANE_LABEL: Record<ContainerPane, string> = {
  logs: "dockerPanelLogs",
  shell: "dockerPanelShell",
  files: "containerFiles",
  details: "dockerScreenDetails",
};

const FULL_RECT = { left: 0, top: 0, width: 100, height: 100 } as const;

/** O shell do container dentro da tela: `docker exec` no container, nunca na
 *  máquina local — a faixa azul diz isso antes de qualquer comando. */
function ShellPane({
  containerId,
  host,
  exitedOf,
  onSessionsChanged,
}: {
  containerId: string;
  host: string | null;
  exitedOf: (id: SessionId) => boolean;
  /** A sessão nasce no core: a lista do webview precisa recarregar. */
  onSessionsChanged: () => void;
}) {
  const { t } = useTranslation();
  const [sessionId, setSessionId] = useState<SessionId | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [nonce, setNonce] = useState(0);
  // Fora das deps do spawn: um callback novo do pai não pode abrir outro shell.
  const changedRef = useRef(onSessionsChanged);
  changedRef.current = onSessionsChanged;

  useEffect(() => {
    let alive = true;
    setSessionId(null);
    setError(null);
    void dockerScreenShell(containerId, host)
      .then((id) => {
        if (alive) setSessionId(id);
        changedRef.current();
      })
      .catch((e) => {
        if (alive) setError(translateError(e, t));
      });
    return () => {
      alive = false;
    };
  }, [containerId, host, nonce, t]);

  const exited = sessionId ? exitedOf(sessionId) : false;
  return (
    <div className="absolute inset-0 flex flex-col">
      <ContainerShellBanner containerId={containerId} />
      {error ? (
        <p role="alert" className="px-[18px] py-3 text-[12px] text-tyba-red">
          {error}
        </p>
      ) : (
        <div className="relative min-h-0 flex-1">
          {sessionId && (
            <TerminalView
              key={sessionId}
              sessionId={sessionId}
              visible
              focused
              rect={FULL_RECT}
              exited={exited}
              onExit={onSessionsChanged}
            />
          )}
          {exited && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => setNonce((n) => n + 1)}
              className="absolute bottom-3 right-4"
            >
              <ArrowClockwise />
              {t("dockerScreenShellReopen")}
            </Button>
          )}
        </div>
      )}
    </div>
  );
}

/** Detalhes do container (inspect), só leitura. Env fica de fora: é onde
 *  mora segredo. */
function DetailsPane({ containerId, host }: { containerId: string; host: string | null }) {
  const { t } = useTranslation();
  const [detail, setDetail] = useState<ContainerDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  // `startedAt` também é o gatilho de "Tentar de novo": mudou, relê.
  const [startedAt, setStartedAt] = useState(() => Date.now());
  useEffect(() => {
    let alive = true;
    setError(null);
    void dockerContainerDetail(containerId, host)
      .then((d) => {
        if (alive) setDetail(d);
      })
      .catch((e) => {
        if (alive) setError(translateError(e, t));
      });
    return () => {
      alive = false;
    };
  }, [containerId, host, t, startedAt]);
  if (!detail) {
    return (
      <DockerLoading
        host={host}
        cmd={`inspect ${containerId.slice(0, 12)}`}
        startedAt={startedAt}
        error={error}
        onRetry={() => setStartedAt(Date.now())}
        shape="lines"
        className="absolute inset-0"
      />
    );
  }
  const row = (label: string, value: React.ReactNode) => (
    <div className="flex gap-4 py-1.5">
      <dt className="w-[120px] shrink-0 text-tyba-text-faint">{label}</dt>
      <dd className="min-w-0 flex-1 break-all font-mono text-tyba-text-muted">{value}</dd>
    </div>
  );
  return (
    <dl className="absolute inset-0 overflow-auto px-[18px] py-3 text-[12px]">
      {row(t("dockerScreenDetailId"), detail.id.slice(0, 12))}
      {row(t("dockerScreenDetailImage"), detail.image)}
      {row(t("dockerScreenDetailState"), detail.state)}
      {row(t("dockerScreenDetailStarted"), detail.started_at ?? "—")}
      {row(t("dockerScreenDetailWorkdir"), detail.workdir ?? "—")}
      {row(
        t("dockerScreenDetailPorts"),
        detail.ports.length > 0 ? detail.ports.join(", ") : "—",
      )}
      {row(
        t("dockerScreenDetailMounts"),
        detail.mounts.length > 0
          ? detail.mounts.map((m) => (
              <span key={`${m.source}:${m.destination}`} className="block">
                {m.source} → {m.destination}
              </span>
            ))
          : "—",
      )}
    </dl>
  );
}

/** Imagens ou volumes do alvo: lista só leitura. */
function ReadOnlyList({ section, host }: { section: "images" | "volumes"; host: string | null }) {
  const { t } = useTranslation();
  const [images, setImages] = useState<ImageInfo[] | null>(null);
  const [volumes, setVolumes] = useState<VolumeInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [startedAt, setStartedAt] = useState(() => Date.now());
  useEffect(() => {
    let alive = true;
    setError(null);
    const fail = (e: unknown) => {
      if (alive) setError(translateError(e, t));
    };
    if (section === "images") {
      void dockerListImages(host)
        .then((list) => alive && setImages(list))
        .catch(fail);
    } else {
      void dockerListVolumes(host)
        .then((list) => alive && setVolumes(list))
        .catch(fail);
    }
    return () => {
      alive = false;
    };
  }, [section, host, t, startedAt]);
  const rows = section === "images" ? images : volumes;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="shrink-0 border-b border-[var(--tyba-line)] bg-[var(--tyba-panel-bg)] px-[18px] py-2.5 text-[11px] text-tyba-text-faint">
        {section === "images"
          ? t("dockerScreenImagesCount", { count: images?.length ?? 0 })
          : t("dockerScreenVolumesCount", { count: volumes?.length ?? 0 })}
        {" · "}
        {t("dockerScreenReadOnly")}
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-[18px] py-2 text-[12.5px]">
        {rows === null ? (
          <DockerLoading
            host={host}
            cmd={section === "images" ? "image ls" : "volume ls"}
            startedAt={startedAt}
            error={error}
            onRetry={() => setStartedAt(Date.now())}
            className="-mx-[18px] -my-2"
          />
        ) : error ? (
          <p role="alert" className="py-2 text-tyba-red">
            {error}
          </p>
        ) : rows.length === 0 ? (
          <p className="py-2 text-tyba-text-faint">
            {section === "images" ? t("dockerScreenNoImages") : t("dockerScreenNoVolumes")}
          </p>
        ) : section === "images" ? (
          <table className="w-full border-collapse">
            <thead className="text-left text-[11px] text-tyba-text-faint">
              <tr>
                <th className="py-1.5 font-normal">{t("dockerScreenImageRepo")}</th>
                <th className="py-1.5 font-normal">{t("dockerScreenImageTag")}</th>
                <th className="py-1.5 font-normal">{t("dockerScreenImageSize")}</th>
                <th className="py-1.5 font-normal">{t("dockerScreenImageCreated")}</th>
              </tr>
            </thead>
            <tbody>
              {(images ?? []).map((img) => (
                <tr key={`${img.id}:${img.repository}:${img.tag}`} className="text-tyba-text-muted">
                  <td className="py-1.5 pr-4 text-tyba-text">{img.repository}</td>
                  <td className="py-1.5 pr-4 font-mono">{img.tag}</td>
                  <td className="py-1.5 pr-4 tabular-nums">{img.size}</td>
                  <td className="py-1.5 text-tyba-text-faint">{img.created_since}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <table className="w-full border-collapse">
            <thead className="text-left text-[11px] text-tyba-text-faint">
              <tr>
                <th className="py-1.5 font-normal">{t("dockerScreenVolumeName")}</th>
                <th className="py-1.5 font-normal">{t("dockerScreenVolumeDriver")}</th>
                <th className="py-1.5 font-normal">{t("dockerScreenVolumeMount")}</th>
              </tr>
            </thead>
            <tbody>
              {(volumes ?? []).map((v) => (
                <tr key={v.name} className="text-tyba-text-muted">
                  <td className="py-1.5 pr-4 text-tyba-text">{v.name}</td>
                  <td className="py-1.5 pr-4">{v.driver}</td>
                  <td className="py-1.5 font-mono text-[11.5px] text-tyba-text-faint">{v.mountpoint}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

/**
 * Tela cheia de Docker (mockup DockerTela): trilho à direita (Containers,
 * Compose, Imagens, Volumes), coluna de 300px com os containers do alvo da sessão (a
 * máquina local, ou o servidor numa sessão SSH) e o container escolhido com
 * Logs | Shell | Arquivos | Detalhes. Seção e container escolhido são do core.
 */
export function DockerScreen({
  session,
  workspaceId,
  state,
  host,
  repoRoot,
  exiting,
  exitedOf,
  onSessionsChanged,
  onClose,
}: {
  session: Session;
  workspaceId: WorkspaceId;
  state: DockerScreenState;
  /** Alias SSH numa sessão remota; `null` = docker local. */
  host: string | null;
  repoRoot: string | null;
  exiting: boolean;
  exitedOf: (id: SessionId) => boolean;
  onSessionsChanged: () => void;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [available, setAvailable] = useState<boolean | null>(null);
  const [list, setList] = useState<ContainerInfo[] | null>(null);
  const [usage, setUsage] = useState<PanelUsage | null>(null);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [logsRestart, setLogsRestart] = useState(0);
  const [composeRuns, setComposeRuns] = useState<Record<string, ComposeRun>>({});
  // A etapa da primeira leitura: o comando que o core está rodando agora.
  const [loadCmd, setLoadCmd] = useState<"version" | "ps -a">("ps -a");
  const [loadStart, setLoadStart] = useState(() => Date.now());

  const sectionRef = useRef<HTMLElement>(null);
  useEffect(() => {
    sectionRef.current?.focus({ preventScroll: true });
  }, []);

  const refresh = useCallback(async () => {
    // Compose em andamento mora no core (só local): sobrevive à tela fechar.
    if (host) setComposeRuns({});
    else
      await dockerComposeRuns().then(
        (snap) => setComposeRuns((cur) => mergeComposeSnapshot(cur, snap)),
        () => {},
      );
    try {
      const read = await loadContainers(
        () => dockerListContainers(host ? null : repoRoot, true, host),
        () => dockerAvailable(host),
        setLoadCmd,
      );
      setAvailable(read.available);
      for (const c of read.list) rememberContainerName(c.id, c.name);
      setList(read.list);
    } catch (e) {
      // Só lança com o `version` respondendo: o docker está de pé, o `ps` é que falhou.
      setAvailable(true);
      setError(translateError(e, t));
    }
  }, [host, repoRoot, t]);

  const retryLoad = useCallback(() => {
    setError(null);
    setLoadStart(Date.now());
    void refresh();
  }, [refresh]);
  const firstLoad = (shape: "list" | null, className?: string) => (
    <DockerLoading
      host={host}
      cmd={loadCmd}
      startedAt={loadStart}
      error={error}
      onRetry={retryLoad}
      shape={shape}
      className={className}
    />
  );

  // Saída ao vivo do compose: lotes do core juntados ao retrato por posição.
  useEffect(() => {
    if (host) return;
    let off: (() => void) | null = null;
    let alive = true;
    void onComposeOutput((batch) => setComposeRuns((runs) => mergeComposeOutput(runs, batch))).then(
      (unlisten) => {
        if (alive) off = unlisten;
        else unlisten();
      },
    );
    return () => {
      alive = false;
      off?.();
    };
  }, [host]);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(
      () => void refresh(),
      host ? REMOTE_REFRESH_MS : LOCAL_REFRESH_MS,
    );
    return () => window.clearInterval(timer);
  }, [refresh, host]);

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

  // Ordem do painel antigo: o projeto do repo primeiro, rodando antes.
  const ordered = useMemo(
    () =>
      groupContainers(list ?? [], query).flatMap((g) => [...g.running, ...g.stopped, ...g.old]),
    [list, query],
  );
  const counts = containerCounts(list ?? []);
  const repoProject = useMemo(
    () => (list ?? []).find((c) => c.in_repo)?.compose_project ?? null,
    [list],
  );
  const repoContainers = useMemo(
    () => (repoProject ? (list ?? []).filter((c) => c.compose_project === repoProject) : []),
    [list, repoProject],
  );
  const selected = (list ?? []).find((c) => c.id === state.container) ?? null;
  const projects = useMemo(() => composeProjects(list ?? [], composeRuns), [list, composeRuns]);

  // Sem escolha (ou o escolhido sumiu): o primeiro da lista.
  useEffect(() => {
    if (state.section !== "containers" || list === null) return;
    if (selected || ordered.length === 0) return;
    void dockerScreenSelect(workspaceId, ordered[0].id).catch(() => {});
  }, [state.section, list, selected, ordered, workspaceId]);

  const go = (section: DockerSection) =>
    void dockerScreenSection(workspaceId, section).catch(() => {});
  const pick = (id: string) => void dockerScreenSelect(workspaceId, id).catch(() => {});
  const showPane = (pane: ContainerPane) =>
    void dockerScreenPane(workspaceId, pane).catch(() => {});

  /** "Subir tudo" / "Parar tudo": os containers do projeto compose do repo. */
  const bulk = async (op: "start" | "stop") => {
    const targets = repoContainers.filter((c) =>
      op === "stop" ? c.state === "running" : c.state !== "running",
    );
    if (targets.length === 0 || busy) return;
    if (op === "stop") {
      const ok = await requestConfirm({
        title: t("dockerScreenStopAllTitle", { project: repoProject ?? "", count: targets.length }),
        detail: t("dockerScreenStopAllDetail"),
        confirmLabel: t("dockerScreenStopAll"),
        destructive: true,
      });
      if (!ok) return;
    }
    setBusy(true);
    setError(null);
    try {
      for (const c of targets) await dockerContainerOp(c.id, op, host);
      await refresh();
    } catch (e) {
      setError(translateError(e, t));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (c: ContainerInfo) => {
    if (busy) return;
    const ok = await requestConfirm({
      title: t("dockerPanelRemoveTitle", { name: displayName(c) }),
      detail: t("dockerPanelRemoveDetail"),
      confirmLabel: t("dockerPanelRemove"),
      destructive: true,
    });
    if (!ok) return;
    setBusy(true);
    setError(null);
    try {
      await dockerRemoveContainer(c.id, host);
      await refresh();
    } catch (e) {
      setError(translateError(e, t));
    } finally {
      setBusy(false);
    }
  };

  const card = (c: ContainerInfo) => {
    const running = c.state === "running";
    const on = c.id === selected?.id;
    const image = shortImage(c.image);
    const port = portList(c.ports)[0];
    const health = running ? healthOf(c.status) : null;
    const stat = running ? usage?.usage.find((u) => u.id === c.id) : undefined;
    const cpu = formatCpu(stat?.cpu_percent, i18n.language);
    const mem = formatBytes(stat?.mem_bytes, i18n.language);
    const up = running ? compactSpan(uptimeMs(c.status)) : null;
    const reason = running
      ? null
      : exitReason(usage?.exits.find((e) => e.id === c.id), exitCode(c.status));
    const ago = running ? null : compactSpan(exitedAgoMs(c.status));
    return (
      <button
        key={c.id}
        type="button"
        aria-pressed={on}
        onClick={() => pick(c.id)}
        title={`${c.name}\n${image.full}\n${c.status}`}
        // Escolhido = fundo elevado com contorno inteiro; nunca barra lateral.
        className={`flex w-full flex-col gap-[5px] rounded-[10px] px-3 py-2.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-tyba-blue ${
          on
            ? "tyba-pill-on"
            : "hover:bg-tyba-text/[.03]"
        } ${running ? "text-tyba-text" : "text-tyba-text-muted"}`}
      >
        <span className="flex w-full items-center justify-between gap-2">
          <span className="flex min-w-0 items-center gap-1.5 text-[13px]">
            <span
              aria-hidden
              className={`size-[7px] shrink-0 rounded-full ${
                running ? "bg-tyba-green-fill" : "bg-tyba-text-ghost"
              }`}
            />
            <span className="truncate">{displayName(c)}</span>
          </span>
          <span className="shrink-0 truncate text-[11px] text-tyba-text-faint">
            {reason && reason.kind !== "clean" ? (
              <span className="text-tyba-red">
                {reason.kind === "code"
                  ? t("dockerPanelExited", { code: reason.code })
                  : exitCode(c.status) !== null
                    ? t("dockerPanelExited", { code: exitCode(c.status) })
                    : t("dockerPanelOom")}
              </span>
            ) : (
              <>
                {image.short}
                {port && ` · :${port}`}
              </>
            )}
          </span>
        </span>
        <span className="flex gap-3 text-[11px] text-tyba-text-faint">
          {running ? (
            <>
              {health && (
                <span
                  className={
                    health === "healthy"
                      ? ""
                      : health === "unhealthy"
                        ? "text-tyba-red"
                        : "text-tyba-amber"
                  }
                >
                  {t(
                    health === "healthy"
                      ? "dockerPanelHealthy"
                      : health === "unhealthy"
                        ? "dockerPanelUnhealthy"
                        : "dockerPanelHealthStarting",
                  )}
                </span>
              )}
              {!health && cpu && <span>CPU {cpu}</span>}
              {mem && <span>RAM {mem}</span>}
              {up && <span>{t("dockerScreenAgo", { span: up })}</span>}
            </>
          ) : (
            <span className="truncate">
              {ago ? t("dockerPanelStoppedAgo", { span: ago }) : c.status}
              {reason?.kind === "oom" && ` — ${t("dockerPanelOom")}`}
            </span>
          )}
        </span>
      </button>
    );
  };

  const titleSub = [host ?? t("dockerPanelLocal"), repoProject && t("dockerScreenCompose", { project: repoProject })]
    .filter(Boolean)
    .join(" · ");

  return (
    <section
      ref={sectionRef}
      tabIndex={-1}
      aria-label={t("sidePanelDocker")}
      data-exiting={exiting}
      className="tyba-git-screen tyba-frost absolute inset-0 z-30 flex flex-row-reverse outline-none"
      onKeyDown={(e) => {
        if (
          escClosesScreen({
            key: e.key,
            defaultPrevented: e.defaultPrevented,
            target: e.target as HTMLElement,
          })
        ) {
          e.preventDefault();
          onClose();
        }
      }}
    >
      <nav
        aria-label={t("dockerScreenSections")}
        className="relative flex w-[52px] shrink-0 flex-col items-center gap-1.5 tyba-divide-l bg-[var(--tyba-panel-bg)] pt-3"
      >
        {DOCKER_SECTIONS.map((key) => {
          const { icon: SectionIcon, label } = RAIL[key];
          const on = key === state.section;
          return (
            <button
              key={key}
              type="button"
              aria-label={t(label)}
              aria-pressed={on}
              title={t(label)}
              onClick={() => go(key)}
              className={`relative flex size-9 items-center justify-center rounded-[8px] transition-colors ${
                on ? "tyba-icon-on text-tyba-blue" : "text-tyba-text-faint hover:text-tyba-text"
              }`}
            >
              <SectionIcon size={17} />
            </button>
          );
        })}
        <span className="flex-1" />
        <button
          type="button"
          aria-label={t("filesScreenClose")}
          title={t("filesScreenClose")}
          onClick={onClose}
          className="mb-2.5 flex size-9 items-center justify-center rounded-[8px] text-tyba-text-faint hover:text-tyba-text"
        >
          <X size={15} />
        </button>
      </nav>

      <div className="flex min-w-0 flex-1 flex-col">
        {state.section === "compose" ? (
          <div className="flex h-[46px] shrink-0 items-center gap-3 tyba-divide-b bg-[var(--tyba-panel-bg)] px-[18px] text-[12px]">
            <Stack size={16} className="shrink-0 text-tyba-blue" />
            <span className="shrink-0 text-[15px] font-bold tracking-[-0.01em] text-tyba-text">
              {t("dockerScreenComposeTitle")}
            </span>
            <span className="min-w-0 flex-1 truncate text-tyba-text-faint" title={session.title}>
              {t("dockerScreenComposeSub", {
                target: host ?? t("dockerPanelLocal"),
                count: visibleProjects(projects, state.hide_stopped).length,
              })}
            </span>
            {error && (
              <span role="alert" className="min-w-0 max-w-[40%] truncate text-tyba-red" title={error}>
                {error}
              </span>
            )}
            <label className="flex shrink-0 items-center gap-1.5 text-tyba-text-muted">
              <input
                type="checkbox"
                checked={state.hide_stopped}
                onChange={(e) =>
                  void dockerScreenHideStopped(workspaceId, e.target.checked).catch(() => {})
                }
                className="accent-tyba-blue"
              />
              {t("dockerScreenHideStopped")}
            </label>
          </div>
        ) : (
          <div className="flex h-[46px] shrink-0 items-center gap-3 tyba-divide-b bg-[var(--tyba-panel-bg)] px-[18px] text-[12px]">
            <PanelIcon kind="docker" size={16} className="shrink-0 text-tyba-blue" />
            <span className="shrink-0 text-[15px] font-bold tracking-[-0.01em] text-tyba-text">
              {t("sidePanelDocker")}
            </span>
            <span className="min-w-0 flex-1 truncate text-tyba-text-faint" title={session.title}>
              {titleSub}
            </span>
            {error && (
              <span role="alert" className="min-w-0 max-w-[40%] truncate text-tyba-red" title={error}>
                {error}
              </span>
            )}
            {available !== false && (
              <span className="shrink-0">
                <span className="text-tyba-green">
                  {t("dockerPanelRunning", { count: counts.running })}
                </span>
                {counts.stopped > 0 && (
                  <span className="text-tyba-text-faint">
                    {" · "}
                    {t("dockerPanelStopped", { count: counts.stopped })}
                  </span>
                )}
              </span>
            )}
            {repoProject && (
              <>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={busy || !repoContainers.some((c) => c.state !== "running")}
                  onClick={() => void bulk("start")}
                >
                  <Play />
                  {t("dockerScreenStartAll")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy || !repoContainers.some((c) => c.state === "running")}
                  onClick={() => void bulk("stop")}
                >
                  <Stop />
                  {t("dockerScreenStopAll")}
                </Button>
              </>
            )}
          </div>
        )}

        {available === false ? (
          <div className="flex flex-1 flex-col items-start gap-2 px-[18px] py-4 text-[12px] text-tyba-text-muted">
            {t("dockerUnavailable")}
            {!host && (
              <Button
                variant="outline"
                size="sm"
                onClick={() => void dockerOpenDesktop().catch(() => {})}
              >
                {t("dockerPanelOpenDesktop")}
              </Button>
            )}
          </div>
        ) : state.section === "compose" ? (
          <DockerComposeSection
            projects={projects}
            loaded={list !== null}
            loading={firstLoad("list")}
            selected={state.project}
            hideStopped={state.hide_stopped}
            host={host}
            session={session.id}
            workspaceId={workspaceId}
            runs={composeRuns}
            onError={setError}
            onChanged={refresh}
          />
        ) : state.section !== "containers" ? (
          <ReadOnlyList key={state.section} section={state.section} host={host} />
        ) : (
          // Coluna à direita, junto do trilho, como em Arquivos (dono, 2026-10-14).
          <div className="flex min-h-0 flex-1 flex-row-reverse">
            <div className="flex w-[clamp(220px,40%,320px)] shrink-0 flex-col border-l border-[var(--tyba-line)] bg-[var(--tyba-panel-bg)]">
              <div className="flex shrink-0 items-center gap-2 px-4 pb-1.5 pt-3.5 text-[12.5px] text-tyba-text-muted">
                <span className="flex-1">
                  {t("dockerScreenContainersCount", { count: list?.length ?? 0 })}
                </span>
              </div>
              <div className="shrink-0 px-3 pb-2">
                <Input
                  icon={<MagnifyingGlass size={16} />}
                  type="text"
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
                />
              </div>
              <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 py-1">
                {list === null ? (
                  firstLoad("list", "-mx-2 -my-1")
                ) : list.length === 0 ? (
                  <p className="px-2 py-2 text-[12px] text-tyba-text-faint">
                    {t("dockerPanelEmptyAll")}
                  </p>
                ) : ordered.length === 0 ? (
                  <p className="px-2 py-2 text-[12px] text-tyba-text-faint">
                    {t("dockerPanelNoMatch")}
                  </p>
                ) : (
                  ordered.map(card)
                )}
              </div>
              {/* A dica só diz algo no servidor; no local o quadro não tem rodapé. */}
              {host && (
                <div className="flex shrink-0 items-center gap-2 border-t border-[var(--tyba-line)] px-3.5 py-2.5 text-[11px] leading-4 text-tyba-text-faint">
                  <p className="min-w-0 flex-1">{t("dockerPanelNoteRemote", { host })}</p>
                </div>
              )}
            </div>

            <div className="flex min-w-0 flex-1 flex-col">
              {selected ? (
                <>
                  <ContainerStrip
                    key={selected.id}
                    containerId={selected.id}
                    current="logs"
                    host={host}
                    inScreen
                    onRestarted={() => setLogsRestart((n) => n + 1)}
                  />
                  {/* Quadro Docker: segue o respiro de 12px do bloco; nos logs
                      o fio é o da linha de filtros, embaixo. */}
                  <div
                    className={`flex shrink-0 items-center gap-2.5 px-5 pt-3 ${
                      state.pane === "logs" ? "" : "tyba-divide-b pb-3"
                    }`}
                  >
                    {/* Segmentado do quadro (Docker): trilho em pílula, knob branco. */}
                    <SegmentedControl
                      aria-label={t("dockerScreenPanes")}
                      value={state.pane}
                      onChange={showPane}
                      options={CONTAINER_PANES.map((pane) => ({
                        value: pane,
                        label: t(PANE_LABEL[pane]),
                      }))}
                    />
                    {selected.state !== "running" && (
                      <Button
                        variant="destructive"
                        size="sm"
                        className="ml-auto"
                        disabled={busy}
                        onClick={() => void remove(selected)}
                      >
                        <Trash />
                        {t("dockerPanelRemove")}
                      </Button>
                    )}
                  </div>
                  <div className="relative min-h-0 flex-1">
                    {state.pane === "logs" && (
                      <ContainerLogsView
                        key={selected.id}
                        containerId={selected.id}
                        host={host}
                        restartNonce={logsRestart}
                      />
                    )}
                    {state.pane === "shell" &&
                      (selected.state === "running" ? (
                        <ShellPane
                          containerId={selected.id}
                          host={host}
                          exitedOf={exitedOf}
                          onSessionsChanged={onSessionsChanged}
                        />
                      ) : (
                        <p className="px-[18px] py-3 text-[12px] text-tyba-text-faint">
                          {t("dockerScreenShellStopped")}
                        </p>
                      ))}
                    {state.pane === "files" &&
                      (selected.state === "running" ? (
                        <div className="absolute inset-0 flex">
                          <ContainerFilesPanel
                            key={selected.id}
                            containerId={selected.id}
                            host={host}
                          />
                        </div>
                      ) : (
                        <p className="px-[18px] py-3 text-[12px] text-tyba-text-faint">
                          {t("dockerScreenFilesStopped")}
                        </p>
                      ))}
                    {state.pane === "details" && (
                      <DetailsPane key={selected.id} containerId={selected.id} host={host} />
                    )}
                  </div>
                </>
              ) : list === null ? null : (
                <div className="flex flex-1 items-center justify-center text-[12px] text-tyba-text-faint">
                  {list.length > 0 ? t("dockerScreenPick") : t("dockerPanelEmptyAll")}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
