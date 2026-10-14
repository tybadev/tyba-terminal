import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { requestConfirm } from "../lib/confirm";
import {
  compactSpan,
  shortImage,
} from "../lib/dockerPanel";
import {
  pickProject,
  portMappings,
  projectRun,
  serviceState,
  visibleProjects,
  type ComposeProject,
  type ProjectRun,
} from "../lib/dockerCompose";
import { translateError } from "../lib/errors";
import {
  dockerComposeOp,
  dockerOpenComposeFile,
  dockerOpenProject,
  dockerScreenProject,
  dockerScreenShow,
  type ComposeOp,
  type ContainerInfo,
  type PathStatus,
  type SessionId,
  type WorkspaceId,
} from "../lib/ipc";
import { compactPath } from "../lib/workspaceCwd";

const DOT: Record<ProjectRun, string> = {
  all: "bg-tyba-green-fill",
  some: "bg-tyba-amber",
  none: "bg-tyba-text-ghost",
};

const RUN_TEXT: Record<ProjectRun, string> = {
  all: "text-tyba-green",
  some: "text-tyba-amber",
  none: "text-tyba-text-faint",
};

const TONE: Record<"green" | "amber" | "red" | "faint", string> = {
  green: "text-tyba-green",
  amber: "text-tyba-amber",
  red: "text-tyba-red",
  faint: "text-tyba-text-faint",
};

const UNREACHABLE: Record<Exclude<PathStatus, "ok">, string> = {
  missing: "containersPathOffHost",
  denied: "containersPathDenied",
  unsupported: "containersPathUnsupported",
};

const CHIP =
  "shrink-0 whitespace-nowrap rounded-[5px] border border-tyba-border-strong px-[9px] py-[3px] text-[11px] text-tyba-text-muted hover:text-tyba-text disabled:opacity-40 disabled:hover:text-tyba-text-muted";

const GRID = "grid grid-cols-[150px_180px_minmax(0,1fr)_190px_150px_120px] gap-3";

/**
 * Seção Compose da tela de Docker (mockup DockerCompose): coluna de 300px à
 * direita com todos os projetos compose do alvo, e o projeto escolhido com as
 * ações (ver compose, abrir projeto, up/restart/down) e a tabela de serviços.
 * Projeto escolhido e "ocultar parados" são do core.
 */
