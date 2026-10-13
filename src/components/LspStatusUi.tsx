// Estado do LSP de um arquivo aberto no editor: o chip da linha de status e
// as faixas de instalação (oferta gerenciada, download, servidor ausente).
// Veio do antigo visualizador do painel Arquivos, sem mudar o comportamento.

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowClockwise,
  Copy,
  DownloadSimple,
  SealCheck,
  Terminal,
  X,
} from "@phosphor-icons/react";

import {
  lspManagedConsent,
  lspManagedDownload,
  lspManagedUseMine,
  lspRetry,
  lspStatus,
  onLspManagedError,
  onLspManagedProgress,
  onLspStatus,
  type LspManagedProgress,
  type LspStatus,
  type SessionId,
} from "@/lib/ipc";
import { toastError } from "@/lib/toast";

const LIVE = new Set(["starting", "restarting", "available", "installing"]);

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function pct(done: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(100, Math.round((done / total) * 100));
}

function managedPhaseKey(phase: LspManagedProgress["phase"]): string {
  switch (phase) {
    case "verifying":
      return "lspManagedVerifying";
    case "extracting":
      return "lspManagedExtracting";
    case "error":
      return "lspManagedError";
    default:
      return "lspManagedPreparing";
  }
}

/** Status do LSP do arquivo, reperguntado enquanto sobe/instala e quando o
 *  core avisa que o servidor caiu ou voltou. */
export function useLspStatus(session: SessionId, rel: string, remote: boolean) {
  const [stat, setStat] = useState<LspStatus | null>(null);
  const timer = useRef<number | undefined>(undefined);

  const refresh = useCallback(() => {
    if (remote) return;
    void lspStatus(session, rel)
      .then((s) => {
        setStat(s);
        if (s.state === "installing" && s.progress.phase === "pending") {
          void lspManagedDownload(session, s.server_id, rel)
            .then(setStat)
            .catch(() => {});
        }
        window.clearTimeout(timer.current);
        if (LIVE.has(s.state)) timer.current = window.setTimeout(refresh, 1500);
      })
      .catch(() => {});
  }, [session, rel, remote]);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  useEffect(() => {
    if (remote) return;
    const un = onLspStatus(session, () => refresh());
    return () => {
      void un.then((f) => f());
    };
  }, [session, remote, refresh]);

  const installingId = stat?.state === "installing" ? stat.server_id : null;
  useEffect(() => {
    if (!installingId) return;
    let alive = true;
    const apply = (progress: LspManagedProgress) => {
      if (!alive) return;
      setStat((prev) =>
        prev?.state === "installing" && prev.server_id === installingId
          ? { ...prev, progress }
          : prev,
      );
      if (progress.phase === "ready") {
        void lspRetry(session, rel)
          .then((s) => {
            setStat(s);
            refresh();
          })
          .catch(() => {});
      } else if (progress.phase === "error") {
        refresh();
      }
    };
    const unP = onLspManagedProgress(installingId, apply);
    const unE = onLspManagedError(installingId, apply);
    return () => {
      alive = false;
      void unP.then((f) => f());
      void unE.then((f) => f());
    };
  }, [installingId, session, rel, refresh]);

  const retry = useCallback(() => {
    void lspRetry(session, rel)
      .then((s) => {
        setStat(s);
        if (LIVE.has(s.state)) refresh();
      })
      .catch(() => {});
  }, [session, rel, refresh]);

  return { stat, setStat, refresh, retry };
}

interface ChipProps {
  stat: LspStatus | null;
  truncated: boolean;
  onRetry: () => void;
}

