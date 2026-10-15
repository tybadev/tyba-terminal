import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ArrowsLeftRight, ListBullets, Plug, X, type Icon } from "@phosphor-icons/react";

import "./gitScreen.css";
import { TunnelsView } from "./TunnelsView";
import { requestConfirm } from "../lib/confirm";
import { openExternalUrl, writeClipboardText } from "../lib/clipboard";
import { compactSpan, formatBytes, formatCpu } from "../lib/dockerPanel";
import { translateError } from "../lib/errors";
import {
  listSessionTunnels,
  servicesBringHere,
  servicesDetail,
  servicesForceKill,
  servicesKill,
  servicesList,
  servicesProcesses,
  servicesScreenOnlyMine,
  servicesScreenSection,
  servicesScreenSelect,
  type KillOutcome,
  type KillRefusal,
  type PortEntry,
  type RemotePort,
  type ServiceDetail,
  type ServiceTreeNode,
  type ServicesList,
  type ServicesScreenState,
  type ServicesSection,
  type Session,
  type SessionId,
  type SessionProcs,
  type SessionTunnel,
  type WorkspaceId,
} from "../lib/ipc";
import { IS_MAC } from "../lib/platform";
import { escClosesScreen, SERVICES_SECTIONS } from "../lib/screens";
import {
  filterPorts,
  forceWaitSeconds,
  originText,
  portTone,
  portUrl,
  refusalKey,
  REMOTE_POLL_MS,
  SCREEN_POLL_MS,
} from "../lib/services";
import { pushToast } from "../lib/toast";

const RAIL: Record<ServicesSection, { icon: Icon; label: string }> = {
  ports: { icon: Plug, label: "servicesPorts" },
  processes: { icon: ListBullets, label: "servicesProcesses" },
  tunnels: { icon: ArrowsLeftRight, label: "tunnelsTitle" },
};

/** Passou disso sem resposta: "demorando mais que o normal". */
const SLOW_LOCAL_MS = 4000;
const SLOW_REMOTE_MS = 10000;

const GRID = "grid grid-cols-[90px_170px_minmax(0,1fr)_150px_90px_120px_250px] items-center gap-3";

const CHIP =
  "shrink-0 whitespace-nowrap rounded-[5px] border border-tyba-border-strong px-[9px] py-[3px] text-[11px] text-tyba-text-muted hover:text-tyba-text disabled:opacity-50";
const CHIP_DANGER = `${CHIP} text-tyba-red hover:text-tyba-red`;

const DOT: Record<ReturnType<typeof portTone>, string> = {
  green: "text-tyba-green",
  amber: "text-tyba-amber",
  faint: "text-tyba-text-ghost",
};

type ForceOffer = { sentAt: number; forceAfterMs: number };

const keyOf = (pid: number, startMs: number) => `${pid}:${startMs}`;

/** Sistema e outro usuário: só leitura (mockup), sem Abrir nem Copiar. */
const readOnly = (refusal: KillRefusal | null) =>
  refusal === "system" || refusal === "other_user" || refusal === "pid1";

/** Esqueleto no formato da tabela + o filamento correndo (carregamento). */
function Loading({ step, slowSeconds, onRetry }: { step: string; slowSeconds: number | null; onRetry: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-col">
      <div className="tyba-load-filament" aria-hidden />
      <p className="px-[18px] pb-1 pt-2.5 text-[11px] text-tyba-text-faint" role="status">
        {step}
      </p>
      {slowSeconds !== null && (
        <p className="flex items-center gap-2 px-[18px] pb-1 text-[11px] text-tyba-amber">
          {t("servicesSlow", { seconds: slowSeconds })}
          <button type="button" className={CHIP} onClick={onRetry}>
            {t("servicesRetry")}
          </button>
        </p>
      )}
      {[0, 1, 2, 3, 4].map((i) => (
        <div key={i} className={`${GRID} px-[18px] py-[13px]`} aria-hidden>
          {[50, 90, 200, 80, 50, 70, 120].map((w, j) => (
            <span key={j} className="h-2.5 rounded-[3px] bg-tyba-text/[.06]" style={{ width: Math.min(w, 200) }} />
          ))}
        </div>
      ))}
    </div>
  );
}

