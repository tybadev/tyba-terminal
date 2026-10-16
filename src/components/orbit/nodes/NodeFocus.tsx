import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { ArrowsIn, FolderSimple } from "@phosphor-icons/react";

import { TerminalView } from "@/components/TerminalView";
import { nodeDir, shortDir, type OrbitNode } from "@/lib/orbit";
import { isFocusCloseKey } from "@/lib/orbitFocus";
import { IS_MAC } from "@/lib/platform";
import { NodeStatusBadge, useOrbit } from "./NodeFrame";
import { FULL } from "./NodeTerminal";

/**
 * Modo foco do nó: o terminal da sessão em tamanho real, por cima do canvas
 * (fora do xyflow, então sem zoom e sem o canvas brigando por teclado,
 * rolagem e Enter). Enquanto aberto, o nó mostra a prévia em texto — o xterm
 * da sessão só monta aqui. Montar dispara o fit e o resize do PTY para este
 * tamanho; fechar devolve ao nó, que refaz o fit no tamanho dele.
 *
 * Fecha pelo ⤡ ou por ⌘. (Ctrl+. fora do mac). Esc NÃO fecha: é do claude
 * (interromper) e segue para o terminal — ver `isFocusCloseKey`.
 */
export function NodeFocus({ node, sessionId }: { node: OrbitNode; sessionId: string }) {
  const { t } = useTranslation();
  const { run, graphDir, setFocusNode } = useOrbit();
  const nodeRun = run?.nodes[node.id];
  const dir = nodeDir(node.kind, graphDir);

  useEffect(() => {
    // Captura na janela: chega antes do textarea do xterm, que nem vê a tecla.
    const onKey = (e: KeyboardEvent) => {
      if (!isFocusCloseKey(e, IS_MAC)) return;
      e.preventDefault();
      e.stopPropagation();
      setFocusNode(null);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [setFocusNode]);

  const close = t("orbitFocusClose", { keys: IS_MAC ? "⌘." : "Ctrl+." });
  return (
    <div
      role="dialog"
      data-occludes
      aria-label={node.title}
      className="nokey absolute inset-0 z-20 flex flex-col bg-tyba-bg font-mono"
    >
      <div className="flex h-[46px] shrink-0 items-center gap-2.5 border-b border-tyba-border bg-tyba-surface px-3.5">
        <span className="min-w-0 truncate text-[14.5px] font-medium text-tyba-text">
          {node.title}
        </span>
        {nodeRun && <NodeStatusBadge nodeId={node.id} nodeRun={nodeRun} />}
        <span className="flex-1" />
        {dir && (
          <span
            title={dir}
            className="flex min-w-0 shrink items-center gap-1 text-[11.5px] text-tyba-text-faint"
          >
            <FolderSimple size={12} className="shrink-0" />
            <span className="truncate">{shortDir(dir)}</span>
          </span>
        )}
        <button
          type="button"
          aria-label={close}
          title={close}
          onClick={() => setFocusNode(null)}
          className="flex shrink-0 items-center gap-1.5 rounded-[6px] px-2 py-1 text-[12px] text-tyba-text-muted hover:bg-tyba-raised hover:text-tyba-text"
        >
          <ArrowsIn size={13} />
          {t("orbitFocusCloseLabel")}
        </button>
      </div>
      <div className="relative min-h-0 flex-1 bg-tyba-sunken">
        <TerminalView sessionId={sessionId} visible focused rect={FULL} />
      </div>
    </div>
  );
}
