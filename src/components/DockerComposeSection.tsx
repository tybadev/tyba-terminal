import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowClockwise, MagnifyingGlass, Play, Stop } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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
  type ComposeRun,
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

const GRID = "grid grid-cols-[150px_180px_minmax(0,1fr)_190px_150px_120px] gap-3";

/** Cor do ponto de estado (o do container) por tom. */
const TONE_DOT: Record<keyof typeof TONE, string> = {
  green: "bg-tyba-green-fill",
  amber: "bg-tyba-amber",
  red: "bg-tyba-red",
  faint: "bg-tyba-text-ghost",
};

const OP_LABEL: Record<ComposeOp, string> = {
  up: "composeUp",
  restart: "composeRestart",
  down: "composeDown",
};

/**
 * Saída do último compose do projeto, ao vivo enquanto roda e guardada até o
 * próximo (estado do core). Segue o fim, a menos que a pessoa tenha rolado
 * para cima.
 */
function ComposeOutputPanel({ run }: { run: ComposeRun }) {
  const { t } = useTranslation();
  const boxRef = useRef<HTMLPreElement>(null);
  const stick = useRef(true);
  useLayoutEffect(() => {
    const box = boxRef.current;
    if (box && stick.current) box.scrollTop = box.scrollHeight;
  }, [run.total]);
  if (!run.last_op) return null;
  const status = run.op
    ? { text: t("dockerScreenComposeRunning"), tone: "text-tyba-amber" }
    : run.error
      ? { text: t("dockerScreenComposeFailed"), tone: "text-tyba-red" }
      : { text: t("dockerScreenComposeDone"), tone: "text-tyba-green" };
  return (
    // Quadro Docker: o bloco do log do container — fio só no topo, sem faixa.
    <div className="flex h-[200px] shrink-0 flex-col border-t border-[var(--tyba-line)] pt-1.5">
      <div className="flex shrink-0 items-center gap-2.5 text-[12px] leading-[22px] text-tyba-text-faint">
        <span className="text-tyba-text-muted">
          {t("dockerScreenComposeOutput", { op: t(OP_LABEL[run.last_op]) })}
        </span>
        <span className={status.tone}>{status.text}</span>
      </div>
      <pre
        ref={boxRef}
        aria-live="polite"
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
        className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-all pb-1.5 font-mono text-[12px] leading-[22px] text-tyba-text-muted"
      >
        {run.output.length > 0
          ? run.output.join("\n")
          : t(run.op ? "dockerScreenComposeNoOutput" : "dockerScreenComposeNoOutputDone")}
      </pre>
    </div>
  );
}

/**
 * Seção Compose da tela de Docker, na linguagem da de Containers (quadro
 * Docker): coluna da lista à direita com todos os projetos compose do alvo, e o projeto escolhido com as
 * ações (ver compose, abrir projeto, up/restart/down) e a tabela de serviços.
 * Projeto escolhido e "ocultar parados" são do core.
 */
