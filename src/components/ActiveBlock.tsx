import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { formatDuration } from "../lib/blockLayout";

import { LIVE_INSET_X_PX, type PaneRectStyle } from "./TerminalView";

export {
  blocksRect,
  hiddenFraction,
  liveRect,
  padSlackPx,
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
  startedAtMs,
  note = null,
  rect,
  liftPx = 0,
  paneTop = 0,
  onHeight,
  below = null,
}: {
  command: string;
  /** Quando o comando começou, pelo relógio do core. `null`: sem contador. */
  startedAtMs: number | null;
  /** Depois do tempo, com " · " (o nó Terminal: "ferramentas do Orbit"). */
  note?: string | null;
  rect: PaneRectStyle;
  /**
   * Quanto a faixa ao vivo sobe além do que a conta em % diz, em px.
   *
   * O recorte do terminal desconta o padding dele, então a saída aparece um
   * pouco mais alta do que `liveRect` calcula. O header sobe junto —
   * senão a tampa flutua no meio da saída. Ver `padSlackPx`.
   */
  liftPx?: number;
  /**
   * Topo do painel, em %. Com a saída enchendo o painel inteiro o header
   * para aqui, preso, e a saída rola por baixo dele — sem o teto ele saía
   * pela borda de cima e o comando sumia de vista.
   */
  paneTop?: number;
  /**
   * A altura que este header ocupa, medida.
   *
   * Ele fica ACIMA da faixa ao vivo, sobre o fim da lista de blocos — que
   * precisa encurtar do mesmo tanto para o último bloco não passar por baixo
   * dele. Medido e não fixo: o header já mudou de altura antes (o ponto
   * pulsante entrou depois), e número fixo aqui volta como bloco cortado.
   */
  onHeight?: (px: number) => void;
  /**
   * Peça entre o comando e a saída (o aviso do `ssh` digitado). O header
   * cresce para caber, e a lista encurta junto pela altura medida.
   */
  below?: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  // O callback entra por ref, e o efeito não depende dele.
  //
  // Quem chama passa uma arrow que fecha sobre o id da sessão, ou seja: função
  // nova a cada render. Na dependência do efeito, isso desmontaria e remontaria
  // o `ResizeObserver` em todo render — um observer que se recria não observa
  // nada, e ainda relata a altura de novo, o que renderiza de novo.
  const onHeightRef = useRef(onHeight);
  onHeightRef.current = onHeight;
  // A altura também segura o teto: com a peça de baixo o header passa de 36px,
  // e preso no topo do painel ele não pode sair pela borda.
  const [height, setHeight] = useState(36);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const report = () => {
      const px = el.getBoundingClientRect().height;
      setHeight(px);
      onHeightRef.current?.(px);
    };
    report();
    const ro = new ResizeObserver(report);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const { t, i18n } = useTranslation();
  // O contador anda de segundo em segundo. O relógio é do core
  // (`started_at_ms`); aqui só se lê a hora para desenhar.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (startedAtMs === null) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [startedAtMs]);
  const elapsed =
    startedAtMs === null
      ? null
      : formatDuration(Math.max(0, now - startedAtMs), i18n.language, {
          whole: true,
        });

  return (
    <div
      ref={ref}
      // Sem caixa, como os blocos prontos: o que diz "este roda" é o filete
      // vivo no topo e o ponto aceso. `px-2.5` + o recuo da faixa põem o comando
      // na mesma coluna do comando dos blocos da lista (`px-[18px]`, mockup).
      className={`z-10 flex flex-col overflow-hidden bg-tyba-paper-solid px-2.5 pt-2.5 ${
        below ? "pb-3" : "h-9"
      }`}
      style={{
        position: "absolute",
        left: `calc(${rect.left}% + ${LIVE_INSET_X_PX}px)`,
        // `translateY(-100%)` põe a BASE do header em `top`; o `max` não
        // deixa a base subir acima de um header de altura abaixo do topo.
        top: `max(calc(${rect.top}% - ${liftPx}px), calc(${paneTop}% + ${Math.max(36, height)}px))`,
        width: `calc(${rect.width}% - ${LIVE_INSET_X_PX * 2}px)`,
        transform: "translateY(-100%)",
      }}
    >
      {/* O filete de luz que atravessa a tampa do cartão enquanto o comando
          roda. É a peça `.tyba-flow-line` do design system — "o que está
          executando" —, que estava no CSS sem nenhum uso.

          Na BORDA, e não no texto: o efeito serve para anunciar que há algo em
          curso, e animar o nome do comando faria mexer justamente o que se quer
          ler. Ele para sozinho em `prefers-reduced-motion`. */}
      <span
        aria-hidden
        className="tyba-flow-line tyba-flow-line--live tyba-flow-line--thick pointer-events-none absolute inset-x-0 top-0"
      />
      <div className="flex min-w-0 items-start gap-2">
        <span className="shrink-0 font-mono text-[13px] leading-5 text-tyba-green">
          ❯
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-[13px] leading-5 text-tyba-text">
          {command}
        </span>
        {/* O ponto fica: o filete diz "algo corre", o ponto diz "é este bloco".
            Aceso (glow) porque está vivo; o pulso para em reduced-motion. */}
        <span className="flex shrink-0 items-center gap-2 font-mono text-[11px] leading-5 text-tyba-green">
          <span className="size-[7px] shrink-0 rounded-full bg-tyba-green-fill shadow-[var(--tyba-glow-green)] tyba-pulse" />
          {[elapsed && t("blockRunningFor", { time: elapsed }), note].filter(Boolean).join(" · ")}
        </span>
      </div>
      {below}
    </div>
  );
}
