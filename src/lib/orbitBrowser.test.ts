import { describe, expect, test } from "bun:test";

import {
  AGENT_ACTIVE_MS,
  agentActiveLeft,
  agentStepKey,
  type BrowserState,
  NATIVE_ZOOM_MAX,
  NATIVE_ZOOM_MIN,
  SETTLE_MS,
  occlusionVerdict,
  ownerFooter,
  previewTime,
  rectInside,
  rectsIntersect,
  sameRect,
  samplePoints,
  trackMotion,
  type OcclusionInput,
} from "./orbitBrowser";

const area = { x: 0, y: 40, w: 1200, h: 760 };
const marker = { x: 100, y: 100, w: 400, h: 300 };
const base: OcclusionInput = {
  marker,
  area,
  zoom: 1,
  moving: false,
  occluders: [],
  covered: false,
};

describe("occlusionVerdict", () => {
  test("inteiro, parado, zoom 1 e nada por cima: viva", () => {
    expect(occlusionVerdict(base)).toBe(true);
  });

  test("cortado pela borda da área útil: prévia", () => {
    expect(occlusionVerdict({ ...base, marker: { ...marker, y: 20 } })).toBe(false);
    expect(occlusionVerdict({ ...base, marker: { ...marker, x: 1000 } })).toBe(false);
  });

  test("zoom fora da faixa: prévia", () => {
    expect(occlusionVerdict({ ...base, zoom: 0.37 })).toBe(false);
    expect(occlusionVerdict({ ...base, zoom: 2.5 })).toBe(false);
    expect(occlusionVerdict({ ...base, zoom: NATIVE_ZOOM_MIN })).toBe(true);
    expect(occlusionVerdict({ ...base, zoom: NATIVE_ZOOM_MAX })).toBe(true);
  });

  test("menu por cima ou ponto coberto: prévia", () => {
    const menu = { x: 450, y: 380, w: 200, h: 120 };
    expect(occlusionVerdict({ ...base, occluders: [menu] })).toBe(false);
    expect(occlusionVerdict({ ...base, occluders: [{ x: 600, y: 0, w: 10, h: 10 }] })).toBe(true);
    expect(occlusionVerdict({ ...base, covered: true })).toBe(false);
  });

  test("pan ou zoom em andamento: prévia", () => {
    expect(occlusionVerdict({ ...base, moving: true })).toBe(false);
  });


  test("marcador sem tamanho nunca fica vivo", () => {
    expect(occlusionVerdict({ ...base, marker: { ...marker, w: 0 } })).toBe(false);
  });
});

describe("trackMotion (debounce)", () => {
  test("retângulo novo é movimento até assentar 150 ms", () => {
    let m = { rect: null, changedAt: 0 } as Parameters<typeof trackMotion>[0];
    let r = trackMotion(m, marker, 1000);
    expect(r.moving).toBe(true);
    m = r.motion;
    r = trackMotion(m, marker, 1000 + SETTLE_MS - 1);
    expect(r.moving).toBe(true);
    r = trackMotion(m, marker, 1000 + SETTLE_MS);
    expect(r.moving).toBe(false);
    // Mexeu de novo: o relógio recomeça.
    r = trackMotion(r.motion, { ...marker, x: 101 }, 2000);
    expect(r.moving).toBe(true);
    expect(r.motion.changedAt).toBe(2000);
    // Meio px não conta.
    expect(trackMotion(r.motion, { ...marker, x: 101.2 }, 2200).moving).toBe(false);
  });
});

describe("geometria", () => {
  test("amostra: grade 5×5 e os quatro cantos, por dentro", () => {
    const pts = samplePoints(marker);
    expect(pts).toHaveLength(29);
    for (const [x, y] of pts) {
      expect(x).toBeGreaterThan(marker.x);
      expect(x).toBeLessThan(marker.x + marker.w);
      expect(y).toBeGreaterThan(marker.y);
      expect(y).toBeLessThan(marker.y + marker.h);
    }
  });

  test("dentro, interseção e igualdade", () => {
    expect(rectInside(marker, area)).toBe(true);
    expect(rectInside(area, marker)).toBe(false);
    expect(rectsIntersect(marker, { x: 499, y: 399, w: 5, h: 5 })).toBe(true);
    expect(rectsIntersect(marker, { x: 500, y: 100, w: 5, h: 5 })).toBe(false);
    expect(sameRect(null, null)).toBe(true);
    expect(sameRect(marker, null)).toBe(false);
  });

  test("hora do print", () => {
    const at = new Date(2026, 9, 16, 9, 5).getTime();
    expect(previewTime(at)).toBe("09:05");
  });
});

describe("agente usando", () => {
  const state = (over: Partial<BrowserState>): BrowserState => ({
    url: "http://localhost:5173/",
    title: "",
    loading: false,
    focus: "none",
    live: true,
    previewAt: null,
    refused: null,
    ephemeral: false,
    agent: null,
    consoleErrors: null,
    ...over,
  });
  const step = { by: "Dev da agenda", action: "click", target: "Semana", at: 10_000 };

  test("sem passo de agente: não está ativo", () => {
    expect(agentActiveLeft(null, 0)).toBe(0);
    expect(agentActiveLeft(state({}), 0)).toBe(0);
  });

  test("ativo por um tempo depois do último passo, sem piscar entre passos", () => {
    expect(agentActiveLeft(state({ agent: step }), 10_000)).toBe(AGENT_ACTIVE_MS);
    expect(agentActiveLeft(state({ agent: step }), 10_000 + AGENT_ACTIVE_MS)).toBe(0);
    // O core segurando o foco "agente" mantém ativo.
    expect(
      agentActiveLeft(state({ agent: step, focus: "agent" }), 10_000 + AGENT_ACTIVE_MS),
    ).toBeGreaterThan(0);
  });

  test("a chave do passo", () => {
    expect(agentStepKey(step)).toBe("orbitBrowserStep_click");
    expect(agentStepKey({ ...step, action: "type", target: null })).toBe("orbitBrowserStep_typeBare");
    expect(agentStepKey({ ...step, action: "snapshot", target: null })).toBe("orbitBrowserStep_snapshot");
    expect(agentStepKey({ ...step, action: "outra" })).toBe("orbitBrowserStep_snapshot");
  });
});

describe("ownerFooter", () => {
  test("com agente ligado, diz quem espera; sem, só o Esc", () => {
    expect(ownerFooter(["Revisor", "Outro"])).toEqual({
      key: "orbitBrowserInUseNativeWaiting",
      name: "Revisor",
    });
    expect(ownerFooter([])).toEqual({ key: "orbitBrowserInUseNative" });
  });
});
