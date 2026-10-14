import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Cube,
  HardDrives,
  MagnifyingGlass,
  Stack,
  X,
  type Icon,
} from "@phosphor-icons/react";

import "./gitScreen.css";
import { ContainerFilesPanel } from "./ContainerFilesPanel";
import { ContainerLogsView } from "./ContainerLogsView";
import { ContainerShellBanner, ContainerStrip } from "./ContainerStrip";
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
  portList,
  shortImage,
  uptimeMs,
} from "../lib/dockerPanel";
import { translateError } from "../lib/errors";
import {
  dockerAvailable,
  dockerContainerDetail,
  dockerContainerOp,
  dockerListContainers,
  dockerListImages,
  dockerListVolumes,
  dockerOpenDesktop,
  dockerPanelUsage,
  dockerRemoveContainer,
  dockerScreenLogs,
  dockerScreenPane,
  dockerScreenSection,
  dockerScreenSelect,
  dockerScreenShell,
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
/** Acima disso aparece o filtro. */
const FILTER_THRESHOLD = 8;

const RAIL: Record<DockerSection, { icon: Icon; label: string }> = {
  containers: { icon: Cube, label: "dockerScreenContainers" },
  images: { icon: Stack, label: "dockerScreenImages" },
  volumes: { icon: HardDrives, label: "dockerScreenVolumes" },
};

const PANE_LABEL: Record<ContainerPane, string> = {
  logs: "dockerPanelLogs",
  shell: "dockerPanelShell",
  files: "containerFiles",
  details: "dockerScreenDetails",
};

const BTN =
  "shrink-0 whitespace-nowrap rounded-[6px] border border-tyba-border-strong px-2.5 py-1 text-[12px] text-tyba-text-muted hover:text-tyba-text disabled:opacity-50";

const FULL_RECT = { left: 0, top: 0, width: 100, height: 100 } as const;

/** O shell do container dentro da tela: `docker exec` no container, nunca na
 *  máquina local — a faixa azul diz isso antes de qualquer comando. Em sessão
 *  SSH os logs são a mesma coisa sem a faixa: um `docker logs -f` no servidor. */
function ShellPane({
  kind = "shell",
  containerId,
  host,
  exitedOf,
  onSessionsChanged,
}: {
  kind?: "shell" | "logs";
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
    void (kind === "logs" ? dockerScreenLogs : dockerScreenShell)(containerId, host)
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
  }, [kind, containerId, host, nonce, t]);

  const exited = sessionId ? exitedOf(sessionId) : false;
  return (
    <div className="absolute inset-0 flex flex-col">
      {kind === "shell" && <ContainerShellBanner containerId={containerId} />}
      {error ? (
        <p role="alert" className="px-[18px] py-3 text-[12px] text-tyba-red">
          {error}
        </p>
      ) : (
        <div className="relative min-h-0 flex-1 bg-tyba-sunken">
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
            <button
              type="button"
              onClick={() => setNonce((n) => n + 1)}
              className={`absolute bottom-3 right-4 ${BTN}`}
            >
              {t(kind === "logs" ? "dockerScreenLogsReopen" : "dockerScreenShellReopen")}
            </button>
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
  useEffect(() => {
    let alive = true;
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
  }, [containerId, host, t]);
  if (error) {
    return <p className="px-[18px] py-3 text-[12px] text-tyba-red">{error}</p>;
  }
  if (!detail) {
    return <p className="px-[18px] py-3 text-[12px] text-tyba-text-faint">{t("filesLoading")}</p>;
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
  }, [section, host, t]);
  const rows = section === "images" ? images : volumes;
  return (
    <div className="flex min-h-0 flex-1 flex-col bg-tyba-sunken">
      <div className="shrink-0 border-b border-[var(--tyba-border-soft)] bg-[var(--tyba-panel-bg)] px-[18px] py-2.5 text-[11px] text-tyba-text-faint">
        {section === "images"
          ? t("dockerScreenImagesCount", { count: images?.length ?? 0 })
          : t("dockerScreenVolumesCount", { count: volumes?.length ?? 0 })}
        {" · "}
        {t("dockerScreenReadOnly")}
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-[18px] py-2 text-[12.5px]">
        {error ? (
          <p role="alert" className="py-2 text-tyba-red">
            {error}
          </p>
        ) : rows === null ? (
          <p className="py-2 text-tyba-text-faint">{t("filesLoading")}</p>
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
 * Imagens, Volumes), coluna de 300px com os containers do alvo da sessão (a
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
  onOpenDashboard,
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
  /** O workspace Docker antigo (projetos compose: subir/derrubar). */
  onOpenDashboard: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [available, setAvailable] = useState<boolean | null>(null);
  const [list, setList] = useState<ContainerInfo[] | null>(null);
  const [usage, setUsage] = useState<PanelUsage | null>(null);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [logsRestart, setLogsRestart] = useState(0);

  const sectionRef = useRef<HTMLElement>(null);
  useEffect(() => {
    sectionRef.current?.focus({ preventScroll: true });
  }, []);

  const refresh = useCallback(async () => {
    try {
      const ok = await dockerAvailable(host);
      setAvailable(ok);
      if (!ok) {
        setList([]);
        return;
      }
      const next = await dockerListContainers(host ? null : repoRoot, true, host);
      for (const c of next) rememberContainerName(c.id, c.name);
      setList(next);
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
        className={`flex w-full flex-col gap-[5px] rounded-[6px] px-3 py-2.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-tyba-blue ${
          on
            ? "bg-tyba-raised shadow-[var(--tyba-raised-ring)]"
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
      className="tyba-git-screen absolute inset-0 z-30 flex flex-row-reverse bg-tyba-bg outline-none"
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
                on ? "bg-tyba-blue/[.12] text-tyba-blue" : "text-tyba-text-faint hover:text-tyba-text"
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
        <div className="flex h-[46px] shrink-0 items-center gap-3 tyba-divide-b bg-[var(--tyba-panel-bg)] px-[18px] text-[12px]">
          <PanelIcon kind="docker" size={16} className="shrink-0 text-tyba-blue" />
          <span className="shrink-0 text-[14px] font-medium text-tyba-text">
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
              <button
                type="button"
                className={BTN}
                disabled={busy || !repoContainers.some((c) => c.state !== "running")}
                onClick={() => void bulk("start")}
              >
                {t("dockerScreenStartAll")}
              </button>
              <button
                type="button"
                className={BTN}
                disabled={busy || !repoContainers.some((c) => c.state === "running")}
                onClick={() => void bulk("stop")}
              >
                {t("dockerScreenStopAll")}
              </button>
            </>
          )}
        </div>

        {available === false ? (
          <div className="flex flex-1 flex-col items-start gap-2 px-[18px] py-4 text-[12px] text-tyba-text-muted">
            {t("dockerUnavailable")}
            {!host && (
              <button
                type="button"
                className={BTN}
                onClick={() => void dockerOpenDesktop().catch(() => {})}
              >
                {t("dockerPanelOpenDesktop")}
              </button>
            )}
          </div>
        ) : state.section !== "containers" ? (
          <ReadOnlyList section={state.section} host={host} />
        ) : (
          // Coluna à direita, junto do trilho, como em Arquivos (dono, 2026-10-14).
          <div className="flex min-h-0 flex-1 flex-row-reverse">
            <div className="flex w-[300px] shrink-0 flex-col border-l border-[var(--tyba-border-soft)] bg-[var(--tyba-panel-bg)]">
              <div className="flex shrink-0 items-center gap-2 px-3.5 pb-1.5 pt-3 text-[11px] text-tyba-text-faint">
                <span className="flex-1">
                  {t("dockerScreenContainersCount", { count: list?.length ?? 0 })}
                </span>
              </div>
              {(list?.length ?? 0) > FILTER_THRESHOLD && (
                <div className="shrink-0 px-3 pb-2">
                  <label className="flex items-center gap-2 rounded-[6px] border border-tyba-border-field bg-tyba-sunken px-2.5 py-[5px] text-[12px] text-tyba-text-faint">
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
                      className="min-w-0 flex-1 bg-transparent text-tyba-text outline-none placeholder:text-tyba-text-faint"
                    />
                  </label>
                </div>
              )}
              <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 py-1">
                {list === null ? (
                  <p className="px-2 py-2 text-[12px] text-tyba-text-faint">{t("filesLoading")}</p>
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
              <div className="flex shrink-0 items-center gap-2 border-t border-[var(--tyba-border-soft)] px-3.5 py-2.5 text-[11px] leading-4 text-tyba-text-faint">
                <p className="min-w-0 flex-1">
                  {host ? t("dockerPanelNoteRemote", { host }) : t("dockerScreenNoteLocal")}
                </p>
                {!host && (
                  <button
                    type="button"
                    onClick={onOpenDashboard}
                    className="shrink-0 hover:text-tyba-text"
                  >
                    {t("dockerScreenComposeProjects")}
                  </button>
                )}
              </div>
            </div>

            <div className="flex min-w-0 flex-1 flex-col bg-tyba-sunken">
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
                  <div className="flex shrink-0 items-center gap-2.5 border-b border-[var(--tyba-border-soft)] px-[18px] py-2">
                    <span
                      role="tablist"
                      aria-label={t("dockerScreenPanes")}
                      className="flex gap-0.5 rounded-[8px] bg-[var(--tyba-panel-bg)] p-[3px]"
                    >
                      {CONTAINER_PANES.map((pane) => {
                        const on = pane === state.pane;
                        // Arquivos lê o docker local; logs remotos viram terminal.
                        const disabled = host !== null && pane === "files";
                        return (
                          <button
                            key={pane}
                            type="button"
                            role="tab"
                            aria-selected={on}
                            disabled={disabled}
                            title={disabled ? t("dockerScreenLocalOnly") : undefined}
                            onClick={() => showPane(pane)}
                            className={`rounded-[6px] px-3 py-[5px] text-[12px] disabled:opacity-40 ${
                              on
                                ? "bg-tyba-raised text-tyba-text shadow-[var(--tyba-raised-ring)]"
                                : "text-tyba-text-faint hover:text-tyba-text"
                            }`}
                          >
                            {t(PANE_LABEL[pane])}
                          </button>
                        );
                      })}
                    </span>
                    {selected.state !== "running" && (
                      <button
                        type="button"
                        className={`ml-auto ${BTN}`}
                        disabled={busy}
                        onClick={() => void remove(selected)}
                      >
                        {t("dockerPanelRemove")}
                      </button>
                    )}
                  </div>
                  <div className="relative min-h-0 flex-1">
                    {state.pane === "logs" &&
                      (host ? (
                        <ShellPane
                          key={selected.id}
                          kind="logs"
                          containerId={selected.id}
                          host={host}
                          exitedOf={exitedOf}
                          onSessionsChanged={onSessionsChanged}
                        />
                      ) : (
                        <ContainerLogsView
                          key={selected.id}
                          containerId={selected.id}
                          restartNonce={logsRestart}
                        />
                      ))}
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
                      (host ? (
                        <p className="px-[18px] py-3 text-[12px] text-tyba-text-faint">
                          {t("dockerScreenLocalOnly")}
                        </p>
                      ) : selected.state === "running" ? (
                        <div className="absolute inset-0 flex">
                          <ContainerFilesPanel key={selected.id} containerId={selected.id} />
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
              ) : (
                <div className="flex flex-1 items-center justify-center text-[12px] text-tyba-text-faint">
                  {list && list.length > 0 ? t("dockerScreenPick") : t("dockerPanelEmptyAll")}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
