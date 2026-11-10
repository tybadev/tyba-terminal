import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";

import { STRIP_GAP, pickStripLevel } from "@/lib/orbitStrip";

/**
 * Degrau da faixa do Orbit pela largura da linha dela, não da janela
 * (quadro FaixaOrbit). Sem breakpoint fixo: mede a largura natural das duas
 * placas no degrau em que está e guarda por degrau. Se a largura do degrau
 * atual mudar (nome do grafo, idioma, "N agentes em conversa"), o que estava
 * guardado dos outros degraus caducou e é medido de novo ao passar por eles —
 * no layout effect, antes de pintar.
 *
 * Devolve também a altura da linha, que cresce quando a placa da direita
 * desce (flex-wrap): as gavetas ficam 8 abaixo dela.
 */
export function useStripLevel(
  row: RefObject<HTMLElement | null>,
  left: RefObject<HTMLElement | null>,
  right: RefObject<HTMLElement | null>,
) {
  const [level, setLevel] = useState(0);
  const [height, setHeight] = useState(44);
  // O degrau que está no DOM (o último renderizado), não o pedido.
  const current = useRef(0);
  const needs = useRef<(number | undefined)[]>([]);

  const measure = useCallback(() => {
    const r = row.current;
    const l = left.current;
    const rt = right.current;
    if (!r || !l || !rt) return;
    const at = current.current;
    const need = l.offsetWidth + rt.offsetWidth + STRIP_GAP;
    if (needs.current[at] !== need) {
      if (needs.current[at] !== undefined) needs.current = [];
      needs.current[at] = need;
    }
    setLevel(pickStripLevel(needs.current, r.clientWidth, at));
    setHeight(r.offsetHeight);
  }, [row, left, right]);

  // A cada render: o conteúdo das placas muda sem a linha mudar de tamanho.
  useLayoutEffect(() => {
    current.current = level;
    measure();
  });

  useEffect(() => {
    const ro = new ResizeObserver(() => measure());
    for (const el of [row.current, left.current, right.current]) if (el) ro.observe(el);
    return () => ro.disconnect();
  }, [measure, row, left, right]);

  return { level, height };
}