/** O começo da linha de status do editor: o servidor e em que pé ele está. */
export function LspStatusChip({ stat, truncated, onRetry }: ChipProps) {
  const { t } = useTranslation();
  if (!stat || stat.state === "unsupported") return null;
  const dot = (cls: string, pulse = false) => (
    <span aria-hidden className={`size-1.5 rounded-full ${cls} ${pulse ? "animate-pulse" : ""}`} />
  );
  switch (stat.state) {
    case "ready":
      return (
        <span className="flex items-center gap-1.5" title={truncated ? t("lspTruncated") : stat.server}>
          {dot("bg-tyba-green-fill shadow-[var(--tyba-glow-green)]")}
          {t("editorLspServer", { server: stat.server })}
          {(truncated || stat.truncated) && <span className="font-mono">+</span>}
        </span>
      );
    case "experimental":
      return (
        <span
          title={t("lspExperimentalHint", { server: stat.server })}
          className="uppercase tracking-wide"
        >
          {t("lspExperimental")}
        </span>
      );
    case "starting":
    case "available":
      return (
        <span className="flex items-center gap-1.5" title={stat.server}>
          {dot("bg-tyba-amber-fill", true)}
          {t("lspStarting")}
        </span>
      );
    case "restarting":
      return (
        <span className="flex items-center gap-1.5" title={stat.reason ?? stat.server}>
          {dot("bg-tyba-amber-fill", true)}
          {t("lspRestarting")}
        </span>
      );
    case "crashed":
      return (
        <button
          type="button"
          onClick={onRetry}
          title={`${stat.reason ? t("lspCrashedReason", { reason: stat.reason }) : t("lspCrashed")} — ${t("lspRetryHint")}`}
          className="flex min-w-0 items-center gap-1 text-tyba-red hover:text-tyba-text"
        >
          <span className="truncate">
            {stat.reason ? t("lspCrashedReason", { reason: stat.reason }) : t("lspCrashed")}
          </span>
          <ArrowClockwise size={11} className="shrink-0" />
        </button>
      );
    case "installing":
      return (
        <span className="flex items-center gap-1.5" title={stat.server}>
          {dot("bg-tyba-blue", true)}
          {stat.progress.phase === "downloading"
            ? t("lspManagedDownloading", {
                percent: pct(stat.progress.downloaded, stat.progress.total),
              })
            : t(managedPhaseKey(stat.progress.phase))}
        </span>
      );
    case "absent":
      return (
        <span className="text-tyba-amber" title={stat.missing ?? undefined}>
          {t("lspAbsent", { server: stat.server })}
        </span>
      );
    case "managed_offer":
      return <span className="text-tyba-blue">{t("lspManagedOffer", { server: stat.server })}</span>;
  }
}

interface BannerProps {
  session: SessionId;
  rel: string;
  stat: LspStatus | null;
  setStat: (s: LspStatus) => void;
  refresh: () => void;
  onRunInTerminal: (command: string) => void;
}

/** Faixas sob o breadcrumb: oferta de download verificado, progresso e o
 *  comando de instalação quando o servidor falta. */
