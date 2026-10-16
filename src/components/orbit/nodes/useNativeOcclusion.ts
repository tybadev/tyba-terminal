import { useEffect, useRef, useState, type RefObject } from "react";
import { useStoreApi } from "@xyflow/react";

import {
  OCCLUDER_SELECTOR,
  SETTLE_MS,
  browserPreview,
  browserState,
  browserView,
  occlusionVerdict,
  onBrowserState,
  samplePoints,
  trackMotion,
  type BrowserRect,
  type BrowserState,
  type Motion,
} from "@/lib/orbitBrowser";

const toRect = (b: DOMRect): BrowserRect => ({ x: b.left, y: b.top, w: b.width, h: b.height });

/**
 * O estado do navegador nativo do nó, como o core emite (`orbit-browser`).
 * O front só desenha: url, título, foco e print vêm daqui.
 */
export function useBrowserState(
  graphId: string,
  nodeId: string,
): { state: BrowserState | null; ready: boolean } {
  const [state, setState] = useState<BrowserState | null>(null);
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let alive = true;
    setReady(false);
    void browserState(graphId, nodeId)
      .then((s) => {
        if (!alive) return;
        setState(s);
        setReady(true);
      })
      .catch(() => alive && setReady(true));
    const off = onBrowserState((e) => {
      if (e.graph === graphId && e.node === nodeId) setState(e.state);
    });
    return () => {
      alive = false;
      void off.then((f) => f());
    };
  }, [graphId, nodeId]);
  return { state, ready };
}

/** O último print (data URL), buscado de novo a cada `previewAt`. */
export function useBrowserPreview(
  graphId: string,
  nodeId: string,
  previewAt: number | null | undefined,
): string | null {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    if (previewAt == null) return;
    let alive = true;
    void browserPreview(graphId, nodeId)
      .then((b64) => alive && b64 && setSrc(`data:image/jpeg;base64,${b64}`))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [graphId, nodeId, previewAt]);
  return src;
}

/**
 * Decide quando a view nativa do nó pode aparecer e manda a geometria ao
 * core (`orbit_browser_view`). A view fica por cima de todo o HTML, então ela
 * só vive com o marcador inteiro na área do canvas, zoom na faixa, canvas
 * parado há 150 ms e nada do DOM por cima (grade de pontos + menus/diálogos
 * medidos). Senão manda `null` e o card mostra o último print. Só a mudança
 * vai ao core.
 *
 * Confere num `requestAnimationFrame` depois de cada mudança do xyflow, de
 * mutação no DOM, de resize e de foco — e a cada 500 ms, de seguro.
 */
export function useNativeOcclusion(
  graphId: string,
  nodeId: string,
  markerRef: RefObject<HTMLElement | null>,
  enabled: boolean,
  owner: boolean,
): void {
  const store = useStoreApi();
  // O foco do dono muda a regra, não a assinatura: um ref, para não
  // desmontar (e piscar a view) a cada clique.
  const ownerRef = useRef(owner);
  ownerRef.current = owner;
  useEffect(() => {
    if (!enabled || !graphId) return;
    let raf = 0;
    let settle = 0;
    let motion: Motion = { rect: null, changedAt: 0 };
    let sent: string | null = null;

    const send = (rect: BrowserRect | null, zoom: number, center: [number, number] | null) => {
      const key = rect
        ? `${Math.round(rect.x)},${Math.round(rect.y)},${Math.round(rect.w)},${Math.round(rect.h)}@${zoom.toFixed(3)}`
        : "null";
      if (key === sent) return;
      sent = key;
      void browserView(graphId, nodeId, rect, zoom, center).catch(() => {});
    };

    const check = () => {
      raf = 0;
      const el = markerRef.current;
      if (!el) return;
      const marker = toRect(el.getBoundingClientRect());
      const tracked = trackMotion(motion, marker, performance.now());
      motion = tracked.motion;
      const s = store.getState();
      const zoom = s.transform[2];
      const busy =
        s.paneDragging ||
        s.userSelectionActive ||
        s.connection.inProgress ||
        s.nodes.some((n) => n.dragging);
      const areaEl = el.closest(".react-flow");
      const area = areaEl
        ? toRect(areaEl.getBoundingClientRect())
        : { x: 0, y: 0, w: window.innerWidth, h: window.innerHeight };
      const covered = samplePoints(marker).some(([x, y]) => {
        const top = document.elementFromPoint(x, y);
        return !top || !(top === el || el.contains(top));
      });
      const occluders: BrowserRect[] = [];
      document.querySelectorAll(OCCLUDER_SELECTOR).forEach((o) => {
        if (!el.contains(o)) occluders.push(toRect(o.getBoundingClientRect()));
      });
      const live = occlusionVerdict({
        marker,
        area,
        zoom,
        moving: busy || tracked.moving,
        occluders,
        covered,
        owner: ownerRef.current,
      });
      send(live ? marker : null, zoom, [area.x + area.w / 2, area.y + area.h / 2]);
      // Ainda mexendo: confere de novo quando assentar.
      if (tracked.moving || busy) {
        window.clearTimeout(settle);
        settle = window.setTimeout(schedule, SETTLE_MS);
      }
    };
    const schedule = () => {
      if (!raf) raf = requestAnimationFrame(check);
    };
    // Mutação no DOM é frequente (terminais, relógio): no máximo uma
    // conferência a cada 100 ms por ela.
    let soon = 0;
    const scheduleSoon = () => {
      if (!soon)
        soon = window.setTimeout(() => {
          soon = 0;
          schedule();
        }, 100);
    };

    const unsubscribe = store.subscribe(schedule);
    const mutations = new MutationObserver(scheduleSoon);
    mutations.observe(document.body, { childList: true, subtree: true });
    const resize = new ResizeObserver(schedule);
    if (markerRef.current) resize.observe(markerRef.current);
    window.addEventListener("resize", schedule);
    window.addEventListener("focusin", schedule);
    window.addEventListener("focusout", schedule);
    const tick = window.setInterval(schedule, 500);
    schedule();
    return () => {
      unsubscribe();
      mutations.disconnect();
      resize.disconnect();
      window.removeEventListener("resize", schedule);
      window.removeEventListener("focusin", schedule);
      window.removeEventListener("focusout", schedule);
      window.clearInterval(tick);
      window.clearTimeout(settle);
      window.clearTimeout(soon);
      if (raf) cancelAnimationFrame(raf);
      // Card saiu (Orbit fechado, nó apagado): a view some na hora.
      void browserView(graphId, nodeId, null, 1, null).catch(() => {});
    };
  }, [enabled, graphId, nodeId, store, markerRef]);
}
