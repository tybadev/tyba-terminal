import { describe, expect, test } from "bun:test";

import {
  freeSpot,
  minimapTone,
  nodeRect,
  onScreen,
  outsideView,
  rightOf,
  type Rect,
} from "./graphLayout";

const size = { w: 200, h: 100 };

describe("freeSpot", () => {
  test("lugar vazio: fica onde se pediu", () => {
    expect(freeSpot({ x: 10.4, y: 20.6 }, size, [])).toEqual({ x: 10, y: 21 });
  });

  test("desce passando por baixo de quem está no caminho", () => {
    const occupied: Rect[] = [{ x: 0, y: 0, w: 300, h: 300 }];
    const at = freeSpot({ x: 50, y: 50 }, size, occupied, "down", 40);
    expect(at).toEqual({ x: 50, y: 340 });
  });

  test("desvia de vários na mesma coluna", () => {
    const occupied: Rect[] = [
      { x: 0, y: 0, w: 300, h: 100 },
      { x: 0, y: 140, w: 300, h: 100 },
    ];
    expect(freeSpot({ x: 0, y: 0 }, size, occupied, "down", 40)).toEqual({ x: 0, y: 280 });
  });

  test("right: anda para a direita (bloco de subagentes embaixo do pai)", () => {
    const occupied: Rect[] = [{ x: 0, y: 400, w: 380, h: 300 }];
    const at = freeSpot({ x: 0, y: 420 }, size, occupied, "right", 40);
    expect(at).toEqual({ x: 420, y: 420 });
  });

  test("nunca cobre nenhum retângulo ocupado", () => {
    const occupied: Rect[] = [];
    for (let i = 0; i < 20; i++) {
      const at = freeSpot({ x: 0, y: 0 }, size, occupied);
      for (const r of occupied) {
        const apart =
          at.x >= r.x + r.w || r.x >= at.x + size.w || at.y >= r.y + r.h || r.y >= at.y + size.h;
        expect(apart).toBe(true);
      }
      occupied.push({ ...at, ...size });
    }
  });
});

describe("nodeRect e rightOf", () => {
  test("o tamanho medido vale sobre o do modelo", () => {
    expect(
      nodeRect({ position: { x: 1, y: 2 }, width: 30, data: { node: { size: { w: 9, h: 8 } } } }),
    ).toEqual({ x: 1, y: 2, w: 30, h: 8 });
  });

  test("vizinho nasce à direita, alinhado pelo topo", () => {
    expect(rightOf({ x: 10, y: 20, w: 100, h: 50 }, 40)).toEqual({ x: 150, y: 20 });
  });
});

describe("outsideView (minimapa liga sozinho)", () => {
  const view = { x: 0, y: 0, zoom: 1 };
  const area = { w: 1000, h: 800 };

  test("tudo à vista: não", () => {
    expect(outsideView([{ x: 10, y: 10, w: 200, h: 100 }], view, area)).toBe(false);
  });

  test("um nó passa da borda: sim", () => {
    expect(outsideView([{ x: 900, y: 10, w: 200, h: 100 }], view, area)).toBe(true);
  });

  test("o zoom muda a área visível", () => {
    const far = [{ x: 1500, y: 10, w: 200, h: 100 }];
    expect(outsideView(far, view, area)).toBe(true);
    expect(outsideView(far, { x: 0, y: 0, zoom: 0.5 }, area)).toBe(false);
  });

  test("área ainda sem medida: não", () => {
    expect(outsideView([{ x: 5000, y: 0, w: 1, h: 1 }], view, { w: 0, h: 0 })).toBe(false);
  });
});

describe("minimapTone (canvas-10)", () => {
  test("estado manda: dono âmbar, rodando verde, falhou vermelho", () => {
    expect(minimapTone("owner", true)).toBe("var(--tyba-amber)");
    expect(minimapTone("working", false)).toBe("var(--tyba-green)");
    expect(minimapTone("live", true)).toBe("var(--tyba-green)");
    expect(minimapTone("failed", true)).toBe("var(--tyba-red)");
  });

  test("criado parado ou na fila fica violeta; o resto, a borda", () => {
    expect(minimapTone("idle", true)).toBe("var(--tyba-violet)");
    expect(minimapTone("done", true)).toBe("var(--tyba-violet)");
    expect(minimapTone("idle", false)).toBe("var(--tyba-border-strong)");
    expect(minimapTone("waiting", false)).toBe("var(--tyba-text-faint)");
  });
});

describe("onScreen (terminal do nó só monta o xterm à vista)", () => {
  const pane = { w: 1200, h: 800 };
  const node: Rect = { x: 100, y: 100, w: 520, h: 300 };

  test("dentro e parcialmente dentro da tela: à vista", () => {
    expect(onScreen(node, { x: 0, y: 0, zoom: 1 }, pane)).toBe(true);
    // Só a ponta direita aparece na borda esquerda.
    expect(onScreen(node, { x: -600, y: 0, zoom: 1 }, pane)).toBe(true);
  });

  test("fora da tela, em qualquer zoom: prévia", () => {
    expect(onScreen(node, { x: -700, y: 0, zoom: 1 }, pane)).toBe(false);
    expect(onScreen(node, { x: 0, y: 900, zoom: 1 }, pane)).toBe(false);
    // Em 50% o nó em x=2500 cai em 1250 px: além da largura.
    expect(onScreen({ ...node, x: 2500 }, { x: 0, y: 0, zoom: 0.5 }, pane)).toBe(false);
    expect(onScreen({ ...node, x: 2300 }, { x: 0, y: 0, zoom: 0.5 }, pane)).toBe(true);
  });

  test("a folga monta o xterm um pouco antes de o nó entrar", () => {
    expect(onScreen(node, { x: -700, y: 0, zoom: 1 }, pane, 200)).toBe(true);
    expect(onScreen(node, { x: -900, y: 0, zoom: 1 }, pane, 200)).toBe(false);
  });

  test("canvas ainda sem medida: à vista (não esconde o terminal)", () => {
    expect(onScreen(node, { x: -5000, y: 0, zoom: 1 }, { w: 0, h: 0 })).toBe(true);
  });
});