/** Contador até o SIGKILL ficar disponível (segundo comando, nova confirmação). */
function ForceButton({ offer, onForce }: { offer: ForceOffer; onForce: () => void }) {
  const { t } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  const wait = forceWaitSeconds(offer.sentAt, offer.forceAfterMs, now);
  useEffect(() => {
    if (wait <= 0) return;
    const timer = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(timer);
  }, [wait]);
  return (
    <button type="button" className={CHIP_DANGER} disabled={wait > 0} onClick={onForce}>
      {wait > 0 ? t("servicesForceIn", { seconds: wait }) : t("servicesForce")}
    </button>
  );
}

/**
 * Tela cheia de Serviços e portas (mockup Servicos): trilho à direita (Portas,
 * Processos, Túneis), barra de topo de 46px, tabela das portas em escuta com o
 * detalhe da escolhida embaixo. Seção, filtro e porta escolhida são do core; a
 * lista vem do core a cada ~3 s só enquanto a tela está aberta.
 */
export function ServicesScreen({
  session,
  workspaceId,
  state,
  hostAlias,
  exiting,
  labelOf,
  onFocusSession,
  onClose,
}: {
  session: Session;
  workspaceId: WorkspaceId;
  state: ServicesScreenState;
  /** Alias do Host numa sessão SSH; `null` = máquina local. */
  hostAlias: string | null;
  exiting: boolean;
  /** Nome de exibição da sessão (o do workspace dela). */
  labelOf: (id: SessionId) => string | null;
  onFocusSession: (id: SessionId) => void;
  onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const remote = hostAlias !== null;
  const [list, setList] = useState<ServicesList | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [loadingSince, setLoadingSince] = useState<number | null>(null);
  const [slow, setSlow] = useState<number | null>(null);
  const [detail, setDetail] = useState<ServiceDetail | null>(null);
  const [procs, setProcs] = useState<SessionProcs[] | null>(null);
  const [tunnels, setTunnels] = useState<SessionTunnel[]>([]);
  const [force, setForce] = useState<Record<string, ForceOffer>>({});
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);

  const sectionRef = useRef<HTMLElement>(null);
  useEffect(() => {
    sectionRef.current?.focus({ preventScroll: true });
  }, []);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setLoadingSince((cur) => cur ?? Date.now());
    try {
      if (state.section === "processes") {
        if (!remote) setProcs(await servicesProcesses());
      } else if (state.section === "ports") {
        const next = await servicesList(session.id);
        setList(next);
        if (next.target === "remote") {
          setTunnels(await listSessionTunnels(session.id).catch(() => []));
        }
      }
      setError(null);
    } catch (e) {
      setError(translateError(e, t));
    } finally {
      inFlight.current = false;
      setLoadingSince(null);
      setSlow(null);
    }
  }, [state.section, remote, session.id, t]);

  // Poll só com a tela aberta (o componente só existe com ela aberta).
  useEffect(() => {
    if (state.section === "tunnels") return;
    void refresh();
    const timer = window.setInterval(() => void refresh(), remote ? REMOTE_POLL_MS : SCREEN_POLL_MS);
    return () => window.clearInterval(timer);
  }, [refresh, state.section, remote]);

  // "Demorando mais que o normal": só na primeira carga (a lista ainda não existe).
  useEffect(() => {
    if (loadingSince === null) return;
    const limit = remote ? SLOW_REMOTE_MS : SLOW_LOCAL_MS;
    const timer = window.setInterval(() => {
      const elapsed = Date.now() - loadingSince;
      if (elapsed >= limit) setSlow(Math.floor(elapsed / 1000));
    }, 500);
    return () => window.clearInterval(timer);
  }, [loadingSince, remote]);

  const localPorts = list?.target === "local" ? list.ports : null;
  const visible = useMemo(
    () => (localPorts ? filterPorts(localPorts, state.only_mine) : []),
    [localPorts, state.only_mine],
  );
  const isSelected = (p: PortEntry) =>
    state.selected !== null && p.port === state.selected.port && p.pid === state.selected.pid;
  const selected = visible.find(isSelected) ?? null;

  // Detalhe da escolhida: árvore e grupo, frescos a cada volta da lista.
  const selectedKey = selected ? keyOf(selected.pid, selected.start_ms) : null;
  // Trocou de linha: o detalhe da anterior não vale para esta.
  useEffect(() => {
    setDetail(null);
  }, [selectedKey]);
  useEffect(() => {
    if (!selected) {
      setDetail(null);
      return;
    }
    let alive = true;
    void servicesDetail(selected.pid)
      .then((d) => alive && setDetail(d))
      .catch(() => alive && setDetail(null));
    return () => {
      alive = false;
    };
    // `list` entra para refazer a árvore a cada retrato.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedKey, list]);

  const go = (section: ServicesSection) =>
    void servicesScreenSection(workspaceId, section).catch(() => {});
  const pick = (p: PortEntry) =>
    void servicesScreenSelect(
      workspaceId,
      isSelected(p) ? null : { port: p.port, pid: p.pid },
    ).catch(() => {});

  const describeOutcome = (label: string, outcome: KillOutcome, pid: number, startMs: number) => {
    const k = keyOf(pid, startMs);
    if (outcome.exited) {
      setForce((cur) => {
        const { [k]: _gone, ...rest } = cur;
        return rest;
      });
      setNotice(t("servicesKilled", { what: label }));
    } else if (outcome.force_after_ms > 0) {
      setForce((cur) => ({ ...cur, [k]: { sentAt: Date.now(), forceAfterMs: outcome.force_after_ms } }));
      setNotice(t("servicesStillRunning", { what: label }));
    } else {
      // Depois do SIGKILL não há outra oferta: o core já esqueceu o TERM.
      setForce((cur) => {
        const { [k]: _gone, ...rest } = cur;
        return rest;
      });
      setNotice(t("servicesStillRunning", { what: label }));
    }
  };

  /**
   * Confirmação do encerrar (TERM ou KILL): porta, PID, comando e quem vai
   * junto, conferidos agora no core. Sem conseguir conferir o grupo, não
   * confirma nada. Devolve o grupo confirmado (`null` = só o processo), que
   * vai junto no comando; `undefined` = cancelado.
   */
  const confirmTarget = async (
    target: { pid: number; comm: string; command: string },
    label: string,
    force: boolean,
  ): Promise<number | null | undefined> => {
    let fresh: ServiceDetail | null = null;
    try {
      fresh = await servicesDetail(target.pid);
    } catch {
      fresh = null;
    }
    if (!fresh?.target) {
      setError(t("servicesKillUnverified"));
      return undefined;
    }
    const group = fresh.target.kind === "group" ? fresh.target.pgid : null;
    const groupLine =
      fresh.target.kind === "group"
        ? t("servicesKillGroup", {
            pgid: fresh.target.pgid,
            members: fresh.group.map((m) => `${m.comm} ${m.pid}`).join(", "),
          })
        : t("servicesKillOnlyProcess");
    const ok = await requestConfirm({
      title: t(force ? "servicesForceTitle" : "servicesKillTitle", { what: label }),
      detail: [
        t("servicesKillWho", { pid: target.pid, command: target.command || target.comm }),
        groupLine,
        t(force ? "servicesForceDetail" : "servicesKillHow"),
      ].join("\n"),
      confirmLabel: t(force ? "servicesForce" : "servicesKill"),
      destructive: true,
    });
    return ok ? group : undefined;
  };

  /** Encerrar: confirma mostrando porta, PID, comando e quem vai junto. */
  const kill = async (target: { pid: number; start_ms: number; comm: string; command: string; port?: number }) => {
    if (busy) return;
    setError(null);
    const label = target.port ? `:${target.port}` : `${target.comm} ${target.pid}`;
    const group = await confirmTarget(target, label, false);
    if (group === undefined) return;
    setBusy(true);
    try {
      const outcome = await servicesKill(target.pid, target.start_ms, group);
      describeOutcome(label, outcome, target.pid, target.start_ms);
      await refresh();
    } catch (e) {
      setError(translateError(e, t));
    } finally {
      setBusy(false);
    }
  };

  /** SIGKILL: nova confirmação, com o mesmo retrato de quem vai junto. */
  const forceKill = async (target: { pid: number; start_ms: number; comm: string; command: string; port?: number }) => {
    if (busy) return;
    setError(null);
    const label = target.port ? `:${target.port}` : `${target.comm} ${target.pid}`;
    const group = await confirmTarget(target, label, true);
    if (group === undefined) return;
    setBusy(true);
    try {
      const outcome = await servicesForceKill(target.pid, target.start_ms, group);
      describeOutcome(label, outcome, target.pid, target.start_ms);
      await refresh();
    } catch (e) {
      setError(translateError(e, t));
    } finally {
      setBusy(false);
    }
  };

  const goToTerminal = (id: SessionId) => {
    onClose();
    onFocusSession(id);
  };

  const copyUrl = (port: number) =>
    void writeClipboardText(portUrl(port)).then(
      () => pushToast({ tone: "info", title: t("servicesCopied", { url: portUrl(port) }) }),
      () => {},
    );

  const killControls = (
    item: { pid: number; start_ms: number; comm: string; command: string; refusal: KillRefusal | null },
    port?: number,
  ) => {
    if (item.refusal) {
      return <span className="truncate text-[11px] text-tyba-text-faint">{t(refusalKey(item.refusal))}</span>;
    }
    const offer = force[keyOf(item.pid, item.start_ms)];
    return offer ? (
      <ForceButton offer={offer} onForce={() => void forceKill({ ...item, port })} />
    ) : (
      <button type="button" className={CHIP_DANGER} disabled={busy} onClick={() => void kill({ ...item, port })}>
        {t("servicesKill")}
      </button>
    );
  };

  const usage = (cpu: number | null, rss: number | null) => {
    const c = formatCpu(cpu, i18n.language);
    const m = formatBytes(rss, i18n.language);
    return [c && `CPU ${c}`, m].filter(Boolean).join(" · ") || "—";
  };

  const row = (p: PortEntry) => {
    const on = isSelected(p);
    const since = p.start_ms > 0 ? compactSpan(Date.now() - p.start_ms) : null;
    return (
      <div
        key={`${p.port}:${p.pid}`}
        role="row"
        aria-selected={on}
        tabIndex={0}
        onClick={() => pick(p)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            pick(p);
          }
        }}
        title={p.addresses.join(", ")}
        // Escolhida = fundo elevado com contorno inteiro; nunca barra lateral.
        className={`${GRID} cursor-default rounded-[6px] px-[18px] py-2.5 text-[12.5px] outline-none focus-visible:ring-2 focus-visible:ring-tyba-blue ${
          on ? "bg-tyba-raised shadow-[var(--tyba-raised-ring)]" : "hover:bg-tyba-text/[.03]"
        }`}
      >
        <span className="text-[14px] text-tyba-text">
          <span className={DOT[portTone(p.origin)]} aria-hidden>
            ●
          </span>{" "}
          :{p.port}
        </span>
        <span className="truncate text-tyba-text">{p.comm || "—"}</span>
        <span className="truncate font-mono text-tyba-text-muted" title={p.command}>
          {p.command || "—"}
        </span>
        <span className="truncate text-tyba-text-muted">{originText(p.origin, t, labelOf)}</span>
        <span className="text-tyba-text-faint">{since ? t("dockerScreenAgo", { span: since }) : "—"}</span>
        <span className="truncate text-tyba-text-faint">{usage(p.cpu_percent, p.rss_bytes)}</span>
        <span className="flex justify-end gap-1.5" onClick={(e) => e.stopPropagation()}>
          {readOnly(p.refusal) ? null : (
            <>
              <button type="button" className={CHIP} onClick={() => void openExternalUrl(portUrl(p.port))}>
                {t("servicesOpen")}
              </button>
              <button type="button" className={CHIP} onClick={() => copyUrl(p.port)}>
                {t("servicesCopyUrl")}
              </button>
            </>
          )}
          {killControls(p, p.port)}
        </span>
      </div>
    );
  };

  const detailCard = (p: PortEntry) => {
    const sessionId = p.origin.kind === "session" ? p.origin.session_id : null;
    return (
      <div className="mx-[18px] my-3.5 flex flex-col rounded-[10px] bg-[var(--tyba-panel-bg)] shadow-[inset_0_0_0_1px_var(--tyba-border-soft)]">
        <div className="flex items-center gap-2.5 border-b border-[var(--tyba-border-soft)] px-4 py-3">
          <span className="text-[15px] text-tyba-text">:{p.port}</span>
          <span className="min-w-0 truncate font-mono text-[12px] text-tyba-text-muted">{p.command || p.comm}</span>
          <span className="shrink-0 text-[11px] text-tyba-text-faint">
            {[
              `PID ${p.pid}`,
              p.pgid !== null && t("servicesGroup", { pgid: p.pgid }),
              sessionId && t("servicesSessionOf", { name: labelOf(sessionId) ?? t("servicesOriginSession") }),
            ]
              .filter(Boolean)
              .join(" · ")}
          </span>
          <span className="flex-1" />
          {!readOnly(p.refusal) && (
            <button
              type="button"
              className="text-[12px] text-tyba-blue hover:underline"
              onClick={() => void openExternalUrl(portUrl(p.port))}
            >
              {portUrl(p.port)}
            </button>
          )}
        </div>
        <div className="grid grid-cols-2">
          <div className="flex flex-col gap-1.5 border-r border-[var(--tyba-border-soft)] px-4 py-3 text-[12px] text-tyba-text-muted">
            <span className="text-[11px] uppercase text-tyba-text-faint">{t("servicesTree")}</span>
            {(detail?.tree ?? []).map((n) => (
              <span key={n.pid} className="truncate" style={{ paddingLeft: n.depth * 14 }} title={n.command}>
                {n.comm} <span className="text-tyba-text-faint">{n.pid}</span>
                {n.ports.map((port) => (
                  <span key={port} className="text-tyba-green">
                    {" "}● :{port}
                  </span>
                ))}
              </span>
            ))}
            {!detail && <span className="text-tyba-text-faint">{t("servicesChecking")}</span>}
          </div>
          <div className="flex flex-col gap-2 px-4 py-3 text-[12px] text-tyba-text-muted">
            <span className="text-[11px] uppercase text-tyba-text-faint">{t("servicesKill")}</span>
            {p.refusal ? (
              <span>{t(refusalKey(p.refusal))}</span>
            ) : !detail?.target ? (
              // Sem o grupo conferido, o card não afirma quem vai junto.
              <span className="text-tyba-text-faint">{t("servicesChecking")}</span>
            ) : (
              <span>
                {detail.target.kind === "group"
                  ? t("servicesKillExplainGroup", {
                      members: detail.group.map((m) => m.comm).join(", "),
                    })
                  : t("servicesKillExplainProcess")}
              </span>
            )}
            <span className="flex gap-2">
              {!p.refusal && detail?.target && killControls(p, p.port)}
              {sessionId && (
                <button type="button" className={CHIP} onClick={() => goToTerminal(sessionId)}>
                  {t("servicesGoToTerminal")}
                </button>
              )}
            </span>
            <span className="text-[11px] text-tyba-text-faint">{t("servicesKillRule")}</span>
          </div>
        </div>
      </div>
    );
  };

  const tunnelFor = (port: number) =>
    tunnels.find((tn) => tn.kind === "local" && tn.target_port === port && tn.state.state !== "error");

  const bring = async (port: number) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const tn = await servicesBringHere(session.id, port);
      setNotice(t("servicesBrought", { remote: port, local: tn.listen_port }));
      setTunnels(await listSessionTunnels(session.id).catch(() => []));
    } catch (e) {
      setError(translateError(e, t));
    } finally {
      setBusy(false);
    }
  };

  const remoteRow = (p: RemotePort) => {
    const tn = tunnelFor(p.port);
    return (
      <div
        key={p.port}
        className="grid grid-cols-[90px_200px_minmax(0,1fr)_260px] items-center gap-3 rounded-[6px] px-[18px] py-2.5 text-[12.5px] hover:bg-tyba-text/[.03]"
      >
        <span className="text-[14px] text-tyba-text">
          <span className="text-tyba-green" aria-hidden>
            ●
          </span>{" "}
          :{p.port}
        </span>
        <span className="truncate text-tyba-text">{p.process ?? "—"}</span>
        <span className="truncate text-tyba-text-faint">
          {p.addresses.join(", ")}
          {p.pid !== null && ` · PID ${p.pid}`}
        </span>
        <span className="flex justify-end gap-1.5">
          {tn ? (
            <>
              <span className="self-center text-[11px] text-tyba-text-faint">
                {t("servicesBroughtAt", { port: tn.listen_port })}
              </span>
              <button type="button" className={CHIP} onClick={() => void openExternalUrl(portUrl(tn.listen_port))}>
                {t("servicesOpen")}
              </button>
            </>
          ) : (
            <button type="button" className={CHIP} disabled={busy} onClick={() => void bring(p.port)}>
              {t("servicesBringHere")}
            </button>
          )}
        </span>
      </div>
    );
  };

  const processRow = (n: ServiceTreeNode) => (
    <div
      key={n.pid}
      className="grid grid-cols-[minmax(0,1.2fr)_70px_minmax(0,2fr)_110px_130px_180px] items-center gap-3 rounded-[6px] px-[18px] py-[7px] text-[12px] hover:bg-tyba-text/[.03]"
    >
      <span className="truncate text-tyba-text" style={{ paddingLeft: n.depth * 14 }}>
        {n.comm || "—"}
      </span>
      <span className="text-tyba-text-faint tabular-nums">{n.pid}</span>
      <span className="truncate font-mono text-tyba-text-muted" title={n.command}>
        {n.command || "—"}
      </span>
      <span className="truncate text-tyba-green">{n.ports.map((p) => `:${p}`).join(" ")}</span>
      <span className="truncate text-tyba-text-faint">{usage(n.cpu_percent, n.rss_bytes)}</span>
      <span className="flex justify-end">{killControls(n)}</span>
    </div>
  );

  const step = remote
    ? t("servicesStepRemote", { host: hostAlias })
    : state.section === "processes"
      ? t("servicesStepProcesses")
      : t(IS_MAC ? "servicesStepLsof" : "servicesStepProc");

  const topBar = (icon: Icon, title: string, sub: string, extra?: ReactNode) => {
    const TitleIcon = icon;
    return (
      <div className="flex h-[46px] shrink-0 items-center gap-3 tyba-divide-b bg-[var(--tyba-panel-bg)] px-[18px] text-[12px]">
        <TitleIcon size={16} className="shrink-0 text-tyba-blue" />
        <span className="shrink-0 text-[14px] font-medium text-tyba-text">{title}</span>
        <span className="min-w-0 flex-1 truncate text-tyba-text-faint">{sub}</span>
        {error ? (
          <span role="alert" className="min-w-0 max-w-[40%] truncate text-tyba-red" title={error}>
            {error}
          </span>
        ) : (
          notice && (
            <span role="status" className="min-w-0 max-w-[40%] truncate text-tyba-text-muted" title={notice}>
              {notice}
            </span>
          )
        )}
        {extra}
        <button type="button" className={CHIP} onClick={() => void refresh()}>
          {t("servicesRefresh")}
        </button>
      </div>
    );
  };

  const target = remote ? (hostAlias ?? "") : t("servicesThisMachine");

  const portsSection = () => {
    if (list === null) {
      return error ? null : <Loading step={step} slowSeconds={slow} onRetry={() => void refresh()} />;
    }
    if (list.target === "remote") {
      return (
        <div className="min-h-0 flex-1 overflow-y-auto py-1">
          {list.ports.length === 0 ? (
            <p className="px-[18px] py-3 text-[12px] text-tyba-text-faint">{t("servicesEmptyRemote")}</p>
          ) : (
            list.ports.map(remoteRow)
          )}
        </div>
      );
    }
    return (
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className={`${GRID} border-b border-[var(--tyba-border-soft)] px-[18px] py-2 text-[11px] text-tyba-text-faint`}>
          <span>{t("servicesColPort")}</span>
          <span>{t("servicesColProcess")}</span>
          <span>{t("servicesColCommand")}</span>
          <span>{t("servicesColOrigin")}</span>
          <span>{t("servicesColSince")}</span>
          <span>{t("servicesColUsage")}</span>
          <span />
        </div>
        <div role="grid" aria-label={t("servicesPorts")} className="flex flex-col bg-tyba-sunken py-1">
          {visible.length === 0 ? (
            <p className="px-[18px] py-3 text-[12px] text-tyba-text-faint">
              {state.only_mine && (localPorts?.length ?? 0) > 0
                ? t("servicesEmptyMine", { count: localPorts?.length ?? 0 })
                : t("servicesEmpty")}
            </p>
          ) : (
            visible.map(row)
          )}
        </div>
        {selected && detailCard(selected)}
      </div>
    );
  };

  const processesSection = () => {
    if (remote) {
      return <p className="px-[18px] py-3 text-[12px] text-tyba-text-faint">{t("servicesProcessesLocalOnly")}</p>;
    }
    if (procs === null) {
      return error ? null : <Loading step={step} slowSeconds={slow} onRetry={() => void refresh()} />;
    }
    if (procs.length === 0) {
      return <p className="px-[18px] py-3 text-[12px] text-tyba-text-faint">{t("servicesNoSessions")}</p>;
    }
    return (
      <div className="min-h-0 flex-1 overflow-y-auto bg-tyba-sunken py-1">
        {procs.map((group) => (
          <div key={group.session_id} className="flex flex-col py-1.5">
            <div className="flex items-center gap-2 px-[18px] py-1.5 text-[11px] text-tyba-text-faint">
              <span className="text-[12px] text-tyba-text">{labelOf(group.session_id) ?? t("servicesOriginSession")}</span>
              <span>{t("servicesProcessCount", { count: group.nodes.length })}</span>
              <span className="flex-1" />
              <button type="button" className={CHIP} onClick={() => goToTerminal(group.session_id)}>
                {t("servicesGoToTerminal")}
              </button>
            </div>
            {group.nodes.map(processRow)}
          </div>
        ))}
      </div>
    );
  };

  const portsCount = list?.target === "local" ? visible.length : (list?.ports.length ?? 0);

  return (
    <section
      ref={sectionRef}
      tabIndex={-1}
      aria-label={t("servicesTitle")}
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
        aria-label={t("servicesSections")}
        className="relative flex w-[52px] shrink-0 flex-col items-center gap-1.5 tyba-divide-l bg-[var(--tyba-panel-bg)] pt-3"
      >
        {SERVICES_SECTIONS.map((key) => {
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
        {state.section === "tunnels" ? (
          remote ? (
            <TunnelsView session={session} hostAlias={hostAlias ?? ""} />
          ) : (
            <>
              {topBar(ArrowsLeftRight, t("tunnelsTitle"), target)}
              <p className="px-[18px] py-3 text-[12px] text-tyba-text-faint">{t("servicesTunnelsSshOnly")}</p>
            </>
          )
        ) : state.section === "processes" ? (
          <>
            {topBar(
              ListBullets,
              t("servicesProcesses"),
              procs ? t("servicesProcessesSub", { count: procs.length }) : target,
            )}
            {processesSection()}
          </>
        ) : (
          <>
            {topBar(
              Plug,
              t("servicesTitle"),
              list ? t("servicesSub", { target, count: portsCount }) : target,
              !remote && (
                <label className="flex shrink-0 items-center gap-1.5 text-tyba-text-muted">
                  <input
                    type="checkbox"
                    checked={state.only_mine}
                    onChange={(e) => void servicesScreenOnlyMine(workspaceId, e.target.checked).catch(() => {})}
                    className="accent-tyba-blue"
                  />
                  {t("servicesOnlyMine")}
                </label>
              ),
            )}
            {portsSection()}
          </>
        )}
        {state.section !== "tunnels" && (
          <div className="flex h-[26px] shrink-0 items-center gap-4 border-t border-[var(--tyba-border-soft)] bg-[var(--tyba-panel-bg)] px-[18px] text-[11px] text-tyba-text-faint">
            <span>
              {remote
                ? t("servicesFooterRemote", { host: hostAlias })
                : t(IS_MAC ? "servicesFooterLsof" : "servicesFooterProc")}
            </span>
            <span className="flex-1" />
            {!remote && <span>{t("servicesFooterSsh")}</span>}
          </div>
        )}
      </div>
    </section>
  );
}
