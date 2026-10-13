import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowsOutSimple, ArrowsInSimple, X } from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { RiskyTunnelConfirm } from "@/components/RiskyTunnelConfirm";
import { PanelIcon } from "@/components/SidePanelBar";
import type { Session, SessionTunnel, Tunnel, TunnelKind } from "@/lib/ipc";
import {
  closeSessionTunnel,
  listSessionTunnels,
  onSessionTunnels,
  openSessionTunnel,
} from "@/lib/ipc";
import { BLANK_TUNNEL, describeTunnel } from "@/lib/tunnels";

interface Props {
  session: Session;
  hostAlias: string;
  expanded: boolean;
  onToggleExpand: () => void;
  onClose: () => void;
}

function dotClass(t: SessionTunnel): string {
  if (t.state.state === "live") return "bg-tyba-green-fill";
  if (t.state.state === "error") return "bg-tyba-red-fill";
  return "bg-tyba-text-faint tyba-pulse";
}

/** Mockup Tuneis: o sentido do túnel em palavras, na cor de quem ele expõe. */
const DIRECTION: Record<TunnelKind, { key: string; cls: string }> = {
  local: { key: "tunnelsDirLocal", cls: "bg-tyba-cyan-tint text-tyba-cyan" },
  remote: { key: "tunnelsDirRemote", cls: "bg-tyba-amber-tint text-tyba-amber" },
  dynamic: { key: "tunnelsDirDynamic", cls: "bg-tyba-cyan-tint text-tyba-cyan" },
};

const KIND_LABEL: Record<TunnelKind, string> = {
  local: "tunnelsKindLocal",
  remote: "tunnelsKindRemote",
  dynamic: "tunnelsKindDynamic",
};

/** O `.btn` dos mockups: 11px, contorno 0,14, cantos de 5px. */
const BTN =
  "shrink-0 rounded-[5px] border border-tyba-border-strong px-[9px] py-[3px] text-[11px] text-tyba-text-muted hover:text-tyba-text";

const FIELD =
  "h-[30px] rounded-[6px] border border-tyba-border-field bg-tyba-sunken px-2 font-mono text-[12px] text-tyba-text outline-none focus:border-tyba-cyan/50";

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
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {/* Fundo do painel lateral que o contém; cabeçalho como o dos mockups
          Painel*: `padding: 12px 16px`, ícone 16, título 13px. */}
      <header className="flex shrink-0 items-center gap-2.5 tyba-divide-b px-4 py-3">
        <PanelIcon kind="tunnels" size={16} className="shrink-0 text-tyba-cyan" />
        <span className="min-w-0 flex-1 truncate text-[13px] text-tyba-text">
          {t("tunnelsTitle")} · {hostAlias}
        </span>
        {!adding && !pendingRisky && (
          <button
            type="button"
            onClick={() => setAdding(true)}
            className={`${BTN} border-tyba-cyan/40 text-tyba-cyan hover:text-tyba-cyan`}
          >
            {t("tunnelsNew")}
          </button>
        )}
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

      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3.5">
        {tunnels.length === 0 && !adding && (
          <p className="py-1 text-[12px] text-tyba-text-faint">
            {t("tunnelsEmpty")}
          </p>
        )}

        <ul className="flex flex-col gap-2">
          {tunnels.map((tn) => {
            const dir = DIRECTION[tn.kind];
            const broke = tn.state.state === "error";
            return (
              <li
                key={tn.id}
                // Mockup: cartão sem contorno, cantos de 8px; o que falhou ganha
                // o véu vermelho.
                className={`-mx-2.5 flex flex-col gap-[5px] rounded-[8px] p-2.5 ${
                  broke ? "bg-tyba-red/[.07]" : "hover:bg-tyba-raised hover:shadow-[var(--tyba-row-raised)]"
                }`}
              >
                <div className="flex items-center gap-2">
                  <span
                    className={`size-[7px] shrink-0 rounded-full ${dotClass(tn)}`}
                  />
                  <span className="min-w-0 flex-1 truncate font-mono text-[13px] text-tyba-text">
                    {describeTunnel(tn)}
                  </span>
                  <span className={`shrink-0 rounded-[4px] px-1.5 py-px text-[10.5px] ${dir.cls}`}>
                    {t(dir.key)}
                  </span>
                </div>
                {tn.state.state === "error" && (
                  <div className="text-[11px] text-tyba-red">{tn.state.detail}</div>
                )}
                <div className="flex gap-1.5">
                  {broke && (
                    <button
                      type="button"
                      onClick={() => void retry(tn)}
                      className={`${BTN} text-tyba-text`}
                    >
                      {t("tunnelsRetry")}
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={() => void remove(tn.id)}
                    aria-label={t("tunnelsRemove")}
                    className={BTN}
                  >
                    {t("tunnelsRemove")}
                  </button>
                </div>
              </li>
            );
          })}
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
            // Mockup: cartão de contorno 0,08, 12px de respiro, campos afundados.
            className="mt-3 flex flex-col gap-2.5 rounded-[8px] border border-tyba-text/[.08] p-3"
            onSubmit={(e) => {
              e.preventDefault();
              void submit(draft, false);
            }}
          >
            <div className="text-[12px] text-tyba-text">{t("tunnelsNew")}</div>
            <div role="radiogroup" aria-label={t("tunnelsKind")} className="flex flex-wrap gap-1.5">
              {(["local", "remote", "dynamic"] as TunnelKind[]).map((kind) => (
                <button
                  key={kind}
                  type="button"
                  role="radio"
                  aria-checked={draft.kind === kind}
                  onClick={() =>
                    setDraft({
                      ...draft,
                      kind,
                      target_host: kind === "dynamic" ? null : (draft.target_host ?? "localhost"),
                      target_port: kind === "dynamic" ? null : draft.target_port,
                    })
                  }
                  className={`rounded-[4px] px-2 py-1 text-[10.5px] ${
                    draft.kind === kind
                      ? DIRECTION[kind].cls
                      : "border border-tyba-border-field text-tyba-text-muted hover:text-tyba-text"
                  }`}
                >
                  {t(KIND_LABEL[kind])}
                </button>
              ))}
            </div>
            <div className="flex items-end gap-2">
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
                className={`${FIELD} w-20`}
              />
              {draft.kind !== "dynamic" && (
                <>
                  <span className="pb-1.5 text-tyba-text-faint">→</span>
                  <input
                    spellCheck={false}
                    autoCapitalize="off"
                    autoCorrect="off"
                    aria-label={t("tunnelsTargetHost")}
                    required
                    placeholder="localhost"
                    value={draft.target_host ?? ""}
                    onChange={(e) =>
                      setDraft({ ...draft, target_host: e.target.value })
                    }
                    className={`${FIELD} min-w-0 flex-1`}
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
                    className={`${FIELD} w-20`}
                  />
                </>
              )}
            </div>
            {error && <p className="text-[11px] text-tyba-red">{error}</p>}
            <div className="flex gap-2">
              <Button
                size="sm"
                type="submit"
                className="h-auto flex-1 rounded-[6px] bg-tyba-cyan px-3.5 py-[7px] text-[12px] font-normal text-tyba-text-inverse hover:bg-tyba-cyan/90"
              >
                {t("tunnelsOpen")}
              </Button>
              <button
                type="button"
                className={BTN}
                onClick={() => {
                  setAdding(false);
                  setError(null);
                }}
              >
                {t("tunnelsCancel")}
              </button>
            </div>
          </form>
        )}
      </div>
    </div>
  );
}
