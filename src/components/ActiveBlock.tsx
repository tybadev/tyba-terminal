import type { PaneRectStyle } from "./TerminalView";

export {
  blocksRect,
  hiddenFraction,
  liveRect,
  LIVE_DELAY_MS,
  LIVE_FRACTION,
  termRect,
  usedFraction,
} from "../lib/liveSeam";

/**
 * O header do bloco que está executando, encaixado logo acima da saída ao vivo.
 *
 * É o que dá ao comando UMA identidade do início ao fim: o cartão existe desde o
 * primeiro byte, e no final só o conteúdo troca de renderizador — de xterm para
 * DOM — no mesmo lugar, com o mesmo texto.
 */
export function ActiveBlockHeader({
  command,
  rect,
}: {
  command: string;
  rect: PaneRectStyle;
}) {
  return (
    <div
      className="z-10 flex items-center gap-2 rounded-t-[5px] border border-b-0 border-tyba-border bg-tyba-sunken px-2.5 py-1"
      style={{
        position: "absolute",
        left: `${rect.left}%`,
        top: `${rect.top}%`,
        width: `${rect.width}%`,
        transform: "translateY(-100%)",
      }}
    >
      <span className="shrink-0 text-tyba-green">❯</span>
      <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-tyba-text">
        {command}
      </span>
      <span className="size-1.5 shrink-0 animate-pulse rounded-full bg-tyba-green" />
    </div>
  );
}

/**
 * A moldura em volta da saída ao vivo — o resto do cartão que o header começa.
 *
 * Sem ela o bloco em execução é o único da lista sem caixa fechada: o header
 * desenha a tampa e o corpo termina no vazio, o que lê como bloco cortado no
 * meio, ainda mais ao lado dos cartões já prontos logo acima.
 *
 * É um irmão do terminal, não uma borda nele: o corpo ao vivo é recortado por
 * `clip-path`, e uma borda no próprio elemento seria cortada junto — some
 * justamente a linha de baixo, que é a que fecha a caixa.
 */
export function ActiveBlockFrame({ rect }: { rect: PaneRectStyle }) {
  return (
    <div
      className="pointer-events-none z-10 rounded-b-[5px] border border-t-0 border-tyba-border"
      style={{
        position: "absolute",
        left: `${rect.left}%`,
        top: `${rect.top}%`,
        width: `${rect.width}%`,
        height: `${rect.height}%`,
      }}
    />
  );
}
