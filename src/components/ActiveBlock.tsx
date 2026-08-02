import type { PaneRectStyle } from "./TerminalView";

/**
 * Fração do painel reservada para a saída ao vivo.
 *
 * É CONSTANTE de propósito: se a área do terminal mudasse quando o comando
 * começa e termina, o PTY seria redimensionado duas vezes por execução — o
 * mesmo bug que a linha de comando sumindo já causou.
 */
export const LIVE_FRACTION = 0.5;

/**
 * Quanto um comando precisa durar para a faixa ao vivo abrir.
 *
 * `clear`, `cd`, `echo`: abrir e fechar a faixa em 30ms espreme a lista pela
 * metade, mostra um retângulo preto e desfaz — um pisca que chama mais atenção
 * do que a coisa que ele ia mostrar. Abaixo disto o comando já virou bloco
 * antes de a faixa fazer falta.
 */
export const LIVE_DELAY_MS = 120;

export function liveRect(pane: PaneRectStyle): PaneRectStyle {
  const height = pane.height * LIVE_FRACTION;
  return {
    left: pane.left,
    top: pane.top + pane.height - height,
    width: pane.width,
    height,
  };
}

export function blocksRect(pane: PaneRectStyle, live: boolean): PaneRectStyle {
  if (!live) return pane;
  return {
    left: pane.left,
    top: pane.top,
    width: pane.width,
    height: pane.height * (1 - LIVE_FRACTION),
  };
}

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
