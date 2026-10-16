import { useEffect, useRef, useState } from "react";

import { useStore } from "@xyflow/react";

import { TerminalView } from "@/components/TerminalView";
import { sessionScreenLines } from "@/lib/ipc";
import { canvasWheel, nodeTerminalClass } from "@/lib/nodeTerminal";
import { getTerm } from "@/lib/termRegistry";
import { useMaestro } from "./NodeFrame";

/** O terminal ocupa o nó inteiro: o wrapper é quem tem o tamanho. */
export const FULL = { left: 0, top: 0, width: 100, height: 100 };

const PREVIEW_LINES = 14;
/** A prévia não precisa de quadro a quadro: a tela do core é lida a cada tanto. */
const PREVIEW_POLL_MS = 400;

/**
 * Terminal do nó: o `TerminalView` de verdade em qualquer zoom — dá para
 * digitar sem ir a 100%. O xterm fica fora da escala do canvas (ver
 * `nodeTerminal`): texto nítido no WebGL, mouse na célula certa, e o tamanho
 * do PTY é o do nó em 100%, que não muda com o zoom.
 *
 * Selecionar o nó não foca o terminal: o foco vem do clique dentro dele (o
 * xterm cancela o mousedown, então o foco é dado aqui) ou do Enter com o nó
 * selecionado, no canvas. Assim ⌘A não foca todos, ⌘D/⌘C seguem do canvas
 * num nó rodando e o título se edita sem o terminal roubar o teclado.
 *
 * Com o nó aberto no modo foco (`NodeFocus`) vai a prévia em texto: a sessão
 * só monta o xterm em UM lugar. O "Abrir" do ⋯ do nó abre o foco.
 */
export function NodeTerminal({
  nodeId,
  sessionId,
  selected,
}: {
  nodeId: string;
  sessionId: string;
  selected: boolean;
}) {
  const { focusNode } = useMaestro();
  const zoom = useStore((s) => s.transform[2]);
  const inFocus = focusNode === nodeId;

  // Roda sem o nó selecionado: pega na captura, antes do xterm (que rolaria
  // o scrollback ou mandaria setas ao PTY), e entrega ao canvas.
  const box = useRef<HTMLDivElement>(null);
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      const init = canvasWheel(e, selectedRef.current);
      if (!init) return;
      e.preventDefault();
      e.stopPropagation();
      el.parentElement?.dispatchEvent(new WheelEvent("wheel", init));
    };
    el.addEventListener("wheel", onWheel, { capture: true, passive: false });
    return () => el.removeEventListener("wheel", onWheel, { capture: true });
  }, []);

  return (
    <div
      ref={box}
      className={`${nodeTerminalClass(selected)} relative min-h-0 flex-1 bg-tyba-sunken`}
    >
      {inFocus ? (
        <TerminalPreview sessionId={sessionId} />
      ) : (
        // A margem fica do lado de fora do invólucro de zoom: escala junto
        // com o nó, como o resto do cartão.
        <div className="absolute top-2 right-2.5 bottom-3 left-2.5">
          <TerminalView
            sessionId={sessionId}
            visible
            focused={false}
            rect={FULL}
            zoom={zoom}
            fullBleed
            onFocus={() => getTerm(sessionId)?.term.focus()}
          />
        </div>
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
      className="absolute inset-0 flex flex-col justify-end overflow-hidden px-3.5 pt-2.5 pb-3 font-mono text-[11.5px] leading-[17px] text-tyba-text-muted"
    >
      {/* Cada linha com altura fixa e sem encolher. Item de flex encolhe por
          padrão, e com `overflow-hidden` o mínimo dele vira 0: no nó baixo as
          14 linhas espremiam em poucos px cada, cada uma mostrando só uma
          fatia do texto de 17px — as fatias empilhadas pareciam linhas umas
          por cima das outras. Assim a pilha passa da altura, o `justify-end` empurra
          o excesso para cima e o `overflow-hidden` do pai corta por cima —
          ficam as últimas linhas inteiras. */}
      {lines.map((line, i) => (
        <div key={i} className="h-[17px] shrink-0 overflow-hidden text-ellipsis whitespace-pre">
          {line || " "}
        </div>
      ))}
    </div>
  );
}