export function DockerComposeSection({
  projects,
  loaded,
  loading,
  selected,
  hideStopped,
  host,
  session,
  workspaceId,
  runs,
  onError,
  onChanged,
}: {
  projects: ComposeProject[];
  /** A lista de containers já chegou (antes disso não se escolhe projeto). */
  loaded: boolean;
  /** O carregamento da tela (esqueleto + etapa), enquanto `!loaded`. */
  loading: React.ReactNode;
  selected: string | null;
  hideStopped: boolean;
  /** Alias SSH numa sessão remota; `null` = docker local. */
  host: string | null;
  session: SessionId;
  workspaceId: WorkspaceId;
  /** Compose rodando e última falha por projeto — estado do core. */
  runs: Record<string, ComposeRun>;
  onError: (message: string | null) => void;
  /** Relista (containers e compose em andamento). */
  onChanged: () => Promise<void> | void;
}) {
  const { t } = useTranslation();

  const visible = visibleProjects(projects, hideStopped);
  // Filtro da lista, como o de containers: só recorta o que se vê.
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const shown = needle ? visible.filter((p) => p.name.toLowerCase().includes(needle)) : visible;
  const currentName = pickProject(visible, selected);
  const current = visible.find((p) => p.name === currentName) ?? null;

  // Sem escolha (ou o escolhido sumiu/ficou oculto): o do repo, ou o primeiro.
  useEffect(() => {
    if (!loaded || currentName === null || currentName === selected) return;
    void dockerScreenProject(workspaceId, currentName).catch(() => {});
  }, [loaded, currentName, selected, workspaceId]);

  const pick = (name: string) => void dockerScreenProject(workspaceId, name).catch(() => {});
  const fail = (e: unknown) => onError(translateError(e, t));

  // O core recusa um segundo compose no mesmo projeto e guarda a falha; a
  // tela só mostra o que ele diz.
  const compose = async (p: ComposeProject, op: ComposeOp) => {
    if (runs[p.name]?.op) return;
    if (op === "down") {
      const ok = await requestConfirm({
        title: t("dockerScreenComposeDownTitle", { project: p.name }),
        detail: t("dockerScreenComposeDownDetail"),
        confirmLabel: t("composeDown"),
        destructive: true,
      });
      if (!ok) return;
    }
    const done = dockerComposeOp(p.name, op).catch(() => {});
    void onChanged();
    await done;
    await onChanged();
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
        // Cartão do container (quadro Docker, .q-ct): escolhido = a placa
        // branca do knob, contorno inteiro; nunca barra lateral.
        className={`flex w-full flex-col gap-[3px] rounded-[10px] px-3 py-2.5 text-left text-tyba-text outline-none focus-visible:ring-2 focus-visible:ring-tyba-blue ${
          on ? "tyba-pill-on" : "hover:bg-tyba-text/[.03]"
        }`}
      >
        <span className="flex w-full items-center gap-2 text-[13px] font-semibold">
          <span aria-hidden className={`size-[7px] shrink-0 rounded-full ${DOT[run]}`} />
          <span className="min-w-0 truncate">{p.name}</span>
          <span className="ml-auto shrink-0 font-mono text-[10.5px] font-medium text-tyba-text-faint">
            {t("dockerScreenProjectRunning", { running: p.running, total: p.services.length })}
          </span>
        </span>
        <span className="flex w-full gap-2 pl-[15px] text-[11.5px] text-tyba-text-muted">
          <span className="min-w-0 truncate">{p.workingDir ? compactPath(p.workingDir) : "—"}</span>
          {p.inRepo && (
            <span className="ml-auto shrink-0 text-tyba-blue">{t("dockerScreenThisRepo")}</span>
          )}
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
      <span
        className={`flex min-w-0 items-center gap-1.5 text-[12px] font-semibold ${TONE[st.tone]}`}
        title={c.status}
      >
        <span aria-hidden className={`size-[7px] shrink-0 rounded-full ${TONE_DOT[st.tone]}`} />
        <span className="truncate">
          {label}
          {span && ` ${t("dockerScreenAgo", { span })}`}
        </span>
      </span>
    );
  };

  const row = (c: ContainerInfo) => {
    const up = c.state === "running";
    const ports = portMappings(c.ports);
    const image = shortImage(c.image);
    return (
      <div
        key={c.id}
        className={`${GRID} items-center rounded-[10px] px-3 py-2 hover:bg-tyba-text/[.03]`}
      >
        <span className="truncate text-[13px] font-semibold text-tyba-text">
          {c.service ?? c.name}
        </span>
        <span className="truncate font-mono text-[11.5px] text-tyba-text-muted" title={c.name}>
          {c.name}
        </span>
        <span className="truncate font-mono text-[11.5px] text-tyba-text-faint" title={image.full}>
          {image.short}
        </span>
        {stateCell(c)}
        <span className="truncate font-mono text-[11.5px] text-tyba-text-muted">
          {ports.length > 0 ? ports.join(", ") : "—"}
        </span>
        <span className="flex gap-1">
          <Button
            variant="ghost"
            size="xs"
            onClick={() => void dockerScreenShow(workspaceId, c.id, "logs").catch(fail)}
          >
            {t("dockerPanelLogs")}
          </Button>
          {up && (
            <Button
              variant="ghost"
              size="xs"
              onClick={() => void dockerScreenShow(workspaceId, c.id, "shell").catch(fail)}
            >
              {t("dockerPanelShell")}
            </Button>
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
      (p.fileStatus && p.fileStatus !== "ok" ? t(UNREACHABLE[p.fileStatus]) : null);
    const file = p.composeFile ?? "compose.yml";
    const running = runs[p.name]?.op ?? null;
    const lastError = runs[p.name]?.error ?? null;
    // Quadro Docker: Subir em vidro com play, Reiniciar em vidro com a seta,
    // Parar fantasma com o quadrado.
    const opChip = (op: ComposeOp, label: string) => (
      <Button
        variant={op === "down" ? "ghost" : "outline"}
        size="sm"
        disabled={blocked !== null || running !== null}
        title={blocked ?? undefined}
        onClick={() => void compose(p, op)}
      >
        {op === "up" ? <Play /> : op === "restart" ? <ArrowClockwise /> : <Stop />}
        {running === op ? `${label}…` : label}
      </Button>
    );
    // Quadro Docker (.q-dk-main): o bloco do container — 14px 20px, 12px
    // entre as linhas, sem faixa de fundo.
    return (
      <div className="flex min-h-0 flex-1 flex-col gap-3 px-5 py-3.5">
        <div className="flex shrink-0 items-center gap-2.5">
          <div className="flex min-w-0 flex-1 items-center gap-2.5">
            <span className="min-w-0 max-w-[40%] shrink-0 truncate text-[14px] font-bold text-tyba-text">
              {p.name}
            </span>
            <span
              className="min-w-0 truncate font-mono text-[11.5px] text-tyba-text-muted"
              title={p.workingDir ?? undefined}
            >
              {p.workingDir ? compactPath(p.workingDir) : "—"}
            </span>
            <span
              className={`flex shrink-0 items-center gap-1.5 text-[12px] font-semibold ${RUN_TEXT[run]}`}
            >
              <span aria-hidden className={`size-[7px] rounded-full ${DOT[run]}`} />
              {t("dockerScreenProjectRunningLong", { running: p.running, total: p.services.length })}
            </span>
          </div>
          <Button
            variant="outline"
            size="sm"
            disabled={fileBlocked !== null}
            title={fileBlocked ?? undefined}
            onClick={() => void dockerOpenComposeFile(session, p.name).catch(fail)}
          >
            {t("dockerScreenViewCompose", { file })}
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={blocked !== null}
            title={blocked ?? undefined}
            onClick={() => void dockerOpenProject(p.name).catch(fail)}
          >
            {t("openProject")}
          </Button>
          <span aria-hidden className="h-[18px] w-px shrink-0 bg-[var(--tyba-line)]" />
          {opChip("up", t("composeUp"))}
          {opChip("restart", t("composeRestart"))}
          {opChip("down", t("composeDown"))}
        </div>
        {lastError && <p className="shrink-0 text-[12px] text-tyba-red">{lastError}</p>}
        {unreachable && (
          <p className="shrink-0 text-[12px] text-tyba-text-faint">{t(UNREACHABLE[unreachable])}</p>
        )}
        {/* Colunas e linhas alinham o texto com o cabeçalho; o -mx-3 dá o
            respiro do hover da linha. */}
        <div className={`${GRID} -mx-3 shrink-0 px-3 text-[12px] text-tyba-text-faint`}>
          <span>{t("dockerScreenColService")}</span>
          <span>{t("dockerScreenColContainer")}</span>
          <span>{t("dockerScreenColImage")}</span>
          <span>{t("dockerScreenColState")}</span>
          <span>{t("dockerScreenColPorts")}</span>
          <span />
        </div>
        <div className="-mx-3 -mt-2 flex min-h-0 flex-1 flex-col overflow-y-auto">
          {p.services.map(row)}
        </div>
        {runs[p.name] && <ComposeOutputPanel key={p.name} run={runs[p.name]} />}
        {/* O cartão do Muriki que o app já usa (SshTypedNote): placa, sem borda. */}
        <div className="tyba-piece flex shrink-0 flex-col gap-1 rounded-[8px] px-3.5 py-3 text-[12px] text-tyba-text-muted">
          <span className="font-semibold text-tyba-text">{t("dockerScreenComposeDownCardTitle")}</span>
          <span>{t("dockerScreenComposeDownCardBody")}</span>
        </div>
        {/* Rodapé do log do container (.q-dk-ft): 12px, sem faixa. */}
        <div className="flex shrink-0 items-center gap-4 text-[12px] leading-4 text-tyba-text-muted">
          <span className="shrink-0">
            {t("dockerScreenComposeFooter", { file, count: p.services.length })}
          </span>
          <span className="min-w-0 flex-1 truncate text-right">
            {t("dockerScreenComposeFooterHint")}
          </span>
        </div>
      </div>
    );
  };

  return (
    <div className="flex min-h-0 flex-1 flex-row-reverse">
      {/* A mesma coluna da lista de containers (DockerScreen). */}
      <div className="flex w-[clamp(220px,40%,320px)] shrink-0 flex-col border-l border-[var(--tyba-line)] bg-[var(--tyba-panel-bg)]">
        <div className="shrink-0 px-4 pb-1.5 pt-3.5 text-[12.5px] text-tyba-text-muted">
          {t("dockerScreenProjectsCount", { count: visible.length })}
        </div>
        <div className="shrink-0 px-3 pb-1.5">
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
            placeholder={t("dockerScreenComposeFilter")}
            aria-label={t("dockerScreenComposeFilter")}
          />
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto px-3 pb-3.5">
          {!loaded ? (
            <div className="-mx-3">{loading}</div>
          ) : visible.length === 0 ? (
            <p className="px-2 py-2 text-[12px] text-tyba-text-faint">
              {projects.length > 0 ? t("dockerScreenNoRunningProjects") : t("dockerScreenNoProjects")}
            </p>
          ) : shown.length === 0 ? (
            <p className="px-2 py-2 text-[12px] text-tyba-text-faint">{t("dockerScreenComposeNoMatch")}</p>
          ) : (
            shown.map(card)
          )}
        </div>
        <p className="shrink-0 border-t border-[var(--tyba-line)] px-3.5 py-2.5 text-[11px] leading-4 text-tyba-text-faint">
          {t("dockerScreenComposeNote")}
        </p>
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        {current ? (
          content(current)
        ) : loaded ? (
          <div className="flex flex-1 items-center justify-center text-[12px] text-tyba-text-faint">
            {t("dockerScreenNoProjects")}
          </div>
        ) : null}
      </div>
    </div>
  );
}