export function LspBanners({ session, rel, stat, setStat, refresh, onRunInTerminal }: BannerProps) {
  const { t } = useTranslation();
  const [hideManaged, setHideManaged] = useState(false);
  const [hideInstall, setHideInstall] = useState(false);
  const [showAlts, setShowAlts] = useState(false);
  const [copied, setCopied] = useState(false);

  const accept = (serverId: string) =>
    void lspManagedConsent(session, serverId, "accept", rel)
      .then((s) => {
        setStat(s);
        refresh();
      })
      .catch((e) => toastError(String(e)));
  const refuse = (serverId: string) => {
    setHideManaged(true);
    void lspManagedConsent(session, serverId, "refuse", rel).catch(() => {});
  };
  const useMine = (label: string, serverId: string) =>
    void lspManagedUseMine(serverId)
      .then((hints) => {
        setStat({
          state: "absent",
          server: label,
          missing: null,
          install: hints.install,
          alternatives: hints.alternatives,
        });
        setHideInstall(false);
      })
      .catch((e) => toastError(String(e)));
  const copy = (command: string) =>
    void navigator.clipboard
      ?.writeText(command)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});

  if (stat?.state === "managed_offer" && !hideManaged) {
    return (
      <div className="flex shrink-0 flex-col gap-2 border-b border-tyba-blue/40 bg-tyba-blue/10 px-4 py-2.5 text-[11px] text-tyba-text">
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5">
              <span className="truncate font-medium">{stat.card.label}</span>
              <span className="shrink-0 font-mono text-tyba-text-faint">v{stat.card.version}</span>
              <span title={t("lspManagedVerifiedHint")} className="flex shrink-0 items-center gap-0.5 text-tyba-green">
                <SealCheck size={12} weight="fill" />
                {t("lspManagedVerified")}
              </span>
            </div>
            {stat.missing && (
              <div className="mt-0.5 text-[10px]">{t("lspManagedMissing", { missing: stat.missing })}</div>
            )}
            <div className="mt-0.5 text-[10px] text-tyba-text-muted">
              {t("lspManagedMeta", { size: formatBytes(stat.card.size), source: stat.card.source })}
            </div>
          </div>
          <button
            type="button"
            onClick={() => refuse(stat.card.server_id)}
            aria-label={t("tunnelsClose")}
            className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
          >
            <X size={12} />
          </button>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => accept(stat.card.server_id)}
            className="flex items-center gap-1 rounded-[3px] bg-tyba-blue/20 px-2 py-0.5 hover:bg-tyba-blue/30"
          >
            <DownloadSimple size={12} />
            {t("lspManagedAccept")}
          </button>
          <button
            type="button"
            onClick={() => useMine(stat.card.label, stat.card.server_id)}
            className="rounded-[3px] px-2 py-0.5 text-tyba-text-muted hover:bg-tyba-text/[.08] hover:text-tyba-text"
          >
            {t("lspManagedUseMine")}
          </button>
          <button
            type="button"
            onClick={() => refuse(stat.card.server_id)}
            className="ml-auto text-tyba-text-faint hover:text-tyba-text"
          >
            {t("lspManagedRefuse")}
          </button>
        </div>
      </div>
    );
  }

  if (stat?.state === "installing") {
    return (
      <div className="flex shrink-0 flex-col gap-1.5 border-b border-tyba-blue/40 bg-tyba-blue/10 px-4 py-2 text-[11px] text-tyba-text">
        <div className="flex items-center gap-2">
          <DownloadSimple size={13} className="shrink-0 text-tyba-blue" />
          <span className="min-w-0 flex-1 truncate">
            {stat.progress.phase === "downloading"
              ? t("lspManagedDownloadingOf", {
                  server: stat.server,
                  done: formatBytes(stat.progress.downloaded),
                  total: formatBytes(stat.progress.total),
                })
              : stat.progress.phase === "error"
                ? stat.progress.message
                : t(managedPhaseKey(stat.progress.phase))}
          </span>
          {stat.progress.phase === "error" && (
            <button
              type="button"
              onClick={() => accept(stat.server_id)}
              title={t("lspManagedRetry")}
              aria-label={t("lspManagedRetry")}
              className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
            >
              <ArrowClockwise size={12} />
            </button>
          )}
        </div>
        {stat.progress.phase === "downloading" && (
          <div className="h-1 w-full overflow-hidden rounded-full bg-tyba-text/[.1]">
            <div
              className="h-full rounded-full bg-tyba-blue transition-all"
              style={{ width: `${pct(stat.progress.downloaded, stat.progress.total)}%` }}
            />
          </div>
        )}
      </div>
    );
  }

  if (stat?.state === "absent" && !hideInstall) {
    const chosen = stat.install;
    return (
      <div className="flex shrink-0 flex-col gap-1.5 border-b border-tyba-amber/40 bg-tyba-amber/10 px-4 py-2 text-[11px] text-tyba-text">
        <div className="flex items-center gap-2">
          <span title={stat.missing ?? undefined} className="min-w-0 flex-1 truncate">
            {stat.missing ?? t("lspInstallHint", { server: stat.server })}
          </span>
          <button
            type="button"
            onClick={() => setHideInstall(true)}
            aria-label={t("tunnelsClose")}
            className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
          >
            <X size={12} />
          </button>
        </div>
        {chosen ? (
          <>
            <div className="flex items-center gap-2">
              <code className="min-w-0 flex-1 truncate rounded-[3px] bg-tyba-surface px-2 py-1 font-mono">
                {chosen.command}
              </code>
              <button
                type="button"
                onClick={() => copy(chosen.command)}
                title={t("lspCopy")}
                aria-label={t("lspCopy")}
                className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
              >
                <Copy size={13} />
              </button>
            </div>
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => onRunInTerminal(chosen.command)}
                className="flex items-center gap-1 rounded-[3px] bg-tyba-text/[.08] px-2 py-0.5 hover:bg-tyba-text/[.14]"
              >
                <Terminal size={12} />
                {t("lspInstallInTerminal")}
              </button>
              {copied && <span className="text-tyba-green">{t("lspCopied")}</span>}
              {stat.alternatives.length > 0 && (
                <button
                  type="button"
                  onClick={() => setShowAlts((v) => !v)}
                  className="ml-auto text-tyba-text-faint hover:text-tyba-text"
                >
                  {showAlts
                    ? t("lspHideAlternatives")
                    : t("lspAlternatives", { count: stat.alternatives.length })}
                </button>
              )}
            </div>
            {showAlts &&
              stat.alternatives.map((alt) => (
                <div key={alt.manager + alt.command} className="flex items-center gap-2">
                  <span className="w-14 shrink-0 text-[10px] uppercase tracking-wide text-tyba-text-faint">
                    {alt.manager}
                  </span>
                  <code className="min-w-0 flex-1 truncate rounded-[3px] bg-tyba-surface px-2 py-1 font-mono text-tyba-text-muted">
                    {alt.command}
                  </code>
                  <button
                    type="button"
                    onClick={() => copy(alt.command)}
                    title={t("lspCopy")}
                    aria-label={t("lspCopy")}
                    className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
                  >
                    <Copy size={12} />
                  </button>
                  <button
                    type="button"
                    onClick={() => onRunInTerminal(alt.command)}
                    title={t("lspInstallInTerminal")}
                    aria-label={t("lspInstallInTerminal")}
                    className="shrink-0 text-tyba-text-faint hover:text-tyba-text"
                  >
                    <Terminal size={12} />
                  </button>
                </div>
              ))}
          </>
        ) : (
          <span className="text-tyba-text-muted">{t("lspNoInstaller", { server: stat.server })}</span>
        )}
      </div>
    );
  }
  return null;
}
