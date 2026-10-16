import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowsOut } from "@phosphor-icons/react";

import { TerminalView } from "@/components/TerminalView";
import { sessionScreenLines } from "@/lib/ipc";
import type { NodeKind, NodeRun } from "@/lib/maestro";
import { useMaestro } from "./NodeFrame";

/** O terminal ocupa o nó inteiro: o wrapper é quem tem o tamanho. */
export const FULL = { left: 0, top: 0, width: 100, height: 100 };

const PREVIEW_LINES = 14;
/** A prévia não precisa de quadro a quadro: a tela do core é lida a cada tanto. */
const PREVIEW_POLL_MS = 400;

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
 * então vai a prévia das últimas linhas em texto. Com o nó aberto no modo foco
 * (`NodeFocus`) também vai a prévia: a sessão só monta o xterm em UM lugar.
 * Duplo clique na prévia abre o foco; no xterm de verdade o duplo clique fica
 * com ele (selecionar palavra), e o ⤢ do cabeçalho serve igual.
 */
export function NodeTerminal({
  nodeId,
  sessionId,
  focused,
}: {
  nodeId: string;
  sessionId: string;
  focused: boolean;
}) {
  const { actualSize, focusNode, setFocusNode } = useMaestro();
  const inFocus = focusNode === nodeId;
  return (
    <div className="nodrag nowheel nopan nokey relative min-h-0 flex-1 bg-tyba-sunken">
      {actualSize && !inFocus ? (
        <TerminalView sessionId={sessionId} visible focused={focused} rect={FULL} />
      ) : (
        <TerminalPreview
          sessionId={sessionId}
          onOpen={inFocus ? undefined : () => setFocusNode(nodeId)}
        />
      )}
    </div>
  );
}

/** ⤢ do cabeçalho: abre o terminal do nó em tamanho real. */
export function FocusButton({ nodeId }: { nodeId: string }) {
  const { t } = useTranslation();
  const { setFocusNode } = useMaestro();
  return (
    <button
      type="button"
      aria-label={t("maestroFocusOpen")}
      title={t("maestroFocusOpen")}
      onClick={() => setFocusNode(nodeId)}
      className="nodrag shrink-0 rounded-[4px] p-1 text-tyba-text-faint hover:bg-tyba-raised hover:text-tyba-text"
    >
      <ArrowsOut size={13} />
    </button>
  );
}

/**
 * Últimas linhas da sessão, em texto. Quem interpreta os bytes é o próprio
 * xterm, sem abrir (sem DOM): a tela fica no buffer e daqui só se lê o texto.
 */
function TerminalPreview({ sessionId, onOpen }: { sessionId: string; onOpen?: () => void }) {
  const [lines, setLines] = useState<string[]>([]);

  // As linhas vêm da tela que o core já renderiza no tamanho real do PTY
  // (`session_screen_lines`). Já foi um segundo xterm de 160 colunas lendo o
  // mesmo fluxo: TUI que move o cursor (o Claude redesenha linhas) saía
  // embaralhada, uma linha escrita por cima da outra.
  useEffect(() => {
    let disposed = false;
    const read = () => {
      void sessionScreenLines(sessionId, PREVIEW_LINES)
        .then((rows) => {
          if (!disposed) setLines(rows);
        })
        .catch(() => {});
    };
    read();
    const timer = window.setInterval(read, PREVIEW_POLL_MS);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [sessionId]);

  return (
    <div
      role="log"
      onDoubleClick={onOpen}
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
