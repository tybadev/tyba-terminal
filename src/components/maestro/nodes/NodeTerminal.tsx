import { useEffect, useState } from "react";
import { Terminal } from "@xterm/xterm";

import { TerminalView } from "@/components/TerminalView";
import { attachSession, detachSession, onPtyOutput } from "@/lib/ipc";
import type { NodeKind, NodeRun } from "@/lib/maestro";
import { useMaestro } from "./NodeFrame";

/** O terminal ocupa o nó inteiro: o wrapper é quem tem o tamanho. */
const FULL = { left: 0, top: 0, width: 100, height: 100 };

const PREVIEW_LINES = 14;

/**
 * Sessão do nó que tem terminal para mostrar: rodando, ou o agente que ficou
 * vivo depois do primeiro turno (no run mais novo) para o dono conversar.
 */
export function nodeSession(
  kind: NodeKind,
  nodeRun: NodeRun | undefined,
  live: boolean,
): string | null {
  if (!nodeRun?.sessionId) return null;
  if (nodeRun.status === "running") return nodeRun.sessionId;
  if (kind.type === "agent" && nodeRun.status === "done" && live) return nodeRun.sessionId;
  return null;
}

/**
 * Terminal do nó. Em 100% é o `TerminalView` de verdade (o PTY tem um tamanho
 * só, e é o do nó); fora disso o canvas escala o nó e o xterm sairia borrado,
 * então vai a prévia das últimas linhas em texto.
 */
export function NodeTerminal({ sessionId, focused }: { sessionId: string; focused: boolean }) {
  const { actualSize } = useMaestro();
  return (
    <div className="nodrag nowheel nopan nokey relative min-h-0 flex-1 bg-tyba-sunken">
      {actualSize ? (
        <TerminalView sessionId={sessionId} visible focused={focused} rect={FULL} />
      ) : (
        <TerminalPreview sessionId={sessionId} />
      )}
    </div>
  );
}

/**
 * Últimas linhas da sessão, em texto. Quem interpreta os bytes é o próprio
 * xterm, sem abrir (sem DOM): a tela fica no buffer e daqui só se lê o texto.
 */
function TerminalPreview({ sessionId }: { sessionId: string }) {
  const [lines, setLines] = useState<string[]>([]);

  useEffect(() => {
    const term = new Terminal({ cols: 160, rows: PREVIEW_LINES * 2, scrollback: 200 });
    let disposed = false;
    let attached = false;
    let frame = 0;
    let unlisten: (() => void) | null = null;

    const read = () => {
      frame = 0;
      const buf = term.buffer.active;
      const end = buf.baseY + buf.cursorY;
      const out: string[] = [];
      for (let i = Math.max(0, end - PREVIEW_LINES + 1); i <= end; i++) {
        out.push(buf.getLine(i)?.translateToString(true) ?? "");
      }
      setLines(out);
    };
    const schedule = () => {
      if (!disposed && !frame) frame = requestAnimationFrame(read);
    };

    void (async () => {
      const un = await onPtyOutput(sessionId, (bytes) => {
        if (!disposed) term.write(bytes, schedule);
      });
      if (disposed) {
        un();
        return;
      }
      unlisten = un;
      await attachSession(sessionId);
      attached = true;
      if (disposed) void detachSession(sessionId).catch(() => {});
    })().catch(() => {});

    return () => {
      disposed = true;
      if (frame) cancelAnimationFrame(frame);
      unlisten?.();
      if (attached) void detachSession(sessionId).catch(() => {});
      term.dispose();
    };
  }, [sessionId]);

  return (
    <div
      role="log"
      className="absolute inset-0 flex flex-col justify-end overflow-hidden px-3.5 pt-2.5 pb-3 font-mono text-[11.5px] leading-[17px] text-tyba-text-muted"
    >
      {lines.map((line, i) => (
        <div key={i} className="overflow-hidden text-ellipsis whitespace-pre">
          {line || " "}
        </div>
      ))}
    </div>
  );
}
