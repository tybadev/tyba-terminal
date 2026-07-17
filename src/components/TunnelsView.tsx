import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowsOutSimple,
  ArrowsInSimple,
  ArrowUUpLeft,
  Plus,
  Trash,
  X,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { RiskyTunnelConfirm } from "@/components/RiskyTunnelConfirm";
import type { Session, SessionTunnel, Tunnel, TunnelKind } from "@/lib/ipc";
import {
  closeSessionTunnel,
  listSessionTunnels,
  onSessionTunnels,
  openSessionTunnel,
} from "@/lib/ipc";
import { BLANK_TUNNEL, describeTunnel, tunnelFlag } from "@/lib/tunnels";

interface Props {
  session: Session;
  hostAlias: string;
  expanded: boolean;
  onToggleExpand: () => void;
  onClose: () => void;
}

function dotClass(t: SessionTunnel): string {
  if (t.state.state === "live") return "bg-tyba-green";
  if (t.state.state === "error") return "bg-tyba-red";
  return "bg-tyba-text-faint animate-pulse";
}

export function TunnelsView({
  session,
  hostAlias,
  expanded,
  onToggleExpand,
  onClose,
}: Props) {
  const { t } = useTranslation();
  const [tunnels, setTunnels] = useState<SessionTunnel[]>([]);
  const [draft, setDraft] = useState<Tunnel>(BLANK_TUNNEL);
  const [adding, setAdding] = useState(false);
  const [pendingRisky, setPendingRisky] = useState<Tunnel | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    void listSessionTunnels(session.id)
      .then(setTunnels)
      .catch(() => {});
  }, [session.id]);

  useEffect(refresh, [refresh]);

  useEffect(() => {
    const un = onSessionTunnels((p) => {
      if (p.session_id === session.id) setTunnels(p.tunnels);
    });
    return () => {
      void un.then((f) => f());
    };
  }, [session.id]);

  const submit = useCallback(
    async (tunnel: Tunnel, confirmed: boolean) => {
      setError(null);
      try {
        await openSessionTunnel(session.id, tunnel, confirmed);
        setDraft(BLANK_TUNNEL);
        setAdding(false);
        setPendingRisky(null);
        refresh();
      } catch (e) {
        const err = e as { code?: string; params?: Record<string, string> };
        if (err.code === "ssh.tunnel_needs_confirmation") {
          setPendingRisky(tunnel);
          return;
        }
        setError(err.params?.detail ?? err.code ?? String(e));
      }
    },
    [session.id, refresh],
  );

  const retry = useCallback(
    async (tn: SessionTunnel) => {
      await closeSessionTunnel(session.id, tn.id).catch(() => {});
      await submit(tn, true);
    },
    [session.id, submit],
  );

  const remove = useCallback(
    async (id: string) => {
      await closeSessionTunnel(session.id, id).catch(() => {});
      refresh();
    },
    [session.id, refresh],
  );

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-tyba-bg">
      <header className="flex h-8 shrink-0 items-center gap-2 border-b border-tyba-border px-3">
        <span className="min-w-0 truncate text-[12px] text-tyba-text">
          {t("tunnelsTitle")}
        </span>
        <span className="truncate font-mono text-[11px] text-tyba-text-muted">
          {hostAlias}
        </span>
        <div className="flex-1" />
        <button
          onClick={onToggleExpand}
          aria-label={t(expanded ? "tunnelsCollapse" : "tunnelsExpand")}
          className="text-tyba-text-faint hover:text-tyba-text"
        >
          {expanded ? (
            <ArrowsInSimple size={13} />
          ) : (
            <ArrowsOutSimple size={13} />
          )}
        </button>
        <button
          onClick={onClose}
          aria-label={t("tunnelsClose")}
          className="text-tyba-text-faint hover:text-tyba-text"
        >
          <X size={13} />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        {tunnels.length === 0 && !adding && (
          <p className="px-1 py-3 text-[12px] text-tyba-text-faint">
            {t("tunnelsEmpty")}
          </p>
        )}

        <ul className="flex flex-col gap-1">
          {tunnels.map((tn) => (
            <li
              key={tn.id}
              className="rounded-[5px] border border-tyba-border px-2 py-1.5"
            >
              <div className="flex items-center gap-2">
                <span
                  className={`size-1.5 shrink-0 rounded-full ${dotClass(tn)}`}
                />
                <span className="shrink-0 font-mono text-[11px] text-tyba-text-muted">
                  {tunnelFlag(tn)}
                </span>
                <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-tyba-text">
                  {describeTunnel(tn)}
                </span>
                <button
                  onClick={() => void remove(tn.id)}
                  aria-label={t("tunnelsRemove")}
                  className="shrink-0 text-tyba-text-faint hover:text-tyba-red"
                >
                  <Trash size={12} />
                </button>
              </div>
              {tn.state.state === "error" && (
                <div className="mt-1 flex items-center gap-2 pl-[14px]">
                  <span className="min-w-0 flex-1 truncate text-[11px] text-tyba-red">
                    {tn.state.detail}
                  </span>
                  <button
                    onClick={() => void retry(tn)}
                    className="flex shrink-0 items-center gap-1 text-[11px] text-tyba-text-faint hover:text-tyba-text"
                  >
                    <ArrowUUpLeft size={11} />
                    {t("tunnelsRetry")}
                  </button>
                </div>
              )}
            </li>
          ))}
        </ul>

        {pendingRisky && (
          <div className="mt-2">
            <RiskyTunnelConfirm
              tunnels={[pendingRisky]}
              host={hostAlias}
              confirmLabel={t("tunnelsConfirmYes")}
              onConfirm={() => void submit(pendingRisky, true)}
              onCancel={() => setPendingRisky(null)}
            />
          </div>
        )}

        {adding && !pendingRisky && (
          <form
            className="mt-2 flex flex-col gap-1.5 rounded-[5px] border border-tyba-border p-2"
            onSubmit={(e) => {
              e.preventDefault();
              void submit(draft, false);
            }}
          >
            <div className="flex gap-1.5">
              <select
                aria-label={t("tunnelsKind")}
                value={draft.kind}
                onChange={(e) =>
                  setDraft({
                    ...draft,
                    kind: e.target.value as TunnelKind,
                    target_host:
                      e.target.value === "dynamic" ? null : "localhost",
                    target_port: e.target.value === "dynamic" ? null : draft.target_port,
                  })
                }
                className="h-7 rounded-[4px] border border-tyba-border bg-tyba-bg px-1.5 text-[11px] text-tyba-text"
              >
                <option value="local">-L</option>
                <option value="remote">-R</option>
                <option value="dynamic">-D</option>
              </select>
              <input
                aria-label={t("tunnelsListenPort")}
                type="number"
                min={1}
                max={65535}
                required
                placeholder={t("tunnelsListenPort")}
                value={draft.listen_port || ""}
                onChange={(e) =>
                  setDraft({ ...draft, listen_port: Number(e.target.value) })
                }
                className="h-7 w-20 rounded-[4px] border border-tyba-border bg-tyba-bg px-1.5 font-mono text-[11px] text-tyba-text"
              />
              {draft.kind !== "dynamic" && (
                <>
                  <input
                    aria-label={t("tunnelsTargetHost")}
                    required
                    placeholder="localhost"
                    value={draft.target_host ?? ""}
                    onChange={(e) =>
                      setDraft({ ...draft, target_host: e.target.value })
                    }
                    className="h-7 min-w-0 flex-1 rounded-[4px] border border-tyba-border bg-tyba-bg px-1.5 font-mono text-[11px] text-tyba-text"
                  />
                  <input
                    aria-label={t("tunnelsTargetPort")}
                    type="number"
                    min={1}
                    max={65535}
                    required
                    placeholder={t("tunnelsTargetPort")}
                    value={draft.target_port || ""}
                    onChange={(e) =>
                      setDraft({ ...draft, target_port: Number(e.target.value) })
                    }
                    className="h-7 w-20 rounded-[4px] border border-tyba-border bg-tyba-bg px-1.5 font-mono text-[11px] text-tyba-text"
                  />
                </>
              )}
            </div>
            {error && <p className="text-[11px] text-tyba-red">{error}</p>}
            <div className="flex gap-1.5">
              <Button size="sm" type="submit">
                {t("tunnelsOpen")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                type="button"
                onClick={() => {
                  setAdding(false);
                  setError(null);
                }}
              >
                {t("tunnelsCancel")}
              </Button>
            </div>
          </form>
        )}

        {!adding && !pendingRisky && (
          <button
            onClick={() => setAdding(true)}
            className="mt-2 flex items-center gap-1.5 px-1 text-[11px] text-tyba-text-faint hover:text-tyba-text"
          >
            <Plus size={11} />
            {t("tunnelsNew")}
          </button>
        )}
      </div>
    </div>
  );
}