export function DockerComposeSection({
  projects,
  loaded,
  selected,
  hideStopped,
  host,
  session,
  workspaceId,
  onError,
  onChanged,
}: {
  projects: ComposeProject[];
  /** A lista de containers já chegou (antes disso não se escolhe projeto). */
  loaded: boolean;
  selected: string | null;
  hideStopped: boolean;
  /** Alias SSH numa sessão remota; `null` = docker local. */
  host: string | null;
  session: SessionId;
  workspaceId: WorkspaceId;
  onError: (message: string | null) => void;
  /** Depois de um compose: relista. */
  onChanged: () => Promise<void> | void;
}) {
  const { t } = useTranslation();
  const [running, setRunning] = useState<ComposeOp | null>(null);

  const visible = visibleProjects(projects, hideStopped);
  const currentName = pickProject(visible, selected);
  const current = visible.find((p) => p.name === currentName) ?? null;

  // Sem escolha (ou o escolhido sumiu/ficou oculto): o do repo, ou o primeiro.
  useEffect(() => {
    if (!loaded || currentName === null || currentName === selected) return;
    void dockerScreenProject(workspaceId, currentName).catch(() => {});
  }, [loaded, currentName, selected, workspaceId]);

  const pick = (name: string) => void dockerScreenProject(workspaceId, name).catch(() => {});
  const fail = (e: unknown) => onError(translateError(e, t));

  const compose = async (p: ComposeProject, op: ComposeOp) => {
    if (running) return;
    if (op === "down") {
      const ok = await requestConfirm({
        title: t("dockerScreenComposeDownTitle", { project: p.name }),
        detail: t("dockerScreenComposeDownDetail"),
        confirmLabel: t("composeDown"),
        destructive: true,
      });
      if (!ok) return;
    }
    setRunning(op);
    onError(null);
    try {
      await dockerComposeOp(p.name, op);
    } catch (e) {
      fail(e);
    } finally {
      setRunning(null);
      await onChanged();
    }
  };

  const card = (p: ComposeProject) => {
    const on = p.name === current?.name;
    const run = projectRun(p);
    return (
      <button
        key={p.name}
        type="button"
        aria-pressed={on}
        onClick={() => pick(p.name)}
        title={p.workingDir ?? p.name}
        // Escolhido = fundo elevado com contorno inteiro; nunca barra lateral.
        className={`flex w-full flex-col gap-[5px] rounded-[6px] px-3 py-2.5 text-left text-tyba-text outline-none focus-visible:ring-2 focus-visible:ring-tyba-blue ${
          on ? "bg-tyba-raised shadow-[var(--tyba-raised-ring)]" : "hover:bg-tyba-text/[.03]"
        }`}
      >
        <span className="flex w-full items-center justify-between gap-2">
          <span className="flex min-w-0 items-center gap-1.5 text-[13px]">
            <span aria-hidden className={`size-[7px] shrink-0 rounded-full ${DOT[run]}`} />
            <span className="truncate">{p.name}</span>
          </span>
          {p.inRepo && (
            <span className="shrink-0 rounded-[4px] border border-tyba-border-strong px-[5px] text-[10px] text-tyba-blue">
              {t("dockerScreenThisRepo")}
            </span>
          )}
        </span>
        <span className="flex w-full justify-between gap-2 text-[11px] text-tyba-text-faint">
          <span className="min-w-0 truncate">{p.workingDir ? compactPath(p.workingDir) : "—"}</span>
          <span className="shrink-0">
            {t("dockerScreenProjectRunning", { running: p.running, total: p.services.length })}
          </span>
        </span>
      </button>
    );
  };

  const stateCell = (c: ContainerInfo) => {
    const st = serviceState(c);
    const span = compactSpan(st.span);
    const label =
      st.key === "exited"
        ? st.code !== null
          ? t("dockerScreenSvcExited", { code: st.code })
          : c.status
        : t(
            st.key === "running"
              ? "dockerScreenSvcRunning"
              : st.key === "healthy"
                ? "dockerPanelHealthy"
                : st.key === "unhealthy"
                  ? "dockerPanelUnhealthy"
                  : "dockerPanelHealthStarting",
          );
    return (
      <span className={`truncate ${TONE[st.tone]}`} title={c.status}>
        ● {label}
        {span && ` ${t("dockerScreenAgo", { span })}`}
      </span>
    );
  };

  const row = (c: ContainerInfo) => {
    const up = c.state === "running";
    const ports = portMappings(c.ports);
    const image = shortImage(c.image);
    const link = "hover:text-tyba-text";
    return (
      <div
        key={c.id}
        className={`${GRID} items-center border-b border-[var(--tyba-border-soft)] px-[18px] py-[9px] text-[12.5px]`}
      >
        <span className="truncate text-tyba-text">{c.service ?? c.name}</span>
        <span className="truncate text-tyba-text-muted" title={c.name}>
          {c.name}
        </span>
        <span className="truncate text-tyba-text-faint" title={image.full}>
          {image.short}
        </span>
        {stateCell(c)}
        <span className="truncate text-tyba-text-muted">
          {ports.length > 0 ? ports.join(", ") : "—"}
        </span>
        <span className="flex gap-2.5 text-[11px] text-tyba-text-muted">
          <button
            type="button"
            className={link}
            onClick={() => void dockerScreenShow(workspaceId, c.id, "logs").catch(fail)}
          >
            {t("dockerPanelLogs")}
          </button>
          {up && (
            <button
              type="button"
              className={link}
              onClick={() => void dockerScreenShow(workspaceId, c.id, "shell").catch(fail)}
            >
              {t("dockerPanelShell")}
            </button>
          )}
        </span>
      </div>
    );
  };

  const content = (p: ComposeProject) => {
    const run = projectRun(p);
    const unreachable = !host && p.dirStatus && p.dirStatus !== "ok" ? p.dirStatus : null;
    // Remoto: o caminho do compose é do servidor. Fora da máquina: idem.
    const blocked = host
      ? t("composeRemoteDisabledHint")
      : unreachable
        ? t(UNREACHABLE[unreachable])
        : !p.workingDir
          ? t("containersPathOffHost")
          : null;
    const fileBlocked =
      blocked ??
      (p.fileStatus !== "ok"
        ? t(UNREACHABLE[(p.fileStatus ?? "missing") as Exclude<PathStatus, "ok">])
        : !p.inRepo
          ? t("dockerScreenComposeOutside")
          : null);
    const file = p.composeFile ?? "compose.yml";
    const opChip = (op: ComposeOp, label: string, tone = "") => (
      <button
        type="button"
        className={`${CHIP} ${tone}`}
        disabled={blocked !== null || running !== null}
        title={blocked ?? undefined}
        onClick={() => void compose(p, op)}
      >
        {running === op ? `${label}…` : label}
      </button>
    );
    return (
      <>
        <div className="flex shrink-0 items-center gap-2.5 border-b border-[var(--tyba-border-soft)] bg-[var(--tyba-panel-bg)] px-[18px] py-3 text-[12px] text-tyba-text-muted">
          <span className="shrink-0 text-[14px] text-tyba-text">{p.name}</span>
          <span className="min-w-0 truncate" title={p.workingDir ?? undefined}>
            {p.workingDir ? compactPath(p.workingDir) : "—"}
          </span>
          <span className={`shrink-0 ${RUN_TEXT[run]}`}>
            {t("dockerScreenProjectRunningLong", { running: p.running, total: p.services.length })}
          </span>
          <span className="flex-1" />
          <button
            type="button"
            className={CHIP}
            disabled={fileBlocked !== null}
            title={fileBlocked ?? undefined}
            onClick={() => void dockerOpenComposeFile(session, p.name).catch(fail)}
          >
            {t("dockerScreenViewCompose", { file })}
          </button>
          <button
            type="button"
            className={CHIP}
            disabled={blocked !== null}
            title={blocked ?? undefined}
            onClick={() => void dockerOpenProject(p.name).catch(fail)}
          >
            {t("openProject")}
          </button>
          <span aria-hidden className="h-[18px] w-px shrink-0 bg-[var(--tyba-border-soft)]" />
          {opChip("up", t("composeUp"), "text-tyba-green")}
          {opChip("restart", t("composeRestart"))}
          {opChip("down", t("composeDown"), "text-tyba-red")}
        </div>
        {unreachable && (
          <p className="shrink-0 border-b border-[var(--tyba-border-soft)] px-[18px] py-2 text-[11px] text-tyba-text-faint">
            {t(UNREACHABLE[unreachable])}
          </p>
        )}
        <div
          className={`${GRID} shrink-0 border-b border-[var(--tyba-border-soft)] px-[18px] py-2 text-[11px] text-tyba-text-faint`}
        >
          <span>{t("dockerScreenColService")}</span>
          <span>{t("dockerScreenColContainer")}</span>
          <span>{t("dockerScreenColImage")}</span>
          <span>{t("dockerScreenColState")}</span>
          <span>{t("dockerScreenColPorts")}</span>
          <span />
        </div>
        <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">{p.services.map(row)}</div>
        <div className="flex h-[26px] shrink-0 items-center gap-4 border-t border-[var(--tyba-border-soft)] bg-[var(--tyba-panel-bg)] px-[18px] text-[11px] text-tyba-text-faint">
          <span>{t("dockerScreenComposeFooter", { file, count: p.services.length })}</span>
          <span className="flex-1" />
          <span>{t("dockerScreenComposeFooterHint")}</span>
        </div>
      </>
    );
  };

  return (
    <div className="flex min-h-0 flex-1 flex-row-reverse">
      <div className="flex w-[300px] shrink-0 flex-col border-l border-[var(--tyba-border-soft)] bg-[var(--tyba-panel-bg)]">
        <div className="shrink-0 px-3.5 pb-1.5 pt-3 text-[11px] text-tyba-text-faint">
          {t("dockerScreenProjectsCount", { count: visible.length })}
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 py-1">
          {!loaded ? (
            <p className="px-2 py-2 text-[12px] text-tyba-text-faint">{t("filesLoading")}</p>
          ) : visible.length === 0 ? (
            <p className="px-2 py-2 text-[12px] text-tyba-text-faint">
              {projects.length > 0 ? t("dockerScreenNoRunningProjects") : t("dockerScreenNoProjects")}
            </p>
          ) : (
            visible.map(card)
          )}
        </div>
        <p className="shrink-0 border-t border-[var(--tyba-border-soft)] px-3.5 py-2.5 text-[11px] leading-4 text-tyba-text-faint">
          {t("dockerScreenComposeNote")}
        </p>
      </div>
      <div className="flex min-w-0 flex-1 flex-col bg-tyba-sunken">
        {current ? (
          content(current)
        ) : (
          <div className="flex flex-1 items-center justify-center text-[12px] text-tyba-text-faint">
            {loaded ? t("dockerScreenNoProjects") : t("filesLoading")}
          </div>
        )}
      </div>
    </div>
  );
}
