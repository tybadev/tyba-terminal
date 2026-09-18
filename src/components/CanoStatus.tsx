import { useState } from "react";
import {
  ArrowClockwise,
  CircleNotch,
  Copy,
  PencilSimple,
  Warning,
  WarningOctagon,
} from "@phosphor-icons/react";

import i18n from "../i18n";
import { writeClipboardText } from "../lib/clipboard";
import { canoDisplay, failureCopy, nextInDrop } from "../lib/canoFailure";
import type { CanoFailure, ConnectionState } from "../lib/ipc";

export interface CanoRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

interface Props {
  rect: CanoRect | null;
  visible: boolean;
  connection: ConnectionState | undefined;
  failure: CanoFailure | null | undefined;
  /** `ssh-keygen -R …` do Host, para a digital trocada. */
  knownHostsCommand: string | null;
  onRetry?: () => void;
  onEditHost?: () => void;
}

const paneBox = (rect: CanoRect) => ({
  position: "absolute" as const,
  left: `${rect.left}%`,
  top: `${rect.top}%`,
  width: `${rect.width}%`,
  height: `${rect.height}%`,
});

function CopyCommand({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="flex items-center gap-1.5 rounded-[3px] border border-tyba-red/30 bg-tyba-bg px-2 py-1">
      <code className="min-w-0 flex-1 select-all truncate font-mono text-[11px] text-tyba-text">
        {command}
      </code>
      <button
        type="button"
        onClick={() =>
          void writeClipboardText(command)
            .then(() => setCopied(true))
            .catch(() => {})
        }
        className="flex shrink-0 items-center gap-1 font-mono text-[10px] text-tyba-text-faint hover:text-tyba-text"
      >
        <Copy size={11} />
        {i18n.t(copied ? "canoCopied" : "canoCopyCommand")}
      </button>
    </div>
  );
}

export function CanoStatus({
  rect: paneRect,
  visible,
  connection,
  failure,
  knownHostsCommand,
  onRetry,
  onEditHost,
}: Props) {
  // Ajuste de estado durante o render: a fase anterior decide se um
  // `connecting` é tentativa dentro de uma queda.
  const [track, setTrack] = useState({ connection, inDrop: false });
  let inDrop = track.inDrop;
  if (track.connection !== connection) {
    inDrop = nextInDrop(track.inDrop, connection);
    setTrack({ connection, inDrop });
  }

  const display = canoDisplay(connection, failure, inDrop);

  if (display.kind === "none" || !visible || !paneRect) return null;
  const rect = paneRect;

  if (display.kind === "strip") {
    // Embaixo e sem capturar clique: o prompt de senha/passphrase nasce no
    // topo de um pane recém-aberto e precisa continuar visível e digitável.
    return (
      <div
        className="pointer-events-none z-10 flex items-end justify-center"
        style={paneBox(rect)}
      >
        <div className="mb-2 flex items-center gap-2 rounded-[4px] border border-tyba-border bg-tyba-sunken/95 px-2.5 py-1">
          <CircleNotch
            size={12}
            className="animate-spin text-tyba-text-faint"
            weight="bold"
          />
          <span className="font-mono text-[11px] text-tyba-text-faint">
            {i18n.t(display.labelKey)}
          </span>
        </div>
      </div>
    );
  }

  if (display.kind === "overlay") {
    return (
      <div
        className="z-10 flex flex-col items-center justify-center gap-2 rounded-[4px] bg-tyba-sunken/90"
        style={paneBox(rect)}
      >
        {display.spinner && (
          <CircleNotch
            size={14}
            className="animate-spin text-tyba-text-faint"
            weight="bold"
          />
        )}
        <span className="font-mono text-[11px] text-tyba-text-faint">
          {i18n.t(display.labelKey)}
        </span>
        <span className="font-mono text-[10px] text-tyba-text-faint/70">
          {i18n.t("sshSessionAlive")}
        </span>
        {display.retry && onRetry && (
          <button
            type="button"
            onClick={onRetry}
            className="mt-1 rounded-[3px] border border-tyba-border px-2 py-1 font-mono text-[10px] text-tyba-text hover:bg-tyba-raised"
          >
            {i18n.t("sshReconnect")}
          </button>
        )}
      </div>
    );
  }

  const { reason, detail } = display.failure;
  const copy = failureCopy(reason);
  const danger = copy.tone === "danger";
  // Cartão embaixo: a saída do `ssh` que explica a falha fica visível acima.
  return (
    <div
      className="pointer-events-none z-10 flex items-end justify-center p-3"
      style={paneBox(rect)}
    >
      <div
        role="alert"
        className={`pointer-events-auto flex w-full max-w-xl flex-col gap-2 rounded-[6px] border bg-tyba-sunken p-3 ${
          danger ? "border-tyba-red/60" : "border-tyba-amber/50"
        }`}
      >
        <div className="flex items-start gap-2">
          {danger ? (
            <WarningOctagon size={16} weight="fill" className="mt-0.5 shrink-0 text-tyba-red" />
          ) : (
            <Warning size={16} weight="fill" className="mt-0.5 shrink-0 text-tyba-amber" />
          )}
          <div className="min-w-0 flex-1">
            <p
              className={`text-[13px] font-semibold ${
                danger ? "text-tyba-red" : "text-tyba-text"
              }`}
            >
              {i18n.t(copy.titleKey)}
            </p>
            <p className="mt-0.5 text-[12px] text-tyba-text-muted">
              {i18n.t(copy.hintKey)}
            </p>
          </div>
        </div>

        {/* Regra 21: só o comando para copiar; nada aqui aceita ou remove digital. */}
        {reason === "host_key_changed" && knownHostsCommand && (
          <CopyCommand command={knownHostsCommand} />
        )}

        {detail && (
          <p className="break-words font-mono text-[10px] text-tyba-text-faint">
            {detail}
          </p>
        )}

        <div className="flex items-center justify-between gap-2">
          <span className="text-[10px] text-tyba-text-faint">
            {i18n.t("canoNoAutoRetry")}
          </span>
          <div className="flex shrink-0 items-center gap-1.5">
            {onEditHost && (
              <button
                type="button"
                onClick={onEditHost}
                className="flex items-center gap-1 rounded-[3px] border border-tyba-border px-2 py-1 font-mono text-[10px] text-tyba-text hover:bg-tyba-raised"
              >
                <PencilSimple size={11} />
                {i18n.t("canoEditHost")}
              </button>
            )}
            {onRetry && (
              <button
                type="button"
                onClick={onRetry}
                className="flex items-center gap-1 rounded-[3px] border border-tyba-border px-2 py-1 font-mono text-[10px] text-tyba-text hover:bg-tyba-raised"
              >
                <ArrowClockwise size={11} />
                {i18n.t("canoRetry")}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
